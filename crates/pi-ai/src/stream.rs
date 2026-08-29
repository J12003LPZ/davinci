use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::ResolvedAuth;
use crate::catalog::Model;
use crate::content_text;
use crate::thinking::{
    clamp_thinking_budget_to_answer_room, google_thinking_budget, thinking_budget_for_level,
    ThinkingBudgets,
};
use crate::{ChatMessage, MessageContent, ToolSpec};
use pi_protocol::{ThinkingLevel, Usage};

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub thinking_level: Option<ThinkingLevel>,
    pub thinking_budgets: Option<ThinkingBudgets>,
    pub timeout_ms: Option<u64>,
    pub max_retries: Option<u32>,
    pub max_retry_delay_ms: Option<u64>,
}

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
        if let Some(event_type) = value.get("type").and_then(Value::as_str) {
            if crate::codex::map_codex_event_type(event_type).is_some() {
                return crate::codex::replay_codex_events(model, corpus);
            }
        }
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

pub fn assistant_to_chat(message: &AssistantMessage) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => MessageContent::Text { text: text.clone() },
                ContentBlock::Thinking { thinking } => MessageContent::Thinking {
                    thinking: thinking.clone(),
                    redacted: None,
                },
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => MessageContent::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
            })
            .collect(),
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

pub fn live_complete(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
) -> Result<AssistantMessage, String> {
    live_complete_with(
        model,
        messages,
        auth,
        system,
        tools,
        &StreamOptions::default(),
    )
}

pub fn live_complete_with(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Result<AssistantMessage, String> {
    let body = request_body_with(model, messages, system, tools, options);
    let url = request_url(model, auth);
    let mut request = ureq::post(&url);
    if let Some(timeout_ms) = options.timeout_ms.filter(|ms| *ms > 0) {
        request = request.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    let _ = (options.max_retries, options.max_retry_delay_ms);
    for (key, value) in &auth.headers {
        request = request.set(key, value);
    }
    for (key, value) in &model.headers {
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

pub fn request_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
) -> Value {
    request_body_with(model, messages, system, tools, &StreamOptions::default())
}

pub fn request_body_with(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    match model.api.as_str() {
        "anthropic-messages" | "pi-messages" => {
            anthropic_body(model, messages, system, tools, options)
        }
        "google-generative-ai" | "google-vertex" => {
            google_body(model, messages, system, tools, options)
        }
        "bedrock-converse-stream" => bedrock_body(model, messages, system, tools),
        "mistral-conversations" => mistral_body(model, messages, system, tools),
        _ => openai_body(model, messages, system, tools, options),
    }
}

/// Stream complete via SSE when the provider returns `data:` frames; otherwise wrap `complete`.
pub fn live_stream(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
) -> Result<Vec<AssistantMessageEvent>, String> {
    let mut body = request_body(model, messages, system, tools);
    if let Value::Object(map) = &mut body {
        map.insert("stream".into(), Value::Bool(true));
    }
    let url = request_url(model, auth);
    let mut request = ureq::post(&url);
    for (key, value) in &auth.headers {
        request = request.set(key, value);
    }
    for (key, value) in &model.headers {
        request = request.set(key, value);
    }
    if let Some(key) = &auth.api_key {
        if model.api.starts_with("google") {
        } else if model.api == "anthropic-messages" || model.api == "pi-messages" {
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
    if text.contains("data:") {
        Ok(replay_sse_events(model, &text))
    } else {
        let message = parse_provider_response(model, &text);
        Ok(events_from_complete(&message))
    }
}

pub fn events_from_complete(message: &AssistantMessage) -> Vec<AssistantMessageEvent> {
    let mut events = vec![AssistantMessageEvent::Start {
        partial: message.clone(),
    }];
    for (index, block) in message.content.iter().enumerate() {
        match block {
            ContentBlock::Text { text } => {
                events.push(AssistantMessageEvent::TextStart {
                    content_index: index,
                    partial: message.clone(),
                });
                events.push(AssistantMessageEvent::TextDelta {
                    content_index: index,
                    delta: text.clone(),
                    partial: message.clone(),
                });
                events.push(AssistantMessageEvent::TextEnd {
                    content_index: index,
                    content: text.clone(),
                    partial: message.clone(),
                });
            }
            ContentBlock::Thinking { thinking } => {
                events.push(AssistantMessageEvent::ThinkingStart {
                    content_index: index,
                    partial: message.clone(),
                });
                events.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: index,
                    delta: thinking.clone(),
                    partial: message.clone(),
                });
                events.push(AssistantMessageEvent::ThinkingEnd {
                    content_index: index,
                    content: thinking.clone(),
                    partial: message.clone(),
                });
            }
            ContentBlock::ToolCall { .. } => {
                events.push(AssistantMessageEvent::ToolcallStart {
                    content_index: index,
                    partial: message.clone(),
                });
                events.push(AssistantMessageEvent::ToolcallEnd {
                    content_index: index,
                    tool_call: block.clone(),
                    partial: message.clone(),
                });
            }
        }
    }
    events.push(AssistantMessageEvent::Done {
        reason: message.stop_reason.unwrap_or(StopReason::Stop),
        message: message.clone(),
    });
    events
}

fn bedrock_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
) -> Value {
    let converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": [{"text": content_text(&message.content)}],
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "modelId": model.id,
        "messages": converted,
    });
    if let Some(system) = system {
        body["system"] = serde_json::json!([{"text": system}]);
    }
    if !tools.is_empty() {
        body["toolConfig"] = serde_json::json!({
            "tools": tools.iter().map(|tool| serde_json::json!({
                "toolSpec": {
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": { "json": tool.parameters },
                }
            })).collect::<Vec<_>>()
        });
    }
    body
}

fn mistral_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
) -> Value {
    let mut body = openai_body(model, messages, system, tools, &StreamOptions::default());
    body["stream"] = Value::Bool(false);
    body
}

pub fn request_url(model: &Model, auth: &ResolvedAuth) -> String {
    let base = model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let base = base.trim_end_matches('/');
    match model.api.as_str() {
        "anthropic-messages" | "pi-messages" => format!("{base}/v1/messages"),
        "google-generative-ai" => {
            let key = auth.api_key.clone().unwrap_or_default();
            format!("{base}/models/{}:generateContent?key={key}", model.id)
        }
        "google-vertex" => format!(
            "{base}/v1/projects/default/locations/us-central1/publishers/google/models/{}:generateContent",
            model.id
        ),
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            format!("{base}/responses")
        }
        "mistral-conversations" => format!("{base}/v1/conversations"),
        "bedrock-converse-stream" => format!("{base}/model/{}/converse", model.id),
        _ => format!("{base}/chat/completions"),
    }
}

fn openai_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    let mut out = Vec::new();
    if let Some(system) = system {
        out.push(serde_json::json!({"role":"system","content":system}));
    }
    for message in messages {
        if message.role == "toolResult" {
            out.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": message.tool_call_id,
                "content": content_text(&message.content),
            }));
        } else if message
            .content
            .iter()
            .any(|block| matches!(block, MessageContent::ToolCall { .. }))
        {
            let tool_calls: Vec<Value> = message
                .content
                .iter()
                .filter_map(|block| match block {
                    MessageContent::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments.to_string(),
                        }
                    })),
                    _ => None,
                })
                .collect();
            out.push(serde_json::json!({
                "role": "assistant",
                "content": content_text(&message.content),
                "tool_calls": tool_calls,
            }));
        } else {
            out.push(serde_json::json!({
                "role": message.role,
                "content": content_text(&message.content),
            }));
        }
    }
    let mut body = serde_json::json!({
        "model": model.id,
        "messages": out,
        "stream": false,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect(),
        );
    }
    apply_openai_thinking(&mut body, model, options);
    body
}

fn apply_openai_thinking(body: &mut Value, model: &Model, options: &StreamOptions) {
    let Some(level) = options
        .thinking_level
        .filter(|level| *level != ThinkingLevel::Off)
    else {
        return;
    };
    let field = model
        .compat
        .get("thinkingTokenBudgetField")
        .and_then(Value::as_str)
        .or_else(|| {
            model
                .compat
                .get("supportsThinkingTokenBudget")
                .and_then(Value::as_bool)
                .filter(|enabled| *enabled)
                .map(|_| "thinking_token_budget")
        });
    let Some(field) = field else {
        return;
    };
    let ceiling = body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .or_else(|| body.get("max_completion_tokens").and_then(Value::as_u64))
        .unwrap_or(model.max_tokens) as u32;
    let budget = clamp_thinking_budget_to_answer_room(
        thinking_budget_for_level(level, options.thinking_budgets.as_ref()),
        ceiling,
    );
    if budget > 0 {
        body[field] = Value::from(budget);
    }
}

fn anthropic_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    let converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            if message.role == "toolResult" {
                serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id,
                        "content": content_text(&message.content),
                    }],
                })
            } else if message.role == "assistant"
                && message
                    .content
                    .iter()
                    .any(|block| matches!(block, MessageContent::ToolCall { .. }))
            {
                let content: Vec<Value> = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        MessageContent::Text { text } if !text.is_empty() => {
                            Some(serde_json::json!({"type":"text","text": text}))
                        }
                        MessageContent::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": arguments,
                        })),
                        _ => None,
                    })
                    .collect();
                serde_json::json!({"role":"assistant","content": content})
            } else {
                serde_json::json!({
                    "role": if message.role == "assistant" { "assistant" } else { "user" },
                    "content": content_text(&message.content),
                })
            }
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
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect(),
        );
    }
    if let Some(level) = options
        .thinking_level
        .filter(|level| *level != ThinkingLevel::Off)
    {
        let budget = thinking_budget_for_level(level, options.thinking_budgets.as_ref());
        let max_tokens = body
            .get("max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(model.max_tokens) as u32;
        let budget = clamp_thinking_budget_to_answer_room(budget, max_tokens);
        if budget > 0 {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": budget,
            });
        }
    }
    body
}

fn google_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
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
    if !tools.is_empty() {
        body["tools"] = serde_json::json!([{
            "functionDeclarations": tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })).collect::<Vec<_>>()
        }]);
    }
    if let Some(level) = options
        .thinking_level
        .filter(|level| *level != ThinkingLevel::Off)
    {
        let budget = google_thinking_budget(&model.id, level, options.thinking_budgets.as_ref());
        body["generationConfig"] = serde_json::json!({
            "thinkingConfig": { "thinkingBudget": budget }
        });
    }
    body
}

fn parse_provider_response(model: &Model, raw: &str) -> AssistantMessage {
    if raw.contains("data:") {
        return fixture_complete(model, &[], raw);
    }
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut content = Vec::new();
    if let Some(text) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/content/0/text").and_then(Value::as_str))
        .or_else(|| {
            value
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(Value::as_str)
        })
        .or_else(|| value.pointer("/output_text").and_then(Value::as_str))
    {
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
    }
    if let Some(calls) = value
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    {
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = call
                .pointer("/function/arguments")
                .and_then(|value| match value {
                    Value::String(raw) => serde_json::from_str(raw).ok(),
                    other => Some(other.clone()),
                })
                .unwrap_or(Value::Object(Default::default()));
            content.push(ContentBlock::ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                content.push(ContentBlock::ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    arguments: block.get("input").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
    if let Some(blocks) = value
        .pointer("/output/message/content")
        .and_then(Value::as_array)
    {
        for block in blocks {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                content.push(ContentBlock::Text {
                    text: text.to_string(),
                });
            }
            if let Some(tool) = block.get("toolUse") {
                content.push(ContentBlock::ToolCall {
                    id: tool
                        .get("toolUseId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    arguments: tool.get("input").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
    if let Some(text) = value
        .pointer("/outputs/0/content")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/output_text").and_then(Value::as_str))
    {
        if !text.is_empty() && content.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
    }
    if let Some(parts) = value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(call) = part.get("functionCall") {
                content.push(ContentBlock::ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    arguments: call.get("args").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
    if content.is_empty() {
        content.push(ContentBlock::Text {
            text: raw.to_string(),
        });
    }
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
    let stop_reason = if content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    {
        Some(StopReason::ToolUse)
    } else {
        Some(StopReason::Stop)
    };
    AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content,
        model: format!("{}/{}", model.provider, model.id),
        usage,
        stop_reason,
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

    #[test]
    fn parses_openai_and_anthropic_tool_calls() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.provider == "openai")
            .expect("openai model");
        let openai = parse_provider_response(
            &model,
            r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]}}]}"#,
        );
        assert!(matches!(openai.content[0], ContentBlock::ToolCall { .. }));
        assert_eq!(openai.stop_reason, Some(StopReason::ToolUse));
        let anthropic = parse_provider_response(
            &model,
            r#"{"content":[{"type":"tool_use","id":"c2","name":"bash","input":{"command":"ls"}}]}"#,
        );
        assert!(matches!(
            anthropic
                .content
                .iter()
                .find(|b| matches!(b, ContentBlock::ToolCall { .. })),
            Some(ContentBlock::ToolCall { .. })
        ));
        let events = events_from_complete(&openai);
        assert!(matches!(events[0], AssistantMessageEvent::Start { .. }));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
        let bedrock = parse_provider_response(
            &model,
            r#"{"output":{"message":{"content":[{"text":"ok"},{"toolUse":{"toolUseId":"t1","name":"read","input":{"path":"a"}}}]}}}"#,
        );
        assert!(bedrock
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolCall { .. })));
    }

    #[test]
    fn request_body_applies_thinking_budgets() {
        let mut anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        anthropic.max_tokens = 8192;
        let options = StreamOptions {
            thinking_level: Some(ThinkingLevel::Medium),
            thinking_budgets: Some(ThinkingBudgets {
                medium: Some(4096),
                ..ThinkingBudgets::default()
            }),
            ..StreamOptions::default()
        };
        let body = request_body_with(&anthropic, &[], None, &[], &options);
        assert_eq!(body["thinking"]["budget_tokens"], 4096);

        let mut google = load_builtin_models()
            .into_iter()
            .find(|m| m.id.contains("2.5-pro") && m.api.starts_with("google"))
            .expect("gemini 2.5 pro");
        google.max_tokens = 8192;
        let google_body = request_body_with(&google, &[], None, &[], &options);
        assert_eq!(
            google_body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            4096
        );

        let mut openai = load_builtin_models()
            .into_iter()
            .find(|m| m.api.contains("openai"))
            .expect("openai");
        openai.compat = serde_json::json!({"thinkingTokenBudgetField": "thinking_budget"});
        openai.max_tokens = 8192;
        let openai_body = request_body_with(&openai, &[], None, &[], &options);
        assert_eq!(openai_body["thinking_budget"], 4096);
    }
}
