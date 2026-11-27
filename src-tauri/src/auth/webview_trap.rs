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
        .incognito(true)
        .build()
        .map_err(|e| format!("Failed to create auth webview window: {}", e))?;

    let app_handle = app.clone();
    let service_id = service.clone();

    // Asynchronously poll for cookies off the main thread to prevent WebView2 deadlock
    tokio::spawn(async move {
        let poll_interval = Duration::from_millis(800);
        let max_attempts: u32 = 375; // ~5 minutes max polling timeout
        let mut attempts: u32 = 0;
        let mut last_rejected_token: Option<(String, u32)> = None;

        while attempts < max_attempts {
            tokio::time::sleep(poll_interval).await;
            attempts += 1;

            // Check if window was closed by the user
            if let Some(w) = app_handle.get_webview_window(&format!("auth-{}", service_id)) {
                let mut raw_cookies: Vec<RawCookie> = Vec::new();

                if let Ok(cookies) = w.cookies_for_url(parsed_url.clone()) {
                    raw_cookies.extend(cookies.into_iter().map(|c| RawCookie {
                        name: c.name().to_string(),
                        value: c.value().to_string(),
                    }));
                }

                // If service is spotify and token not found yet, also check open.spotify.com
                if service_id == "spotify" && parse_spotify_cookie(&raw_cookies).is_none() {
                    if let Ok(open_url) = "https://open.spotify.com".parse::<Url>() {
                        if let Ok(spotify_cookies) = w.cookies_for_url(open_url) {
                            raw_cookies.extend(spotify_cookies.into_iter().map(|c| RawCookie {
                                name: c.name().to_string(),
                                value: c.value().to_string(),
                            }));
                        }
                    }
                }

                // Only collect YouTube Music / YouTube domain cookies (never .google.com mid-redirect cookies)
                if service_id == "ytmusic" {
                    if let Ok(yt_url) = "https://www.youtube.com".parse::<Url>() {
                        if let Ok(yt_cookies) = w.cookies_for_url(yt_url) {
                            raw_cookies.extend(yt_cookies.into_iter().map(|c| RawCookie {
                                name: c.name().to_string(),
                                value: c.value().to_string(),
                            }));
                        }
                    }
                }

                let captured_token = match service_id.as_str() {
                    "ytmusic" => parse_ytmusic_cookie(&raw_cookies),
                    "spotify" => parse_spotify_cookie(&raw_cookies),
                    _ => None,
                };

                if let Some(token) = captured_token {
                    if let Some((ref prev_token, prev_attempt)) = last_rejected_token {
                        if prev_token == &token && attempts.saturating_sub(prev_attempt) < 4 {
                            continue;
                        }
                    }

                    // Verify live provider session before accepting and closing popup
                    let session_verified = match service_id.as_str() {
                        "ytmusic" => {
                            use crate::providers::traits::MusicProvider;
                            crate::providers::ytmusic::YouTubeMusicProvider::new(token.clone())
                                .check_session()
                                .await
                                .is_ok()
                        }
                        "spotify" => {
                            use crate::providers::traits::MusicProvider;
                            crate::providers::spotify::SpotifyProvider::new(token.clone())
                                .check_session()
                                .await
                                .is_ok()
                        }
                        _ => false,
                    };

                    if !session_verified {
                        last_rejected_token = Some((token, attempts));
                        continue;
                    }

                    // Persist credential into OS Keyring with AES-256 fallback (AUTH-04, D-03)
                    let store_res = if let Some(state) = app_handle.try_state::<crate::AppState>() {
                        if let Ok(conn) = state.db.lock() {
                            crate::auth::keyring_store::store_credential(&service_id, &token, &conn)
                        } else {
                            Err("Database lock unavailable".to_string())
                        }
                    } else {
                        Err("Application state unavailable".to_string())
                    };

                    match store_res {
                        Ok(()) => {
                            let payload = serde_json::json!({
                                "service": service_id,
                                "status": "connected"
                            });
                            let _ = app_handle.emit("auth:status_changed", payload);
                        }
                        Err(e) => {
                            crate::diagnostics::write_error(&format!(
                                "Failed to store credential for {}: {}",
                                service_id, e
                            ));
                            let payload = serde_json::json!({
                                "service": service_id,
                                "status": "disconnected",
                                "error": crate::diagnostics::sanitize_for_log(&format!("Failed to persist credentials: {}", e))
                            });
                            let _ = app_handle.emit("auth:status_changed", payload);
                        }
                    }

                    // Auto-close popup window immediately per D-01/D-02
                    let _ = w.close();
                    return;
                }
            } else {
                // Window closed manually before completion; clear loading spinner using persisted status
                let fallback_status = if let Some(state) = app_handle.try_state::<crate::AppState>()
                {
                    if let Ok(conn) = state.db.lock() {
                        if crate::auth::keyring_store::retrieve_credential(&service_id, &conn)
                            .ok()
                            .flatten()
                            .is_some()
                        {
                            "connected"
                        } else {
                            "disconnected"
                        }
                    } else {
                        "disconnected"
                    }
                } else {
                    "disconnected"
                };
                let _ = app_handle.emit(
                    "auth:status_changed",
                    serde_json::json!({
                        "service": service_id,
                        "status": fallback_status
                    }),
                );
                return;
            }
        }
    });

    Ok(())
}
