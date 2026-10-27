//! Recovery is explicit and operates on durable identities, never UI track slices.
use crate::providers::contracts::{MutationOutcome, OccurrenceStatus, PlaylistRead};
use crate::providers::error::{ProviderError, SendCertainty};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryCounts {
    pub selected: usize,
    pub confirmed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub uncertain: usize,
    pub unsent: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedOperation {
    pub operation_id: String,
    pub operation_type: String,
    pub operation_index: usize,
    pub payload: Vec<String>,
    pub status: String,
    pub attempt_count: u32,
    pub evidence_version: i64,
    pub before: Option<PlaylistRead>,
    pub outcome: Option<MutationOutcome>,
    pub sent_at: Option<i64>,
    pub source_indices: Option<Vec<usize>>,
    pub error: Option<ProviderError>,
    pub retry_not_before_ms: Option<i64>,
}
impl RecordedOperation {
    pub fn can_retry(&self) -> bool {
        if self
            .retry_not_before_ms
            .is_some_and(|deadline| deadline > chrono::Utc::now().timestamp_millis())
        {
            return false;
        }
        if self.operation_type != "add_tracks"
            || self.status == "confirmed"
            || self.evidence_version != 2
        {
            return false;
        }
        if self.outcome.as_ref().is_some_and(|o| {
            o.occurrences.iter().any(|p| {
                matches!(
                    p.status,
                    OccurrenceStatus::Accepted
                        | OccurrenceStatus::Uncertain
                        | OccurrenceStatus::Skipped
                )
            })
        }) {
            return false;
        }
        (self.sent_at.is_none() && self.attempt_count == 0)
            || self.error.as_ref().is_some_and(|e| {
                matches!(
                    e.certainty,
                    SendCertainty::NotSent | SendCertainty::Rejected
                )
            })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInspection {
    pub job_id: String,
    pub state: String,
    pub event_seq: u64,
    pub counts: RecoveryCounts,
    pub verification: String,
    pub operations: Vec<RecordedOperation>,
    pub can_resume: bool,
    pub next_action: String,
}

pub fn inspect_job(conn: &Connection, job_id: &str) -> Result<JobInspection, String> {
    let (state,total,skipped,manifest,target):(String,usize,String,Option<String>,String)=conn.query_row(
        "SELECT job_state,total_tracks,skipped_indices_json,manifest_json,target_playlist_id FROM transfer_jobs WHERE job_id=?1",[job_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"Recorded job not found")?;
    let event_seq: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(event_seq),0) FROM transfer_events WHERE job_id=?1",
            [job_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut stmt=conn.prepare("SELECT operation_id,operation_type,operation_index,payload_json,status,attempt_count,evidence_version,before_json,outcome_json,sent_at,source_indices_json,provider_response,updated_at,payload_hash FROM transfer_operations WHERE job_id=?1 ORDER BY operation_index").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([job_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, u32>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, String>(13)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut operations = Vec::new();
    for row in rows {
        let (
            operation_id,
            operation_type,
            index,
            payload,
            status,
            attempt_count,
            evidence_version,
            before,
            outcome,
            sent_at,
            indices,
            error,
            updated_at,
            payload_hash,
        ) = row.map_err(|e| e.to_string())?;
        let error: Option<ProviderError> = error.and_then(|s| serde_json::from_str(&s).ok());
        let retry_not_before_ms = error.as_ref().and_then(|e| e.retry_after_ms).map(|delay| {
            updated_at
                .saturating_add(1)
                .saturating_mul(1000)
                .saturating_add(delay.min(i64::MAX as u64) as i64)
        });
        operations.push(RecordedOperation {
            operation_id,
            operation_type,
            operation_index: index as usize,
            payload: match payload {
                Some(json) => {
                    crate::engine::snapshot::decode_operation_payload(&json, &payload_hash)?
                }
                None if evidence_version == 0 => vec![],
                None => return Err("Mutation payload missing; manual review required".into()),
            },
            status,
            attempt_count,
            evidence_version,
            before: before
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| e.to_string())?,
            outcome: outcome
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| e.to_string())?,
            sent_at,
            source_indices: indices
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| e.to_string())?,
            error,
            retry_not_before_ms,
        });
    }
    for operation in &operations {
        if operation
            .outcome
            .as_ref()
            .is_some_and(|o| !o.matches_payload(&operation.payload))
        {
            return Err(
                "Occurrence evidence differs from durable payload; manual review required".into(),
            );
        }
    }
    let skipped: Vec<usize> = serde_json::from_str(&skipped).map_err(|e| e.to_string())?;
    let mut counts = RecoveryCounts {
        selected: total,
        skipped: skipped.len(),
        ..Default::default()
    };
    for op in operations
        .iter()
        .filter(|o| o.operation_type == "add_tracks")
    {
        if let Some(outcome) = &op.outcome {
            for occurrence in &outcome.occurrences {
                match occurrence.status {
                    OccurrenceStatus::Accepted => counts.confirmed += 1,
                    OccurrenceStatus::Skipped => counts.skipped += 1,
                    OccurrenceStatus::Rejected if op.attempt_count == 0 => {}
                    OccurrenceStatus::Rejected => counts.failed += 1,
                    OccurrenceStatus::Uncertain => counts.uncertain += 1,
                }
            }
        } else if op.status == "confirmed" {
            counts.confirmed += op.payload.len();
        } else if op.evidence_version == 2 && op.sent_at.is_none() && op.attempt_count == 0 {
        } else {
            counts.uncertain += op.payload.len();
        }
    }
    let accounted = counts.confirmed + counts.failed + counts.skipped + counts.uncertain;
    if accounted > total {
        return Err("Journal occurrence counts exceed manifest; manual review required".into());
    }
    counts.unsent = total - accounted;
    let used_retries: u32 = operations
        .iter()
        .map(|o| o.attempt_count.saturating_sub(1))
        .sum();
    let budget_allows_resume = used_retries < 20
        || operations
            .iter()
            .filter(|o| o.can_retry())
            .all(|o| o.attempt_count == 0);
    let can_resume = budget_allows_resume
        && manifest.is_some()
        && !target.is_empty()
        && counts.uncertain == 0
        && matches!(
            state.as_str(),
            "cancelled" | "recovery_required" | "manual_review"
        )
        && operations
            .iter()
            .any(|o| o.can_retry() && o.source_indices.is_some())
        && operations.iter().all(|o| {
            o.evidence_version == 2
                && (o.status == "confirmed" || (o.can_retry() && o.source_indices.is_some()))
        });
    let next_action = if can_resume {
        "Resume proven unsent or rejected operations".to_string()
    } else if let Some(deadline) = operations
        .iter()
        .filter_map(|o| o.retry_not_before_ms)
        .filter(|d| *d > chrono::Utc::now().timestamp_millis())
        .max()
    {
        format!("Provider deferred retry until Unix milliseconds {}; no automatic send before that time",deadline)
    } else {
        "Inspect recorded outcomes; unresolved or legacy evidence requires manual provider review"
            .into()
    };
    Ok(JobInspection {
        job_id: job_id.into(),
        verification: if matches!(state.as_str(), "completed_verified" | "rolled_back") {
            "verified"
        } else {
            "unavailable"
        }
        .into(),
        state,
        event_seq,
        counts,
        operations,
        can_resume,
        next_action,
    })
}

fn transition(
    conn: &Connection,
    job_id: &str,
    state: &str,
    emit: &impl Fn(crate::engine::lifecycle::LifecycleUpdate),
) -> Result<(), String> {
    let seq = crate::engine::snapshot::persist_job_state(conn, job_id, state, None)?;
    emit(crate::engine::lifecycle::LifecycleUpdate {
        job_id: job_id.into(),
        state: serde_json::from_value(serde_json::Value::String(state.into()))
            .map_err(|e| e.to_string())?,
        event_seq: seq,
    });
    Ok(())
}

pub async fn reconcile_job(
    db: &std::sync::Mutex<Connection>,
    provider: &dyn crate::providers::traits::MusicProvider,
    job_id: &str,
    emit: impl Fn(crate::engine::lifecycle::LifecycleUpdate),
) -> Result<JobInspection, String> {
    {
        let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
        let inspection = inspect_job(&conn, job_id)?;
        if matches!(
            inspection.state.as_str(),
            "completed_verified" | "rolled_back"
        ) {
            return Ok(inspection);
        }
        transition(&conn, job_id, "reconciling", &emit)?;
        #[cfg(test)]
        crash_point("reconcile_intent");
    }
    let read = provider
        .read_playlist(&{
            let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
            conn.query_row(
                "SELECT target_playlist_id FROM transfer_jobs WHERE job_id=?1",
                [job_id],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?
        })
        .await;
    #[cfg(test)]
    crash_point("reconcile_response");
    let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
    if read.is_err() {
        transition(&conn, job_id, "recovery_required", &emit)?;
        return Err("Reconciliation read failed; no remote mutation sent".into());
    }
    let live = read.map_err(|e| e.to_string())?;
    let inspection = inspect_job(&conn, job_id)?;
    for op in inspection.operations.iter().filter(|o| {
        o.operation_type == "add_tracks" && o.status != "confirmed" && o.evidence_version == 2
    }) {
        if let (Some(before), Some(receipt)) = (&op.before, &op.outcome) {
            if let Some(outcome) = reconcile_receipts(before, receipt, &live) {
                if !outcome.all_accepted() {
                    continue;
                }
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                crate::engine::snapshot::update_operation_outcome(
                    &tx,
                    &op.operation_id,
                    crate::engine::snapshot::OperationOutcome::Confirmed,
                    op.attempt_count,
                    Some("Reconciled by attributed occurrence receipts"),
                )?;
                tx.execute(
                    "UPDATE transfer_operations SET outcome_json=?1 WHERE operation_id=?2",
                    rusqlite::params![
                        serde_json::to_string(&outcome).map_err(|e| e.to_string())?,
                        op.operation_id
                    ],
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
            }
        }
    }
    // Neither membership nor absence alone attributes an operation to this job.
    #[cfg(test)]
    crash_point("reconcile_checkpoint");
    let result = inspect_job(&conn, job_id)?;
    let snapshot = crate::engine::snapshot::load_snapshot(&conn, &format!("snap_{}", job_id))?;
    let mut expected = snapshot.pre_existing_track_ids;
    expected.extend(snapshot.added_track_ids);
    let revision = crate::engine::snapshot::latest_playlist_revision(&conn, job_id)?;
    let verified = live.complete
        && live.ids() == expected
        && revision
            .as_ref()
            .is_none_or(|r| live.revision.as_ref() == Some(r))
        && result.counts.confirmed + result.counts.skipped == result.counts.selected
        && result
            .operations
            .iter()
            .all(|o| o.evidence_version == 2 && o.status == "confirmed");
    transition(
        &conn,
        job_id,
        if verified {
            "completed_verified"
        } else {
            "manual_review"
        },
        &emit,
    )?;
    inspect_job(&conn, job_id)
}

fn reconcile_receipts(
    before: &PlaylistRead,
    receipt: &MutationOutcome,
    live: &PlaylistRead,
) -> Option<MutationOutcome> {
    if !before.complete
        || !live.complete
        || before.occurrences.iter().any(|o| o.remote_id.is_none())
    {
        return None;
    }
    let additions: Vec<_> = receipt
        .occurrences
        .iter()
        .filter(|o| {
            matches!(
                o.status,
                OccurrenceStatus::Accepted | OccurrenceStatus::Uncertain
            )
        })
        .collect();
    if live.occurrences.len() != before.occurrences.len() + additions.len() {
        return None;
    }
    if before
        .occurrences
        .iter()
        .zip(&live.occurrences)
        .any(|(a, b)| a.track.id != b.track.id || a.remote_id != b.remote_id)
    {
        return None;
    }
    if receipt.revision.is_some() && receipt.revision != live.revision {
        return None;
    }
    let mut seen: std::collections::HashSet<String> = before
        .occurrences
        .iter()
        .filter_map(|o| o.remote_id.clone())
        .collect();
    for (receipt, occurrence) in additions
        .iter()
        .zip(live.occurrences.iter().skip(before.occurrences.len()))
    {
        let remote = receipt.remote_id.as_ref()?;
        if remote.is_empty()
            || !seen.insert(remote.clone())
            || occurrence.remote_id.as_ref() != Some(remote)
            || occurrence.track.id != receipt.track_id
        {
            return None;
        }
    }
    let mut result = receipt.clone();
    for occurrence in &mut result.occurrences {
        if occurrence.status == OccurrenceStatus::Uncertain {
            occurrence.status = OccurrenceStatus::Accepted;
        }
    }
    Some(result)
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

pub async fn resume_job(
    db: &std::sync::Mutex<Connection>,
    provider: &dyn crate::providers::traits::MusicProvider,
    job_id: &str,
    control: &crate::engine::lifecycle::TransferControl,
) -> Result<JobInspection, String> {
    let (inspection, target, config) = {
        let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
        let inspection = inspect_job(&conn, job_id)?;
        if !inspection.can_resume {
            return Err("Job has no safely resumable operations; reconcile or review recorded outcomes first".into());
        }
        let (target, manifest): (String, String) = conn
            .query_row(
                "SELECT target_playlist_id,manifest_json FROM transfer_jobs WHERE job_id=?1",
                [job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let config: crate::engine::worker::BatchTransferConfig = serde_json::from_str(&manifest)
            .map_err(|_| "Manifest unavailable; manual review required")?;
        let mut indices = std::collections::HashSet::new();
        for operation in inspection
            .operations
            .iter()
            .filter(|o| o.operation_type == "add_tracks")
        {
            let positions = operation
                .source_indices
                .as_ref()
                .ok_or("Legacy occurrence mapping missing; manual review required")?;
            if positions.len() != operation.payload.len()
                || positions.iter().zip(&operation.payload).any(|(i, id)| {
                    !indices.insert(*i) || config.tracks.get(*i).is_none_or(|track| track.id != *id)
                })
            {
                return Err("Invalid manifest occurrence mapping; no mutation sent".into());
            }
        }
        if indices.len() + inspection.counts.skipped != config.tracks.len() {
            return Err("Incomplete durable batch plan; manual review required".into());
        }
        (inspection, target, config)
    };
    let attempts: u32 = inspection
        .operations
        .iter()
        .map(|o| o.attempt_count.saturating_sub(1))
        .sum();
    let budget = std::sync::atomic::AtomicU32::new(20u32.saturating_sub(attempts));
    control.start()?;
    let work:Result<(),String>=async {
        for operation in inspection.operations.iter().filter(|o|o.can_retry()) {
            if operation.attempt_count>0 && budget.fetch_update(std::sync::atomic::Ordering::SeqCst,std::sync::atomic::Ordering::SeqCst,|n|n.checked_sub(1)).is_err() {return Err("Per-job retry budget exhausted; manual review required".into());}
            {
                let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;
                if operation.retry_not_before_ms.is_some_and(|deadline|chrono::Utc::now().timestamp_millis()<deadline) {return Err("Provider retry guidance still defers this operation; retry later".into());}
                conn.execute("UPDATE transfer_operations SET status='pending',sent_at=NULL WHERE operation_id=?1 AND status!='confirmed'",[&operation.operation_id]).map_err(|e|e.to_string())?;
            }
            if !crate::engine::worker::journaled_add(db,provider,job_id,&target,operation.operation_index,&operation.payload,&budget,control).await? {break;}
        }
        if control.is_stopping() {control.finish().await?;} else {
            let expected={let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;let snapshot=crate::engine::snapshot::load_snapshot(&conn,&format!("snap_{}",config.job_id))?;let mut ids=snapshot.pre_existing_track_ids;ids.extend(snapshot.added_track_ids);ids};
            let live=provider.read_playlist(&target).await.map_err(|e|e.to_string())?;
            let revision={let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;crate::engine::snapshot::latest_playlist_revision(&conn,job_id)?};
            if !live.complete || live.ids()!=expected || revision.as_ref().is_some_and(|r|live.revision.as_ref()!=Some(r)) {return Err("Resumed writes checkpointed; ordered verification failed, recovery required".into());}
            let counts=inspect_job(&*db.lock().map_err(|_|"Recovery storage lock unavailable")?,job_id)?.counts;
            if counts.confirmed+counts.skipped!=counts.selected {return Err("Unfinished operations remain; completion cannot be verified".into());}
            control.finish().await?;
        }
        Ok(())
    }.await;
    if let Err(error) = work {
        let _ = control.fail(&error);
        return Err(error);
    }
    inspect_job(
        &*db.lock().map_err(|_| "Recovery storage lock unavailable")?,
        job_id,
    )
}

/// Restores a proven baseline. An uncertain removal is inspected before any retry.
pub async fn rollback_recorded(
    db: &std::sync::Mutex<Connection>,
    snapshot_id: &str,
    delete: bool,
    provider: &dyn crate::providers::traits::MusicProvider,
    emit: impl Fn(crate::engine::lifecycle::LifecycleUpdate),
) -> Result<JobInspection, String> {
    use crate::engine::snapshot::{
        load_snapshot, record_operation_intent, update_operation_outcome, OperationOutcome,
    };
    let (snapshot, inspection, already) = {
        let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
        let snapshot = load_snapshot(&conn, snapshot_id)?;
        let inspection = inspect_job(&conn, &snapshot.job_id)?;
        let already: bool = conn
            .query_row(
                "SELECT is_rolled_back FROM transfer_snapshots WHERE snapshot_id=?1",
                [snapshot_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        (snapshot, inspection, already)
    };
    let job_id = &snapshot.job_id;
    if already {
        if inspection.state != "rolled_back"
            && inspection.operations.iter().any(|o| {
                o.operation_type.starts_with("rollback")
                    && o.status == "confirmed"
                    && o.evidence_version == 2
            })
        {
            let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
            transition(&conn, job_id, "rolled_back", &emit)?;
            return inspect_job(&conn, job_id);
        }
        return Ok(inspection);
    }
    if inspection.counts.uncertain > 0
        || inspection
            .operations
            .iter()
            .any(|o| o.operation_type == "create_playlist" && o.status != "confirmed")
    {
        return Err(
            "Uncertain remote operations require manual reconciliation before rollback".into(),
        );
    }
    provider.check_session().await.map_err(|e| e.to_string())?;
    let live = provider
        .read_playlist(&snapshot.target_playlist_id)
        .await
        .map_err(|e| e.to_string())?;
    if !live.complete {
        return Err("Incomplete rollback baseline; no mutation sent".into());
    }
    let mut expected = snapshot.pre_existing_track_ids.clone();
    expected.extend(snapshot.added_track_ids.clone());
    let prior = inspection
        .operations
        .iter()
        .find(|o| o.operation_type.starts_with("rollback"));
    let baseline = inspection
        .operations
        .iter()
        .find(|o| o.operation_type == "add_tracks")
        .and_then(|o| o.before.clone());
    let same_ids = |a: &[String], b: &[String]| {
        a.iter()
            .map(|s| s.trim_start_matches("spotify:track:"))
            .eq(b.iter().map(|s| s.trim_start_matches("spotify:track:")))
    };
    if !same_ids(&live.ids(), &expected)
        && !(prior.is_some() && same_ids(&live.ids(), &snapshot.pre_existing_track_ids))
    {
        return Err("Playlist changed since the transfer. No rollback mutation sent; manual review required".into());
    }
    let mut owned = Vec::new();
    for operation in inspection
        .operations
        .iter()
        .filter(|o| o.operation_type == "add_tracks")
    {
        if operation.evidence_version != 2 || operation.before.is_none() {
            return Err(
                "Legacy snapshot lacks occurrence attribution; manual review required".into(),
            );
        }
        let receipt = operation
            .outcome
            .as_ref()
            .ok_or("Legacy occurrence receipts unavailable; manual review required")?;
        for occurrence in receipt
            .occurrences
            .iter()
            .filter(|o| o.status == OccurrenceStatus::Accepted)
        {
            owned.push(
                occurrence
                    .remote_id
                    .clone()
                    .ok_or("Provider cannot attribute exact occurrences; manual review required")?,
            );
        }
    }
    if owned.len() != snapshot.added_track_ids.len()
        || owned.iter().collect::<std::collections::HashSet<_>>().len() != owned.len()
    {
        return Err(
            "Legacy snapshot lacks unique occurrence attribution; manual review required".into(),
        );
    }
    let baseline =
        baseline.ok_or("Legacy baseline occurrence evidence missing; manual review required")?;
    if !same_ids(&baseline.ids(), &snapshot.pre_existing_track_ids)
        || baseline
            .occurrences
            .iter()
            .filter_map(|o| o.remote_id.as_ref())
            .any(|id| owned.contains(id))
    {
        return Err(
            "Baseline attribution conflicts with added occurrences; manual review required".into(),
        );
    }
    let deletion = delete && snapshot.is_new_playlist;
    if delete && !snapshot.is_new_playlist {
        return Err("Pre-existing playlists cannot be deleted by rollback".into());
    }
    if deletion && !provider.supports_verified_deletion() {
        return Err(
            "Independent playlist-removal verification unavailable; no deletion sent".into(),
        );
    }
    if !deletion && !provider.supports_occurrence_removal() {
        return Err("Exact occurrence removal unavailable; no mutation sent".into());
    }
    let restored = |current: &PlaylistRead| {
        same_ids(&current.ids(), &baseline.ids())
            && current
                .occurrences
                .iter()
                .zip(&baseline.occurrences)
                .all(|(a, b)| b.remote_id.is_none() || a.remote_id == b.remote_id)
    };
    let mut verified = prior.is_some() && !deletion && restored(&live);
    if prior.is_some() && deletion {
        verified = provider
            .verify_playlist_removed(&snapshot.target_playlist_id)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(previous) = prior {
        if previous.sent_at.is_some()
            && !verified
            && !previous
                .error
                .as_ref()
                .is_some_and(|e| e.certainty == SendCertainty::Rejected)
        {
            return Err(
                "Rollback outcome uncertain; post-state does not prove restoration, no replay sent"
                    .into(),
            );
        }
    }
    let removal: Vec<_> = live
        .occurrences
        .iter()
        .filter(|o| o.remote_id.as_ref().is_some_and(|id| owned.contains(id)))
        .cloned()
        .collect();
    if !verified && !deletion && removal.len() != owned.len() {
        return Err("Attributed rollback occurrences changed; no removal sent".into());
    }
    let operation_id = {
        let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
        let id = if let Some(prior) = prior {
            prior.operation_id.clone()
        } else {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let id = record_operation_intent(
                &tx,
                job_id,
                usize::MAX / 2 - 1,
                &snapshot.added_track_ids,
            )?;
            tx.execute("UPDATE transfer_operations SET operation_type=?1,evidence_version=2,before_json=?2 WHERE operation_id=?3",rusqlite::params![if deletion {"rollback_delete"} else {"rollback_remove"},serde_json::to_string(&live).map_err(|e|e.to_string())?,id]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            id
        };
        #[cfg(test)]
        crash_point("rollback_intent");
        transition(&conn, job_id, "rolling_back", &emit)?;
        id
    };
    let work:Result<(),String>=async {
        if !verified {
            {
                let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;
                conn.execute("UPDATE transfer_operations SET sent_at=?1,attempt_count=attempt_count+1 WHERE operation_id=?2",rusqlite::params![chrono::Utc::now().timestamp(),operation_id]).map_err(|e|e.to_string())?;
            }
            let response=if deletion {
                provider.delete_playlist(&snapshot.target_playlist_id).await.map_err(|_|ProviderError::new(crate::providers::error::ProviderErrorKind::Unknown,SendCertainty::MayHaveApplied,"Playlist-removal response unavailable; outcome uncertain"))
            } else {provider.remove_occurrences(&snapshot.target_playlist_id,&removal,live.revision.as_deref()).await};
            #[cfg(test)]
            crash_point("rollback_response");
            if let Err(error)=response {
                let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;
                conn.execute("UPDATE transfer_operations SET status=?1,certainty=?1,provider_response=?2 WHERE operation_id=?3",rusqlite::params![if error.certainty==SendCertainty::Rejected {"failed"} else {"ambiguous"},serde_json::to_string(&error).map_err(|e|e.to_string())?,operation_id]).map_err(|e|e.to_string())?;
                return Err(error.to_string());
            }
            verified=if deletion {provider.verify_playlist_removed(&snapshot.target_playlist_id).await.map_err(|e|e.to_string())?} else {
                let after=provider.read_playlist(&snapshot.target_playlist_id).await.map_err(|e|e.to_string())?;
                after.complete && restored(&after)
            };
        }
        if !verified {return Err("Rollback post-state unverified; recovery required, no blind replay".into());}
        let conn=db.lock().map_err(|_|"Recovery storage lock unavailable")?;
        let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
        let attempts=tx.query_row("SELECT attempt_count FROM transfer_operations WHERE operation_id=?1",[&operation_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        update_operation_outcome(&tx,&operation_id,OperationOutcome::Confirmed,attempts,Some("Ordered rollback post-state verified"))?;
        tx.execute("UPDATE transfer_snapshots SET is_rolled_back=1 WHERE snapshot_id=?1",[snapshot_id]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;
        #[cfg(test)]
        crash_point("rollback_checkpoint");
        transition(&conn,job_id,"rolled_back",&emit)?;Ok(())
    }.await;
    if let Err(error) = work {
        let conn = db.lock().map_err(|_| "Recovery storage lock unavailable")?;
        transition(&conn, job_id, "recovery_required", &emit)?;
        return Err(error);
    }
    inspect_job(
        &*db.lock().map_err(|_| "Recovery storage lock unavailable")?,
        job_id,
    )
}

#[cfg(test)]
fn crash_point(point: &str) {
    if std::env::var("SOUNDSHIFT_RECOVERY_CRASH_POINT").as_deref() == Ok(point) {
        std::process::exit(73);
    }
}
