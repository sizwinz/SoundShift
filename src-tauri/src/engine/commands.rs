use crate::engine::matcher::{match_track, MatchResult};
use crate::engine::rate_limiter::SearchRateLimiter;
use crate::models::SourceTrack;
use crate::providers::spotify::SpotifyProvider;
use crate::providers::traits::MusicProvider;
use crate::providers::ytmusic::YouTubeMusicProvider;
use crate::AppState;
use rusqlite::params;
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
            Some(&state.db),
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
