use crate::engine::lifecycle::{JobState, LifecycleUpdate};
use crate::engine::matcher::{match_track, MatchResult};
use crate::engine::rate_limiter::SearchRateLimiter;
use crate::engine::snapshot::{AuditResult, TransferHistoryEntry};
use crate::engine::worker::{
    BatchTransferConfig, BatchTransferSummary, TransferControl, TransferWorkerPool,
};
use crate::models::SourceTrack;
use crate::providers::spotify::SpotifyProvider;
use crate::providers::traits::MusicProvider;
use crate::providers::ytmusic::YouTubeMusicProvider;
use crate::AppState;
use rusqlite::params;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Executes the 4-stage matching pipeline across an ingested playlist.
/// Emits progressive `matching:progress` events to frontend diff view.
#[tauri::command]
pub async fn execute_playlist_matching(
    app: tauri::AppHandle,
    source_service: String,
    target_service: String,
    playlist_id: String,
) -> Result<Vec<MatchResult>, String> {
    let state = app.state::<AppState>();

    // Step 1: Load tracks from SQLite cache
    let tracks: Vec<SourceTrack> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT track_data 
                 FROM cached_tracks 
                 WHERE playlist_id = ?1 
                 ORDER BY position ASC",
            )
            .map_err(|e| e.to_string())?;

        let rows: Result<Vec<SourceTrack>, _> = stmt
            .query_map(params![playlist_id], |row| {
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

        rows.map_err(|e| format!("Failed to read cached tracks: {}", e))?
    };

    if tracks.is_empty() {
        return Err("No tracks found for playlist. Ingest playlist tracks first.".to_string());
    }

    // Step 2: Initialize target provider driver
    let target_token = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::auth::keyring_store::retrieve_credential(&target_service, &conn)?
            .ok_or_else(|| format!("Target service '{}' is not authenticated", target_service))?
    };

    let target_provider: Arc<Box<dyn MusicProvider>> = match target_service.as_str() {
        "spotify" => Arc::new(Box::new(SpotifyProvider::new(target_token))),
        "ytmusic" => Arc::new(Box::new(YouTubeMusicProvider::new(target_token))),
        _ => return Err(format!("Unsupported target service: {}", target_service)),
    };

    let rate_limiter = Arc::new(SearchRateLimiter::default());
    let total = tracks.len();
    let mut indexed_results: Vec<Option<MatchResult>> = (0..total).map(|_| None).collect();
    let mut join_set = tokio::task::JoinSet::new();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let processed_counter = Arc::new(AtomicUsize::new(0));

    for (idx, track) in tracks.into_iter().enumerate() {
        let sem_permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| e.to_string())?;
        let source_service = source_service.clone();
        let target_service = target_service.clone();
        let target_provider = target_provider.clone();
        let rate_limiter = rate_limiter.clone();
        let db = state.db.clone();
        let app_handle = app.clone();
        let pid = playlist_id.clone();
        let processed_ref = processed_counter.clone();

        join_set.spawn(async move {
            let _permit = sem_permit;
            let result = match_track(
                &track,
                &source_service,
                &target_service,
                target_provider.as_ref().as_ref(),
                Some(&*db),
                &rate_limiter,
            )
            .await;

            let processed = processed_ref.fetch_add(1, Ordering::SeqCst) + 1;
            let payload = serde_json::json!({
                "playlist_id": pid,
                "processed": processed,
                "total": total,
                "current_track": track.title,
                "current_result": &result,
            });
            let _ = app_handle.emit("matching:progress", payload);

            (idx, result)
        });
    }

    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((idx, match_result)) => {
                indexed_results[idx] = Some(match_result);
            }
            Err(e) => {
                crate::diagnostics::write_error(&format!("Error joining matching task: {}", e));
            }
        }
    }

    let results: Vec<MatchResult> = indexed_results.into_iter().flatten().collect();

    Ok(results)
}

/// Starts an asynchronous batch migration using the Tokio worker pool.
#[tauri::command]
pub async fn start_batch_transfer(
    app: tauri::AppHandle,
    config: BatchTransferConfig,
) -> Result<BatchTransferSummary, String> {
    let state = app.state::<AppState>();
    let _mutation_guard = state.mutation_gate.clone().try_lock_owned().map_err(|_| {
        "Another transfer or rollback is active. Wait for it to finish.".to_string()
    })?;
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

    let target_provider: Arc<Box<dyn MusicProvider>> = match config.target_service.as_str() {
        "spotify" => Arc::new(Box::new(SpotifyProvider::new(target_token))),
        "ytmusic" => Arc::new(Box::new(YouTubeMusicProvider::new(target_token))),
        _ => {
            return Err(format!(
                "Unsupported target service: {}",
                config.target_service
            ))
        }
    };

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
    let _mutation_guard = state.mutation_gate.clone().try_lock_owned().map_err(|_| {
        "Another transfer or rollback is active. Wait for it to finish.".to_string()
    })?;
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

    let target_provider: Box<dyn MusicProvider> = match target_service.as_str() {
        "spotify" => Box::new(SpotifyProvider::new(target_token)),
        "ytmusic" => Box::new(YouTubeMusicProvider::new(target_token)),
        _ => return Err(format!("Unsupported target service: {}", target_service)),
    };

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

    let target_provider: Box<dyn MusicProvider> = match target_service.as_str() {
        "spotify" => Box::new(SpotifyProvider::new(target_token)),
        "ytmusic" => Box::new(YouTubeMusicProvider::new(target_token)),
        _ => return Err(format!("Unsupported target service: {}", target_service)),
    };

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
    let _guard = state
        .mutation_gate
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for the active transfer or rollback to finish")?;
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
    match service.as_str() {
        "spotify" => Ok(Box::new(SpotifyProvider::new(token))),
        "ytmusic" => Ok(Box::new(YouTubeMusicProvider::new(token))),
        _ => Err("Unsupported recorded provider".into()),
    }
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
    let _guard = state
        .mutation_gate
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for the active transfer or rollback to finish")?;
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
    let _guard = state
        .mutation_gate
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for the active transfer or rollback to finish")?;
    {
        let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
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
