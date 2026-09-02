//! Incremental provider stream decoders.
//!
//! One decoder per wire format, fed one parsed SSE payload at a time and
//! producing `AssistantMessageEvent`s as it goes, so a token can be painted
//! the moment it arrives instead of after the whole body has been buffered.
//! Each decoder carries the partial `AssistantMessage` that every event
//! references, exactly as the TypeScript stream functions do.
//!
//! `ResponsesDecoder` mirrors `processResponsesStream` in
//! `vendor/pi/packages/ai/src/api/openai-responses-shared.ts` (with the Codex
//! normalisation from `mapCodexEvents` in `openai-codex-responses.ts`).

use std::collections::HashMap;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::catalog::Model;
use crate::stream::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};

/// A decoder for one provider wire format.
pub trait StreamDecoder {
    /// Fold one provider event into the message, pushing the stream events it
    /// produces onto `out`.
    fn feed(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>);

    /// The stream ended, either on its terminal event or because the
    /// connection closed. Closes whatever is still open, emits `Done` (or
    /// `Error`) if that has not happened yet, and returns the finished message.
    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage;

    /// Whether the terminal event has been seen, so a reader can stop early.
    fn is_done(&self) -> bool;
}

/// The decoder for a model's API, or `None` when the API has no incremental
/// decoder and must be requested without `stream: true`.
pub fn decoder_for(model: &Model) -> Option<Box<dyn StreamDecoder>> {
    match model.api.as_str() {
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            Some(Box::new(ResponsesDecoder::new(model)))
        }
        "openai-completions" => Some(Box::new(
            crate::stream_decoder_completions::CompletionsDecoder::new(model),
        )),
        "anthropic-messages" => Some(Box::new(
            crate::stream_decoder_anthropic::AnthropicDecoder::new(model),
        )),
        _ => None,
    }
}

/// Whether requests for this model should carry `stream: true`.
pub fn supports_incremental_stream(model: &Model) -> bool {
    matches!(
        model.api.as_str(),
        "openai-responses"
            | "azure-openai-responses"
            | "openai-codex-responses"
            | "openai-completions"
            | "anthropic-messages"
    )
}

/// An empty assistant message for `model`, the shape every decoder starts from.
pub fn new_message(model: &Model) -> AssistantMessage {
    AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: Vec::new(),
        model: format!("{}/{}", model.provider, model.id),
        usage: None,
        stop_reason: None,
        error_message: None,
    }
}

/// Splits a byte stream into SSE frames. Lines are accumulated until a blank
/// line ends the frame; `data:` lines are joined with newlines, `event:` names
/// are kept, comments (`:` lines) are dropped, and the `[DONE]` sentinel is
/// swallowed.
#[derive(Debug, Default)]
pub struct SseFramer {
    event: String,
    data: String,
    has_data: bool,
    saw_done: bool,
}

/// One SSE frame: the `event:` name (often empty) and the parsed `data:` JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct SseFrame {
    pub event: String,
    pub data: Value,
}

impl SseFramer {
    /// Feed one line (without its line terminator). Returns the frame that the
    /// line completed, if any.
    pub fn feed_line(&mut self, line: &str) -> Option<SseFrame> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.flush();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
        None
    }

    /// Whether the `[DONE]` sentinel has gone by: the stream is over even if
    /// the server keeps the connection open.
    pub fn saw_done(&self) -> bool {
        self.saw_done
    }

    /// End of input: whatever is pending is a frame too.
    pub fn flush(&mut self) -> Option<SseFrame> {
        if !self.has_data {
            self.event.clear();
            return None;
        }
        let event = std::mem::take(&mut self.event);
        let data = std::mem::take(&mut self.data);
        self.has_data = false;
        if data.trim() == "[DONE]" {
            self.saw_done = true;
            return None;
        }
        let parsed = match serde_json::from_str::<Value>(&data) {
            Ok(parsed) => parsed,
            Err(err) => {
                // The frames that fail to parse are the ones a trace is
                // read for: a truncated tail, a proxy's HTML, a stray line.
                if crate::trace::enabled() {
                    crate::trace::log(&format!(
                        "sse frame dropped: {err}: {}",
                        data.chars().take(200).collect::<String>()
                    ));
                }
                return None;
            }
        };
        Some(SseFrame {
            event,
            data: parsed,
        })
    }
}

/// Parse a whole SSE corpus into frames, for replay and tests.
pub fn frames_of(corpus: &str) -> Vec<SseFrame> {
    let mut framer = SseFramer::default();
    let mut frames = Vec::new();
    for line in corpus.lines() {
        if let Some(frame) = framer.feed_line(line) {
            frames.push(frame);
        }
    }
    if let Some(frame) = framer.flush() {
        frames.push(frame);
    }
    frames
}

/// Lenient JSON for streamed tool arguments: the text so far if it parses,
/// otherwise the last value that did. Arguments are always an object.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Text,
    Thinking,
    ToolCall,
}

#[derive(Debug, Clone)]
struct Slot {
    kind: SlotKind,
    content_index: usize,
    /// The argument JSON as it streams in (function calls), or the custom
    /// tool input so far (custom tool calls).
    partial: String,
    custom_input: bool,
}

/// Decoder for the OpenAI Responses API and the ChatGPT Codex flavour of it.
pub struct ResponsesDecoder {
    model: Model,
    message: AssistantMessage,
    slots: HashMap<u64, Slot>,
    started: bool,
    done: bool,
}

impl ResponsesDecoder {
    pub fn new(model: &Model) -> Self {
        Self {
            model: model.clone(),
            message: new_message(model),
            slots: HashMap::new(),
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

    fn output_index(event: &Value) -> u64 {
        event
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }

    fn create_slot(
        &mut self,
        output_index: u64,
        item: &Value,
        out: &mut Vec<AssistantMessageEvent>,
    ) -> Option<Slot> {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        let content_index = self.message.content.len();
        let slot = match item_type {
            "reasoning" => {
                self.message.content.push(ContentBlock::Thinking {
                    thinking: String::new(),
                });
                out.push(AssistantMessageEvent::ThinkingStart {
                    content_index,
                    partial: self.message.clone(),
                });
                Slot {
                    kind: SlotKind::Thinking,
                    content_index,
                    partial: String::new(),
                    custom_input: false,
                }
            }
            "message" => {
                self.message.content.push(ContentBlock::Text {
                    text: String::new(),
                });
                out.push(AssistantMessageEvent::TextStart {
                    content_index,
                    partial: self.message.clone(),
                });
                Slot {
                    kind: SlotKind::Text,
                    content_index,
                    partial: String::new(),
                    custom_input: false,
                }
            }
            "function_call" => {
                let partial = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.message.content.push(ContentBlock::ToolCall {
                    id: tool_call_id(item),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: parse_streaming_json(&partial, &Value::Null),
                });
                out.push(AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.message.clone(),
                });
                Slot {
                    kind: SlotKind::ToolCall,
                    content_index,
                    partial,
                    custom_input: false,
                }
            }
            "custom_tool_call" => {
                let input = item
                    .get("input")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let mut arguments = Map::new();
                arguments.insert("input".into(), Value::String(input.clone()));
                self.message.content.push(ContentBlock::ToolCall {
                    id: tool_call_id(item),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: Value::Object(arguments),
                });
                out.push(AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.message.clone(),
                });
                Slot {
                    kind: SlotKind::ToolCall,
                    content_index,
                    partial: input,
                    custom_input: true,
                }
            }
            _ => return None,
        };
        self.slots.insert(output_index, slot.clone());
        Some(slot)
    }

    /// The slot at `output_index` if it is of `kind`. A delta that arrives
    /// without a preceding `output_item.added` (older Codex streams) gets a
    /// slot made for it, which the TypeScript decoder would drop; being
    /// lenient here loses nothing.
    fn slot_for(
        &mut self,
        output_index: u64,
        kind: SlotKind,
        out: &mut Vec<AssistantMessageEvent>,
    ) -> Option<Slot> {
        match self.slots.get(&output_index) {
            Some(slot) if slot.kind == kind => Some(slot.clone()),
            Some(_) => None,
            None => {
                let item = match kind {
                    SlotKind::Text => serde_json::json!({"type": "message"}),
                    SlotKind::Thinking => serde_json::json!({"type": "reasoning"}),
                    SlotKind::ToolCall => return None,
                };
                self.create_slot(output_index, &item, out)
            }
        }
    }

    fn append_text(&mut self, slot: &Slot, delta: &str, out: &mut Vec<AssistantMessageEvent>) {
        if let Some(ContentBlock::Text { text }) = self.message.content.get_mut(slot.content_index)
        {
            text.push_str(delta);
        }
        out.push(AssistantMessageEvent::TextDelta {
            content_index: slot.content_index,
            delta: delta.to_string(),
            partial: self.message.clone(),
        });
    }

    fn append_thinking(&mut self, slot: &Slot, delta: &str, out: &mut Vec<AssistantMessageEvent>) {
        if let Some(ContentBlock::Thinking { thinking }) =
            self.message.content.get_mut(slot.content_index)
        {
            thinking.push_str(delta);
        }
        out.push(AssistantMessageEvent::ThinkingDelta {
            content_index: slot.content_index,
            delta: delta.to_string(),
            partial: self.message.clone(),
        });
    }

    fn set_arguments(&mut self, slot: &Slot, partial: &str) {
        if let Some(ContentBlock::ToolCall { arguments, .. }) =
            self.message.content.get_mut(slot.content_index)
        {
            if slot.custom_input {
                if let Value::Object(map) = arguments {
                    map.insert("input".into(), Value::String(partial.to_string()));
                }
            } else {
                *arguments = parse_streaming_json(partial, arguments);
            }
        }
        if let Some(stored) = self
            .slots
            .values_mut()
            .find(|stored| stored.content_index == slot.content_index)
        {
            stored.partial = partial.to_string();
        }
    }

    fn tool_delta(&mut self, slot: &Slot, delta: &str, out: &mut Vec<AssistantMessageEvent>) {
        out.push(AssistantMessageEvent::ToolcallDelta {
            content_index: slot.content_index,
            delta: delta.to_string(),
            partial: self.message.clone(),
        });
    }

    fn close_slot(
        &mut self,
        output_index: u64,
        item: Option<&Value>,
        out: &mut Vec<AssistantMessageEvent>,
    ) {
        let Some(slot) = self.slots.remove(&output_index) else {
            return;
        };
        match slot.kind {
            SlotKind::Thinking => {
                if let Some(item) = item {
                    let summary = join_texts(item.get("summary"), "text");
                    let content = join_texts(item.get("content"), "text");
                    let text = if !summary.is_empty() {
                        Some(summary)
                    } else if !content.is_empty() {
                        Some(content)
                    } else {
                        None
                    };
                    if let (Some(text), Some(ContentBlock::Thinking { thinking })) =
                        (text, self.message.content.get_mut(slot.content_index))
                    {
                        *thinking = text;
                    }
                }
                let content = match self.message.content.get(slot.content_index) {
                    Some(ContentBlock::Thinking { thinking }) => thinking.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::ThinkingEnd {
                    content_index: slot.content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            SlotKind::Text => {
                if let Some(item) = item {
                    let joined: String = item
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|part| {
                                    part.get("text")
                                        .or_else(|| part.get("refusal"))
                                        .and_then(Value::as_str)
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    if let (false, Some(ContentBlock::Text { text })) = (
                        joined.is_empty(),
                        self.message.content.get_mut(slot.content_index),
                    ) {
                        *text = joined;
                    }
                }
                let content = match self.message.content.get(slot.content_index) {
                    Some(ContentBlock::Text { text }) => text.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::TextEnd {
                    content_index: slot.content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            SlotKind::ToolCall => {
                if let Some(item) = item {
                    if slot.custom_input {
                        if let Some(input) = item.get("input").and_then(Value::as_str) {
                            self.set_arguments(&slot, input);
                        }
                    } else {
                        let raw = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .filter(|raw| !raw.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| slot.partial.clone());
                        let raw = if raw.is_empty() {
                            "{}".to_string()
                        } else {
                            raw
                        };
                        self.set_arguments(&slot, &raw);
                    }
                } else if !slot.custom_input {
                    let raw = if slot.partial.is_empty() {
                        "{}".to_string()
                    } else {
                        slot.partial.clone()
                    };
                    self.set_arguments(&slot, &raw);
                }
                let block = self
                    .message
                    .content
                    .get(slot.content_index)
                    .cloned()
                    .unwrap_or(ContentBlock::ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: Value::Object(Map::new()),
                    });
                out.push(AssistantMessageEvent::ToolcallEnd {
                    content_index: slot.content_index,
                    tool_call: block,
                    partial: self.message.clone(),
                });
            }
        }
    }

    fn close_open_slots(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        let mut indices: Vec<u64> = self.slots.keys().copied().collect();
        indices.sort_unstable();
        for index in indices {
            self.close_slot(index, None, out);
        }
    }

    fn has_tool_call(&self) -> bool {
        self.message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }

    fn finalize(&mut self, response: Option<&Value>, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        self.start(out);
        self.close_open_slots(out);
        if let Some(usage) = response.and_then(|response| response.get("usage")) {
            self.message.usage = Some(responses_usage(&self.model, usage));
        }
        let status = response
            .and_then(|response| response.get("status"))
            .and_then(Value::as_str);
        let incomplete_reason = response
            .and_then(|response| response.pointer("/incomplete_details/reason"))
            .and_then(Value::as_str);
        let (mut stop_reason, error_message) = map_stop_reason(status, incomplete_reason);
        if stop_reason == StopReason::Stop && self.has_tool_call() {
            stop_reason = StopReason::ToolUse;
        }
        self.message.stop_reason = Some(stop_reason);
        self.message.error_message = error_message;
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
        self.close_open_slots(out);
        self.message.stop_reason = Some(StopReason::Error);
        self.message.error_message = Some(message);
        self.done = true;
        out.push(AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error: self.message.clone(),
        });
    }
}

/// TS `finalizeResponse` usage mapping: OpenAI counts cached and cache-write
/// tokens inside `input_tokens`, so both are subtracted, and the cost is
/// applied from the model's table.
pub(crate) fn responses_usage(model: &Model, usage: &Value) -> pi_protocol::Usage {
    let get = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cached = usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = usage
        .pointer("/input_tokens_details/cache_write_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = get("input_tokens")
        .saturating_sub(cached)
        .saturating_sub(cache_write);
    let output = get("output_tokens");
    let reasoning = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64);
    let mut computed = crate::calculate_usage(model, input, output, cached, cache_write);
    computed.reasoning = reasoning;
    let total = get("total_tokens");
    if total > 0 {
        computed.total_tokens = total;
    }
    computed
}

/// TS `mapStopReason` in `openai-responses-shared.ts`.
fn map_stop_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
) -> (StopReason, Option<String>) {
    match status {
        None | Some("completed") | Some("in_progress") | Some("queued") => (StopReason::Stop, None),
        Some("incomplete") => match incomplete_reason {
            Some("max_output_tokens") => (StopReason::Length, None),
            Some(reason) => (
                StopReason::Error,
                Some(format!("Response incomplete: {reason}")),
            ),
            None => (
                StopReason::Error,
                Some("Response incomplete without a provider reason".into()),
            ),
        },
        Some("failed") | Some("cancelled") => (StopReason::Error, None),
        Some(other) => (
            StopReason::Error,
            Some(format!("Unknown response status: {other}")),
        ),
    }
}

/// `call_id|item_id`, the shape TS persists so a later turn can replay both.
fn tool_call_id(item: &Value) -> String {
    let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
    let id = item.get("id").and_then(Value::as_str).unwrap_or("");
    match (call_id.is_empty(), id.is_empty()) {
        (false, false) => format!("{call_id}|{id}"),
        (false, true) => call_id.to_string(),
        (true, false) => id.to_string(),
        (true, true) => Uuid::new_v4().to_string(),
    }
}

fn join_texts(parts: Option<&Value>, key: &str) -> String {
    parts
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get(key).and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

fn event_delta(event: &Value) -> String {
    event
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// TS `extractCodexEventError`: the message, else the code, from either the
/// event itself or its nested `error`.
fn codex_error_text(event: &Value) -> String {
    let nested = event.get("error");
    let message = event.get("message").and_then(Value::as_str).or_else(|| {
        nested
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
    });
    let code = event.get("code").and_then(Value::as_str).or_else(|| {
        nested
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
    });
    match (message, code) {
        (Some(message), _) => message.to_string(),
        (None, Some(code)) => code.to_string(),
        (None, None) => event.to_string(),
    }
}

impl StreamDecoder for ResponsesDecoder {
    fn feed(&mut self, event: &Value, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type.is_empty() {
            return;
        }
        self.start(out);
        let output_index = Self::output_index(event);
        match event_type {
            "response.created" | "response.in_progress" | "response.queued" => {}
            "response.output_item.added" => {
                if let Some(item) = event.get("item") {
                    self.create_slot(output_index, item, out);
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::Text, out) {
                    let delta = event_delta(event);
                    self.append_text(&slot, &delta, out);
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::Thinking, out) {
                    let delta = event_delta(event);
                    self.append_thinking(&slot, &delta, out);
                }
            }
            "response.reasoning_summary_part.done" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::Thinking, out) {
                    self.append_thinking(&slot, "\n\n", out);
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::ToolCall, out) {
                    if slot.custom_input {
                        return;
                    }
                    let delta = event_delta(event);
                    let partial = format!("{}{}", slot.partial, delta);
                    self.set_arguments(&slot, &partial);
                    self.tool_delta(&slot, &delta, out);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::ToolCall, out) {
                    if slot.custom_input {
                        return;
                    }
                    let arguments = event
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let previous = slot.partial.clone();
                    self.set_arguments(&slot, &arguments);
                    if let Some(rest) = arguments.strip_prefix(previous.as_str()) {
                        if !rest.is_empty() {
                            self.tool_delta(&slot, rest, out);
                        }
                    }
                }
            }
            "response.custom_tool_call_input.delta" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::ToolCall, out) {
                    if !slot.custom_input {
                        return;
                    }
                    let delta = event_delta(event);
                    let input = format!("{}{}", slot.partial, delta);
                    self.set_arguments(&slot, &input);
                    self.tool_delta(&slot, &delta, out);
                }
            }
            "response.custom_tool_call_input.done" => {
                if let Some(slot) = self.slot_for(output_index, SlotKind::ToolCall, out) {
                    if !slot.custom_input {
                        return;
                    }
                    let input = event
                        .get("input")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let previous = slot.partial.clone();
                    self.set_arguments(&slot, &input);
                    if let Some(rest) = input.strip_prefix(previous.as_str()) {
                        if !rest.is_empty() {
                            self.tool_delta(&slot, rest, out);
                        }
                    }
                }
            }
            "response.output_item.done" => {
                let item = event.get("item");
                if !self.slots.contains_key(&output_index) {
                    if let Some(item) = item {
                        self.create_slot(output_index, item, out);
                    }
                }
                self.close_slot(output_index, item, out);
            }
            "response.completed" | "response.done" | "response.incomplete" => {
                let response = event.get("response").cloned();
                self.finalize(response.as_ref(), out);
            }
            "response.failed" => {
                let text = event
                    .get("response")
                    .map(codex_error_text)
                    .filter(|text| !text.is_empty() && !text.starts_with('{'))
                    .unwrap_or_else(|| "Codex response failed".into());
                self.fail(text, out);
            }
            "error" => {
                let text = codex_error_text(event);
                self.fail(format!("Codex error: {text}"), out);
            }
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage {
        if !self.done {
            // The connection closed before the terminal event. Text already
            // received is worth keeping; a tool call cut off mid-arguments is
            // not, because executing it would guess at what the model meant.
            let cut_tool_call = self
                .slots
                .values()
                .any(|slot| slot.kind == SlotKind::ToolCall);
            if cut_tool_call {
                self.fail("Stream ended before the tool call was complete".into(), out);
            } else {
                self.finalize(None, out);
            }
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
    use crate::catalog::ModelCost;

    fn model() -> Model {
        Model {
            id: "gpt-5".into(),
            name: "gpt-5".into(),
            api: "openai-codex-responses".into(),
            provider: "openai-codex".into(),
            base_url: None,
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
            thinking_level_map: Default::default(),
        }
    }

    fn run(corpus: &str) -> (AssistantMessage, Vec<AssistantMessageEvent>) {
        let mut decoder = ResponsesDecoder::new(&model());
        let mut out = Vec::new();
        for frame in frames_of(corpus) {
            decoder.feed(&frame.data, &mut out);
        }
        let message = decoder.finish(&mut out);
        (message, out)
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

    #[test]
    fn sse_framer_joins_data_lines_and_skips_comments_and_done() {
        let frames = frames_of(
            ": keep-alive\nevent: ping\ndata: {\"a\":1}\ndata: \n\ndata: [DONE]\n\ndata: {\"b\":2}",
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].event, "ping");
        assert_eq!(frames[0].data["a"], 1);
        assert_eq!(frames[1].data["b"], 2);
    }

    #[test]
    fn a_function_call_becomes_a_tool_call_block_and_tool_use_stop() {
        let corpus = r#"
data: {"type":"response.created","response":{"id":"resp_1"}}

data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":""}}

data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":"}

data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"\"Cargo.toml\"}"}

data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"Cargo.toml\"}"}

data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":"{\"path\":\"Cargo.toml\"}"}}

data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":120,"output_tokens":9,"total_tokens":129,"input_tokens_details":{"cached_tokens":100}}}}
"#;
        let (message, events) = run(corpus);
        assert_eq!(
            names(&events),
            [
                "start",
                "toolcall_start",
                "toolcall_delta",
                "toolcall_delta",
                "toolcall_end",
                "done"
            ]
        );
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        match &message.content[0] {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_1|fc_1");
                assert_eq!(name, "read");
                assert_eq!(arguments["path"], "Cargo.toml");
            }
            other => panic!("expected a tool call, got {other:?}"),
        }
        let usage = message.usage.expect("usage");
        assert_eq!(usage.input, 20);
        assert_eq!(usage.cache_read, 100);
        assert_eq!(usage.output, 9);
        assert_eq!(usage.total_tokens, 129);
    }

    #[test]
    fn reasoning_then_text_then_two_tool_calls_keep_their_content_indices() {
        let corpus = r#"
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1"}}

data: {"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"Need the file"}

data: {"type":"response.reasoning_summary_part.done","output_index":0}

data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Need the file"}]}}

data: {"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg_1"}}

data: {"type":"response.output_text.delta","output_index":1,"delta":"Reading "}

data: {"type":"response.output_text.delta","output_index":1,"delta":"both."}

data: {"type":"response.output_item.done","output_index":1,"item":{"type":"message","id":"msg_1","content":[{"type":"output_text","text":"Reading both."}]}}

data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc_a","call_id":"call_a","name":"read","arguments":""}}

data: {"type":"response.function_call_arguments.done","output_index":2,"arguments":"{\"path\":\"a\"}"}

data: {"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","id":"fc_a","call_id":"call_a","name":"read","arguments":"{\"path\":\"a\"}"}}

data: {"type":"response.output_item.added","output_index":3,"item":{"type":"function_call","id":"fc_b","call_id":"call_b","name":"read","arguments":""}}

data: {"type":"response.output_item.done","output_index":3,"item":{"type":"function_call","id":"fc_b","call_id":"call_b","name":"read","arguments":"{\"path\":\"b\"}"}}

data: {"type":"response.completed","response":{"status":"completed"}}
"#;
        let (message, events) = run(corpus);
        assert_eq!(message.content.len(), 4);
        assert!(
            matches!(&message.content[0], ContentBlock::Thinking { thinking } if thinking == "Need the file")
        );
        assert!(
            matches!(&message.content[1], ContentBlock::Text { text } if text == "Reading both.")
        );
        assert!(
            matches!(&message.content[2], ContentBlock::ToolCall { arguments, .. } if arguments["path"] == "a")
        );
        assert!(
            matches!(&message.content[3], ContentBlock::ToolCall { arguments, .. } if arguments["path"] == "b")
        );
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        let ends: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::ToolcallEnd { content_index, .. } => Some(*content_index),
                _ => None,
            })
            .collect();
        assert_eq!(ends, [2, 3]);
        // The summary_part.done newline is replaced by the item's own summary.
        assert!(names(&events).contains(&"thinking_end"));
    }

    #[test]
    fn text_deltas_without_an_added_item_still_render() {
        let corpus = r#"
data: {"type":"response.created","response":{"id":"resp_1"}}

data: {"type":"response.output_text.delta","output_index":0,"delta":"Hello"}

data: {"type":"response.output_text.delta","output_index":0,"delta":" Codex"}

data: {"type":"response.completed","response":{"status":"completed"}}
"#;
        let (message, events) = run(corpus);
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
        assert!(
            matches!(&message.content[0], ContentBlock::Text { text } if text == "Hello Codex")
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
    }

    #[test]
    fn incomplete_for_max_output_tokens_is_length_and_other_reasons_are_errors() {
        let (message, _) = run(
            r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Length));
        let (message, events) = run(
            r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"content_filter"}}}"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Response incomplete: content_filter")
        );
        assert_eq!(names(&events).last(), Some(&"error"));
    }

    #[test]
    fn provider_error_frames_carry_their_message() {
        let (message, events) = run(
            r#"data: {"type":"error","error":{"code":"rate_limit_exceeded","message":"slow down"}}"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Codex error: slow down")
        );
        assert_eq!(names(&events), ["start", "error"]);

        let (message, _) = run(
            r#"data: {"type":"response.failed","response":{"error":{"code":"server_error","message":"boom"}}}"#,
        );
        assert_eq!(message.error_message.as_deref(), Some("boom"));
    }

    #[test]
    fn a_stream_cut_mid_text_keeps_the_text_but_cut_mid_tool_call_is_an_error() {
        let (message, events) = run(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}

data: {"type":"response.output_text.delta","output_index":0,"delta":"half"}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "half"));
        assert_eq!(names(&events).last(), Some(&"done"));

        let (message, _) = run(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc","call_id":"c","name":"bash","arguments":""}}

data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"command\":\"rm"}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
    }

    #[test]
    fn custom_tool_calls_accumulate_their_input() {
        let (message, _) = run(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"custom_tool_call","id":"ct","call_id":"c","name":"apply_patch","input":""}}

data: {"type":"response.custom_tool_call_input.delta","output_index":0,"delta":"*** Begin"}

data: {"type":"response.custom_tool_call_input.done","output_index":0,"input":"*** Begin Patch"}

data: {"type":"response.output_item.done","output_index":0,"item":{"type":"custom_tool_call","id":"ct","call_id":"c","name":"apply_patch","input":"*** Begin Patch"}}

data: {"type":"response.completed","response":{"status":"completed"}}
"#,
        );
        assert!(
            matches!(&message.content[0], ContentBlock::ToolCall { name, arguments, .. } if name == "apply_patch" && arguments["input"] == "*** Begin Patch")
        );
    }

    #[test]
    fn feeding_after_done_is_ignored_and_is_done_reports_it() {
        let mut decoder = ResponsesDecoder::new(&model());
        let mut out = Vec::new();
        decoder.feed(
            &serde_json::json!({"type":"response.completed","response":{"status":"completed"}}),
            &mut out,
        );
        assert!(decoder.is_done());
        decoder.feed(
            &serde_json::json!({"type":"response.output_text.delta","output_index":0,"delta":"late"}),
            &mut out,
        );
        let message = decoder.finish(&mut out);
        assert!(message.content.is_empty());
        assert_eq!(names(&out), ["start", "done"]);
    }
}
