use super::{model::SyncPair, store};
use rusqlite::{params, Connection, OptionalExtension};

/// The caller holds the application mutation gate before claiming a due pair.
/// Persist one next deadline, never a queue of elapsed hourly ticks.
pub fn claim_due(conn: &Connection, now: i64) -> Result<Option<SyncPair>, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let id:Option<String>=tx.query_row("SELECT pair_id FROM sync_pairs p WHERE enabled=1 AND state='synced' AND next_due_at<=?1 AND NOT EXISTS(SELECT 1 FROM sync_runs r JOIN transfer_jobs j ON j.job_id=r.job_id WHERE r.pair_id=p.pair_id AND j.job_state!='completed_verified') ORDER BY next_due_at,pair_id LIMIT 1",[now],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let Some(id) = id else {
        return Ok(None);
    };
    let mut pair = store::get_pair(&tx, &id)?;
    store::validate_generations(&tx, &pair)?;
    if pair.baseline.is_none() {
        return Err("Initialize the pair before enabling scheduling".into());
    }
    pair.next_due_at = Some(now + 3600);
    pair.state = "checking".into();
    store::save_pair(&tx, &pair)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(pair))
}
pub fn set_enabled(
    conn: &Connection,
    id: &str,
    enabled: bool,
    now: i64,
) -> Result<SyncPair, String> {
    let mut pair = store::get_pair(conn, id)?;
    if enabled {
        store::validate_generations(conn, &pair)?;
        if pair.baseline.is_none() || pair.state != "synced" {
            return Err("Resolve this pair and complete a verified sync before enabling its hourly schedule".into());
        }
    }
    pair.enabled = enabled;
    pair.next_due_at = enabled.then_some(now + 3600);
    store::save_pair(conn, &pair)?;
    Ok(pair)
}
pub fn block(conn: &Connection, id: &str, state: &str, error: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sync_pairs SET enabled=0,state=?2,last_error=?3 WHERE pair_id=?1",
        params![id, state, crate::diagnostics::sanitize_for_log(error)],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn recover_startup(conn: &Connection, now: i64) -> Result<(), String> {
    conn.execute("UPDATE sync_pairs SET state='recovery_required',enabled=0,last_error='Interrupted sync requires inspection; no mutations replayed' WHERE EXISTS(SELECT 1 FROM sync_runs r JOIN transfer_jobs j ON j.job_id=r.job_id WHERE r.pair_id=sync_pairs.pair_id AND j.job_state!='completed_verified') AND state!='unlinked'",[]).map_err(|e|e.to_string())?;
    conn.execute("UPDATE sync_pairs SET state='synced',next_due_at=?1 WHERE state='checking' AND baseline_version>0",[now]).map_err(|e|e.to_string())?;
    Ok(())
}
