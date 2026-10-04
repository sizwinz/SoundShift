use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use keyring::Entry;
use rand::{thread_rng, Rng};
use rusqlite::{params, Connection};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const SERVICE_NAME: &str = "SoundShift";

/// Generates a random AES-256-GCM envelope for fallback-only secret storage.
fn generate_fallback_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    thread_rng().fill(&mut key[..]);
    key
}

/// Creates an encrypted envelope payload with a random nonce and per-entry key.
pub fn encrypt_aes_gcm(plaintext: &str) -> Result<String, String> {
    let key = generate_fallback_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce: [u8; 12] = thread_rng().gen();
    let nonce = Nonce::from_slice(&nonce);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption error: {}", e))?;

    let key_hex = format!("{:x}", fmt_hex::Hex(&key));
    let nonce_hex = format!("{:x}", fmt_hex::Hex(nonce.as_slice()));
    let ciphertext_hex = format!("{:x}", fmt_hex::Hex(&ciphertext));
    let envelope = format!("{}:{}", key_hex, nonce_hex);
    Ok(format!("{}:{}", envelope, ciphertext_hex))
}

/// Transparently decrypts ciphertext secret using the random envelope stored with the value.
pub fn decrypt_aes_gcm(ciphertext_hex: &str) -> Result<String, String> {
    let trimmed = ciphertext_hex.trim();
    if trimmed.is_empty() {
        return Err("Empty ciphertext payload".to_string());
    }

    let parts: Vec<&str> = trimmed.split(':').collect();

    if parts.len() == 3 {
        let key_bytes = hex_to_bytes(parts[0]).map_err(|e| format!("Key decode error: {}", e))?;
        let nonce_bytes =
            hex_to_bytes(parts[1]).map_err(|e| format!("Nonce decode error: {}", e))?;
        let cipher_bytes =
            hex_to_bytes(parts[2]).map_err(|e| format!("Ciphertext decode error: {}", e))?;

        let key: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| "Fallback secret key length mismatch".to_string())?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext_bytes = cipher
            .decrypt(nonce, cipher_bytes.as_ref())
            .map_err(|e| format!("Decryption error: {}", e))?;
        return String::from_utf8(plaintext_bytes).map_err(|e| e.to_string());
    }

    let key = [0u8; 32];
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(b"SoundShift12");
    let bytes = hex_to_bytes(trimmed).map_err(|e| format!("Hex decode error: {}", e))?;
    let plaintext_bytes = cipher
        .decrypt(nonce, bytes.as_ref())
        .map_err(|e| format!("Decryption error: {}", e))?;

    String::from_utf8(plaintext_bytes).map_err(|e| e.to_string())
}

mod fmt_hex {
    use std::fmt;

    pub struct Hex<'a>(pub &'a [u8]);

    impl<'a> fmt::LowerHex for Hex<'a> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for b in self.0 {
                write!(f, "{:02x}", b)?;
            }
            Ok(())
        }
    }
}

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("Invalid hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("Invalid hex byte: {}", e))
        })
        .collect()
}

/// Stores credentials in the OS keyring. The SQLite fallback is only used when the keyring is unavailable
/// and stores a versioned random-envelope payload rather than a deterministic key.
pub fn store_credential(service: &str, secret: &str, conn: &Connection) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        if entry.set_password(secret).is_ok() {
            // Prefer the OS keyring as the authoritative credential store. If it succeeds,
            // do not populate the SQLite fallback so stale fallback entries do not mask the
            // live platform credential state.
            return Ok(());
        }
    }

    let encrypted = encrypt_aes_gcm(secret)?;
    conn.execute(
        "INSERT OR REPLACE INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
        params![service, format!("fallback:v1:{}", encrypted), now],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Retrieves stored credentials, checking OS Keyring first, then SQLite fallback.
pub fn retrieve_credential(service: &str, conn: &Connection) -> Result<Option<String>, String> {
    // 1. Primary check: platform OS Keyring
    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        if let Ok(password) = entry.get_password() {
            if !password.is_empty() {
                return Ok(Some(password));
            }
        }
    }

    // 2. Fall back to SQLite database lookup
    let mut stmt = conn
        .prepare("SELECT auth_data FROM service_sessions WHERE service_id = ?1")
        .map_err(|e| e.to_string())?;

    let auth_data: Result<String, _> = stmt.query_row(params![service], |row| row.get(0));

    match auth_data {
        Ok(data) => {
            if let Some(encrypted_part) = data.strip_prefix("fallback:v1:") {
                let decrypted = decrypt_aes_gcm(encrypted_part)?;
                Ok(Some(decrypted))
            } else if let Some(encrypted_part) = data.strip_prefix("encrypted:") {
                let decrypted = decrypt_aes_gcm(encrypted_part)?;
                Ok(Some(decrypted))
            } else if data == "keyring:managed" {
                Ok(None)
            } else {
                Ok(Some(data))
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Purges credentials from both OS Keyring and SQLite service_sessions (D-04).
/// Explicitly preserves reusable track match caches in track_match_cache.
pub fn purge_credential(service: &str, conn: &Connection) -> Result<(), String> {
    // Delete from OS Keyring
    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        let _ = entry.delete_credential();
    }

    // Delete session from SQLite database (preserving track_match_cache)
    conn.execute(
        "DELETE FROM service_sessions WHERE service_id = ?1",
        params![service],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Validates session status asynchronously in the background (D-07).
/// Returns "connected", "expired", or "disconnected".
#[tauri::command]
pub async fn check_auth(
    app: tauri::AppHandle,
    service: String,
) -> Result<serde_json::Value, String> {
    let state = app.state::<crate::AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let credential = retrieve_credential(&service, &conn)?;

    match credential {
        None => Ok(serde_json::json!({
            "service": service,
            "status": "disconnected"
        })),
        Some(token) => {
            let is_valid = match service.as_str() {
                "ytmusic" => {
                    !token.trim().is_empty()
                        && token.contains('=')
                        && (token.contains("SAPISID") || token.contains("__Secure-3PAPISID"))
                }
                "spotify" => !token.trim().is_empty(),
                _ => false,
            };

            if is_valid {
                Ok(serde_json::json!({
                    "service": service,
                    "status": "connected"
                }))
            } else {
                Ok(serde_json::json!({
                    "service": service,
                    "status": "expired"
                }))
            }
        }
    }
}

/// Disconnects an account, completely purging stored credentials while preserving track match cache (D-04).
#[tauri::command]
pub async fn disconnect_account(app: tauri::AppHandle, service: String) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    purge_credential(&service, &conn)?;

    let payload = serde_json::json!({
        "service": service,
        "status": "disconnected"
    });

    use tauri::Emitter;
    let _ = app.emit("auth:status_changed", payload);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::create_tables;

    #[test]
    fn test_aes_gcm_encryption_roundtrip() {
        let secret = "AQB-sample-sp_dc-token-123456789";
        let encrypted = encrypt_aes_gcm(secret).expect("encryption failed");
        assert_ne!(secret, encrypted);

        let decrypted = decrypt_aes_gcm(&encrypted).expect("decryption failed");
        assert_eq!(secret, decrypted);
    }

    #[test]
    fn test_sqlite_fallback_and_purge() {
        let conn = Connection::open_in_memory().expect("open memory db");
        create_tables(&conn).expect("create tables");

        // Insert fake track match cache entry to verify preservation (D-04)
        conn.execute(
            "INSERT INTO track_match_cache (source_service, source_track_id, target_service, target_track_id, match_method, confidence, created_at) VALUES ('spotify', 'track1', 'ytmusic', 'video1', 'isrc', 1.0, 1000)",
            [],
        ).expect("insert track match cache");

        // Store fallback credential
        let secret = "ytmusic_sapisid_test_token";
        let encrypted = encrypt_aes_gcm(secret).expect("encrypt");
        conn.execute(
            "INSERT OR REPLACE INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
            params!["test_ytmusic_service", format!("encrypted:{}", encrypted), 1000],
        ).expect("insert session");

        // Retrieve and verify
        let retrieved = retrieve_credential("test_ytmusic_service", &conn).expect("retrieve");
        assert_eq!(retrieved, Some(secret.to_string()));

        // Purge credential
        purge_credential("test_ytmusic_service", &conn).expect("purge");

        // Verify session deleted
        let retrieved_after =
            retrieve_credential("test_ytmusic_service", &conn).expect("retrieve after purge");
        assert_eq!(retrieved_after, None);

        // Verify track_match_cache preserved intact (D-04)
        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_match_cache WHERE source_service = 'spotify'",
                [],
                |row| row.get(0),
            )
            .expect("query cache count");
        assert_eq!(cache_count, 1);
    }

    #[test]
    fn test_store_and_retrieve_credential() {
        let conn = Connection::open_in_memory().expect("open memory db");
        create_tables(&conn).expect("create tables");

        let secret = "AQB-sample-sp_dc-token-123456789";
        store_credential("test_spotify_service", secret, &conn).expect("store credential");
        let retrieved =
            retrieve_credential("test_spotify_service", &conn).expect("retrieve credential");
        assert_eq!(retrieved, Some(secret.to_string()));

        // Clean up
        purge_credential("test_spotify_service", &conn).expect("purge");
        assert_eq!(
            retrieve_credential("test_spotify_service", &conn).expect("retrieve"),
            None
        );
    }

    #[test]
    fn test_sqlite_fallback_when_keyring_fails() {
        let conn = Connection::open_in_memory().expect("open memory db");
        create_tables(&conn).expect("create tables");

        let secret = "fallback_secret_xyz_987";
        let encrypted = encrypt_aes_gcm(secret).expect("encrypt");
        conn.execute(
            "INSERT INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
            params![
                "mock_nonexistent_service",
                format!("encrypted:{}", encrypted),
                1000
            ],
        )
        .expect("insert session");

        let retrieved = retrieve_credential("mock_nonexistent_service", &conn)
            .expect("retrieve from sqlite fallback");
        assert_eq!(retrieved, Some(secret.to_string()));
    }
}
