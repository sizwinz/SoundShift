use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderErrorKind {
    Authentication,
    Permission,
    RateLimited,
    Transient,
    Timeout,
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
        ProviderErrorKind::RateLimited | ProviderErrorKind::Transient | ProviderErrorKind::Timeout
    )
}

pub fn retry_delay_ms(attempt: u32, retry_after_ms: Option<u64>) -> u64 {
    if let Some(delay) = retry_after_ms {
        return delay.min(30_000);
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
    fn classifies_ambiguous_transport_failures() {
        assert_eq!(
            classify_error("request failed: connection reset by peer"),
            ProviderErrorKind::Timeout
        );
    }

    #[test]
    fn honors_retry_after_without_exceeding_cap() {
        assert_eq!(retry_delay_ms(0, Some(60_000)), 30_000);
    }
}
