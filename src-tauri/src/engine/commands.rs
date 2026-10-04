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

    let target_provider: Box<dyn MusicProvider> = match target_service.as_str() {
        "spotify" => Box::new(SpotifyProvider::new(target_token)),
        "ytmusic" => Box::new(YouTubeMusicProvider::new(target_token)),
        _ => return Err(format!("Unsupported target service: {}", target_service)),
    };

    let rate_limiter = SearchRateLimiter::default();
    let total = tracks.len();
    let mut results = Vec::with_capacity(total);

    for (idx, track) in tracks.iter().enumerate() {
        let match_result = match_track(
            track,
            &source_service,
            &target_service,
            target_provider.as_ref(),
            Some(&*state.db),
            &rate_limiter,
        )
        .await;

        // Emit live progress to frontend
        let payload = serde_json::json!({
            "playlist_id": playlist_id,
            "processed": idx + 1,
            "total": total,
            "current_result": match_result,
        });
        let _ = app.emit("matching:progress", payload);

        results.push(match_result);
    }

    Ok(results)
}

/// Starts an asynchronous batch migration using the Tokio worker pool.
#[tauri::command]
pub async fn start_batch_transfer(
    app: tauri::AppHandle,
    config: BatchTransferConfig,
) -> Result<BatchTransferSummary, String> {
    let state = app.state::<AppState>();

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

    let control = TransferControl::new();
    {
        let mut transfers = state.active_transfers.lock().map_err(|e| e.to_string())?;
        transfers.insert(config.job_id.clone(), control.clone());
    }

    let pool = TransferWorkerPool::new(app.clone(), state.db.clone());
    let job_id = config.job_id.clone();
    let result = pool.execute_batch(config, target_provider, control).await;

    // Clean up active transfer controller
    if let Ok(mut transfers) = state.active_transfers.lock() {
        transfers.remove(&job_id);
    }

    result
}

/// Pauses an in-flight batch migration.
#[tauri::command]
pub async fn pause_batch_transfer(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let transfers = state.active_transfers.lock().map_err(|e| e.to_string())?;
    if let Some(ctrl) = transfers.get(&job_id) {
        ctrl.pause();
        Ok(())
    } else {
        Err(format!("No active transfer found with job_id: {}", job_id))
    }
}

/// Resumes a paused batch migration.
#[tauri::command]
pub async fn resume_batch_transfer(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let transfers = state.active_transfers.lock().map_err(|e| e.to_string())?;
    if let Some(ctrl) = transfers.get(&job_id) {
        ctrl.resume();
        Ok(())
    } else {
        Err(format!("No active transfer found with job_id: {}", job_id))
    }
}

/// Cancels an in-flight batch migration.
#[tauri::command]
pub async fn cancel_batch_transfer(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let transfers = state.active_transfers.lock().map_err(|e| e.to_string())?;
    if let Some(ctrl) = transfers.get(&job_id) {
        ctrl.cancel();
        Ok(())
    } else {
        Err(format!("No active transfer found with job_id: {}", job_id))
    }
}

/// Executes a 1-click snapshot rollback.
#[tauri::command]
pub async fn execute_snapshot_rollback(
    app: tauri::AppHandle,
    snapshot_id: String,
    delete_entire_playlist: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();

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

    crate::engine::snapshot::rollback_snapshot(
        &state.db,
        &snapshot_id,
        delete_entire_playlist,
        target_provider.as_ref(),
    )
    .await
}

/// Runs an on-demand integrity audit asserting destination playlist against expected track IDs.
#[tauri::command]
pub async fn audit_playlist_integrity(
    app: tauri::AppHandle,
    target_service: String,
    target_playlist_id: String,
    expected_track_ids: Vec<String>,
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

    crate::engine::snapshot::audit_playlist(
        target_provider.as_ref(),
        &target_playlist_id,
        &expected_track_ids,
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
