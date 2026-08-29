//! Stream/complete lifecycle. Tests inject fixture SSE/HTTP — never live providers.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::events::{AssistantMessage, AssistantMessageEvent, ModelRef, StopReason};
use crate::types::{ContentBlock, Usage};

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct StreamOptions {
    pub provider: String,
    pub model: String,
    pub fixture: Option<FixtureResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureResponse {
    pub events: Vec<AssistantMessageEvent>,
    #[serde(default)]
    pub sse: Option<String>,
}

pub type StreamEvent = AssistantMessageEvent;

pub fn parse_sse_fixture(body: &str, provider: &str, model: &str) -> Vec<AssistantMessageEvent> {
    let mut text = String::new();
    let mut events = Vec::new();
    let now = 1;
    let mut partial = AssistantMessage {
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
        timestamp: now,
    };
    events.push(AssistantMessageEvent::Start {
        partial: partial.clone(),
    });
    events.push(AssistantMessageEvent::TextStart {
        content_index: 0,
        partial: partial.clone(),
    });
    for line in body.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(delta) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(|v| v.as_str())
                    .or_else(|| value.pointer("/delta/text").and_then(|v| v.as_str()))
                    .or_else(|| {
                        value
                            .pointer("/candidates/0/content/parts/0/text")
                            .and_then(|v| v.as_str())
                    })
                {
                    text.push_str(delta);
                    partial.content = vec![ContentBlock::Text { text: text.clone() }];
                    events.push(AssistantMessageEvent::TextDelta {
                        content_index: 0,
                        delta: delta.to_string(),
                        partial: partial.clone(),
                    });
                }
            }
        }
    }
    if text.is_empty() && !body.trim().is_empty() && !body.contains("data:") {
        text = body.to_string();
        partial.content = vec![ContentBlock::Text { text: text.clone() }];
        events.push(AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: text.clone(),
            partial: partial.clone(),
        });
    }
    events.push(AssistantMessageEvent::TextEnd {
        content_index: 0,
        content: text.clone(),
        partial: partial.clone(),
    });
    let mut message = partial;
    message.stop_reason = Some(StopReason::Stop);
    message.usage = Some(Usage {
        input: 1,
        output: text.split_whitespace().count() as u64,
        total_tokens: 1,
        ..Usage::default()
    });
    events.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message,
    });
    events
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
    Err(StreamError::Message(
        "No fixture provided; live provider calls are disabled in tests and default builds".into(),
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
}
