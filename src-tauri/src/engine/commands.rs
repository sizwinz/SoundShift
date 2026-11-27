use crate::engine::lifecycle::{JobState, LifecycleUpdate};
use crate::engine::matcher::{
    flush_match_cache_writes, format_auth_abort_error, load_match_cache_map,
    match_track_with_cache, MatchResult, PendingCacheWrite,
};
use crate::engine::rate_limiter::SearchRateLimiter;
use crate::engine::snapshot::{AuditResult, TransferHistoryEntry};
use crate::engine::worker::{
    BatchTransferConfig, BatchTransferSummary, TransferControl, TransferWorkerPool,
};
use crate::models::SourceTrack;
use crate::providers::error::ProviderErrorKind;
use crate::providers::spotify::SpotifyProvider;
use crate::providers::traits::MusicProvider;
use crate::providers::ytmusic::YouTubeMusicProvider;
use crate::AppState;
use rusqlite::params;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

fn build_provider(
    state: &AppState,
    service: &str,
    token: String,
) -> Result<Box<dyn MusicProvider>, String> {
    match service {
        "spotify" => Ok(Box::new(SpotifyProvider::with_session_cache(
            token,
            state.spotify_cache.clone(),
        ))),
        "ytmusic" => Ok(Box::new(YouTubeMusicProvider::with_session_cache(
            token,
            state.ytmusic_cache.clone(),
        ))),
        _ => Err(format!("Unsupported service: {}", service)),
    }
}

async fn run_matching_pipeline(
    app: &tauri::AppHandle,
    source_service: String,
    target_service: String,
    playlist_id: String,
    tracks: Vec<SourceTrack>,
    matching_concurrency: Option<usize>,
) -> Result<Vec<MatchResult>, String> {
    if tracks.is_empty() {
        return Err("No tracks provided for matching.".to_string());
    }

    let state = app.state::<AppState>();
    let db_arc = state.db.clone();
    let src_svc_read = source_service.clone();
    let tgt_svc_read = target_service.clone();

    let (target_token, cache_map) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = db_arc.lock().map_err(|e| e.to_string())?;
        let token = crate::auth::keyring_store::retrieve_credential(&tgt_svc_read, &conn)?
            .ok_or_else(|| format!("Target service '{}' is not authenticated", tgt_svc_read))?;
        let map = Arc::new(load_match_cache_map(&conn, &src_svc_read, &tgt_svc_read));
        Ok((token, map))
    })
    .await
    .map_err(|e| e.to_string())??;

    let target_provider: Arc<Box<dyn MusicProvider>> =
        Arc::new(build_provider(&state, &target_service, target_token)?);

    let concurrency = matching_concurrency.unwrap_or(4).clamp(1, 4);
    let rate_limiter = Arc::new(SearchRateLimiter::new(concurrency, 120));
    let total = tracks.len();
    let mut indexed_results: Vec<Option<MatchResult>> = (0..total).map(|_| None).collect();
    let mut join_set = tokio::task::JoinSet::new();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let processed_counter = Arc::new(AtomicUsize::new(0));
    let auth_aborted = Arc::new(AtomicBool::new(false));
    let mut pending_writes: Vec<PendingCacheWrite> = Vec::new();

    for (idx, track) in tracks.into_iter().enumerate() {
        if auth_aborted.load(Ordering::SeqCst) {
            break;
        }
        let sem_permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| e.to_string())?;
        let source_service = source_service.clone();
        let target_service = target_service.clone();
        let target_provider = target_provider.clone();
        let rate_limiter = rate_limiter.clone();
        let cache_ref = cache_map.clone();
        let app_handle = app.clone();
        let pid = playlist_id.clone();
        let processed_ref = processed_counter.clone();
        let aborted_ref = auth_aborted.clone();

        join_set.spawn(async move {
            let _permit = sem_permit;
            if aborted_ref.load(Ordering::SeqCst) {
                return Err(format_auth_abort_error(&target_service));
            }
            let cached_entry = cache_ref.get(&track.id);
            let outcome = match_track_with_cache(
                &track,
                &source_service,
                &target_service,
                target_provider.as_ref().as_ref(),
                cached_entry,
                &rate_limiter,
            )
            .await;

            match outcome {
                Ok((result, cache_write)) => {
                    let processed = processed_ref.fetch_add(1, Ordering::SeqCst) + 1;
                    let payload = serde_json::json!({
                        "playlist_id": pid,
                        "processed": processed,
                        "total": total,
                        "current_track": track.title,
                        "current_result": &result,
                    });
                    let _ = app_handle.emit("matching:progress", payload);
                    Ok((idx, result, cache_write))
                }
                Err(err) if err.kind == ProviderErrorKind::Authentication => {
                    aborted_ref.store(true, Ordering::SeqCst);
                    Err(err)
                }
                Err(_) => Ok((
                    idx,
                    MatchResult {
                        source_track: track,
                        status: crate::engine::matcher::MatchClassification::NotFound,
                        matched_track: None,
                        candidates: Vec::new(),
                        confidence: 0.0,
                        match_method: None,
                    },
                    None,
                )),
            }
        });
    }

    let mut auth_error_message: Option<String> = None;
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok((idx, match_result, write_opt))) => {
                indexed_results[idx] = Some(match_result);
                if let Some(w) = write_opt {
                    pending_writes.push(w);
                }
            }
            Ok(Err(provider_err)) => {
                if auth_error_message.is_none() {
                    auth_error_message = Some(format!(
                        "{}: {}",
                        format_auth_abort_error(&target_service).detail,
                        provider_err.detail
                    ));
                }
            }
            Err(e) => {
                crate::diagnostics::write_error(&format!("Error joining matching task: {}", e));
            }
        }
    }

    if !pending_writes.is_empty() {
        let db_write = state.db.clone();
        let _ = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let mut conn = db_write.lock().map_err(|e| e.to_string())?;
            flush_match_cache_writes(&mut conn, &pending_writes)
        })
        .await;
    }

    if let Some(err_msg) = auth_error_message {
        return Err(err_msg);
    }

    Ok(indexed_results.into_iter().flatten().collect())
}

/// Executes the 4-stage matching pipeline across an ingested playlist.
/// Emits progressive `matching:progress` events to frontend diff view.
#[tauri::command]
pub async fn execute_playlist_matching(
    app: tauri::AppHandle,
    source_service: String,
    target_service: String,
    playlist_id: String,
    matching_concurrency: Option<usize>,
) -> Result<Vec<MatchResult>, String> {
    let state = app.state::<AppState>();
    let db_arc = state.db.clone();
    let pid = playlist_id.clone();
    let svc = source_service.clone();

    let tracks = tokio::task::spawn_blocking(move || -> Result<Vec<SourceTrack>, String> {
        let conn = db_arc.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT track_data 
                 FROM cached_tracks 
                 WHERE playlist_id = ?1 AND (service = ?2 OR service = '')
                 ORDER BY position ASC",
            )
            .map_err(|e| e.to_string())?;

        let rows: Result<Vec<SourceTrack>, _> = stmt
            .query_map(params![pid, svc], |row| {
                let json_str: String = row.get(0)?;
                serde_json::from_str(&json_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })
            .and_then(|mapped| mapped.collect());

        rows.map_err(|e| format!("Failed to read cached tracks: {}", e))
    })
    .await
    .map_err(|e| e.to_string())??;

    if tracks.is_empty() {
        return Err("No tracks found for playlist. Ingest playlist tracks first.".to_string());
    }

    run_matching_pipeline(
        &app,
        source_service,
        target_service,
        playlist_id,
        tracks,
        matching_concurrency,
    )
    .await
}

#[tauri::command]
pub async fn match_custom_tracks(
    app: tauri::AppHandle,
    source_service: String,
    target_service: String,
    tracks: Vec<SourceTrack>,
    matching_concurrency: Option<usize>,
) -> Result<Vec<MatchResult>, String> {
    run_matching_pipeline(
        &app,
        source_service,
        target_service,
        "custom_import".to_string(),
        tracks,
        matching_concurrency,
    )
    .await
}

#[tauri::command]
pub async fn save_manual_match_override(
    app: tauri::AppHandle,
    source_service: String,
    source_track_id: String,
    target_service: String,
    target_track: SourceTrack,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db_arc = state.db.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = db_arc.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        let write = PendingCacheWrite {
            source_service,
            source_track_id,
            target_service,
            target_track_id: target_track.id.clone(),
            match_method: "manual_disambiguation".to_string(),
            confidence: 1.0,
            target_track_json: serde_json::to_string(&target_track).ok(),
            created_at: now,
        };
        flush_match_cache_writes(&mut conn, &[write])
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clear_match_cache(app: tauri::AppHandle) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let db_arc = state.db.clone();
    tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let conn = db_arc.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM track_match_cache", [])
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Starts an asynchronous batch migration using the Tokio worker pool.
#[tauri::command]
pub async fn start_batch_transfer(
    app: tauri::AppHandle,
    config: BatchTransferConfig,
) -> Result<BatchTransferSummary, String> {
    let state = app.state::<AppState>();
    let _mutation_guard =
        crate::sync::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)?;
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
    }

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM transfer_jobs WHERE job_id = ?1)",
                [&config.job_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Err(
                "Job ID already exists. Use the recorded job for recovery; do not replay it."
                    .into(),
            );
        }
    }

    // Resolve target provider credentials
    let target_token = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::auth::keyring_store::retrieve_credential(&config.target_service, &conn)?.ok_or_else(
            || {
                format!(
                    "Target service '{}' is not authenticated",
                    config.target_service
                )
            },
        )?
    };

    let target_provider: Arc<Box<dyn MusicProvider>> = Arc::new(
        build_provider(&state, &config.target_service, target_token)
            .map_err(|e| e.replace("Unsupported service:", "Unsupported target service:"))?,
    );

    let event_app = app.clone();
    let control = TransferControl::new(config.job_id.clone(), state.db.clone(), move |update| {
        let _ = event_app.emit("job:state_changed", update);
    });
    {
        let mut transfers = state.active_transfers.lock().map_err(|e| e.to_string())?;
        transfers.insert(config.job_id.clone(), control.clone());
    }

    let pool = TransferWorkerPool::new(app.clone(), state.db.clone());
    let job_id = config.job_id.clone();
    let result = pool
        .execute_batch(config, target_provider, control.clone())
        .await;
    if let Err(ref error) = result {
        let _ = control.fail(error);
    }

    // Clean up active transfer controller
    if let Ok(mut transfers) = state.active_transfers.lock() {
        transfers.remove(&job_id);
    }

    result.or_else(|error| {
        let conn = state.db.lock().map_err(|_| "Summary storage unavailable")?;
        let inspection = crate::engine::recovery::inspect_job(&conn, &job_id)
            .map_err(|_| crate::diagnostics::sanitize_for_log(&error))?;
        if !matches!(
            inspection.state.as_str(),
            "recovery_required"
                | "manual_review"
                | "cancelled"
                | "completed_verified"
                | "rolled_back"
        ) {
            return Err(crate::diagnostics::sanitize_for_log(&error));
        }
        let target: String = conn
            .query_row(
                "SELECT target_playlist_id FROM transfer_jobs WHERE job_id=?1",
                [&job_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(BatchTransferSummary {
            job_id: job_id.clone(),
            snapshot_id: format!("snap_{}", job_id),
            target_playlist_id: target,
            total_tracks: inspection.counts.selected,
            successful_tracks: inspection.counts.confirmed,
            failed_tracks: inspection.counts.failed,
            skipped_duplicates: inspection.counts.skipped,
            is_cancelled: inspection.state == "cancelled",
            audit: None,
            state: inspection.state,
            event_seq: inspection.event_seq,
            counts: inspection.counts,
            verification: inspection.verification,
            error: Some(crate::diagnostics::sanitize_for_log(&error)),
        })
    })
}

/// The registry is used only to clone the controller, never held across work.
fn control_job(
    app: &tauri::AppHandle,
    job_id: &str,
    action: &str,
) -> Result<LifecycleUpdate, String> {
    let state = app.state::<AppState>();
    let control = {
        state
            .active_transfers
            .lock()
            .map_err(|e| e.to_string())?
            .get(job_id)
            .cloned()
    };
    if let Some(control) = control {
        return match action {
            "pause" => control.pause(),
            "resume" => control.resume(),
            _ => control.cancel(),
        };
    }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let (state, event_seq): (String, u64) = conn.query_row(
        "SELECT job_state, COALESCE((SELECT MAX(event_seq) FROM transfer_events WHERE job_id = ?1), 0)
         FROM transfer_jobs WHERE job_id = ?1", [job_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "No active or recorded transfer found".to_string())?;
    let state: JobState = serde_json::from_value(serde_json::Value::String(state))
        .map_err(|_| "Transfer cannot be controlled in its recorded state")?;
    if !matches!(
        state,
        JobState::Cancelled
            | JobState::CompletedVerified
            | JobState::RecoveryRequired
            | JobState::ManualReview
            | JobState::RolledBack
    ) {
        return Err("Transfer is interrupted; review recovery before taking action".into());
    }
    Ok(LifecycleUpdate {
        job_id: job_id.into(),
        state,
        event_seq,
    })
}

#[tauri::command]
pub async fn pause_batch_transfer(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<LifecycleUpdate, String> {
    control_job(&app, &job_id, "pause")
}
#[tauri::command]
pub async fn resume_batch_transfer(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<LifecycleUpdate, String> {
    control_job(&app, &job_id, "resume")
}
#[tauri::command]
pub async fn cancel_batch_transfer(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<LifecycleUpdate, String> {
    control_job(&app, &job_id, "cancel")
}

/// Executes a 1-click snapshot rollback.
#[tauri::command]
pub async fn execute_snapshot_rollback(
    app: tauri::AppHandle,
    snapshot_id: String,
    delete_entire_playlist: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _mutation_guard =
        crate::sync::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)?;
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
    }

    // Retrieve target service from snapshot payload first to know which provider to build
    let (target_service, target_token) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT mutation_payload FROM transfer_snapshots WHERE snapshot_id = ?1")
            .map_err(|e| e.to_string())?;
        let payload_str: String = stmt
            .query_row(rusqlite::params![snapshot_id], |row| row.get(0))
            .map_err(|e| format!("Snapshot not found: {}", e))?;
        let payload: crate::engine::snapshot::SnapshotPayload = serde_json::from_str(&payload_str)
            .map_err(|e| format!("Failed to parse snapshot: {}", e))?;
        crate::storage::recovery::ensure_job_replay_allowed(&conn, &payload.job_id)?;

        let token =
            crate::auth::keyring_store::retrieve_credential(&payload.target_service, &conn)?
                .ok_or_else(|| {
                    format!(
                        "Target service '{}' is not authenticated",
                        payload.target_service
                    )
                })?;

        (payload.target_service, token)
    };

    let target_provider = build_provider(&state, &target_service, target_token)
        .map_err(|e| e.replace("Unsupported service:", "Unsupported target service:"))?;

    crate::engine::recovery::rollback_recorded(
        &state.db,
        &snapshot_id,
        delete_entire_playlist,
        target_provider.as_ref(),
        |update| {
            let _ = app.emit("job:state_changed", update);
        },
    )
    .await
    .map(|_| ())
}

/// Runs an on-demand integrity audit asserting destination playlist against expected track IDs.
#[tauri::command]
pub async fn audit_playlist_integrity(
    app: tauri::AppHandle,
    target_service: String,
    target_playlist_id: String,
    expected_track_ids: Vec<String>,
    snapshot_id: Option<String>,
) -> Result<AuditResult, String> {
    let state = app.state::<AppState>();
    let target_token = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::auth::keyring_store::retrieve_credential(&target_service, &conn)?
            .ok_or_else(|| format!("Target service '{}' is not authenticated", target_service))?
    };

    let target_provider = build_provider(&state, &target_service, target_token)
        .map_err(|e| e.replace("Unsupported service:", "Unsupported target service:"))?;

    let (expected_track_ids, revision) = if let Some(snapshot_id) = snapshot_id {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let payload = crate::engine::snapshot::load_snapshot(&conn, &snapshot_id)?;
        if payload.target_service != target_service
            || payload.target_playlist_id != target_playlist_id
        {
            return Err("Audit target does not match the recorded snapshot".into());
        }
        let uncertain: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM transfer_operations WHERE job_id = ?1 AND status IN ('pending', 'ambiguous'))",
            [&payload.job_id], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if uncertain {
            return Err("Uncertain operations require reconciliation; content audit alone cannot verify this job".into());
        }
        let mut ids = payload.pre_existing_track_ids;
        ids.extend(payload.added_track_ids);
        (
            ids,
            crate::engine::snapshot::latest_playlist_revision(&conn, &payload.job_id)?,
        )
    } else {
        if expected_track_ids.is_empty() {
            return Err(
                "A snapshot or expected occurrences are required to verify playlist contents"
                    .into(),
            );
        }
        (expected_track_ids, None)
    };
    crate::engine::snapshot::audit_playlist_at_revision(
        target_provider.as_ref(),
        &target_playlist_id,
        &expected_track_ids,
        revision.as_deref(),
    )
    .await
}

/// Retrieves all past transfer jobs with snapshots from SQLite.
#[tauri::command]
pub async fn get_transfer_history(
    app: tauri::AppHandle,
) -> Result<Vec<TransferHistoryEntry>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    crate::engine::snapshot::get_transfer_history(&conn)
}

#[tauri::command]
pub async fn get_storage_recovery_notice(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    crate::storage::recovery::recovery_notice(&conn)
}

#[tauri::command]
pub async fn acknowledge_storage_recovery(
    app: tauri::AppHandle,
    reviewed: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _guard =
        crate::sync::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    crate::storage::recovery::acknowledge_recovery(&conn, reviewed)
}

fn recorded_job_provider(
    app: &tauri::AppHandle,
    job_id: &str,
) -> Result<Box<dyn MusicProvider>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    let service: String = conn
        .query_row(
            "SELECT target_service FROM transfer_jobs WHERE job_id=?1",
            [job_id],
            |r| r.get(0),
        )
        .map_err(|_| "Recorded job not found")?;
    let token = crate::auth::keyring_store::retrieve_credential(&service, &conn)?
        .ok_or("Session unavailable; re-authenticate before recovery")?;
    build_provider(&state, &service, token).map_err(|_| "Unsupported recorded provider".into())
}

#[tauri::command]
pub async fn inspect_transfer_job(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<crate::engine::recovery::JobInspection, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    crate::engine::recovery::inspect_job(&conn, &job_id)
}

#[tauri::command]
pub async fn reconcile_transfer_job(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<crate::engine::recovery::JobInspection, String> {
    let state = app.state::<AppState>();
    let _guard =
        crate::sync::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)?;
    let provider = recorded_job_provider(&app, &job_id)?;
    crate::engine::recovery::reconcile_job(&state.db, provider.as_ref(), &job_id, |update| {
        let _ = app.emit("job:state_changed", update);
    })
    .await
}

#[tauri::command]
pub async fn resume_recorded_transfer(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<crate::engine::recovery::JobInspection, String> {
    let state = app.state::<AppState>();
    let _guard =
        crate::sync::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)?;
    {
        let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
        crate::storage::recovery::ensure_job_replay_allowed(&conn, &job_id)?;
    }
    let provider = recorded_job_provider(&app, &job_id)?;
    let event_app = app.clone();
    let control = TransferControl::new(job_id.clone(), state.db.clone(), move |update| {
        let _ = event_app.emit("job:state_changed", update);
    });
    {
        state
            .active_transfers
            .lock()
            .map_err(|_| "Transfer registry unavailable")?
            .insert(job_id.clone(), control.clone());
    }
    let result =
        crate::engine::recovery::resume_job(&state.db, provider.as_ref(), &job_id, &control).await;
    {
        state
            .active_transfers
            .lock()
            .map_err(|_| "Transfer registry unavailable")?
            .remove(&job_id);
    }
    result.map_err(|error| crate::diagnostics::sanitize_for_log(&error))
}

#[tauri::command]
pub fn get_sanitized_diagnostic_report(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    let schema_version: String = conn
        .query_row(
            "SELECT value FROM schema_metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "unknown".to_string());

    let recovery_notice = crate::storage::recovery::recovery_notice(&conn).unwrap_or(None);
    let jobs = crate::engine::snapshot::get_transfer_history(&conn).unwrap_or_default();
    let sync_pairs = crate::sync::store::list_pairs(&conn).unwrap_or_default();

    let payload = serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "schema_version": schema_version,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "recovery_notice": recovery_notice,
        "transfer_jobs_count": jobs.len(),
        "recovery_required_jobs": jobs.iter().filter(|j| j.recovery_required).count(),
        "transfer_jobs": jobs.into_iter().take(25).map(|j| serde_json::json!({
            "job_id": j.job_id,
            "source_service": j.source_service,
            "target_service": j.target_service,
            "job_state": j.job_state,
            "recovery_required": j.recovery_required,
            "total_tracks": j.total_tracks,
            "matched_tracks": j.matched_tracks,
            "added_tracks_count": j.added_tracks_count,
            "is_rolled_back": j.is_rolled_back,
            "last_error": j.last_error,
        })).collect::<Vec<_>>(),
        "sync_pairs": sync_pairs.into_iter().map(|p| serde_json::json!({
            "pair_id": p.pair_id,
            "state": p.state,
            "enabled": p.enabled,
            "next_due_at": p.next_due_at,
            "last_error": p.last_error,
        })).collect::<Vec<_>>(),
    });

    let serialized = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    Ok(crate::diagnostics::sanitize_for_log(&serialized))
}
