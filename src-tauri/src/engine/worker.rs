use crate::engine::lifecycle::JobState;
pub use crate::engine::lifecycle::TransferControl;
use crate::engine::snapshot::{
    audit_playlist, create_pre_mutation_snapshot, record_operation_intent,
    update_operation_outcome, AuditResult, OperationOutcome,
};
use crate::models::SourceTrack;
use crate::providers::error::{classify_error, retry_delay_ms, ProviderErrorKind};
use crate::providers::traits::MusicProvider;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

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
    #[serde(default)]
    pub skip_duplicates: bool,
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
    pub job_id: String,
    pub timestamp: i64,
    pub worker_id: usize,
    pub message: String,
    pub level: String,
}

/// All transfer log sinks receive a sanitized message, including test sinks.
pub fn dispatch_transfer_log(
    mut payload: TransferLogPayload,
    sink: impl FnOnce(TransferLogPayload),
) {
    payload.message = crate::diagnostics::sanitize_for_log(&payload.message);
    sink(payload);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchTransferSummary {
    pub job_id: String,
    pub snapshot_id: String,
    pub target_playlist_id: String,
    pub total_tracks: usize,
    pub successful_tracks: usize,
    pub failed_tracks: usize,
    #[serde(default)]
    pub skipped_duplicates: usize,
    pub is_cancelled: bool,
    pub audit: Option<AuditResult>,
    pub state: String,
    pub event_seq: u64,
}

pub fn resolve_concurrency(target_service: &str, requested_concurrency: usize) -> usize {
    if target_service == "ytmusic" {
        1
    } else {
        requested_concurrency.clamp(1, 8)
    }
}

fn strip_parentheticals(s: &str) -> String {
    let mut result = String::new();
    let mut in_paren = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => in_paren += 1,
            ')' | ']' => {
                if in_paren > 0 {
                    in_paren -= 1;
                }
            }
            _ if in_paren == 0 => result.push(c),
            _ => {}
        }
    }
    result.trim().to_string()
}

pub fn is_duplicate_track(track: &SourceTrack, existing_tracks: &[SourceTrack]) -> bool {
    let clean_id = track.id.trim().trim_start_matches("spotify:track:");

    for existing in existing_tracks {
        let existing_clean_id = existing.id.trim().trim_start_matches("spotify:track:");
        // 1. Exact ID equality
        if !clean_id.is_empty() && clean_id == existing_clean_id {
            return true;
        }

        // 2. ISRC match (if both provide valid ISRCs)
        if let (Some(ref isrc_a), Some(ref isrc_b)) = (&track.isrc, &existing.isrc) {
            let clean_a = isrc_a.trim();
            let clean_b = isrc_b.trim();
            if !clean_a.is_empty() && !clean_b.is_empty() && clean_a.eq_ignore_ascii_case(clean_b) {
                return true;
            }
        }

        // 3. Normalized title & artist overlap match
        let (norm_title_a, _) = crate::engine::normalizer::normalize_title(&track.title);
        let (norm_title_b, _) = crate::engine::normalizer::normalize_title(&existing.title);

        let title_matches = if norm_title_a == norm_title_b && !norm_title_a.is_empty() {
            true
        } else {
            let base_a = strip_parentheticals(&norm_title_a);
            let base_b = strip_parentheticals(&norm_title_b);
            let is_live_a = norm_title_a.contains("live");
            let is_live_b = norm_title_b.contains("live");
            let is_acoustic_a = norm_title_a.contains("acoustic");
            let is_acoustic_b = norm_title_b.contains("acoustic");

            !base_a.is_empty()
                && base_a == base_b
                && is_live_a == is_live_b
                && is_acoustic_a == is_acoustic_b
        };

        if title_matches {
            let artists_a: Vec<String> = track
                .artists
                .iter()
                .map(|a| crate::engine::normalizer::canonicalize_string(a))
                .filter(|a| !a.is_empty())
                .collect();
            let artists_b: Vec<String> = existing
                .artists
                .iter()
                .map(|a| crate::engine::normalizer::canonicalize_string(a))
                .filter(|a| !a.is_empty())
                .collect();

            if artists_a.is_empty() && artists_b.is_empty() {
                return true;
            }

            let has_artist_overlap = artists_a.iter().any(|a| artists_b.contains(a));
            if has_artist_overlap {
                return true;
            }
        }
    }

    false
}

fn consume_retry_budget(budget: &AtomicU32) -> bool {
    let mut current = budget.load(Ordering::SeqCst);
    loop {
        if current == 0 {
            return false;
        }
        match budget.compare_exchange_weak(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

struct ChunkWorkerContext {
    job_id: String,
    target_id: String,
    total: usize,
    chunk_idx: usize,
    worker_id: usize,
    chunk: Vec<SourceTrack>,
    provider: Arc<Box<dyn MusicProvider>>,
    db: Arc<std::sync::Mutex<rusqlite::Connection>>,
    app: AppHandle,
    control: TransferControl,
    processed_count: Arc<AtomicUsize>,
    successful_count: Arc<AtomicUsize>,
    failed_count: Arc<AtomicUsize>,
    retry_budget: Arc<AtomicU32>,
    added_ids: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Persist intent before the send and a checkpoint before reporting success.
/// Transport/partial outcomes are never replayed automatically. Presence in a
/// playlist cannot establish whether this operation added a new occurrence.
#[allow(clippy::too_many_arguments)]
async fn journaled_add(
    db: &std::sync::Mutex<rusqlite::Connection>,
    provider: &dyn MusicProvider,
    job_id: &str,
    target_id: &str,
    operation_index: usize,
    ids: &[String],
    retry_budget: &AtomicU32,
    control: &TransferControl,
) -> Result<bool, String> {
    let operation_id = {
        let conn = db
            .lock()
            .map_err(|_| "Database lock unavailable; no mutation sent")?;
        record_operation_intent(&conn, job_id, operation_index, ids)?
    };
    #[cfg(test)]
    safety_tests::crash_point("intent");
    for attempt in 0..=5 {
        let Some(permit) = control.reserve_mutation().await? else {
            let conn = db
                .lock()
                .map_err(|_| "Cancelled mutation checkpoint unavailable")?;
            update_operation_outcome(
                &conn,
                &operation_id,
                OperationOutcome::Failed,
                attempt,
                Some("cancelled before send"),
            )?;
            return Ok(false);
        };
        let response = provider.add_tracks_to_playlist(target_id, ids).await;
        #[cfg(test)]
        safety_tests::crash_point("response");
        match response {
            Ok(()) => {
                {
                    let conn = db.lock().map_err(|_| {
                        "Remote write accepted but checkpoint unavailable; recovery required"
                    })?;
                    update_operation_outcome(
                        &conn,
                        &operation_id,
                        OperationOutcome::Confirmed,
                        attempt + 1,
                        Some("accepted"),
                    )
                    .map_err(|_| {
                        "Remote write accepted but checkpoint failed; recovery required".to_string()
                    })?;
                }
                #[cfg(test)]
                safety_tests::crash_point("checkpoint");
                permit.checkpointed()?;
                return Ok(true);
            }
            Err(error) => {
                let kind = classify_error(&error);
                let rejected = matches!(
                    kind,
                    ProviderErrorKind::RateLimited
                        | ProviderErrorKind::Authentication
                        | ProviderErrorKind::Permission
                );
                {
                    let conn = db
                        .lock()
                        .map_err(|_| "Mutation outcome could not be saved; recovery required")?;
                    update_operation_outcome(
                        &conn,
                        &operation_id,
                        if rejected {
                            OperationOutcome::Failed
                        } else {
                            OperationOutcome::Ambiguous
                        },
                        attempt + 1,
                        Some(if rejected {
                            "provider rejected mutation"
                        } else {
                            "remote outcome uncertain"
                        }),
                    )?;
                }
                if !rejected {
                    control.fail("Remote mutation outcome is uncertain. Automatic replay stopped; manual reconciliation required.")?;
                }
                permit.checkpointed()?;
                if kind == ProviderErrorKind::RateLimited
                    && attempt < 5
                    && consume_retry_budget(retry_budget)
                {
                    control
                        .interruptible_delay(std::time::Duration::from_millis(retry_delay_ms(
                            attempt, None,
                        )))
                        .await;
                    continue;
                }
                return Err(if rejected {
                    "Provider rejected the mutation. Confirmed earlier writes are saved; review history before retrying.".into()
                } else {
                    "Remote mutation outcome is uncertain. Automatic replay stopped; manual reconciliation is required.".into()
                });
            }
        }
    }
    unreachable!("bounded retry loop always returns")
}

async fn process_chunk(ctx: ChunkWorkerContext) -> Result<(), String> {
    ctx.control.wait_until_ready().await?;
    if ctx.control.is_stopping() {
        return Ok(());
    }
    let ids: Vec<String> = ctx.chunk.iter().map(|track| track.id.clone()).collect();
    let start = std::time::Instant::now();
    match journaled_add(
        &ctx.db,
        ctx.provider.as_ref().as_ref(),
        &ctx.job_id,
        &ctx.target_id,
        ctx.chunk_idx,
        &ids,
        &ctx.retry_budget,
        &ctx.control,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(error) => {
            let _ = ctx.control.fail(&error);
            ctx.failed_count
                .fetch_add(ctx.chunk.len(), Ordering::SeqCst);
            return Err(error);
        }
    }
    let latency_ms = (start.elapsed().as_millis() as u64 / ctx.chunk.len().max(1) as u64).max(1);
    ctx.added_ids
        .lock()
        .map_err(|_| "Accepted tracks checkpointed, but progress lock failed")?
        .extend(ids);
    for track in &ctx.chunk {
        let successful = ctx.successful_count.fetch_add(1, Ordering::SeqCst) + 1;
        let processed = ctx.processed_count.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = ctx.app.emit(
            "transfer:progress",
            TransferProgressPayload {
                job_id: ctx.job_id.clone(),
                stage: "transferring".into(),
                processed,
                total: ctx.total,
                successful,
                failed: ctx.failed_count.load(Ordering::SeqCst),
                current_track: Some(format!("{} - {}", track.title, track.artists.join(", "))),
                worker_id: ctx.worker_id,
                latency_ms,
                http_status: 200,
                is_retry: false,
            },
        );
    }
    Ok(())
}

/// Read a trusted baseline, then commit recovery identity before creating remotely.
async fn prepare_transfer(
    db: &std::sync::Mutex<rusqlite::Connection>,
    provider: &dyn MusicProvider,
    config: &BatchTransferConfig,
    control: &TransferControl,
) -> Result<(String, String, Vec<SourceTrack>), String> {
    let total = config.tracks.len();
    let target_name = config
        .target_playlist_name
        .clone()
        .unwrap_or_else(|| config.source_playlist_name.clone());
    let initial_target = if config.is_new_playlist {
        String::new()
    } else {
        config
            .target_playlist_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .ok_or("Target playlist ID required for append mode")?
    };
    let pre_existing_tracks = if config.is_new_playlist {
        Vec::new()
    } else {
        provider
            .get_playlist_tracks(&initial_target, None)
            .await
            .map_err(|_| "Destination baseline read failed. No mutation was sent.".to_string())?
    };
    let pre_existing_ids: Vec<String> = pre_existing_tracks.iter().map(|t| t.id.clone()).collect();
    let snapshot_id = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        create_pre_mutation_snapshot(
            &conn,
            &config.job_id,
            &config.source_service,
            &config.target_service,
            &config.source_playlist_name,
            &initial_target,
            &target_name,
            config.is_new_playlist,
            total as u32,
            total as u32,
            pre_existing_ids.clone(),
            Vec::new(),
        )?
    };
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let manifest = serde_json::to_string(&config).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE transfer_jobs SET manifest_json = ?1 WHERE job_id = ?2",
            rusqlite::params![manifest, config.job_id],
        )
        .map_err(|e| e.to_string())?;
    }
    control.start()?;
    let target_id = if config.is_new_playlist {
        let operation_id = {
            let conn = db.lock().map_err(|e| e.to_string())?;
            let id = record_operation_intent(
                &conn,
                &config.job_id,
                usize::MAX / 2,
                std::slice::from_ref(&target_name),
            )?;
            conn.execute("UPDATE transfer_operations SET operation_type = 'create_playlist' WHERE operation_id = ?1", [&id])
                    .map_err(|e| e.to_string())?;
            id
        };
        let Some(permit) = control.reserve_mutation().await? else {
            let conn = db.lock().map_err(|e| e.to_string())?;
            update_operation_outcome(
                &conn,
                &operation_id,
                OperationOutcome::Failed,
                0,
                Some("cancelled before create"),
            )?;
            return Ok((snapshot_id, initial_target, pre_existing_tracks));
        };
        // No automatic create retry: a lost response can mean a playlist exists.
        let id = provider
            .create_playlist(&target_name, Some("Transferred with SoundShift"))
            .await
            .map_err(|_| {
                "Playlist creation outcome uncertain. Review the provider before retrying."
                    .to_string()
            })?;
        if id.trim().is_empty() {
            return Err("Provider returned an empty playlist identity; recovery required".into());
        }
        {
            let conn = db.lock().map_err(|e| e.to_string())?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE transfer_jobs SET target_playlist_id = ?1 WHERE job_id = ?2",
                rusqlite::params![id, config.job_id],
            )
            .map_err(|e| e.to_string())?;
            update_operation_outcome(
                &tx,
                &operation_id,
                OperationOutcome::Confirmed,
                1,
                Some("created"),
            )?;
            tx.commit().map_err(|_| {
                "Playlist created, but identity checkpoint failed; recovery required"
            })?;
        }
        permit.checkpointed()?;
        id
    } else {
        initial_target
    };

    Ok((snapshot_id, target_id, pre_existing_tracks))
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
            job_id: config.job_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
            worker_id: 0,
            message: format!(
                "Starting transfer of {} tracks from {} to {}",
                total, config.source_service, config.target_service
            ),
            level: "info".to_string(),
        };
        dispatch_transfer_log(log, |payload| {
            let _ = self.app.emit("transfer:log", payload);
        });

        let (snapshot_id, target_id, pre_existing_tracks) =
            prepare_transfer(&self.db, provider.as_ref().as_ref(), &config, &control).await?;
        let pre_existing_ids: Vec<String> = pre_existing_tracks
            .iter()
            .map(|track| track.id.clone())
            .collect();

        // Deduplication phase for appending to existing playlists
        let (tracks_to_transfer, skipped_duplicates) = if !config.is_new_playlist
            && config.skip_duplicates
            && !pre_existing_tracks.is_empty()
        {
            let mut deduplicated = Vec::new();
            let mut skipped = 0usize;
            let mut existing_pool = pre_existing_tracks.clone();

            for track in config.tracks {
                if is_duplicate_track(&track, &existing_pool) {
                    skipped += 1;
                    let log = TransferLogPayload {
                        job_id: config.job_id.clone(),
                        timestamp: chrono::Utc::now().timestamp(),
                        worker_id: 0,
                        message: format!(
                            "Skipped '{}' (already in destination playlist)",
                            track.title
                        ),
                        level: "info".to_string(),
                    };
                    dispatch_transfer_log(log, |payload| {
                        let _ = self.app.emit("transfer:log", payload);
                    });
                } else {
                    existing_pool.push(track.clone());
                    deduplicated.push(track);
                }
            }

            if skipped > 0 {
                let log = TransferLogPayload {
                job_id: config.job_id.clone(),
                    timestamp: chrono::Utc::now().timestamp(),
                    worker_id: 0,
                    message: format!(
                        "Deduplication complete: skipped {} duplicate track(s). Transferring {} remaining track(s).",
                        skipped,
                        deduplicated.len()
                    ),
                    level: "info".to_string(),
                };
                dispatch_transfer_log(log, |payload| {
                    let _ = self.app.emit("transfer:log", payload);
                });
            }

            (deduplicated, skipped)
        } else {
            (config.tracks, 0usize)
        };

        // Stage 4: Chunked batch execution with true Tokio concurrency and retry
        let concurrency = resolve_concurrency(&config.target_service, config.concurrency);
        if config.target_service == "ytmusic" && config.concurrency > 1 {
            let log = TransferLogPayload {
                job_id: config.job_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: format!(
                    "YouTube Music target service enforces single-writer playlist mutations. Concurrency serialized to 1 worker (requested: {}) to prevent 409 Conflict collisions.",
                    config.concurrency
                ),
                level: "info".to_string(),
            };
            dispatch_transfer_log(log, |payload| {
                let _ = self.app.emit("transfer:log", payload);
            });
        }

        let processed_count = Arc::new(AtomicUsize::new(skipped_duplicates));
        let successful_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));
        let retry_budget = Arc::new(AtomicU32::new(20));
        let added_ids = Arc::new(std::sync::Mutex::new(Vec::with_capacity(
            tracks_to_transfer.len(),
        )));

        // YouTube Music silently truncates multi-action edit_playlist requests
        // after roughly 25 actions. Use one action per request there so an HTTP
        // success cannot leave the remainder of a batch unpersisted.
        let chunk_size = if config.target_service == "ytmusic" {
            1
        } else {
            25
        };
        let chunks: Vec<Vec<SourceTrack>> = tracks_to_transfer
            .chunks(chunk_size)
            .map(|c| c.to_vec())
            .collect();

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut join_set = tokio::task::JoinSet::new();
        let errors = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
            if control.is_stopping() {
                break;
            }

            control.wait_until_ready().await?;
            if control.is_stopping() {
                break;
            }

            let sem_permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };

            if control.is_stopping() {
                drop(sem_permit);
                break;
            }

            let worker_id = (chunk_idx % concurrency) + 1;
            let ctx = ChunkWorkerContext {
                job_id: config.job_id.clone(),
                target_id: target_id.clone(),
                total,
                chunk_idx,
                worker_id,
                chunk,
                provider: provider.clone(),
                db: self.db.clone(),
                app: self.app.clone(),
                control: control.clone(),
                processed_count: processed_count.clone(),
                successful_count: successful_count.clone(),
                failed_count: failed_count.clone(),
                retry_budget: retry_budget.clone(),
                added_ids: added_ids.clone(),
            };

            let errors = errors.clone();
            join_set.spawn(async move {
                let _permit = sem_permit;
                if let Err(error) = process_chunk(ctx).await {
                    errors.lock().unwrap().push(error);
                }
            });
        }

        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                let _ = control.fail("Worker task interrupted before completion");
                errors
                    .lock()
                    .map_err(|e| e.to_string())?
                    .push(format!("Worker interrupted: {}; recovery required", e));
            }
        }

        control.wait_until_ready().await?;
        let is_cancelled = control.is_stopping();
        let final_added_ids = {
            let ids = added_ids.lock().unwrap();
            ids.clone()
        };
        let total_successful = successful_count.load(Ordering::SeqCst);
        let total_failed = failed_count.load(Ordering::SeqCst);
        let total_processed = processed_count.load(Ordering::SeqCst);

        if let Some(error) = errors.lock().map_err(|e| e.to_string())?.first() {
            return Err(error.clone());
        }

        if is_cancelled {
            let final_state = control.finish().await?;
            let progress = TransferProgressPayload {
                job_id: config.job_id.clone(),
                stage: "cancelled".to_string(),
                processed: total_processed,
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
                job_id: config.job_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: "Transfer cancelled by user. Added tracks preserved in snapshot."
                    .to_string(),
                level: "warn".to_string(),
            };
            dispatch_transfer_log(log, |payload| {
                let _ = self.app.emit("transfer:log", payload);
            });

            return Ok(BatchTransferSummary {
                job_id: config.job_id,
                snapshot_id,
                target_playlist_id: target_id,
                total_tracks: total,
                successful_tracks: total_successful,
                failed_tracks: total_failed,
                skipped_duplicates,
                is_cancelled: true,
                audit: None,
                state: final_state.state.as_str().into(),
                event_seq: final_state.event_seq,
            });
        }

        // Stage 5: Automated integrity audit (SAFE-04, D-08)
        let log = TransferLogPayload {
            job_id: config.job_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
            worker_id: 0,
            message: "Running post-transfer integrity audit...".to_string(),
            level: "info".to_string(),
        };
        dispatch_transfer_log(log, |payload| {
            let _ = self.app.emit("transfer:log", payload);
        });

        let mut expected_ids = pre_existing_ids;
        expected_ids.extend(final_added_ids);
        let audit = audit_playlist(provider.as_ref().as_ref(), &target_id, &expected_ids)
            .await
            .map_err(|_| {
                "Transfer writes checkpointed, but verification read failed; recovery required"
                    .to_string()
            })?;
        if !audit.is_verified {
            return Err(format!("Transfer verification found {} missing occurrence(s); manual reconciliation required", audit.missing_ids.len()));
        }
        let audit_result = Some(audit);
        let final_state = control.finish().await?;

        if let Some(ref audit) = audit_result {
            let _ = self.app.emit(
                "transfer:audit",
                serde_json::json!({ "job_id": config.job_id, "audit": audit }),
            );

            let log = TransferLogPayload {
                job_id: config.job_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: format!(
                    "Integrity audit complete: verified={} (found {}/{})",
                    audit.is_verified, audit.total_found, audit.total_expected
                ),
                level: if audit.is_verified {
                    "info".to_string()
                } else {
                    "warn".to_string()
                },
            };
            dispatch_transfer_log(log, |payload| {
                let _ = self.app.emit("transfer:log", payload);
            });
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
            skipped_duplicates,
            is_cancelled: final_state.state == JobState::Cancelled,
            audit: audit_result,
            state: final_state.state.as_str().into(),
            event_seq: final_state.event_seq,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_concurrency_serializes_ytmusic() {
        assert_eq!(resolve_concurrency("ytmusic", 8), 1);
        assert_eq!(resolve_concurrency("ytmusic", 4), 1);
        assert_eq!(resolve_concurrency("ytmusic", 1), 1);
    }

    #[test]
    fn test_resolve_concurrency_preserves_other_services() {
        assert_eq!(resolve_concurrency("spotify", 8), 8);
        assert_eq!(resolve_concurrency("spotify", 4), 4);
        assert_eq!(resolve_concurrency("spotify", 0), 1);
        assert_eq!(resolve_concurrency("spotify", 16), 8);
    }

    fn sample_track(id: &str, title: &str, artists: Vec<&str>, isrc: Option<&str>) -> SourceTrack {
        SourceTrack {
            id: id.to_string(),
            title: title.to_string(),
            artists: artists.into_iter().map(|s| s.to_string()).collect(),
            album: None,
            duration_ms: 210000,
            isrc: isrc.map(|s| s.to_string()),
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        }
    }

    #[test]
    fn test_is_duplicate_track_by_exact_and_prefixed_id() {
        let existing = vec![sample_track(
            "4cOdK2wGLETKBW3PvgPWqT",
            "Song A",
            vec!["Artist 1"],
            None,
        )];

        let incoming_exact = sample_track(
            "4cOdK2wGLETKBW3PvgPWqT",
            "Different Title",
            vec!["Diff Artist"],
            None,
        );
        assert!(is_duplicate_track(&incoming_exact, &existing));

        let incoming_prefixed = sample_track(
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "Different",
            vec!["Diff"],
            None,
        );
        assert!(is_duplicate_track(&incoming_prefixed, &existing));
    }

    #[test]
    fn test_is_duplicate_track_by_isrc() {
        let existing = vec![sample_track(
            "id_1",
            "Song Title",
            vec!["Artist"],
            Some("USUM71703861"),
        )];

        let incoming = sample_track(
            "id_2",
            "Song Title (Radio Edit)",
            vec!["Different Artist Label"],
            Some("usum71703861"),
        );
        assert!(is_duplicate_track(&incoming, &existing));
    }

    #[test]
    fn test_is_duplicate_track_by_normalized_title_and_artist_overlap() {
        let existing = vec![sample_track(
            "id_1",
            "Deva Deva",
            vec!["Pritam", "Arijit Singh"],
            None,
        )];

        // Reversed artists or subsets still overlap on primary contributors
        let incoming_reversed = sample_track(
            "id_2",
            "Deva Deva (Film Version)",
            vec!["Arijit Singh", "Pritam"],
            None,
        );
        assert!(is_duplicate_track(&incoming_reversed, &existing));

        // Remaster stripping
        let incoming_remaster =
            sample_track("id_3", "Deva Deva - 2024 Remaster", vec!["Pritam"], None);
        assert!(is_duplicate_track(&incoming_remaster, &existing));
    }

    #[test]
    fn test_is_not_duplicate_when_artists_differ() {
        let existing = vec![sample_track("id_1", "Hello", vec!["Adele"], None)];

        let incoming = sample_track("id_2", "Hello", vec!["Lionel Richie"], None);
        assert!(!is_duplicate_track(&incoming, &existing));
    }

    #[test]
    fn test_consume_retry_budget() {
        let budget = AtomicU32::new(2);
        assert!(consume_retry_budget(&budget));
        assert_eq!(budget.load(Ordering::SeqCst), 1);
        assert!(consume_retry_budget(&budget));
        assert_eq!(budget.load(Ordering::SeqCst), 0);
        assert!(!consume_retry_budget(&budget));
        assert_eq!(budget.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
#[path = "worker_safety_tests.rs"]
mod safety_tests;
