use crate::engine::snapshot::{
    audit_playlist, create_pre_mutation_snapshot, record_operation_intent,
    update_operation_outcome, update_snapshot_added_tracks, AuditResult, OperationOutcome,
};
use crate::models::SourceTrack;
use crate::providers::error::{
    classify_error, is_retryable, retry_delay_ms, ProviderErrorKind, ReconciliationResult,
};
use crate::providers::traits::MusicProvider;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

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
    #[serde(default)]
    pub skipped_duplicates: usize,
    pub is_cancelled: bool,
    pub audit: Option<AuditResult>,
}

#[derive(Clone)]
pub struct TransferControl {
    pub is_paused: Arc<AtomicBool>,
    pub is_cancelled: Arc<AtomicBool>,
    pub pause_notify: Arc<Notify>,
}

impl Default for TransferControl {
    fn default() -> Self {
        Self::new()
    }
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
        match budget.compare_exchange_weak(
            current,
            current - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
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
    base_delay_ms: u64,
    max_retries: u32,
    chunk_size: usize,
}

async fn process_chunk(ctx: ChunkWorkerContext) {
    if ctx.control.is_cancelled.load(Ordering::SeqCst) {
        return;
    }

    while ctx.control.is_paused.load(Ordering::SeqCst) {
        if ctx.control.is_cancelled.load(Ordering::SeqCst) {
            return;
        }
        ctx.control.pause_notify.notified().await;
    }

    if ctx.control.is_cancelled.load(Ordering::SeqCst) {
        return;
    }

    let chunk_track_ids: Vec<String> = ctx.chunk.iter().map(|t| t.id.clone()).collect();
    let operation_id = match ctx.db.lock() {
        Ok(conn) => {
            match record_operation_intent(&conn, &ctx.job_id, ctx.chunk_idx, &chunk_track_ids) {
                Ok(op_id) => op_id,
                Err(e) => {
                    eprintln!("Failed to record operation intent: {}", e);
                    format!("op_chunk_{}", ctx.chunk_idx)
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to acquire db lock for operation intent: {}", e);
            format!("op_chunk_{}", ctx.chunk_idx)
        }
    };

    let mut attempt = 0;
    let mut chunk_succeeded = false;
    let mut chunk_ambiguous = false;

    // Step 4a: Atomic batch addition
    loop {
        if ctx.control.is_cancelled.load(Ordering::SeqCst) {
            break;
        }

        while ctx.control.is_paused.load(Ordering::SeqCst) {
            if ctx.control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }
            ctx.control.pause_notify.notified().await;
        }

        if ctx.control.is_cancelled.load(Ordering::SeqCst) {
            break;
        }

        let start = std::time::Instant::now();
        let res = ctx
            .provider
            .add_tracks_to_playlist(&ctx.target_id, &chunk_track_ids)
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match res {
            Ok(_) => {
                if let Ok(conn) = ctx.db.lock() {
                    let _ = update_operation_outcome(
                        &conn,
                        &operation_id,
                        OperationOutcome::Confirmed,
                        attempt + 1,
                        Some("accepted"),
                    );
                }
                chunk_succeeded = true;
                let per_track_latency = (latency_ms / (ctx.chunk.len() as u64).max(1)).max(1);

                let mut chunk_added_ids = Vec::with_capacity(ctx.chunk.len());
                for track in &ctx.chunk {
                    let succ = ctx.successful_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let proc = ctx.processed_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let fail = ctx.failed_count.load(Ordering::SeqCst);
                    chunk_added_ids.push(track.id.clone());

                    let progress = TransferProgressPayload {
                        job_id: ctx.job_id.clone(),
                        stage: "transferring".to_string(),
                        processed: proc,
                        total: ctx.total,
                        successful: succ,
                        failed: fail,
                        current_track: Some(format!(
                            "{} - {}",
                            track.title,
                            track.artists.join(", ")
                        )),
                        worker_id: ctx.worker_id,
                        latency_ms: per_track_latency,
                        http_status: 200,
                        is_retry: attempt > 0,
                    };
                    let _ = ctx.app.emit("transfer:progress", progress);

                    let log = TransferLogPayload {
                        timestamp: chrono::Utc::now().timestamp(),
                        worker_id: ctx.worker_id,
                        message: format!("Added '{}' ({}ms)", track.title, per_track_latency),
                        level: "info".to_string(),
                    };
                    let _ = ctx.app.emit("transfer:log", log);
                }

                if let Ok(mut ids) = ctx.added_ids.lock() {
                    ids.extend(chunk_added_ids);
                }
                break;
            }
            Err(err_msg) => {
                let kind = classify_error(&err_msg);

                if kind == ProviderErrorKind::Timeout {
                    if let Ok(reconciled) = ctx
                        .provider
                        .reconcile_added_tracks(&ctx.target_id, &chunk_track_ids)
                        .await
                    {
                        match reconciled {
                            ReconciliationResult::Confirmed => {
                                if let Ok(conn) = ctx.db.lock() {
                                    let _ = update_operation_outcome(
                                        &conn,
                                        &operation_id,
                                        OperationOutcome::Confirmed,
                                        attempt + 1,
                                        Some("confirmed by read-back"),
                                    );
                                }
                                chunk_succeeded = true;
                                let mut chunk_added_ids = Vec::with_capacity(ctx.chunk.len());
                                for track in &ctx.chunk {
                                    ctx.successful_count.fetch_add(1, Ordering::SeqCst);
                                    ctx.processed_count.fetch_add(1, Ordering::SeqCst);
                                    chunk_added_ids.push(track.id.clone());
                                }
                                if let Ok(mut ids) = ctx.added_ids.lock() {
                                    ids.extend(chunk_added_ids);
                                }
                                break;
                            }
                            ReconciliationResult::Uncertain => {
                                if let Ok(conn) = ctx.db.lock() {
                                    let _ = update_operation_outcome(
                                        &conn,
                                        &operation_id,
                                        OperationOutcome::Ambiguous,
                                        attempt + 1,
                                        Some(&err_msg),
                                    );
                                }
                                chunk_ambiguous = true;
                                break;
                            }
                            ReconciliationResult::NotApplied => {}
                        }
                    } else {
                        if let Ok(conn) = ctx.db.lock() {
                            let _ = update_operation_outcome(
                                &conn,
                                &operation_id,
                                OperationOutcome::Ambiguous,
                                attempt + 1,
                                Some(&err_msg),
                            );
                        }
                        chunk_ambiguous = true;
                        break;
                    }
                }

                let has_budget = consume_retry_budget(&ctx.retry_budget);

                if is_retryable(kind) && attempt < ctx.max_retries && has_budget {
                    let delay = retry_delay_ms(attempt, None).max(ctx.base_delay_ms);
                    attempt += 1;

                    let log = TransferLogPayload {
                        timestamp: chrono::Utc::now().timestamp(),
                        worker_id: ctx.worker_id,
                        message: format!(
                            "Transient error ({}) for batch {}. Retrying ({}/{}) in {}ms",
                            err_msg,
                            ctx.chunk_idx + 1,
                            attempt,
                            ctx.max_retries,
                            delay
                        ),
                        level: "warn".to_string(),
                    };
                    let _ = ctx.app.emit("transfer:log", log);

                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }

                if let Ok(conn) = ctx.db.lock() {
                    let outcome = if kind == ProviderErrorKind::Timeout {
                        OperationOutcome::Ambiguous
                    } else {
                        OperationOutcome::Failed
                    };
                    let _ = update_operation_outcome(
                        &conn,
                        &operation_id,
                        outcome,
                        attempt + 1,
                        Some(&err_msg),
                    );
                }

                let log = TransferLogPayload {
                    timestamp: chrono::Utc::now().timestamp(),
                    worker_id: ctx.worker_id,
                    message: format!(
                        "Batch {} failed ({}). Falling back to individual track insertion.",
                        ctx.chunk_idx + 1,
                        err_msg
                    ),
                    level: "warn".to_string(),
                };
                let _ = ctx.app.emit("transfer:log", log);
                break;
            }
        }
    }

    // Step 4b: Fallback to individual track addition
    if !chunk_succeeded && !chunk_ambiguous && !ctx.control.is_cancelled.load(Ordering::SeqCst) {
        for (t_idx, track) in ctx.chunk.into_iter().enumerate() {
            if ctx.control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            while ctx.control.is_paused.load(Ordering::SeqCst) {
                if ctx.control.is_cancelled.load(Ordering::SeqCst) {
                    break;
                }
                ctx.control.pause_notify.notified().await;
            }

            if ctx.control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            let mut track_attempt = 0;
            let track_seq = ctx.chunk_idx * ctx.chunk_size + t_idx;
            let track_op_id = match ctx.db.lock() {
                Ok(conn) => {
                    match record_operation_intent(
                        &conn,
                        &ctx.job_id,
                        track_seq,
                        std::slice::from_ref(&track.id),
                    ) {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("Failed to record track operation intent: {}", e);
                            format!("op_track_{}_{}", ctx.chunk_idx, t_idx)
                        }
                    }
                }
                Err(_) => format!("op_track_{}_{}", ctx.chunk_idx, t_idx),
            };

            loop {
                if ctx.control.is_cancelled.load(Ordering::SeqCst) {
                    break;
                }

                let start = std::time::Instant::now();
                let res = ctx
                    .provider
                    .add_tracks_to_playlist(&ctx.target_id, std::slice::from_ref(&track.id))
                    .await;
                let latency_ms = start.elapsed().as_millis() as u64;

                match res {
                    Ok(_) => {
                        if let Ok(conn) = ctx.db.lock() {
                            let _ = update_operation_outcome(
                                &conn,
                                &track_op_id,
                                OperationOutcome::Confirmed,
                                track_attempt + 1,
                                Some("accepted"),
                            );
                        }
                        let succ = ctx.successful_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let proc = ctx.processed_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let fail = ctx.failed_count.load(Ordering::SeqCst);
                        if let Ok(mut ids) = ctx.added_ids.lock() {
                            ids.push(track.id.clone());
                        }

                        let progress = TransferProgressPayload {
                            job_id: ctx.job_id.clone(),
                            stage: "transferring".to_string(),
                            processed: proc,
                            total: ctx.total,
                            successful: succ,
                            failed: fail,
                            current_track: Some(format!(
                                "{} - {}",
                                track.title,
                                track.artists.join(", ")
                            )),
                            worker_id: ctx.worker_id,
                            latency_ms,
                            http_status: 200,
                            is_retry: track_attempt > 0,
                        };
                        let _ = ctx.app.emit("transfer:progress", progress);

                        let log = TransferLogPayload {
                            timestamp: chrono::Utc::now().timestamp(),
                            worker_id: ctx.worker_id,
                            message: format!("Added '{}' ({}ms)", track.title, latency_ms),
                            level: "info".to_string(),
                        };
                        let _ = ctx.app.emit("transfer:log", log);
                        break;
                    }
                    Err(track_err) => {
                        let kind = classify_error(&track_err);
                        if kind == ProviderErrorKind::Timeout {
                            if let Ok(reconciled) = ctx
                                .provider
                                .reconcile_added_tracks(
                                    &ctx.target_id,
                                    std::slice::from_ref(&track.id),
                                )
                                .await
                            {
                                if reconciled == ReconciliationResult::Confirmed {
                                    if let Ok(conn) = ctx.db.lock() {
                                        let _ = update_operation_outcome(
                                            &conn,
                                            &track_op_id,
                                            OperationOutcome::Confirmed,
                                            track_attempt + 1,
                                            Some("confirmed by read-back"),
                                        );
                                    }
                                    let _succ =
                                        ctx.successful_count.fetch_add(1, Ordering::SeqCst) + 1;
                                    let _proc =
                                        ctx.processed_count.fetch_add(1, Ordering::SeqCst) + 1;
                                    if let Ok(mut ids) = ctx.added_ids.lock() {
                                        ids.push(track.id.clone());
                                    }
                                    break;
                                }
                                if reconciled == ReconciliationResult::Uncertain {
                                    if let Ok(conn) = ctx.db.lock() {
                                        let _ = update_operation_outcome(
                                            &conn,
                                            &track_op_id,
                                            OperationOutcome::Ambiguous,
                                            track_attempt + 1,
                                            Some(&track_err),
                                        );
                                    }
                                    ctx.failed_count.fetch_add(1, Ordering::SeqCst);
                                    ctx.processed_count.fetch_add(1, Ordering::SeqCst);
                                    break;
                                }
                            }
                        }

                        let has_budget = consume_retry_budget(&ctx.retry_budget);

                        if is_retryable(kind) && track_attempt < ctx.max_retries && has_budget {
                            let delay =
                                retry_delay_ms(track_attempt, None).max(ctx.base_delay_ms);
                            track_attempt += 1;

                            let log = TransferLogPayload {
                                timestamp: chrono::Utc::now().timestamp(),
                                worker_id: ctx.worker_id,
                                message: format!(
                                    "Transient error for '{}'. Retrying ({}/{}) in {}ms",
                                    track.title, track_attempt, ctx.max_retries, delay
                                ),
                                level: "warn".to_string(),
                            };
                            let _ = ctx.app.emit("transfer:log", log);

                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            continue;
                        }

                        if let Ok(conn) = ctx.db.lock() {
                            let outcome = if kind == ProviderErrorKind::Timeout {
                                OperationOutcome::Ambiguous
                            } else {
                                OperationOutcome::Failed
                            };
                            let _ = update_operation_outcome(
                                &conn,
                                &track_op_id,
                                outcome,
                                track_attempt + 1,
                                Some(&track_err),
                            );
                        }

                        let fail = ctx.failed_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let proc = ctx.processed_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let succ = ctx.successful_count.load(Ordering::SeqCst);

                        let progress = TransferProgressPayload {
                            job_id: ctx.job_id.clone(),
                            stage: "transferring".to_string(),
                            processed: proc,
                            total: ctx.total,
                            successful: succ,
                            failed: fail,
                            current_track: Some(format!(
                                "{} - {}",
                                track.title,
                                track.artists.join(", ")
                            )),
                            worker_id: ctx.worker_id,
                            latency_ms,
                            http_status: if matches!(kind, ProviderErrorKind::RateLimited) {
                                429
                            } else if matches!(kind, ProviderErrorKind::Conflict) {
                                409
                            } else {
                                500
                            },
                            is_retry: track_attempt > 0,
                        };
                        let _ = ctx.app.emit("transfer:progress", progress);

                        let log = TransferLogPayload {
                            timestamp: chrono::Utc::now().timestamp(),
                            worker_id: ctx.worker_id,
                            message: format!(
                                "Failed to add '{}': {}",
                                track.title, track_err
                            ),
                            level: "error".to_string(),
                        };
                        let _ = ctx.app.emit("transfer:log", log);
                        break;
                    }
                }
            }
        }
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
        let pre_existing_tracks = provider
            .get_playlist_tracks(&target_id, None)
            .await
            .unwrap_or_default();
        let pre_existing_ids: Vec<String> = pre_existing_tracks.iter().map(|t| t.id.clone()).collect();

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
                        timestamp: chrono::Utc::now().timestamp(),
                        worker_id: 0,
                        message: format!(
                            "Skipped '{}' (already in destination playlist)",
                            track.title
                        ),
                        level: "info".to_string(),
                    };
                    let _ = self.app.emit("transfer:log", log);
                } else {
                    existing_pool.push(track.clone());
                    deduplicated.push(track);
                }
            }

            if skipped > 0 {
                let log = TransferLogPayload {
                    timestamp: chrono::Utc::now().timestamp(),
                    worker_id: 0,
                    message: format!(
                        "Deduplication complete: skipped {} duplicate track(s). Transferring {} remaining track(s).",
                        skipped,
                        deduplicated.len()
                    ),
                    level: "info".to_string(),
                };
                let _ = self.app.emit("transfer:log", log);
            }

            (deduplicated, skipped)
        } else {
            (config.tracks, 0usize)
        };

        // If all tracks were duplicates, complete early without firing network mutations
        if tracks_to_transfer.is_empty() {
            let log = TransferLogPayload {
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: "All requested tracks already exist in destination playlist. No transfer operations needed.".to_string(),
                level: "info".to_string(),
            };
            let _ = self.app.emit("transfer:log", log);

            let complete_payload = TransferProgressPayload {
                job_id: config.job_id.clone(),
                stage: "completed".to_string(),
                processed: total,
                total,
                successful: 0,
                failed: 0,
                current_track: None,
                worker_id: 0,
                latency_ms: 0,
                http_status: 200,
                is_retry: false,
            };
            let _ = self.app.emit("transfer:progress", complete_payload);

            return Ok(BatchTransferSummary {
                job_id: config.job_id,
                snapshot_id,
                target_playlist_id: target_id,
                total_tracks: total,
                successful_tracks: 0,
                failed_tracks: 0,
                skipped_duplicates,
                is_cancelled: false,
                audit: None,
            });
        }

        // Stage 4: Chunked batch execution with true Tokio concurrency and retry
        let concurrency = resolve_concurrency(&config.target_service, config.concurrency);
        if config.target_service == "ytmusic" && config.concurrency > 1 {
            let log = TransferLogPayload {
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: format!(
                    "YouTube Music target service enforces single-writer playlist mutations. Concurrency serialized to 1 worker (requested: {}) to prevent 409 Conflict collisions.",
                    config.concurrency
                ),
                level: "info".to_string(),
            };
            let _ = self.app.emit("transfer:log", log);
        }

        let processed_count = Arc::new(AtomicUsize::new(skipped_duplicates));
        let successful_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));
        let retry_budget = Arc::new(AtomicU32::new(20));
        let added_ids = Arc::new(std::sync::Mutex::new(Vec::with_capacity(tracks_to_transfer.len())));

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

        let base_delay_ms = 1500u64;
        let max_retries = 5;

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut join_set = tokio::task::JoinSet::new();

        for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
            if control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            while control.is_paused.load(Ordering::SeqCst) {
                if control.is_cancelled.load(Ordering::SeqCst) {
                    break;
                }
                control.pause_notify.notified().await;
            }

            if control.is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            let sem_permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };

            if control.is_cancelled.load(Ordering::SeqCst) {
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
                base_delay_ms,
                max_retries,
                chunk_size,
            };

            join_set.spawn(async move {
                let _permit = sem_permit;
                process_chunk(ctx).await;
            });
        }

        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                eprintln!("Worker task join error: {}", e);
            }
        }

        let is_cancelled = control.is_cancelled.load(Ordering::SeqCst);
        let final_added_ids = {
            let ids = added_ids.lock().unwrap();
            ids.clone()
        };
        let total_successful = successful_count.load(Ordering::SeqCst);
        let total_failed = failed_count.load(Ordering::SeqCst);
        let total_processed = processed_count.load(Ordering::SeqCst);

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
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: 0,
                message: "Transfer cancelled by user. Added tracks preserved in snapshot."
                    .to_string(),
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
                skipped_duplicates,
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
                level: if audit.is_verified {
                    "info".to_string()
                } else {
                    "warn".to_string()
                },
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
            skipped_duplicates,
            is_cancelled: false,
            audit: audit_result,
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
        let existing = vec![
            sample_track("4cOdK2wGLETKBW3PvgPWqT", "Song A", vec!["Artist 1"], None),
        ];

        let incoming_exact = sample_track("4cOdK2wGLETKBW3PvgPWqT", "Different Title", vec!["Diff Artist"], None);
        assert!(is_duplicate_track(&incoming_exact, &existing));

        let incoming_prefixed = sample_track("spotify:track:4cOdK2wGLETKBW3PvgPWqT", "Different", vec!["Diff"], None);
        assert!(is_duplicate_track(&incoming_prefixed, &existing));
    }

    #[test]
    fn test_is_duplicate_track_by_isrc() {
        let existing = vec![
            sample_track("id_1", "Song Title", vec!["Artist"], Some("USUM71703861")),
        ];

        let incoming = sample_track("id_2", "Song Title (Radio Edit)", vec!["Different Artist Label"], Some("usum71703861"));
        assert!(is_duplicate_track(&incoming, &existing));
    }

    #[test]
    fn test_is_duplicate_track_by_normalized_title_and_artist_overlap() {
        let existing = vec![
            sample_track("id_1", "Deva Deva", vec!["Pritam", "Arijit Singh"], None),
        ];

        // Reversed artists or subsets still overlap on primary contributors
        let incoming_reversed = sample_track("id_2", "Deva Deva (Film Version)", vec!["Arijit Singh", "Pritam"], None);
        assert!(is_duplicate_track(&incoming_reversed, &existing));

        // Remaster stripping
        let incoming_remaster = sample_track("id_3", "Deva Deva - 2024 Remaster", vec!["Pritam"], None);
        assert!(is_duplicate_track(&incoming_remaster, &existing));
    }

    #[test]
    fn test_is_not_duplicate_when_artists_differ() {
        let existing = vec![
            sample_track("id_1", "Hello", vec!["Adele"], None),
        ];

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
