use crate::AppState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub fn acquire_gate(
    gate: std::sync::Arc<tokio::sync::Mutex<()>>,
    closing: &std::sync::atomic::AtomicBool,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    if closing.load(Ordering::SeqCst) {
        return Err("SoundShift is closing; no new work can start".into());
    }
    let guard = gate
        .try_lock_owned()
        .map_err(|_| "Another transfer, sync, or recovery action is active")?;
    if closing.load(Ordering::SeqCst) {
        return Err("SoundShift is closing; no new work can start".into());
    }
    Ok(guard)
}

/// Keep the native window alive until accepted requests and their checkpoints drain.
pub fn request_shutdown(app: &AppHandle) {
    if app
        .state::<AppState>()
        .shutting_down
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let controls = {
                let state = app.state::<AppState>();
                state
                    .active_transfers
                    .lock()
                    .map(|jobs| jobs.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            for control in controls {
                let _ = control.cancel();
            }
            let gate = app.state::<AppState>().mutation_gate.clone();
            if let Ok(guard) = gate.try_lock_owned() {
                app.state::<AppState>()
                    .exit_ready
                    .store(true, Ordering::SeqCst);
                drop(guard);
                app.exit(0);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    });
}
