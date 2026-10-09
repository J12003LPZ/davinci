//! Decoder for the `pi-messages` wire protocol (Radius gateway and custom
//! backends), mirroring `createEventConverter` in
//! `vendor/davinci/packages/ai/src/api/pi-messages.ts`. The backend streams
//! serialized assistant-message events (`text_delta`, `toolcall_end`, ...)
//! addressed by `contentIndex`, then a terminal `done` or `error`.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::Model;
use crate::stream::{
    AssistantMessage, AssistantMessageEvent, ContentBlock, PartialMessageSnapshot, StopReason,
};
use crate::stream_decoder::{new_message, StreamDecoder};

pub struct PiMessagesDecoder {
    message: AssistantMessage,
    snapshot: PartialMessageSnapshot,
    /// Backend `contentIndex` -> position in `message.content`.
    slots: HashMap<u64, usize>,
    tool_json: HashMap<u64, String>,
    started: bool,
    done: bool,
}

impl PiMessagesDecoder {
    pub fn new(model: &Model) -> Self {
        let message = new_message(model);
        Self {
            snapshot: PartialMessageSnapshot::new(&message),
            message,
            slots: HashMap::new(),
            tool_json: HashMap::new(),
            started: false,
            done: false,
        }
    }

    fn start(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        if !self.started {
            self.started = true;
            out.push(AssistantMessageEvent::Start {
                partial: self.snapshot.force(&self.message),
            });
        }
    }

    fn open(&mut self, index: u64, block: ContentBlock) -> usize {
        let position = self.message.content.len();
        self.message.content.push(block);
        self.slots.insert(index, position);
        position
    }

    fn slot(&self, index: u64) -> Option<usize> {
        self.slots.get(&index).copied()
    }

    fn finish_with(
        &mut self,
        reason: StopReason,
        event: &Value,
        out: &mut Vec<AssistantMessageEvent>,
    ) {
        self.done = true;
        if let Some(usage) = event
            .get("usage")
            .and_then(|usage| serde_json::from_value(usage.clone()).ok())
        {
            self.message.usage = Some(usage);
        }
        if let Some(id) = event.get("responseId").and_then(Value::as_str) {
            self.message
                .extra
                .insert("responseId".into(), Value::String(id.into()));
        }
        self.message.stop_reason = Some(reason);
        if matches!(reason, StopReason::Error | StopReason::Aborted) {
            self.message.error_message = Some(
                event
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or("pi-messages backend reported an error")
                    .to_string(),
            );
            out.push(AssistantMessageEvent::Error {
                reason,
                error: self.message.clone(),
            });
        } else {
            out.push(AssistantMessageEvent::Done {
                reason,
                message: self.message.clone(),
            });
        }
    }

    fn fail(&mut self, message: &str, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        self.start(out);
        self.done = true;
        self.message.stop_reason = Some(StopReason::Error);
        self.message.error_message = Some(message.into());
        out.push(AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error: self.message.clone(),
        });
    }
}

impl StreamDecoder for PiMessagesDecoder {
    fn feed(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if kind.is_empty() {
            return;
        }
        self.start(out);
        let index = event
            .get("contentIndex")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let text = |key: &str| {
            event
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match kind {
            "start" => {}
            "text_start" => {
                let content_index = self.open(
                    index,
                    ContentBlock::Text {
                        text: String::new(),
                    },
                );
                out.push(AssistantMessageEvent::TextStart {
                    content_index,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "text_delta" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let delta = text("delta");
                if let Some(ContentBlock::Text { text }) =
                    self.message.content.get_mut(content_index)
                {
                    text.push_str(&delta);
                }
                out.push(AssistantMessageEvent::TextDelta {
                    content_index,
                    partial: self.snapshot.update(&self.message, delta.len()),
                    delta,
                });
            }
            "text_end" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let content = text("content");
                if let Some(ContentBlock::Text { text }) =
                    self.message.content.get_mut(content_index)
                {
                    *text = content.clone();
                }
                out.push(AssistantMessageEvent::TextEnd {
                    content_index,
                    content,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "thinking_start" => {
                let content_index = self.open(
                    index,
                    ContentBlock::Thinking {
                        thinking: String::new(),
                        signature: None,
                        redacted: false,
                    },
                );
                out.push(AssistantMessageEvent::ThinkingStart {
                    content_index,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "thinking_delta" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let delta = text("delta");
                if let Some(ContentBlock::Thinking { thinking, .. }) =
                    self.message.content.get_mut(content_index)
                {
                    thinking.push_str(&delta);
                }
                out.push(AssistantMessageEvent::ThinkingDelta {
                    content_index,
                    partial: self.snapshot.update(&self.message, delta.len()),
                    delta,
                });
            }
            "thinking_end" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let content = text("content");
                if let Some(ContentBlock::Thinking {
                    thinking,
                    signature,
                    redacted,
                }) = self.message.content.get_mut(content_index)
                {
                    *thinking = content.clone();
                    *signature = event
                        .get("contentSignature")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    *redacted = event
                        .get("redacted")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
                out.push(AssistantMessageEvent::ThinkingEnd {
                    content_index,
                    content,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "toolcall_start" => {
                let content_index = self.open(
                    index,
                    ContentBlock::ToolCall {
                        id: text("id"),
                        name: text("toolName"),
                        arguments: Value::Object(Default::default()),
                    },
                );
                self.tool_json.insert(index, String::new());
                out.push(AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "toolcall_delta" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let delta = text("delta");
                self.tool_json.entry(index).or_default().push_str(&delta);
                out.push(AssistantMessageEvent::ToolcallDelta {
                    content_index,
                    partial: self.snapshot.update(&self.message, delta.len()),
                    delta,
                });
            }
            "toolcall_end" => {
                let Some(content_index) = self.slot(index) else {
                    return;
                };
                let streamed = self.tool_json.remove(&index).unwrap_or_default();
                let call = event.get("toolCall");
                let arguments = match call.and_then(|call| call.get("arguments")) {
                    Some(arguments @ Value::Object(_)) => arguments.clone(),
                    // Same rule as the other decoders: malformed arguments
                    // keep the sentinel and never execute.
                    Some(other) => crate::invalid_arguments(&other.to_string()),
                    None => crate::final_tool_arguments(&streamed),
                };
                if let Some(ContentBlock::ToolCall {
                    id,
                    name,
                    arguments: slot,
                }) = self.message.content.get_mut(content_index)
                {
                    if let Some(value) = call.and_then(|c| c.get("id")).and_then(Value::as_str) {
                        *id = value.into();
                    }
                    if let Some(value) = call.and_then(|c| c.get("name")).and_then(Value::as_str) {
                        *name = value.into();
                    }
                    *slot = arguments;
                }
                let tool_call = self.message.content[content_index].clone();
                out.push(AssistantMessageEvent::ToolcallEnd {
                    content_index,
                    tool_call,
                    partial: self.snapshot.force(&self.message),
                });
            }
            "done" => {
                let reason = match event.get("reason").and_then(Value::as_str) {
                    Some("length") => StopReason::Length,
                    Some("toolUse") => StopReason::ToolUse,
                    _ => StopReason::Stop,
                };
                self.finish_with(reason, event, out);
            }
            "error" => {
                let reason = match event.get("reason").and_then(Value::as_str) {
                    Some("aborted") => StopReason::Aborted,
                    _ => StopReason::Error,
                };
                self.finish_with(reason, event, out);
            }
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage {
        if !self.done {
            self.fail(crate::stream_decoder::TRUNCATED_STREAM, out);
        }
        self.message.clone()
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        let mut model = crate::catalog::load_builtin_models()
            .into_iter()
            .next()
            .expect("a built-in model");
        model.api = "pi-messages".into();
        model.provider = "radius".into();
        model.id = "audit-radius".into();
        model
    }

    fn run(events: &[Value]) -> (AssistantMessage, Vec<AssistantMessageEvent>) {
        let mut decoder = PiMessagesDecoder::new(&model());
        let mut out = Vec::new();
        for event in events {
            decoder.feed(event, &mut out);
        }
        (decoder.finish(&mut out), out)
    }

    #[test]
    fn text_and_tool_calls_round_trip_to_done() {
        let (message, events) = run(&[
            serde_json::json!({"type":"start"}),
            serde_json::json!({"type":"text_start","contentIndex":0}),
            serde_json::json!({"type":"text_delta","contentIndex":0,"delta":"Hel"}),
            serde_json::json!({"type":"text_delta","contentIndex":0,"delta":"lo"}),
            serde_json::json!({"type":"text_end","contentIndex":0,"content":"Hello"}),
            serde_json::json!({"type":"toolcall_start","contentIndex":1,"id":"c1","toolName":"read"}),
            serde_json::json!({"type":"toolcall_delta","contentIndex":1,"delta":"{\"path\":"}),
            serde_json::json!({"type":"toolcall_delta","contentIndex":1,"delta":"\"a\"}"}),
            serde_json::json!({"type":"toolcall_end","contentIndex":1,"toolCall":{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a"}}}),
            serde_json::json!({"type":"done","reason":"toolUse","usage":{"input":3,"output":4,"cacheRead":0,"cacheWrite":0,"totalTokens":7,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}),
        ]);
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(
            message.content[0],
            ContentBlock::Text {
                text: "Hello".into()
            }
        );
        assert_eq!(
            message.content[1],
            ContentBlock::ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: serde_json::json!({"path":"a"}),
            }
        );
        assert_eq!(
            message.usage.as_ref().map(|usage| usage.total_tokens),
            Some(7)
        );
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
    }

    #[test]
    fn backend_errors_and_truncation_are_errors() {
        let (message, _) =
            run(&[serde_json::json!({"type":"error","reason":"error","errorMessage":"quota"})]);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(message.error_message.as_deref(), Some("quota"));
        let (message, _) = run(&[serde_json::json!({"type":"text_start","contentIndex":0})]);
        assert_eq!(
            message.error_message.as_deref(),
            Some(crate::stream_decoder::TRUNCATED_STREAM)
        );
    }
}
