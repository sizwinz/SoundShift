pub mod auth;
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
            auth::keyring_store::disconnect_account
        ])
        .setup(|app| {
            let app_dir = app.path().app_data_dir().expect("failed to get app data dir");
            let conn = storage::init_db(&app_dir).expect("failed to initialize sqlite database");
            app.manage(AppState {
                db: Mutex::new(conn),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
