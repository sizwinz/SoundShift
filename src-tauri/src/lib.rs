pub mod auth;
pub mod engine;
pub mod models;
pub mod providers;
pub mod storage;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            auth::webview_trap::open_auth_window,
            auth::keyring_store::check_auth,
            auth::keyring_store::disconnect_account,
            providers::commands::list_provider_playlists,
            providers::commands::fetch_playlist_tracks,
            engine::commands::execute_playlist_matching
        ])
        .setup(|app| {
            let app_dir = app.path().app_data_dir().expect("failed to get app data dir");
            let conn = storage::init_db(&app_dir).expect("failed to initialize sqlite database");
            app.manage(AppState {
                db: Mutex::new(conn),
            });

            // Asynchronously center, reveal, and focus window after event loop initialization
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                if let Some(main_win) = handle.get_webview_window("main") {
                    let _ = main_win.center();
                    let _ = main_win.unminimize();
                    let _ = main_win.show();
                    let _ = main_win.set_focus();
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
