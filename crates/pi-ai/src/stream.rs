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
    pub max_tokens: Option<u64>,
    pub websocket_connect_timeout_ms: Option<u64>,
    pub transport: Option<String>,
    pub session_id: Option<String>,
    pub cache_retention: Option<String>,
    pub install_telemetry: Option<bool>,
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

impl AssistantMessageEvent {
    pub fn message(&self) -> &AssistantMessage {
        match self {
            Self::Start { partial }
            | Self::TextStart { partial, .. }
            | Self::TextDelta { partial, .. }
            | Self::TextEnd { partial, .. }
            | Self::ThinkingStart { partial, .. }
            | Self::ThinkingDelta { partial, .. }
            | Self::ThinkingEnd { partial, .. }
            | Self::ToolcallStart { partial, .. }
            | Self::ToolcallDelta { partial, .. }
            | Self::ToolcallEnd { partial, .. } => partial,
            Self::Done { message, .. } => message,
            Self::Error { error, .. } => error,
        }
    }
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
        ..ChatMessage::default()
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
    if model.api == "openai-codex-responses" {
        if let Some(token) = auth.api_key.as_deref() {
            match crate::codex::try_codex_websocket_transport(
                model,
                &body,
                token,
                options.transport.as_deref(),
                options.session_id.as_deref(),
                options.cache_retention.as_deref(),
                options.websocket_connect_timeout_ms,
                options.timeout_ms,
            ) {
                Ok(crate::codex::CodexWebsocketOutcome::Message(message)) => return Ok(*message),
                Ok(crate::codex::CodexWebsocketOutcome::FallbackToSse) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let url = request_url(model, auth);
    let headers = crate::merge_provider_attribution_headers(
        model,
        options.session_id.as_deref(),
        options.install_telemetry,
        &collect_request_headers(model, auth, options.session_id.as_deref()),
    );
    let timeout_ms = options.timeout_ms.filter(|ms| *ms > 0);
    let compress_zstd = model.api == "openai-codex-responses";
    let text = crate::provider_retry::retry_provider_request(
        || send_provider_body(&url, &headers, &body, timeout_ms, compress_zstd),
        crate::provider_retry::ProviderRetryOptions {
            max_retries: options.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.max_retry_delay_ms,
        },
    )
    .map_err(|err| err.message)?;
    Ok(parse_provider_response(model, &text))
}

/// Streaming complete: prefer live SSE events over synthesizing `events_from_complete`.
pub fn live_complete_streaming_with(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Result<(AssistantMessage, Vec<AssistantMessageEvent>), String> {
    let mut body = request_body_with(model, messages, system, tools, options);
    if let Value::Object(map) = &mut body {
        map.insert("stream".into(), Value::Bool(true));
    }
    if model.api == "openai-codex-responses" {
        if let Some(token) = auth.api_key.as_deref() {
            match crate::codex::try_codex_websocket_transport(
                model,
                &body,
                token,
                options.transport.as_deref(),
                options.session_id.as_deref(),
                options.cache_retention.as_deref(),
                options.websocket_connect_timeout_ms,
                options.timeout_ms,
            ) {
                Ok(crate::codex::CodexWebsocketOutcome::Message(message)) => {
                    let events = events_from_complete(&message);
                    return Ok((*message, events));
                }
                Ok(crate::codex::CodexWebsocketOutcome::FallbackToSse) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let url = request_url(model, auth);
    let headers = crate::merge_provider_attribution_headers(
        model,
        options.session_id.as_deref(),
        options.install_telemetry,
        &collect_request_headers(model, auth, options.session_id.as_deref()),
    );
    let timeout_ms = options.timeout_ms.filter(|ms| *ms > 0);
    let compress_zstd = model.api == "openai-codex-responses";
    let text = crate::provider_retry::retry_provider_request(
        || send_provider_body(&url, &headers, &body, timeout_ms, compress_zstd),
        crate::provider_retry::ProviderRetryOptions {
            max_retries: options.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.max_retry_delay_ms,
        },
    )
    .map_err(|err| err.message)?;
    if text.contains("data:") {
        let events = replay_sse_events(model, &text);
        let message = complete_from_events(&events)
            .or_else(|| events.last().map(|event| event.message().clone()))
            .ok_or_else(|| "Empty provider stream".to_string())?;
        Ok((message, events))
    } else {
        let message = parse_provider_response(model, &text);
        let events = events_from_complete(&message);
        Ok((message, events))
    }
}

fn collect_request_headers(
    model: &Model,
    auth: &ResolvedAuth,
    session_id: Option<&str>,
) -> Vec<(String, String)> {
    if model.api == "openai-codex-responses" {
        if let Some(token) = &auth.api_key {
            if let Ok(account_id) = crate::codex::extract_account_id(token) {
                let extra: Vec<(String, String)> = auth
                    .headers
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                return crate::codex::build_sse_headers(
                    &model.headers,
                    &extra,
                    &account_id,
                    token,
                    session_id,
                );
            }
        }
    }
    let mut headers = Vec::new();
    for (key, value) in &auth.headers {
        headers.push((key.clone(), value.clone()));
    }
    for (key, value) in &model.headers {
        headers.push((key.clone(), value.clone()));
    }
    if let Some(key) = &auth.api_key {
        if model.api.starts_with("google") {
        } else if model.api == "anthropic-messages" {
            headers.push(("x-api-key".into(), key.clone()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        } else {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        }
    }
    headers.push(("content-type".into(), "application/json".into()));
    headers
}

fn send_provider_body(
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    timeout_ms: Option<u64>,
    compress_zstd: bool,
) -> Result<String, crate::provider_retry::ProviderError> {
    let mut request = ureq::post(url);
    if let Some(timeout_ms) = timeout_ms {
        request = request.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    for (key, value) in headers {
        request = request.set(key, value);
    }
    let response = if compress_zstd {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
        if compressed {
            request = request.set("content-encoding", "zstd");
        }
        request
            .send_bytes(&bytes)
            .map_err(crate::provider_retry::provider_error_from_ureq)?
    } else {
        request
            .send_string(&body.to_string())
            .map_err(crate::provider_retry::provider_error_from_ureq)?
    };
    response.into_string().map_err(|err| {
        crate::provider_retry::ProviderError::new(
            None,
            format!("Unable to read provider response: {err}"),
        )
    })
}

/// TS `completeSimple`: one-shot user prompt with no tools.
pub fn complete_simple(
    model: &Model,
    prompt: &str,
    system: Option<&str>,
    auth: &ResolvedAuth,
    options: &StreamOptions,
) -> Result<AssistantMessage, String> {
    live_complete_with(
        model,
        &[ChatMessage::text("user", prompt)],
        auth,
        system,
        &[],
        options,
    )
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
    let mut body = match model.api.as_str() {
        "anthropic-messages" | "pi-messages" => {
            anthropic_body(model, messages, system, tools, options)
        }
        "google-generative-ai" | "google-vertex" => {
            google_body(model, messages, system, tools, options)
        }
        "bedrock-converse-stream" => bedrock_body(model, messages, system, tools),
        "mistral-conversations" => mistral_body(model, messages, system, tools, options),
        "openai-responses" | "openai-codex-responses" | "azure-openai-responses" => {
            openai_responses_body(model, messages, system, tools, options)
        }
        _ => openai_body(model, messages, system, tools, options),
    };
    apply_max_tokens_override(&mut body, options);
    body
}

/// OpenAI Responses API body (TS `openai-responses.ts` `buildParams` /
/// `openai-codex-responses.ts` `buildRequestBody`). Chat-completions bodies
/// are rejected here; ChatGPT Codex additionally rejects a missing or true
/// `store` field ("Store must be set to false").
fn openai_responses_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    let codex = model.api == "openai-codex-responses";
    let mut input = Vec::new();
    for message in messages {
        if message.role == "toolResult" {
            input.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": content_text(&message.content),
            }));
            continue;
        }
        if message.role == "assistant" {
            let text = content_text(&message.content);
            if !text.is_empty() {
                input.push(serde_json::json!({
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}],
                }));
            }
            for block in &message.content {
                if let MessageContent::ToolCall {
                    id,
                    name,
                    arguments,
                } = block
                {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": id,
                        "name": name,
                        "arguments": arguments.to_string(),
                    }));
                }
            }
            continue;
        }
        let text = content_text(&message.content);
        if text.is_empty() {
            continue;
        }
        input.push(serde_json::json!({
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        }));
    }
    let instructions = system
        .filter(|value| !value.is_empty())
        .unwrap_or("You are a helpful assistant.");
    let mut body = serde_json::json!({
        "model": model.id,
        "store": false,
        "stream": false,
        "instructions": instructions,
        "input": input,
    });
    if codex {
        body["text"] = serde_json::json!({"verbosity": "low"});
        body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
        body["tool_choice"] = Value::String("auto".into());
        body["parallel_tool_calls"] = Value::Bool(true);
    }
    let retention = crate::cache::cache_retention_from_options(options);
    let session_key = options
        .session_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(crate::cache::clamp_openai_prompt_cache_key);
    match model.api.as_str() {
        // azure-openai-responses.ts:293 — clamped key, no retention gate.
        "azure-openai-responses" => {
            if let Some(key) = session_key {
                body["prompt_cache_key"] = Value::String(key);
            }
        }
        // openai-codex-responses.ts:267-268, 557 — key unless retention none.
        "openai-codex-responses" => {
            if retention != crate::cache::CacheRetention::None {
                if let Some(key) = session_key {
                    body["prompt_cache_key"] = Value::String(key);
                }
            }
        }
        // openai-responses.ts:288-296.
        _ => {
            if retention != crate::cache::CacheRetention::None {
                if let Some(key) = session_key {
                    body["prompt_cache_key"] = Value::String(key);
                }
            }
            if retention == crate::cache::CacheRetention::Long
                && crate::cache::supports_long_cache_retention(&model.compat)
            {
                body["prompt_cache_retention"] = Value::String("24h".into());
            }
            let explicit_mode = model
                .compat
                .get("supportsExplicitPromptCacheMode")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if retention == crate::cache::CacheRetention::None && explicit_mode {
                body["prompt_cache_options"] = serde_json::json!({"mode": "explicit"});
            }
        }
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    let mut function = serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    });
                    if resolve_json_schema_strict_sampling(tool).unwrap_or(false) {
                        function["strict"] = Value::Bool(true);
                    }
                    function
                })
                .collect(),
        );
    }
    if model.reasoning {
        if let Some(level) = options
            .thinking_level
            .filter(|level| *level != ThinkingLevel::Off)
        {
            // `thinkingLevelMap` renames or (with an explicit null) drops a level.
            let mapped = match model.thinking_level_map.get(level.as_str()) {
                Some(Some(mapped)) => Some(mapped.clone()),
                Some(None) => None,
                None => Some(level.as_str().to_string()),
            };
            if let Some(effort) = mapped {
                body["reasoning"] = serde_json::json!({
                    "effort": effort,
                    "summary": "auto",
                });
            }
        }
    }
    body
}

fn apply_max_tokens_override(body: &mut Value, options: &StreamOptions) {
    let Some(max_tokens) = options.max_tokens.filter(|value| *value > 0) else {
        return;
    };
    let Value::Object(map) = body else {
        return;
    };
    if map.contains_key("max_tokens") {
        map.insert("max_tokens".into(), Value::from(max_tokens));
    }
    if map.contains_key("max_completion_tokens") {
        map.insert("max_completion_tokens".into(), Value::from(max_tokens));
    }
    if let Some(Value::Object(config)) = map.get_mut("generationConfig") {
        config.insert("maxOutputTokens".into(), Value::from(max_tokens));
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
    let response = if model.api == "openai-codex-responses" {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(&body);
        if compressed {
            request = request.set("content-encoding", "zstd");
        }
        request
            .send_bytes(&bytes)
            .map_err(|err| format!("Provider request failed: {err}"))?
    } else {
        request
            .send_string(&body.to_string())
            .map_err(|err| format!("Provider request failed: {err}"))?
    };
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
    options: &StreamOptions,
) -> Value {
    // Build on the completions shape without inheriting its OpenAI cache
    // fields; mistral-conversations.ts has its own gate.
    let mut body = openai_body(model, messages, system, tools, &StreamOptions::default());
    body["stream"] = Value::Bool(false);
    let retention = crate::cache::cache_retention_from_options(options);
    if retention != crate::cache::CacheRetention::None {
        if let Some(session_id) = options.session_id.as_deref().filter(|id| !id.is_empty()) {
            body["prompt_cache_key"] = Value::String(session_id.to_string());
        }
    }
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
        "openai-codex-responses" => crate::codex::resolve_codex_url(model.base_url.as_deref()),
        "openai-responses" | "azure-openai-responses" => {
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
                    let mut function = serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    });
                    if resolve_json_schema_strict_sampling(tool).unwrap_or(false) {
                        function["strict"] = Value::Bool(true);
                    }
                    serde_json::json!({
                        "type": "function",
                        "function": function,
                    })
                })
                .collect(),
        );
    }
    apply_openai_thinking(&mut body, model, options);

    // openai-completions.ts:805-810.
    let retention = crate::cache::cache_retention_from_options(options);
    let long_supported = retention == crate::cache::CacheRetention::Long
        && crate::cache::completions_supports_long_cache_retention(model);
    let is_openai_host = model
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1")
        .contains("api.openai.com");
    if (is_openai_host && retention != crate::cache::CacheRetention::None) || long_supported {
        if let Some(key) = options
            .session_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(crate::cache::clamp_openai_prompt_cache_key)
        {
            body["prompt_cache_key"] = Value::String(key);
        }
    }
    if long_supported {
        body["prompt_cache_retention"] = Value::String("24h".into());
    }
    // openai-completions.ts getCompatCacheControl + applyAnthropicCacheControl.
    if crate::cache::completions_cache_control_format(model).as_deref() == Some("anthropic") {
        if let Some(cache_control) = crate::cache::anthropic_cache_control(
            &serde_json::json!({
                "supportsLongCacheRetention":
                    crate::cache::completions_supports_long_cache_retention(model),
            }),
            retention,
        ) {
            apply_anthropic_cache_control_to_completions(&mut body, &cache_control);
        }
    }
    body
}

/// TS openai-completions.ts `applyAnthropicCacheControl`: mark the system
/// prompt, the last tool definition, and the last user/assistant/tool message
/// with Anthropic-style `cache_control` for `cacheControlFormat: "anthropic"`
/// providers.
fn apply_anthropic_cache_control_to_completions(body: &mut Value, cache_control: &Value) {
    fn mark_text_content(message: &mut Value, cache_control: &Value) -> bool {
        match message.get_mut("content") {
            Some(content) if content.is_string() => {
                let text = content.as_str().unwrap_or_default().to_string();
                if text.is_empty() {
                    return false;
                }
                *content = serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": cache_control,
                }]);
                true
            }
            Some(Value::Array(parts)) => {
                for part in parts.iter_mut().rev() {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        part["cache_control"] = cache_control.clone();
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages.iter_mut() {
            let role = message.get("role").and_then(Value::as_str);
            if role == Some("system") || role == Some("developer") {
                mark_text_content(message, cache_control);
                break;
            }
        }
        for message in messages.iter_mut().rev() {
            let role = message.get("role").and_then(Value::as_str);
            if (role == Some("user") || role == Some("assistant") || role == Some("tool"))
                && mark_text_content(message, cache_control)
            {
                break;
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = cache_control.clone();
        }
    }
}

/// TS `resolveJsonSchemaStrictSampling` — attach `strict: true` when the tool
/// asks for JSON-schema constrained sampling and the schema is an object.
pub fn resolve_json_schema_strict_sampling(tool: &ToolSpec) -> Option<bool> {
    let config = tool.constrained_sampling.as_ref()?;
    if config.get("type").and_then(Value::as_str) != Some("json_schema") {
        return None;
    }
    let require = config.get("strict").and_then(Value::as_str) == Some("require");
    let is_object = tool.parameters.get("type").and_then(Value::as_str) == Some("object")
        || tool.parameters.get("properties").is_some();
    if is_object {
        return Some(true);
    }
    if require {
        return None;
    }
    None
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
    let cache_control = crate::cache::anthropic_cache_control(
        &model.compat,
        crate::cache::cache_retention_from_options(options),
    );
    let mut converted: Vec<Value> = messages
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
    if let Some(cache_control) = cache_control.as_ref() {
        if let Some(last) = converted.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some("user") {
                match last.get_mut("content") {
                    Some(Value::Array(blocks)) => {
                        if let Some(block) = blocks.last_mut() {
                            block["cache_control"] = cache_control.clone();
                        }
                    }
                    Some(content) if content.is_string() => {
                        let text = content.as_str().unwrap_or_default().to_string();
                        *content = serde_json::json!([{
                            "type": "text",
                            "text": text,
                            "cache_control": cache_control,
                        }]);
                    }
                    _ => {}
                }
            }
        }
    }
    let mut body = serde_json::json!({
        "model": model.id,
        "max_tokens": model.max_tokens.min(8192),
        "messages": converted,
        "stream": false,
    });
    if let Some(system) = system {
        let mut block = serde_json::json!({"type": "text", "text": system});
        if let Some(cache_control) = cache_control.as_ref() {
            block["cache_control"] = cache_control.clone();
        }
        body["system"] = Value::Array(vec![block]);
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
    if let Some(cache_control) = cache_control
        .as_ref()
        .filter(|_| crate::cache::supports_cache_control_on_tools(&model.compat))
    {
        if let Some(tools_array) = body.get_mut("tools").and_then(Value::as_array_mut) {
            if let Some(last) = tools_array.last_mut() {
                last["cache_control"] = cache_control.clone();
            }
        }
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

    #[test]
    fn live_complete_retries_429_with_retry_after() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        use crate::auth::ResolvedAuth;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicU32::new(0));
        let server_hits = hits.clone();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                // Drain the full request (headers + Content-Length body) before
                // responding; closing with unread data pending RSTs the socket
                // on Windows and the client sees 10054 instead of the status.
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    let n = match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break request.len(),
                        Ok(n) => n,
                    };
                    request.extend_from_slice(&buf[..n]);
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let content_length = String::from_utf8_lossy(&request[..header_end])
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while request.len() < header_end + content_length {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let n = server_hits.fetch_add(1, Ordering::SeqCst);
                let (status, body) = if n == 0 {
                    ("429 Too Many Requests", r#"{"error":"rate limited"}"#)
                } else {
                    ("200 OK", r#"{"choices":[{"message":{"content":"ok"}}]}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nretry-after-ms: 1\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api.contains("openai"))
            .expect("openai");
        model.base_url = Some(format!("http://{addr}"));
        let auth = ResolvedAuth {
            api_key: Some("test".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let message = live_complete_with(
            &model,
            &[ChatMessage::text("user", "hi")],
            &auth,
            None,
            &[],
            &StreamOptions {
                max_retries: Some(1),
                max_retry_delay_ms: Some(1000),
                ..StreamOptions::default()
            },
        )
        .unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(
            content_text(&assistant_to_chat(&message).content).contains("ok")
                || message.content.iter().any(
                    |block| matches!(block, ContentBlock::Text { text } if text.contains("ok"))
                )
        );
    }

    #[test]
    fn anthropic_body_places_cache_control_breakpoints() {
        let anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        let messages = vec![
            ChatMessage::text("user", "first"),
            ChatMessage {
                role: "assistant".into(),
                content: vec![MessageContent::Text {
                    text: "reply".into(),
                }],
                ..ChatMessage::default()
            },
            ChatMessage::text("user", "second"),
        ];
        let tools = vec![
            ToolSpec {
                name: "read".into(),
                description: "read".into(),
                parameters: serde_json::json!({"type":"object"}),
                constrained_sampling: None,
            },
            ToolSpec {
                name: "write".into(),
                description: "write".into(),
                parameters: serde_json::json!({"type":"object"}),
                constrained_sampling: None,
            },
        ];
        let options = StreamOptions {
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        };
        let body = request_body_with(&anthropic, &messages, Some("sys"), &tools, &options);

        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "sys");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["text"], "second");
        assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][0]["content"], "first");

        let long = request_body_with(
            &anthropic,
            &messages,
            Some("sys"),
            &tools,
            &StreamOptions {
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(long["system"][0]["cache_control"]["ttl"], "1h");

        let none = request_body_with(
            &anthropic,
            &messages,
            Some("sys"),
            &tools,
            &StreamOptions {
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(none["system"][0]["text"], "sys");
        assert!(none["system"][0].get("cache_control").is_none());
        assert!(none["tools"][1].get("cache_control").is_none());
        let none_last = none["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(none_last["content"], "second");
    }

    #[test]
    fn anthropic_body_marks_tool_result_content_block() {
        let anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        let messages = vec![ChatMessage {
            role: "toolResult".into(),
            tool_call_id: Some("t1".into()),
            content: vec![MessageContent::Text { text: "out".into() }],
            ..ChatMessage::default()
        }];
        let body = request_body_with(
            &anthropic,
            &messages,
            None,
            &[],
            &StreamOptions {
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["type"], "tool_result");
        assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_body_respects_tool_cache_compat() {
        let mut anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        anthropic.compat = serde_json::json!({"supportsCacheControlOnTools": false});
        let tools = vec![ToolSpec {
            name: "read".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type":"object"}),
            constrained_sampling: None,
        }];
        let body = request_body_with(
            &anthropic,
            &[],
            None,
            &tools,
            &StreamOptions {
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );
        assert!(body["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn responses_bodies_carry_prompt_cache_key() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-responses")
            .expect("openai responses model");
        let session = StreamOptions {
            session_id: Some("sess-1234".into()),
            ..StreamOptions::default()
        };
        let body = request_body_with(&model, &[], None, &[], &session);
        assert_eq!(body["prompt_cache_key"], "sess-1234");
        assert!(body.get("prompt_cache_retention").is_none());
        assert!(body.get("prompt_cache_options").is_none());

        let long = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-1234".into()),
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(long["prompt_cache_retention"], "24h");

        let none = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-1234".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert!(none.get("prompt_cache_key").is_none());
        assert!(none.get("prompt_cache_options").is_none());
        model.compat = serde_json::json!({"supportsExplicitPromptCacheMode": true});
        let explicit = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-1234".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(explicit["prompt_cache_options"]["mode"], "explicit");

        let clamped = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("k".repeat(80)),
                ..StreamOptions::default()
            },
        );
        assert_eq!(
            clamped["prompt_cache_key"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            64
        );

        let anonymous = request_body_with(&model, &[], None, &[], &StreamOptions::default());
        assert!(anonymous.get("prompt_cache_key").is_none());
    }

    #[test]
    fn codex_and_azure_bodies_carry_prompt_cache_key() {
        let codex = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-codex-responses")
            .expect("codex model");
        let body = request_body_with(
            &codex,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-codex".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(body["prompt_cache_key"], "sess-codex");
        assert!(body.get("prompt_cache_retention").is_none());
        let none = request_body_with(
            &codex,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-codex".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert!(none.get("prompt_cache_key").is_none());

        let mut azure = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "azure-openai-responses")
            .unwrap_or_else(|| codex.clone());
        azure.api = "azure-openai-responses".into();
        let azure_body = request_body_with(
            &azure,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-azure".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(azure_body["prompt_cache_key"], "sess-azure");
        assert!(azure_body.get("prompt_cache_retention").is_none());
    }

    #[test]
    fn completions_body_carries_prompt_cache_key_for_openai() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-completions")
            .expect("openai completions shape");
        model.provider = "openai".into();
        model.base_url = Some("https://api.openai.com/v1".into());
        model.compat = serde_json::Value::Null;
        let body = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-c".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(body["prompt_cache_key"], "sess-c");
        assert!(body.get("prompt_cache_retention").is_none());
        let long = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-c".into()),
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(long["prompt_cache_retention"], "24h");
        let none = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-c".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert!(none.get("prompt_cache_key").is_none());
        assert!(none.get("prompt_cache_retention").is_none());
    }

    #[test]
    fn completions_body_skips_key_for_non_openai_hosts_on_short() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-completions")
            .expect("completions model");
        model.provider = "custom".into();
        model.base_url = Some("https://api.example.com/v1".into());
        model.compat = serde_json::Value::Null;
        let short = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-x".into()),
                ..StreamOptions::default()
            },
        );
        assert!(short.get("prompt_cache_key").is_none());
        let long = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-x".into()),
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(long["prompt_cache_key"], "sess-x");
        assert_eq!(long["prompt_cache_retention"], "24h");
    }

    #[test]
    fn completions_body_applies_anthropic_markers_for_openrouter_claude() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-completions")
            .expect("completions model");
        model.provider = "openrouter".into();
        model.id = "anthropic/claude-sonnet-5".into();
        model.base_url = Some("https://openrouter.ai/api/v1".into());
        model.compat = serde_json::Value::Null;
        let tools = vec![ToolSpec {
            name: "read".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type":"object"}),
            constrained_sampling: None,
        }];
        let messages = vec![ChatMessage::text("user", "hello")];
        let body = request_body_with(
            &model,
            &messages,
            Some("sys"),
            &tools,
            &StreamOptions {
                session_id: Some("sess-or".into()),
                ..StreamOptions::default()
            },
        );
        let system = &body["messages"][0];
        assert_eq!(system["role"], "system");
        assert_eq!(system["content"][0]["text"], "sys");
        assert_eq!(system["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn mistral_body_carries_prompt_cache_key() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "mistral-conversations")
            .unwrap_or_else(|| {
                load_builtin_models()
                    .into_iter()
                    .find(|m| m.api == "openai-completions")
                    .expect("base model")
            });
        model.api = "mistral-conversations".into();
        let body = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-m".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(body["prompt_cache_key"], "sess-m");
        let none = request_body_with(
            &model,
            &[],
            None,
            &[],
            &StreamOptions {
                session_id: Some("sess-m".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert!(none.get("prompt_cache_key").is_none());
    }

    #[test]
    fn openai_tools_attach_strict_when_json_schema_sampling() {
        let mut openai = load_builtin_models()
            .into_iter()
            .find(|m| m.api.contains("openai"))
            .expect("openai");
        openai.api = "openai-completions".into();
        let tools = [ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            constrained_sampling: Some(serde_json::json!({"type":"json_schema","strict":"prefer"})),
        }];
        let body = request_body_with(&openai, &[], None, &tools, &StreamOptions::default());
        assert_eq!(body["tools"][0]["function"]["strict"], true);
        assert_eq!(resolve_json_schema_strict_sampling(&tools[0]), Some(true));
        let off = ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: tools[0].parameters.clone(),
            constrained_sampling: None,
        };
        assert_eq!(resolve_json_schema_strict_sampling(&off), None);
    }
}
