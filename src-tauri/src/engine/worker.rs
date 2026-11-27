use crate::engine::lifecycle::JobState;
pub use crate::engine::lifecycle::TransferControl;
#[cfg(test)]
use crate::engine::snapshot::audit_playlist;
use crate::engine::snapshot::{
    create_pre_mutation_snapshot, record_operation_intent, update_operation_outcome, AuditResult,
    OperationOutcome,
};
use crate::models::SourceTrack;
use crate::providers::contracts::{MutationOutcome, OccurrenceOutcome, OccurrenceStatus};
use crate::providers::error::{retry_delay_ms, ProviderErrorKind, SendCertainty};
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
    pub counts: crate::engine::recovery::RecoveryCounts,
    pub verification: String,
    pub error: Option<String>,
}

pub fn resolve_concurrency(_target_service: &str, _requested_concurrency: usize) -> usize {
    // Appends are non-commutative. Search/matching remains concurrent.
    1
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

async fn pace_batch(control: &TransferControl) {
    control
        .interruptible_delay(std::time::Duration::from_millis(300))
        .await;
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

struct PlannedBatch {
    operation_index: usize,
    source_indices: Vec<usize>,
    tracks: Vec<SourceTrack>,
}

fn plan_batches(
    conn: &rusqlite::Connection,
    config: &BatchTransferConfig,
    before: &[SourceTrack],
) -> Result<Vec<PlannedBatch>, String> {
    let mut pool = before.to_vec();
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    for (index, track) in config.tracks.iter().enumerate() {
        if config.skip_duplicates && is_duplicate_track(track, &pool) {
            skipped.push(index);
        } else {
            pool.push(track.clone());
            selected.push((index, track.clone()));
        }
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE transfer_jobs SET skipped_indices_json=?1 WHERE job_id=?2",
        rusqlite::params![
            serde_json::to_string(&skipped).map_err(|e| e.to_string())?,
            config.job_id
        ],
    )
    .map_err(|e| e.to_string())?;
    let size = if config.target_service == "ytmusic" {
        10
    } else {
        25
    };
    let mut batches = Vec::new();
    for (operation_index, chunk) in selected.chunks(size).enumerate() {
        let source_indices: Vec<usize> = chunk.iter().map(|p| p.0).collect();
        let tracks: Vec<SourceTrack> = chunk.iter().map(|p| p.1.clone()).collect();
        let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
        let id = record_operation_intent(&tx, &config.job_id, operation_index, &ids)?;
        tx.execute("UPDATE transfer_operations SET evidence_version=2,source_indices_json=?1 WHERE operation_id=?2",rusqlite::params![serde_json::to_string(&source_indices).map_err(|e|e.to_string())?,id]).map_err(|e|e.to_string())?;
        batches.push(PlannedBatch {
            operation_index,
            source_indices,
            tracks,
        });
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(batches)
}

struct ChunkWorkerContext {
    job_id: String,
    target_id: String,
    total: usize,
    chunk_idx: usize,
    worker_id: usize,
    chunk: Vec<SourceTrack>,
    skip_duplicates: bool,
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

fn checkpoint_cancelled_unsent(conn: &rusqlite::Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE transfer_operations SET status='failed',certainty='failed' WHERE operation_id=?1 AND sent_at IS NULL AND status='pending'",[id]).map_err(|_|"Cancellation checkpoint unavailable")?;
    Ok(())
}

/// Persist intent before the send and a checkpoint before reporting success.
/// Transport/partial outcomes are never replayed automatically. Presence in a
/// playlist cannot establish whether this operation added a new occurrence.
#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) async fn journaled_add(
    db: &std::sync::Mutex<rusqlite::Connection>,
    provider: &dyn MusicProvider,
    job_id: &str,
    target_id: &str,
    operation_index: usize,
    ids: &[String],
    retry_budget: &AtomicU32,
    control: &TransferControl,
) -> Result<bool, String> {
    journaled_add_with_options(
        db,
        provider,
        job_id,
        target_id,
        operation_index,
        ids,
        retry_budget,
        control,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn journaled_add_with_options(
    db: &std::sync::Mutex<rusqlite::Connection>,
    provider: &dyn MusicProvider,
    job_id: &str,
    target_id: &str,
    operation_index: usize,
    ids: &[String],
    retry_budget: &AtomicU32,
    control: &TransferControl,
    skip_duplicates: bool,
) -> Result<bool, String> {
    let operation_id = {
        let conn = db
            .lock()
            .map_err(|_| "Database lock unavailable; no mutation sent")?;
        use rusqlite::OptionalExtension;
        let id = format!("op_{}_{}", job_id, operation_index);
        let existing: Option<(String, Option<String>, Option<i64>)> = conn
            .query_row(
                "SELECT status,payload_json,sent_at FROM transfer_operations WHERE operation_id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((status, payload, sent)) = existing {
            let recorded: Vec<String> =
                serde_json::from_str(payload.as_deref().ok_or("Missing durable payload")?)
                    .map_err(|e| e.to_string())?;
            if status != "pending" || sent.is_some() || recorded != ids {
                return Err(
                    "Recorded operation cannot be replayed; inspect recovery evidence".into(),
                );
            }
            id
        } else {
            let id = record_operation_intent(&conn, job_id, operation_index, ids)?;
            conn.execute(
                "UPDATE transfer_operations SET evidence_version=2 WHERE operation_id=?1",
                [&id],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };
    #[cfg(test)]
    safety_tests::crash_point("intent");
    let previous_attempts: u32 = db
        .lock()
        .map_err(|_| "Attempt metadata unavailable")?
        .query_row(
            "SELECT attempt_count FROM transfer_operations WHERE operation_id=?1",
            [&operation_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut attempt = 0;
    loop {
        let Some(permit) = control.reserve_mutation().await? else {
            let conn = db
                .lock()
                .map_err(|_| "Cancelled mutation checkpoint unavailable")?;
            checkpoint_cancelled_unsent(&conn, &operation_id)?;
            return Ok(false);
        };
        if let Err(error) = provider.check_session().await {
            let conn = db
                .lock()
                .map_err(|_| "Preflight outcome checkpoint unavailable")?;
            conn.execute("UPDATE transfer_operations SET provider_response=?1,status='failed',certainty='failed',updated_at=?2 WHERE operation_id=?3",rusqlite::params![serde_json::to_string(&error).map_err(|e|e.to_string())?,chrono::Utc::now().timestamp(),operation_id]).map_err(|_|"Preflight failure checkpoint unavailable")?;
            return Err(error.to_string());
        }
        let fast_before = if operation_index > 0 {
            let revision_and_expected = {
                let conn = db
                    .lock()
                    .map_err(|_| "Mutation preflight storage lock unavailable")?;
                let rev = crate::engine::snapshot::latest_playlist_revision(&conn, job_id)?;
                if rev.is_some() {
                    let snapshot =
                        crate::engine::snapshot::load_snapshot(&conn, &format!("snap_{}", job_id))?;
                    let mut exp = snapshot.pre_existing_track_ids;
                    exp.extend(snapshot.added_track_ids);
                    Some((rev, exp))
                } else {
                    None
                }
            };
            if let Some((Some(expected_rev), expected_ids)) = revision_and_expected {
                let live_rev = provider
                    .playlist_revision(target_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if live_rev.as_deref() == Some(expected_rev.as_str()) {
                    Some(crate::providers::contracts::PlaylistRead {
                        occurrences: expected_ids
                            .into_iter()
                            .enumerate()
                            .map(
                                |(index, id)| crate::providers::contracts::PlaylistOccurrence {
                                    index,
                                    track: SourceTrack {
                                        id,
                                        ..SourceTrack::unavailable(index)
                                    },
                                    remote_id: None,
                                },
                            )
                            .collect(),
                        revision: Some(expected_rev),
                        complete: true,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let before = match fast_before {
            Some(read) => read,
            None => provider
                .read_playlist(target_id)
                .await
                .map_err(|e| e.to_string())?,
        };
        if control.is_stopping() {
            let conn = db
                .lock()
                .map_err(|_| "Cancelled preflight checkpoint unavailable")?;
            checkpoint_cancelled_unsent(&conn, &operation_id)?;
            drop(conn);
            permit.checkpointed()?;
            return Ok(false);
        }
        if control.snapshot()?.state != JobState::Running {
            permit.checkpointed()?;
            continue;
        }
        if !before.complete {
            return Err("Incomplete destination baseline; no mutation sent".into());
        }
        {
            let conn = db
                .lock()
                .map_err(|_| "Mutation send marker unavailable; no send")?;
            if let Some(revision) =
                crate::engine::snapshot::latest_playlist_revision(&conn, job_id)?
            {
                if before.revision.as_deref() != Some(revision.as_str()) {
                    return Err("Destination revision changed since the durable checkpoint; no mutation sent".into());
                }
            }
            let snapshot =
                crate::engine::snapshot::load_snapshot(&conn, &format!("snap_{}", job_id))?;
            let mut expected = snapshot.pre_existing_track_ids;
            expected.extend(snapshot.added_track_ids);
            let canonical = |ids: Vec<String>| {
                ids.into_iter()
                    .map(|id| id.trim_start_matches("spotify:track:").to_string())
                    .collect::<Vec<_>>()
            };
            if canonical(before.ids()) != canonical(expected) {
                return Err("Destination changed since the durable baseline; no mutation sent, manual review required".into());
            }
            conn.execute("UPDATE transfer_operations SET before_json=?1,evidence_version=2 WHERE operation_id=?2",rusqlite::params![serde_json::to_string(&before).map_err(|e|e.to_string())?,operation_id]).map_err(|_|"Before-state checkpoint failed; no send")?;
            conn.execute(
                "UPDATE transfer_operations SET sent_at=?1 WHERE operation_id=?2",
                rusqlite::params![chrono::Utc::now().timestamp(), operation_id],
            )
            .map_err(|_| "Mutation send marker failed; no send")?;
        }
        let response = provider
            .add_occurrences_with_options(target_id, ids, skip_duplicates)
            .await;
        #[cfg(test)]
        safety_tests::crash_point("response");
        match response {
            Ok(mut outcome) => {
                if !outcome.matches_payload(ids) {
                    outcome = MutationOutcome {
                        occurrences: ids
                            .iter()
                            .enumerate()
                            .map(|(index, id)| OccurrenceOutcome {
                                index,
                                track_id: id.clone(),
                                status: OccurrenceStatus::Uncertain,
                                remote_id: None,
                            })
                            .collect(),
                        revision: None,
                    };
                }
                let complete = outcome.all_accepted();
                {
                    let conn = db.lock().map_err(|_| {
                        "Remote write accepted but checkpoint unavailable; recovery required"
                    })?;
                    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                    update_operation_outcome(
                        &tx,
                        &operation_id,
                        if complete {
                            OperationOutcome::Confirmed
                        } else {
                            OperationOutcome::Ambiguous
                        },
                        previous_attempts + attempt + 1,
                        Some("accepted"),
                    )
                    .map_err(|_| {
                        "Remote write accepted but checkpoint failed; recovery required".to_string()
                    })?;
                    tx.execute(
                        "UPDATE transfer_operations SET outcome_json=?1 WHERE operation_id=?2",
                        rusqlite::params![
                            serde_json::to_string(&outcome).map_err(|e| e.to_string())?,
                            operation_id
                        ],
                    )
                    .map_err(|_| "Outcome evidence checkpoint failed; recovery required")?;
                    tx.commit()
                        .map_err(|_| "Outcome evidence commit failed; recovery required")?;
                }
                #[cfg(test)]
                safety_tests::crash_point("checkpoint");
                if !complete {
                    control.fail("Partial or uncertain mutation; accepted occurrences saved, reconcile before retry")?;
                }
                permit.checkpointed()?;
                if !complete {
                    return Err("Partial or uncertain mutation; reconcile recorded occurrences before retry".into());
                }
                return Ok(true);
            }
            Err(error) => {
                let kind = error.kind;
                let rejected = matches!(
                    error.certainty,
                    SendCertainty::NotSent | SendCertainty::Rejected
                );
                {
                    let conn = db
                        .lock()
                        .map_err(|_| "Mutation outcome could not be saved; recovery required")?;
                    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                    update_operation_outcome(
                        &tx,
                        &operation_id,
                        if rejected {
                            OperationOutcome::Failed
                        } else {
                            OperationOutcome::Ambiguous
                        },
                        previous_attempts + attempt + 1,
                        Some(if rejected {
                            "provider rejected mutation"
                        } else {
                            "remote outcome uncertain"
                        }),
                    )?;
                    let outcome = MutationOutcome {
                        occurrences: ids
                            .iter()
                            .enumerate()
                            .map(|(index, id)| OccurrenceOutcome {
                                index,
                                track_id: id.clone(),
                                status: if rejected {
                                    OccurrenceStatus::Rejected
                                } else {
                                    OccurrenceStatus::Uncertain
                                },
                                remote_id: None,
                            })
                            .collect(),
                        revision: None,
                    };
                    tx.execute("UPDATE transfer_operations SET outcome_json=?1,provider_response=?2 WHERE operation_id=?3",rusqlite::params![serde_json::to_string(&outcome).map_err(|e|e.to_string())?,serde_json::to_string(&error).map_err(|e|e.to_string())?,operation_id]).map_err(|_|"Failure evidence checkpoint failed")?;
                    tx.commit()
                        .map_err(|_| "Failure evidence commit failed; recovery required")?;
                }
                if !rejected {
                    control.fail("Remote mutation outcome is uncertain. Automatic replay stopped; manual reconciliation required.")?;
                }
                permit.checkpointed()?;
                if kind == ProviderErrorKind::RateLimited
                    && error.certainty == SendCertainty::Rejected
                    && attempt < 5
                    && consume_retry_budget(retry_budget)
                {
                    let delay = retry_delay_ms(attempt, error.retry_after_ms);
                    if delay > 30_000 {
                        return Err("Provider retry delay exceeds automatic wait limit; operation deferred, retry later".into());
                    }
                    control
                        .interruptible_delay(std::time::Duration::from_millis(delay))
                        .await;
                    attempt += 1;
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
}

async fn process_chunk(ctx: ChunkWorkerContext) -> Result<(), String> {
    ctx.control.wait_until_ready().await?;
    if ctx.control.is_stopping() {
        return Ok(());
    }
    let ids: Vec<String> = ctx.chunk.iter().map(|track| track.id.clone()).collect();
    let start = std::time::Instant::now();
    match journaled_add_with_options(
        &ctx.db,
        ctx.provider.as_ref().as_ref(),
        &ctx.job_id,
        &ctx.target_id,
        ctx.chunk_idx,
        &ids,
        &ctx.retry_budget,
        &ctx.control,
        ctx.skip_duplicates,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(error) => {
            let log = TransferLogPayload {
                job_id: ctx.job_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
                worker_id: ctx.worker_id,
                message: format!("Track addition failed: {}", error),
                level: "error".to_string(),
            };
            dispatch_transfer_log(log, |payload| {
                let _ = ctx.app.emit("transfer:log", payload);
            });
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
    provider.check_session().await.map_err(|e| e.to_string())?;
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
    let baseline = if config.is_new_playlist {
        crate::providers::contracts::PlaylistRead {
            occurrences: vec![],
            revision: None,
            complete: true,
        }
    } else {
        provider
            .read_playlist(&initial_target)
            .await
            .map_err(|_| "Destination baseline read failed. No mutation was sent.")?
    };
    if !baseline.complete {
        return Err("Incomplete destination baseline; no mutation sent".into());
    }
    let pre_existing_tracks: Vec<SourceTrack> = baseline
        .occurrences
        .iter()
        .map(|o| o.track.clone())
        .collect();
    if config.target_service == "ytmusic" && !config.skip_duplicates {
        let mut seen: std::collections::HashSet<&str> =
            pre_existing_tracks.iter().map(|t| t.id.as_str()).collect();
        if config
            .tracks
            .iter()
            .any(|track| !seen.insert(track.id.as_str()))
        {
            return Err("YouTube Music duplicate occurrence preservation is unsupported. Enable Skip duplicates or review the selection; no mutation was sent.".into());
        }
    }
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
            "UPDATE transfer_jobs SET manifest_json = ?1,target_fingerprint=?3 WHERE job_id = ?2",
            rusqlite::params![
                manifest,
                config.job_id,
                serde_json::json!({"revision":baseline.revision,"ids":baseline.ids()}).to_string()
            ],
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
            conn.execute("UPDATE transfer_operations SET operation_type = 'create_playlist',evidence_version=2 WHERE operation_id = ?1", [&id])
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
        let id = match provider
            .create_playlist_checked(&target_name, Some("Transferred with SoundShift"))
            .await
        {
            Ok(id) => id,
            Err(err) => {
                let is_rejected = matches!(
                    err.certainty,
                    crate::providers::error::SendCertainty::NotSent
                        | crate::providers::error::SendCertainty::Rejected
                );
                let outcome = if is_rejected {
                    OperationOutcome::Failed
                } else {
                    OperationOutcome::Ambiguous
                };
                if let Ok(conn) = db.lock() {
                    let payload =
                        serde_json::to_string(&err).unwrap_or_else(|_| err.detail.clone());
                    let _ =
                        update_operation_outcome(&conn, &operation_id, outcome, 1, Some(&payload));
                }
                if is_rejected {
                    return Err(crate::diagnostics::sanitize_for_log(&format!(
                        "Playlist creation failed: {}",
                        err.detail
                    )));
                }
                return Err(crate::diagnostics::sanitize_for_log(&format!(
                    "Playlist creation outcome uncertain: {}. Review the provider before retrying.",
                    err.detail
                )));
            }
        };
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
            match prepare_transfer(&self.db, provider.as_ref().as_ref(), &config, &control).await {
                Ok(prepared) => prepared,
                Err(err) => {
                    let log = TransferLogPayload {
                        job_id: config.job_id.clone(),
                        timestamp: chrono::Utc::now().timestamp(),
                        worker_id: 0,
                        message: crate::diagnostics::sanitize_for_log(&err),
                        level: "error".to_string(),
                    };
                    dispatch_transfer_log(log, |payload| {
                        let _ = self.app.emit("transfer:log", payload);
                    });
                    return Err(err);
                }
            };
        let pre_existing_ids: Vec<String> = pre_existing_tracks
            .iter()
            .map(|track| track.id.clone())
            .collect();

        let batches = {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            plan_batches(&conn, &config, &pre_existing_tracks)?
        };
        let selected_count: usize = batches.iter().map(|b| b.tracks.len()).sum();
        let skipped_duplicates = total - selected_count;

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
        let added_ids = Arc::new(std::sync::Mutex::new(Vec::with_capacity(selected_count)));

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut join_set = tokio::task::JoinSet::new();
        let errors = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        for batch in batches {
            if batch.source_indices.len() != batch.tracks.len() {
                return Err("Invalid durable batch occurrence mapping".into());
            }
            let chunk_idx = batch.operation_index;
            let chunk = batch.tracks;
            if control.is_stopping() {
                break;
            }

            control.wait_until_ready().await?;
            if control.is_stopping() {
                break;
            }

            if chunk_idx > 0 && config.target_service == "ytmusic" {
                pace_batch(&control).await;
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
                skip_duplicates: config.skip_duplicates,
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
                job_id: config.job_id.clone(),
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
                counts: crate::engine::recovery::inspect_job(
                    &*self.db.lock().map_err(|_| "Summary storage unavailable")?,
                    &config.job_id,
                )?
                .counts,
                verification: "unavailable".into(),
                error: None,
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
        let revision = {
            let conn = self
                .db
                .lock()
                .map_err(|_| "Verification storage unavailable")?;
            crate::engine::snapshot::latest_playlist_revision(&conn, &config.job_id)?
        };
        let audit = crate::engine::snapshot::audit_playlist_at_revision(
            provider.as_ref().as_ref(),
            &target_id,
            &expected_ids,
            revision.as_deref(),
        )
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
            job_id: config.job_id.clone(),
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
            counts: crate::engine::recovery::inspect_job(
                &*self.db.lock().map_err(|_| "Summary storage unavailable")?,
                &config.job_id,
            )?
            .counts,
            verification: "verified".into(),
            error: None,
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
    fn test_resolve_concurrency_serializes_other_services() {
        assert_eq!(resolve_concurrency("spotify", 8), 1);
        assert_eq!(resolve_concurrency("spotify", 4), 1);
        assert_eq!(resolve_concurrency("spotify", 0), 1);
        assert_eq!(resolve_concurrency("spotify", 16), 1);
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
