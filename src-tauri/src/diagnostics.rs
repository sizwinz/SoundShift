//! Shared redaction boundary for emitted, printed, and persisted diagnostics.
use regex::Regex;
use std::sync::OnceLock;

pub fn sanitize_for_log(input: &str) -> String {
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    let rules = RULES.get_or_init(|| [
        r"(?im)(\b(?:authorization|proxy-authorization|cookie|set-cookie)\s*:\s*)[^\r\n]*",
        r#"(?i)((?:["']?)(?:sp_dc|sapisid|__secure-3papisid|__secure-1papisid|sid|hsid|ssid|apisid|access_token|refresh_token|id_token|client_secret|password|token)(?:["']?)\s*[:=]\s*)(?:"(?:\\.|[^"\\])*"|'[^']*'|[^\s;,}&]+)"#,
        r"(?i)(\bbearer\s+)[A-Za-z0-9_./+=-]+",
        r"(?i)fallback:(?:v[12]|dpapi):[A-Za-z0-9_:/+=-]+",
    ].iter().map(|pattern| Regex::new(pattern).expect("constant redaction pattern")).collect());
    let mut result = input.to_string();
    for (index, rule) in rules.iter().enumerate() {
        result = rule
            .replace_all(
                &result,
                if index == 3 {
                    "[REDACTED_SECRET]"
                } else {
                    "${1}[REDACTED]"
                },
            )
            .into_owned();
    }
    result
}

pub fn write_error(message: &str) {
    eprintln!("{}", sanitize_for_log(message));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_headers_json_cookies_short_bearers_and_multiline_values() {
        let message = "Cookie: sp_dc=cookie_secret;other=public\r\nAuthorization: Bearer tiny\n{\"access_token\":\"json_secret\", 'password': 'password_secret'}\nSAPISID = bare_secret; Bearer abc+=/\nfallback:v2:abc:def";
        let safe = sanitize_for_log(message);
        for secret in [
            "cookie_secret",
            "tiny",
            "json_secret",
            "password_secret",
            "bare_secret",
            "abc+=/",
            "abc:def",
        ] {
            assert!(!safe.contains(secret), "unredacted fixture: {secret}");
        }
        assert!(safe.contains("[REDACTED]"));
    }
}
