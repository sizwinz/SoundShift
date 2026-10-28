use super::{model::*, store};
use crate::engine::lifecycle::{JobState, TransferControl};
use crate::providers::{
    contracts::{MutationOutcome, PlaylistRead},
    error::SendCertainty,
    traits::MusicProvider,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

pub type Providers = [Arc<dyn MusicProvider>; 2];
pub type Database = Arc<Mutex<Connection>>;

pub fn same_state(a: &PlaylistRead, b: &PlaylistRead) -> bool {
    a.complete
        && b.complete
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
    let mut counts = data.counts.clone();
    counts.unsent = usize::from(next_edit(&data).is_some());
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

pub async fn execute(
    db: Database,
    plan_id: &str,
    providers: Providers,
    control: TransferControl,
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
    let result = drive(&db, &pair, &mut data, &providers, &control).await;
    if let Err(error) = result {
        let error = crate::diagnostics::sanitize_for_log(&error);
        let _ = control.fail(&error);
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        conn.execute("UPDATE sync_pairs SET state='recovery_required',enabled=0,last_error=?2 WHERE pair_id=?1",params![pair.pair_id,error]).map_err(|e|e.to_string())?;
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    inspect(&conn, &job)
}
pub async fn resume(
    db: Database,
    job: &str,
    providers: Providers,
    control: TransferControl,
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
    let result = drive(&db, &pair, &mut data, &providers, &control).await;
    if let Err(error) = result {
        let _ = control.fail(&error);
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        conn.execute("UPDATE sync_pairs SET enabled=0,state='recovery_required',last_error=?2 WHERE pair_id=?1",params![pair.pair_id,crate::diagnostics::sanitize_for_log(&error)]).map_err(|e|e.to_string())?;
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    inspect(&conn, job)
}
pub fn validate_operations(conn: &Connection, job: &str) -> Result<(), String> {
    let mut statement=conn.prepare("SELECT payload_json,payload_hash,status,sent_at,attempt_count,evidence_version,outcome_json,updated_at FROM transfer_operations WHERE job_id=?1 ORDER BY operation_index").map_err(|e|e.to_string())?;
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
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut retries = 0;
    for row in rows {
        let (json, hash, status, sent, attempts, version, outcome, updated) =
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
                if attempts >= 3 {
                    return Err("Sync operation retry limit reached; manual review required".into());
                }
                if error.retry_after_ms.is_some_and(|delay| {
                    chrono::Utc::now().timestamp_millis()
                        < (updated + 1)
                            .saturating_mul(1000)
                            .saturating_add(delay.min(i64::MAX as u64) as i64)
                }) {
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
) -> Result<(), String> {
    let job = control.snapshot()?.job_id;
    while let Some(edit) = next_edit(data) {
        if !control.wait_until_ready().await? {
            control.finish().await?;
            return Ok(());
        }
        let i = edit.side.index();
        let provider = &providers[i];
        let playlist = &pair.endpoint(edit.side).playlist_id;
        provider.check_session().await.map_err(|e| e.to_string())?;
        provider
            .check_playlist_editable(playlist)
            .await
            .map_err(|e| e.to_string())?;
        let current = provider
            .read_playlist(playlist)
            .await
            .map_err(|e| e.to_string())?;
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
            Ok((proposed, _)) => *data = proposed,
            Err(error) => return Err(error),
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
