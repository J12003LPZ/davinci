//! Codex websocket / SSE transport matching
//! `vendor/pi/packages/ai/src/api/openai-codex-responses.ts`.

use serde_json::Value;
use uuid::Uuid;

use crate::catalog::Model;
use crate::stream::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};

pub const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub const WEBSOCKET_CONNECTION_LIMIT_REACHED: &str = "websocket_connection_limit_reached";
pub const PREVIOUS_RESPONSE_NOT_FOUND: &str = "previous_response_not_found";
pub const WEBSOCKET_MESSAGE_TOO_BIG_CLOSE_CODE: u16 = 1009;
pub const WEBSOCKET_CLOSED_BEFORE_COMPLETED: &str =
    "WebSocket stream closed before response.completed";
pub const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS: u64 = 15_000;

/// TS `normalizeTimeoutMs(options?.websocketConnectTimeoutMs)` then default 15000.
/// Settings export `PI_WEBSOCKET_CONNECT_TIMEOUT_MS`.
pub fn resolve_websocket_connect_timeout_ms(explicit: Option<u64>) -> u64 {
    explicit
        .or_else(|| {
            std::env::var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS)
}

pub fn websocket_connect_timeout_error(timeout_ms: u64) -> String {
    format!("WebSocket connect timeout after {timeout_ms}ms")
}

/// Codex websocket connect using the TS timeout. Tests never hit ChatGPT:
/// `PI_CODEX_WS_REPLY` / localhost only.
pub fn connect_codex_websocket(url: &str, timeout_ms: u64) -> Result<(), String> {
    if let Ok(reply) = std::env::var("PI_CODEX_WS_REPLY") {
        if reply == "timeout" {
            return Err(websocket_connect_timeout_error(timeout_ms));
        }
        return Ok(());
    }
    if cfg!(test) && !url.contains("127.0.0.1") && !url.contains("localhost") {
        return Ok(());
    }
    let host_port = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url);
    let addr: std::net::SocketAddr = host_port
        .parse()
        .or_else(|_| format!("{host_port}:80").parse())
        .map_err(|err| format!("WebSocket address: {err}"))?;
    match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(timeout_ms))
    {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            Err(websocket_connect_timeout_error(timeout_ms))
        }
        Err(err) => Err(format!("WebSocket connect failed: {err}")),
    }
}

/// Map a Codex / OpenAI Responses event `type` to a pi-ai stream event name.
pub fn map_codex_event_type(event_type: &str) -> Option<&'static str> {
    match event_type {
        "response.created" => Some("start"),
        "response.output_text.delta" | "response.refusal.delta" => Some("text_delta"),
        "response.reasoning_summary_text.delta"
        | "response.reasoning_text.delta"
        | "response.reasoning_summary_part.done" => Some("thinking_delta"),
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            Some("toolcall_delta")
        }
        "response.done" | "response.completed" | "response.incomplete" => Some("done"),
        "error" => Some("error"),
        _ => None,
    }
}

pub fn normalize_codex_terminal_event(event_type: &str) -> &str {
    match event_type {
        "response.done" | "response.completed" | "response.incomplete" => "response.completed",
        other => other,
    }
}

pub fn is_websocket_connection_limit_reached(error: &str) -> bool {
    error.contains(WEBSOCKET_CONNECTION_LIMIT_REACHED)
}

pub fn is_previous_response_not_found(error: &str) -> bool {
    error.contains(PREVIOUS_RESPONSE_NOT_FOUND)
}

/// TS retries websocket once on connection-limit, then falls back to SSE
/// only when no websocket message stream has started.
pub fn should_fallback_to_sse(error: &str, websocket_started: bool) -> bool {
    !websocket_started && is_websocket_connection_limit_reached(error)
}

pub fn should_retry_websocket_connection_limit(error: &str, already_retried: bool) -> bool {
    !already_retried && is_websocket_connection_limit_reached(error)
}

pub fn should_retry_missing_previous_response(error: &str, already_retried: bool) -> bool {
    !already_retried && is_previous_response_not_found(error)
}

fn event_delta(value: &Value) -> String {
    value
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Replay a Codex Responses JSON/SSE fixture into pi-ai assistant events.
/// Never opens a websocket or hits the network.
pub fn replay_codex_events(model: &Model, corpus: &str) -> Vec<AssistantMessageEvent> {
    let mut message = AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: Vec::new(),
        model: format!("{}/{}", model.provider, model.id),
        usage: None,
        stop_reason: None,
        error_message: None,
    };
    let mut events = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut started = false;
    let mut text_started = false;
    let mut thinking_started = false;

    for raw_event in corpus_events(corpus) {
        let event_type = raw_event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mapped = map_codex_event_type(event_type);
        match mapped {
            Some("start") => {
                if !started {
                    events.push(AssistantMessageEvent::Start {
                        partial: message.clone(),
                    });
                    started = true;
                }
            }
            Some("text_delta") => {
                let delta = event_delta(&raw_event);
                if !started {
                    events.push(AssistantMessageEvent::Start {
                        partial: message.clone(),
                    });
                    started = true;
                }
                if !text_started {
                    events.push(AssistantMessageEvent::TextStart {
                        content_index: 0,
                        partial: message.clone(),
                    });
                    text_started = true;
                }
                text.push_str(&delta);
                if let Some(ContentBlock::Text { text: existing }) = message.content.get_mut(0) {
                    existing.push_str(&delta);
                } else {
                    message.content.insert(
                        0,
                        ContentBlock::Text {
                            text: delta.clone(),
                        },
                    );
                }
                events.push(AssistantMessageEvent::TextDelta {
                    content_index: 0,
                    delta,
                    partial: message.clone(),
                });
            }
            Some("thinking_delta") => {
                let delta = if event_type == "response.reasoning_summary_part.done" {
                    "\n\n".into()
                } else {
                    event_delta(&raw_event)
                };
                if !thinking_started {
                    events.push(AssistantMessageEvent::ThinkingStart {
                        content_index: if text_started { 1 } else { 0 },
                        partial: message.clone(),
                    });
                    thinking_started = true;
                }
                thinking.push_str(&delta);
                events.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: if text_started { 1 } else { 0 },
                    delta,
                    partial: message.clone(),
                });
            }
            Some("toolcall_delta") => {
                let delta = raw_event
                    .get("delta")
                    .and_then(Value::as_str)
                    .or_else(|| raw_event.get("arguments").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                events.push(AssistantMessageEvent::ToolcallDelta {
                    content_index: message.content.len(),
                    delta,
                    partial: message.clone(),
                });
            }
            Some("done") => {
                if text_started {
                    events.push(AssistantMessageEvent::TextEnd {
                        content_index: 0,
                        content: text.clone(),
                        partial: message.clone(),
                    });
                }
                if thinking_started {
                    events.push(AssistantMessageEvent::ThinkingEnd {
                        content_index: if text_started { 1 } else { 0 },
                        content: thinking.clone(),
                        partial: message.clone(),
                    });
                }
                message.stop_reason = Some(StopReason::Stop);
                events.push(AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: message.clone(),
                });
            }
            Some("error") => {
                message.stop_reason = Some(StopReason::Error);
                message.error_message = raw_event
                    .pointer("/error/code")
                    .and_then(Value::as_str)
                    .or_else(|| raw_event.get("code").and_then(Value::as_str))
                    .map(str::to_string);
                events.push(AssistantMessageEvent::Error {
                    reason: StopReason::Error,
                    error: message.clone(),
                });
            }
            _ => {}
        }
    }
    if !events
        .iter()
        .any(|event| matches!(event, AssistantMessageEvent::Done { .. }))
    {
        message.stop_reason = Some(StopReason::Stop);
        events.push(AssistantMessageEvent::Done {
            reason: StopReason::Stop,
            message,
        });
    }
    events
}

fn corpus_events(corpus: &str) -> Vec<Value> {
    let trimmed = corpus.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).unwrap_or_default();
    }
    let mut events = Vec::new();
    if trimmed.contains("data:") {
        for block in trimmed.split("\n\n") {
            let mut data = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(rest.trim_start());
                }
            }
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                events.push(value);
            }
        }
        return events;
    }
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            events.push(value);
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;

    fn model() -> Model {
        Model {
            id: "gpt-5".into(),
            name: "gpt-5".into(),
            api: "openai-codex-responses".into(),
            provider: "openai-codex".into(),
            base_url: Some(DEFAULT_CODEX_BASE_URL.into()),
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 1,
            max_tokens: 1,
            compat: Value::Null,
            headers: Default::default(),
        }
    }

    #[test]
    fn maps_ts_event_names_and_sse_fallback() {
        assert_eq!(map_codex_event_type("response.created"), Some("start"));
        assert_eq!(
            map_codex_event_type("response.output_text.delta"),
            Some("text_delta")
        );
        assert_eq!(map_codex_event_type("response.completed"), Some("done"));
        assert_eq!(
            normalize_codex_terminal_event("response.done"),
            "response.completed"
        );
        assert!(should_fallback_to_sse(
            "error: websocket_connection_limit_reached",
            false
        ));
        assert!(!should_fallback_to_sse(
            "error: websocket_connection_limit_reached",
            true
        ));
        assert!(should_retry_missing_previous_response(
            PREVIOUS_RESPONSE_NOT_FOUND,
            false
        ));
        assert_eq!(WEBSOCKET_MESSAGE_TOO_BIG_CLOSE_CODE, 1009);
        assert_eq!(
            WEBSOCKET_CLOSED_BEFORE_COMPLETED,
            "WebSocket stream closed before response.completed"
        );
    }

    #[test]
    fn websocket_connect_timeout_uses_explicit_then_env_then_default() {
        let previous = std::env::var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS").ok();
        std::env::remove_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS");
        assert_eq!(
            resolve_websocket_connect_timeout_ms(None),
            DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS
        );
        assert_eq!(resolve_websocket_connect_timeout_ms(Some(2500)), 2500);
        std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", "3200");
        assert_eq!(resolve_websocket_connect_timeout_ms(None), 3200);
        assert_eq!(resolve_websocket_connect_timeout_ms(Some(900)), 900);
        match previous {
            Some(value) => std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", value),
            None => std::env::remove_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS"),
        }
    }

    #[test]
    fn websocket_connect_fixture_timeout_uses_resolved_ms() {
        let previous = std::env::var("PI_CODEX_WS_REPLY").ok();
        std::env::set_var("PI_CODEX_WS_REPLY", "timeout");
        let error = connect_codex_websocket("wss://chatgpt.com/backend-api", 1234).unwrap_err();
        assert_eq!(error, websocket_connect_timeout_error(1234));
        match previous {
            Some(value) => std::env::set_var("PI_CODEX_WS_REPLY", value),
            None => std::env::remove_var("PI_CODEX_WS_REPLY"),
        }
    }

    #[test]
    fn replays_codex_sse_fixture_without_network() {
        let corpus = r#"
data: {"type":"response.created","response":{"id":"resp_1"}}

data: {"type":"response.output_text.delta","output_index":0,"delta":"Hello"}

data: {"type":"response.output_text.delta","output_index":0,"delta":" Codex"}

data: {"type":"response.completed","response":{"status":"completed"}}
"#;
        let events = replay_codex_events(&model(), corpus);
        let names: Vec<&str> = events
            .iter()
            .map(|event| match event {
                AssistantMessageEvent::Start { .. } => "start",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::Done { .. } => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(
            names,
            [
                "start",
                "text_start",
                "text_delta",
                "text_delta",
                "text_end",
                "done"
            ]
        );
        let done = events.last().unwrap();
        match done {
            AssistantMessageEvent::Done { message, .. } => {
                assert_eq!(
                    match &message.content[0] {
                        ContentBlock::Text { text } => text.as_str(),
                        _ => "",
                    },
                    "Hello Codex"
                );
            }
            _ => panic!("expected done"),
        }
    }
}
