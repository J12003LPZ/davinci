use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

use crate::events::AssistantMessage;
use crate::events::StopReason;

fn non_retryable() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("(?i)(GoUsageLimitError|FreeUsageLimitError|Monthly usage limit reached|available balance|insufficient_quota|out of budget|quota exceeded|billing)").unwrap()
    })
}

fn retryable() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("(?i)(overloaded|rate.?limit|too many requests|429|500|502|503|504|524|service.?unavailable|server.?error|internal.?error|provider.?returned.?error|network.?error|connection.?error|timed? out|timeout)").unwrap()
    })
}

pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    if message.stop_reason != Some(StopReason::Error) {
        return false;
    }
    let Some(err) = &message.error_message else {
        return false;
    };
    if non_retryable().is_match(err) {
        return false;
    }
    retryable().is_match(err)
}

pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 524)
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
        Duration::from_millis(self.base_delay_ms.saturating_mul(factor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_retryable_errors() {
        let message = AssistantMessage {
            id: "1".into(),
            role: "assistant".into(),
            content: vec![],
            model: None,
            stop_reason: Some(StopReason::Error),
            usage: None,
            error_message: Some("429 rate limit".into()),
            timestamp: 1,
        };
        assert!(is_retryable_assistant_error(&message));
        let billing = AssistantMessage {
            error_message: Some("insufficient_quota".into()),
            ..message.clone()
        };
        assert!(!is_retryable_assistant_error(&billing));
        assert!(is_retryable_status(503));
        assert_eq!(
            RetryPolicy::default().delay_for_attempt(3),
            Duration::from_millis(4000)
        );
    }
}
