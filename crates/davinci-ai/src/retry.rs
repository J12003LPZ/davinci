//! Assistant-error retry classification matching `vendor/pi/packages/ai/src/utils/retry.ts`.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::stream::{AssistantMessage, StopReason};

const RESPONSE_STREAM_FAILURE_KEY: &str = "__davinci_response_stream_failure";

/// Host decoder/reader evidence, not provider text or operation retry authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStreamFailure {
    MissingTerminal,
    TransportInterrupted,
    InvalidStream,
    UnfinishedTerminalToolCall,
    ProviderTerminalError,
    ProviderTerminalRejection,
    Unknown,
}

pub(crate) fn set_response_stream_failure(
    message: &mut AssistantMessage,
    failure: ResponseStreamFailure,
) {
    message.extra.insert(
        RESPONSE_STREAM_FAILURE_KEY.into(),
        serde_json::to_value(failure).expect("stream failure enum"),
    );
}

pub fn response_stream_failure(message: &AssistantMessage) -> Option<ResponseStreamFailure> {
    message.extra.get(RESPONSE_STREAM_FAILURE_KEY).map(|value| {
        serde_json::from_value(value.clone()).unwrap_or(ResponseStreamFailure::Unknown)
    })
}

fn read_failure(kind: std::io::ErrorKind) -> ResponseStreamFailure {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof
        | ErrorKind::TimedOut => ResponseStreamFailure::TransportInterrupted,
        ErrorKind::InvalidData => ResponseStreamFailure::InvalidStream,
        _ => ResponseStreamFailure::Unknown,
    }
}

pub(crate) fn classify_stream_read_error(error: &std::io::Error) -> ResponseStreamFailure {
    // ureq 3 keeps its timeout evidence inside an ErrorKind::Other wrapper.
    // Do not infer interruption from arbitrary Other error strings.
    if error.kind() == std::io::ErrorKind::Other
        && matches!(
            error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ureq_stream::Error>()),
            Some(ureq_stream::Error::Timeout(_))
        )
    {
        return ResponseStreamFailure::TransportInterrupted;
    }
    read_failure(error.kind())
}

pub(crate) fn classify_terminal_response_error(
    response: &serde_json::Value,
) -> ResponseStreamFailure {
    match response.get("status") {
        Some(serde_json::Value::String(status)) if status == "cancelled" => {
            return ResponseStreamFailure::ProviderTerminalRejection;
        }
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(status))
            if matches!(status.as_str(), "completed" | "failed" | "incomplete") => {}
        _ => return ResponseStreamFailure::Unknown,
    }
    classify_provider_terminal_error(response)
}

pub(crate) fn classify_provider_terminal_error(event: &serde_json::Value) -> ResponseStreamFailure {
    let rejected = event
        .pointer("/incomplete_details/reason")
        .and_then(serde_json::Value::as_str)
        == Some("content_filter")
        || std::iter::once(event)
            .chain(event.get("error"))
            .any(|error| {
                ["code", "type"].iter().any(|key| {
                    error
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|code| {
                            let code = code.to_ascii_lowercase();
                            non_retryable_limit_pattern().is_match(&code)
                                || code.starts_with("invalid_")
                                || code.ends_with("_policy_violation")
                                || matches!(
                                    code.as_str(),
                                    "authentication_error" | "permission_denied" | "content_filter"
                                )
                        })
                })
            });
    if rejected {
        ResponseStreamFailure::ProviderTerminalRejection
    } else {
        ResponseStreamFailure::ProviderTerminalError
    }
}

pub(crate) fn declared_response_veto(
    response: &serde_json::Value,
) -> Option<ResponseStreamFailure> {
    match response.get("status") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(status)) if status == "cancelled" => {
            return Some(ResponseStreamFailure::ProviderTerminalRejection);
        }
        Some(serde_json::Value::String(status))
            if matches!(
                status.as_str(),
                "queued" | "in_progress" | "completed" | "failed" | "incomplete"
            ) => {}
        _ => return Some(ResponseStreamFailure::Unknown),
    }
    (classify_provider_terminal_error(response) == ResponseStreamFailure::ProviderTerminalRejection)
        .then_some(ResponseStreamFailure::ProviderTerminalRejection)
}

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
            "usage_limit_reached",
            "subscription_sharing_usage_limit_exceeded",
            "subscription_sharing_usage_unavailable",
            "subscription_sharing_unsupported_capability",
            "subscription_sharing_route_not_supported",
            "chatpass_v2_scope_not_authorized",
            "chatpass_v2_invalid_authorization_context",
            "subscription-only",
            "out of budget",
            "quota exceeded",
            "billing",
            // Retrying a request that exceeds the model's context cannot help.
            "prompt is too long",
            "context.?length.?exceeded",
            "maximum context length",
            "exceeds the context window",
            "input is too long",
            "too many (input )?tokens",
            "request too large",
        ])
    })
}

fn context_overflow_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        build_pattern(&[
            "prompt is too long",
            "context.?length.?exceeded",
            "maximum context length",
            "exceeds the context window",
            "input is too long",
            "too many (input )?tokens",
        ])
    })
}

/// True when a provider refused a request because its input does not fit the
/// model's context window. A retry of the same input cannot succeed; a
/// smaller input can.
pub fn is_context_overflow_text(text: &str) -> bool {
    context_overflow_pattern().is_match(text)
}

fn retryable_provider_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        build_pattern(&[
            "overloaded",
            "rate.?limit",
            "too many requests",
            r"\b(?:429|5\d\d)\b",
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
            // The same cut, with a tool call still open. Nothing from a cut
            // response is executed, so a retry cannot repeat a side effect.
            "stream ended before the tool call was complete",
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
    if let Some(failure) = response_stream_failure(message) {
        let retryable = match failure {
            ResponseStreamFailure::MissingTerminal
            | ResponseStreamFailure::TransportInterrupted => true,
            // A terminal event ends this attempt, not necessarily the recovery
            // budget. Retain the existing transient-provider policy.
            ResponseStreamFailure::ProviderTerminalError => {
                retryable_provider_pattern().is_match(error_message)
            }
            _ => false,
        };
        return retryable
            && !message
                .content
                .iter()
                .any(|block| matches!(block, crate::ContentBlock::ToolCall { .. }));
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

    #[test]
    fn context_overflow_is_recognized_and_rate_limits_are_not() {
        for text in [
            "prompt is too long: 205000 tokens > 200000 maximum",
            "This model's maximum context length is 128000 tokens",
            "context_length_exceeded",
            "input exceeds the context window",
        ] {
            assert!(is_context_overflow_text(text), "{text}");
        }
        for text in ["HTTP 429 rate limit", "usage_limit_reached", "server error"] {
            assert!(!is_context_overflow_text(text), "{text}");
        }
    }

    #[test]
    fn typed_stream_evidence_is_fail_closed_and_never_operation_authority() {
        for (failure, retryable) in [
            (ResponseStreamFailure::MissingTerminal, true),
            (ResponseStreamFailure::TransportInterrupted, true),
            (ResponseStreamFailure::InvalidStream, false),
            (ResponseStreamFailure::UnfinishedTerminalToolCall, false),
            (ResponseStreamFailure::ProviderTerminalError, true),
            (ResponseStreamFailure::ProviderTerminalRejection, false),
            (ResponseStreamFailure::Unknown, false),
        ] {
            let mut message = error_message("server error 503: stream terminated");
            set_response_stream_failure(&mut message, failure);
            assert_eq!(response_stream_failure(&message), Some(failure));
            assert_eq!(is_retryable_assistant_error(&message), retryable);
            message.stop_reason = Some(StopReason::Aborted);
            assert!(!is_retryable_assistant_error(&message));
            message.stop_reason = Some(StopReason::Error);
            message.error_message = Some("HTTP 429 usage_limit_reached".into());
            assert!(!is_retryable_assistant_error(&message));
        }
        let mut unknown = error_message("server error 503");
        unknown.extra.insert(
            RESPONSE_STREAM_FAILURE_KEY.into(),
            serde_json::json!("future_or_invalid_kind"),
        );
        assert!(!is_retryable_assistant_error(&unknown));

        let mut executable = error_message("stream terminated");
        set_response_stream_failure(&mut executable, ResponseStreamFailure::MissingTerminal);
        executable.content.push(crate::ContentBlock::ToolCall {
            id: "new_id".into(),
            name: "write".into(),
            arguments: serde_json::json!({}),
        });
        assert!(!is_retryable_assistant_error(&executable));
    }

    #[test]
    fn typed_reader_causes_separate_interruption_invalid_data_and_unknown() {
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::TimedOut,
        ] {
            assert_eq!(
                read_failure(kind),
                ResponseStreamFailure::TransportInterrupted
            );
        }
        assert_eq!(
            read_failure(std::io::ErrorKind::InvalidData),
            ResponseStreamFailure::InvalidStream
        );
        assert_eq!(
            read_failure(std::io::ErrorKind::Other),
            ResponseStreamFailure::Unknown
        );
        assert_eq!(
            read_failure(std::io::ErrorKind::PermissionDenied),
            ResponseStreamFailure::Unknown
        );
    }

    #[test]
    fn wrapped_ureq_idle_timeout_is_an_interruption_not_arbitrary_other() {
        let timeout = ureq_stream::Error::Timeout(ureq_stream::Timeout::RecvBody).into_io();
        assert_eq!(timeout.kind(), std::io::ErrorKind::Other);
        assert_eq!(
            classify_stream_read_error(&timeout),
            ResponseStreamFailure::TransportInterrupted
        );
        let other = std::io::Error::other("timeout 503, but no typed transport evidence");
        assert_eq!(
            classify_stream_read_error(&other),
            ResponseStreamFailure::Unknown
        );
        for (kind, expected) in [
            (
                std::io::ErrorKind::InvalidData,
                ResponseStreamFailure::InvalidStream,
            ),
            (
                std::io::ErrorKind::PermissionDenied,
                ResponseStreamFailure::Unknown,
            ),
        ] {
            let contradictory = std::io::Error::new(
                kind,
                ureq_stream::Error::Timeout(ureq_stream::Timeout::RecvBody),
            );
            assert_eq!(classify_stream_read_error(&contradictory), expected);
        }
    }

    #[test]
    fn terminal_rejection_codes_override_misleading_transient_messages() {
        for event in [
            serde_json::json!({"code": "insufficient_quota", "error": {"message": "HTTP 503; please retry your request"}}),
            serde_json::json!({"message": "HTTP 503", "error": {"type": "permission_denied"}}),
            serde_json::json!({"code": "server_error", "error": {"code": "content_policy_violation"}}),
        ] {
            let mut message = error_message("HTTP 503; please retry your request");
            set_response_stream_failure(&mut message, classify_provider_terminal_error(&event));
            assert!(!is_retryable_assistant_error(&message), "{event}");
        }
        let mut executable = error_message("HTTP 503 server error");
        set_response_stream_failure(
            &mut executable,
            ResponseStreamFailure::ProviderTerminalError,
        );
        executable.content.push(crate::ContentBlock::ToolCall {
            id: "new_id".into(),
            name: "write".into(),
            arguments: serde_json::json!({}),
        });
        assert!(!is_retryable_assistant_error(&executable));
    }

    #[test]
    fn codex_subscription_exhaustion_and_policy_denials_never_retry() {
        for error in [
            "HTTP 429 usage_limit_reached",
            "subscription-only receipt rejected: HTTP 503",
        ] {
            assert!(!is_retryable_error_text(error));
            assert!(!is_retryable_assistant_error(&error_message(error)));
        }
    }

    fn error_message(text: &str) -> AssistantMessage {
        AssistantMessage {
            extra: Default::default(),
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
            extra: Default::default(),
            id: "t".into(),
            role: "assistant".into(),
            content: Vec::new(),
            model: "fixture".into(),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        }));
    }

    #[test]
    fn numbers_inside_other_numbers_are_not_status_codes() {
        assert!(!is_retryable_error_text(
            "prompt is too long: 205000 tokens > 200000 maximum"
        ));
        assert!(!is_retryable_error_text("input has 135000 tokens"));
        assert!(is_retryable_error_text("HTTP 500 Internal Server Error"));
        assert!(is_retryable_error_text("status=429"));
    }

    #[test]
    fn context_overflow_is_never_retryable() {
        for text in [
            "prompt is too long: 205000 tokens > 200000 maximum",
            "This model's maximum context length is 128000 tokens",
            "context_length_exceeded",
            "input exceeds the context window",
            "Request too large: 500 server error while counting; prompt is too long",
        ] {
            assert!(!is_retryable_error_text(text), "{text}");
        }
    }
}
