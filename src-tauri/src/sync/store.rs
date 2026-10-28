//! Synchronous SQLite boundaries. No caller may retain a connection guard over await.
use super::model::*;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

pub fn checksum(json: &str) -> String {
    format!("{:x}", Sha256::digest(json.as_bytes()))
}
pub fn pair_stamp(pair: &SyncPair) -> Result<String, String> {
    serde_json::to_string(&(
        &pair.spotify,
        &pair.ytmusic,
        pair.baseline.as_ref().map_or(0, |b| b.version),
    ))
    .map(|s| checksum(&s))
    .map_err(|e| e.to_string())
}

pub fn generation(conn: &Connection, service: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT generation FROM credential_generations WHERE service=?1",
        [service],
        |r| r.get(0),
    )
    .map_err(|_| format!("Reconnect {service} before linking or revalidating playlists"))
}
pub fn create_pair(
    conn: &Connection,
    spotify_id: &str,
    spotify_title: &str,
    ytmusic_id: &str,
    ytmusic_title: &str,
) -> Result<SyncPair, String> {
    if spotify_id.is_empty() || ytmusic_id.is_empty() {
        return Err("Choose two existing playlists".into());
    }
    let pair = SyncPair {
        pair_id: new_id("pair"),
        spotify: SyncEndpoint {
            playlist_id: spotify_id.into(),
            title: spotify_title.into(),
            generation: generation(conn, "spotify")?,
        },
        ytmusic: SyncEndpoint {
            playlist_id: ytmusic_id.into(),
            title: ytmusic_title.into(),
            generation: generation(conn, "ytmusic")?,
        },
        baseline: None,
        approved_mappings: vec![],
        tombstones: vec![],
        enabled: false,
        state: "draft".into(),
        next_due_at: None,
        last_run_id: None,
        last_error: None,
    };
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO sync_pairs(pair_id,payload_json,updated_at) VALUES(?1,?2,?3)",
        params![
            pair.pair_id,
            serde_json::to_string(&pair).map_err(|e| e.to_string())?,
            chrono::Utc::now().timestamp()
        ],
    )
    .map_err(|e| e.to_string())?;
    for side in Side::ALL {
        let endpoint = pair.endpoint(side);
        tx.execute("INSERT INTO sync_endpoints(service,playlist_id,pair_id,generation) VALUES(?1,?2,?3,?4)",params![side.service(),endpoint.playlist_id,pair.pair_id,endpoint.generation]).map_err(|_|"This playlist is already linked; unlink its existing pair first".to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(pair)
}
pub fn get_pair(conn: &Connection, id: &str) -> Result<SyncPair, String> {
    conn.query_row("SELECT payload_json,state,enabled,next_due_at,last_run_id,last_error FROM sync_pairs WHERE pair_id=?1",[id],|r|{
        let json:String=r.get(0)?;
        let mut pair:SyncPair=serde_json::from_str(&json).map_err(|e|rusqlite::Error::FromSqlConversionFailure(0,rusqlite::types::Type::Text,Box::new(e)))?;
        pair.state=r.get(1)?;pair.enabled=r.get(2)?;pair.next_due_at=r.get(3)?;pair.last_run_id=r.get(4)?;pair.last_error=r.get(5)?;Ok(pair)
    }).map_err(|e|e.to_string())
}
pub fn list_pairs(conn: &Connection) -> Result<Vec<SyncPair>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT pair_id FROM sync_pairs WHERE state!='unlinked' ORDER BY updated_at,pair_id",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    ids.iter().map(|id| get_pair(conn, id)).collect()
}
pub fn save_pair(conn: &Connection, pair: &SyncPair) -> Result<(), String> {
    let n=conn.execute("UPDATE sync_pairs SET payload_json=?2,state=?3,enabled=?4,next_due_at=?5,last_run_id=?6,last_error=?7,baseline_version=?8,updated_at=?9 WHERE pair_id=?1",params![pair.pair_id,serde_json::to_string(pair).map_err(|e|e.to_string())?,pair.state,pair.enabled,pair.next_due_at,pair.last_run_id,pair.last_error,pair.baseline.as_ref().map_or(0,|b|b.version),chrono::Utc::now().timestamp()]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("Sync pair no longer exists".into());
    }
    Ok(())
}
pub fn validate_generations(conn: &Connection, pair: &SyncPair) -> Result<(), String> {
    if pair.state == "unlinked" {
        return Err("This playlist pair has been unlinked; no sync writes permitted".into());
    }
    for side in Side::ALL {
        if generation(conn, side.service())? != pair.endpoint(side).generation {
            return Err("Account changed; revalidate this pair before syncing".into());
        }
    }
    Ok(())
}
pub fn save_preview(conn: &Connection, plan: &SyncPlan) -> Result<SyncPlan, String> {
    let pair = get_pair(conn, &plan.pair_id)?;
    validate_generations(conn, &pair)?;
    let mut plan = plan.clone();
    plan.pair_stamp = pair_stamp(&pair)?;
    let json = serde_json::to_string(&plan).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO sync_previews(plan_id,pair_id,payload_json,payload_hash,created_at) VALUES(?1,?2,?3,?4,?5)",params![plan.plan_id,plan.pair_id,json,checksum(&json),chrono::Utc::now().timestamp()]).map_err(|e|e.to_string())?;
    Ok(plan)
}
pub fn load_preview(conn: &Connection, id: &str) -> Result<SyncPlan, String> {
    let (json, hash): (String, String) = conn
        .query_row(
            "SELECT payload_json,payload_hash FROM sync_previews WHERE plan_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if checksum(&json) != hash {
        return Err("Sync preview checksum failed; no writes permitted".into());
    }
    let plan: SyncPlan = serde_json::from_str(&json).map_err(|_| "Sync preview is malformed")?;
    let pair = get_pair(conn, &plan.pair_id)?;
    validate_generations(conn, &pair)?;
    if plan.pair_stamp != pair_stamp(&pair)? {
        return Err("Sync preview is stale; review a fresh preview".into());
    }
    Ok(plan)
}

pub fn unlink(conn: &Connection, id: &str) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let active:i64=tx.query_row("SELECT COUNT(*) FROM sync_runs r JOIN transfer_jobs j ON j.job_id=r.job_id WHERE r.pair_id=?1 AND j.job_state IN ('planned','starting','running','pausing','paused','cancelling','reconciling')",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if active != 0 {
        return Err(
            "Cancel the active sync and wait for its requests to drain before unlinking".into(),
        );
    }
    let mut pair = get_pair(&tx, id)?;
    pair.state = "unlinked".into();
    pair.enabled = false;
    pair.next_due_at = None;
    save_pair(&tx, &pair)?;
    tx.execute("DELETE FROM sync_endpoints WHERE pair_id=?1", [id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
