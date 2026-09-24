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
use davinci_protocol::{ThinkingLevel, Usage};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub thinking_level: Option<ThinkingLevel>,
    pub thinking_budgets: Option<ThinkingBudgets>,
    /// Provider HTTP idle timeout per socket read, in milliseconds. Defaults to 300 seconds.
    pub timeout_ms: Option<u64>,
    pub max_retries: Option<u32>,
    pub max_retry_delay_ms: Option<u64>,
    pub max_tokens: Option<u64>,
    pub websocket_connect_timeout_ms: Option<u64>,
    pub transport: Option<String>,
    pub session_id: Option<String>,
    pub cache_key: Option<String>,
    pub cache_retention: Option<String>,
    /// Latest durable native Responses turn from this real conversation.
    /// It is validated against the current provider projection and stable
    /// model-visible request contract before use.
    pub native_responses_resume: Option<crate::responses_ledger::NativeResponsesResumeRecord>,
    pub install_telemetry: Option<bool>,
    /// Set from another thread to stop a live stream between frames. The
    /// message then closes with `StopReason::Aborted`.
    pub abort_signal: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[allow(unused_imports)]
pub use crate::cache::effective_prompt_cache_key;

fn codex_responses_affinity_id(options: &StreamOptions) -> Option<String> {
    if crate::cache::cache_retention_from_options(options) == crate::cache::CacheRetention::None {
        return None;
    }
    let conversation_id = options.session_id.as_deref().filter(|id| !id.is_empty())?;

    // Only an actual root/session-owned conversation may align its Codex
    // affinity header to a cache partition. Graph workers intentionally run
    // without session_id, so a shared worker cache key cannot become their
    // continuation/socket identity.
    let raw = options
        .cache_key
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(conversation_id);
    Some(crate::cache::clamp_openai_prompt_cache_key(raw))
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
        /// Anthropic's signature, or the opaque payload for redacted thinking.
        /// Required to replay this block in the next request of a tool loop.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        redacted: bool,
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

/// Shares message snapshots across stream events and refreshes them at a
/// bounded rate while content is arriving.
pub(crate) struct PartialMessageSnapshot {
    current: Arc<AssistantMessage>,
    last_refresh: Instant,
    bytes_since_refresh: usize,
}

impl PartialMessageSnapshot {
    pub(crate) fn new(message: &AssistantMessage) -> Self {
        Self {
            current: Arc::new(message.clone()),
            last_refresh: Instant::now(),
            bytes_since_refresh: 0,
        }
    }

    pub(crate) fn update(
        &mut self,
        message: &AssistantMessage,
        appended_bytes: usize,
    ) -> Arc<AssistantMessage> {
        self.bytes_since_refresh = self.bytes_since_refresh.saturating_add(appended_bytes);
        if self.bytes_since_refresh >= 4 * 1024
            || self.last_refresh.elapsed() >= Duration::from_millis(50)
        {
            self.refresh(message);
        }
        Arc::clone(&self.current)
    }

    pub(crate) fn force(&mut self, message: &AssistantMessage) -> Arc<AssistantMessage> {
        self.refresh(message);
        Arc::clone(&self.current)
    }

    fn refresh(&mut self, message: &AssistantMessage) {
        self.current = Arc::new(message.clone());
        self.last_refresh = Instant::now();
        self.bytes_since_refresh = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantMessageEvent {
    #[serde(rename = "start")]
    Start { partial: Arc<AssistantMessage> },
    #[serde(rename = "text_start")]
    TextStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "text_end")]
    TextEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        content: String,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "thinking_start")]
    ThinkingStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "thinking_end")]
    ThinkingEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        content: String,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "toolcall_start")]
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "toolcall_delta")]
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
        partial: Arc<AssistantMessage>,
    },
    #[serde(rename = "toolcall_end")]
    ToolcallEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(rename = "toolCall")]
        tool_call: ContentBlock,
        partial: Arc<AssistantMessage>,
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
            | Self::ToolcallEnd { partial, .. } => partial.as_ref(),
            Self::Done { message, .. } => message,
            Self::Error { error, .. } => error,
        }
    }
}

pub type StreamEvent = AssistantMessageEvent;

#[derive(Debug, Clone)]
pub struct ProviderCompletionEnvelope {
    pub message: AssistantMessage,
    pub stream_events: Vec<AssistantMessageEvent>,
    pub native_responses: Option<crate::responses_ledger::NativeResponsesTurn>,
}

fn native_responses_api(model: &Model) -> bool {
    matches!(
        model.api.as_str(),
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses"
    )
}

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

/// Replay a recorded SSE corpus through the decoder its frames call for.
/// Fixtures replay any provider's stream through any model, so the frames
/// are sniffed before the model's API is consulted.
pub fn replay_sse_events(model: &Model, corpus: &str) -> Vec<AssistantMessageEvent> {
    let frames = crate::stream_decoder::frames_of(corpus);
    let mut decoder = decoder_for_frames(model, &frames);
    let mut events = Vec::new();
    for frame in &frames {
        decoder.feed(&frame.data, &mut events);
    }
    decoder.finish(&mut events);
    events
}

fn decoder_for_frames(
    model: &Model,
    frames: &[crate::stream_decoder::SseFrame],
) -> Box<dyn crate::stream_decoder::StreamDecoder> {
    use crate::stream_decoder::ResponsesDecoder;
    use crate::stream_decoder_anthropic::AnthropicDecoder;
    use crate::stream_decoder_completions::CompletionsDecoder;
    for frame in frames {
        let kind = frame.data.get("type").and_then(Value::as_str).unwrap_or("");
        if kind.starts_with("response.") {
            return Box::new(ResponsesDecoder::new(model));
        }
        if matches!(
            kind,
            "message_start"
                | "content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"
        ) {
            return Box::new(AnthropicDecoder::new(model));
        }
        if frame.data.get("choices").is_some() {
            return Box::new(CompletionsDecoder::new(model));
        }
    }
    crate::stream_decoder::decoder_for(model)
        .unwrap_or_else(|| Box::new(CompletionsDecoder::new(model)))
}

/// Read a provider's event stream as it arrives. Every frame goes through
/// `decoder` and every event it yields reaches `on_event` before the next
/// frame is read, so a token is painted the moment it lands. A body that
/// turns out not to be an event stream (a provider that ignored `stream`, an
/// error document) is parsed whole instead.
fn read_provider_stream(
    response: ureq::Response,
    model: &Model,
    decoder: &mut dyn crate::stream_decoder::StreamDecoder,
    abort: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<
    (
        AssistantMessage,
        Vec<AssistantMessageEvent>,
        Option<crate::responses_ledger::NativeResponsesOutput>,
    ),
    String,
> {
    use std::sync::atomic::Ordering;

    // The body is read on its own thread and handed over line by line, so the
    // abort flag is checked every few milliseconds even while the provider is
    // silent — a stalled request answers `esc` at once instead of at its next
    // token. When the receiver goes away the reader ends at its next line and
    // the connection closes with it.
    let line_rx = crate::stream_reader::response_lines(response.into_reader());
    let mut framer = crate::stream_decoder::SseFramer::default();
    let mut events = Vec::new();
    let mut raw_events = Vec::new();
    let mut raw = String::new();
    let mut frames = 0usize;
    let mut aborted = false;
    let mut read_error: Option<String> = None;

    let feed = |data: &Value,
                decoder: &mut dyn crate::stream_decoder::StreamDecoder,
                events: &mut Vec<AssistantMessageEvent>,
                on_event: &mut dyn FnMut(&AssistantMessageEvent)| {
        let start = events.len();
        decoder.feed(data, events);
        for event in &events[start..] {
            on_event(event);
        }
    };

    loop {
        if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            aborted = true;
            break;
        }
        let line = match line_rx.recv_timeout(Duration::from_millis(40)) {
            Ok(Ok(line)) => line,
            Ok(Err(err)) => {
                crate::trace::log(&format!("sse read error after {frames} frames: {err}"));
                read_error = Some(format!("Unable to read provider response: {err}"));
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if frames == 0 {
            if raw.len().saturating_add(line.len()) > crate::stream_reader::MAX_FRAME_BYTES {
                read_error = Some("non-streaming provider response exceeds 16 MiB".into());
                break;
            }
            raw.push_str(&line);
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if let Some(frame) = framer.feed_line(trimmed) {
            frames += 1;
            raw.clear();
            if crate::trace::enabled() {
                crate::trace::log(&format!(
                    "sse frame {}",
                    crate::trace::describe_event(&frame.data)
                ));
            }
            raw_events.push(frame.data.clone());
            feed(&frame.data, decoder, &mut events, on_event);
            if decoder.is_done() {
                break;
            }
        }
        if framer.saw_done() {
            break;
        }
    }
    // A connection torn down after the stream's own end (`[DONE]`, the
    // terminal event) is how some servers hang up; it is not a failure of
    // the reply. A drop before that point is reported below.
    if read_error.is_some() && frames > 0 && (framer.saw_done() || decoder.is_done()) {
        read_error = None;
    }
    if !aborted && read_error.is_none() && !decoder.is_done() {
        if let Some(frame) = framer.flush() {
            frames += 1;
            raw.clear();
            raw_events.push(frame.data.clone());
            feed(&frame.data, decoder, &mut events, on_event);
        }
    }

    if frames == 0 && !aborted {
        if let Some(err) = read_error {
            return Err(err);
        }
        if crate::trace::enabled() {
            // Response bodies can contain user-visible output, provider prompt
            // fragments in errors, or opaque reasoning. Trace only the size.
            crate::trace::log(&format!(
                "body was not an event stream ({} bytes)",
                raw.len()
            ));
        }
        let message = parse_provider_response(model, &raw);
        let synthesized = events_from_complete(&message);
        for event in &synthesized {
            on_event(event);
        }
        let native = native_responses_api(model)
            .then(|| serde_json::from_str::<Value>(&raw).ok())
            .flatten()
            .and_then(|value| {
                crate::responses_ledger::NativeResponsesOutput::from_response_value(&value)
            });
        return Ok((message, synthesized, native));
    }

    if aborted {
        let mut closing = Vec::new();
        let mut message = decoder.finish(&mut closing);
        message.stop_reason = Some(StopReason::Aborted);
        message.error_message = Some("Request was aborted".into());
        let event = AssistantMessageEvent::Error {
            reason: StopReason::Aborted,
            error: message.clone(),
        };
        on_event(&event);
        events.push(event);
        return Ok((message, events, None));
    }

    let start = events.len();
    let mut message = decoder.finish(&mut events);
    if let Some(err) = read_error.filter(|_| message.stop_reason != Some(StopReason::Error)) {
        // The connection dropped after the stream began: keep what arrived,
        // say why it stopped.
        message.stop_reason = Some(StopReason::Error);
        message.error_message = Some(err);
        events.truncate(start);
        events.push(AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error: message.clone(),
        });
    }
    for event in &events[start..] {
        on_event(event);
    }
    let native = if native_responses_api(model)
        && message.stop_reason != Some(StopReason::Error)
        && message.stop_reason != Some(StopReason::Aborted)
    {
        crate::responses_ledger::NativeResponsesOutput::from_events(&raw_events)
    } else {
        None
    };
    Ok((message, events, native))
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
                ContentBlock::Thinking {
                    thinking,
                    signature,
                    redacted,
                } => MessageContent::Thinking {
                    thinking: thinking.clone(),
                    redacted: redacted.then_some(true),
                    signature: signature.clone(),
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
    let prepared = crate::responses_request::PreparedProviderRequest::new(body);
    let body = prepared.body();
    if crate::trace::enabled() {
        crate::trace::log(&format!(
            "prepared request segments={} prefix={} bytes={}",
            prepared.manifest().segments.len(),
            prepared.manifest().ordered_prefix_fingerprint,
            prepared.manifest().request_bytes_before_compression
        ));
    }
    if model.api == "openai-codex-responses" {
        if let Some(token) = auth.api_key.as_deref() {
            let codex_affinity_id = codex_responses_affinity_id(options);
            match crate::codex::try_codex_websocket_transport_with_affinity(
                model,
                body,
                token,
                options.transport.as_deref(),
                options.session_id.as_deref(),
                codex_affinity_id.as_deref(),
                options.cache_retention.as_deref(),
                options.websocket_connect_timeout_ms,
                options.timeout_ms,
                options.abort_signal.as_ref(),
                &mut |_| {},
            ) {
                Ok(crate::codex::CodexWebsocketOutcome::Message(message)) => {
                    return Ok(message.message)
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
        &collect_request_headers(model, auth, options),
    );
    let timeout_ms = options.timeout_ms.filter(|ms| *ms > 0);
    let compress_zstd = model.api == "openai-codex-responses";
    let text = crate::provider_retry::retry_provider_request_controlled(
        || send_provider_body(&url, &headers, body, timeout_ms, compress_zstd),
        crate::provider_retry::ProviderRetryOptions {
            max_retries: options.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.max_retry_delay_ms,
        },
        options.abort_signal.as_ref(),
        |ms| std::thread::sleep(Duration::from_millis(ms)),
    )
    .map_err(|err| err.message)?;
    Ok(parse_provider_response(model, &text))
}

/// Streaming complete: the events the provider sent, replayed after the fact.
/// `live_complete_streaming_with_sink` is the same request with a sink that
/// sees each event as it arrives.
pub fn live_complete_streaming_with(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Result<(AssistantMessage, Vec<AssistantMessageEvent>), String> {
    live_complete_streaming_with_sink(model, messages, auth, system, tools, options, &mut |_| {})
}

/// Streaming complete with a live sink: `on_event` is called for every stream
/// event the moment it is decoded, before the next frame is read. APIs with an
/// incremental decoder are asked for `stream: true`; the rest are requested
/// whole and their events synthesised, so tool calls work everywhere and
/// tokens arrive live wherever the wire format is understood.
pub fn live_complete_streaming_with_sink(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<(AssistantMessage, Vec<AssistantMessageEvent>), String> {
    let envelope = live_complete_streaming_with_sink_envelope(
        model, messages, auth, system, tools, options, on_event,
    )?;
    Ok((envelope.message, envelope.stream_events))
}

pub fn live_complete_streaming_with_sink_envelope(
    model: &Model,
    messages: &[ChatMessage],
    auth: &ResolvedAuth,
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<ProviderCompletionEnvelope, String> {
    let incremental = crate::stream_decoder::supports_incremental_stream(model);
    let mut body = request_body_with(model, messages, system, tools, options);
    if crate::trace::enabled() {
        crate::trace::log(&format!(
            "request {}/{} api={} incremental={} messages={} tools={}",
            model.provider,
            model.id,
            model.api,
            incremental,
            messages.len(),
            tools.len()
        ));
    }
    if incremental {
        if let Value::Object(map) = &mut body {
            map.insert("stream".into(), Value::Bool(true));
            // TS asks completions endpoints for a trailing usage chunk.
            if model.api == "openai-completions" && !map.contains_key("stream_options") {
                map.insert(
                    "stream_options".into(),
                    serde_json::json!({"include_usage": true}),
                );
            }
        }
    }
    let prepared = crate::responses_request::PreparedProviderRequest::new(body);
    let body = prepared.body();
    if crate::trace::enabled() {
        crate::trace::log(&format!(
            "prepared stream request segments={} prefix={} bytes={}",
            prepared.manifest().segments.len(),
            prepared.manifest().ordered_prefix_fingerprint,
            prepared.manifest().request_bytes_before_compression
        ));
    }
    if model.api == "openai-codex-responses" {
        if let Some(token) = auth.api_key.as_deref() {
            let mut collected = Vec::new();
            let codex_affinity_id = codex_responses_affinity_id(options);
            let outcome = crate::codex::try_codex_websocket_transport_with_affinity(
                model,
                body,
                token,
                options.transport.as_deref(),
                options.session_id.as_deref(),
                codex_affinity_id.as_deref(),
                options.cache_retention.as_deref(),
                options.websocket_connect_timeout_ms,
                options.timeout_ms,
                options.abort_signal.as_ref(),
                &mut |event| {
                    collected.push(event.clone());
                    on_event(event);
                },
            );
            match outcome {
                Ok(crate::codex::CodexWebsocketOutcome::Message(message)) => {
                    if crate::trace::enabled() {
                        crate::trace::log(&format!(
                            "websocket reply stop={:?} blocks={} error={:?}",
                            message.message.stop_reason,
                            message.message.content.len(),
                            message.message.error_message
                        ));
                    }
                    let events = if collected.is_empty() {
                        let synthesized = events_from_complete(&message.message);
                        for event in &synthesized {
                            on_event(event);
                        }
                        synthesized
                    } else {
                        collected
                    };
                    let native_responses = message.native_responses.and_then(|output| {
                        crate::responses_ledger::NativeResponsesTurn::from_prepared(
                            &prepared, output,
                        )
                    });
                    return Ok(ProviderCompletionEnvelope {
                        message: message.message,
                        stream_events: events,
                        native_responses,
                    });
                }
                Ok(crate::codex::CodexWebsocketOutcome::FallbackToSse) => {
                    crate::trace::log("websocket fell back to sse");
                }
                Err(error) => {
                    crate::trace::log(&format!("websocket failed: {error}"));
                    return Err(error);
                }
            }
        }
    }
    let url = request_url(model, auth);
    let headers = crate::merge_provider_attribution_headers(
        model,
        options.session_id.as_deref(),
        options.install_telemetry,
        &collect_request_headers(model, auth, options),
    );
    let timeout_ms = options.timeout_ms.filter(|ms| *ms > 0);
    let compress_zstd = model.api == "openai-codex-responses";
    crate::trace::log(&format!("sse post {}", crate::trace::redact_url(&url)));
    let response = crate::provider_retry::retry_provider_request_controlled(
        || send_provider_request(&url, &headers, body, timeout_ms, compress_zstd),
        crate::provider_retry::ProviderRetryOptions {
            max_retries: options.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.max_retry_delay_ms,
        },
        options.abort_signal.as_ref(),
        |ms| std::thread::sleep(Duration::from_millis(ms)),
    )
    .map_err(|err| {
        crate::trace::log(&format!("sse request failed: {}", err.message));
        err.message
    })?;
    crate::trace::log(&format!("sse status {}", response.status()));
    if model.api == "openai-codex-responses" {
        let response_headers: Vec<(String, String)> = response
            .headers_names()
            .into_iter()
            .filter_map(|name| {
                response
                    .header(&name)
                    .map(|value| (name.clone(), value.to_string()))
            })
            .collect();
        if let Some(snapshot) = crate::codex_usage::parse_usage_headers(&response_headers) {
            crate::codex_usage::record(snapshot);
        }
    }
    match crate::stream_decoder::decoder_for(model).filter(|_| incremental) {
        Some(mut decoder) => {
            let (message, stream_events, native_output) = read_provider_stream(
                response,
                model,
                decoder.as_mut(),
                options.abort_signal.as_ref(),
                on_event,
            )?;
            let native_responses = native_output.and_then(|output| {
                crate::responses_ledger::NativeResponsesTurn::from_prepared(&prepared, output)
            });
            Ok(ProviderCompletionEnvelope {
                message,
                stream_events,
                native_responses,
            })
        }
        None => {
            let text = response
                .into_string()
                .map_err(|err| format!("Unable to read provider response: {err}"))?;
            let (message, events) = if text.contains("data:") {
                let events = replay_sse_events(model, &text);
                let message = complete_from_events(&events)
                    .or_else(|| events.last().map(|event| event.message().clone()))
                    .ok_or_else(|| "Empty provider stream".to_string())?;
                (message, events)
            } else {
                let message = parse_provider_response(model, &text);
                let events = events_from_complete(&message);
                (message, events)
            };
            for event in &events {
                on_event(event);
            }
            let native_output = if native_responses_api(model) {
                if text.contains("data:") {
                    let raw_events = crate::stream_decoder::frames_of(&text)
                        .into_iter()
                        .map(|frame| frame.data)
                        .collect::<Vec<_>>();
                    crate::responses_ledger::NativeResponsesOutput::from_events(&raw_events)
                } else {
                    serde_json::from_str::<Value>(&text).ok().and_then(|value| {
                        crate::responses_ledger::NativeResponsesOutput::from_response_value(&value)
                    })
                }
            } else {
                None
            };
            let native_responses = native_output.and_then(|output| {
                crate::responses_ledger::NativeResponsesTurn::from_prepared(&prepared, output)
            });
            Ok(ProviderCompletionEnvelope {
                message,
                stream_events: events,
                native_responses,
            })
        }
    }
}

/// One raw provider exchange used by the explicit maintainer Codex probe.
#[derive(Debug, Clone)]
pub struct RawProviderReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Perform one direct provider POST without retries or response decoding.
/// Normal product traffic continues to use the standard retry/stream path.
pub fn raw_provider_post(
    model: &Model,
    auth: &ResolvedAuth,
    url: &str,
    body: &Value,
) -> Result<RawProviderReply, String> {
    let headers = collect_request_headers(model, auth, &StreamOptions::default());
    let mut request = crate::http::agent(crate::http::PROVIDER_IDLE_TIMEOUT).post(url);
    for (key, value) in &headers {
        request = request.set(key, value);
    }

    let result = if model.api == "openai-codex-responses" {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
        if compressed {
            request = request.set("content-encoding", "zstd");
        }
        request.send_bytes(&bytes)
    } else {
        request.send_string(&body.to_string())
    };

    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(error.to_string()),
    };
    let status = response.status();
    let headers = response
        .headers_names()
        .into_iter()
        .filter_map(|name| {
            response
                .header(&name)
                .map(|value| (name.clone(), value.to_string()))
        })
        .collect();
    let body = response
        .into_string()
        .map_err(|error| error.to_string())?;
    Ok(RawProviderReply {
        status,
        headers,
        body,
    })
}

fn collect_request_headers(
    model: &Model,
    auth: &ResolvedAuth,
    options: &StreamOptions,
) -> Vec<(String, String)> {
    let session_id = options.session_id.as_deref().filter(|id| !id.is_empty());
    let codex_affinity_id = codex_responses_affinity_id(options);
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
                    codex_affinity_id.as_deref(),
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
        if model.api == "google-generative-ai" {
            headers.push(("x-goog-api-key".into(), key.clone()));
        } else if model.api == "anthropic-messages" {
            headers.push(("x-api-key".into(), key.clone()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        } else {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        }
    }
    headers.push(("content-type".into(), "application/json".into()));

    let push_if_absent = |headers: &mut Vec<(String, String)>, name: &str, value: &str| {
        if !headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name))
        {
            headers.push((name.to_string(), value.to_string()));
        }
    };
    if let Some(session_id) = session_id {
        let retention = crate::cache::cache_retention_from_options(options);
        let affinity_format = model
            .compat
            .get("sessionAffinityFormat")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let openrouter = model.provider == "openrouter"
                    || model
                        .base_url
                        .as_deref()
                        .is_some_and(|url| url.contains("openrouter.ai"));
                if openrouter {
                    "openrouter".into()
                } else {
                    "openai".into()
                }
            });
        let opted_in = model
            .compat
            .get("sendSessionAffinityHeaders")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match model.api.as_str() {
            "anthropic-messages" | "pi-messages" => {
                if opted_in && retention != crate::cache::CacheRetention::None {
                    push_if_absent(&mut headers, "x-session-affinity", session_id);
                }
            }
            "openai-responses" | "azure-openai-responses" => {
                if affinity_format == "openrouter" {
                    push_if_absent(&mut headers, "x-session-id", session_id);
                } else {
                    if affinity_format == "openai" {
                        push_if_absent(&mut headers, "session_id", session_id);
                    }
                    push_if_absent(&mut headers, "x-client-request-id", session_id);
                }
            }
            "mistral-conversations" => {
                if retention != crate::cache::CacheRetention::None {
                    push_if_absent(&mut headers, "x-affinity", session_id);
                }
            }
            "openai-codex-responses" => {}
            _ => {
                if opted_in {
                    if affinity_format == "openrouter" {
                        push_if_absent(&mut headers, "x-session-id", session_id);
                    } else {
                        if affinity_format == "openai" {
                            push_if_absent(&mut headers, "session_id", session_id);
                        }
                        push_if_absent(&mut headers, "x-client-request-id", session_id);
                        push_if_absent(&mut headers, "x-session-affinity", session_id);
                    }
                }
            }
        }
    }
    headers
}

/// Send the request and hand back the response with its body unread, so the
/// caller can stream it. HTTP failures are already `ProviderError`s here,
/// which is what the retry loop inspects.
fn send_provider_request(
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    timeout_ms: Option<u64>,
    compress_zstd: bool,
) -> Result<ureq::Response, crate::provider_retry::ProviderError> {
    let idle = timeout_ms
        .filter(|timeout_ms| *timeout_ms > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or(crate::http::PROVIDER_IDLE_TIMEOUT);
    let mut request = crate::http::agent(idle).post(url);
    for (key, value) in headers {
        request = request.set(key, value);
    }
    if compress_zstd {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
        if compressed {
            request = request.set("content-encoding", "zstd");
        }
        request
            .send_bytes(&bytes)
            .map_err(crate::provider_retry::provider_error_from_ureq)
    } else {
        request
            .send_string(&body.to_string())
            .map_err(crate::provider_retry::provider_error_from_ureq)
    }
}

fn send_provider_body(
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    timeout_ms: Option<u64>,
    compress_zstd: bool,
) -> Result<String, crate::provider_retry::ProviderError> {
    let response = send_provider_request(url, headers, body, timeout_ms, compress_zstd)?;
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
        "bedrock-converse-stream" => bedrock_body(model, messages, system, tools, options),
        "mistral-conversations" => mistral_body(model, messages, system, tools, options),
        "openai-responses" | "openai-codex-responses" | "azure-openai-responses" => {
            openai_responses_body(model, messages, system, tools, options)
        }
        _ => openai_body(model, messages, system, tools, options),
    };
    apply_max_tokens_override(&mut body, options);
    apply_native_responses_resume(&mut body, model, messages, options);
    body
}

/// OpenAI Responses API body (TS `openai-responses.ts` `buildParams` /
/// `openai-codex-responses.ts` `buildRequestBody`). Chat-completions bodies
/// are rejected here; ChatGPT Codex additionally rejects a missing or true
/// `store` field ("Store must be set to false").
/// The `call_id` half of a tool call id. The Responses decoders persist
/// `call_id|item_id` (TS does the same), and a replayed `function_call` or
/// `function_call_output` carries only the `call_id`: the item id pairs the
/// call with a reasoning item that is never replayed here (no signature is
/// stored), and OpenAI rejects an unpaired `fc_…` id — as it rejects the
/// joined form, which is longer than the 64 characters a `call_id` may be.
pub(crate) fn responses_call_id(id: &str) -> &str {
    id.split_once('|').map(|(call_id, _)| call_id).unwrap_or(id)
}

fn verbosity_from(value: Option<&str>) -> &'static str {
    match value {
        Some("medium") => "medium",
        Some("high") => "high",
        _ => "low",
    }
}

fn summary_from(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("none") => None,
        Some("concise") => Some("concise"),
        Some("detailed") => Some("detailed"),
        _ => Some("auto"),
    }
}

fn openai_verbosity() -> &'static str {
    verbosity_from(std::env::var("DAVINCI_OPENAI_VERBOSITY").ok().as_deref())
}

fn reasoning_summary() -> Option<&'static str> {
    summary_from(std::env::var("DAVINCI_REASONING_SUMMARY").ok().as_deref())
}

pub const NATIVE_ITEMS_KEY: &str = "responsesOutputItems";
pub const NATIVE_MODEL_KEY: &str = "responsesOutputModel";

#[derive(Debug, Default, Clone, Copy)]
pub struct ResponsesInputOptions<'a> {
    /// `provider/model` whose saved output items may be replayed verbatim.
    pub native_items_model: Option<&'a str>,
    /// Tools represented as Responses custom/freeform tools.
    pub custom_tools: &'a [&'a str],
}

pub fn attach_native_items(chat: &mut ChatMessage, items: &[Value], model_key: &str) {
    if items.is_empty() {
        return;
    }
    chat.extra
        .insert(NATIVE_ITEMS_KEY.into(), Value::Array(items.to_vec()));
    chat.extra
        .insert(NATIVE_MODEL_KEY.into(), Value::String(model_key.into()));
}

fn native_items<'m>(
    message: &'m ChatMessage,
    model_key: Option<&str>,
) -> Option<&'m Vec<Value>> {
    let model_key = model_key?;
    if message.extra.get(NATIVE_MODEL_KEY).and_then(Value::as_str) != Some(model_key) {
        return None;
    }
    message.extra.get(NATIVE_ITEMS_KEY).and_then(Value::as_array)
}

#[doc(hidden)]
pub fn openai_responses_input(messages: &[ChatMessage]) -> Vec<Value> {
    openai_responses_input_with(messages, &ResponsesInputOptions::default())
}

#[doc(hidden)]
pub fn openai_responses_input_with(
    messages: &[ChatMessage],
    options: &ResponsesInputOptions<'_>,
) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        if message.role == "toolResult" {
            input.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": responses_call_id(message.tool_call_id.as_deref().unwrap_or_default()),
                "output": content_text(&message.content),
            }));
            continue;
        }
        if message.role == "assistant" {
            if let Some(items) = native_items(message, options.native_items_model) {
                input.extend(items.iter().cloned());
                continue;
            }
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
                        "call_id": responses_call_id(id),
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
        let role = match message.role.as_str() {
            "developer" => "developer",
            "system" => "system",
            _ => "user",
        };
        input.push(serde_json::json!({
            "type": "message",
            "role": role,
            "content": [{"type": "input_text", "text": text}],
        }));
    }
    input
}

fn openai_responses_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    let codex = model.api == "openai-codex-responses";
    let retention = crate::cache::cache_retention_from_options(options);
    let cache_capabilities = match model.api.as_str() {
        "openai-responses" => crate::openai_cache_policy::OpenAiCacheCapabilities::resolve(
            model,
            model.base_url.as_deref(),
            false,
        ),
        "openai-codex-responses" => {
            crate::openai_cache_policy::OpenAiCacheCapabilities::resolve(
                model,
                model.base_url.as_deref(),
                true,
            )
        }
        _ => crate::openai_cache_policy::OpenAiCacheCapabilities::unknown(),
    };
    let cache_capabilities = crate::openai_cache_policy::apply_runtime_features(
        cache_capabilities,
        retention,
        crate::openai_cache_policy::runtime_features(),
    );
    let trusted_system = system.filter(|value| !value.is_empty());
    let cache_plan = crate::openai_cache_policy::PromptCacheWirePlan::resolve(
        &cache_capabilities,
        retention,
        trusted_system.is_some(),
    );

    let model_key = format!("{}/{}", model.provider, model.id);
    let input_options = ResponsesInputOptions {
        native_items_model: Some(&model_key),
        custom_tools: &[],
    };
    let mut input = openai_responses_input_with(messages, &input_options);
    if cache_plan.use_stable_bootstrap_breakpoint {
        let text = trusted_system.expect("checked above");
        input.insert(
            0,
            serde_json::json!({
                "type": "message",
                "role": "developer",
                "content": [{
                    "type": "input_text",
                    "text": text,
                    "prompt_cache_breakpoint": {"mode": "explicit"}
                }],
            }),
        );
    }

    let instructions = trusted_system.unwrap_or("You are a helpful assistant.");
    let mut body = serde_json::json!({
        "model": model.id,
        "store": false,
        "stream": false,
        "input": input,
    });
    if !cache_plan.use_stable_bootstrap_breakpoint {
        body["instructions"] = Value::String(instructions.to_string());
    }
    if codex {
        body["text"] = serde_json::json!({"verbosity": openai_verbosity()});
        body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
        body["tool_choice"] = Value::String("auto".into());
        body["parallel_tool_calls"] = Value::Bool(true);
    }
    let session_key = crate::cache::effective_prompt_cache_key(options)
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
            if let Some(mode) = cache_plan.prompt_cache_mode {
                let mut prompt_cache_options = serde_json::json!({"mode": mode});
                if let Some(ttl) = cache_plan.prompt_cache_ttl {
                    prompt_cache_options["ttl"] = Value::String(ttl.into());
                }
                body["prompt_cache_options"] = prompt_cache_options;
            }
        }
        // Public Responses cache dialect is capability-scoped. The pure
        // wire planner is the sole authority for new cache fields.
        _ => {
            if cache_plan.emit_cache_key {
                if let Some(key) = session_key {
                    body["prompt_cache_key"] = Value::String(key);
                }
            }

            if let Some(retention) = cache_plan.legacy_retention {
                if crate::cache::supports_long_cache_retention(&model.compat) {
                    body["prompt_cache_retention"] = Value::String(retention.into());
                }
            }

            if let Some(mode) = cache_plan.prompt_cache_mode {
                let mut prompt_cache_options = serde_json::json!({"mode": mode});
                if let Some(ttl) = cache_plan.prompt_cache_ttl {
                    prompt_cache_options["ttl"] = Value::String(ttl.into());
                }
                body["prompt_cache_options"] = prompt_cache_options;
            }
            if let Some(comparison_response_id) =
                crate::openai_cache_diagnostics::configured_comparison_response_id(model, options)
            {
                if !body["prompt_cache_options"].is_object() {
                    body["prompt_cache_options"] = serde_json::json!({});
                }
                body["prompt_cache_options"]["comparison_response_id"] =
                    Value::String(comparison_response_id);
            }
        }
    }
    if !tools.is_empty() {
        let mut sorted_tools = tools.to_vec();
        sorted_tools.sort_by(|a, b| a.name.cmp(&b.name));
        body["tools"] = Value::Array(
            sorted_tools
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
                let mut reasoning = serde_json::json!({ "effort": effort });
                if let Some(summary) = reasoning_summary() {
                    reasoning["summary"] = Value::String(summary.into());
                }
                body["reasoning"] = reasoning;
            }
        }
    }
    body
}

fn apply_native_responses_resume(
    body: &mut Value,
    model: &Model,
    messages: &[ChatMessage],
    options: &StreamOptions,
) {
    if !native_responses_api(model)
        || !crate::openai_cache_policy::runtime_features().native_responses_replay
    {
        return;
    }
    let Some(resume) = options.native_responses_resume.as_ref() else {
        return;
    };
    if !resume.matches_provider_prefix(messages) {
        if crate::trace::enabled() {
            crate::trace::log("native responses replay skipped: provider prefix changed");
        }
        return;
    }

    let current = crate::responses_request::PreparedProviderRequest::new(body.clone());
    if !current
        .manifest()
        .stable_contract_compatible_with(&resume.turn.wire_manifest)
    {
        if crate::trace::enabled() {
            crate::trace::log("native responses replay skipped: stable request contract changed");
        }
        return;
    }

    let mut input = resume.turn.full_native_replay_prefix();
    let model_key = format!("{}/{}", model.provider, model.id);
    input.extend(openai_responses_input_with(
        &messages[resume.resume_provider_message_count..],
        &ResponsesInputOptions {
            native_items_model: Some(&model_key),
            custom_tools: &[],
        },
    ));
    body["input"] = Value::Array(input);
    if crate::trace::enabled() {
        crate::trace::log(&format!(
            "native responses replay applied baseline_messages={} replay_items={}",
            resume.resume_provider_message_count,
            body["input"].as_array().map(Vec::len).unwrap_or(0)
        ));
    }
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
    let prepared = crate::responses_request::PreparedProviderRequest::new(body);
    let body = prepared.body();
    let url = request_url(model, auth);
    let mut request = crate::http::agent(crate::http::PROVIDER_IDLE_TIMEOUT).post(&url);
    for (key, value) in &auth.headers {
        request = request.set(key, value);
    }
    for (key, value) in &model.headers {
        request = request.set(key, value);
    }
    if let Some(key) = &auth.api_key {
        if model.api == "google-generative-ai" {
            request = request.set("x-goog-api-key", key);
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
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
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
    let partial = Arc::new(message.clone());
    let mut events = vec![AssistantMessageEvent::Start {
        partial: Arc::clone(&partial),
    }];
    for (index, block) in message.content.iter().enumerate() {
        match block {
            ContentBlock::Text { text } => {
                events.push(AssistantMessageEvent::TextStart {
                    content_index: index,
                    partial: Arc::clone(&partial),
                });
                events.push(AssistantMessageEvent::TextDelta {
                    content_index: index,
                    delta: text.clone(),
                    partial: Arc::clone(&partial),
                });
                events.push(AssistantMessageEvent::TextEnd {
                    content_index: index,
                    content: text.clone(),
                    partial: Arc::clone(&partial),
                });
            }
            ContentBlock::Thinking { thinking, .. } => {
                events.push(AssistantMessageEvent::ThinkingStart {
                    content_index: index,
                    partial: Arc::clone(&partial),
                });
                events.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: index,
                    delta: thinking.clone(),
                    partial: Arc::clone(&partial),
                });
                events.push(AssistantMessageEvent::ThinkingEnd {
                    content_index: index,
                    content: thinking.clone(),
                    partial: Arc::clone(&partial),
                });
            }
            ContentBlock::ToolCall { .. } => {
                events.push(AssistantMessageEvent::ToolcallStart {
                    content_index: index,
                    partial: Arc::clone(&partial),
                });
                events.push(AssistantMessageEvent::ToolcallEnd {
                    content_index: index,
                    tool_call: block.clone(),
                    partial: Arc::clone(&partial),
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
    options: &StreamOptions,
) -> Value {
    let retention = crate::cache::cache_retention_from_options(options);
    let cache_point = if crate::cache::bedrock_supports_prompt_caching(
        model,
        std::env::var("AWS_BEDROCK_FORCE_CACHE").ok().as_deref(),
    ) {
        crate::cache::bedrock_cache_point(retention)
    } else {
        None
    };
    let mut converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": [{"text": content_text(&message.content)}],
            })
        })
        .collect();
    if let Some(cache_point) = cache_point.as_ref() {
        if let Some(last) = converted.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some("user") {
                if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
                    content.push(cache_point.clone());
                }
            }
        }
    }
    let mut body = serde_json::json!({
        "modelId": model.id,
        "messages": converted,
    });
    if let Some(system) = system {
        let mut blocks = vec![serde_json::json!({"text": system})];
        if let Some(cache_point) = cache_point.as_ref() {
            blocks.push(cache_point.clone());
        }
        body["system"] = Value::Array(blocks);
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
        if let Some(key) =
            crate::cache::effective_prompt_cache_key(options).filter(|id| !id.is_empty())
        {
            body["prompt_cache_key"] = Value::String(key.to_string());
        }
    }
    body
}

pub fn request_url(model: &Model, _auth: &ResolvedAuth) -> String {
    let base = model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let base = base.trim_end_matches('/');
    match model.api.as_str() {
        "anthropic-messages" | "pi-messages" => format!("{base}/v1/messages"),
        "google-generative-ai" => format!("{base}/models/{}:generateContent", model.id),
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
        if let Some(key) = crate::cache::effective_prompt_cache_key(options)
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
                let mut content: Vec<Value> = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        MessageContent::Thinking {
                            signature: Some(signature),
                            redacted,
                            thinking,
                        } => Some(if redacted.unwrap_or(false) {
                            serde_json::json!({"type":"redacted_thinking","data": signature})
                        } else {
                            serde_json::json!({"type":"thinking","thinking": thinking,"signature": signature})
                        }),
                        // Unsigned thinking cannot be replayed to Anthropic.
                        MessageContent::Thinking { .. } => None,
                        _ => None,
                    })
                    .collect();
                content.extend(message
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
                    }));
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

/// Cache-aware usage extraction across provider shapes. Mirrors:
/// - anthropic-messages.ts:604, 749-750 (input excludes cached; own keys)
/// - openai-completions.ts:1509-1531 (subtract cached + written from prompt)
/// - openai-responses-shared.ts:561-570 (subtract details from input_tokens)
/// - bedrock-converse-stream.ts:693 (camelCase converse keys)
pub(crate) fn usage_from_value(model: &Model, usage: &Value) -> Usage {
    let get = |key: &str| usage.get(key).and_then(Value::as_u64);
    let base_input = get("prompt_tokens")
        .or_else(|| get("input_tokens"))
        .or_else(|| get("inputTokens"))
        .unwrap_or(0);
    let output = get("completion_tokens")
        .or_else(|| get("output_tokens"))
        .or_else(|| get("outputTokens"))
        .unwrap_or(0);
    let anthropic_read = get("cache_read_input_tokens").or_else(|| get("cacheReadInputTokens"));
    let anthropic_write =
        get("cache_creation_input_tokens").or_else(|| get("cacheWriteInputTokens"));
    let (input, cache_read, cache_write) = if anthropic_read.is_some() || anthropic_write.is_some()
    {
        (
            base_input,
            anthropic_read.unwrap_or(0),
            anthropic_write.unwrap_or(0),
        )
    } else {
        let read = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| get("prompt_cache_hit_tokens"))
            .or_else(|| get("cached_tokens"))
            .unwrap_or(0);
        let write = usage
            .pointer("/prompt_tokens_details/cache_write_tokens")
            .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        (
            base_input.saturating_sub(read).saturating_sub(write),
            read,
            write,
        )
    };
    let mut computed = crate::calculate_usage(model, input, output, cache_read, cache_write);
    if let Some(total) = get("total_tokens").or_else(|| get("totalTokens")) {
        computed.total_tokens = total;
    }
    computed
}

/// google-generative-ai.ts:225-236 — usageMetadata mapping with cached tokens
/// subtracted from the prompt count.
fn usage_from_google_metadata(model: &Model, metadata: &Value) -> Usage {
    let get = |key: &str| metadata.get(key).and_then(Value::as_u64);
    let cached = get("cachedContentTokenCount").unwrap_or(0);
    let input = get("promptTokenCount").unwrap_or(0).saturating_sub(cached);
    let output = get("candidatesTokenCount").unwrap_or(0);
    let mut computed = crate::calculate_usage(model, input, output, cached, 0);
    if let Some(total) = get("totalTokenCount") {
        computed.total_tokens = total;
    }
    computed
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
    let usage = value
        .get("usage")
        .map(|usage| usage_from_value(model, usage))
        .or_else(|| {
            value
                .get("usageMetadata")
                .map(|metadata| usage_from_google_metadata(model, metadata))
        });
    let stop_reason = if content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    {
        Some(StopReason::ToolUse)
    } else {
        Some(StopReason::Stop)
    };
    // A 200 whose body is an error document (`{"error": …}` from a proxy or
    // a gateway) is not the assistant's reply, and an empty body is not a
    // reply either: report both as the failure they are.
    // The Responses object carries `"error": null` on success.
    let error_message = value
        .get("error")
        .filter(|error| !error.is_null())
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string())
        });
    let error_message = error_message.or_else(|| {
        raw.trim()
            .is_empty()
            .then(|| "provider returned an empty response".to_string())
    });
    if let Some(error) = error_message {
        return AssistantMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: Vec::new(),
            model: format!("{}/{}", model.provider, model.id),
            usage,
            stop_reason: Some(StopReason::Error),
            error_message: Some(error),
        };
    }
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
    fn openai_response_control_values_are_conservative() {
        assert_eq!(verbosity_from(Some("medium")), "medium");
        assert_eq!(verbosity_from(Some("loud")), "low");
        assert_eq!(verbosity_from(None), "low");
        assert_eq!(summary_from(Some("none")), None);
        assert_eq!(summary_from(Some("concise")), Some("concise"));
        assert_eq!(summary_from(None), Some("auto"));
    }

    #[test]
    fn assistant_message_replays_its_native_items_for_same_model() {
        let items = vec![
            serde_json::json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "enc",
                "summary": []
            }),
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{"type": "output_text", "text": "Done."}]
            }),
        ];
        let mut assistant = ChatMessage::text("assistant", "Done.");
        attach_native_items(&mut assistant, &items, "openai-codex/gpt-5.6-luna");
        let input = openai_responses_input_with(
            &[
                ChatMessage::text("user", "go"),
                assistant,
                ChatMessage::text("user", "next"),
            ],
            &ResponsesInputOptions {
                native_items_model: Some("openai-codex/gpt-5.6-luna"),
                custom_tools: &[],
            },
        );
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[2]["phase"], "final_answer");
        assert_eq!(input.len(), 4);
    }

    /// A loopback HTTP server that answers one POST with an SSE body in two
    /// halves, the second only once `release` fires (or after `patience`).
    /// Returns whether the release arrived in time.
    fn sse_server(
        first: &'static str,
        second: &'static str,
        patience: Duration,
    ) -> (
        String,
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<bool>,
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 65536];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(request.starts_with("POST "), "{request}");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            stream.write_all(first.as_bytes()).unwrap();
            stream.flush().unwrap();
            let released = release_rx.recv_timeout(patience).is_ok();
            stream.write_all(second.as_bytes()).unwrap();
            stream.flush().unwrap();
            let _ = stream.shutdown(std::net::Shutdown::Both);
            released
        });
        (format!("http://{addr}"), release_tx, server)
    }

    fn loopback_model(base_url: &str) -> Model {
        Model {
            id: "loop-1".into(),
            name: "loop-1".into(),
            api: "openai-completions".into(),
            provider: "loopback".into(),
            base_url: Some(base_url.to_string()),
            reasoning: false,
            input: vec!["text".into()],
            cost: crate::catalog::ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 8_000,
            max_tokens: 1_000,
            compat: Value::Null,
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    fn loopback_auth() -> ResolvedAuth {
        ResolvedAuth {
            api_key: Some("test-key".into()),
            headers: Default::default(),
            source: "test".into(),
        }
    }

    #[test]
    fn a_live_stream_delivers_each_frame_before_the_next_is_sent() {
        let (base, release, server) = sse_server(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            Duration::from_secs(5),
        );
        let model = loopback_model(&base);
        let mut seen = Vec::new();
        let (message, events) = live_complete_streaming_with_sink(
            &model,
            &[ChatMessage::text("user", "hi")],
            &loopback_auth(),
            None,
            &[],
            &StreamOptions::default(),
            &mut |event| {
                if let AssistantMessageEvent::TextDelta { delta, .. } = event {
                    seen.push(delta.clone());
                    // The first token has arrived: only now may the server
                    // send the rest. A client that buffered the whole body
                    // would never get here in time.
                    let _ = release.send(());
                }
            },
        )
        .expect("stream");
        assert!(
            server.join().unwrap(),
            "first delta must arrive before the second frame is sent"
        );
        assert_eq!(seen, ["Hel", "lo"]);
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "Hello"));
        assert_eq!(message.stop_reason, Some(StopReason::Stop));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
    }

    #[test]
    fn an_abort_flag_stops_a_stalled_stream_at_once() {
        let (base, _release, server) = sse_server(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
            Duration::from_secs(4),
        );
        let model = loopback_model(&base);
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = abort.clone();
        let started = std::time::Instant::now();
        let (message, events) = live_complete_streaming_with_sink(
            &model,
            &[ChatMessage::text("user", "hi")],
            &loopback_auth(),
            None,
            &[],
            &StreamOptions {
                abort_signal: Some(abort),
                ..StreamOptions::default()
            },
            &mut |event| {
                if matches!(event, AssistantMessageEvent::TextDelta { .. }) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            },
        )
        .expect("stream");
        // The server sits on the second frame for four seconds; the abort
        // must not wait for it.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "abort waited for the server"
        );
        assert_eq!(message.stop_reason, Some(StopReason::Aborted));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "Hel"));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Error {
                reason: StopReason::Aborted,
                ..
            })
        ));
        let _ = server.join();
    }

    #[test]
    fn replayed_tool_calls_carry_only_the_call_id_half() {
        let messages = vec![
            ChatMessage {
                role: "assistant".into(),
                content: vec![MessageContent::ToolCall {
                    id: "call_abc|fc_0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "a"}),
                }],
                ..ChatMessage::default()
            },
            ChatMessage::tool_result(
                "call_abc|fc_0123456789abcdef0123456789abcdef0123456789abcdef",
                "read",
                "hello",
                false,
            ),
        ];
        let input = openai_responses_input(&messages);
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["call_id"], "call_abc");
        assert!(input[0].get("id").is_none());
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_abc");
        // A plain id, from a provider that never joined one, passes through.
        assert_eq!(responses_call_id("call_plain"), "call_plain");
    }

    #[test]
    fn sse_lifecycle_matches_ts_event_names() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.provider == "openai")
            .expect("openai model");
        let corpus = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\ndata: [DONE]\n";
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
    fn anthropic_body_replays_signed_thinking_first_in_tool_turns() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic model");
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: vec![
                MessageContent::Thinking {
                    thinking: "plan".into(),
                    redacted: None,
                    signature: Some("SIG".into()),
                },
                MessageContent::Thinking {
                    thinking: String::new(),
                    redacted: Some(true),
                    signature: Some("OPAQUE".into()),
                },
                MessageContent::Thinking {
                    thinking: "unsigned".into(),
                    redacted: None,
                    signature: None,
                },
                MessageContent::ToolCall {
                    id: "t1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path":"a"}),
                },
            ],
            ..ChatMessage::default()
        };
        let body = anthropic_body(&model, &[assistant], None, &[], &StreamOptions::default());
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(
            content[0],
            serde_json::json!({"type":"thinking","thinking":"plan","signature":"SIG"})
        );
        assert_eq!(
            content[1],
            serde_json::json!({"type":"redacted_thinking","data":"OPAQUE"})
        );
        assert_eq!(content[2]["type"], "tool_use");
        assert_eq!(content.len(), 3);
    }

    #[test]
    fn thinking_signature_fields_preserve_legacy_serialized_shapes() {
        let old_block: ContentBlock = serde_json::from_value(serde_json::json!({
            "type": "thinking",
            "thinking": "plan",
        }))
        .unwrap();
        assert_eq!(
            old_block,
            ContentBlock::Thinking {
                thinking: "plan".into(),
                signature: None,
                redacted: false,
            }
        );
        assert_eq!(
            serde_json::to_value(&old_block).unwrap(),
            serde_json::json!({"type":"thinking","thinking":"plan"})
        );

        let old_message: MessageContent = serde_json::from_value(serde_json::json!({
            "type": "thinking",
            "thinking": "plan",
        }))
        .unwrap();
        assert_eq!(
            old_message,
            MessageContent::Thinking {
                thinking: "plan".into(),
                redacted: None,
                signature: None,
            }
        );
        assert_eq!(
            serde_json::to_value(&old_message).unwrap(),
            serde_json::json!({"type":"thinking","thinking":"plan"})
        );
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
    fn bodies_carry_explicit_cache_key_over_session_id() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-responses")
            .expect("openai responses model");
        let options = StreamOptions {
            session_id: Some("sess-ignore".into()),
            cache_key: Some("cache-explicit".into()),
            ..StreamOptions::default()
        };
        let body = request_body_with(&model, &[], None, &[], &options);
        assert_eq!(body["prompt_cache_key"], "cache-explicit");

        // Codex
        let mut codex = model.clone();
        codex.api = "openai-codex-responses".into();
        let codex_body = request_body_with(&codex, &[], None, &[], &options);
        assert_eq!(codex_body["prompt_cache_key"], "cache-explicit");

        // Azure
        let mut azure = model.clone();
        azure.api = "azure-openai-responses".into();
        let azure_body = request_body_with(&azure, &[], None, &[], &options);
        assert_eq!(azure_body["prompt_cache_key"], "cache-explicit");
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
    fn bedrock_body_adds_cache_points_for_claude() {
        let mut model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "bedrock-converse-stream")
            .expect("bedrock model");
        model.id = "anthropic.claude-fable-5".into();
        model.name = model.id.clone();
        let messages = vec![ChatMessage::text("user", "hello")];
        let body = request_body_with(
            &model,
            &messages,
            Some("sys"),
            &[],
            &StreamOptions {
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );
        let system = body["system"].as_array().unwrap();
        assert_eq!(system[0]["text"], "sys");
        assert_eq!(system[1]["cachePoint"]["type"], "default");
        let last = body["messages"].as_array().unwrap().last().unwrap();
        let content = last["content"].as_array().unwrap();
        assert_eq!(content.last().unwrap()["cachePoint"]["type"], "default");

        let long = request_body_with(
            &model,
            &messages,
            Some("sys"),
            &[],
            &StreamOptions {
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(long["system"][1]["cachePoint"]["ttl"], "1h");

        model.id = "amazon.nova-pro-v1:0".into();
        model.name = model.id.clone();
        let nova = request_body_with(
            &model,
            &messages,
            Some("sys"),
            &[],
            &StreamOptions {
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(nova["system"].as_array().unwrap().len(), 1);
        let nova_last = nova["messages"].as_array().unwrap().last().unwrap();
        assert!(nova_last["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|block| block.get("cachePoint").is_none()));

        model.id = "anthropic.claude-fable-5".into();
        model.name = model.id.clone();
        let none = request_body_with(
            &model,
            &messages,
            Some("sys"),
            &[],
            &StreamOptions {
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(none["system"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn usage_parses_cache_tokens_per_provider() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("model");
        let anthropic = usage_from_value(
            &model,
            &serde_json::json!({
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 100,
            }),
        );
        assert_eq!(anthropic.input, 10);
        assert_eq!(anthropic.cache_read, 900);
        assert_eq!(anthropic.cache_write, 100);
        assert!(anthropic.cost.total > 0.0);

        let completions = usage_from_value(
            &model,
            &serde_json::json!({
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "prompt_tokens_details": {"cached_tokens": 700, "cache_write_tokens": 100},
            }),
        );
        assert_eq!(completions.input, 200);
        assert_eq!(completions.cache_read, 700);
        assert_eq!(completions.cache_write, 100);

        let deepseek = usage_from_value(
            &model,
            &serde_json::json!({"prompt_tokens": 100, "completion_tokens": 1, "prompt_cache_hit_tokens": 40}),
        );
        assert_eq!(deepseek.cache_read, 40);
        assert_eq!(deepseek.input, 60);
        let kimi = usage_from_value(
            &model,
            &serde_json::json!({"prompt_tokens": 100, "completion_tokens": 1, "cached_tokens": 30}),
        );
        assert_eq!(kimi.cache_read, 30);
        assert_eq!(kimi.input, 70);

        let responses = usage_from_value(
            &model,
            &serde_json::json!({
                "input_tokens": 500,
                "output_tokens": 10,
                "input_tokens_details": {"cached_tokens": 450},
                "total_tokens": 510,
            }),
        );
        assert_eq!(responses.input, 50);
        assert_eq!(responses.cache_read, 450);
        assert_eq!(responses.total_tokens, 510);

        let bedrock = usage_from_value(
            &model,
            &serde_json::json!({
                "inputTokens": 25,
                "outputTokens": 5,
                "cacheReadInputTokens": 300,
                "cacheWriteInputTokens": 50,
            }),
        );
        assert_eq!(bedrock.input, 25);
        assert_eq!(bedrock.cache_read, 300);
        assert_eq!(bedrock.cache_write, 50);

        let odd = usage_from_value(
            &model,
            &serde_json::json!({"prompt_tokens": 10, "prompt_tokens_details": {"cached_tokens": 50}}),
        );
        assert_eq!(odd.input, 0);
    }

    #[test]
    fn usage_parses_google_metadata() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "google-generative-ai")
            .expect("google model");
        let usage = usage_from_google_metadata(
            &model,
            &serde_json::json!({
                "promptTokenCount": 1000,
                "candidatesTokenCount": 30,
                "totalTokenCount": 1030,
                "cachedContentTokenCount": 800,
            }),
        );
        assert_eq!(usage.input, 200);
        assert_eq!(usage.cache_read, 800);
        assert_eq!(usage.output, 30);
        assert_eq!(usage.total_tokens, 1030);
    }

    #[test]
    fn google_api_key_is_a_header_not_a_query_parameter() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "google-generative-ai")
            .expect("a built-in Gemini model");
        let auth = ResolvedAuth {
            api_key: Some("AIzaSECRET".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let url = request_url(&model, &auth);
        assert!(!url.contains("AIzaSECRET"), "{url}");
        assert!(!url.contains("key="), "{url}");
        let headers = collect_request_headers(&model, &auth, &StreamOptions::default());
        assert!(headers
            .iter()
            .any(|(name, value)| name == "x-goog-api-key" && value == "AIzaSECRET"));

        let vertex = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "google-vertex")
            .expect("a built-in Vertex model");
        let vertex_auth = ResolvedAuth {
            api_key: Some("ya29.ACCESS_TOKEN".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let headers = collect_request_headers(&vertex, &vertex_auth, &StreamOptions::default());
        assert!(headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value == "Bearer ya29.ACCESS_TOKEN"
        }));
    }

    #[test]
    fn google_live_stream_sends_the_api_key_header() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let content_length = String::from_utf8_lossy(&request[..header_end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|length| length.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
            }
            request_tx
                .send(String::from_utf8_lossy(&request[..header_end]).into_owned())
                .unwrap();
            let body = r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let mut model = load_builtin_models()
            .into_iter()
            .find(|model| model.api == "google-generative-ai")
            .expect("a built-in Gemini model");
        model.base_url = Some(format!("http://{addr}"));
        let auth = ResolvedAuth {
            api_key: Some("AIzaSECRET".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let events = live_stream(
            &model,
            &[ChatMessage::text("user", "hello")],
            &auth,
            None,
            &[],
        )
        .unwrap();
        let request = request_rx.recv().unwrap().to_ascii_lowercase();
        server.join().unwrap();

        assert!(
            request.contains("x-goog-api-key: aizasecret\r\n"),
            "{request}"
        );
        assert!(!request.lines().next().unwrap_or_default().contains("key="));
        assert!(!events.is_empty());
    }

    #[test]
    fn parse_provider_response_reads_cache_usage() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("model");
        let parsed = parse_provider_response(
            &model,
            r#"{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":3,"output_tokens":2,"cache_read_input_tokens":70,"cache_creation_input_tokens":7}}"#,
        );
        let usage = parsed.usage.expect("usage");
        assert_eq!(usage.cache_read, 70);
        assert_eq!(usage.cache_write, 7);
    }

    #[test]
    fn parse_provider_response_reports_error_documents_and_empty_bodies() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-completions")
            .expect("model");
        let parsed = parse_provider_response(
            &model,
            r#"{"error":{"message":"upstream unavailable","type":"server_error"}}"#,
        );
        assert_eq!(parsed.stop_reason, Some(StopReason::Error));
        assert_eq!(
            parsed.error_message.as_deref(),
            Some("upstream unavailable")
        );
        assert!(parsed.content.is_empty());

        let parsed = parse_provider_response(&model, "   ");
        assert_eq!(parsed.stop_reason, Some(StopReason::Error));
        assert!(parsed.error_message.is_some());

        // A Responses object says `"error": null` when nothing went wrong.
        let parsed = parse_provider_response(
            &model,
            r#"{"error":null,"output_text":"hi","choices":[{"message":{"content":"hi"}}]}"#,
        );
        assert_eq!(parsed.stop_reason, Some(StopReason::Stop));
        assert!(parsed.error_message.is_none());
    }

    #[test]
    fn parse_provider_response_reads_google_usage_metadata() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "google-generative-ai")
            .expect("google model");
        let parsed = parse_provider_response(
            &model,
            r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":5,"totalTokenCount":105,"cachedContentTokenCount":60}}"#,
        );
        let usage = parsed.usage.expect("usage");
        assert_eq!(usage.cache_read, 60);
        assert_eq!(usage.input, 40);
    }

    #[test]
    fn compaction_retention_none_strips_all_cache_fields() {
        let anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        // main.rs:644 passes cache_retention "none" for compaction requests,
        // matching TS compaction.ts:591.
        let options = StreamOptions {
            session_id: Some("sess-compact".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        };
        let body = request_body_with(
            &anthropic,
            &[ChatMessage::text("user", "summarize")],
            Some("sys"),
            &[],
            &options,
        );
        let raw = serde_json::to_string(&body).unwrap();
        assert!(!raw.contains("cache_control"));
        assert!(!raw.contains("prompt_cache_key"));
    }

    #[test]
    fn affinity_headers_follow_ts_rules() {
        let session = StreamOptions {
            session_id: Some("sess-aff".into()),
            ..StreamOptions::default()
        };
        let auth = crate::auth::ResolvedAuth {
            api_key: Some("k".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let header = |headers: &[(String, String)], name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };

        let mut anthropic = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .expect("anthropic");
        let plain = collect_request_headers(&anthropic, &auth, &session);
        assert_eq!(header(&plain, "x-session-affinity"), None);
        anthropic.compat = serde_json::json!({"sendSessionAffinityHeaders": true});
        let opted = collect_request_headers(&anthropic, &auth, &session);
        assert_eq!(
            header(&opted, "x-session-affinity"),
            Some("sess-aff".into())
        );
        let none = collect_request_headers(
            &anthropic,
            &auth,
            &StreamOptions {
                session_id: Some("sess-aff".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );
        assert_eq!(header(&none, "x-session-affinity"), None);

        let responses = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-responses")
            .expect("responses model");
        let resp_headers = collect_request_headers(&responses, &auth, &session);
        assert_eq!(
            header(&resp_headers, "x-client-request-id"),
            Some("sess-aff".into())
        );
        assert_eq!(header(&resp_headers, "session_id"), Some("sess-aff".into()));

        let mut openrouter = responses.clone();
        openrouter.provider = "openrouter".into();
        openrouter.base_url = Some("https://openrouter.ai/api/v1".into());
        let or_headers = collect_request_headers(&openrouter, &auth, &session);
        assert_eq!(header(&or_headers, "x-session-id"), Some("sess-aff".into()));
        assert_eq!(header(&or_headers, "x-client-request-id"), None);

        let mut completions = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "openai-completions")
            .expect("completions model");
        completions.provider = "openai".into();
        completions.base_url = Some("https://api.openai.com/v1".into());
        completions.compat = serde_json::Value::Null;
        let comp_plain = collect_request_headers(&completions, &auth, &session);
        assert_eq!(header(&comp_plain, "x-session-affinity"), None);
        let mut comp_opted_model = completions.clone();
        comp_opted_model.compat = serde_json::json!({"sendSessionAffinityHeaders": true});
        let comp_opted = collect_request_headers(&comp_opted_model, &auth, &session);
        assert_eq!(
            header(&comp_opted, "x-session-affinity"),
            Some("sess-aff".into())
        );
        assert_eq!(
            header(&comp_opted, "x-client-request-id"),
            Some("sess-aff".into())
        );
        assert_eq!(header(&comp_opted, "session_id"), Some("sess-aff".into()));

        let mut mistral = completions.clone();
        mistral.api = "mistral-conversations".into();
        mistral.compat = serde_json::Value::Null;
        let mistral_headers = collect_request_headers(&mistral, &auth, &session);
        assert_eq!(
            header(&mistral_headers, "x-affinity"),
            Some("sess-aff".into())
        );
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

    #[test]
    fn gpt6_astra_codex_request_uses_responses_instructions_without_sampling_fields() {
        let model = crate::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == "openai-codex" && model.id == "gpt-6-astra")
            .expect("gpt-6-astra");
        let messages = vec![ChatMessage::text("user", "Fix the parser.")];
        let body = request_body_with(
            &model,
            &messages,
            Some("ASTRA_SYSTEM_SENTINEL"),
            &[],
            &StreamOptions {
                thinking_level: Some(ThinkingLevel::Low),
                ..StreamOptions::default()
            },
        );
        assert_eq!(body["model"], "gpt-6-astra");
        assert_eq!(body["instructions"], "ASTRA_SYSTEM_SENTINEL");
        assert_eq!(body["store"], false);
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("top_logprobs").is_none());
        assert_ne!(body["reasoning"]["effort"], "none");
    }
}

#[cfg(test)]
mod openai_cache_wire_tests {
    use super::*;
    use crate::catalog::ModelCost;
    use serde_json::json;

    fn public_model(explicit: bool, base_url: &str) -> Model {
        Model {
            id: if explicit { "gpt-5.6-sol" } else { "gpt-5" }.into(),
            name: "cache-test".into(),
            api: "openai-responses".into(),
            provider: "openai".into(),
            base_url: Some(base_url.into()),
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.2,
            },
            context_window: 272_000,
            max_tokens: 128_000,
            compat: if explicit {
                json!({"supportsExplicitPromptCacheMode": true})
            } else {
                json!({})
            },
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn codex_affinity_uses_cache_partition_only_for_session_owned_root() {
        let root = StreamOptions {
            session_id: Some("conversation-1".into()),
            cache_key: Some("shared-root-partition".into()),
            ..StreamOptions::default()
        };
        assert_eq!(
            codex_responses_affinity_id(&root).as_deref(),
            Some("shared-root-partition")
        );

        let worker = StreamOptions {
            session_id: None,
            cache_key: Some("shared-worker-bootstrap".into()),
            ..StreamOptions::default()
        };
        assert_eq!(codex_responses_affinity_id(&worker), None);

        let disabled = StreamOptions {
            session_id: Some("conversation-1".into()),
            cache_key: Some("partition".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        };
        assert_eq!(codex_responses_affinity_id(&disabled), None);
    }

    #[test]
    fn responses_input_preserves_developer_and_system_authority() {
        let input = openai_responses_input(&[
            ChatMessage::text("developer", "developer rules"),
            ChatMessage::text("system", "system rules"),
            ChatMessage::text("user", "question"),
        ]);
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[1]["role"], "system");
        assert_eq!(input[2]["role"], "user");
    }

    #[test]
    fn verified_gpt56_moves_stable_system_to_developer_breakpoint() {
        let model = public_model(true, "https://api.openai.com/v1");
        let body = openai_responses_body(
            &model,
            &[ChatMessage::text("user", "variable tail")],
            Some("stable trusted bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("partition".into()),
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );

        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(
            body["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
        assert_eq!(body["prompt_cache_options"]["mode"], "implicit");
        assert_eq!(body["prompt_cache_options"]["ttl"], "30m");
        assert_eq!(body["prompt_cache_key"], "partition");
    }

    #[test]
    fn verified_gpt56_none_uses_explicit_mode_without_marker() {
        let model = public_model(true, "https://api.openai.com/v1");
        let body = openai_responses_body(
            &model,
            &[ChatMessage::text("user", "tail")],
            Some("trusted bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("must-not-be-sent".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );

        assert_eq!(body["instructions"], "trusted bootstrap");
        assert!(body.get("prompt_cache_key").is_none());
        assert_eq!(body["prompt_cache_options"]["mode"], "explicit");
        assert_eq!(body["prompt_cache_options"]["ttl"], "30m");
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.pointer("/content/0/prompt_cache_breakpoint").is_none()));
    }

    #[test]
    fn deceptive_proxy_never_receives_gpt56_cache_dialect() {
        let model = public_model(true, "https://api.openai.com.evil.test/v1");
        let body = openai_responses_body(
            &model,
            &[ChatMessage::text("user", "tail")],
            Some("trusted bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("legacy-key".into()),
                cache_retention: Some("none".into()),
                ..StreamOptions::default()
            },
        );

        assert_eq!(body["instructions"], "trusted bootstrap");
        assert!(body.get("prompt_cache_options").is_none());
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.pointer("/content/0/prompt_cache_breakpoint").is_none()));
    }

    #[test]
    fn durable_native_replay_replaces_generic_assistant_with_exact_provider_items() {
        let model = public_model(true, "https://api.openai.com/v1");
        let prior_messages = vec![ChatMessage::text("user", "first")];
        let prior_body = openai_responses_body(
            &model,
            &prior_messages,
            Some("stable bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("partition".into()),
                cache_retention: Some("short".into()),
                ..StreamOptions::default()
            },
        );
        let prior_prepared = crate::responses_request::PreparedProviderRequest::new(prior_body);
        let turn = crate::responses_ledger::NativeResponsesTurn::from_prepared(
            &prior_prepared,
            crate::responses_ledger::NativeResponsesOutput {
                response_id: Some("resp_1".into()),
                output_items: vec![
                    serde_json::json!({
                        "type":"reasoning",
                        "id":"rs_1",
                        "encrypted_content":"opaque"
                    }),
                    serde_json::json!({
                        "type":"message",
                        "id":"msg_1",
                        "role":"assistant",
                        "content":[{"type":"output_text","text":"answer"}]
                    }),
                ],
                final_response: None,
                terminal_event_type: "response.completed".into(),
            },
        )
        .unwrap();

        let assistant = ChatMessage::text("assistant", "answer");
        let mut resume_projection = prior_messages.clone();
        resume_projection.push(assistant.clone());
        let record = crate::responses_ledger::NativeResponsesResumeRecord {
            turn,
            resume_provider_message_count: resume_projection.len(),
            resume_provider_messages_fingerprint:
                crate::responses_ledger::provider_messages_fingerprint(&resume_projection),
        };

        let mut current = resume_projection;
        current.push(ChatMessage::tool_result("call_1", "read", "result", false));
        let body = request_body_with(
            &model,
            &current,
            Some("stable bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("partition".into()),
                cache_retention: Some("short".into()),
                native_responses_resume: Some(record),
                ..StreamOptions::default()
            },
        );
        let input = body["input"].as_array().unwrap();
        assert!(input.iter().any(|item| item["id"] == "rs_1"));
        assert!(input.iter().any(|item| item["id"] == "msg_1"));
        assert_eq!(
            input
                .iter()
                .filter(|item| {
                    item["role"] == "assistant"
                        && item.get("id").is_none()
                        && item.get("type").is_none()
                })
                .count(),
            0,
            "generic assistant projection must not duplicate the exact native output"
        );
        assert_eq!(
            input.last().unwrap()["type"],
            "function_call_output",
            "only the new tail follows the exact native replay prefix"
        );
    }

    #[test]
    fn older_public_responses_keeps_legacy_retention_fields() {
        let model = public_model(false, "https://api.openai.com/v1");
        let body = openai_responses_body(
            &model,
            &[ChatMessage::text("user", "tail")],
            Some("trusted bootstrap"),
            &[],
            &StreamOptions {
                cache_key: Some("legacy-key".into()),
                cache_retention: Some("long".into()),
                ..StreamOptions::default()
            },
        );

        assert_eq!(body["instructions"], "trusted bootstrap");
        assert_eq!(body["prompt_cache_key"], "legacy-key");
        assert_eq!(body["prompt_cache_retention"], "24h");
        assert!(body.get("prompt_cache_options").is_none());
    }
}
