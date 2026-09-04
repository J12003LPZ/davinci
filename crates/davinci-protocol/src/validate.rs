//! Strict TypeBox-equivalent validation for protocol messages.

use serde_json::Value;
use thiserror::Error;

use crate::PROTOCOL_VERSION;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct ProtocolValidationError(pub String);

impl ProtocolValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

fn is_object(value: &Value) -> bool {
    value.is_object()
}

fn keys_only(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .map(|obj| obj.keys().all(|k| allowed.contains(&k.as_str())))
        .unwrap_or(false)
}

fn has_keys(value: &Value, required: &[&str]) -> bool {
    value
        .as_object()
        .map(|obj| required.iter().all(|k| obj.contains_key(*k)))
        .unwrap_or(false)
}

fn is_id(value: &Value) -> bool {
    value.as_str().is_some_and(|s| !s.is_empty())
}

fn is_timestamp(value: &Value) -> bool {
    value.as_i64().is_some_and(|n| n >= 0) || value.as_u64().is_some()
}

fn is_nonneg_int(value: &Value) -> bool {
    value.as_i64().is_some_and(|n| n >= 0) || value.as_u64().is_some()
}

fn is_pos_int(value: &Value) -> bool {
    value.as_i64().is_some_and(|n| n >= 1) || value.as_u64().is_some_and(|n| n >= 1)
}

fn is_nonneg_number(value: &Value) -> bool {
    value.as_f64().is_some_and(|n| n >= 0.0 && n.is_finite())
}

fn is_protocol_value(value: &Value, _optional_property: bool) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
        Value::Array(items) => items.iter().all(|item| is_protocol_value(item, false)),
        Value::Object(map) => map.values().all(|item| is_protocol_value(item, true)),
    }
}

fn thinking_level(value: &Value) -> bool {
    matches!(
        value.as_str(),
        Some("off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max")
    )
}

fn session_phase(value: &Value) -> bool {
    matches!(
        value.as_str(),
        Some("idle" | "turn" | "compaction" | "branch_summary" | "retry")
    )
}

fn model_ref(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["provider", "id"])
        && keys_only(value, &["provider", "id"])
        && is_id(&value["provider"])
        && is_id(&value["id"])
}

fn model_cost(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["input", "output", "cacheRead", "cacheWrite"])
        && keys_only(value, &["input", "output", "cacheRead", "cacheWrite"])
        && ["input", "output", "cacheRead", "cacheWrite"]
            .iter()
            .all(|k| is_nonneg_number(&value[*k]))
}

fn model_metadata(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "provider",
        "id",
        "name",
        "api",
        "reasoning",
        "input",
        "contextWindow",
        "maxTokens",
        "cost",
        "supportedThinkingLevels",
        "authenticated",
    ];
    is_object(value)
        && has_keys(value, KEYS)
        && keys_only(value, KEYS)
        && is_id(&value["provider"])
        && is_id(&value["id"])
        && value["name"].as_str().is_some_and(|s| !s.is_empty())
        && is_id(&value["api"])
        && value["reasoning"].is_boolean()
        && value["input"].as_array().is_some_and(|items| {
            !items.is_empty()
                || items
                    .iter()
                    .all(|i| matches!(i.as_str(), Some("text" | "image")))
        })
        && value["input"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|i| matches!(i.as_str(), Some("text" | "image")))
        })
        && is_pos_int(&value["contextWindow"])
        && is_pos_int(&value["maxTokens"])
        && model_cost(&value["cost"])
        && value["supportedThinkingLevels"]
            .as_array()
            .is_some_and(|items| !items.is_empty() && items.iter().all(thinking_level))
        && value["authenticated"].is_boolean()
}

fn json_value(value: &Value) -> bool {
    is_protocol_value(value, false)
}

fn text_content(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "text"])
        && keys_only(value, &["type", "text"])
        && value["type"] == "text"
        && value["text"].is_string()
}

fn thinking_content(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "thinking"])
        && keys_only(value, &["type", "thinking", "redacted"])
        && value["type"] == "thinking"
        && value["thinking"].is_string()
        && (value.get("redacted").is_none() || value["redacted"].is_boolean())
}

fn image_content(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "data", "mimeType"])
        && keys_only(value, &["type", "data", "mimeType"])
        && value["type"] == "image"
        && value["data"].is_string()
        && value["mimeType"].as_str().is_some_and(|s| !s.is_empty())
}

fn tool_call_content(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "toolCallId", "toolName", "input"])
        && keys_only(value, &["type", "toolCallId", "toolName", "input"])
        && value["type"] == "toolCall"
        && is_id(&value["toolCallId"])
        && is_id(&value["toolName"])
        && json_value(&value["input"])
}

fn user_content(value: &Value) -> bool {
    text_content(value) || image_content(value)
}

fn assistant_content(value: &Value) -> bool {
    text_content(value) || thinking_content(value) || tool_call_content(value)
}

fn tool_content(value: &Value) -> bool {
    text_content(value) || image_content(value)
}

fn usage(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "input",
        "output",
        "cacheRead",
        "cacheWrite",
        "reasoning",
        "totalTokens",
        "cost",
    ];
    is_object(value)
        && has_keys(
            value,
            &[
                "input",
                "output",
                "cacheRead",
                "cacheWrite",
                "totalTokens",
                "cost",
            ],
        )
        && keys_only(value, KEYS)
        && ["input", "output", "cacheRead", "cacheWrite", "totalTokens"]
            .iter()
            .all(|k| is_nonneg_int(&value[*k]))
        && (value.get("reasoning").is_none() || is_nonneg_int(&value["reasoning"]))
        && {
            let cost = &value["cost"];
            is_object(cost)
                && has_keys(
                    cost,
                    &["input", "output", "cacheRead", "cacheWrite", "total"],
                )
                && keys_only(
                    cost,
                    &["input", "output", "cacheRead", "cacheWrite", "total"],
                )
                && ["input", "output", "cacheRead", "cacheWrite", "total"]
                    .iter()
                    .all(|k| is_nonneg_number(&cost[*k]))
        }
}

fn user_item(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["id", "role", "content", "timestamp"])
        && keys_only(value, &["id", "role", "content", "timestamp"])
        && is_id(&value["id"])
        && value["role"] == "user"
        && value["content"]
            .as_array()
            .is_some_and(|items| items.iter().all(user_content))
        && is_timestamp(&value["timestamp"])
}

fn assistant_item(value: &Value, finished: bool) -> bool {
    const BASE: &[&str] = &[
        "id",
        "role",
        "content",
        "model",
        "responseModel",
        "usage",
        "timestamp",
        "status",
        "stopReason",
        "errorMessage",
    ];
    if !is_object(value)
        || !has_keys(
            value,
            &["id", "role", "content", "model", "timestamp", "status"],
        )
        || !keys_only(value, BASE)
        || !is_id(&value["id"])
        || value["role"] != "assistant"
        || !value["content"]
            .as_array()
            .is_some_and(|items| items.iter().all(assistant_content))
        || !model_ref(&value["model"])
        || (value.get("responseModel").is_some()
            && value["responseModel"].as_str().is_none_or(|s| s.is_empty()))
        || (value.get("usage").is_some() && !usage(&value["usage"]))
        || !is_timestamp(&value["timestamp"])
    {
        return false;
    }
    match value["status"].as_str() {
        Some("streaming") => {
            !finished && value.get("stopReason").is_none() && value.get("errorMessage").is_none()
        }
        Some("complete") => {
            matches!(
                value.get("stopReason").and_then(|v| v.as_str()),
                Some("stop" | "length" | "toolUse")
            ) && value.get("errorMessage").is_none()
        }
        Some("error") => {
            value.get("stopReason").and_then(|v| v.as_str()) == Some("error")
                && (value.get("errorMessage").is_none()
                    || value["errorMessage"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()))
        }
        Some("aborted") => {
            value.get("stopReason").and_then(|v| v.as_str()) == Some("aborted")
                && (value.get("errorMessage").is_none() || value["errorMessage"].is_string())
        }
        _ => false,
    }
}

fn tool_item(value: &Value, finished: bool) -> bool {
    const KEYS: &[&str] = &[
        "id",
        "role",
        "toolCallId",
        "toolName",
        "input",
        "content",
        "details",
        "usage",
        "timestamp",
        "status",
        "isError",
    ];
    if !is_object(value)
        || !has_keys(
            value,
            &[
                "id",
                "role",
                "toolCallId",
                "toolName",
                "input",
                "content",
                "timestamp",
                "status",
                "isError",
            ],
        )
        || !keys_only(value, KEYS)
        || !is_id(&value["id"])
        || value["role"] != "tool"
        || !is_id(&value["toolCallId"])
        || !is_id(&value["toolName"])
        || !json_value(&value["input"])
        || !value["content"]
            .as_array()
            .is_some_and(|items| items.iter().all(tool_content))
        || (value.get("details").is_some() && !json_value(&value["details"]))
        || (value.get("usage").is_some() && !usage(&value["usage"]))
        || !is_timestamp(&value["timestamp"])
    {
        return false;
    }
    match (value["status"].as_str(), value["isError"].as_bool()) {
        (Some("running"), Some(false)) => !finished,
        (Some("complete"), Some(false)) => true,
        (Some("error"), Some(true)) => true,
        _ => false,
    }
}

fn transcript_item(value: &Value) -> bool {
    user_item(value) || assistant_item(value, false) || tool_item(value, false)
}

fn finished_item(value: &Value) -> bool {
    (assistant_item(value, true)
        && matches!(
            value["status"].as_str(),
            Some("complete" | "error" | "aborted")
        ))
        || (tool_item(value, true)
            && matches!(value["status"].as_str(), Some("complete" | "error")))
}

fn updated_item(value: &Value) -> bool {
    assistant_item(value, false) || tool_item(value, false)
}

fn transcript_progress(value: &Value) -> bool {
    if !is_object(value) || value.get("type").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    match value["type"].as_str() {
        Some("item_started") => {
            has_keys(value, &["type", "item"])
                && keys_only(value, &["type", "item"])
                && transcript_item(&value["item"])
        }
        Some("assistant_delta") => {
            has_keys(
                value,
                &["type", "messageId", "contentIndex", "kind", "delta"],
            ) && keys_only(
                value,
                &["type", "messageId", "contentIndex", "kind", "delta"],
            ) && is_id(&value["messageId"])
                && is_nonneg_int(&value["contentIndex"])
                && matches!(
                    value["kind"].as_str(),
                    Some("text" | "thinking" | "toolCall")
                )
                && value["delta"].is_string()
        }
        Some("item_updated") => {
            has_keys(value, &["type", "item"])
                && keys_only(value, &["type", "item"])
                && updated_item(&value["item"])
        }
        Some("item_finished") => {
            has_keys(value, &["type", "item"])
                && keys_only(value, &["type", "item"])
                && finished_item(&value["item"])
        }
        _ => false,
    }
}

fn session_metadata(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "id",
        "createdAt",
        "updatedAt",
        "parentSessionId",
        "sessionName",
        "cwd",
    ];
    is_object(value)
        && has_keys(value, &["id", "createdAt"])
        && keys_only(value, KEYS)
        && is_id(&value["id"])
        && is_timestamp(&value["createdAt"])
        && (value.get("updatedAt").is_none() || is_timestamp(&value["updatedAt"]))
        && (value.get("parentSessionId").is_none() || is_id(&value["parentSessionId"]))
        && (value.get("sessionName").is_none() || value["sessionName"].is_string())
        && (value.get("cwd").is_none() || value["cwd"].as_str().is_some_and(|s| !s.is_empty()))
}

fn session_snapshot(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "id",
        "name",
        "cwd",
        "createdAt",
        "updatedAt",
        "phase",
        "model",
        "thinkingLevel",
        "attached",
        "locked",
        "revision",
        "transcript",
        "queuedSteer",
        "queuedSteerCount",
    ];
    is_object(value)
        && has_keys(
            value,
            &[
                "id",
                "cwd",
                "createdAt",
                "updatedAt",
                "phase",
                "model",
                "thinkingLevel",
                "attached",
                "locked",
                "revision",
                "transcript",
                "queuedSteer",
                "queuedSteerCount",
            ],
        )
        && keys_only(value, KEYS)
        && is_id(&value["id"])
        && (value.get("name").is_none() || value["name"].is_string())
        && value["cwd"].as_str().is_some_and(|s| !s.is_empty())
        && is_timestamp(&value["createdAt"])
        && is_timestamp(&value["updatedAt"])
        && session_phase(&value["phase"])
        && model_ref(&value["model"])
        && thinking_level(&value["thinkingLevel"])
        && value["attached"].is_boolean()
        && value["locked"].is_boolean()
        && is_nonneg_int(&value["revision"])
        && value["transcript"]
            .as_array()
            .is_some_and(|items| items.iter().all(transcript_item))
        && value["queuedSteer"]
            .as_array()
            .is_some_and(|items| items.iter().all(user_item))
        && is_nonneg_int(&value["queuedSteerCount"])
}

fn server_snapshot(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "serverId",
        "protocolVersion",
        "revision",
        "sessions",
        "models",
    ];
    is_object(value)
        && has_keys(value, KEYS)
        && keys_only(value, KEYS)
        && is_id(&value["serverId"])
        && value["protocolVersion"] == PROTOCOL_VERSION
        && is_nonneg_int(&value["revision"])
        && value["sessions"]
            .as_array()
            .is_some_and(|items| items.iter().all(session_metadata))
        && value["models"]
            .as_array()
            .is_some_and(|items| items.iter().all(model_metadata))
}

fn protocol_error(value: &Value) -> bool {
    const KEYS: &[&str] = &["code", "message", "details"];
    is_object(value)
        && has_keys(value, &["code", "message"])
        && keys_only(value, KEYS)
        && matches!(
            value["code"].as_str(),
            Some(
                "version"
                    | "busy"
                    | "session_locked"
                    | "not_found"
                    | "invalid_request"
                    | "not_implemented"
                    | "internal_error"
            )
        )
        && value["message"].is_string()
        && (value.get("details").is_none() || json_value(&value["details"]))
}

fn command(value: &Value) -> bool {
    if !is_object(value) || value.get("command").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    match value["command"].as_str() {
        Some("list") => keys_only(value, &["command"]),
        Some("create") => {
            keys_only(value, &["command", "cwd", "name", "model", "thinkingLevel"])
                && (value.get("cwd").is_none()
                    || value["cwd"].as_str().is_some_and(|s| !s.is_empty()))
                && (value.get("name").is_none() || value["name"].is_string())
                && (value.get("model").is_none() || model_ref(&value["model"]))
                && (value.get("thinkingLevel").is_none() || thinking_level(&value["thinkingLevel"]))
        }
        Some("attach" | "detach" | "abort") => {
            has_keys(value, &["command", "sessionId"])
                && keys_only(value, &["command", "sessionId"])
                && is_id(&value["sessionId"])
        }
        Some("prompt" | "steer") => {
            has_keys(value, &["command", "sessionId", "text"])
                && keys_only(value, &["command", "sessionId", "text"])
                && is_id(&value["sessionId"])
                && value["text"].is_string()
        }
        Some("set_model") => {
            has_keys(value, &["command", "sessionId", "model"])
                && keys_only(value, &["command", "sessionId", "model"])
                && is_id(&value["sessionId"])
                && model_ref(&value["model"])
        }
        Some("set_thinking") => {
            has_keys(value, &["command", "sessionId", "thinkingLevel"])
                && keys_only(value, &["command", "sessionId", "thinkingLevel"])
                && is_id(&value["sessionId"])
                && thinking_level(&value["thinkingLevel"])
        }
        _ => false,
    }
}

fn command_result(value: &Value) -> bool {
    if !is_object(value) || value.get("command").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    match value["command"].as_str() {
        Some("list") => {
            has_keys(value, &["command", "sessions"])
                && keys_only(value, &["command", "sessions"])
                && value["sessions"]
                    .as_array()
                    .is_some_and(|items| items.iter().all(session_metadata))
        }
        Some("detach") => {
            has_keys(value, &["command", "sessionId"])
                && keys_only(value, &["command", "sessionId"])
                && is_id(&value["sessionId"])
        }
        Some("create" | "attach" | "prompt" | "steer" | "abort" | "set_model" | "set_thinking") => {
            has_keys(value, &["command", "session"])
                && keys_only(value, &["command", "session"])
                && session_snapshot(&value["session"])
        }
        _ => false,
    }
}

fn client_hello_fixed(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "version"])
        && keys_only(value, &["type", "version"])
        && value["type"] == "hello"
        && value["version"].as_i64().is_some_and(|n| n >= 0)
        && value["version"].as_f64().is_some_and(|n| n.fract() == 0.0)
        && !value["version"].is_string()
}

fn request_envelope(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "id", "request"])
        && keys_only(value, &["type", "id", "request"])
        && value["type"] == "request"
        && is_id(&value["id"])
        && command(&value["request"])
}

fn server_event(value: &Value) -> bool {
    if !is_object(value) {
        return false;
    }
    match value.get("type").and_then(|v| v.as_str()) {
        Some("server_snapshot") => {
            has_keys(value, &["type", "snapshot"])
                && keys_only(value, &["type", "snapshot"])
                && server_snapshot(&value["snapshot"])
        }
        Some("session_snapshot") => {
            has_keys(value, &["type", "snapshot"])
                && keys_only(value, &["type", "snapshot"])
                && session_snapshot(&value["snapshot"])
        }
        Some("session_progress") => {
            has_keys(value, &["type", "sessionId", "progress"])
                && keys_only(value, &["type", "sessionId", "progress"])
                && is_id(&value["sessionId"])
                && transcript_progress(&value["progress"])
        }
        Some("session_removed") => {
            has_keys(value, &["type", "sessionId"])
                && keys_only(value, &["type", "sessionId"])
                && is_id(&value["sessionId"])
        }
        _ => false,
    }
}

fn server_hello(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "version", "connectionId", "snapshot"])
        && keys_only(value, &["type", "version", "connectionId", "snapshot"])
        && value["type"] == "hello"
        && value["version"] == PROTOCOL_VERSION
        && is_id(&value["connectionId"])
        && server_snapshot(&value["snapshot"])
}

fn server_hello_error(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "error"])
        && keys_only(value, &["type", "error"])
        && value["type"] == "hello_error"
        && protocol_error(&value["error"])
}

fn response_envelope(value: &Value) -> bool {
    if !is_object(value)
        || value.get("type") != Some(&Value::String("response".into()))
        || !is_id(&value["id"])
    {
        return false;
    }
    match value.get("ok").and_then(|v| v.as_bool()) {
        Some(true) => {
            has_keys(value, &["type", "id", "ok", "result"])
                && keys_only(value, &["type", "id", "ok", "result"])
                && command_result(&value["result"])
        }
        Some(false) => {
            has_keys(value, &["type", "id", "ok", "error"])
                && keys_only(value, &["type", "id", "ok", "error"])
                && protocol_error(&value["error"])
        }
        _ => false,
    }
}

fn event_envelope(value: &Value) -> bool {
    is_object(value)
        && has_keys(value, &["type", "event"])
        && keys_only(value, &["type", "event"])
        && value["type"] == "event"
        && server_event(&value["event"])
}

pub fn parse_client_message(value: &Value) -> Result<Value, ProtocolValidationError> {
    if !is_protocol_value(value, false) || !(client_hello_fixed(value) || request_envelope(value)) {
        return Err(ProtocolValidationError::new(
            "Invalid client protocol message",
        ));
    }
    Ok(value.clone())
}

pub fn parse_server_message(value: &Value) -> Result<Value, ProtocolValidationError> {
    if !is_protocol_value(value, false)
        || !(server_hello(value)
            || server_hello_error(value)
            || response_envelope(value)
            || event_envelope(value))
    {
        return Err(ProtocolValidationError::new(
            "Invalid server protocol message",
        ));
    }
    Ok(value.clone())
}
