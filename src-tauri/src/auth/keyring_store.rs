use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use keyring::Entry;
#[cfg(test)]
use rand::{thread_rng, Rng};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
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

    let seed = format!(
        "SoundShift::Entropy::Salt::v2::{}:{}",
        machine_id, user_name
    );
    let hash = Sha256::digest(seed.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    Ok(key)
}

/// Encrypts plaintext using AES-256-GCM with the host-derived master key and a random 96-bit nonce.
/// Produces a versioned envelope in format `fallback:v2:{nonce_hex}:{ciphertext_hex}`.
#[cfg(test)]
fn encrypt_aes_gcm(plaintext: &str) -> Result<String, String> {
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
        if nonce_bytes.len() != 12 {
            return Err("Legacy credential nonce must contain 12 bytes".into());
        }
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext_bytes = cipher
            .decrypt(nonce, cipher_bytes.as_ref())
            .map_err(|e| format!("Decryption error: {}", e))?;
        return String::from_utf8(plaintext_bytes).map_err(|e| e.to_string());
    }

    let legacy_body = trimmed.strip_prefix("fallback:v1:").unwrap_or(trimmed);
    let parts: Vec<&str> = legacy_body.split(':').collect();

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
        if nonce_bytes.len() != 12 {
            return Err("Legacy credential nonce must contain 12 bytes".into());
        }
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
        if nonce_bytes.len() != 12 {
            return Err("Legacy credential nonce must contain 12 bytes".into());
        }
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
    if !s.is_ascii() || !s.len().is_multiple_of(2) {
        return Err("Invalid hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("Invalid hex byte: {}", e))
        })
        .collect()
}

#[cfg(target_os = "windows")]
pub mod dpapi {
    use super::fmt_hex;
    use super::hex_to_bytes;
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            pDataIn: *const DataBlob,
            szDataDescr: *const u16,
            pOptionalEntropy: *const DataBlob,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *const std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            pDataIn: *const DataBlob,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DataBlob,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *const std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    pub fn encrypt_dpapi(plaintext: &str) -> Result<String, String> {
        let bytes = plaintext.as_bytes();
        let data_in = DataBlob {
            cb_data: bytes.len() as u32,
            pb_data: bytes.as_ptr() as *mut u8,
        };
        let mut data_out = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        unsafe {
            let res = CryptProtectData(
                &data_in,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut data_out,
            );
            if res == 0 {
                return Err("DPAPI CryptProtectData failed".to_string());
            }
            let slice = std::slice::from_raw_parts(data_out.pb_data, data_out.cb_data as usize);
            let hex = format!("{:x}", fmt_hex::Hex(slice));
            LocalFree(data_out.pb_data as _);
            Ok(format!("fallback:dpapi:{}", hex))
        }
    }

    pub fn decrypt_dpapi(payload: &str) -> Result<String, String> {
        let hex_body = payload.strip_prefix("fallback:dpapi:").unwrap_or(payload);
        let mut cipher_bytes =
            hex_to_bytes(hex_body).map_err(|e| format!("DPAPI hex decode error: {}", e))?;
        let data_in = DataBlob {
            cb_data: cipher_bytes.len() as u32,
            pb_data: cipher_bytes.as_mut_ptr(),
        };
        let mut data_out = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        unsafe {
            let res = CryptUnprotectData(
                &data_in,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut data_out,
            );
            if res == 0 {
                return Err("DPAPI CryptUnprotectData failed".to_string());
            }
            let slice = std::slice::from_raw_parts(data_out.pb_data, data_out.cb_data as usize);
            let result = String::from_utf8(slice.to_vec())
                .map_err(|e| format!("DPAPI UTF-8 decode error: {}", e));
            LocalFree(data_out.pb_data as _);
            result
        }
    }
}

/// New credentials require OS-backed protection. Linux/macOS never write a
/// fallback encrypted with host/user identifiers; unlock the native vault first.
pub fn store_credential(service: &str, secret: &str, conn: &Connection) -> Result<(), String> {
    store_credential_with(service, secret, conn, || {
        let entry =
            Entry::new(SERVICE_NAME, service).map_err(|_| "Native secret storage unavailable")?;
        entry
            .set_password(secret)
            .map_err(|_| "Native secret storage unavailable")
    })
}

fn store_credential_with(
    service: &str,
    secret: &str,
    conn: &Connection,
    native_store: impl FnOnce() -> Result<(), &'static str>,
) -> Result<(), String> {
    if native_store().is_ok() {
        // A stale weak fallback must not survive a successful vault write.
        conn.execute(
            "DELETE FROM service_sessions WHERE service_id = ?1",
            [service],
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    store_platform_fallback(service, secret, conn)
}

fn store_platform_fallback(service: &str, secret: &str, conn: &Connection) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let encrypted = dpapi::encrypt_dpapi(secret)?;
        conn.execute(
            "INSERT OR REPLACE INTO service_sessions (service_id, auth_data, updated_at) VALUES (?1, ?2, ?3)",
            params![service, encrypted, chrono::Utc::now().timestamp()],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (service, secret, conn);
        Err("Secure credential storage is unavailable. Unlock or enable your OS keychain/secret service and reconnect. Credentials were not saved to a weaker fallback.".into())
    }
}

/// Read legacy values only to migrate them into an OS-protected store. Never
/// silently continue if secure migration cannot finish.
pub fn retrieve_credential(service: &str, conn: &Connection) -> Result<Option<String>, String> {
    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        if let Ok(password) = entry.get_password() {
            if !password.is_empty() {
                // Only a successful new vault write establishes that an older
                // local row is obsolete. A read must not discard recovery data.
                return Ok(Some(password));
            }
        }
    }
    retrieve_local_credential(service, conn)
}

fn retrieve_local_credential(service: &str, conn: &Connection) -> Result<Option<String>, String> {
    retrieve_local_credential_with(service, conn, |secret| {
        #[cfg(target_os = "windows")]
        {
            store_platform_fallback(service, secret, conn)
        }
        #[cfg(not(target_os = "windows"))]
        {
            store_credential(service, secret, conn)
        }
    })
}

fn retrieve_local_credential_with(
    service: &str,
    conn: &Connection,
    secure_migration: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension;
    let data: Option<String> = conn
        .query_row(
            "SELECT auth_data FROM service_sessions WHERE service_id = ?1",
            [service],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(data) = data else {
        return Ok(None);
    };
    if data == "keyring:managed" {
        return Err("Unlock the OS credential vault or reconnect this account".into());
    }
    if data.starts_with("fallback:dpapi:") {
        #[cfg(target_os = "windows")]
        {
            return dpapi::decrypt_dpapi(&data).map(Some);
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Err(
                "This credential is protected by Windows. Reconnect on this operating system."
                    .into(),
            );
        }
    }
    let secret = if data.starts_with("fallback:v1:") || data.starts_with("fallback:v2:") {
        decrypt_aes_gcm(&data)?
    } else if let Some(encrypted) = data.strip_prefix("encrypted:") {
        decrypt_aes_gcm(encrypted)?
    } else if data.starts_with("fallback:") {
        return Err("Unrecognized credential envelope. Reconnect this account.".into());
    } else {
        // Migrate historical plaintext rows, never return them without securing storage.
        data
    };
    secure_migration(&secret)?;
    Ok(Some(secret))
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

/// Redacts sensitive auth tokens, cookies, and secrets from log strings or diagnostic dumps (DATA-07).
pub use crate::diagnostics::sanitize_for_log;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::create_tables;

    #[test]
    fn unavailable_native_store_uses_only_os_protected_fallback() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        let result = store_credential_with("isolated_test", "secret", &conn, || Err("unavailable"));
        #[cfg(target_os = "windows")]
        {
            result.unwrap();
            let stored: String = conn
                .query_row("SELECT auth_data FROM service_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(stored.starts_with("fallback:dpapi:"));
            assert_eq!(dpapi::decrypt_dpapi(&stored).unwrap(), "secret");
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(result
                .unwrap_err()
                .contains("Secure credential storage is unavailable"));
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM service_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "no host-derived fallback may be written");
        }
    }

    #[test]
    fn successful_native_write_removes_stale_weak_fallback() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.execute("INSERT INTO service_sessions (service_id, auth_data, updated_at) VALUES ('isolated_test', 'legacy_plaintext', 1)", []).unwrap();
        store_credential_with("isolated_test", "new_secret", &conn, || Ok(())).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM service_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn failed_secure_migration_never_returns_legacy_secret() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        let legacy = encrypt_aes_gcm("old_secret").unwrap();
        conn.execute("INSERT INTO service_sessions (service_id, auth_data, updated_at) VALUES ('isolated_test', ?1, 1)", [&legacy]).unwrap();
        let result = retrieve_local_credential_with("isolated_test", &conn, |_| {
            Err("unlock your vault".into())
        });
        assert_eq!(result.unwrap_err(), "unlock your vault");
        let stored: String = conn
            .query_row("SELECT auth_data FROM service_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            stored, legacy,
            "migration failure preserves the original record"
        );
    }

    #[test]
    fn malformed_legacy_envelopes_return_errors_instead_of_panicking() {
        for value in [
            "fallback:v2:01:abcd",
            "fallback:v1:000000:01:abcd",
            "fallback:v2:💿:abcd",
        ] {
            assert!(decrypt_aes_gcm(value).is_err());
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_dpapi_encryption_roundtrip() {
        let secret = "AQB-sample-sp_dc-dpapi-token-987654321";
        let encrypted = dpapi::encrypt_dpapi(secret).expect("dpapi encryption failed");
        assert_ne!(secret, encrypted);
        assert!(encrypted.starts_with("fallback:dpapi:"));

        let decrypted = dpapi::decrypt_dpapi(&encrypted).expect("dpapi decryption failed");
        assert_eq!(secret, decrypted);
    }

    #[test]
    fn test_sanitize_for_log_redacts_sensitive_tokens() {
        let input =
            "Cookie: sp_dc=AQB_secret_cookie_value; SAPISID=secret_sapisid_123; other=public";
        let sanitized = sanitize_for_log(input);
        assert!(!sanitized.contains("AQB_secret_cookie_value"));
        assert!(!sanitized.contains("secret_sapisid_123"));
        // Entire headers can contain unknown cookie names, so redact the whole value.
        assert!(sanitized.contains("Cookie: [REDACTED]"));
        let bare = sanitize_for_log(
            "sp_dc=AQB_secret_cookie_value; SAPISID=secret_sapisid_123; other=public",
        );
        assert!(bare.contains("sp_dc=[REDACTED]"));
        assert!(bare.contains("SAPISID=[REDACTED]"));
        assert!(bare.contains("other=public"));

        let bearer = "Authorization: Bearer my_secret_token_12345";
        let sanitized_bearer = sanitize_for_log(bearer);
        assert!(!sanitized_bearer.contains("my_secret_token_12345"));
        assert!(sanitized_bearer.contains("Authorization: [REDACTED]"));

        let dpapi_str = "Credentials: fallback:dpapi:01000000d08c9ddf0115d1118c7a00c04fc297eb";
        let sanitized_dpapi = sanitize_for_log(dpapi_str);
        assert!(sanitized_dpapi.contains("[REDACTED_SECRET]"));
        assert!(!sanitized_dpapi.contains("01000000d08c9ddf0115d1118c7a00c04fc297eb"));
    }

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

    #[cfg(target_os = "windows")]
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

        // Check that auth_data in SQLite was migrated to fallback:dpapi (on Windows) or fallback:v2 (on non-Windows)
        #[cfg(target_os = "windows")]
        let updated_auth_data: String = conn
            .query_row(
                "SELECT auth_data FROM service_sessions WHERE service_id = 'test_legacy_migration_service'",
                [],
                |row| row.get(0),
            )
            .expect("query updated auth_data");

        #[cfg(target_os = "windows")]
        assert!(updated_auth_data.starts_with("fallback:dpapi:"));

        #[cfg(not(target_os = "windows"))]
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM service_sessions WHERE service_id = 'test_legacy_migration_service'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);

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

    #[cfg(target_os = "windows")]
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

    #[cfg(target_os = "windows")]
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

    #[cfg(target_os = "windows")]
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
