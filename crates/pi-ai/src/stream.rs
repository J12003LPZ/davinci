use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::ResolvedAuth;
use crate::catalog::Model;
use crate::content_text;
use crate::ChatMessage;
use pi_protocol::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Deferred,
    Aborted,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    #[serde(rename = "toolCall")]
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub id: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(rename = "errorMessage", skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantMessageEvent {
    #[serde(rename = "start")]
    Start { partial: AssistantMessage },
    #[serde(rename = "text_start")]
    TextStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "text_end")]
    TextEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_start")]
    ThinkingStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_end")]
    ThinkingEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_start")]
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_delta")]
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_end")]
    ToolcallEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(rename = "toolCall")]
        tool_call: ContentBlock,
        partial: AssistantMessage,
    },
    #[serde(rename = "done")]
    Done {
        reason: StopReason,
        message: AssistantMessage,
    },
    #[serde(rename = "error")]
    Error {
        reason: StopReason,
        error: AssistantMessage,
    },
}

pub type StreamEvent = AssistantMessageEvent;

pub fn parse_sse_block(block: &str) -> Option<Value> {
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
        return None;
    }
    serde_json::from_str(&data).ok()
}

pub fn replay_sse_events(model: &Model, corpus: &str) -> Vec<AssistantMessageEvent> {
    let mut message = AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: Vec::new(),
        model: format!("{}/{}", model.provider, model.id),
        usage: None,
        stop_reason: None,
        error_message: None,
    };
    let mut events = vec![AssistantMessageEvent::Start {
        partial: message.clone(),
    }];
    let mut text = String::new();
    let mut started_text = false;
    for block in corpus.split("\n\n") {
        let Some(value) = parse_sse_block(block) else {
            continue;
        };
        if let Some(delta) = value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        {
            if !started_text {
                events.push(AssistantMessageEvent::TextStart {
                    content_index: 0,
                    partial: message.clone(),
                });
                started_text = true;
            }
            text.push_str(delta);
            if let Some(ContentBlock::Text { text: existing }) = message.content.get_mut(0) {
                existing.push_str(delta);
            } else {
                message.content.push(ContentBlock::Text {
                    text: delta.to_string(),
                });
            }
            events.push(AssistantMessageEvent::TextDelta {
                content_index: 0,
                delta: delta.to_string(),
                partial: message.clone(),
            });
        }
        if let Some(usage) = value.get("usage") {
            message.usage = Some(Usage {
                input: usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                output: usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cache_read: 0,
                cache_write: 0,
                reasoning: None,
                total_tokens: usage
                    .get("total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cost: Default::default(),
            });
        }
    }
    if started_text {
        events.push(AssistantMessageEvent::TextEnd {
            content_index: 0,
            content: text,
            partial: message.clone(),
        });
    }
    message.stop_reason = Some(StopReason::Stop);
    events.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message,
    });
    events
}

pub fn complete_from_events(events: &[AssistantMessageEvent]) -> Option<AssistantMessage> {
    events.iter().rev().find_map(|event| match event {
        AssistantMessageEvent::Done { message, .. } => Some(message.clone()),
        AssistantMessageEvent::Error { error, .. } => Some(error.clone()),
        _ => None,
    })
}

pub fn fixture_complete(
    model: &Model,
    _messages: &[ChatMessage],
    corpus: &str,
) -> AssistantMessage {
    complete_from_events(&replay_sse_events(model, corpus)).expect("fixture stream")
}

pub fn live_complete(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
) -> Result<AssistantMessage, String> {
    let body = match model.api.as_str() {
        "anthropic-messages" => anthropic_body(model, messages, system),
        "google-generative-ai" | "google-vertex" => google_body(model, messages, system),
        _ => openai_body(model, messages, system),
    };
    let url = request_url(model, auth);
    let mut request = ureq::post(&url);
    for (key, value) in &auth.headers {
        request = request.set(key, value);
    }
    if let Some(key) = &auth.api_key {
        if model.api.starts_with("google") {
            // key is already in the URL for Gemini
        } else if model.api == "anthropic-messages" {
            request = request
                .set("x-api-key", key)
                .set("anthropic-version", "2023-06-01");
        } else {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
    }
    request = request.set("content-type", "application/json");
    let response = request
        .send_string(&body.to_string())
        .map_err(|err| format!("Provider request failed: {err}"))?;
    let text = response
        .into_string()
        .map_err(|err| format!("Unable to read provider response: {err}"))?;
    Ok(parse_provider_response(model, &text))
}

fn request_url(model: &Model, auth: &ResolvedAuth) -> String {
    let base = model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    match model.api.as_str() {
        "anthropic-messages" => format!("{}/v1/messages", base.trim_end_matches('/')),
        "google-generative-ai" => {
            let key = auth.api_key.clone().unwrap_or_default();
            format!(
                "{}/models/{}:generateContent?key={key}",
                base.trim_end_matches('/'),
                model.id
            )
        }
        "openai-responses" | "azure-openai-responses" => {
            format!("{}/responses", base.trim_end_matches('/'))
        }
        _ => format!("{}/chat/completions", base.trim_end_matches('/')),
    }
}

fn openai_body(model: &Model, messages: &[ChatMessage], system: Option<&str>) -> Value {
    let mut out = Vec::new();
    if let Some(system) = system {
        out.push(serde_json::json!({"role":"system","content":system}));
    }
    for message in messages {
        out.push(serde_json::json!({
            "role": message.role,
            "content": content_text(&message.content),
        }));
    }
    serde_json::json!({
        "model": model.id,
        "messages": out,
        "stream": false,
    })
}

fn anthropic_body(model: &Model, messages: &[ChatMessage], system: Option<&str>) -> Value {
    let converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": content_text(&message.content),
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "model": model.id,
        "max_tokens": model.max_tokens.min(8192),
        "messages": converted,
        "stream": false,
    });
    if let Some(system) = system {
        body["system"] = Value::String(system.to_string());
    }
    body
}

fn google_body(model: &Model, messages: &[ChatMessage], system: Option<&str>) -> Value {
    let contents: Vec<Value> = messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": if message.role == "assistant" { "model" } else { "user" },
                "parts": [{"text": content_text(&message.content)}],
            })
        })
        .collect();
    let mut body = serde_json::json!({"contents": contents});
    if let Some(system) = system {
        body["systemInstruction"] = serde_json::json!({"parts":[{"text":system}]});
    }
    let _ = model;
    body
}

fn parse_provider_response(model: &Model, raw: &str) -> AssistantMessage {
    if raw.contains("data:") {
        return fixture_complete(model, &[], raw);
    }
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/content/0/text").and_then(Value::as_str))
        .or_else(|| {
            value
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(Value::as_str)
        })
        .or_else(|| value.pointer("/output_text").and_then(Value::as_str))
        .unwrap_or(raw)
        .to_string();
    let usage = value.get("usage").map(|usage| Usage {
        input: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read: 0,
        cache_write: 0,
        reasoning: None,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cost: Default::default(),
    });
    AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text { text }],
        model: format!("{}/{}", model.provider, model.id),
        usage,
        stop_reason: Some(StopReason::Stop),
        error_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::load_builtin_models;

    #[test]
    fn sse_lifecycle_matches_ts_event_names() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.provider == "openai")
            .expect("openai model");
        let corpus = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\ndata: [DONE]\n";
        let events = replay_sse_events(&model, corpus);
        let types: Vec<_> = events
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
            types,
            ["start", "text_start", "text_delta", "text_end", "done"]
        );
        let done = complete_from_events(&events).unwrap();
        assert_eq!(done.usage.unwrap().input, 3);
    }
}
