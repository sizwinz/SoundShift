use super::{
    model::*,
    runner::{self, Providers},
    scheduler, service, store,
};
use crate::{
    engine::lifecycle::{LifecycleUpdate, TransferControl},
    providers::{SpotifyProvider, YouTubeMusicProvider},
    AppState,
};
use rusqlite::{params, OptionalExtension};
use std::sync::{atomic::Ordering, Arc};
use tauri::{AppHandle, Emitter, Manager};

fn gate(app: &AppHandle) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let state = app.state::<AppState>();
    super::runtime::acquire_gate(state.mutation_gate.clone(), &state.shutting_down)
}
fn providers(app: &AppHandle) -> Result<Providers, String> {
    let state = app.state::<AppState>();
    let tokens = {
        let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
        let mut tokens = vec![];
        for side in Side::ALL {
            tokens.push(
                crate::auth::keyring_store::retrieve_credential(side.service(), &conn)?
                    .ok_or_else(|| format!("Reconnect {} before syncing", side.service()))?,
            );
        }
        tokens
    };
    Ok([
        Arc::new(SpotifyProvider::new(tokens[0].clone())),
        Arc::new(YouTubeMusicProvider::new(tokens[1].clone())),
    ])
}
fn publish(app: &AppHandle, pair: &str, update: LifecycleUpdate) {
    let payload = SyncUpdate {
        pair_id: pair.into(),
        job_id: update.job_id.clone(),
        state: update.state.as_str().into(),
        event_seq: update.event_seq,
    };
    let _ = app.emit("sync:state_changed", payload);
    let _ = app.emit("job:state_changed", update);
    let _ = app.emit("sync:pairs_changed", ());
}
fn control(app: &AppHandle, pair: &str, job: &str) -> Result<TransferControl, String> {
    let state = app.state::<AppState>();
    let handle = app.clone();
    let pair = pair.to_string();
    let control = TransferControl::new(job.into(), state.db.clone(), move |update| {
        publish(&handle, &pair, update)
    });
    let mut registry = state
        .active_transfers
        .lock()
        .map_err(|_| "Active job registry unavailable")?;
    if registry.contains_key(job) {
        return Err("This sync is already active".into());
    }
    registry.insert(job.into(), control.clone());
    Ok(control)
}
fn cleanup(app: &AppHandle, job: &str) {
    if let Ok(mut registry) = app.state::<AppState>().active_transfers.lock() {
        registry.remove(job);
    }
    let _ = app.emit("sync:pairs_changed", ());
}

#[tauri::command]
pub fn list_sync_pairs(app: AppHandle) -> Result<Vec<SyncPair>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    store::list_pairs(&conn)
}

#[tauri::command]
pub async fn create_sync_pair(
    app: AppHandle,
    spotify_id: String,
    spotify_title: String,
    ytmusic_id: String,
    ytmusic_title: String,
) -> Result<SyncPair, String> {
    let _gate = gate(&app)?;
    let providers = providers(&app)?;
    for side in Side::ALL {
        providers[side.index()]
            .check_session()
            .await
            .map_err(|e| e.to_string())?;
        providers[side.index()]
            .check_playlist_editable(if side == Side::Spotify {
                &spotify_id
            } else {
                &ytmusic_id
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    crate::storage::recovery::ensure_mutations_allowed(&conn)?;
    let pair = store::create_pair(
        &conn,
        &spotify_id,
        &spotify_title,
        &ytmusic_id,
        &ytmusic_title,
    )?;
    let _ = app.emit("sync:pairs_changed", ());
    Ok(pair)
}
#[tauri::command]
pub async fn preview_sync_pair(
    app: AppHandle,
    pair_id: String,
    resolution: Option<SyncResolution>,
) -> Result<SyncPlan, String> {
    let _gate = gate(&app)?;
    let providers = providers(&app)?;
    let db = app.state::<AppState>().db.clone();
    service::preview(&db, &pair_id, &providers, resolution).await
}
#[tauri::command]
pub async fn execute_sync_plan(
    app: AppHandle,
    plan_id: String,
    job_id: String,
) -> Result<SyncRunSummary, String> {
    let _gate = gate(&app)?;
    if !job_id.starts_with("sync_")
        || job_id.len() > 96
        || !job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Invalid sync run identity".into());
    }
    let db = app.state::<AppState>().db.clone();
    let pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT plan_id FROM sync_runs WHERE job_id=?1",
                [&job_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(existing) = existing {
            if existing != plan_id {
                return Err("Run identity belongs to a different preview".into());
            }
            return runner::inspect(&conn, &job_id);
        }
        store::load_preview(&conn, &plan_id)?.pair_id
    };
    let providers = providers(&app)?;
    let control = control(&app, &pair, &job_id)?;
    let result = runner::execute(db, &plan_id, providers, control).await;
    cleanup(&app, &job_id);
    result
}
#[tauri::command]
pub fn set_sync_schedule(
    app: AppHandle,
    pair_id: String,
    enabled: bool,
) -> Result<SyncPair, String> {
    if app.state::<AppState>().shutting_down.load(Ordering::SeqCst) {
        return Err("SoundShift is closing".into());
    }
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    let pair = scheduler::set_enabled(&conn, &pair_id, enabled, chrono::Utc::now().timestamp())?;
    let _ = app.emit("sync:pairs_changed", ());
    Ok(pair)
}
#[tauri::command]
pub fn unlink_sync_pair(app: AppHandle, pair_id: String) -> Result<(), String> {
    let _gate = gate(&app)?;
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    store::unlink(&conn, &pair_id)?;
    let _ = app.emit("sync:pairs_changed", ());
    Ok(())
}

#[tauri::command]
pub async fn revalidate_sync_pair(app: AppHandle, pair_id: String) -> Result<SyncPair, String> {
    let _gate = gate(&app)?;
    let providers = providers(&app)?;
    let db = app.state::<AppState>().db.clone();
    let mut pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        store::get_pair(&conn, &pair_id)?
    };
    for side in Side::ALL {
        providers[side.index()]
            .check_session()
            .await
            .map_err(|e| e.to_string())?;
        providers[side.index()]
            .check_playlist_editable(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    let unfinished:i64=conn.query_row("SELECT COUNT(*) FROM sync_runs r JOIN transfer_jobs j ON j.job_id=r.job_id WHERE r.pair_id=?1 AND j.job_state!='completed_verified'",[&pair_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unfinished != 0 || pair.state == "unlinked" {
        return Err("Inspect the unfinished run before changing its account binding; uncertain work remains blocked".into());
    }
    pair.spotify.generation = store::generation(&conn, "spotify")?;
    pair.ytmusic.generation = store::generation(&conn, "ytmusic")?;
    pair.baseline = None;
    pair.approved_mappings.clear();
    pair.tombstones.clear();
    pair.enabled = false;
    pair.next_due_at = None;
    pair.state = "draft".into();
    pair.last_error = None;
    store::save_pair(&conn, &pair)?;
    conn.execute("DELETE FROM sync_previews WHERE pair_id=?1", [&pair_id])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE sync_endpoints SET generation=CASE service WHEN 'spotify' THEN ?2 ELSE ?3 END WHERE pair_id=?1",params![pair_id,pair.spotify.generation,pair.ytmusic.generation]).map_err(|e|e.to_string())?;
    let _ = app.emit("sync:pairs_changed", ());
    Ok(pair)
}
pub fn inspection(conn: &rusqlite::Connection, job: &str) -> Result<SyncInspection, String> {
    let summary = runner::inspect(conn, job)?;
    let validation = runner::validate_operations(conn, job);
    let pair = store::get_pair(conn, &summary.pair_id)?;
    let can_resume = !summary.verified
        && pair.state != "unlinked"
        && validation.is_ok()
        && store::validate_generations(conn, &pair).is_ok();
    let mut statement=conn.prepare("SELECT operation_id,payload_json,status,attempt_count,provider_response FROM transfer_operations WHERE job_id=?1 ORDER BY operation_index").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map([job], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut operations = vec![];
    for row in rows {
        let (id, json, status, attempt_count, detail) = row.map_err(|e| e.to_string())?;
        let edit: SyncEdit =
            serde_json::from_str(&json).map_err(|_| "Recorded sync operation is malformed")?;
        operations.push(SyncOperationRecord {
            operation_id: id,
            side: edit.side,
            kind: edit.kind,
            status,
            attempt_count,
            detail,
        });
    }
    let next_action = if summary.verified {
        "Both ordered playlists were verified".into()
    } else if let Err(error) = validation {
        error
    } else {
        "Resume revalidates both playlists and sends only proven unsent or rejected operations"
            .into()
    };
    Ok(SyncInspection {
        summary,
        operations,
        can_resume,
        next_action,
    })
}
#[tauri::command]
pub fn inspect_sync_run(app: AppHandle, job_id: String) -> Result<SyncInspection, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    inspection(&conn, &job_id)
}

#[tauri::command]
pub async fn resume_sync_run(app: AppHandle, job_id: String) -> Result<SyncRunSummary, String> {
    let _gate = gate(&app)?;
    let db = app.state::<AppState>().db.clone();
    let pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
        let inspection = inspection(&conn, &job_id)?;
        if inspection.summary.verified {
            return Ok(inspection.summary);
        }
        if !inspection.can_resume {
            return Err(inspection.next_action);
        }
        inspection.summary.pair_id
    };
    let providers = providers(&app)?;
    let control = control(&app, &pair, &job_id)?;
    let result = runner::resume(db, &job_id, providers, control).await;
    cleanup(&app, &job_id);
    result
}
#[tauri::command]
pub async fn reconcile_sync_run(app: AppHandle, job_id: String) -> Result<SyncInspection, String> {
    let _gate = gate(&app)?;
    let db = app.state::<AppState>().db.clone();
    let (data, pair) = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        let data = runner::load_data(&conn, &job_id)?;
        let pair = store::get_pair(&conn, &data.plan.pair_id)?;
        (data, pair)
    };
    let providers = providers(&app)?;
    let mut same = true;
    for side in Side::ALL {
        let observed = providers[side.index()]
            .read_playlist(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        same &= runner::same_state(&data.expected[side.index()], &observed);
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    let mut result = inspection(&conn, &job_id)?;
    if !same {
        result.can_resume = false;
        result.next_action="Remote state differs from the durable checkpoint; manual review required. Membership cannot prove uncertain outcomes.".into();
    }
    Ok(result)
}
#[tauri::command]
pub fn control_sync_run(
    app: AppHandle,
    job_id: String,
    action: String,
) -> Result<SyncUpdate, String> {
    let state = app.state::<AppState>();
    let active = {
        state
            .active_transfers
            .lock()
            .map_err(|_| "Active job registry unavailable")?
            .get(&job_id)
            .cloned()
    };
    if !["pause", "resume", "cancel"].contains(&action.as_str()) {
        return Err("Unknown sync control".into());
    }
    let pair = {
        let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
        runner::load_data(&conn, &job_id)?.plan.pair_id
    };
    if let Some(control) = active {
        let update = match action.as_str() {
            "pause" => control.pause()?,
            "resume" => control.resume()?,
            _ => control.cancel()?,
        };
        return Ok(SyncUpdate {
            pair_id: pair,
            job_id,
            state: update.state.as_str().into(),
            event_seq: update.event_seq,
        });
    }
    let conn = state.db.lock().map_err(|_| "Storage lock unavailable")?;
    let summary = runner::inspect(&conn, &job_id)?;
    if ["running", "pausing", "paused", "cancelling", "planned"].contains(&summary.state.as_str()) {
        return Err("Sync was interrupted; inspect recovery before continuing".into());
    }
    Ok(SyncUpdate {
        pair_id: pair,
        job_id,
        state: summary.state,
        event_seq: summary.event_seq,
    })
}

pub async fn scheduler_loop(app: AppHandle) {
    loop {
        if app.state::<AppState>().shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let result = scheduled_tick(&app).await;
        if let Err(error) = result {
            let _ = app.emit(
                "sync:scheduler_error",
                crate::diagnostics::sanitize_for_log(&error),
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }
}
async fn scheduled_tick(app: &AppHandle) -> Result<(), String> {
    let Ok(_guard) = gate(app) else {
        return Ok(());
    };
    let db = app.state::<AppState>().db.clone();
    let pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
        scheduler::claim_due(&conn, chrono::Utc::now().timestamp())?
    };
    let Some(pair) = pair else {
        return Ok(());
    };
    let result = async {
        let providers = providers(app)?;
        let plan = service::preview(&db, &pair.pair_id, &providers, None).await?;
        if !plan.conflicts.is_empty() {
            return Err("Scheduled sync needs match or conflict review; open Sync now".to_string());
        }
        let job = new_id("sync");
        let control = control(app, &pair.pair_id, &job)?;
        let result = runner::execute(db.clone(), &plan.plan_id, providers, control).await;
        cleanup(app, &job);
        result.map(|_| ())
    }
    .await;
    if let Err(error) = result {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        scheduler::block(&conn, &pair.pair_id, "needs_review", &error)?;
    }
    let _ = app.emit("sync:pairs_changed", ());
    Ok(())
}
