//! Assistant-error retry classification matching `vendor/pi/packages/ai/src/utils/retry.ts`.

use std::sync::OnceLock;

use regex::Regex;

use crate::stream::{AssistantMessage, StopReason};

fn build_pattern(patterns: &[&str]) -> Regex {
    Regex::new(&format!("(?i){}", patterns.join("|"))).expect("retry pattern")
}

fn non_retryable_limit_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        build_pattern(&[
            "GoUsageLimitError",
            "FreeUsageLimitError",
            "Monthly usage limit reached",
            "available balance",
            "insufficient_quota",
            "out of budget",
            "quota exceeded",
            "billing",
        ])
    })
}

fn retryable_provider_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        build_pattern(&[
            "overloaded",
            "rate.?limit",
            "too many requests",
            "429",
            "500",
            "502",
            "503",
            "504",
            "524",
            "service.?unavailable",
            "server.?error",
            "internal.?error",
            "provider.?returned.?error",
            "exceeded request buffer limit while retrying upstream",
            "network.?error",
            "connection.?error",
            "connection.?refused",
            "connection.?lost",
            "other side closed",
            "fetch failed",
            "getaddrinfo",
            "ENOTFOUND",
            "EAI_AGAIN",
            "upstream.?connect",
            "reset before headers",
            "socket hang up",
            "socket connection was closed",
            "timed? out",
            "timeout",
            "terminated",
            "websocket.?closed",
            "websocket.?error",
            "ended without",
            "stream ended before message_stop",
            "stream ended before a terminal response event",
            "http2 request did not get a response",
            "retry delay",
            "you can retry your request",
            "try your request again",
            "please retry your request",
            "ResourceExhausted",
        ])
    })
}

/// TS `isRetryableAssistantError`.
pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    if message.stop_reason != Some(StopReason::Error) {
        return false;
    }
    let Some(error_message) = message.error_message.as_deref() else {
        return false;
    };
    if non_retryable_limit_pattern().is_match(error_message) {
        return false;
    }
    retryable_provider_pattern().is_match(error_message)
}

pub fn is_retryable_error_text(text: &str) -> bool {
    if non_retryable_limit_pattern().is_match(text) {
        return false;
    }
    retryable_provider_pattern().is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::AssistantMessage;

    fn error_message(text: &str) -> AssistantMessage {
        AssistantMessage {
            id: "t".into(),
            role: "assistant".into(),
            content: Vec::new(),
            model: "fixture".into(),
            usage: None,
            stop_reason: Some(StopReason::Error),
            error_message: Some(text.into()),
        }
    }

    #[test]
    fn classifies_ts_retryable_assistant_errors() {
        let open_ai = "An error occurred while processing your request. You can retry your request, or contact us through our help center at help.openai.com if the error persists. Please include the request ID req_******** in your message.";
        let bedrock = r#"{"message":"The system encountered an unexpected error during processing. Try your request again."}"#;
        let nim = "ResourceExhausted: Worker local total request limit reached (288/48)";
        let bun = "The socket connection was closed unexpectedly. For more information, pass `verbose: true` in the second argument to fetch()";
        let eof = "OpenAI Responses stream ended before a terminal response event";
        let dns = "The pending stream has been canceled (caused by: getaddrinfo ENOTFOUND bedrock-runtime.us-east-1.amazonaws.com)";
        assert!(is_retryable_assistant_error(&error_message(open_ai)));
        assert!(is_retryable_assistant_error(&error_message(bedrock)));
        assert!(is_retryable_assistant_error(&error_message(nim)));
        assert!(is_retryable_assistant_error(&error_message(bun)));
        assert!(is_retryable_assistant_error(&error_message(
            "Error: exceeded request buffer limit while retrying upstream"
        )));
        assert!(is_retryable_assistant_error(&error_message(dns)));
        assert!(is_retryable_assistant_error(&error_message(
            "connect ENOTFOUND api.example.com"
        )));
        assert!(is_retryable_assistant_error(&error_message(eof)));
        assert!(is_retryable_assistant_error(&error_message(
            "overloaded_error"
        )));
        assert!(is_retryable_assistant_error(&error_message(
            "524 status code (no body)"
        )));
        assert!(!is_retryable_assistant_error(&error_message(
            "429 quota exceeded"
        )));
        assert!(!is_retryable_assistant_error(&AssistantMessage {
            id: "t".into(),
            role: "assistant".into(),
            content: Vec::new(),
            model: "fixture".into(),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        }));
    }
}
