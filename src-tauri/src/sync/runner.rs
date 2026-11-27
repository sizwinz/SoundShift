use super::{model::*, store};
use crate::engine::lifecycle::{JobState, TransferControl};
use crate::providers::{
    contracts::{MutationOutcome, PlaylistRead},
    error::{is_retryable, retry_delay_ms, SendCertainty},
    traits::MusicProvider,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

pub type Providers = [Arc<dyn MusicProvider>; 2];
pub type Database = Arc<Mutex<Connection>>;

pub fn same_state(a: &PlaylistRead, b: &PlaylistRead) -> bool {
    validate_read(a).is_ok()
        && validate_read(b).is_ok()
        && a.revision == b.revision
        && a.occurrences.len() == b.occurrences.len()
        && a.occurrences
            .iter()
            .zip(&b.occurrences)
            .enumerate()
            .all(|(i, (a, b))| {
                a.index == i
                    && b.index == i
                    && a.track.id == b.track.id
                    && a.remote_id
                        .as_ref()
                        .is_none_or(|id| b.remote_id.as_ref() == Some(id))
            })
}
pub fn validate_read(read: &PlaylistRead) -> Result<(), String> {
    let mut identities = std::collections::HashSet::new();
    if !read.complete
        || read.occurrences.iter().enumerate().any(|(i, o)| {
            o.index != i
                || o.track.id.is_empty()
                || o.track.id.starts_with("unavailable_")
                || !o.track.is_playable
                || o.remote_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || !identities.insert(id))
        })
    {
        return Err(
            "Complete playable playlist occurrence evidence is required; no writes permitted"
                .into(),
        );
    }
    Ok(())
}
pub fn prepare(pair: &SyncPair, plan: SyncPlan) -> Result<SyncRunData, String> {
    if !plan.conflicts.is_empty() {
        return Err("Resolve sync conflicts before writing".into());
    }
    for side in Side::ALL {
        validate_read(plan.before(side))?;
        if plan.desired.iter().any(|o| o.track(side).is_none()) {
            return Err("Resolve every missing match before writing".into());
        }
    }
    let mut all = plan.desired.clone();
    if let Some(b) = &pair.baseline {
        for o in &b.occurrences {
            if !all.iter().any(|e| e.logical_id == o.logical_id) {
                all.push(o.clone());
            }
        }
    }
    let mut orders: [Vec<String>; 2] = [vec![], vec![]];
    for side in Side::ALL {
        let mut used = std::collections::HashSet::new();
        for occurrence in &plan.before(side).occurrences {
            let found = all
                .iter()
                .find(|o| {
                    !used.contains(&o.logical_id)
                        && o.track(side).is_some_and(|t| t.id == occurrence.track.id)
                        && o.remote(side)
                            .is_none_or(|id| Some(id) == occurrence.remote_id.as_deref())
                })
                .map(|o| o.logical_id.clone());
            let id = found.unwrap_or_else(|| {
                let id = new_id("orphan");
                let mut o = LinkedOccurrence {
                    logical_id: id.clone(),
                    spotify: None,
                    ytmusic: None,
                    spotify_remote_id: None,
                    ytmusic_remote_id: None,
                };
                o.set_track(side, occurrence.track.clone());
                o.set_remote(side, occurrence.remote_id.clone());
                all.push(o);
                id
            });
            used.insert(id.clone());
            if let Some(linked) = all.iter_mut().find(|o| o.logical_id == id) {
                linked.set_remote(side, occurrence.remote_id.clone());
            }
            orders[side.index()].push(id);
        }
    }
    Ok(SyncRunData {
        expected: [plan.spotify_before.clone(), plan.ytmusic_before.clone()],
        orders,
        all,
        counts: SyncCounts::default(),
        verified: false,
        plan,
    })
}
pub fn next_edit(data: &SyncRunData) -> Option<SyncEdit> {
    for side in Side::ALL {
        for o in &data.plan.desired {
            if !data.orders[side.index()].contains(&o.logical_id) {
                return Some(SyncEdit {
                    side,
                    kind: EditKind::Add,
                    logical_id: o.logical_id.clone(),
                    at: data.orders[side.index()].len(),
                    before: None,
                    track: o.track(side)?.clone(),
                });
            }
        }
    }
    for side in Side::ALL {
        for (at, id) in data.orders[side.index()].iter().enumerate().rev() {
            if !data.plan.desired.iter().any(|o| o.logical_id == *id) {
                return Some(SyncEdit {
                    side,
                    kind: EditKind::Remove,
                    logical_id: id.clone(),
                    at,
                    before: None,
                    track: data.expected[side.index()].occurrences[at].track.clone(),
                });
            }
        }
    }
    for side in Side::ALL {
        for (to, o) in data.plan.desired.iter().enumerate() {
            if data.orders[side.index()].get(to) != Some(&o.logical_id) {
                return Some(SyncEdit {
                    side,
                    kind: EditKind::Move,
                    logical_id: o.logical_id.clone(),
                    at: data.orders[side.index()]
                        .iter()
                        .position(|id| id == &o.logical_id)?,
                    before: Some(to),
                    track: o.track(side)?.clone(),
                });
            }
        }
    }
    None
}
pub fn preflight(data: &SyncRunData, providers: &Providers) -> Result<(), String> {
    for side in Side::ALL {
        let caps = providers[side.index()].sync_capabilities();
        let read = &data.expected[side.index()];
        for o in &data.plan.desired {
            let id = &o.track(side).ok_or("Unresolved mapping")?.id;
            if !data.orders[side.index()].contains(&o.logical_id)
                && read.occurrences.iter().any(|o| o.track.id == *id)
                && !caps.duplicate_insertion
            {
                return Err("Provider cannot preserve duplicate occurrences".into());
            }
        }
        for (index, id) in data.orders[side.index()].iter().enumerate() {
            if !data.plan.desired.iter().any(|o| o.logical_id == *id) {
                let o = &read.occurrences[index];
                if !caps.exact_removal
                    && (side != Side::Spotify
                        || read.revision.is_none()
                        || read
                            .occurrences
                            .iter()
                            .filter(|r| r.track.id == o.track.id)
                            .count()
                            != 1)
                {
                    return Err(
                        "Exact occurrence removal unsupported; manual review required".into(),
                    );
                }
            }
        }
        let desired: Vec<_> = data
            .plan
            .desired
            .iter()
            .map(|o| o.logical_id.clone())
            .collect();
        if data.orders[side.index()] != desired && !caps.occurrence_movement {
            return Err("Provider cannot safely reproduce playlist ordering".into());
        }
        if side == Side::Ytmusic && read.occurrences.iter().any(|o| o.remote_id.is_none()) {
            return Err("YouTube occurrence identifiers missing; no sync writes permitted".into());
        }
    }
    Ok(())
}
pub fn save_data(conn: &Connection, job: &str, data: &SyncRunData) -> Result<(), String> {
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    let changed = conn
        .execute(
            "UPDATE sync_runs SET payload_json=?2,payload_hash=?3 WHERE job_id=?1",
            params![job, json, store::checksum(&json)],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("Sync run checkpoint missing".into());
    }
    Ok(())
}
pub fn load_data(conn: &Connection, job: &str) -> Result<SyncRunData, String> {
    let (json, hash): (String, String) = conn
        .query_row(
            "SELECT payload_json,payload_hash FROM sync_runs WHERE job_id=?1",
            [job],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if store::checksum(&json) != hash {
        return Err("Sync run checksum failed; manual review required".into());
    }
    serde_json::from_str(&json).map_err(|_| "Sync run is malformed; manual review required".into())
}
pub fn inspect(conn: &Connection, job: &str) -> Result<SyncRunSummary, String> {
    let data = load_data(conn, job)?;
    let(state,event_seq,error):(String,u64,Option<String>)=conn.query_row("SELECT job_state,COALESCE((SELECT MAX(event_seq) FROM transfer_events WHERE job_id=?1),0),last_error FROM transfer_jobs WHERE job_id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let mut counts = SyncCounts::default();
    (counts.added,counts.removed,counts.moved,counts.failed,counts.uncertain)=conn.query_row("SELECT COALESCE(SUM(status='confirmed' AND operation_type='sync_add'),0),COALESCE(SUM(status='confirmed' AND operation_type='sync_remove'),0),COALESCE(SUM(status='confirmed' AND operation_type='sync_move'),0),COALESCE(SUM(status='rejected'),0),COALESCE(SUM(status IN ('sent','uncertain')),0) FROM transfer_operations WHERE job_id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
    let desired: Vec<_> = data
        .plan
        .desired
        .iter()
        .map(|o| o.logical_id.clone())
        .collect();
    let desired_set: std::collections::HashSet<_> = desired.iter().collect();
    let mut remaining = 0usize;
    for side in Side::ALL {
        let mut order = data.orders[side.index()].clone();
        let present: std::collections::HashSet<_> = order.iter().cloned().collect();
        let added: Vec<_> = desired
            .iter()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        remaining += added.len();
        order.extend(added);
        remaining += order.iter().filter(|id| !desired_set.contains(*id)).count();
        order.retain(|id| desired_set.contains(id));
        for (index, id) in desired.iter().enumerate() {
            if order.get(index) != Some(id) {
                let at = order
                    .iter()
                    .position(|entry| entry == id)
                    .ok_or("Recorded occurrence order is malformed")?;
                let entry = order.remove(at);
                order.insert(index, entry);
                remaining += 1;
            }
        }
    }
    counts.unsent = remaining.saturating_sub(counts.failed + counts.uncertain);
    Ok(SyncRunSummary {
        pair_id: data.plan.pair_id,
        job_id: job.into(),
        verified: data.verified && state == "completed_verified",
        state,
        event_seq,
        counts,
        error,
    })
}
pub fn start_run(
    conn: &Connection,
    pair: &SyncPair,
    data: &SyncRunData,
    job: &str,
) -> Result<(), String> {
    store::validate_generations(conn, pair)?;
    let unfinished:i64=conn.query_row("SELECT COUNT(*) FROM sync_runs r JOIN transfer_jobs j ON r.job_id=j.job_id WHERE r.pair_id=?1 AND j.job_state!='completed_verified'",[&pair.pair_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unfinished != 0 {
        return Err("Inspect and recover the unfinished sync run before starting another".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO transfer_jobs(job_id,source_service,target_service,source_playlist_name,target_playlist_id,target_playlist_name,total_tracks,matched_tracks,created_at,updated_at) VALUES(?1,'spotify','ytmusic',?2,?3,?4,?5,?5,?6,?6)",params![job,pair.spotify.title,pair.ytmusic.playlist_id,pair.ytmusic.title,data.plan.desired.len(),chrono::Utc::now().timestamp()]).map_err(|e|e.to_string())?;
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO sync_runs(job_id,pair_id,plan_id,payload_json,payload_hash) VALUES(?1,?2,?3,?4,?5)",params![job,pair.pair_id,data.plan.plan_id,json,store::checksum(&json)]).map_err(|e|e.to_string())?;
    tx.execute(
        "UPDATE sync_pairs SET state='running',last_run_id=?2,last_error=NULL WHERE pair_id=?1",
        params![pair.pair_id, job],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
fn changed_data(
    data: &SyncRunData,
    edit: &SyncEdit,
    outcome: &MutationOutcome,
) -> Result<SyncRunData, String> {
    let mut next = data.clone();
    let i = edit.side.index();
    match edit.kind {
        EditKind::Add => {
            let remote = outcome
                .occurrences
                .first()
                .and_then(|o| o.remote_id.clone());
            next.orders[i].push(edit.logical_id.clone());
            next.expected[i]
                .occurrences
                .push(crate::providers::contracts::PlaylistOccurrence {
                    index: edit.at,
                    track: edit.track.clone(),
                    remote_id: remote.clone(),
                });
            next.all
                .iter_mut()
                .find(|o| o.logical_id == edit.logical_id)
                .ok_or("Occurrence mapping lost")?
                .set_remote(edit.side, remote);
            next.counts.added += 1;
        }
        EditKind::Remove => {
            next.orders[i].remove(edit.at);
            next.expected[i].occurrences.remove(edit.at);
            next.counts.removed += 1;
        }
        EditKind::Move => {
            let to = edit.before.unwrap_or(next.orders[i].len());
            let to = if edit.at < to { to - 1 } else { to };
            let id = next.orders[i].remove(edit.at);
            next.orders[i].insert(to, id);
            let occurrence = next.expected[i].occurrences.remove(edit.at);
            next.expected[i].occurrences.insert(to, occurrence);
            next.counts.moved += 1;
        }
    }
    next.expected[i].revision = outcome.revision.clone();
    for (index, o) in next.expected[i].occurrences.iter_mut().enumerate() {
        o.index = index;
    }
    Ok(next)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncProgressPayload {
    pub pair_id: String,
    pub job_id: String,
    pub processed: usize,
    pub total: usize,
    pub side: String,
    pub track_title: String,
}

fn count_remaining_edits(data: &SyncRunData) -> usize {
    let mut temp = data.clone();
    let mut count = 0usize;
    while let Some(edit) = next_edit(&temp) {
        count += 1;
        let fake_outcome = MutationOutcome {
            occurrences: vec![crate::providers::contracts::OccurrenceOutcome {
                index: 0,
                track_id: edit.track.id.clone(),
                status: crate::providers::contracts::OccurrenceStatus::Accepted,
                remote_id: Some(format!("sim_{count}")),
            }],
            revision: None,
        };
        match changed_data(&temp, &edit, &fake_outcome) {
            Ok(next) => temp = next,
            Err(_) => break,
        }
        if count > 50_000 {
            break;
        }
    }
    count
}

pub async fn execute(
    db: Database,
    plan_id: &str,
    providers: Providers,
    control: TransferControl,
) -> Result<SyncRunSummary, String> {
    execute_with_progress(db, plan_id, providers, control, |_| {}).await
}

pub async fn execute_with_progress(
    db: Database,
    plan_id: &str,
    providers: Providers,
    control: TransferControl,
    on_progress: impl Fn(SyncProgressPayload) + Send + Sync,
) -> Result<SyncRunSummary, String> {
    let plan = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        store::load_preview(&conn, plan_id)?
    };
    let pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        store::get_pair(&conn, &plan.pair_id)?
    };
    let mut data = prepare(&pair, plan)?;
    preflight(&data, &providers)?;
    for side in Side::ALL {
        providers[side.index()]
            .check_session()
            .await
            .map_err(|e| e.to_string())?;
        providers[side.index()]
            .check_playlist_editable(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        let current = providers[side.index()]
            .read_playlist(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        if !same_state(&data.expected[side.index()], &current) {
            return Err("Sync preview is stale; playlists changed before dispatch".into());
        }
    }
    let job = control.snapshot()?.job_id;
    {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        start_run(&conn, &pair, &data, &job)?;
    }
    control.start()?;
    let result = drive(&db, &pair, &mut data, &providers, &control, &on_progress).await;
    if let Err(error) = result {
        let error = crate::diagnostics::sanitize_for_log(&error);
        let _ = control.fail(&error);
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        conn.execute("UPDATE sync_pairs SET state='recovery_required',enabled=0,last_error=?2 WHERE pair_id=?1",params![pair.pair_id,error]).map_err(|e|e.to_string())?;
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    finish_summary(&conn, &job)
}
pub async fn resume(
    db: Database,
    job: &str,
    providers: Providers,
    control: TransferControl,
) -> Result<SyncRunSummary, String> {
    resume_with_progress(db, job, providers, control, |_| {}).await
}

pub async fn resume_with_progress(
    db: Database,
    job: &str,
    providers: Providers,
    control: TransferControl,
    on_progress: impl Fn(SyncProgressPayload) + Send + Sync,
) -> Result<SyncRunSummary, String> {
    if control.snapshot()?.job_id != job {
        return Err("Sync control identity differs from recorded run".into());
    }
    let (mut data, pair) = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        let summary = inspect(&conn, job)?;
        if summary.verified {
            return Ok(summary);
        }
        let data = load_data(&conn, job)?;
        let pair = store::get_pair(&conn, &data.plan.pair_id)?;
        store::validate_generations(&conn, &pair)?;
        validate_operations(&conn, job)?;
        (data, pair)
    };
    preflight(&data, &providers)?;
    for side in Side::ALL {
        let provider = &providers[side.index()];
        provider.check_session().await.map_err(|e| e.to_string())?;
        provider
            .check_playlist_editable(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        let current = provider
            .read_playlist(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        if !same_state(&data.expected[side.index()], &current) {
            return Err(
                "Recorded sync state differs from remote state; no replay permitted".into(),
            );
        }
    }
    control.start()?;
    let result = drive(&db, &pair, &mut data, &providers, &control, &on_progress).await;
    if let Err(error) = result {
        let _ = control.fail(&error);
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        conn.execute("UPDATE sync_pairs SET enabled=0,state='recovery_required',last_error=?2 WHERE pair_id=?1",params![pair.pair_id,crate::diagnostics::sanitize_for_log(&error)]).map_err(|e|e.to_string())?;
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    finish_summary(&conn, job)
}
fn finish_summary(conn: &Connection, job: &str) -> Result<SyncRunSummary, String> {
    let summary = inspect(conn, job)?;
    if summary.state == "cancelled" {
        conn.execute("UPDATE sync_pairs SET state='cancelled',enabled=0,next_due_at=NULL,last_error='Sync cancelled; inspect this run before continuing' WHERE pair_id=?1",[&summary.pair_id]).map_err(|e|e.to_string())?;
    }
    Ok(summary)
}
pub fn validate_operations(conn: &Connection, job: &str) -> Result<(), String> {
    crate::storage::recovery::ensure_job_replay_allowed(conn, job)?;
    let mut statement=conn.prepare("SELECT payload_json,payload_hash,status,sent_at,attempt_count,evidence_version,outcome_json,updated_at,retry_not_before_ms FROM transfer_operations WHERE job_id=?1 ORDER BY operation_index").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map([job], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut retries = 0;
    for row in rows {
        let (json, hash, status, sent, attempts, version, outcome, updated, retry_deadline) =
            row.map_err(|e| e.to_string())?;
        if version != 3 || store::checksum(&json) != hash {
            return Err("Recorded sync operation checksum or version is invalid".into());
        }
        let edit: SyncEdit =
            serde_json::from_str(&json).map_err(|_| "Recorded sync edit malformed")?;
        retries += attempts.saturating_sub(1);
        match status.as_str() {
            "confirmed" => {
                let receipt: Result<MutationOutcome, crate::providers::error::ProviderError> =
                    serde_json::from_str(outcome.as_deref().ok_or("Confirmed receipt missing")?)
                        .map_err(|_| "Confirmed receipt malformed")?;
                let receipt = receipt.map_err(|_| "Confirmed operation contains an error")?;
                if !receipt.all_accepted()
                    || !receipt.matches_payload(std::slice::from_ref(&edit.track.id))
                {
                    return Err("Confirmed receipt differs from recorded edit".into());
                }
            }
            "pending" if sent.is_none() && attempts == 0 => {}
            "rejected" => {
                let receipt: Result<MutationOutcome, crate::providers::error::ProviderError> =
                    serde_json::from_str(outcome.as_deref().ok_or("Rejection evidence missing")?)
                        .map_err(|_| "Rejection evidence malformed")?;
                let error = receipt
                    .err()
                    .ok_or("Rejection evidence does not prove rejection")?;
                if !matches!(
                    error.certainty,
                    SendCertainty::NotSent | SendCertainty::Rejected
                ) {
                    return Err("Uncertain sync operations cannot be replayed".into());
                }
                if attempts >= 6 {
                    return Err("Sync operation retry limit reached; manual review required".into());
                }
                let deadline = retry_deadline.or_else(|| {
                    error.retry_after_ms.map(|delay| {
                        (updated + 1)
                            .saturating_mul(1000)
                            .saturating_add(delay.min(i64::MAX as u64) as i64)
                    })
                });
                if deadline.is_some_and(|deadline| chrono::Utc::now().timestamp_millis() < deadline)
                {
                    return Err("Provider retry deadline has not elapsed; sync deferred".into());
                }
            }
            _ => {
                return Err(
                    "Uncertain sync operations cannot be replayed; manual review required".into(),
                )
            }
        }
    }
    if retries >= 20 {
        return Err("Per-job retry budget exhausted; manual review required".into());
    }
    Ok(())
}
async fn drive(
    db: &Database,
    pair: &SyncPair,
    data: &mut SyncRunData,
    providers: &Providers,
    control: &TransferControl,
    on_progress: &(impl Fn(SyncProgressPayload) + Send + Sync),
) -> Result<(), String> {
    let job = control.snapshot()?.job_id;
    let mut verified_preflight = [false, false];
    let mut verified_cache: [Option<PlaylistRead>; 2] = [None, None];
    let mut processed = 0usize;

    while let Some(edit) = next_edit(data) {
        if !control.wait_until_ready().await? {
            control.finish().await?;
            return Ok(());
        }
        let i = edit.side.index();
        let provider = &providers[i];
        let playlist = &pair.endpoint(edit.side).playlist_id;
        if !verified_preflight[i] {
            provider.check_session().await.map_err(|e| e.to_string())?;
            provider
                .check_playlist_editable(playlist)
                .await
                .map_err(|e| e.to_string())?;
            verified_preflight[i] = true;
        }
        let current = match verified_cache[i].take() {
            Some(cached) if cached.revision.is_some() => {
                let live_rev = provider
                    .playlist_revision(playlist)
                    .await
                    .map_err(|e| e.to_string())?;
                if live_rev == cached.revision {
                    cached
                } else {
                    provider
                        .read_playlist(playlist)
                        .await
                        .map_err(|e| e.to_string())?
                }
            }
            _ => provider
                .read_playlist(playlist)
                .await
                .map_err(|e| e.to_string())?,
        };
        if !same_state(&data.expected[i], &current) {
            return Err("Playlist drift detected before mutation; review required".into());
        }
        let (operation_id, payload) = {
            let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
            store::validate_generations(&conn, pair)?;
            validate_operations(&conn, &job)?;
            let payload = serde_json::to_string(&edit).map_err(|e| e.to_string())?;
            let existing:Option<(String,String)>=conn.query_row("SELECT operation_id,payload_json FROM transfer_operations WHERE job_id=?1 AND status!='confirmed' ORDER BY operation_index LIMIT 1",[&job],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
            if let Some((id, recorded)) = existing {
                if recorded != payload {
                    return Err("Recorded unsent edit differs from the next planned operation; no replay permitted".into());
                }
                (id, payload)
            } else {
                let index: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM transfer_operations WHERE job_id=?1",
                        [&job],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let id = format!("sync_op_{job}_{index}");
                conn.execute("INSERT INTO transfer_operations(operation_id,job_id,operation_type,operation_index,payload_hash,certainty,status,attempt_count,created_at,updated_at,payload_json,evidence_version,before_json) VALUES(?1,?2,?3,?4,?5,'unsent','pending',0,?6,?6,?7,3,?8)",params![id,job,edit.kind.operation_type(),index,store::checksum(&payload),chrono::Utc::now().timestamp(),payload,serde_json::to_string(&current).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
                (id, payload)
            }
        };
        crash_point("intent");
        let Some(permit) = control.reserve_mutation().await? else {
            control.finish().await?;
            return Ok(());
        };
        {
            let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
            conn.execute("UPDATE transfer_operations SET sent_at=?2,status='sent',certainty='ambiguous',attempt_count=attempt_count+1 WHERE operation_id=?1 AND payload_hash=?3",params![operation_id,chrono::Utc::now().timestamp(),store::checksum(&payload)]).map_err(|e|e.to_string())?;
        }
        crash_point("send");
        let result = match edit.kind {
            EditKind::Add => {
                provider
                    .add_occurrences(playlist, std::slice::from_ref(&edit.track.id))
                    .await
            }
            EditKind::Remove => {
                provider
                    .sync_remove_occurrence(playlist, &current.occurrences[edit.at], &current)
                    .await
            }
            EditKind::Move => {
                provider
                    .move_occurrence(
                        playlist,
                        &current.occurrences[edit.at],
                        edit.before.map(|index| &current.occurrences[index]),
                        &current,
                    )
                    .await
            }
        };
        crash_point("response");
        let verification = match &result {
            Ok(outcome)
                if outcome.matches_payload(std::slice::from_ref(&edit.track.id))
                    && outcome.all_accepted() =>
            {
                let proposed = changed_data(data, &edit, outcome)?;
                match provider.read_playlist(playlist).await {
                    Ok(after) if same_state(&proposed.expected[i], &after) => Ok((proposed, after)),
                    _ => Err(
                        "Accepted mutation has an unverified post-state; recovery required"
                            .to_string(),
                    ),
                }
            }
            Ok(_) => Err("Provider mutation outcome uncertain; no replay permitted".into()),
            Err(error) => Err(error.to_string()),
        };
        let attempts = {
            let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
            conn.query_row(
                "SELECT attempt_count FROM transfer_operations WHERE operation_id=?1",
                [&operation_id],
                |r| r.get::<_, u32>(0),
            )
            .map_err(|e| e.to_string())?
        };
        let retry_delay = result
            .as_ref()
            .err()
            .filter(|error| {
                matches!(
                    error.certainty,
                    SendCertainty::NotSent | SendCertainty::Rejected
                ) && is_retryable(error.kind)
            })
            .map(|error| retry_delay_ms(attempts.saturating_sub(1), error.retry_after_ms));
        let retry_deadline = retry_delay.map(|delay| {
            chrono::Utc::now()
                .timestamp_millis()
                .saturating_add(delay.min(i64::MAX as u64) as i64)
        });
        {
            let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let status = if verification.is_ok() {
                "confirmed"
            } else if result.as_ref().err().is_some_and(|e| {
                matches!(
                    e.certainty,
                    SendCertainty::NotSent | SendCertainty::Rejected
                )
            }) {
                "rejected"
            } else {
                "uncertain"
            };
            tx.execute("UPDATE transfer_operations SET status=?2,certainty=?2,outcome_json=?3,provider_response=?4,updated_at=?5 WHERE operation_id=?1",params![operation_id,status,serde_json::to_string(&result).map_err(|e|e.to_string())?,verification.as_ref().err().map(|s|crate::diagnostics::sanitize_for_log(s)),chrono::Utc::now().timestamp()]).map_err(|e|e.to_string())?;
            tx.execute(
                "UPDATE transfer_operations SET retry_not_before_ms=?2 WHERE operation_id=?1",
                params![operation_id, retry_deadline],
            )
            .map_err(|e| e.to_string())?;
            if let Ok((proposed, _)) = &verification {
                save_data(&tx, &job, proposed)?;
            } else {
                let mut failed = data.clone();
                if status == "rejected" {
                    failed.counts.failed += 1;
                } else {
                    failed.counts.uncertain += 1;
                }
                save_data(&tx, &job, &failed)?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        crash_point("checkpoint");
        permit.checkpointed()?;
        match verification {
            Ok((proposed, after)) => {
                verified_cache[i] = Some(after);
                *data = proposed;
                processed += 1;
                let total_remaining = count_remaining_edits(data);
                on_progress(SyncProgressPayload {
                    pair_id: pair.pair_id.clone(),
                    job_id: job.clone(),
                    processed,
                    total: (processed + total_remaining).max(processed),
                    side: edit.side.service().to_string(),
                    track_title: edit.track.title.clone(),
                });
            }
            Err(error) => {
                verified_preflight[i] = false;
                verified_cache[i] = None;
                if let Some(delay) = retry_delay.filter(|delay| *delay <= 30_000 && attempts < 6) {
                    let _ = delay;
                    let deadline = retry_deadline.ok_or("Retry deadline missing")?;
                    while chrono::Utc::now().timestamp_millis() < deadline {
                        if !control.wait_until_ready().await? {
                            control.finish().await?;
                            return Ok(());
                        }
                        let remaining =
                            (deadline - chrono::Utc::now().timestamp_millis()).max(0) as u64;
                        control
                            .interruptible_delay(std::time::Duration::from_millis(
                                remaining.min(30_000),
                            ))
                            .await;
                    }
                    continue;
                }
                return Err(error);
            }
        }
    }
    for side in Side::ALL {
        let current = providers[side.index()]
            .read_playlist(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        if !same_state(&data.expected[side.index()], &current) {
            return Err("Final ordered state changed; shared baseline withheld".into());
        }
    }
    crash_point("baseline");
    let update = control
        .finish_with_checkpoint(|conn| {
            store::validate_generations(conn, pair)?;
            let mut completed = data.clone();
            completed.verified = true;
            save_data(conn, &job, &completed)?;
            let mut pair = store::get_pair(conn, &pair.pair_id)?;
            if let Some(previous) = &pair.baseline {
                for entry in &previous.occurrences {
                    if !data
                        .plan
                        .desired
                        .iter()
                        .any(|o| o.logical_id == entry.logical_id)
                        && !pair
                            .tombstones
                            .iter()
                            .any(|o| o.logical_id == entry.logical_id)
                    {
                        pair.tombstones.push(entry.clone());
                    }
                }
            }
            for entry in data
                .all
                .iter()
                .filter(|o| o.spotify.is_some() && o.ytmusic.is_some())
            {
                if !pair.approved_mappings.iter().any(|old| {
                    old.spotify.as_ref().map(|t| &t.id) == entry.spotify.as_ref().map(|t| &t.id)
                        && old.ytmusic.as_ref().map(|t| &t.id)
                            == entry.ytmusic.as_ref().map(|t| &t.id)
                }) {
                    pair.approved_mappings.push(entry.clone());
                }
            }
            pair.baseline = Some(SyncBaseline {
                version: data.plan.baseline_version + 1,
                occurrences: data
                    .plan
                    .desired
                    .iter()
                    .map(|o| {
                        data.all
                            .iter()
                            .find(|all| all.logical_id == o.logical_id)
                            .cloned()
                            .unwrap_or_else(|| o.clone())
                    })
                    .collect(),
                spotify: data.expected[0].clone(),
                ytmusic: data.expected[1].clone(),
            });
            pair.state = "synced".into();
            pair.last_error = None;
            pair.next_due_at = pair.enabled.then(|| chrono::Utc::now().timestamp() + 3600);
            store::save_pair(conn, &pair)
        })
        .await?;
    if update.state == JobState::CompletedVerified {
        crash_point("baseline_committed");
    }
    if update.state == JobState::Cancelled {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        conn.execute("UPDATE sync_pairs SET enabled=0,state='cancelled',last_error='Sync cancelled; inspect this run before continuing' WHERE pair_id=?1",[&pair.pair_id]).map_err(|e|e.to_string())?;
    }
    Ok(())
}
fn crash_point(_boundary: &str) {
    #[cfg(test)]
    if std::env::var("SOUNDSHIFT_SYNC_CRASH").as_deref() == Ok(_boundary) {
        std::process::exit(91);
    }
}
