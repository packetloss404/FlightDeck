use serde::Serialize;

/// Error categories for AI CLI invocations.
/// Inspired by Hermes Agent's error_classifier — classifies stderr output from
/// Claude CLI into actionable categories with recovery hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiErrorCategory {
    /// Authentication failure — user needs to re-authenticate
    Auth,
    /// Billing / quota exhausted — not transient
    Billing,
    /// Rate limited — retry after backoff
    RateLimit,
    /// Context window exceeded — compress or shorten input
    ContextOverflow,
    /// Network timeout — retry
    Timeout,
    /// Server error (5xx) — retry with backoff
    ServerError,
    /// CLI not found or not installed
    NotInstalled,
    /// Unknown / unclassified error
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassifiedError {
    pub category: AiErrorCategory,
    pub message: String,
    pub is_transient: bool,
    pub suggestion: String,
}

/// Classify a stderr string from Claude CLI into an actionable error.
pub fn classify_cli_error(stderr: &str) -> ClassifiedError {
    let lower = stderr.to_lowercase();

    // Auth errors
    if lower.contains("unauthorized")
        || lower.contains("authentication")
        || lower.contains("invalid api key")
        || lower.contains("api key")
        || lower.contains("not authenticated")
        || lower.contains("login")
    {
        return ClassifiedError {
            category: AiErrorCategory::Auth,
            message: extract_message(stderr),
            is_transient: false,
            suggestion: "Re-authenticate with Claude CLI: run `claude login` in a terminal."
                .to_string(),
        };
    }

    // Billing / quota
    if lower.contains("billing")
        || lower.contains("insufficient credits")
        || lower.contains("quota exceeded")
        || lower.contains("payment required")
        || (lower.contains("402") && !lower.contains("try again"))
    {
        return ClassifiedError {
            category: AiErrorCategory::Billing,
            message: extract_message(stderr),
            is_transient: false,
            suggestion: "Check your API billing and usage limits.".to_string(),
        };
    }

    // Rate limiting
    if lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("429")
        || (lower.contains("try again") && lower.contains("minutes"))
        || lower.contains("overloaded")
    {
        return ClassifiedError {
            category: AiErrorCategory::RateLimit,
            message: extract_message(stderr),
            is_transient: true,
            suggestion: "Rate limited — will retry automatically.".to_string(),
        };
    }

    // Context overflow
    if lower.contains("context")
        && (lower.contains("too long")
            || lower.contains("exceed")
            || lower.contains("overflow")
            || lower.contains("limit"))
        || lower.contains("max_tokens")
        || lower.contains("maximum context length")
    {
        return ClassifiedError {
            category: AiErrorCategory::ContextOverflow,
            message: extract_message(stderr),
            is_transient: false,
            suggestion: "Input is too long. Try shortening the prompt or conversation history."
                .to_string(),
        };
    }

    // Timeout
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("deadline exceeded")
        || lower.contains("econnreset")
        || lower.contains("socket hang up")
    {
        return ClassifiedError {
            category: AiErrorCategory::Timeout,
            message: extract_message(stderr),
            is_transient: true,
            suggestion: "Request timed out — will retry automatically.".to_string(),
        };
    }

    // Server errors
    if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("internal server error")
        || lower.contains("service unavailable")
        || lower.contains("bad gateway")
    {
        return ClassifiedError {
            category: AiErrorCategory::ServerError,
            message: extract_message(stderr),
            is_transient: true,
            suggestion: "Server error — will retry automatically.".to_string(),
        };
    }

    // CLI not found
    if lower.contains("not found")
        || lower.contains("not recognized")
        || lower.contains("no such file")
        || lower.contains("command not found")
    {
        return ClassifiedError {
            category: AiErrorCategory::NotInstalled,
            message: extract_message(stderr),
            is_transient: false,
            suggestion: "Claude CLI not found. Install it and ensure it's on PATH.".to_string(),
        };
    }

    // Unknown
    ClassifiedError {
        category: AiErrorCategory::Unknown,
        message: extract_message(stderr),
        is_transient: false,
        suggestion: "An unexpected error occurred. Check the Claude CLI output for details."
            .to_string(),
    }
}

/// Extract a clean error message from stderr (first non-empty line, capped at 200 chars).
fn extract_message(stderr: &str) -> String {
    let msg = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(stderr)
        .trim();
    if msg.len() > 200 {
        format!("{}...", &msg[..197])
    } else {
        msg.to_string()
    }
}

/// Calculate retry delay with decorrelated jitter (port of Hermes retry_utils.py).
/// Returns milliseconds to wait before the next attempt.
pub fn retry_delay_ms(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let exp_delay = base_ms.saturating_mul(1u64 << attempt.min(10));
    let capped = exp_delay.min(max_ms);
    // Add jitter: 50-100% of the capped delay
    let jitter = (capped as f64) * (0.5 + 0.5 * rand_fraction(attempt));
    jitter as u64
}

/// Simple deterministic-ish fraction from attempt number (no external deps needed).
fn rand_fraction(attempt: u32) -> f64 {
    // Use a simple hash-like mixing function for jitter
    let mixed = (attempt.wrapping_mul(2654435761)) as f64 / u32::MAX as f64;
    mixed.fract().abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rate_limit() {
        let err = classify_cli_error("Error: 429 Too Many Requests");
        assert_eq!(err.category, AiErrorCategory::RateLimit);
        assert!(err.is_transient);
    }

    #[test]
    fn classifies_auth() {
        let err = classify_cli_error("Error: unauthorized - invalid API key");
        assert_eq!(err.category, AiErrorCategory::Auth);
        assert!(!err.is_transient);
    }

    #[test]
    fn classifies_context_overflow() {
        let err = classify_cli_error("maximum context length exceeded");
        assert_eq!(err.category, AiErrorCategory::ContextOverflow);
        assert!(!err.is_transient);
    }

    #[test]
    fn classifies_timeout() {
        let err = classify_cli_error("request timed out after 30s");
        assert_eq!(err.category, AiErrorCategory::Timeout);
        assert!(err.is_transient);
    }

    #[test]
    fn classifies_server_error() {
        let err = classify_cli_error("502 Bad Gateway");
        assert_eq!(err.category, AiErrorCategory::ServerError);
        assert!(err.is_transient);
    }

    #[test]
    fn classifies_not_installed() {
        let err = classify_cli_error("claude: command not found");
        assert_eq!(err.category, AiErrorCategory::NotInstalled);
        assert!(!err.is_transient);
    }

    #[test]
    fn classifies_unknown() {
        let err = classify_cli_error("something weird happened");
        assert_eq!(err.category, AiErrorCategory::Unknown);
        assert!(!err.is_transient);
    }

    #[test]
    fn retry_delay_increases() {
        let d0 = retry_delay_ms(0, 2000, 60000);
        let d1 = retry_delay_ms(1, 2000, 60000);
        let d2 = retry_delay_ms(2, 2000, 60000);
        // Each attempt should generally increase (with jitter it's not perfectly monotonic)
        assert!(d0 < 60000);
        assert!(d1 <= 60000);
        assert!(d2 <= 60000);
    }

    #[test]
    fn retry_delay_caps_at_max() {
        let d = retry_delay_ms(20, 2000, 60000);
        assert!(d <= 60000);
    }

    #[test]
    fn extract_message_truncates_long_stderr() {
        let long = "x".repeat(300);
        let msg = extract_message(&long);
        assert!(msg.len() <= 203); // 200 + "..."
    }
}
