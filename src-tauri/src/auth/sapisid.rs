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

    format!("SAPISIDHASH {}_{}", timestamp, hash)
}

fn sapisid_digest_scheme(scheme: &str, sapisid: &str, origin: &str, timestamp: u64) -> String {
    let payload = format!("{} {} {}", timestamp, sapisid, origin);
    let mut hasher = Sha1::new();
    hasher.update(payload.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("{} {}_{}", scheme, timestamp, hash)
}

fn extract_cookie_value<'a>(cookie_str: &'a str, name: &str) -> Option<&'a str> {
    cookie_str.split(';').find_map(|part| {
        let mut kv = part.trim().splitn(2, '=');
        let k = kv.next()?.trim();
        let v = kv.next()?.trim();
        if k == name && !v.is_empty() {
            Some(v)
        } else {
            None
        }
    })
}

/// Builds the full InnerTube `Authorization` header (`SAPISIDHASH` plus `SAPISID1PHASH` /
/// `SAPISID3PHASH` when `__Secure-1PAPISID` or `__Secure-3PAPISID` cookies are present).
pub fn generate_youtube_auth_header(
    cookie_str: &str,
    fallback_sapisid: &str,
    origin: &str,
) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    generate_youtube_auth_header_with_timestamp(cookie_str, fallback_sapisid, origin, timestamp)
}

pub fn generate_youtube_auth_header_with_timestamp(
    cookie_str: &str,
    fallback_sapisid: &str,
    origin: &str,
    timestamp: u64,
) -> String {
    let primary = extract_cookie_value(cookie_str, "SAPISID")
        .or_else(|| extract_cookie_value(cookie_str, "__Secure-3PAPISID"))
        .or_else(|| extract_cookie_value(cookie_str, "__Secure-1PAPISID"))
        .unwrap_or(fallback_sapisid);

    let mut tokens = vec![sapisid_digest_scheme(
        "SAPISIDHASH",
        primary,
        origin,
        timestamp,
    )];
    if let Some(p1) = extract_cookie_value(cookie_str, "__Secure-1PAPISID") {
        tokens.push(sapisid_digest_scheme(
            "SAPISID1PHASH",
            p1,
            origin,
            timestamp,
        ));
    }
    if let Some(p3) = extract_cookie_value(cookie_str, "__Secure-3PAPISID") {
        tokens.push(sapisid_digest_scheme(
            "SAPISID3PHASH",
            p3,
            origin,
            timestamp,
        ));
    }
    tokens.join(" ")
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

/// Extracts the YouTube Music session cookies (full cookie string containing SAPISID, SID/HSID/SSID).
pub fn parse_ytmusic_cookie(cookies: &[RawCookie]) -> Option<String> {
    // 1. Must contain SAPISID or __Secure-3PAPISID to calculate SAPISIDHASH
    let has_sapisid = cookies.iter().any(|c| {
        c.name == "SAPISID" || c.name == "__Secure-3PAPISID" || c.name == "__Secure-1PAPISID"
    });
    if !has_sapisid {
        return None;
    }

    // 2. Must contain an authenticated Google session cookie (SID, HSID, SSID, or __Secure-3PSID)
    let is_authenticated = cookies.iter().any(|c| {
        c.name == "SID"
            || c.name == "HSID"
            || c.name == "SSID"
            || c.name == "__Secure-3PSID"
            || c.name == "__Secure-1PSID"
    });
    if !is_authenticated {
        return None;
    }

    // 3. Serialize all non-empty cookies into a deduplicated Cookie header string
    let mut seen = std::collections::HashSet::new();
    let mut parts = Vec::new();
    for c in cookies {
        if !c.value.is_empty() && seen.insert(c.name.clone()) {
            parts.push(format!("{}={}", c.name, c.value));
        }
    }

    Some(parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sapisid_hash_format() {
        let hash_header = generate_sapisid_hash("test_sapisid_value", "https://music.youtube.com");
        assert!(hash_header.starts_with("SAPISIDHASH "));

        let parts: Vec<&str> = hash_header
            .trim_start_matches("SAPISIDHASH ")
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
            "SAPISIDHASH 1700000000_76e48436c0d5c167b88646bf016ac64ccd2a0f07"
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
        assert_eq!(
            token,
            Some("HSID=hsid123; SAPISID=sapisid_secret_456".into())
        );

        // Reject if SAPISID is present but no session cookie
        let unauthenticated = vec![RawCookie {
            name: "SAPISID".into(),
            value: "sapisid_only".into(),
        }];
        assert_eq!(parse_ytmusic_cookie(&unauthenticated), None);

        // Reject if session cookie is present but no SAPISID
        let no_sapisid = vec![RawCookie {
            name: "SID".into(),
            value: "sid_only".into(),
        }];
        assert_eq!(parse_ytmusic_cookie(&no_sapisid), None);
    }

    #[test]
    fn test_youtube_auth_header_includes_multi_cookie_hashes() {
        let header = generate_youtube_auth_header_with_timestamp(
            "SAPISID=base; __Secure-1PAPISID=p1; __Secure-3PAPISID=p3",
            "fallback",
            "https://music.youtube.com",
            1700000000,
        );
        assert!(header.contains("SAPISIDHASH 1700000000_"));
        assert!(header.contains("SAPISID1PHASH 1700000000_"));
        assert!(header.contains("SAPISID3PHASH 1700000000_"));
    }
}
