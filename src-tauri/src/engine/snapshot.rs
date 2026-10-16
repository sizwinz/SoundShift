use crate::providers::traits::MusicProvider;
use serde::{Deserialize, Serialize};
use sha1::Digest;
use std::collections::HashMap;

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
    pub payload_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub job_id: String,
    pub state: String,
    pub last_error: Option<String>,
    pub recovery_required: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub backup_id: String,
    pub created_at: i64,
    pub database_path: String,
    pub wal_path: Option<String>,
    pub checksum: String,
    pub status: String,
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
    pub job_state: String,
    pub recovery_required: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOutcome {
    Confirmed,
    Ambiguous,
    Failed,
}

impl OperationOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Ambiguous => "ambiguous",
            Self::Failed => "failed",
        }
    }
}

pub fn record_operation_intent(
    conn: &rusqlite::Connection,
    job_id: &str,
    operation_index: usize,
    track_ids: &[String],
) -> Result<String, String> {
    let operation_id = format!("op_{}_{}", job_id, operation_index);
    let payload = serde_json::to_string(track_ids).map_err(|e| e.to_string())?;
    let payload_hash = format!("{:x}", sha1::Sha1::digest(payload.as_bytes()));
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO transfer_operations
            (operation_id, job_id, operation_type, operation_index, payload_hash,
             certainty, status, attempt_count, created_at, updated_at, payload_json)
         VALUES (?1, ?2, 'add_tracks', ?3, ?4, 'ambiguous', 'pending', 0, ?5, ?5, ?6)",
        rusqlite::params![
            operation_id,
            job_id,
            operation_index as i64,
            payload_hash,
            now,
            payload
        ],
    )
    .map_err(|e| format!("Failed to journal operation intent: {}", e))?;
    Ok(operation_id)
}

pub fn update_operation_outcome(
    conn: &rusqlite::Connection,
    operation_id: &str,
    outcome: OperationOutcome,
    attempt_count: u32,
    provider_response: Option<&str>,
) -> Result<(), String> {
    let provider_response = provider_response.map(crate::diagnostics::sanitize_for_log);
    let changed = conn
        .execute(
            "UPDATE transfer_operations
         SET certainty = ?1, status = ?2, attempt_count = ?3,
             provider_response = ?4, updated_at = ?5
         WHERE operation_id = ?6",
            rusqlite::params![
                outcome.as_str(),
                outcome.as_str(),
                attempt_count,
                provider_response,
                chrono::Utc::now().timestamp(),
                operation_id
            ],
        )
        .map_err(|e| format!("Failed to journal operation outcome: {}", e))?;
    if changed != 1 {
        return Err("Operation checkpoint not found; recovery required".into());
    }
    Ok(())
}

/// Transactionally records transfer job and pre-mutation snapshot into SQLite before any mutations.
#[allow(clippy::too_many_arguments)]
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
    let snapshot_id = format!("snap_{}", job_id);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

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
        payload_hash: String::new(),
    };

    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize snapshot payload: {}", e))?;
    let payload_hash = format!("{:x}", sha1::Sha1::digest(payload_json.as_bytes()));
    let payload = SnapshotPayload {
        payload_hash: payload_hash.clone(),
        ..payload
    };
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize snapshot payload: {}", e))?;

    tx.execute(
        "INSERT INTO transfer_jobs (
            job_id, source_service, target_service, source_playlist_name, 
            target_playlist_id, target_playlist_name, total_tracks, matched_tracks, job_state,
            manifest_hash, source_fingerprint, target_fingerprint, recovery_required,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'planned', ?9, ?10, ?11, 0, ?12, ?13)",
        rusqlite::params![
            job_id,
            source_service,
            target_service,
            source_playlist_name,
            target_playlist_id,
            target_playlist_name,
            total_tracks,
            matched_tracks,
            payload_hash,
            format!("src:{}", source_service),
            format!("dst:{}:{}", target_service, target_playlist_id),
            now,
            now,
        ],
    )
    .map_err(|e| format!("Failed to insert transfer job: {}", e))?;

    tx.execute(
        "INSERT INTO transfer_snapshots (
            snapshot_id, job_id, mutation_payload, payload_hash, is_rolled_back, created_at
        ) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        rusqlite::params![snapshot_id, job_id, payload_json, payload_hash, now],
    )
    .map_err(|e| format!("Failed to insert transfer snapshot: {}", e))?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(snapshot_id)
}

/// Baselines stay immutable. Confirmed operation payloads are the durable additions ledger.
pub fn load_snapshot(
    conn: &rusqlite::Connection,
    snapshot_id: &str,
) -> Result<SnapshotPayload, String> {
    let (json, stored_hash, target_id): (String, String, String) = conn
        .query_row(
            "SELECT s.mutation_payload, s.payload_hash, j.target_playlist_id
         FROM transfer_snapshots s JOIN transfer_jobs j ON j.job_id = s.job_id
         WHERE snapshot_id = ?1",
            [snapshot_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let mut payload: SnapshotPayload = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let embedded_hash = std::mem::take(&mut payload.payload_hash);
    let unsigned = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let actual = format!("{:x}", sha1::Sha1::digest(unsigned.as_bytes()));
    if embedded_hash != actual || stored_hash != actual {
        return Err("Snapshot integrity check failed. Manual recovery is required.".into());
    }
    payload.payload_hash = embedded_hash;
    payload.target_playlist_id = target_id;
    let mut stmt = conn
        .prepare(
            "SELECT payload_json FROM transfer_operations WHERE job_id = ?1
         AND operation_type = 'add_tracks' AND status = 'confirmed' ORDER BY operation_index",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([&payload.job_id], |row| row.get::<_, Option<String>>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let json = row
            .map_err(|e| e.to_string())?
            .ok_or("Legacy operation lacks a recoverable payload; manual review required")?;
        let ids: Vec<String> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        payload.added_track_ids.extend(ids);
    }
    Ok(payload)
}

/// Validates snapshot payload checksum against recorded payload hash (DATA-06).
pub fn verify_snapshot_integrity(
    conn: &rusqlite::Connection,
    snapshot_id: &str,
) -> Result<bool, String> {
    match load_snapshot(conn, snapshot_id) {
        Ok(_) => Ok(true),
        Err(e) if e.contains("Snapshot integrity check failed") => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn set_job_state(
    conn: &rusqlite::Connection,
    job_id: &str,
    state: &str,
    error: Option<&str>,
) -> Result<(), String> {
    persist_job_state(conn, job_id, state, error).map(|_| ())
}

pub fn persist_job_state(
    conn: &rusqlite::Connection,
    job_id: &str,
    state: &str,
    error: Option<&str>,
) -> Result<u64, String> {
    let error = error.map(crate::diagnostics::sanitize_for_log);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let changed = tx
        .execute(
            "UPDATE transfer_jobs SET job_state = ?1, recovery_required = ?2, last_error = ?3,
         updated_at = ?4 WHERE job_id = ?5",
            rusqlite::params![
                state,
                state == "recovery_required",
                error,
                chrono::Utc::now().timestamp(),
                job_id
            ],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("Transfer job not found".into());
    }
    tx.execute(
        "INSERT INTO transfer_events (job_id, event_type, event_payload, event_seq, created_at)
         SELECT ?1, 'state_changed', ?2, COALESCE(MAX(event_seq), 0) + 1, ?3
         FROM transfer_events WHERE job_id = ?1",
        rusqlite::params![
            job_id,
            serde_json::json!({"state": state}).to_string(),
            chrono::Utc::now().timestamp()
        ],
    )
    .map_err(|e| e.to_string())?;
    let seq: u64 = tx
        .query_row(
            "SELECT MAX(event_seq) FROM transfer_events WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(seq)
}

/// Never resume mutations automatically after an interrupted process.
pub fn reconcile_incomplete_jobs(conn: &rusqlite::Connection) -> Result<(), String> {
    let ids = {
        let mut stmt = conn
            .prepare(
                "SELECT job_id FROM transfer_jobs WHERE job_state IN
             ('planned', 'running', 'pausing', 'paused', 'cancelling', 'rolling_back')",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for id in ids {
        set_job_state(
            conn,
            &id,
            "recovery_required",
            Some("Interrupted transfer. Review remote state before retrying or rolling back."),
        )?;
    }
    Ok(())
}

pub fn canonical_track_id(id: &str) -> &str {
    id.trim()
        .strip_prefix("spotify:track:")
        .or_else(|| id.trim().strip_prefix("ytmusic:video:"))
        .unwrap_or(id.trim())
}

fn missing_occurrences(expected: &[String], live: &[String]) -> Vec<String> {
    let mut counts = HashMap::new();
    for id in live {
        *counts.entry(canonical_track_id(id)).or_insert(0usize) += 1;
    }
    expected
        .iter()
        .filter(|id| {
            let count = counts.entry(canonical_track_id(id)).or_insert(0);
            if *count == 0 {
                true
            } else {
                *count -= 1;
                false
            }
        })
        .cloned()
        .collect()
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
            .query_row(rusqlite::params![snapshot_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| format!("Snapshot not found: {}", e))?;

        if is_rolled_back != 0 {
            return Err("This snapshot has already been rolled back".to_string());
        }

        let _ = payload_str;
        let payload = load_snapshot(&conn, snapshot_id)?;
        let uncertain: i64 = conn.query_row(
            "SELECT COUNT(*) FROM transfer_operations WHERE job_id = ?1 AND status IN ('pending', 'ambiguous')",
            [&payload.job_id], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if uncertain > 0 || payload.target_playlist_id.is_empty() {
            return Err(
                "Uncertain remote operations require manual reconciliation before rollback".into(),
            );
        }
        payload
    };

    if !(delete_entire_playlist && payload.is_new_playlist)
        && payload.added_track_ids.iter().any(|added| {
            payload
                .pre_existing_track_ids
                .iter()
                .any(|prior| canonical_track_id(prior) == canonical_track_id(added))
        })
    {
        return Err("Provider removal cannot safely distinguish added duplicates from pre-existing occurrences. Manual recovery required.".into());
    }

    let before: Vec<String> = provider
        .get_playlist_tracks(&payload.target_playlist_id, None)
        .await?
        .into_iter()
        .map(|track| track.id)
        .collect();
    let mut expected = payload.pre_existing_track_ids.clone();
    expected.extend(payload.added_track_ids.clone());
    if before.len() != expected.len() || !missing_occurrences(&expected, &before).is_empty() {
        return Err("Playlist changed since the transfer. No rollback mutation was sent; review remote changes first.".into());
    }
    let operation_id = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let id = record_operation_intent(
            &conn,
            &payload.job_id,
            usize::MAX / 2 - 1,
            &payload.added_track_ids,
        )?;
        conn.execute(
            "UPDATE transfer_operations SET operation_type = 'rollback' WHERE operation_id = ?1",
            [&id],
        )
        .map_err(|e| e.to_string())?;
        set_job_state(&conn, &payload.job_id, "rolling_back", None)?;
        id
    };

    if delete_entire_playlist && payload.is_new_playlist {
        provider
            .delete_playlist(&payload.target_playlist_id)
            .await?;
    } else {
        provider
            .remove_tracks_from_playlist(&payload.target_playlist_id, &payload.added_track_ids)
            .await?;

        let live_tracks = provider
            .get_playlist_tracks(&payload.target_playlist_id, None)
            .await
            .map_err(|e| format!("Rollback verification read failed: {}", e))?;
        let remaining: Vec<String> = live_tracks.into_iter().map(|track| track.id).collect();
        let canonical = |ids: &[String]| {
            ids.iter()
                .map(|id| canonical_track_id(id).to_string())
                .collect::<Vec<_>>()
        };
        if canonical(&remaining) != canonical(&payload.pre_existing_track_ids) {
            return Err("Rollback post-state differs from the original ordered playlist; manual review required".into());
        }
    }

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        update_operation_outcome(
            &tx,
            &operation_id,
            OperationOutcome::Confirmed,
            1,
            Some("rollback accepted and track restoration checked"),
        )?;
        tx.execute(
            "UPDATE transfer_snapshots SET is_rolled_back = 1 WHERE snapshot_id = ?1",
            rusqlite::params![snapshot_id],
        )
        .map_err(|e| format!("Failed to update rollback state in SQLite: {}", e))?;
        tx.commit().map_err(|e| e.to_string())?;
        set_job_state(&conn, &payload.job_id, "rolled_back", None)?;
    }

    Ok(())
}

/// Audits target playlist contents against expected added tracks and computes verification state.
pub async fn audit_playlist(
    provider: &dyn MusicProvider,
    target_playlist_id: &str,
    expected_track_ids: &[String],
) -> Result<AuditResult, String> {
    let mut last_missing_ids = Vec::new();

    // Provider playlist reads can be eventually consistent immediately after a
    // batch mutation. Retry the complete paginated read before reporting a
    // discrepancy to the user.
    for attempt in 0..3 {
        let live_tracks = provider
            .get_playlist_tracks(target_playlist_id, None)
            .await?;
        let live_ids: Vec<String> = live_tracks.into_iter().map(|t| t.id).collect();
        let missing_ids = missing_occurrences(expected_track_ids, &live_ids);

        if missing_ids.is_empty() || attempt == 2 {
            let total_expected = expected_track_ids.len();
            let total_found = total_expected.saturating_sub(missing_ids.len());
            return Ok(AuditResult {
                is_verified: missing_ids.is_empty(),
                total_expected,
                total_found,
                missing_ids,
            });
        }

        last_missing_ids = missing_ids;
        tokio::time::sleep(std::time::Duration::from_millis(750 * (attempt + 1) as u64)).await;
    }

    let total_expected = expected_track_ids.len();
    Ok(AuditResult {
        is_verified: false,
        total_expected,
        total_found: total_expected.saturating_sub(last_missing_ids.len()),
        missing_ids: last_missing_ids,
    })
}

/// Retrieves all past transfer jobs with snapshots from SQLite.
pub fn get_transfer_history(
    conn: &rusqlite::Connection,
) -> Result<Vec<TransferHistoryEntry>, String> {
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
            let added_tracks_count = payload
                .as_ref()
                .map(|p| p.added_track_ids.len())
                .unwrap_or(0);
            let pre_existing_count = payload
                .as_ref()
                .map(|p| p.pre_existing_track_ids.len())
                .unwrap_or(0);

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
                job_state: String::new(),
                recovery_required: false,
                last_error: None,
            })
        })
        .map_err(|e| format!("Failed to read history query: {}", e))?;

    let mut entries = Vec::new();
    for entry in rows {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let (state, recovery, error): (String, bool, Option<String>) = conn.query_row(
            "SELECT job_state, recovery_required, last_error FROM transfer_jobs WHERE job_id = ?1",
            [&entry.job_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|e| e.to_string())?;
        entry.job_state = state;
        entry.recovery_required = recovery;
        entry.last_error = error.as_deref().map(crate::diagnostics::sanitize_for_log);
        match load_snapshot(conn, &entry.snapshot_id) {
            Ok(payload) => entry.added_tracks_count = payload.added_track_ids.len(),
            Err(error) => {
                entry.recovery_required = true;
                entry.last_error = Some(error);
            }
        }
        entries.push(entry);
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
        initial_ids: Vec<String>,
        removal_noop: bool,
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
            let removed = self.removed_track_ids.lock().unwrap();
            Ok(self
                .initial_ids
                .iter()
                .filter(|id| self.removal_noop || !removed.contains(id))
                .map(|id| SourceTrack {
                    id: id.clone(),
                    title: "Track".into(),
                    artists: vec![],
                    album: None,
                    duration_ms: 1000,
                    isrc: None,
                    is_explicit: false,
                    is_playable: true,
                    preview_url: None,
                    thumbnail_url: None,
                })
                .collect())
        }
        async fn search_track(&self, _query: &str) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn create_playlist(
            &self,
            _title: &str,
            _description: Option<&str>,
        ) -> Result<String, String> {
            Ok("test_pl".to_string())
        }
        async fn add_tracks_to_playlist(
            &self,
            _playlist_id: &str,
            _track_ids: &[String],
        ) -> Result<(), String> {
            Ok(())
        }
        async fn remove_tracks_from_playlist(
            &self,
            _playlist_id: &str,
            track_ids: &[String],
        ) -> Result<(), String> {
            self.removed_track_ids
                .lock()
                .unwrap()
                .extend_from_slice(track_ids);
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
            initial_ids: vec![
                "track_pre_1".into(),
                "track_add_1".into(),
                "track_add_2".into(),
            ],
            removal_noop: false,
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
            initial_ids: vec!["track_1".into()],
            removal_noop: false,
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

    #[tokio::test]
    async fn test_rollback_fails_if_remote_state_changed() {
        let conn = setup_test_db();
        let db = Arc::new(Mutex::new(conn));

        let job_id = "job_test_diff";
        let snapshot_id = {
            let conn = db.lock().unwrap();
            create_pre_mutation_snapshot(
                &conn,
                job_id,
                "spotify",
                "ytmusic",
                "Playlist",
                "PL_diff_1",
                "Playlist",
                false,
                1,
                1,
                vec!["pre_1".into()],
                vec!["add_1".into()],
            )
            .expect("create snapshot")
        };

        // Live provider has an unexpected track "external_user_added"
        let provider = MockRollbackProvider {
            deleted_playlist_id: Arc::new(Mutex::new(None)),
            removed_track_ids: Arc::new(Mutex::new(Vec::new())),
            is_deleted: Arc::new(AtomicBool::new(false)),
            initial_ids: vec!["pre_1".into(), "add_1".into(), "external_user_added".into()],
            removal_noop: false,
        };

        let result = rollback_snapshot(&db, &snapshot_id, false, &provider).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Playlist changed since the transfer"));
    }

    #[tokio::test]
    async fn test_rollback_fails_if_uncertain_operations_exist() {
        let conn = setup_test_db();
        let db = Arc::new(Mutex::new(conn));

        let job_id = "job_test_uncertain";
        let snapshot_id = {
            let conn = db.lock().unwrap();
            create_pre_mutation_snapshot(
                &conn,
                job_id,
                "spotify",
                "ytmusic",
                "Playlist",
                "PL_unc_1",
                "Playlist",
                false,
                1,
                1,
                vec!["pre_1".into()],
                vec!["add_1".into()],
            )
            .expect("create snapshot")
        };

        // Record a pending operation
        {
            let conn = db.lock().unwrap();
            record_operation_intent(&conn, job_id, 0, &["add_1".into()]).expect("record intent");
        }

        let provider = MockRollbackProvider {
            deleted_playlist_id: Arc::new(Mutex::new(None)),
            removed_track_ids: Arc::new(Mutex::new(Vec::new())),
            is_deleted: Arc::new(AtomicBool::new(false)),
            initial_ids: vec!["pre_1".into(), "add_1".into()],
            removal_noop: false,
        };

        let result = rollback_snapshot(&db, &snapshot_id, false, &provider).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Uncertain remote operations require manual reconciliation"));
    }

    #[test]
    fn test_reconcile_incomplete_jobs_transitions_interrupted_states() {
        let conn = setup_test_db();
        let job_id = "job_interrupted";
        create_pre_mutation_snapshot(
            &conn,
            job_id,
            "spotify",
            "ytmusic",
            "Interrupted",
            "PL_int_1",
            "Interrupted",
            false,
            1,
            1,
            vec![],
            vec![],
        )
        .expect("create snapshot");

        set_job_state(&conn, job_id, "running", None).expect("set running");

        // Reconcile
        reconcile_incomplete_jobs(&conn).expect("reconcile");

        let (state, recovery): (String, bool) = conn
            .query_row(
                "SELECT job_state, recovery_required FROM transfer_jobs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("query job");

        assert_eq!(state, "recovery_required");
        assert!(recovery);
    }

    #[test]
    fn test_verify_snapshot_integrity() {
        let conn = setup_test_db();
        let job_id = "job_integrity";
        let snap_id = create_pre_mutation_snapshot(
            &conn,
            job_id,
            "spotify",
            "ytmusic",
            "Integrity",
            "PL_int_2",
            "Integrity",
            false,
            1,
            1,
            vec![],
            vec![],
        )
        .expect("create snapshot");

        let is_valid = verify_snapshot_integrity(&conn, &snap_id).expect("verify");
        assert!(is_valid);

        // Tamper with snapshot payload
        conn.execute(
            "UPDATE transfer_snapshots SET mutation_payload = '{\"tampered\": true}' WHERE snapshot_id = ?1",
            [&snap_id],
        )
        .expect("tamper payload");

        let tampered_result = verify_snapshot_integrity(&conn, &snap_id);
        assert!(tampered_result.is_err() || !tampered_result.unwrap());
    }

    #[test]
    fn test_missing_occurrences_multiset() {
        let expected = vec!["track_1".into(), "track_1".into(), "track_2".into()];
        let live = vec!["track_1".into(), "track_2".into()];
        let missing = missing_occurrences(&expected, &live);
        assert_eq!(missing, vec!["track_1".to_string()]);
    }
}
