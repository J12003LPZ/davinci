//! Incremental decoder for the Anthropic Messages API SSE stream
//! (`"api": "anthropic-messages"`).
//!
//! Mirrors the event loop of `streamAnthropic` in
//! `vendor/pi/packages/ai/src/api/anthropic-messages.ts` (`message_start`,
//! `content_block_start`, `content_block_delta`, `content_block_stop`,
//! `message_delta`, `message_stop`, `error`) and its `mapStopReason`. Every
//! event is routed by the `type` field of its `data` JSON; the SSE `event:`
//! name duplicates it and is ignored.
//!
//! Deliberate departures from the TypeScript, each noted at the site:
//! - a stream that ends before `message_stop` keeps the text it received (TS
//!   throws "Anthropic stream ended before message_stop") and errors only
//!   when a tool call was cut off, the rule `ResponsesDecoder::finish` uses;
//! - streamed tool arguments are re-parsed only once the buffer is a complete
//!   JSON object (TS repairs and partially parses the fragment);
//! - thinking signatures and redacted-thinking payloads are dropped because
//!   `ContentBlock::Thinking` has nowhere to keep them;
//! - a provider `error` event carries `error.message` (TS throws the raw
//!   `data` string).

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::catalog::{Model, ModelCost};
use crate::stream::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};
use crate::stream_decoder::{new_message, StreamDecoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    ToolCall,
}

/// An open content block. Anthropic keys its events by `index`; the block's
/// position in `AssistantMessage::content` is what our events report.
#[derive(Debug, Clone)]
struct Block {
    kind: BlockKind,
    content_index: usize,
    /// The tool input JSON as it streams in through `input_json_delta` (TS
    /// `partialJson`); empty for text and thinking blocks.
    partial_json: String,
}

/// Token counts as reported so far. `message_start` seeds every field and
/// `message_delta` overwrites only the fields it carries, so a proxy that
/// omits `input_tokens` from the final delta cannot zero it (TS lines
/// 740-760).
#[derive(Debug, Clone, Copy, Default)]
struct TokenCounts {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    reasoning: Option<u64>,
}

/// Decoder for the Anthropic Messages API stream.
pub struct AnthropicDecoder {
    model: Model,
    /// The model whose cost table prices the usage: the requested model, or
    /// the allowed fallback the response says it was served by.
    usage_model: Model,
    message: AssistantMessage,
    blocks: HashMap<u64, Block>,
    counts: TokenCounts,
    started: bool,
    done: bool,
}

impl AnthropicDecoder {
    pub fn new(model: &Model) -> Self {
        Self {
            model: model.clone(),
            usage_model: model.clone(),
            message: new_message(model),
            blocks: HashMap::new(),
            counts: TokenCounts::default(),
            started: false,
            done: false,
        }
    }

    fn start(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        if !self.started {
            self.started = true;
            out.push(AssistantMessageEvent::Start {
                partial: self.message.clone(),
            });
        }
    }

    fn block_index(event: &Value) -> u64 {
        event.get("index").and_then(Value::as_u64).unwrap_or(0)
    }

    /// TS lines 590-610: the response id, the model that served the request
    /// (which may switch the cost table to an allowed fallback's), and the
    /// input-side token counts, kept even if the stream is cut early.
    fn message_start(&mut self, message: &Value) {
        if let Some(id) = message
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            self.message.id = id.to_string();
        }
        if let Some(served_by) = message.get("model").and_then(Value::as_str) {
            self.usage_model = usage_model_for(&self.model, served_by);
        }
        let usage = message.get("usage").cloned().unwrap_or(Value::Null);
        self.counts = TokenCounts {
            input: count(&usage, "input_tokens").unwrap_or(0),
            output: count(&usage, "output_tokens").unwrap_or(0),
            cache_read: count(&usage, "cache_read_input_tokens").unwrap_or(0),
            cache_write: count(&usage, "cache_creation_input_tokens").unwrap_or(0),
            reasoning: None,
        };
        self.recompute_usage();
    }

    /// Anthropic sends no total; TS sums the components and re-prices them
    /// on every usage update.
    fn recompute_usage(&mut self) {
        let counts = self.counts;
        let mut usage = crate::calculate_usage(
            &self.usage_model,
            counts.input,
            counts.output,
            counts.cache_read,
            counts.cache_write,
        );
        usage.reasoning = counts.reasoning;
        self.message.usage = Some(usage);
    }

    fn content_block_start(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>) {
        let index = Self::block_index(event);
        let Some(content_block) = event.get("content_block") else {
            return;
        };
        let content_index = self.message.content.len();
        let kind = match content_block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "text" => {
                self.message.content.push(ContentBlock::Text {
                    text: string_field(content_block, "text"),
                });
                out.push(AssistantMessageEvent::TextStart {
                    content_index,
                    partial: self.message.clone(),
                });
                BlockKind::Text
            }
            "thinking" => {
                self.message.content.push(ContentBlock::Thinking {
                    thinking: string_field(content_block, "thinking"),
                });
                out.push(AssistantMessageEvent::ThinkingStart {
                    content_index,
                    partial: self.message.clone(),
                });
                BlockKind::Thinking
            }
            "redacted_thinking" => {
                // TS keeps the opaque `data` payload as the block's signature
                // (with `redacted: true` and a "[Reasoning redacted]" label);
                // `ContentBlock::Thinking` has no signature field, so the
                // block stays empty and the payload is dropped.
                self.message.content.push(ContentBlock::Thinking {
                    thinking: String::new(),
                });
                out.push(AssistantMessageEvent::ThinkingStart {
                    content_index,
                    partial: self.message.clone(),
                });
                BlockKind::Thinking
            }
            "tool_use" => {
                let arguments = match content_block.get("input") {
                    Some(Value::Object(input)) => Value::Object(input.clone()),
                    _ => Value::Object(Map::new()),
                };
                self.message.content.push(ContentBlock::ToolCall {
                    id: string_field(content_block, "id"),
                    name: string_field(content_block, "name"),
                    arguments,
                });
                out.push(AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.message.clone(),
                });
                BlockKind::ToolCall
            }
            // server_tool_use, web_search_tool_result and friends have no
            // counterpart in the message; TS drops them too.
            _ => return,
        };
        self.blocks.insert(
            index,
            Block {
                kind,
                content_index,
                partial_json: String::new(),
            },
        );
    }

    fn content_block_delta(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>) {
        let index = Self::block_index(event);
        let Some(delta) = event.get("delta") else {
            return;
        };
        // A delta for an index no content_block_start opened is dropped, as
        // in TS.
        let Some(block) = self.blocks.get(&index).cloned() else {
            return;
        };
        let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
        match (delta_type, block.kind) {
            ("text_delta", BlockKind::Text) => {
                let text = string_field(delta, "text");
                if let Some(ContentBlock::Text { text: existing }) =
                    self.message.content.get_mut(block.content_index)
                {
                    existing.push_str(&text);
                }
                out.push(AssistantMessageEvent::TextDelta {
                    content_index: block.content_index,
                    delta: text,
                    partial: self.message.clone(),
                });
            }
            ("thinking_delta", BlockKind::Thinking) => {
                let thinking = string_field(delta, "thinking");
                if let Some(ContentBlock::Thinking { thinking: existing }) =
                    self.message.content.get_mut(block.content_index)
                {
                    existing.push_str(&thinking);
                }
                out.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: block.content_index,
                    delta: thinking,
                    partial: self.message.clone(),
                });
            }
            ("input_json_delta", BlockKind::ToolCall) => {
                let fragment = string_field(delta, "partial_json");
                let partial_json = match self.blocks.get_mut(&index) {
                    Some(stored) => {
                        stored.partial_json.push_str(&fragment);
                        stored.partial_json.clone()
                    }
                    None => return,
                };
                self.set_arguments(block.content_index, &partial_json);
                out.push(AssistantMessageEvent::ToolcallDelta {
                    content_index: block.content_index,
                    delta: fragment,
                    partial: self.message.clone(),
                });
            }
            // The signature authenticates the thinking block on replay; there
            // is no field to keep it in, so it is dropped.
            ("signature_delta", _) => {}
            _ => {}
        }
    }

    fn set_arguments(&mut self, content_index: usize, partial_json: &str) {
        if let Some(ContentBlock::ToolCall { arguments, .. }) =
            self.message.content.get_mut(content_index)
        {
            *arguments = parse_streaming_json(partial_json, arguments);
        }
    }

    fn close_block(&mut self, index: u64, out: &mut Vec<AssistantMessageEvent>) {
        let Some(block) = self.blocks.remove(&index) else {
            return;
        };
        match block.kind {
            BlockKind::Text => {
                let content = match self.message.content.get(block.content_index) {
                    Some(ContentBlock::Text { text }) => text.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::TextEnd {
                    content_index: block.content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            BlockKind::Thinking => {
                let content = match self.message.content.get(block.content_index) {
                    Some(ContentBlock::Thinking { thinking }) => thinking.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::ThinkingEnd {
                    content_index: block.content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            BlockKind::ToolCall => {
                // TS re-parses the buffer here, which yields `{}` when nothing
                // streamed. An empty buffer keeps whatever the block already
                // holds (`{}` unless the input arrived whole in
                // content_block_start), so a proxy that sends the input up
                // front is not silently emptied.
                self.set_arguments(block.content_index, &block.partial_json);
                let tool_call = self
                    .message
                    .content
                    .get(block.content_index)
                    .cloned()
                    .unwrap_or(ContentBlock::ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: Value::Object(Map::new()),
                    });
                out.push(AssistantMessageEvent::ToolcallEnd {
                    content_index: block.content_index,
                    tool_call,
                    partial: self.message.clone(),
                });
            }
        }
    }

    fn close_open_blocks(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        let mut indices: Vec<u64> = self.blocks.keys().copied().collect();
        indices.sort_unstable();
        for index in indices {
            self.close_block(index, out);
        }
    }

    /// TS lines 731-760: the stop reason (kept on the message so partials
    /// carry it, `Done` waits for `message_stop`) and the output-side usage,
    /// merged field by field over what `message_start` reported.
    fn message_delta(&mut self, event: &Value) {
        if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
            let (stop_reason, error_message) =
                map_stop_reason(reason, event.pointer("/delta/stop_details"));
            self.message.stop_reason = Some(stop_reason);
            if error_message.is_some() {
                self.message.error_message = error_message;
            }
        }
        if let Some(usage) = event.get("usage").filter(|usage| usage.is_object()) {
            if let Some(input) = count(usage, "input_tokens") {
                self.counts.input = input;
            }
            if let Some(output) = count(usage, "output_tokens") {
                self.counts.output = output;
            }
            if let Some(cache_read) = count(usage, "cache_read_input_tokens") {
                self.counts.cache_read = cache_read;
            }
            if let Some(cache_write) = count(usage, "cache_creation_input_tokens") {
                self.counts.cache_write = cache_write;
            }
            // Reasoning tokens are a subset of output_tokens, reported only
            // on the final delta.
            if let Some(thinking) = usage
                .pointer("/output_tokens_details/thinking_tokens")
                .and_then(Value::as_u64)
            {
                self.counts.reasoning = Some(thinking);
            }
        }
        self.recompute_usage();
    }

    fn has_tool_call(&self) -> bool {
        self.message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }

    /// `message_stop`, or the connection closing on a message whose blocks
    /// were all complete: the stop reason seen in `message_delta`, else
    /// `Stop`. TS raises "Anthropic stream ended without a stop reason" for
    /// the latter; the text received is worth more than the complaint.
    fn finalize(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        self.start(out);
        self.close_open_blocks(out);
        let mut stop_reason = self.message.stop_reason.unwrap_or(StopReason::Stop);
        if stop_reason == StopReason::Stop && self.has_tool_call() {
            stop_reason = StopReason::ToolUse;
        }
        self.message.stop_reason = Some(stop_reason);
        self.done = true;
        if stop_reason == StopReason::Error {
            out.push(AssistantMessageEvent::Error {
                reason: StopReason::Error,
                error: self.message.clone(),
            });
        } else {
            out.push(AssistantMessageEvent::Done {
                reason: stop_reason,
                message: self.message.clone(),
            });
        }
    }

    fn fail(&mut self, message: String, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        self.start(out);
        self.close_open_blocks(out);
        self.message.stop_reason = Some(StopReason::Error);
        self.message.error_message = Some(message);
        self.done = true;
        out.push(AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error: self.message.clone(),
        });
    }
}

impl StreamDecoder for AnthropicDecoder {
    fn feed(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type.is_empty() {
            return;
        }
        self.start(out);
        match event_type {
            "message_start" => {
                if let Some(message) = event.get("message") {
                    self.message_start(message);
                }
            }
            "content_block_start" => self.content_block_start(event, out),
            "content_block_delta" => self.content_block_delta(event, out),
            "content_block_stop" => {
                let index = Self::block_index(event);
                self.close_block(index, out);
            }
            "message_delta" => self.message_delta(event),
            "message_stop" => self.finalize(out),
            "error" => {
                let text = anthropic_error_text(event);
                self.fail(text, out);
            }
            // `ping` keeps the connection warm; anything else is an event
            // type newer than this decoder.
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage {
        if !self.done {
            // The connection closed before message_stop. Text already
            // received is worth keeping; a tool call cut off mid-arguments is
            // not, because executing it would guess at what the model meant.
            let cut_tool_call = self
                .blocks
                .values()
                .any(|block| block.kind == BlockKind::ToolCall);
            if cut_tool_call {
                self.fail("Stream ended before the tool call was complete".into(), out);
            } else {
                self.finalize(out);
            }
        }
        self.message.clone()
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

/// TS `mapStopReason`. Unknown reasons make TS throw, which its catch turns
/// into an error event carrying the same text.
fn map_stop_reason(reason: &str, stop_details: Option<&Value>) -> (StopReason, Option<String>) {
    match reason {
        "end_turn" => (StopReason::Stop, None),
        "max_tokens" => (StopReason::Length, None),
        "tool_use" => (StopReason::ToolUse, None),
        "refusal" => {
            let explanation = stop_details
                .and_then(|details| details.get("explanation"))
                .and_then(Value::as_str)
                .filter(|explanation| !explanation.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| "The model refused to complete the request".into());
            (StopReason::Error, Some(explanation))
        }
        // Stop is good enough -> resubmit.
        "pause_turn" => (StopReason::Stop, None),
        // We supply no stop sequences, so this should never happen.
        "stop_sequence" => (StopReason::Stop, None),
        // Content flagged by safety filters.
        "sensitive" => (
            StopReason::Error,
            Some("Provider stopped with: sensitive".into()),
        ),
        other => (
            StopReason::Error,
            Some(format!("Unhandled stop reason: {other}")),
        ),
    }
}

/// The model whose cost table prices this response. TS switches to the
/// matching `compat.allowedFallbackModels` entry when the server-side
/// fallback served the request with a different model.
fn usage_model_for(model: &Model, served_by: &str) -> Model {
    if served_by.is_empty() || served_by == model.id {
        return model.clone();
    }
    let fallback_cost = model
        .compat
        .get("allowedFallbackModels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|fallback| {
            fallback.get("provider").and_then(Value::as_str) == Some(model.provider.as_str())
                && fallback.get("model").and_then(Value::as_str) == Some(served_by)
        })
        .and_then(|fallback| fallback.get("cost"))
        .and_then(|cost| serde_json::from_value::<ModelCost>(cost.clone()).ok());
    match fallback_cost {
        Some(cost) => Model {
            id: served_by.to_string(),
            cost,
            ..model.clone()
        },
        None => model.clone(),
    }
}

/// Lenient JSON for streamed tool arguments: the text so far if it parses as
/// an object, otherwise the last value that did. TS `parseStreamingJson`
/// additionally repairs and partially parses the fragment, so its arguments
/// track the stream more closely; only the complete object matters to
/// callers.
fn parse_streaming_json(text: &str, previous: &Value) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Value::Object(map),
        Ok(_) | Err(_) => {
            if previous.is_object() {
                previous.clone()
            } else {
                Value::Object(Map::new())
            }
        }
    }
}

/// `{"type":"error","error":{"type":"overloaded_error","message":"..."}}`:
/// the message, else the error type, else the whole event.
fn anthropic_error_text(event: &Value) -> String {
    let error = event.get("error");
    let field = |key: &str| {
        error
            .and_then(|error| error.get(key))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    field("message")
        .or_else(|| field("type"))
        .unwrap_or_else(|| event.to_string())
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A token count when present and non-null (TS `!= null`), so a proxy that
/// omits the field leaves the earlier count alone.
fn count(usage: &Value, key: &str) -> Option<u64> {
    usage.get(key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_decoder::frames_of;

    fn model() -> Model {
        Model {
            id: "claude-sonnet-4-5".into(),
            name: "Claude Sonnet 4.5".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: None,
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            cost: ModelCost {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_write: 3.75,
            },
            context_window: 200_000,
            max_tokens: 64_000,
            compat: Value::Null,
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    fn run_with(model: &Model, corpus: &str) -> (AssistantMessage, Vec<AssistantMessageEvent>) {
        let mut decoder = AnthropicDecoder::new(model);
        let mut out = Vec::new();
        for frame in frames_of(corpus) {
            decoder.feed(&frame.data, &mut out);
        }
        let message = decoder.finish(&mut out);
        (message, out)
    }

    fn run(corpus: &str) -> (AssistantMessage, Vec<AssistantMessageEvent>) {
        run_with(&model(), corpus)
    }

    fn names(events: &[AssistantMessageEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                AssistantMessageEvent::Start { .. } => "start",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::ThinkingStart { .. } => "thinking_start",
                AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                AssistantMessageEvent::ThinkingEnd { .. } => "thinking_end",
                AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
                AssistantMessageEvent::ToolcallDelta { .. } => "toolcall_delta",
                AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
                AssistantMessageEvent::Done { .. } => "done",
                AssistantMessageEvent::Error { .. } => "error",
            })
            .collect()
    }

    const MESSAGE_START: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"cache_creation_input_tokens":0,"cache_read_input_tokens":10,"output_tokens":1}}}

"#;

    #[test]
    fn text_only_stream_keeps_message_start_usage_and_prices_it() {
        let corpus = format!(
            "{MESSAGE_START}{}",
            r#"event: ping
data: {"type":"ping"}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":7}}

event: message_stop
data: {"type":"message_stop"}
"#
        );
        let (message, events) = run(&corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "text_start",
                "text_delta",
                "text_delta",
                "text_end",
                "done"
            ]
        );
        assert_eq!(message.id, "msg_01");
        assert_eq!(message.model, "anthropic/claude-sonnet-4-5");
        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentBlock::Text { text } if text == "Hello world")
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(message.error_message, None);
        let usage = message.usage.as_ref().expect("usage");
        // message_delta carried only output_tokens; input and cache_read
        // survive from message_start.
        assert_eq!(usage.input, 25);
        assert_eq!(usage.cache_read, 10);
        assert_eq!(usage.cache_write, 0);
        assert_eq!(usage.output, 7);
        assert_eq!(usage.total_tokens, 42);
        assert_eq!(usage.reasoning, None);
        assert!((usage.cost.input - 0.000_075).abs() < 1e-12);
        assert!((usage.cost.output - 0.000_105).abs() < 1e-12);
        assert!((usage.cost.cache_read - 0.000_003).abs() < 1e-12);
        assert!((usage.cost.total - 0.000_183).abs() < 1e-12);
        match events.last() {
            Some(AssistantMessageEvent::Done {
                reason,
                message: done,
            }) => {
                assert_eq!(*reason, StopReason::Stop);
                assert_eq!(done, &message);
            }
            other => panic!("expected done, got {other:?}"),
        }
    }

    #[test]
    fn thinking_then_text_keep_their_content_indices() {
        let corpus = r#"
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me "}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Answer"}}

data: {"type":"content_block_stop","index":1}

data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":12}}

data: {"type":"message_stop"}
"#;
        let (message, events) = run(corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "thinking_start",
                "thinking_delta",
                "thinking_delta",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
                "done"
            ]
        );
        assert_eq!(message.content.len(), 2);
        assert!(
            matches!(&message.content[0], ContentBlock::Thinking { thinking } if thinking == "Let me think")
        );
        assert!(matches!(&message.content[1], ContentBlock::Text { text } if text == "Answer"));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantMessageEvent::ThinkingEnd { content_index: 0, content, .. } if content == "Let me think"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantMessageEvent::TextEnd { content_index: 1, content, .. } if content == "Answer"
        )));
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
    }

    #[test]
    fn a_tool_use_streamed_in_fragments_becomes_a_tool_call() {
        let corpus = format!(
            "{MESSAGE_START}{}",
            r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"read","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\": "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"Cargo.toml\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":30}}

event: message_stop
data: {"type":"message_stop"}
"#
        );
        let (message, events) = run(&corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "toolcall_start",
                "toolcall_delta",
                "toolcall_delta",
                "toolcall_delta",
                "toolcall_delta",
                "toolcall_end",
                "done"
            ]
        );
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "read");
                assert_eq!(arguments, &serde_json::json!({"path": "Cargo.toml"}));
            }
            other => panic!("expected a tool call, got {other:?}"),
        }
        // While the JSON is incomplete the arguments stay at the last object
        // that parsed; each delta still carries its fragment.
        let deltas: Vec<(&str, &Value)> = events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::ToolcallDelta { delta, partial, .. } => {
                    match &partial.content[0] {
                        ContentBlock::ToolCall { arguments, .. } => {
                            Some((delta.as_str(), arguments))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas[1].0, "{\"path\": ");
        assert_eq!(deltas[1].1, &serde_json::json!({}));
        assert_eq!(deltas[2].1, &serde_json::json!({}));
        assert_eq!(deltas[3].0, "}");
        assert_eq!(deltas[3].1, &serde_json::json!({"path": "Cargo.toml"}));
        match &events[6] {
            AssistantMessageEvent::ToolcallEnd {
                content_index,
                tool_call,
                ..
            } => {
                assert_eq!(*content_index, 0);
                assert_eq!(tool_call, &message.content[0]);
            }
            other => panic!("expected toolcall_end, got {other:?}"),
        }
        let usage = message.usage.expect("usage");
        assert_eq!((usage.input, usage.cache_read, usage.output), (25, 10, 30));
    }

    #[test]
    fn text_followed_by_tool_use_are_two_blocks() {
        let corpus = r#"
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I'll read it."}}

data: {"type":"content_block_stop","index":0}

data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_02","name":"read","input":{}}}

data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.rs\"}"}}

data: {"type":"content_block_stop","index":1}

data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":20}}

data: {"type":"message_stop"}
"#;
        let (message, events) = run(corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "text_start",
                "text_delta",
                "text_end",
                "toolcall_start",
                "toolcall_delta",
                "toolcall_end",
                "done"
            ]
        );
        assert_eq!(message.content.len(), 2);
        assert!(
            matches!(&message.content[0], ContentBlock::Text { text } if text == "I'll read it.")
        );
        assert!(matches!(
            &message.content[1],
            ContentBlock::ToolCall { id, name, arguments }
                if id == "toolu_02" && name == "read" && arguments["path"] == "a.rs"
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantMessageEvent::TextEnd {
                content_index: 0,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantMessageEvent::ToolcallEnd {
                content_index: 1,
                ..
            }
        )));
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
    }

    #[test]
    fn max_tokens_is_length() {
        let corpus = r#"
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"truncated"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"output_tokens":4096}}

data: {"type":"message_stop"}
"#;
        let (message, events) = run(corpus);
        assert_eq!(message.stop_reason, Some(StopReason::Length));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "truncated"));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done {
                reason: StopReason::Length,
                ..
            })
        ));
    }

    #[test]
    fn an_error_event_ends_the_stream_with_its_message() {
        let corpus = format!(
            "{MESSAGE_START}{}",
            r#"event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}

event: message_stop
data: {"type":"message_stop"}
"#
        );
        let (message, events) = run(&corpus);
        assert_eq!(names(&events), ["start", "error"]);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(message.error_message.as_deref(), Some("Overloaded"));
        // Input tokens from message_start survive the failure.
        assert_eq!(message.usage.as_ref().map(|usage| usage.input), Some(25));

        let (message, _) = run(r#"data: {"type":"error","error":{"type":"overloaded_error"}}"#);
        assert_eq!(message.error_message.as_deref(), Some("overloaded_error"));

        let (message, _) = run(r#"data: {"type":"error"}"#);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some(r#"{"type":"error"}"#)
        );
    }

    #[test]
    fn a_stream_cut_mid_tool_use_is_an_error() {
        let (message, events) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_03","name":"bash","input":{}}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"rm"}}
"#,
        );
        assert_eq!(
            names(&events),
            [
                "start",
                "toolcall_start",
                "toolcall_delta",
                "toolcall_end",
                "error"
            ]
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Stream ended before the tool call was complete")
        );
        // The half-received JSON never became arguments.
        assert!(matches!(
            &message.content[0],
            ContentBlock::ToolCall { arguments, .. } if arguments == &serde_json::json!({})
        ));
    }

    #[test]
    fn a_stream_cut_mid_text_keeps_the_text() {
        let (message, events) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half"}}
"#,
        );
        assert_eq!(
            names(&events),
            ["start", "text_start", "text_delta", "text_end", "done"]
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "half"));

        // With a stop reason already seen, the cut stream keeps it.
        let (message, events) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cut"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"output_tokens":3}}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Length));
        assert_eq!(names(&events).last(), Some(&"done"));
    }

    #[test]
    fn redacted_thinking_is_an_empty_thinking_block() {
        let corpus = r#"
data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EmwKAhgBEgy3va3pzix/LafPsn4aDFIT2Xlxh0L5L8rLVyIw9WFgxKJfx"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"ok"}}

data: {"type":"content_block_stop","index":1}

data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":9}}

data: {"type":"message_stop"}
"#;
        let (message, events) = run(corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "thinking_start",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
                "done"
            ]
        );
        assert_eq!(message.content.len(), 2);
        assert!(
            matches!(&message.content[0], ContentBlock::Thinking { thinking } if thinking.is_empty())
        );
        assert!(matches!(&message.content[1], ContentBlock::Text { text } if text == "ok"));
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
    }

    #[test]
    fn refusal_and_unknown_stop_reasons_are_errors_at_message_stop() {
        let refusal = |delta: &str| {
            format!(
                r#"data: {{"type":"content_block_start","index":0,"content_block":{{"type":"text","text":""}}}}

data: {{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"I can"}}}}

data: {{"type":"content_block_stop","index":0}}

data: {{"type":"message_delta","delta":{delta},"usage":{{"output_tokens":2}}}}

data: {{"type":"message_stop"}}
"#
            )
        };
        let (message, events) = run(&refusal(
            r#"{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber","explanation":"Declined for safety"}}"#,
        ));
        assert_eq!(
            names(&events),
            ["start", "text_start", "text_delta", "text_end", "error"]
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Declined for safety")
        );
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "I can"));

        let (message, _) = run(&refusal(r#"{"stop_reason":"refusal","stop_details":null}"#));
        assert_eq!(
            message.error_message.as_deref(),
            Some("The model refused to complete the request")
        );

        let (message, _) = run(&refusal(r#"{"stop_reason":"sensitive"}"#));
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Provider stopped with: sensitive")
        );

        let (message, events) = run(&refusal(r#"{"stop_reason":"brand_new_reason"}"#));
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Unhandled stop reason: brand_new_reason")
        );
        assert_eq!(names(&events).last(), Some(&"error"));
    }

    #[test]
    fn pause_turn_and_stop_sequence_are_stop() {
        for reason in ["pause_turn", "stop_sequence"] {
            let (message, events) = run(&format!(
                r#"data: {{"type":"message_delta","delta":{{"stop_reason":"{reason}","stop_sequence":null}},"usage":{{"output_tokens":1}}}}

data: {{"type":"message_stop"}}
"#
            ));
            assert_eq!(message.stop_reason, Some(StopReason::Stop), "{reason}");
            assert_eq!(names(&events), ["start", "done"], "{reason}");
        }
    }

    #[test]
    fn message_delta_usage_overrides_only_the_fields_it_carries() {
        let corpus = format!(
            "{MESSAGE_START}{}",
            r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":30,"output_tokens":40,"cache_creation_input_tokens":5,"cache_read_input_tokens":null,"output_tokens_details":{"thinking_tokens":11}}}

event: message_stop
data: {"type":"message_stop"}
"#
        );
        let (message, _) = run(&corpus);
        let usage = message.usage.expect("usage");
        assert_eq!(usage.input, 30);
        assert_eq!(usage.output, 40);
        assert_eq!(usage.cache_write, 5);
        // null is "not present": the message_start value stands.
        assert_eq!(usage.cache_read, 10);
        assert_eq!(usage.reasoning, Some(11));
        assert_eq!(usage.total_tokens, 30 + 40 + 5 + 10);

        // Without message_start, message_delta alone still produces usage.
        let (message, _) = run(
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}

data: {"type":"message_stop"}
"#,
        );
        let usage = message.usage.expect("usage");
        assert_eq!((usage.input, usage.output, usage.total_tokens), (0, 3, 3));
    }

    #[test]
    fn a_fallback_model_is_priced_from_its_compat_entry() {
        let mut fallback_model = model();
        fallback_model.compat = serde_json::json!({
            "allowedFallbackModels": [
                {
                    "provider": "anthropic",
                    "model": "claude-fallback",
                    "cost": {"input": 1.0, "output": 2.0, "cacheRead": 0.1, "cacheWrite": 1.25}
                },
                {
                    "provider": "other",
                    "model": "claude-elsewhere",
                    "cost": {"input": 9.0, "output": 9.0}
                }
            ]
        });
        let served_by = |served_by: &str| {
            format!(
                r#"data: {{"type":"message_start","message":{{"id":"msg_02","model":"{served_by}","usage":{{"input_tokens":1000000,"output_tokens":0}}}}}}

data: {{"type":"message_delta","delta":{{"stop_reason":"end_turn"}},"usage":{{"output_tokens":1000000}}}}

data: {{"type":"message_stop"}}
"#
            )
        };
        let (message, _) = run_with(&fallback_model, &served_by("claude-fallback"));
        let usage = message.usage.expect("usage");
        assert!((usage.cost.input - 1.0).abs() < 1e-9);
        assert!((usage.cost.output - 2.0).abs() < 1e-9);
        // The message keeps naming the model that was asked for.
        assert_eq!(message.model, "anthropic/claude-sonnet-4-5");

        let (message, _) = run_with(&fallback_model, &served_by("claude-sonnet-4-5"));
        let usage = message.usage.expect("usage");
        assert!((usage.cost.input - 3.0).abs() < 1e-9);

        // Served by a model outside the allow-list, or listed for another
        // provider: the requested model's table applies.
        for other in ["claude-unknown", "claude-elsewhere"] {
            let (message, _) = run_with(&fallback_model, &served_by(other));
            let usage = message.usage.expect("usage");
            assert!((usage.cost.input - 3.0).abs() < 1e-9, "{other}");
        }
    }

    #[test]
    fn tool_input_that_arrived_whole_is_kept_when_nothing_streams() {
        let (message, events) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_04","name":"ls","input":{"path":"src"}}}

data: {"type":"content_block_stop","index":0}

data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}

data: {"type":"message_stop"}
"#,
        );
        assert_eq!(
            names(&events),
            ["start", "toolcall_start", "toolcall_end", "done"]
        );
        assert!(matches!(
            &message.content[0],
            ContentBlock::ToolCall { arguments, .. } if arguments == &serde_json::json!({"path": "src"})
        ));
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
    }

    #[test]
    fn message_stop_without_a_stop_reason_is_stop_or_tool_use() {
        let (message, events) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"message_stop"}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(names(&events).last(), Some(&"done"));

        let (message, _) = run(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_05","name":"ls","input":{}}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}

data: {"type":"content_block_stop","index":0}

data: {"type":"message_stop"}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
    }

    #[test]
    fn malformed_events_never_panic() {
        let mut decoder = AnthropicDecoder::new(&model());
        let mut out = Vec::new();
        let junk = [
            serde_json::json!([]),
            serde_json::json!("message_start"),
            serde_json::json!({}),
            serde_json::json!({"type": 7}),
            serde_json::json!({"type": "message_start"}),
            serde_json::json!({"type": "message_start", "message": "x"}),
            serde_json::json!({"type": "message_start", "message": {"id": 3, "model": 4, "usage": "none"}}),
            serde_json::json!({"type": "content_block_start"}),
            serde_json::json!({"type": "content_block_start", "index": "a", "content_block": "b"}),
            serde_json::json!({"type": "content_block_start", "index": 2, "content_block": {"type": "server_tool_use"}}),
            serde_json::json!({"type": "content_block_delta", "index": 0}),
            serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "orphan"}}),
            serde_json::json!({"type": "content_block_delta", "index": 2, "delta": {"type": "text_delta", "text": "dropped"}}),
            serde_json::json!({"type": "content_block_stop", "index": 9}),
            serde_json::json!({"type": "message_delta", "delta": "nope", "usage": "nah"}),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": 7}, "usage": {"output_tokens": -1}}),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": "x"}}),
            serde_json::json!({"type": "ping"}),
            serde_json::json!({"type": "something_new", "index": 0}),
        ];
        for event in &junk {
            decoder.feed(event, &mut out);
        }
        assert!(!decoder.is_done());
        decoder.feed(
            &serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use"}}),
            &mut out,
        );
        decoder.feed(
            &serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "not json"}}),
            &mut out,
        );
        decoder.feed(
            &serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "wrong kind"}}),
            &mut out,
        );
        decoder.feed(
            &serde_json::json!({"type": "content_block_stop", "index": 0}),
            &mut out,
        );
        decoder.feed(&serde_json::json!({"type": "message_stop"}), &mut out);
        let message = decoder.finish(&mut out);
        assert!(decoder.is_done());
        assert_eq!(message.content.len(), 1);
        assert!(matches!(
            &message.content[0],
            ContentBlock::ToolCall { id, name, arguments }
                if id.is_empty() && name.is_empty() && arguments == &serde_json::json!({})
        ));
        // The refusal delta was malformed but its stop reason still counts.
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("The model refused to complete the request")
        );
        assert_eq!(
            names(&out),
            [
                "start",
                "toolcall_start",
                "toolcall_delta",
                "toolcall_end",
                "error"
            ]
        );
    }

    #[test]
    fn feeding_after_done_is_ignored_and_is_done_reports_it() {
        let mut decoder = AnthropicDecoder::new(&model());
        let mut out = Vec::new();
        decoder.feed(
            &serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            &mut out,
        );
        assert!(!decoder.is_done());
        decoder.feed(&serde_json::json!({"type":"message_stop"}), &mut out);
        assert!(decoder.is_done());
        decoder.feed(
            &serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"late"}}),
            &mut out,
        );
        decoder.feed(
            &serde_json::json!({"type":"error","error":{"type":"late","message":"late"}}),
            &mut out,
        );
        let message = decoder.finish(&mut out);
        assert!(message.content.is_empty());
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(names(&out), ["start", "done"]);
    }

    #[test]
    fn finishing_an_empty_stream_emits_start_and_done() {
        let (message, events) = run("");
        assert_eq!(names(&events), ["start", "done"]);
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert!(message.content.is_empty());
        assert_eq!(message.usage, None);
    }
}
