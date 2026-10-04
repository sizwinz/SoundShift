use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use keyring::Entry;
use rand::{thread_rng, Rng};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const SERVICE_NAME: &str = "SoundShift";

/// Derives a consistent 256-bit machine master key from an application salt and host identity.
fn get_fallback_master_key() -> Result<[u8; 32], String> {
    let machine_id = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "SoundShiftFallbackHost".to_string());
    let user_name = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "SoundShiftUser".to_string());

    let seed = format!("SoundShift::Entropy::Salt::v2::{}:{}", machine_id, user_name);
    let hash = Sha256::digest(seed.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    Ok(key)
}

/// Encrypts plaintext using AES-256-GCM with the host-derived master key and a random 96-bit nonce.
/// Produces a versioned envelope in format `fallback:v2:{nonce_hex}:{ciphertext_hex}`.
pub fn encrypt_aes_gcm(plaintext: &str) -> Result<String, String> {
    let key = get_fallback_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    thread_rng().fill(&mut nonce_bytes[..]);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption error: {}", e))?;

    let nonce_hex = format!("{:x}", fmt_hex::Hex(&nonce_bytes));
    let ciphertext_hex = format!("{:x}", fmt_hex::Hex(&ciphertext));
    Ok(format!("fallback:v2:{}:{}", nonce_hex, ciphertext_hex))
}

/// Decrypts ciphertext secret. Supports `fallback:v2:` using the host-derived master key,
/// with backwards-compatible read support for legacy `fallback:v1:` records.
pub fn decrypt_aes_gcm(payload: &str) -> Result<String, String> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return Err("Empty ciphertext payload".to_string());
    }

    if let Some(v2_body) = trimmed.strip_prefix("fallback:v2:") {
        let parts: Vec<&str> = v2_body.split(':').collect();
        if parts.len() != 2 {
            return Err("Invalid fallback:v2 payload format".to_string());
        }
        let nonce_bytes =
            hex_to_bytes(parts[0]).map_err(|e| format!("Nonce decode error: {}", e))?;
        let cipher_bytes =
            hex_to_bytes(parts[1]).map_err(|e| format!("Ciphertext decode error: {}", e))?;
        let key = get_fallback_master_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext_bytes = cipher
            .decrypt(nonce, cipher_bytes.as_ref())
            .map_err(|e| format!("Decryption error: {}", e))?;
        return String::from_utf8(plaintext_bytes).map_err(|e| e.to_string());
    }

    let legacy_body = trimmed.strip_prefix("fallback:v1:").unwrap_or(trimmed);
    let parts: Vec<&str> = legacy_body.split(':').collect();

    if parts.len() == 3 {
        let key_bytes =
            hex_to_bytes(parts[0]).map_err(|e| format!("Key decode error: {}", e))?;
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

    if parts.len() == 2 {
        let nonce_bytes =
            hex_to_bytes(parts[0]).map_err(|e| format!("Nonce decode error: {}", e))?;
        let cipher_bytes =
            hex_to_bytes(parts[1]).map_err(|e| format!("Ciphertext decode error: {}", e))?;
        let key = get_fallback_master_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext_bytes = cipher
            .decrypt(nonce, cipher_bytes.as_ref())
            .map_err(|e| format!("Decryption error: {}", e))?;
        return String::from_utf8(plaintext_bytes).map_err(|e| e.to_string());
    }

    Err("Unrecognized ciphertext envelope format".to_string())
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

/// Stores credentials in the OS keyring. When keyring is unavailable, stores in SQLite
/// using the host-derived master key in versioned envelope format `fallback:v2:nonce:ciphertext`.
pub fn store_credential(service: &str, secret: &str, conn: &Connection) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        if entry.set_password(secret).is_ok() {
            return Ok(());
        }
    }

    let encrypted = encrypt_aes_gcm(secret)?;
    conn.execute(
        "INSERT OR REPLACE INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
        params![service, encrypted, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Retrieves stored credentials, checking OS Keyring first, then SQLite fallback.
/// Migrates legacy `fallback:v1:` records to `fallback:v2:` on first read.
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
            if data.starts_with("fallback:v2:") {
                let decrypted = decrypt_aes_gcm(&data)?;
                Ok(Some(decrypted))
            } else if data.starts_with("fallback:v1:") {
                let decrypted = decrypt_aes_gcm(&data)?;
                if let Ok(re_encrypted) = encrypt_aes_gcm(&decrypted) {
                    let _ = conn.execute(
                        "UPDATE service_sessions SET auth_data = ?1 WHERE service_id = ?2",
                        params![re_encrypted, service],
                    );
                }
                Ok(Some(decrypted))
            } else if let Some(encrypted_part) = data.strip_prefix("encrypted:") {
                let decrypted = decrypt_aes_gcm(encrypted_part)?;
                if let Ok(re_encrypted) = encrypt_aes_gcm(&decrypted) {
                    let _ = conn.execute(
                        "UPDATE service_sessions SET auth_data = ?1 WHERE service_id = ?2",
                        params![re_encrypted, service],
                    );
                }
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
        assert!(encrypted.starts_with("fallback:v2:"));

        let decrypted = decrypt_aes_gcm(&encrypted).expect("decryption failed");
        assert_eq!(secret, decrypted);
    }

    #[test]
    fn test_fallback_master_key_consistency() {
        let key1 = get_fallback_master_key().expect("key derivation 1");
        let key2 = get_fallback_master_key().expect("key derivation 2");
        assert_eq!(key1, key2);
        assert_ne!(key1, [0u8; 32]);
    }

    #[test]
    fn test_legacy_fallback_v1_migration() {
        let conn = Connection::open_in_memory().expect("open memory db");
        create_tables(&conn).expect("create tables");

        // Manually construct a legacy v1 envelope: fallback:v1:key_hex:nonce_hex:ciphertext_hex
        let legacy_key = [7u8; 32];
        let legacy_nonce = [9u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&legacy_key).expect("cipher");
        let secret = "legacy_secret_cookie_token_456";
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&legacy_nonce), secret.as_bytes())
            .expect("encrypt");

        let key_hex = format!("{:x}", fmt_hex::Hex(&legacy_key));
        let nonce_hex = format!("{:x}", fmt_hex::Hex(&legacy_nonce));
        let cipher_hex = format!("{:x}", fmt_hex::Hex(&ciphertext));
        let legacy_payload = format!("fallback:v1:{}:{}:{}", key_hex, nonce_hex, cipher_hex);

        conn.execute(
            "INSERT INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
            params!["test_legacy_migration_service", legacy_payload, 1000],
        )
        .expect("insert legacy session");

        // Retrieve should decrypt and migrate to fallback:v2
        let retrieved =
            retrieve_credential("test_legacy_migration_service", &conn).expect("retrieve legacy");
        assert_eq!(retrieved, Some(secret.to_string()));

        // Check that auth_data in SQLite was migrated to fallback:v2
        let updated_auth_data: String = conn
            .query_row(
                "SELECT auth_data FROM service_sessions WHERE service_id = 'test_legacy_migration_service'",
                [],
                |row| row.get(0),
            )
            .expect("query updated auth_data");
        assert!(updated_auth_data.starts_with("fallback:v2:"));

        // Subsequent retrieve should succeed with migrated v2 record
        let second_retrieved =
            retrieve_credential("test_legacy_migration_service", &conn).expect("retrieve migrated");
        assert_eq!(second_retrieved, Some(secret.to_string()));
    }

    #[test]
    fn test_zero_key_rejected() {
        // Plain hex that cannot be decoded as valid v1/v2 envelope
        let invalid = "00112233445566778899aabbccddeeff";
        let res = decrypt_aes_gcm(invalid);
        assert!(res.is_err());
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
