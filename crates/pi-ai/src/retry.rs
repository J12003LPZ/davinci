use crate::types::AssistantMessage;
use regex::Regex;
use std::sync::LazyLock;
use std::time::Duration;

static NON_RETRYABLE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(GoUsageLimitError|FreeUsageLimitError|Monthly usage limit reached|available balance|insufficient_quota|out of budget|quota exceeded|billing)").unwrap()
});

static RETRYABLE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(overloaded|rate.?limit|too many requests|429|500|502|503|504|524|service.?unavailable|server.?error|internal.?error|provider.?returned.?error|exceeded request buffer limit while retrying upstream|network.?error|connection.?error|connection.?refused|connection.?lost|other side closed|fetch failed|getaddrinfo|ENOTFOUND|EAI_AGAIN|upstream.?connect|reset before headers|socket hang up|socket connection was closed|timed? out|timeout|terminated|websocket.?closed|websocket.?error|ended without|stream ended before message_stop|stream ended before a terminal response event|http2 request did not get a response|retry delay|you can retry your request|try your request again|please retry your request|ResourceExhausted)").unwrap()
});

pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    if message.stop_reason != crate::types::StopReason::Error {
        return false;
    }
    let Some(err) = &message.error_message else {
        return false;
    };

    if NON_RETRYABLE_PATTERN.is_match(err) {
        return false;
    }

    RETRYABLE_PATTERN.is_match(err)
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            base_delay_ms: 1000,
        }
    }
}

impl RetryPolicy {
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = 1u64
            .checked_shl(attempt.saturating_sub(1))
            .unwrap_or(u64::MAX);
        let delay_ms = self.base_delay_ms.saturating_mul(factor);
        Duration::from_millis(delay_ms)
    }
}
