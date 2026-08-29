//! Stream/complete lifecycle. Tests inject fixture SSE/HTTP — never live providers.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::events::{AssistantMessage, AssistantMessageEvent, ModelRef, StopReason};
use crate::http::{post_sse, LiveRequest};
use crate::request::{resolve_api, RequestContext};
use crate::types::{ContentBlock, Message, ToolCall, Usage};

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub provider: String,
    pub model: String,
    pub api: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<serde_json::Value>,
    pub fixture: Option<FixtureResponse>,
    pub extra_headers: Vec<(String, String)>,
    pub allow_network: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureResponse {
    pub events: Vec<AssistantMessageEvent>,
    #[serde(default)]
    pub sse: Option<String>,
}

pub type StreamEvent = AssistantMessageEvent;

fn empty_partial(provider: &str, model: &str) -> AssistantMessage {
    AssistantMessage {
        id: "msg_fixture".into(),
        role: "assistant".into(),
        content: vec![],
        model: Some(ModelRef {
            provider: provider.into(),
            id: model.into(),
        }),
        stop_reason: None,
        usage: None,
        error_message: None,
        timestamp: 1,
    }
}

pub fn parse_sse_fixture(body: &str, provider: &str, model: &str) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();
    let mut partial = empty_partial(provider, model);
    events.push(AssistantMessageEvent::Start {
        partial: partial.clone(),
    });
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_args = String::new();
    let mut tool_name = String::new();
    let mut tool_id = String::new();
    let mut text_started = false;
    let mut thinking_started = false;
    let mut tool_started = false;

    for line in body.lines() {
        let line = line.trim();
        let data = if let Some(data) = line.strip_prefix("data: ") {
            data
        } else if line.starts_with('{') {
            line
        } else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };

        if let Some(delta) = openai_text_delta(&value)
            .or_else(|| anthropic_text_delta(&value))
            .or_else(|| google_text_delta(&value))
            .or_else(|| bedrock_text_delta(&value))
            .or_else(|| mistral_text_delta(&value))
            .or_else(|| responses_text_delta(&value))
        {
            if !text_started {
                events.push(AssistantMessageEvent::TextStart {
                    content_index: 0,
                    partial: partial.clone(),
                });
                text_started = true;
            }
            text.push_str(&delta);
            partial.content = vec![ContentBlock::Text { text: text.clone() }];
            events.push(AssistantMessageEvent::TextDelta {
                content_index: 0,
                delta,
                partial: partial.clone(),
            });
        }

        if let Some(delta) =
            anthropic_thinking_delta(&value).or_else(|| google_thinking_delta(&value))
        {
            if !thinking_started {
                events.push(AssistantMessageEvent::ThinkingStart {
                    content_index: 1,
                    partial: partial.clone(),
                });
                thinking_started = true;
            }
            thinking.push_str(&delta);
            events.push(AssistantMessageEvent::ThinkingDelta {
                content_index: 1,
                delta,
                partial: partial.clone(),
            });
        }

        if let Some((id, name, args)) =
            openai_tool_delta(&value).or_else(|| anthropic_tool_delta(&value))
        {
            if !tool_started {
                events.push(AssistantMessageEvent::ToolcallStart {
                    content_index: 2,
                    partial: partial.clone(),
                });
                tool_started = true;
            }
            if !id.is_empty() {
                tool_id = id;
            }
            if !name.is_empty() {
                tool_name = name;
            }
            tool_args.push_str(&args);
            events.push(AssistantMessageEvent::ToolcallDelta {
                content_index: 2,
                delta: args,
                partial: partial.clone(),
            });
        }

        if let Some(usage) = parse_usage(&value) {
            partial.usage = Some(usage);
        }
    }

    if text.is_empty() && !body.trim().is_empty() && !body.contains("data:") && !body.contains('{')
    {
        text = body.to_string();
        events.push(AssistantMessageEvent::TextStart {
            content_index: 0,
            partial: partial.clone(),
        });
        partial.content = vec![ContentBlock::Text { text: text.clone() }];
        events.push(AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: text.clone(),
            partial: partial.clone(),
        });
        text_started = true;
    }

    if text_started {
        events.push(AssistantMessageEvent::TextEnd {
            content_index: 0,
            content: text.clone(),
            partial: partial.clone(),
        });
    }
    if thinking_started {
        events.push(AssistantMessageEvent::ThinkingEnd {
            content_index: 1,
            content: thinking,
            partial: partial.clone(),
        });
    }
    if tool_started {
        let parsed_args = serde_json::from_str(&tool_args).unwrap_or(serde_json::json!({}));
        let tool_call = ToolCall {
            id: tool_id.clone(),
            name: tool_name.clone(),
            arguments: parsed_args.clone(),
        };
        partial.content.push(ContentBlock::ToolCall {
            tool_call_id: tool_id,
            tool_name,
            input: parsed_args,
        });
        events.push(AssistantMessageEvent::ToolcallEnd {
            content_index: 2,
            tool_call,
            partial: partial.clone(),
        });
    }
    if !text.is_empty() {
        partial
            .content
            .insert(0, ContentBlock::Text { text: text.clone() });
        partial.content.dedup();
    }
    let reason = if tool_started {
        StopReason::ToolUse
    } else {
        StopReason::Stop
    };
    partial.stop_reason = Some(reason);
    if partial.usage.is_none() {
        partial.usage = Some(Usage {
            input: 1,
            output: text.split_whitespace().count() as u64,
            total_tokens: 1,
            ..Usage::default()
        });
    }
    events.push(AssistantMessageEvent::Done {
        reason,
        message: partial,
    });
    events
}

fn bedrock_text_delta(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/contentBlockDelta/delta/text")
        .or_else(|| value.pointer("/delta/text"))
        .and_then(|v| v.as_str())
        .filter(|_| {
            value.get("contentBlockDelta").is_some()
                || value.get("contentBlockStart").is_some()
                || value.pointer("/delta/text").is_some()
                    && value.get("type").and_then(|v| v.as_str()) == Some("contentBlockDelta")
        })
        .map(str::to_string)
        .or_else(|| {
            value
                .pointer("/contentBlockDelta/delta/text")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn mistral_text_delta(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/choices/0/delta/content")
        .or_else(|| value.pointer("/data/choices/0/delta/content"))
        .or_else(|| value.pointer("/output/content"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn responses_text_delta(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) == Some("response.output_text.delta")
        || value.get("type").and_then(|v| v.as_str()) == Some("response.output_text.delta.delta")
    {
        return value
            .get("delta")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    value
        .pointer("/response/output_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn openai_text_delta(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn anthropic_text_delta(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
        && value.pointer("/delta/type").and_then(|v| v.as_str()) == Some("text_delta")
    {
        return value
            .pointer("/delta/text")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    value
        .pointer("/delta/text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn google_text_delta(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .filter(|_| {
            value
                .pointer("/candidates/0/content/parts/0/thought")
                .and_then(|v| v.as_bool())
                != Some(true)
        })
        .map(str::to_string)
}

fn anthropic_thinking_delta(value: &serde_json::Value) -> Option<String> {
    if value.pointer("/delta/type").and_then(|v| v.as_str()) == Some("thinking_delta") {
        value
            .pointer("/delta/thinking")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        None
    }
}

fn google_thinking_delta(value: &serde_json::Value) -> Option<String> {
    if value
        .pointer("/candidates/0/content/parts/0/thought")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        value
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        None
    }
}

fn openai_tool_delta(value: &serde_json::Value) -> Option<(String, String, String)> {
    let call = value.pointer("/choices/0/delta/tool_calls/0")?;
    Some((
        call.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        call.pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        call.pointer("/function/arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    ))
}

fn anthropic_tool_delta(value: &serde_json::Value) -> Option<(String, String, String)> {
    if value.get("type").and_then(|v| v.as_str()) == Some("content_block_start")
        && value
            .pointer("/content_block/type")
            .and_then(|v| v.as_str())
            == Some("tool_use")
    {
        return Some((
            value
                .pointer("/content_block/id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            value
                .pointer("/content_block/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            String::new(),
        ));
    }
    if value.pointer("/delta/type").and_then(|v| v.as_str()) == Some("input_json_delta") {
        return Some((
            String::new(),
            String::new(),
            value
                .pointer("/delta/partial_json")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ));
    }
    None
}

fn parse_usage(value: &serde_json::Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    Some(Usage {
        input: usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output: usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_read: usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_write: usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        total_tokens: usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        ..Usage::default()
    })
}

pub fn stream_complete(options: &StreamOptions) -> Result<Vec<AssistantMessageEvent>, StreamError> {
    if let Some(fixture) = &options.fixture {
        if !fixture.events.is_empty() {
            return Ok(fixture.events.clone());
        }
        if let Some(sse) = &fixture.sse {
            return Ok(parse_sse_fixture(sse, &options.provider, &options.model));
        }
    }
    if options.allow_network {
        let api_key = options
            .api_key
            .clone()
            .or_else(|| crate::get_env_api_key(&options.provider));
        let Some(api_key) = api_key else {
            return Err(StreamError::Message(format!(
                "No API key for provider: {}",
                options.provider
            )));
        };
        let api = options
            .api
            .clone()
            .unwrap_or_else(|| resolve_api(None, &options.provider));
        let raw = post_sse(&LiveRequest {
            provider: options.provider.clone(),
            api,
            model: options.model.clone(),
            base_url: options.base_url.clone(),
            api_key,
            context: RequestContext {
                system: options.system.clone(),
                messages: options.messages.clone(),
                tools: options.tools.clone(),
                max_tokens: 16_384,
                stream: true,
            },
            extra_headers: options.extra_headers.clone(),
        })?;
        return Ok(parse_sse_fixture(&raw, &options.provider, &options.model));
    }
    Err(StreamError::Message(
        "No fixture provided; live provider calls require credentials and allow_network".into(),
    ))
}

pub fn complete(options: &StreamOptions) -> Result<AssistantMessage, StreamError> {
    let events = stream_complete(options)?;
    for event in events.into_iter().rev() {
        match event {
            AssistantMessageEvent::Done { message, .. } => return Ok(message),
            AssistantMessageEvent::Error { error, .. } => return Ok(error),
            _ => {}
        }
    }
    Err(StreamError::Message(
        "Stream ended without a terminal event".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_fixture_emits_start_delta_done() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n";
        let events = parse_sse_fixture(sse, "openai", "gpt-4o");
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { .. })));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
    }

    #[test]
    fn anthropic_and_google_sse_corpora() {
        let anthropic = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Ok\"}}\n";
        let events = parse_sse_fixture(anthropic, "anthropic", "claude-sonnet-4-5");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { delta, .. } if delta == "Ok")));
        let google = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Go\"}]}}]}\n";
        let events = parse_sse_fixture(google, "google", "gemini-2.5-flash");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { delta, .. } if delta == "Go")));
    }

    #[test]
    fn bedrock_mistral_codex_sse_corpora() {
        let bedrock = r#"{"contentBlockDelta":{"delta":{"text":"Hi"},"contentBlockIndex":0}}"#;
        let events = parse_sse_fixture(bedrock, "amazon-bedrock", "claude");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { delta, .. } if delta == "Hi")));
        let mistral = "data: {\"choices\":[{\"delta\":{\"content\":\"Yo\"}}]}\n";
        let events = parse_sse_fixture(mistral, "mistral", "devstral");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { delta, .. } if delta == "Yo")));
        let codex = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Ok\"}\n";
        let events = parse_sse_fixture(codex, "openai-codex", "gpt-5.5");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { delta, .. } if delta == "Ok")));
    }

    #[test]
    fn missing_key_uses_ts_error_string() {
        let err = stream_complete(&StreamOptions {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            allow_network: true,
            ..StreamOptions::default()
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "No API key for provider: openai");
    }
}
