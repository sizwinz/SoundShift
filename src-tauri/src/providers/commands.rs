use crate::models::{Playlist, SourceTrack};
use crate::providers::spotify::SpotifyProvider;
use crate::providers::traits::MusicProvider;
use crate::providers::ytmusic::YouTubeMusicProvider;
use crate::AppState;
use rusqlite::params;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

/// Lists user playlists with SQLite caching per D-01.
/// If refresh is false and cached records exist, loads instantly from local storage.
#[tauri::command]
pub async fn list_provider_playlists(
    app: tauri::AppHandle,
    service: String,
    refresh: Option<bool>,
) -> Result<Vec<Playlist>, String> {
    let state = app.state::<AppState>();

    // Pass 1: Try reading from local SQLite cache if refresh not requested
    if refresh != Some(true) {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, service, title, description, track_count, is_public, cover_url 
                 FROM cached_playlists 
                 WHERE service = ?1 
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let cached_rows: Result<Vec<Playlist>, _> = stmt
            .query_map(params![service], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    service: row.get(1)?,
                    title: row.get(2)?,
                    description: row.get(3)?,
                    track_count: row.get(4)?,
                    is_public: row.get::<_, i64>(5)? != 0,
                    cover_url: row.get(6)?,
                })
            })
            .and_then(|mapped| mapped.collect());

        if let Ok(playlists) = cached_rows {
            if !playlists.is_empty() {
                return Ok(playlists);
            }
        }
    }

    // Pass 2: Fetch fresh from provider driver
    let token = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::auth::keyring_store::retrieve_credential(&service, &conn)?
            .ok_or_else(|| format!("No active session found for {}", service))?
    };

    let provider: Box<dyn MusicProvider> = match service.as_str() {
        "spotify" => Box::new(SpotifyProvider::new(token)),
        "ytmusic" => Box::new(YouTubeMusicProvider::new(token)),
        _ => return Err(format!("Unsupported service: {}", service)),
    };

    let playlists = provider.list_playlists().await?;

    // Persist into SQLite cache
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        for p in &playlists {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO cached_playlists 
                 (id, service, title, description, track_count, is_public, cover_url, updated_at) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    p.id,
                    p.service,
                    p.title,
                    p.description,
                    p.track_count,
                    if p.is_public { 1 } else { 0 },
                    p.cover_url,
                    now
                ],
            );
        }
    }

    Ok(playlists)
}

/// Ingests playlist tracks in 100-track streaming chunks per D-02.
/// Emits `playlist:ingest_progress` events to frontend for instant inspection.
#[tauri::command]
pub async fn fetch_playlist_tracks(
    app: tauri::AppHandle,
    service: String,
    playlist_id: String,
    refresh: Option<bool>,
) -> Result<Vec<SourceTrack>, String> {
    let state = app.state::<AppState>();

    // Pass 1: Try reading from local SQLite cache if refresh not requested
    if refresh != Some(true) {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT track_data 
                 FROM cached_tracks 
                 WHERE playlist_id = ?1 
                 ORDER BY position ASC",
            )
            .map_err(|e| e.to_string())?;

        let cached_rows: Result<Vec<SourceTrack>, _> = stmt
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

        if let Ok(tracks) = cached_rows {
            if !tracks.is_empty() {
                return Ok(tracks);
            }
        }
    }

    // Pass 2: Ingest from provider with chunked streaming events
    let token = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::auth::keyring_store::retrieve_credential(&service, &conn)?
            .ok_or_else(|| format!("No active session found for {}", service))?
    };

    let provider: Box<dyn MusicProvider> = match service.as_str() {
        "spotify" => Box::new(SpotifyProvider::new(token)),
        "ytmusic" => Box::new(YouTubeMusicProvider::new(token)),
        _ => return Err(format!("Unsupported service: {}", service)),
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<SourceTrack>>(32);
    let app_handle = app.clone();
    let p_id = playlist_id.clone();

    // Stream chunks live to frontend per D-02
    let stream_task = tokio::spawn(async move {
        let mut loaded = 0;
        while let Some(chunk) = rx.recv().await {
            loaded += chunk.len();
            let payload = serde_json::json!({
                "playlist_id": p_id,
                "loaded": loaded,
                "tracks": chunk,
            });
            let _ = app_handle.emit("playlist:ingest_progress", payload);
        }
    });

    let tracks = provider
        .get_playlist_tracks(&playlist_id, Some(tx))
        .await?;

    let _ = stream_task.await;

    // Cache tracks into SQLite
    {
        let mut conn = state.db.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        tx.execute(
            "DELETE FROM cached_tracks WHERE playlist_id = ?1",
            params![playlist_id],
        )
        .map_err(|e| e.to_string())?;

        for (pos, track) in tracks.iter().enumerate() {
            let json_str = serde_json::to_string(track).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT OR REPLACE INTO cached_tracks 
                 (playlist_id, track_id, position, track_data) 
                 VALUES (?1, ?2, ?3, ?4)",
                params![playlist_id, track.id, pos as i64, json_str],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(tracks)
}
