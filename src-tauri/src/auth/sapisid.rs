use sha1::{Digest, Sha1};
use std::time::{SystemTime, UNIX_EPOCH};

/// Generates the SAPISIDHASH Authorization header required by Google InnerTube endpoints.
/// Format: `SAPISIDHASH {timestamp}_{sha1_hash}_u`
/// where sha1_hash is SHA-1 of `{timestamp} {sapisid} {origin}`
pub fn generate_sapisid_hash(sapisid: &str, origin: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    generate_sapisid_hash_with_timestamp(sapisid, origin, timestamp)
}

/// Helper for deterministic testing with an explicit timestamp.
pub fn generate_sapisid_hash_with_timestamp(sapisid: &str, origin: &str, timestamp: u64) -> String {
    let payload = format!("{} {} {}", timestamp, sapisid, origin);
    let mut hasher = Sha1::new();
    hasher.update(payload.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    format!("SAPISIDHASH {}_{}_u", timestamp, hash)
}

/// Simple cookie representation for extractor functions.
#[derive(Debug, Clone)]
pub struct RawCookie {
    pub name: String,
    pub value: String,
}

/// Extracts the Spotify session cookie `sp_dc` from a list of cookies.
pub fn parse_spotify_cookie(cookies: &[RawCookie]) -> Option<String> {
    cookies
        .iter()
        .find(|c| c.name == "sp_dc")
        .map(|c| c.value.clone())
}

/// Extracts the YouTube Music session cookie `SAPISID` or `__Secure-3PAPISID`.
pub fn parse_ytmusic_cookie(cookies: &[RawCookie]) -> Option<String> {
    cookies
        .iter()
        .find(|c| c.name == "SAPISID" || c.name == "__Secure-3PAPISID")
        .map(|c| c.value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sapisid_hash_format() {
        let hash_header = generate_sapisid_hash("test_sapisid_value", "https://music.youtube.com");
        assert!(hash_header.starts_with("SAPISIDHASH "));
        assert!(hash_header.ends_with("_u"));

        let parts: Vec<&str> = hash_header
            .trim_start_matches("SAPISIDHASH ")
            .trim_end_matches("_u")
            .split('_')
            .collect();

        assert_eq!(parts.len(), 2);
        // First part is timestamp digits
        assert!(parts[0].parse::<u64>().is_ok());
        // Second part is 40-character hexadecimal SHA-1 string
        assert_eq!(parts[1].len(), 40);
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sapisid_hash_deterministic() {
        let sapisid = "sample_cookie_secret_123";
        let origin = "https://music.youtube.com";
        let timestamp = 1700000000;

        let result1 = generate_sapisid_hash_with_timestamp(sapisid, origin, timestamp);
        let result2 = generate_sapisid_hash_with_timestamp(sapisid, origin, timestamp);

        assert_eq!(result1, result2);
        assert_eq!(
            result1,
            "SAPISIDHASH 1700000000_76e48436c0d5c167b88646bf016ac64ccd2a0f07_u"
        );
    }

    #[test]
    fn test_parse_spotify_cookie() {
        let cookies = vec![
            RawCookie {
                name: "remember".into(),
                value: "false".into(),
            },
            RawCookie {
                name: "sp_dc".into(),
                value: "AQBxyz123testsessiontoken".into(),
            },
        ];

        let token = parse_spotify_cookie(&cookies);
        assert_eq!(token, Some("AQBxyz123testsessiontoken".into()));

        let empty_cookies = vec![RawCookie {
            name: "other".into(),
            value: "123".into(),
        }];
        assert_eq!(parse_spotify_cookie(&empty_cookies), None);
    }

    #[test]
    fn test_parse_ytmusic_cookie() {
        let cookies = vec![
            RawCookie {
                name: "HSID".into(),
                value: "hsid123".into(),
            },
            RawCookie {
                name: "SAPISID".into(),
                value: "sapisid_secret_456".into(),
            },
        ];

        let token = parse_ytmusic_cookie(&cookies);
        assert_eq!(token, Some("sapisid_secret_456".into()));
    }
}
