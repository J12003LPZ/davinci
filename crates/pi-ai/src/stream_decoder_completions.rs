//! Incremental decoder for the OpenAI chat-completions SSE format, mirroring
//! the streaming loop of `streamOpenAICompletions` (the
//! `for await (const chunk of openaiStream)` body, `parseChunkUsage` and
//! `mapStopReason`) in `vendor/pi/packages/ai/src/api/openai-completions.ts`.
//! Every provider with `api: "openai-completions"` — OpenAI, Groq, OpenRouter,
//! DeepSeek, xAI, llama.cpp and the other compatible endpoints — streams
//! through it.
//!
//! A chunk carries `choices[0].delta` (text in `content`, reasoning in the
//! first non-empty of `reasoning_content`/`reasoning`/`reasoning_text`, tool
//! call fragments in `tool_calls[]` keyed by `index`) and, on the last one,
//! `choices[0].finish_reason`; with `stream_options.include_usage` a final
//! chunk with empty `choices` brings `usage`. Chat completions has no
//! terminal event the decoder can see (`[DONE]` is swallowed by the framer)
//! and that usage chunk follows `finish_reason`, so `finish_reason` only
//! closes the open blocks and records the stop reason, `Done`/`Error` is
//! emitted from `finish()`, and `is_done()` turns true on `finish()` or on an
//! `error` chunk: a reader must drain the stream to EOF.
//!
//! Deliberate differences from the TypeScript:
//! - Text and thinking blocks are closed (`TextEnd`/`ThinkingEnd`) as soon as
//!   a block of another kind starts, and a later delta of that kind opens a
//!   new block. TS keeps one text and one thinking block per message and
//!   emits every `*_end` after the loop; the final content only differs for
//!   interleavings such as text → tool call → text, which TS folds into one
//!   text block.
//! - A `ToolcallDelta` is emitted only for a non-empty arguments fragment;
//!   TS also emits one, with an empty delta, for the id/name chunk.
//! - A stream ending without `finish_reason` keeps its text with `Stop` and
//!   is an error only when a tool call was cut off (TS fails the whole
//!   message); `compat.supportsFinishReason: false` is honoured as in TS.
//! - A plain `stop` is promoted to `ToolUse` whenever the message has tool
//!   calls; TS only does that for providers flagged as never sending
//!   `finish_reason`.
//! - A tool call the provider never gave an id gets a UUID at finalisation
//!   instead of the empty string, so tool results can still be paired.
//! - `reasoning_details` (OpenRouter replay metadata), custom/grammar tool
//!   calls, `responseId` and `responseModel` are not carried: the Rust
//!   `ContentBlock` and `AssistantMessage` have no field for them.

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::catalog::Model;
use crate::stream::{
    usage_from_value, AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason,
};
use crate::stream_decoder::{new_message, StreamDecoder};

/// The reasoning fields in the order TS checks them; the first non-empty one
/// wins so a provider that sends the same text under two names (chutes.ai)
/// is not doubled.
const REASONING_FIELDS: [&str; 3] = ["reasoning_content", "reasoning", "reasoning_text"];

/// The text or thinking block currently receiving deltas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    Text(usize),
    Thinking(usize),
}

/// A tool call still streaming its arguments.
#[derive(Debug, Clone)]
struct ToolSlot {
    content_index: usize,
    /// `tool_calls[].index`, the key later fragments are matched on; `None`
    /// while the provider has only identified the call by id.
    stream_index: Option<u64>,
    /// The argument JSON as it streams in.
    partial: String,
}

/// Decoder for the OpenAI chat-completions API and its compatible providers.
pub struct CompletionsDecoder {
    model: Model,
    message: AssistantMessage,
    open: Option<Open>,
    tool_calls: Vec<ToolSlot>,
    started: bool,
    finish_reason_seen: bool,
    done: bool,
}

impl CompletionsDecoder {
    pub fn new(model: &Model) -> Self {
        Self {
            model: model.clone(),
            message: new_message(model),
            open: None,
            tool_calls: Vec::new(),
            started: false,
            finish_reason_seen: false,
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

    /// Close the text or thinking block receiving deltas, if any.
    fn close_open(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        match self.open.take() {
            Some(Open::Text(content_index)) => {
                let content = match self.message.content.get(content_index) {
                    Some(ContentBlock::Text { text }) => text.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::TextEnd {
                    content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            Some(Open::Thinking(content_index)) => {
                let content = match self.message.content.get(content_index) {
                    Some(ContentBlock::Thinking { thinking }) => thinking.clone(),
                    _ => String::new(),
                };
                out.push(AssistantMessageEvent::ThinkingEnd {
                    content_index,
                    content,
                    partial: self.message.clone(),
                });
            }
            None => {}
        }
    }

    /// TS `ensureTextBlock`: the open text block, or a new one after closing
    /// whatever else was open.
    fn ensure_text(&mut self, out: &mut Vec<AssistantMessageEvent>) -> usize {
        if let Some(Open::Text(content_index)) = self.open {
            return content_index;
        }
        self.close_open(out);
        let content_index = self.message.content.len();
        self.message.content.push(ContentBlock::Text {
            text: String::new(),
        });
        self.open = Some(Open::Text(content_index));
        out.push(AssistantMessageEvent::TextStart {
            content_index,
            partial: self.message.clone(),
        });
        content_index
    }

    /// TS `ensureThinkingBlock`.
    fn ensure_thinking(&mut self, out: &mut Vec<AssistantMessageEvent>) -> usize {
        if let Some(Open::Thinking(content_index)) = self.open {
            return content_index;
        }
        self.close_open(out);
        let content_index = self.message.content.len();
        self.message.content.push(ContentBlock::Thinking {
            thinking: String::new(),
        });
        self.open = Some(Open::Thinking(content_index));
        out.push(AssistantMessageEvent::ThinkingStart {
            content_index,
            partial: self.message.clone(),
        });
        content_index
    }

    fn append_text(&mut self, delta: &str, out: &mut Vec<AssistantMessageEvent>) {
        let content_index = self.ensure_text(out);
        if let Some(ContentBlock::Text { text }) = self.message.content.get_mut(content_index) {
            text.push_str(delta);
        }
        out.push(AssistantMessageEvent::TextDelta {
            content_index,
            delta: delta.to_string(),
            partial: self.message.clone(),
        });
    }

    fn append_thinking(&mut self, delta: &str, out: &mut Vec<AssistantMessageEvent>) {
        let content_index = self.ensure_thinking(out);
        if let Some(ContentBlock::Thinking { thinking }) =
            self.message.content.get_mut(content_index)
        {
            thinking.push_str(delta);
        }
        out.push(AssistantMessageEvent::ThinkingDelta {
            content_index,
            delta: delta.to_string(),
            partial: self.message.clone(),
        });
    }

    fn block_id(&self, content_index: usize) -> &str {
        match self.message.content.get(content_index) {
            Some(ContentBlock::ToolCall { id, .. }) => id,
            _ => "",
        }
    }

    /// TS `ensureToolCallBlock` plus the per-fragment handling: a fragment is
    /// matched to its call by `index`, then by `id`; one with neither
    /// continues call 0. Later fragments may repeat the id and name, and the
    /// arguments arrive as string pieces appended to a per-call buffer.
    fn tool_call_fragment(&mut self, fragment: &Value, out: &mut Vec<AssistantMessageEvent>) {
        if !fragment.is_object() {
            return;
        }
        let id = fragment
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let stream_index = match fragment.get("index").and_then(Value::as_u64) {
            Some(index) => Some(index),
            None if id.is_none() => Some(0),
            None => None,
        };
        let name = fragment
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let arguments = fragment
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or("");

        let found = stream_index
            .and_then(|index| {
                self.tool_calls
                    .iter()
                    .position(|slot| slot.stream_index == Some(index))
            })
            .or_else(|| {
                id.and_then(|id| {
                    self.tool_calls
                        .iter()
                        .position(|slot| self.block_id(slot.content_index) == id)
                })
            });
        let position = match found {
            Some(position) => position,
            None => {
                self.close_open(out);
                let content_index = self.message.content.len();
                self.message.content.push(ContentBlock::ToolCall {
                    id: id.unwrap_or_default().to_string(),
                    name: name.to_string(),
                    arguments: Value::Object(Map::new()),
                });
                self.tool_calls.push(ToolSlot {
                    content_index,
                    stream_index,
                    partial: String::new(),
                });
                out.push(AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.message.clone(),
                });
                self.tool_calls.len() - 1
            }
        };

        let slot = &mut self.tool_calls[position];
        if slot.stream_index.is_none() {
            slot.stream_index = stream_index;
        }
        let content_index = slot.content_index;
        if !arguments.is_empty() {
            slot.partial.push_str(arguments);
        }
        if let Some(ContentBlock::ToolCall {
            id: block_id,
            name: block_name,
            arguments: block_arguments,
        }) = self.message.content.get_mut(content_index)
        {
            if block_id.is_empty() {
                if let Some(id) = id {
                    *block_id = id.to_string();
                }
            }
            if block_name.is_empty() && !name.is_empty() {
                *block_name = name.to_string();
            }
            if !arguments.is_empty() {
                *block_arguments = parse_streaming_arguments(&slot.partial, block_arguments);
            }
        }
        if !arguments.is_empty() {
            out.push(AssistantMessageEvent::ToolcallDelta {
                content_index,
                delta: arguments.to_string(),
                partial: self.message.clone(),
            });
        }
    }

    /// TS `finishBlock` for every tool call still open, in content order: the
    /// arguments become the parsed buffer (`{}` when empty or unparseable), an
    /// id the provider never sent is generated, and `ToolcallEnd` carries the
    /// finished block.
    fn finalize_tool_calls(&mut self, out: &mut Vec<AssistantMessageEvent>) {
        for slot in std::mem::take(&mut self.tool_calls) {
            let Some(ContentBlock::ToolCall { id, arguments, .. }) =
                self.message.content.get_mut(slot.content_index)
            else {
                continue;
            };
            *arguments = final_arguments(&slot.partial);
            if id.is_empty() {
                *id = Uuid::new_v4().to_string();
            }
            let tool_call = self.message.content[slot.content_index].clone();
            out.push(AssistantMessageEvent::ToolcallEnd {
                content_index: slot.content_index,
                tool_call,
                partial: self.message.clone(),
            });
        }
    }

    fn has_tool_call(&self) -> bool {
        self.message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }

    /// TS `compat.supportsFinishReason`: true unless the model's compat says
    /// otherwise, in which case every stream from that provider ends without
    /// one and that is not a cut-off.
    fn expects_finish_reason(&self) -> bool {
        self.model
            .compat
            .get("supportsFinishReason")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    /// `finish_reason` arrived: record the stop reason and close every open
    /// block. The terminal event waits for `finish()`, because the usage chunk
    /// may still follow.
    fn apply_finish_reason(&mut self, reason: &str, out: &mut Vec<AssistantMessageEvent>) {
        self.finish_reason_seen = true;
        let (stop_reason, error_message) = map_stop_reason(reason);
        self.message.stop_reason = Some(stop_reason);
        self.message.error_message = error_message;
        self.close_open(out);
        self.finalize_tool_calls(out);
    }

    /// Emit the terminal event: `Error` for an error stop, else `Done`, with a
    /// plain `Stop` promoted to `ToolUse` when the message has tool calls.
    fn conclude(&mut self, out: &mut Vec<AssistantMessageEvent>) {
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

    /// An `{"error": …}` chunk: the openai SDK throws on it, so TS ends the
    /// stream right there with an error event.
    fn fail(&mut self, error_message: String, out: &mut Vec<AssistantMessageEvent>) {
        self.close_open(out);
        self.finalize_tool_calls(out);
        self.message.stop_reason = Some(StopReason::Error);
        self.message.error_message = Some(error_message);
        self.conclude(out);
    }
}

impl StreamDecoder for CompletionsDecoder {
    fn feed(&mut self, chunk: &Value, out: &mut Vec<AssistantMessageEvent>) {
        if self.done {
            return;
        }
        self.start(out);
        if !chunk.is_object() {
            return;
        }
        if let Some(error) = chunk.get("error").filter(|error| !error.is_null()) {
            self.fail(error_text(error), out);
            return;
        }
        let chunk_usage = chunk.get("usage").filter(|usage| usage.is_object());
        if let Some(usage) = chunk_usage {
            self.message.usage = Some(completions_usage(&self.model, usage));
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        // Some providers (Moonshot) put the usage on the choice instead.
        if chunk_usage.is_none() {
            if let Some(usage) = choice.get("usage").filter(|usage| usage.is_object()) {
                self.message.usage = Some(completions_usage(&self.model, usage));
            }
        }
        if let Some(delta) = choice.get("delta").filter(|delta| delta.is_object()) {
            if let Some(content) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
            {
                self.append_text(content, out);
            }
            let reasoning = REASONING_FIELDS.iter().find_map(|field| {
                delta
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|reasoning| !reasoning.is_empty())
            });
            if let Some(reasoning) = reasoning {
                self.append_thinking(reasoning, out);
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for fragment in tool_calls {
                    self.tool_call_fragment(fragment, out);
                }
            }
        }
        if let Some(reason) = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
        {
            self.apply_finish_reason(reason, out);
        }
    }

    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage {
        if !self.done {
            self.start(out);
            if !self.finish_reason_seen {
                // The connection closed before finish_reason. Text already
                // received is worth keeping; a tool call cut off mid-arguments
                // is not, because executing it would guess at what the model
                // meant. A provider flagged as never sending finish_reason
                // ends every stream this way, so its tool calls are complete.
                if !self.tool_calls.is_empty() && self.expects_finish_reason() {
                    self.message.stop_reason = Some(StopReason::Error);
                    self.message.error_message =
                        Some("Stream ended before the tool call was complete".into());
                } else {
                    self.message.stop_reason = Some(StopReason::Stop);
                }
            }
            self.close_open(out);
            self.finalize_tool_calls(out);
            self.conclude(out);
        }
        self.message.clone()
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

/// TS `parseChunkUsage`, through the shared cache-aware mapping, plus the
/// reasoning token count the chat-completions shape carries.
fn completions_usage(model: &Model, usage: &Value) -> pi_protocol::Usage {
    let mut computed = usage_from_value(model, usage);
    if let Some(reasoning) = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
    {
        computed.reasoning = Some(reasoning);
    }
    computed
}

/// TS `mapStopReason` in `openai-completions.ts`: anything but the known
/// reasons (`content_filter` and `network_error` included) is an error naming
/// the provider's value.
fn map_stop_reason(reason: &str) -> (StopReason, Option<String>) {
    match reason {
        "stop" | "end" => (StopReason::Stop, None),
        "length" => (StopReason::Length, None),
        "function_call" | "tool_calls" => (StopReason::ToolUse, None),
        other => (
            StopReason::Error,
            Some(format!("Provider finish_reason: {other}")),
        ),
    }
}

/// TS `parseStreamingJson` while the arguments stream, without its repair
/// pass: the buffer if it parses as an object, else the last value that did.
fn parse_streaming_arguments(buffer: &str, previous: &Value) -> Value {
    match serde_json::from_str::<Value>(buffer) {
        Ok(value @ Value::Object(_)) => value,
        _ if previous.is_object() => previous.clone(),
        _ => Value::Object(Map::new()),
    }
}

/// The finished arguments: the parsed buffer, `{}` when empty or unparseable.
fn final_arguments(buffer: &str) -> Value {
    match serde_json::from_str::<Value>(buffer) {
        Ok(value @ Value::Object(_)) => value,
        _ => Value::Object(Map::new()),
    }
}

/// The message of an `error` payload, else its code, else the raw JSON.
fn error_text(error: &Value) -> String {
    if let Some(text) = error.as_str() {
        return text.to_string();
    }
    if let Some(message) = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
    {
        return message.to_string();
    }
    match error.get("code") {
        Some(Value::String(code)) if !code.is_empty() => code.clone(),
        Some(code) if code.is_number() => code.to_string(),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;
    use crate::stream_decoder::frames_of;

    fn model() -> Model {
        Model {
            id: "gpt-4o".into(),
            name: "gpt-4o".into(),
            api: "openai-completions".into(),
            provider: "openai".into(),
            base_url: None,
            reasoning: false,
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

    fn run_with(model: &Model, corpus: &str) -> (AssistantMessage, Vec<AssistantMessageEvent>) {
        let mut decoder = CompletionsDecoder::new(model);
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

    fn tool_call(block: &ContentBlock) -> (&str, &str, &Value) {
        match block {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => (id, name, arguments),
            other => panic!("expected a tool call, got {other:?}"),
        }
    }

    const ONE_TOOL_CALL_OPENING: &str = r#"
data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"read","arguments":""}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\""}}]},"finish_reason":null}]}
"#;

    #[test]
    fn plain_text_ends_with_the_done_sentinel() {
        let corpus = r#"
data: {"id":"chatcmpl-1","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
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
        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentBlock::Text { text } if text == "Hello world")
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(message.error_message, None);
        assert!(message.usage.is_none());
        assert_eq!(message.model, "openai/gpt-4o");
        match events.last() {
            Some(AssistantMessageEvent::Done { reason, message }) => {
                assert_eq!(*reason, StopReason::Stop);
                assert_eq!(message.content.len(), 1);
            }
            other => panic!("expected done, got {other:?}"),
        }
    }

    #[test]
    fn a_usage_only_trailing_chunk_lands_on_the_final_message() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"},"finish_reason":null}],"usage":null}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null}

data: {"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":5,"total_tokens":125,"prompt_tokens_details":{"cached_tokens":100},"completion_tokens_details":{"reasoning_tokens":2}}}

data: [DONE]
"#;
        let mut decoder = CompletionsDecoder::new(&model());
        let mut out = Vec::new();
        let frames = frames_of(corpus);
        assert_eq!(frames.len(), 3);
        decoder.feed(&frames[0].data, &mut out);
        decoder.feed(&frames[1].data, &mut out);
        // finish_reason closes the blocks but the stream is not done: the
        // usage chunk is still to come.
        assert!(!decoder.is_done());
        assert_eq!(
            names(&out),
            ["start", "text_start", "text_delta", "text_end"]
        );
        decoder.feed(&frames[2].data, &mut out);
        assert!(!decoder.is_done());
        let message = decoder.finish(&mut out);
        assert!(decoder.is_done());
        assert_eq!(names(&out).last(), Some(&"done"));

        let usage = message.usage.clone().expect("usage");
        assert_eq!(usage.input, 20);
        assert_eq!(usage.cache_read, 100);
        assert_eq!(usage.output, 5);
        assert_eq!(usage.total_tokens, 125);
        assert_eq!(usage.reasoning, Some(2));
        match out.last() {
            Some(AssistantMessageEvent::Done { message, .. }) => {
                assert_eq!(message.usage.as_ref().map(|usage| usage.input), Some(20));
            }
            other => panic!("expected done, got {other:?}"),
        }
    }

    #[test]
    fn usage_on_the_same_chunk_as_finish_reason_is_kept() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}
"#;
        let (message, _) = run(corpus);
        let usage = message.usage.expect("usage");
        assert_eq!((usage.input, usage.output, usage.total_tokens), (10, 1, 11));
    }

    #[test]
    fn usage_on_the_choice_is_read_when_the_chunk_has_none() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop","usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}]}
"#;
        let (message, _) = run(corpus);
        let usage = message.usage.expect("usage");
        assert_eq!((usage.input, usage.output), (7, 3));
    }

    #[test]
    fn one_tool_call_streamed_in_three_argument_fragments() {
        let corpus = format!(
            r#"{ONE_TOOL_CALL_OPENING}
data: {{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":": \"Cargo"}}}}]}},"finish_reason":null}}]}}

data: {{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":".toml\"}}"}}}}]}},"finish_reason":null}}]}}

data: {{"choices":[{{"index":0,"delta":{{}},"finish_reason":"tool_calls"}}]}}

data: [DONE]
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
                "toolcall_end",
                "done"
            ]
        );
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(message.content.len(), 1);
        let (id, name, arguments) = tool_call(&message.content[0]);
        assert_eq!(id, "call_abc");
        assert_eq!(name, "read");
        assert_eq!(arguments, &serde_json::json!({"path": "Cargo.toml"}));

        // While the buffer does not parse the arguments stay at the last
        // parsed value, `{}`; the last fragment completes them.
        let deltas: Vec<(&str, &Value)> = events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::ToolcallDelta { delta, partial, .. } => {
                    Some((delta.as_str(), tool_call(&partial.content[0]).2))
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas[0].0, "{\"path\"");
        assert_eq!(deltas[0].1, &serde_json::json!({}));
        assert_eq!(deltas[1].0, ": \"Cargo");
        assert_eq!(deltas[1].1, &serde_json::json!({}));
        assert_eq!(deltas[2].0, ".toml\"}");
        assert_eq!(deltas[2].1, &serde_json::json!({"path": "Cargo.toml"}));

        match &events[5] {
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
    }

    #[test]
    fn two_parallel_tool_calls_interleave_by_index() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":""}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"ls","arguments":""}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"a\""}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"path\":\"b\""}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"}"}},{"index":1,"function":{"arguments":"}"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;
        let (message, events) = run(corpus);
        assert_eq!(message.content.len(), 2);
        let (id, name, arguments) = tool_call(&message.content[0]);
        assert_eq!((id, name), ("call_a", "read"));
        assert_eq!(arguments, &serde_json::json!({"path": "a"}));
        let (id, name, arguments) = tool_call(&message.content[1]);
        assert_eq!((id, name), ("call_b", "ls"));
        assert_eq!(arguments, &serde_json::json!({"path": "b"}));
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));

        let delta_indices: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::ToolcallDelta { content_index, .. } => Some(*content_index),
                _ => None,
            })
            .collect();
        assert_eq!(delta_indices, [0, 1, 0, 1]);
        let end_indices: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::ToolcallEnd { content_index, .. } => Some(*content_index),
                _ => None,
            })
            .collect();
        assert_eq!(end_indices, [0, 1]);
        assert_eq!(names(&events).last(), Some(&"done"));
    }

    #[test]
    fn reasoning_content_then_content_closes_the_thinking_block_first() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Think"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"reasoning_content":" hard"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"content":"Answer"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
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
            matches!(&message.content[0], ContentBlock::Thinking { thinking } if thinking == "Think hard")
        );
        assert!(matches!(&message.content[1], ContentBlock::Text { text } if text == "Answer"));
        match &events[4] {
            AssistantMessageEvent::ThinkingEnd { content, .. } => assert_eq!(content, "Think hard"),
            other => panic!("expected thinking_end, got {other:?}"),
        }
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
    }

    #[test]
    fn the_first_non_empty_reasoning_field_wins() {
        // chutes.ai sends the same text as reasoning_content and reasoning;
        // OpenRouter sends `reasoning`; some endpoints `reasoning_text`.
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"reasoning_content":"same","reasoning":"same"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"reasoning_content":"","reasoning":" more"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"reasoning_text":"!"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}
"#;
        let (message, events) = run(corpus);
        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentBlock::Thinking { thinking } if thinking == "same more!")
        );
        assert_eq!(
            names(&events)
                .iter()
                .filter(|name| **name == "thinking_delta")
                .count(),
            3
        );
    }

    #[test]
    fn finish_reasons_map_like_the_typescript() {
        let (message, events) = run(
            r#"data: {"choices":[{"index":0,"delta":{"content":"I"},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}]}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Provider finish_reason: content_filter")
        );
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "I"));
        assert_eq!(
            names(&events),
            ["start", "text_start", "text_delta", "text_end", "error"]
        );

        let (message, events) = run(
            r#"data: {"choices":[{"index":0,"delta":{"content":"a very long"},"finish_reason":"length"}]}"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Length));
        assert_eq!(message.error_message, None);
        assert_eq!(names(&events).last(), Some(&"done"));

        let (message, _) =
            run(r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"end"}]}"#);
        assert_eq!(message.stop_reason, Some(StopReason::Stop));

        let (message, _) =
            run(r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"eos"}]}"#);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Provider finish_reason: eos")
        );

        // A provider that says `stop` despite tool calls still means tool use.
        let corpus = format!(
            r#"{ONE_TOOL_CALL_OPENING}
data: {{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":":\"x\"}}"}}}}]}},"finish_reason":"stop"}}]}}
"#
        );
        let (message, _) = run(&corpus);
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(
            tool_call(&message.content[0]).2,
            &serde_json::json!({"path": "x"})
        );
    }

    #[test]
    fn an_error_chunk_ends_the_stream_with_its_message() {
        let mut decoder = CompletionsDecoder::new(&model());
        let mut out = Vec::new();
        decoder.feed(
            &serde_json::json!({"choices":[{"index":0,"delta":{"content":"par"},"finish_reason":null}]}),
            &mut out,
        );
        decoder.feed(
            &serde_json::json!({"error":{"message":"Rate limit reached","type":"rate_limit_error","code":"rate_limit_exceeded"}}),
            &mut out,
        );
        assert!(decoder.is_done());
        assert_eq!(
            names(&out),
            ["start", "text_start", "text_delta", "text_end", "error"]
        );
        // Anything after the error is ignored, and finish adds nothing.
        decoder.feed(
            &serde_json::json!({"choices":[{"index":0,"delta":{"content":"late"},"finish_reason":"stop"}]}),
            &mut out,
        );
        let message = decoder.finish(&mut out);
        assert_eq!(out.len(), 5);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(message.error_message.as_deref(), Some("Rate limit reached"));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "par"));
        match &out[4] {
            AssistantMessageEvent::Error { reason, error } => {
                assert_eq!(*reason, StopReason::Error);
                assert_eq!(error.error_message.as_deref(), Some("Rate limit reached"));
            }
            other => panic!("expected error, got {other:?}"),
        }

        let (message, events) = run(r#"data: {"error":{"code":"insufficient_quota"}}"#);
        assert_eq!(message.error_message.as_deref(), Some("insufficient_quota"));
        assert_eq!(names(&events), ["start", "error"]);

        let (message, _) = run(r#"data: {"error":"upstream is down"}"#);
        assert_eq!(message.error_message.as_deref(), Some("upstream is down"));

        // `"error": null` is not an error.
        let (message, _) = run(
            r#"data: {"error":null,"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
    }

    #[test]
    fn a_stream_cut_mid_tool_call_is_an_error() {
        let (message, events) = run(ONE_TOOL_CALL_OPENING);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(
            message.error_message.as_deref(),
            Some("Stream ended before the tool call was complete")
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
        // The half-received arguments are not guessed at.
        assert_eq!(tool_call(&message.content[0]).2, &serde_json::json!({}));
    }

    #[test]
    fn a_stream_cut_mid_text_keeps_the_text() {
        let (message, events) = run(
            r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"half"},"finish_reason":null}]}
"#,
        );
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(message.error_message, None);
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "half"));
        assert_eq!(
            names(&events),
            ["start", "text_start", "text_delta", "text_end", "done"]
        );

        // Nothing at all still concludes cleanly.
        let (message, events) = run("");
        assert!(message.content.is_empty());
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert_eq!(names(&events), ["start", "done"]);
    }

    #[test]
    fn providers_that_never_send_finish_reason_still_get_tool_use() {
        let mut lenient = model();
        lenient.compat = serde_json::json!({"supportsFinishReason": false});
        let corpus = format!(
            r#"{ONE_TOOL_CALL_OPENING}
data: {{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":":\"Cargo.toml\"}}"}}}}]}},"finish_reason":null}}]}}
"#
        );
        let (message, events) = run_with(&lenient, &corpus);
        assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(message.error_message, None);
        assert_eq!(
            tool_call(&message.content[0]).2,
            &serde_json::json!({"path": "Cargo.toml"})
        );
        assert_eq!(names(&events).last(), Some(&"done"));
    }

    #[test]
    fn text_after_a_tool_call_opens_a_new_text_block() {
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"content":"Let me look."},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"ls","arguments":"{}"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"content":"Done."},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}
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
                "text_start",
                "text_delta",
                "text_end",
                "toolcall_end",
                "done"
            ]
        );
        assert_eq!(message.content.len(), 3);
        assert!(
            matches!(&message.content[0], ContentBlock::Text { text } if text == "Let me look.")
        );
        assert_eq!(tool_call(&message.content[1]).1, "ls");
        assert!(matches!(&message.content[2], ContentBlock::Text { text } if text == "Done."));
    }

    #[test]
    fn fragments_without_index_continue_the_first_call_and_a_missing_id_is_generated() {
        // Some compatible servers omit `index` (and repeat the name), and
        // some never send an id at all.
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"tool_calls":[{"type":"function","function":{"name":"read","arguments":"{\"path\":"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"function":{"name":"read","arguments":"\"a.rs\"}"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}
"#;
        let (message, events) = run(corpus);
        assert_eq!(message.content.len(), 1);
        let (id, name, arguments) = tool_call(&message.content[0]);
        assert_eq!(id.len(), 36, "a UUID stands in for the missing id");
        assert_eq!(name, "read");
        assert_eq!(arguments, &serde_json::json!({"path": "a.rs"}));
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

        // A call first seen by id alone is found again by that id, and an id
        // that only arrives on a later fragment is adopted.
        let corpus = r#"
data: {"choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_x","function":{"name":"ls","arguments":"{\"path\""}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_x","index":0,"function":{"arguments":":\".\"}"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"name":"read","arguments":"{}"}}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_y"}]},"finish_reason":null}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}
"#;
        let (message, _) = run(corpus);
        assert_eq!(message.content.len(), 2);
        let (id, name, arguments) = tool_call(&message.content[0]);
        assert_eq!((id, name), ("call_x", "ls"));
        assert_eq!(arguments, &serde_json::json!({"path": "."}));
        let (id, name, _) = tool_call(&message.content[1]);
        assert_eq!((id, name), ("call_y", "read"));
    }

    #[test]
    fn malformed_chunks_are_ignored() {
        let mut decoder = CompletionsDecoder::new(&model());
        let mut out = Vec::new();
        for chunk in [
            serde_json::json!("not an object"),
            serde_json::json!(42),
            serde_json::json!({}),
            serde_json::json!({"choices": "nope"}),
            serde_json::json!({"choices": []}),
            serde_json::json!({"choices": [null]}),
            serde_json::json!({"choices": [{"delta": null, "finish_reason": null}]}),
            serde_json::json!({"choices": [{"delta": {"content": 7, "reasoning": [], "tool_calls": "x"}}]}),
            serde_json::json!({"choices": [{"delta": {"tool_calls": [null, 5, "s"]}}]}),
            serde_json::json!({"choices": [{"delta": {"content": "ok"}, "finish_reason": 3}]}),
            serde_json::json!({"usage": "none", "choices": [{"delta": {}, "finish_reason": "stop"}]}),
        ] {
            decoder.feed(&chunk, &mut out);
        }
        let message = decoder.finish(&mut out);
        assert_eq!(message.content.len(), 1);
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "ok"));
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert!(message.usage.is_none());
        assert_eq!(
            names(&out),
            ["start", "text_start", "text_delta", "text_end", "done"]
        );
    }

    #[test]
    fn feeding_after_finish_is_ignored() {
        let mut decoder = CompletionsDecoder::new(&model());
        let mut out = Vec::new();
        decoder.feed(
            &serde_json::json!({"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}),
            &mut out,
        );
        let message = decoder.finish(&mut out);
        assert!(decoder.is_done());
        decoder.feed(
            &serde_json::json!({"choices":[{"index":0,"delta":{"content":"late"},"finish_reason":null}]}),
            &mut out,
        );
        let again = decoder.finish(&mut out);
        assert_eq!(again, message);
        assert_eq!(
            names(&out),
            ["start", "text_start", "text_delta", "text_end", "done"]
        );
    }
}
