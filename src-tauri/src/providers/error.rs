use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendCertainty {
    NotSent,
    Rejected,
    MayHaveApplied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub certainty: SendCertainty,
    pub detail: String,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, certainty: SendCertainty, detail: &str) -> Self {
        Self {
            kind,
            http_status: None,
            retry_after_ms: None,
            certainty,
            detail: crate::diagnostics::sanitize_for_log(detail),
        }
    }
    pub fn http(status: u16, headers: &reqwest::header::HeaderMap) -> Self {
        let kind = match status {
            401 => ProviderErrorKind::Authentication,
            403 => ProviderErrorKind::Permission,
            429 => ProviderErrorKind::RateLimited,
            409 | 412 => ProviderErrorKind::Conflict,
            500..=599 => ProviderErrorKind::Transient,
            _ => ProviderErrorKind::Unknown,
        };
        let retry_after_ms = headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|h| h.to_str().ok())
            .and_then(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .map(|s| s.saturating_mul(1000))
                    .or_else(|| {
                        chrono::DateTime::parse_from_rfc2822(value)
                            .ok()
                            .map(|date| {
                                (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                                    .num_milliseconds()
                                    .max(0) as u64
                            })
                    })
            });
        Self {
            kind,
            http_status: Some(status),
            retry_after_ms,
            certainty: if matches!(status, 401 | 403 | 429) {
                SendCertainty::Rejected
            } else {
                SendCertainty::MayHaveApplied
            },
            detail: format!(
                "Provider {:?} (HTTP {}); review the recorded operation before retry",
                kind, status
            ),
        }
    }
    pub fn read_http(status: u16, headers: &reqwest::header::HeaderMap) -> Self {
        let mut error = Self::http(status, headers);
        error.certainty = SendCertainty::NotSent;
        error
    }
    pub fn read_transport(error: reqwest::Error) -> Self {
        Self::new(
            if error.is_timeout() {
                ProviderErrorKind::Timeout
            } else {
                ProviderErrorKind::Transient
            },
            SendCertainty::NotSent,
            "Provider read connection failed; no mutation sent",
        )
    }
    pub fn transport(error: reqwest::Error) -> Self {
        Self::new(
            if error.is_timeout() {
                ProviderErrorKind::Timeout
            } else {
                ProviderErrorKind::Transient
            },
            SendCertainty::MayHaveApplied,
            "Provider connection failed; remote mutation outcome may be uncertain",
        )
    }
    /// Compatibility boundary for legacy read/auth APIs, never used to infer mutation certainty.
    pub fn before_send(error: String) -> Self {
        Self::new(classify_error(&error), SendCertainty::NotSent, &error)
    }
}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for ProviderError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderErrorKind {
    Authentication,
    Permission,
    RateLimited,
    Transient,
    Timeout,
    Conflict,
    MalformedResponse,
    Pagination,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationResult {
    Confirmed,
    NotApplied,
    Uncertain,
}

pub fn classify_error(message: &str) -> ProviderErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("token") && (lower.contains("expired") || lower.contains("invalid"))
    {
        ProviderErrorKind::Authentication
    } else if lower.contains("403") || lower.contains("forbidden") || lower.contains("permission") {
        ProviderErrorKind::Permission
    } else if lower.contains("429") || lower.contains("rate limit") || lower.contains("retry-after")
    {
        ProviderErrorKind::RateLimited
    } else if lower.contains("409") || lower.contains("conflict") {
        ProviderErrorKind::Conflict
    } else if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("reset by peer")
    {
        ProviderErrorKind::Timeout
    } else if lower.contains("cancel") {
        ProviderErrorKind::Cancelled
    } else if lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("temporar")
        || lower.contains("unavailable")
    {
        ProviderErrorKind::Transient
    } else if lower.contains("json")
        || lower.contains("parse")
        || lower.contains("schema")
        || lower.contains("malformed")
    {
        ProviderErrorKind::MalformedResponse
    } else if lower.contains("pagination")
        || lower.contains("continuation")
        || lower.contains("page")
    {
        ProviderErrorKind::Pagination
    } else {
        ProviderErrorKind::Unknown
    }
}

pub fn is_retryable(kind: ProviderErrorKind) -> bool {
    matches!(
        kind,
        ProviderErrorKind::RateLimited
            | ProviderErrorKind::Transient
            | ProviderErrorKind::Timeout
            | ProviderErrorKind::Conflict
    )
}

pub fn retry_delay_ms(attempt: u32, retry_after_ms: Option<u64>) -> u64 {
    if let Some(delay) = retry_after_ms {
        return delay;
    }

    let exponent = attempt.min(5);
    let backoff = 1_000u64.saturating_mul(1u64 << exponent);
    let jitter = (chrono::Utc::now().timestamp_subsec_millis() as u64) % 400;
    (backoff + jitter).min(30_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_error_retains_retry_guidance_and_send_certainty() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "60".parse().unwrap());
        let error = ProviderError::http(429, &headers);
        assert_eq!(error.kind, ProviderErrorKind::RateLimited);
        assert_eq!(error.http_status, Some(429));
        assert_eq!(error.retry_after_ms, Some(60_000));
        assert_eq!(error.certainty, SendCertainty::Rejected);
        assert!(error.to_string().contains("429"));
    }

    #[test]
    fn provider_retry_guidance_is_never_shortened() {
        assert_eq!(retry_delay_ms(0, Some(60_000)), 60_000);
    }

    #[test]
    fn classifies_ambiguous_transport_failures() {
        assert_eq!(
            classify_error("request failed: connection reset by peer"),
            ProviderErrorKind::Timeout
        );
    }

    #[test]
    fn honors_retry_after_without_shortening_guidance() {
        assert_eq!(retry_delay_ms(0, Some(60_000)), 60_000);
    }

    #[test]
    fn classifies_conflict_and_is_retryable() {
        let kind = classify_error("Add tracks failed with status: 409 Conflict");
        assert_eq!(kind, ProviderErrorKind::Conflict);
        assert!(is_retryable(kind));
    }
}
