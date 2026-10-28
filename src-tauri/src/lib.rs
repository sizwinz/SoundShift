pub mod auth;
pub mod diagnostics;
pub mod engine;
pub mod models;
pub mod providers;
pub mod storage;
pub mod sync;

use crate::engine::worker::TransferControl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub active_transfers: Arc<Mutex<HashMap<String, TransferControl>>>,
    pub mutation_gate: Arc<tokio::sync::Mutex<()>>,
    pub shutting_down: Arc<AtomicBool>,
    pub exit_ready: Arc<AtomicBool>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));
    builder
        .invoke_handler(tauri::generate_handler![
            auth::webview_trap::open_auth_window,
            auth::keyring_store::check_auth,
            auth::keyring_store::disconnect_account,
            providers::commands::list_provider_playlists,
            providers::commands::check_provider_connection,
            providers::commands::fetch_playlist_tracks,
            providers::commands::search_provider_tracks,
            providers::commands::resolve_track_by_url,
            engine::commands::execute_playlist_matching,
            engine::commands::start_batch_transfer,
            engine::commands::pause_batch_transfer,
            engine::commands::resume_batch_transfer,
            engine::commands::cancel_batch_transfer,
            engine::commands::execute_snapshot_rollback,
            engine::commands::audit_playlist_integrity,
            engine::commands::get_transfer_history,
            engine::commands::inspect_transfer_job,
            engine::commands::reconcile_transfer_job,
            engine::commands::resume_recorded_transfer,
            engine::commands::get_storage_recovery_notice,
            engine::commands::acknowledge_storage_recovery,
            sync::commands::list_sync_pairs,
            sync::commands::create_sync_pair,
            sync::commands::preview_sync_pair,
            sync::commands::execute_sync_plan,
            sync::commands::set_sync_schedule,
            sync::commands::unlink_sync_pair,
            sync::commands::revalidate_sync_pair,
            sync::commands::inspect_sync_run,
            sync::commands::resume_sync_run,
            sync::commands::reconcile_sync_run,
            sync::commands::control_sync_run
        ])
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            let conn = storage::init_db(&app_dir).expect("failed to initialize sqlite database");
            sync::scheduler::recover_startup(&conn, chrono::Utc::now().timestamp())
                .expect("failed to inspect interrupted sync runs");
            app.manage(AppState {
                db: Arc::new(Mutex::new(conn)),
                active_transfers: Arc::new(Mutex::new(HashMap::new())),
                mutation_gate: Arc::new(tokio::sync::Mutex::new(())),
                shutting_down: Arc::new(AtomicBool::new(false)),
                exit_ready: Arc::new(AtomicBool::new(false)),
            });
            let scheduler_app = app.handle().clone();
            tauri::async_runtime::spawn(sync::commands::scheduler_loop(scheduler_app));

            // Asynchronously center, reveal, set icon, and focus window after event loop initialization
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                if let Some(main_win) = handle.get_webview_window("main") {
                    let icon_bytes = include_bytes!("../icons/icon.png");
                    if let Ok(icon) = tauri::image::Image::from_bytes(icon_bytes) {
                        let _ = main_win.set_icon(icon);
                    }
                    let _ = main_win.center();
                    let _ = main_win.unminimize();
                    let _ = main_win.show();
                    let _ = main_win.set_focus();
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    sync::runtime::request_shutdown(window.app_handle());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !app.state::<AppState>().exit_ready.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    sync::runtime::request_shutdown(app);
                }
            }
        });
}
