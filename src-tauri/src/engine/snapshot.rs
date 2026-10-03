use crate::providers::traits::MusicProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub snapshot_id: String,
    pub job_id: String,
    pub source_service: String,
    pub target_service: String,
    pub target_playlist_id: String,
    pub target_playlist_name: String,
    pub is_new_playlist: bool,
    pub pre_existing_track_ids: Vec<String>,
    pub added_track_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResult {
    pub is_verified: bool,
    pub total_expected: usize,
    pub total_found: usize,
    pub missing_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHistoryEntry {
    pub job_id: String,
    pub snapshot_id: String,
    pub source_service: String,
    pub target_service: String,
    pub source_playlist_name: String,
    pub target_playlist_id: String,
    pub is_new_playlist: bool,
    pub total_tracks: u32,
    pub matched_tracks: u32,
    pub added_tracks_count: usize,
    pub pre_existing_count: usize,
    pub is_rolled_back: bool,
    pub created_at: i64,
    pub audit_status: Option<AuditResult>,
}

/// Transactionally records transfer job and pre-mutation snapshot into SQLite before any mutations.
pub fn create_pre_mutation_snapshot(
    conn: &rusqlite::Connection,
    job_id: &str,
    source_service: &str,
    target_service: &str,
    source_playlist_name: &str,
    target_playlist_id: &str,
    target_playlist_name: &str,
    is_new_playlist: bool,
    total_tracks: u32,
    matched_tracks: u32,
    pre_existing_track_ids: Vec<String>,
    added_track_ids: Vec<String>,
) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp();
    let snapshot_id = format!("snap_{}_{}", now, &job_id[..std::cmp::min(8, job_id.len())]);

    let payload = SnapshotPayload {
        snapshot_id: snapshot_id.clone(),
        job_id: job_id.to_string(),
        source_service: source_service.to_string(),
        target_service: target_service.to_string(),
        target_playlist_id: target_playlist_id.to_string(),
        target_playlist_name: target_playlist_name.to_string(),
        is_new_playlist,
        pre_existing_track_ids,
        added_track_ids,
        created_at: now,
    };

    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize snapshot payload: {}", e))?;

    conn.execute(
        "INSERT INTO transfer_jobs (
            job_id, source_service, target_service, source_playlist_name, 
            target_playlist_id, total_tracks, matched_tracks, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            job_id,
            source_service,
            target_service,
            source_playlist_name,
            target_playlist_id,
            total_tracks,
            matched_tracks,
            now
        ],
    )
    .map_err(|e| format!("Failed to insert transfer job: {}", e))?;

    conn.execute(
        "INSERT INTO transfer_snapshots (
            snapshot_id, job_id, mutation_payload, is_rolled_back, created_at
        ) VALUES (?1, ?2, ?3, 0, ?4)",
        rusqlite::params![snapshot_id, job_id, payload_json, now],
    )
    .map_err(|e| format!("Failed to insert transfer snapshot: {}", e))?;

    Ok(snapshot_id)
}

/// Updates the recorded added tracks in the snapshot after batch completion.
pub fn update_snapshot_added_tracks(
    conn: &rusqlite::Connection,
    snapshot_id: &str,
    actual_added_track_ids: &[String],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT mutation_payload FROM transfer_snapshots WHERE snapshot_id = ?1")
        .map_err(|e| e.to_string())?;

    let payload_str: String = stmt
        .query_row(rusqlite::params![snapshot_id], |row| row.get(0))
        .map_err(|e| format!("Snapshot not found: {}", e))?;

    let mut payload: SnapshotPayload = serde_json::from_str(&payload_str)
        .map_err(|e| format!("Failed to parse snapshot: {}", e))?;

    payload.added_track_ids = actual_added_track_ids.to_vec();

    let updated_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize updated snapshot: {}", e))?;

    conn.execute(
        "UPDATE transfer_snapshots SET mutation_payload = ?1 WHERE snapshot_id = ?2",
        rusqlite::params![updated_json, snapshot_id],
    )
    .map_err(|e| format!("Failed to update snapshot: {}", e))?;

    Ok(())
}

/// Executes 1-click snapshot rollback: either deletes newly created playlist or removes added tracks.
pub async fn rollback_snapshot(
    db: &std::sync::Mutex<rusqlite::Connection>,
    snapshot_id: &str,
    delete_entire_playlist: bool,
    provider: &dyn MusicProvider,
) -> Result<(), String> {
    let payload = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT mutation_payload, is_rolled_back FROM transfer_snapshots WHERE snapshot_id = ?1")
            .map_err(|e| e.to_string())?;

        let (payload_str, is_rolled_back): (String, i64) = stmt
            .query_row(rusqlite::params![snapshot_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Snapshot not found: {}", e))?;

        if is_rolled_back != 0 {
            return Err("This snapshot has already been rolled back".to_string());
        }

        let payload: SnapshotPayload = serde_json::from_str(&payload_str)
            .map_err(|e| format!("Failed to parse snapshot payload: {}", e))?;
        payload
    };

    if delete_entire_playlist && payload.is_new_playlist {
        provider.delete_playlist(&payload.target_playlist_id).await?;
    } else {
        provider
            .remove_tracks_from_playlist(&payload.target_playlist_id, &payload.added_track_ids)
            .await?;
    }

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE transfer_snapshots SET is_rolled_back = 1 WHERE snapshot_id = ?1",
            rusqlite::params![snapshot_id],
        )
        .map_err(|e| format!("Failed to update rollback state in SQLite: {}", e))?;
    }

    Ok(())
}

/// Audits target playlist contents against expected added tracks and computes verification state.
pub async fn audit_playlist(
    provider: &dyn MusicProvider,
    target_playlist_id: &str,
    expected_track_ids: &[String],
) -> Result<AuditResult, String> {
    let live_tracks = provider.get_playlist_tracks(target_playlist_id, None).await?;
    let live_ids: HashSet<String> = live_tracks.into_iter().map(|t| t.id).collect();

    let mut missing_ids = Vec::new();
    for expected_id in expected_track_ids {
        let clean_expected = expected_id.trim_start_matches("spotify:track:");
        let found = live_ids.contains(expected_id)
            || live_ids.contains(clean_expected)
            || live_ids.iter().any(|live_id| live_id.contains(clean_expected) || clean_expected.contains(live_id));
        if !found {
            missing_ids.push(expected_id.clone());
        }
    }

    let total_expected = expected_track_ids.len();
    let total_found = total_expected.saturating_sub(missing_ids.len());
    let is_verified = missing_ids.is_empty();

    Ok(AuditResult {
        is_verified,
        total_expected,
        total_found,
        missing_ids,
    })
}

/// Retrieves all past transfer jobs with snapshots from SQLite.
pub fn get_transfer_history(conn: &rusqlite::Connection) -> Result<Vec<TransferHistoryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT j.job_id, s.snapshot_id, j.source_service, j.target_service, 
                    j.source_playlist_name, j.target_playlist_id, s.mutation_payload, 
                    s.is_rolled_back, j.total_tracks, j.matched_tracks, j.created_at
             FROM transfer_jobs j
             JOIN transfer_snapshots s ON j.job_id = s.job_id
             ORDER BY j.created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            let job_id: String = row.get(0)?;
            let snapshot_id: String = row.get(1)?;
            let source_service: String = row.get(2)?;
            let target_service: String = row.get(3)?;
            let source_playlist_name: String = row.get(4)?;
            let target_playlist_id: String = row.get(5)?;
            let mutation_payload: String = row.get(6)?;
            let is_rolled_back: i64 = row.get(7)?;
            let total_tracks: u32 = row.get(8)?;
            let matched_tracks: u32 = row.get(9)?;
            let created_at: i64 = row.get(10)?;

            let payload: Option<SnapshotPayload> = serde_json::from_str(&mutation_payload).ok();
            let is_new_playlist = payload.as_ref().map(|p| p.is_new_playlist).unwrap_or(true);
            let added_tracks_count = payload.as_ref().map(|p| p.added_track_ids.len()).unwrap_or(0);
            let pre_existing_count = payload.as_ref().map(|p| p.pre_existing_track_ids.len()).unwrap_or(0);

            Ok(TransferHistoryEntry {
                job_id,
                snapshot_id,
                source_service,
                target_service,
                source_playlist_name,
                target_playlist_id,
                is_new_playlist,
                total_tracks,
                matched_tracks,
                added_tracks_count,
                pre_existing_count,
                is_rolled_back: is_rolled_back != 0,
                created_at,
                audit_status: None,
            })
        })
        .map_err(|e| format!("Failed to read history query: {}", e))?;

    let mut entries = Vec::new();
    for row in rows {
        if let Ok(entry) = row {
            entries.push(entry);
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Playlist, SourceTrack};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockRollbackProvider {
        deleted_playlist_id: Arc<Mutex<Option<String>>>,
        removed_track_ids: Arc<Mutex<Vec<String>>>,
        is_deleted: Arc<AtomicBool>,
    }

    #[async_trait]
    impl MusicProvider for MockRollbackProvider {
        async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
            Ok(vec![])
        }
        async fn get_playlist_tracks(
            &self,
            _playlist_id: &str,
            _tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
        ) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn search_track(&self, _query: &str) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn create_playlist(&self, _title: &str, _description: Option<&str>) -> Result<String, String> {
            Ok("test_pl".to_string())
        }
        async fn add_tracks_to_playlist(&self, _playlist_id: &str, _track_ids: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn remove_tracks_from_playlist(&self, _playlist_id: &str, track_ids: &[String]) -> Result<(), String> {
            self.removed_track_ids.lock().unwrap().extend_from_slice(track_ids);
            Ok(())
        }
        async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
            *self.deleted_playlist_id.lock().unwrap() = Some(playlist_id.to_string());
            self.is_deleted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        crate::storage::schema::create_tables(&conn).expect("create tables");
        conn
    }

    #[tokio::test]
    async fn test_snapshot_lifecycle_and_rollback_tracks_only() {
        let conn = setup_test_db();
        let db = Arc::new(Mutex::new(conn));

        let job_id = "job_test_123";
        let source_service = "spotify";
        let target_service = "ytmusic";
        let source_playlist_name = "My Summer Vibes";
        let target_playlist_id = "PL_dest_999";
        let target_playlist_name = "My Summer Vibes";
        let is_new_playlist = false;
        let pre_existing = vec!["track_pre_1".to_string()];
        let added = vec!["track_add_1".to_string(), "track_add_2".to_string()];

        let snapshot_id = {
            let conn = db.lock().unwrap();
            create_pre_mutation_snapshot(
                &conn,
                job_id,
                source_service,
                target_service,
                source_playlist_name,
                target_playlist_id,
                target_playlist_name,
                is_new_playlist,
                2,
                2,
                pre_existing.clone(),
                added.clone(),
            )
            .expect("create snapshot")
        };

        // Assert history returns snapshot
        {
            let conn = db.lock().unwrap();
            let history = get_transfer_history(&conn).expect("get history");
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].job_id, job_id);
            assert_eq!(history[0].snapshot_id, snapshot_id);
            assert_eq!(history[0].is_rolled_back, false);
            assert_eq!(history[0].added_tracks_count, 2);
            assert_eq!(history[0].pre_existing_count, 1);
        }

        // Test rollback
        let provider = MockRollbackProvider {
            deleted_playlist_id: Arc::new(Mutex::new(None)),
            removed_track_ids: Arc::new(Mutex::new(Vec::new())),
            is_deleted: Arc::new(AtomicBool::new(false)),
        };

        rollback_snapshot(&db, &snapshot_id, false, &provider)
            .await
            .expect("rollback");

        assert_eq!(provider.is_deleted.load(Ordering::SeqCst), false);
        let removed = provider.removed_track_ids.lock().unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0], "track_add_1");
        assert_eq!(removed[1], "track_add_2");

        // Verify marked as rolled back
        {
            let conn = db.lock().unwrap();
            let history = get_transfer_history(&conn).expect("get history");
            assert_eq!(history[0].is_rolled_back, true);
        }
    }

    #[tokio::test]
    async fn test_rollback_delete_entire_new_playlist() {
        let conn = setup_test_db();
        let db = Arc::new(Mutex::new(conn));

        let job_id = "job_test_456";
        let snapshot_id = {
            let conn = db.lock().unwrap();
            create_pre_mutation_snapshot(
                &conn,
                job_id,
                "spotify",
                "ytmusic",
                "New Playlist",
                "PL_new_777",
                "New Playlist",
                true, // is_new_playlist
                1,
                1,
                vec![],
                vec!["track_1".to_string()],
            )
            .expect("create snapshot")
        };

        let provider = MockRollbackProvider {
            deleted_playlist_id: Arc::new(Mutex::new(None)),
            removed_track_ids: Arc::new(Mutex::new(Vec::new())),
            is_deleted: Arc::new(AtomicBool::new(false)),
        };

        // When delete_entire_playlist is true and is_new_playlist is true, delete_playlist is invoked
        rollback_snapshot(&db, &snapshot_id, true, &provider)
            .await
            .expect("rollback entire playlist");

        assert_eq!(provider.is_deleted.load(Ordering::SeqCst), true);
        assert_eq!(
            provider.deleted_playlist_id.lock().unwrap().as_deref(),
            Some("PL_new_777")
        );
    }
}
