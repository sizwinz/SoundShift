use crate::engine::snapshot::{
    audit_playlist, create_pre_mutation_snapshot, update_snapshot_added_tracks, AuditResult,
};
use crate::models::SourceTrack;
use crate::providers::traits::MusicProvider;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Notify, Semaphore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchTransferConfig {
    pub job_id: String,
    pub source_service: String,
    pub target_service: String,
    pub source_playlist_name: String,
    pub target_playlist_id: Option<String>,
    pub target_playlist_name: Option<String>,
    pub is_new_playlist: bool,
    pub tracks: Vec<SourceTrack>,
    pub concurrency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgressPayload {
    pub job_id: String,
    pub stage: String,
    pub processed: usize,
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub current_track: Option<String>,
    pub worker_id: usize,
    pub latency_ms: u64,
    pub http_status: u16,
    pub is_retry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferLogPayload {
    pub timestamp: i64,
    pub worker_id: usize,
    pub message: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchTransferSummary {
    pub job_id: String,
    pub snapshot_id: String,
    pub target_playlist_id: String,
    pub total_tracks: usize,
    pub successful_tracks: usize,
    pub failed_tracks: usize,
    pub is_cancelled: bool,
    pub audit: Option<AuditResult>,
}

#[derive(Clone)]
pub struct TransferControl {
    pub is_paused: Arc<AtomicBool>,
    pub is_cancelled: Arc<AtomicBool>,
    pub pause_notify: Arc<Notify>,
}

impl TransferControl {
    pub fn new() -> Self {
        Self {
            is_paused: Arc::new(AtomicBool::new(false)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            pause_notify: Arc::new(Notify::new()),
        }
    }

    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
        self.pause_notify.notify_waiters();
    }

    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
        self.pause_notify.notify_waiters();
    }
}

pub struct TransferWorkerPool {
    app: AppHandle,
    db: Arc<std::sync::Mutex<rusqlite::Connection>>,
}

impl TransferWorkerPool {
    pub fn new(app: AppHandle, db: Arc<std::sync::Mutex<rusqlite::Connection>>) -> Self {
        Self { app, db }
    }

    pub async fn execute_batch(
        &self,
        config: BatchTransferConfig,
        provider: Arc<Box<dyn MusicProvider>>,
        control: TransferControl,
    ) -> Result<BatchTransferSummary, String> {
        let total = config.tracks.len();
        let target_name = config
            .target_playlist_name
            .clone()
            .unwrap_or_else(|| config.source_playlist_name.clone());

        // Stage 1: Starting
        let start_payload = TransferProgressPayload {
            job_id: config.job_id.clone(),
            stage: "starting".to_string(),
            processed: 0,
            total,
            successful: 0,
            failed: 0,
            current_track: None,
            worker_id: 0,
            latency_ms: 0,
            http_status: 200,
            is_retry: false,
        };
        let _ = self.app.emit("transfer:progress", start_payload);

        let log = TransferLogPayload {
            timestamp: chrono::Utc::now().timestamp(),
            worker_id: 0,
            message: format!(
                "Starting transfer of {} tracks from {} to {}",
                total, config.source_service, config.target_service
            ),
            level: "info".to_string(),
        };
        let _ = self.app.emit("transfer:log", log);

        // Stage 2: Create playlist if new, or resolve target playlist ID
        let target_id = if config.is_new_playlist {
            let log = TransferLogPayload {
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: format!("Creating new playlist '{}' on destination", target_name),
                level: "info".to_string(),
            };
            let _ = self.app.emit("transfer:log", log);

            provider
                .create_playlist(&target_name, Some("Transferred with SoundShift"))
                .await?
        } else {
            config
                .target_playlist_id
                .clone()
                .ok_or_else(|| "Target playlist ID required for append mode".to_string())?
        };

        // Fetch pre-existing tracks for safety snapshot
        let pre_existing_tracks = provider.get_playlist_tracks(&target_id, None).await.unwrap_or_default();
        let pre_existing_ids: Vec<String> = pre_existing_tracks.into_iter().map(|t| t.id).collect();

        // Stage 3: Write immutable pre-mutation snapshot to SQLite (SAFE-01)
        let snapshot_id = {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            create_pre_mutation_snapshot(
                &conn,
                &config.job_id,
                &config.source_service,
                &config.target_service,
                &config.source_playlist_name,
                &target_id,
                &target_name,
                config.is_new_playlist,
                total as u32,
                total as u32,
                pre_existing_ids,
                Vec::new(),
            )?
        };

        let log = TransferLogPayload {
            timestamp: chrono::Utc::now().timestamp(),
            worker_id: 0,
            message: format!("Pre-mutation snapshot '{}' written to SQLite", snapshot_id),
            level: "info".to_string(),
        };
        let _ = self.app.emit("transfer:log", log);

        // Stage 4: Worker pool execution
        let concurrency = config.concurrency.clamp(1, 8);
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let processed_count = Arc::new(AtomicUsize::new(0));
        let successful_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));
        let added_ids = Arc::new(Mutex::new(Vec::new()));
        let worker_counter = Arc::new(AtomicUsize::new(0));

        let mut task_handles = Vec::with_capacity(total);

        for track in config.tracks {
            // Check cancellation before spawning
            if control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            // Check pause
            while control.is_paused.load(Ordering::SeqCst) {
                if control.is_cancelled.load(Ordering::SeqCst) {
                    break;
                }
                control.pause_notify.notified().await;
            }

            if control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            let permit = semaphore.clone().acquire_owned().await.map_err(|e| e.to_string())?;
            let track_clone = track.clone();
            let provider_clone = provider.clone();
            let target_id_clone = target_id.clone();
            let job_id_clone = config.job_id.clone();
            let app_handle = self.app.clone();
            let control_clone = control.clone();
            let processed_ref = processed_count.clone();
            let successful_ref = successful_count.clone();
            let failed_ref = failed_count.clone();
            let added_ids_ref = added_ids.clone();
            let worker_id = (worker_counter.fetch_add(1, Ordering::SeqCst) % concurrency) + 1;

            let handle = tokio::spawn(async move {
                let _permit = permit;

                // Wait if paused before executing mutation
                while control_clone.is_paused.load(Ordering::SeqCst) {
                    if control_clone.is_cancelled.load(Ordering::SeqCst) {
                        return;
                    }
                    control_clone.pause_notify.notified().await;
                }

                if control_clone.is_cancelled.load(Ordering::SeqCst) {
                    return;
                }

                let mut attempt = 0;
                let max_retries = 5;
                let base_delay_ms = 1500u64;

                loop {
                    if control_clone.is_cancelled.load(Ordering::SeqCst) {
                        return;
                    }

                    let start = std::time::Instant::now();
                    let res = provider_clone
                        .add_tracks_to_playlist(&target_id_clone, &[track_clone.id.clone()])
                        .await;
                    let latency_ms = start.elapsed().as_millis() as u64;

                    match res {
                        Ok(_) => {
                            let succ = successful_ref.fetch_add(1, Ordering::SeqCst) + 1;
                            let proc = processed_ref.fetch_add(1, Ordering::SeqCst) + 1;
                            let fail = failed_ref.load(Ordering::SeqCst);

                            if let Ok(mut lock) = added_ids_ref.lock() {
                                lock.push(track_clone.id.clone());
                            }

                            let progress = TransferProgressPayload {
                                job_id: job_id_clone.clone(),
                                stage: "transferring".to_string(),
                                processed: proc,
                                total,
                                successful: succ,
                                failed: fail,
                                current_track: Some(format!("{} - {}", track_clone.title, track_clone.artists.join(", "))),
                                worker_id,
                                latency_ms,
                                http_status: 200,
                                is_retry: attempt > 0,
                            };
                            let _ = app_handle.emit("transfer:progress", progress);

                            let log = TransferLogPayload {
                                timestamp: chrono::Utc::now().timestamp(),
                                worker_id,
                                message: format!("Added '{}' ({}ms)", track_clone.title, latency_ms),
                                level: "info".to_string(),
                            };
                            let _ = app_handle.emit("transfer:log", log);
                            break;
                        }
                        Err(err_msg) => {
                            let is_429 = err_msg.contains("429") || err_msg.to_lowercase().contains("rate limit");
                            if is_429 && attempt < max_retries {
                                let jitter = (chrono::Utc::now().timestamp_subsec_millis() as u64) % 400;
                                let delay = std::cmp::min(base_delay_ms * (1 << attempt) + jitter, 16000);
                                attempt += 1;

                                let log = TransferLogPayload {
                                    timestamp: chrono::Utc::now().timestamp(),
                                    worker_id,
                                    message: format!(
                                        "Rate limit 429 for '{}'. Retrying ({}/{}) in {}ms",
                                        track_clone.title, attempt, max_retries, delay
                                    ),
                                    level: "warn".to_string(),
                                };
                                let _ = app_handle.emit("transfer:log", log);

                                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                continue;
                            }

                            // Non-blocking skip after retries per D-05
                            let fail = failed_ref.fetch_add(1, Ordering::SeqCst) + 1;
                            let proc = processed_ref.fetch_add(1, Ordering::SeqCst) + 1;
                            let succ = successful_ref.load(Ordering::SeqCst);

                            let progress = TransferProgressPayload {
                                job_id: job_id_clone.clone(),
                                stage: "transferring".to_string(),
                                processed: proc,
                                total,
                                successful: succ,
                                failed: fail,
                                current_track: Some(format!("{} - {}", track_clone.title, track_clone.artists.join(", "))),
                                worker_id,
                                latency_ms,
                                http_status: if is_429 { 429 } else { 500 },
                                is_retry: attempt > 0,
                            };
                            let _ = app_handle.emit("transfer:progress", progress);

                            let log = TransferLogPayload {
                                timestamp: chrono::Utc::now().timestamp(),
                                worker_id,
                                message: format!("Failed to add '{}': {}", track_clone.title, err_msg),
                                level: "error".to_string(),
                            };
                            let _ = app_handle.emit("transfer:log", log);
                            break;
                        }
                    }
                }
            });

            task_handles.push(handle);
        }

        // Await all spawned worker tasks
        for handle in task_handles {
            let _ = handle.await;
        }

        let is_cancelled = control.is_cancelled.load(Ordering::SeqCst);
        let final_added_ids = added_ids.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let total_successful = successful_count.load(Ordering::SeqCst);
        let total_failed = failed_count.load(Ordering::SeqCst);

        // Update snapshot in SQLite with actually added tracks
        {
            if let Ok(conn) = self.db.lock() {
                let _ = update_snapshot_added_tracks(&conn, &snapshot_id, &final_added_ids);
            }
        }

        if is_cancelled {
            let progress = TransferProgressPayload {
                job_id: config.job_id.clone(),
                stage: "cancelled".to_string(),
                processed: processed_count.load(Ordering::SeqCst),
                total,
                successful: total_successful,
                failed: total_failed,
                current_track: None,
                worker_id: 0,
                latency_ms: 0,
                http_status: 200,
                is_retry: false,
            };
            let _ = self.app.emit("transfer:progress", progress);

            let log = TransferLogPayload {
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: "Transfer cancelled by user. Added tracks preserved in snapshot.".to_string(),
                level: "warn".to_string(),
            };
            let _ = self.app.emit("transfer:log", log);

            return Ok(BatchTransferSummary {
                job_id: config.job_id,
                snapshot_id,
                target_playlist_id: target_id,
                total_tracks: total,
                successful_tracks: total_successful,
                failed_tracks: total_failed,
                is_cancelled: true,
                audit: None,
            });
        }

        // Stage 5: Automated integrity audit (SAFE-04, D-08)
        let log = TransferLogPayload {
            timestamp: chrono::Utc::now().timestamp(),
            worker_id: 0,
            message: "Running post-transfer integrity audit...".to_string(),
            level: "info".to_string(),
        };
        let _ = self.app.emit("transfer:log", log);

        let audit_result = audit_playlist(provider.as_ref().as_ref(), &target_id, &final_added_ids)
            .await
            .ok();

        if let Some(ref audit) = audit_result {
            let _ = self.app.emit("transfer:audit", audit.clone());

            let log = TransferLogPayload {
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: format!(
                    "Integrity audit complete: verified={} (found {}/{})",
                    audit.is_verified, audit.total_found, audit.total_expected
                ),
                level: if audit.is_verified { "info".to_string() } else { "warn".to_string() },
            };
            let _ = self.app.emit("transfer:log", log);
        }

        // Completion event
        let complete_payload = TransferProgressPayload {
            job_id: config.job_id.clone(),
            stage: "completed".to_string(),
            processed: total,
            total,
            successful: total_successful,
            failed: total_failed,
            current_track: None,
            worker_id: 0,
            latency_ms: 0,
            http_status: 200,
            is_retry: false,
        };
        let _ = self.app.emit("transfer:progress", complete_payload);

        Ok(BatchTransferSummary {
            job_id: config.job_id,
            snapshot_id,
            target_playlist_id: target_id,
            total_tracks: total,
            successful_tracks: total_successful,
            failed_tracks: total_failed,
            is_cancelled: false,
            audit: audit_result,
        })
    }
}
