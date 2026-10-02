use crate::auth::sapisid::{parse_spotify_cookie, parse_ytmusic_cookie, RawCookie};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;

/// Opens an isolated native WebView popup for authentication trap.
/// Fulfills D-01: Dedicated native popup window.
/// Auto-closes upon detecting target cookies and emits auth:status_changed per D-02.
#[tauri::command]
pub async fn open_auth_window(app: tauri::AppHandle, service: String) -> Result<(), String> {
    let (target_url_str, label, title) = match service.as_str() {
        "ytmusic" => (
            "https://music.youtube.com",
            "auth-ytmusic",
            "Connect YouTube Music",
        ),
        "spotify" => (
            "https://accounts.spotify.com/en/login",
            "auth-spotify",
            "Connect Spotify",
        ),
        _ => return Err(format!("Unsupported service: {}", service)),
    };

    // If window is already open, focus it
    if let Some(existing_window) = app.get_webview_window(label) {
        let _ = existing_window.set_focus();
        return Ok(());
    }

    let parsed_url = target_url_str
        .parse::<Url>()
        .map_err(|e| format!("Invalid URL: {}", e))?;

    let webview_url = WebviewUrl::External(parsed_url.clone());

    let _window = WebviewWindowBuilder::new(&app, label, webview_url)
        .title(title)
        .inner_size(600.0, 720.0)
        .resizable(true)
        .build()
        .map_err(|e| format!("Failed to create auth webview window: {}", e))?;

    let app_handle = app.clone();
    let service_id = service.clone();

    // Asynchronously poll for cookies off the main thread to prevent WebView2 deadlock
    tokio::spawn(async move {
        let poll_interval = Duration::from_millis(800);
        let max_attempts = 375; // ~5 minutes max polling timeout
        let mut attempts = 0;

        while attempts < max_attempts {
            tokio::time::sleep(poll_interval).await;
            attempts += 1;

            // Check if window was closed by the user
            if let Some(w) = app_handle.get_webview_window(&format!("auth-{}", service_id)) {
                let cookies_result = w.cookies_for_url(parsed_url.clone());
                if let Ok(cookies) = cookies_result {
                    let raw_cookies: Vec<RawCookie> = cookies
                        .into_iter()
                        .map(|c| RawCookie {
                            name: c.name().to_string(),
                            value: c.value().to_string(),
                        })
                        .collect();

                    let captured_token = match service_id.as_str() {
                        "ytmusic" => parse_ytmusic_cookie(&raw_cookies),
                        "spotify" => parse_spotify_cookie(&raw_cookies),
                        _ => None,
                    };

                    if let Some(token) = captured_token {
                        // Persist credential into OS Keyring with AES-256 fallback (AUTH-04, D-03)
                        if let Some(state) = app_handle.try_state::<crate::AppState>() {
                            if let Ok(conn) = state.db.lock() {
                                let _ = crate::auth::keyring_store::store_credential(
                                    &service_id,
                                    &token,
                                    &conn,
                                );
                            }
                        }

                        // Credential detected: notify frontend with payload per D-02
                        let payload = serde_json::json!({
                            "service": service_id,
                            "status": "connected"
                        });

                        let _ = app_handle.emit("auth:status_changed", payload);

                        // Auto-close popup window immediately per D-01/D-02
                        let _ = w.close();
                        break;
                    }
                }
            } else {
                // Window no longer exists (user closed it)
                break;
            }
        }
    });

    Ok(())
}
