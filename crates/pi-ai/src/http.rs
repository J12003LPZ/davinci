//! OpenAI-compatible and Anthropic Messages HTTP stream adapters.
//!
//! Tests parse fixture SSE strings and never open a socket.

use serde_json::{json, Value};

use crate::{
    assistant_message, AssistantContent, AssistantMessageEvent, Context, Message, Model,
    StopReason, StreamOptions, Usage,
};

#[derive(Debug, Clone, PartialEq)]
pub struct HttpChatRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

pub fn resolve_api_key(options: Option<&StreamOptions>, env_var: &str) -> Option<String> {
    if let Some(key) = options.and_then(|opts| opts.api_key.clone()) {
        if !key.is_empty() {
            return Some(key);
        }
    }
    std::env::var(env_var).ok().filter(|key| !key.is_empty())
}

pub fn join_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn user_content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(items) = content.as_array() {
        return items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| item.as_str().map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("");
    }
    content.to_string()
}

fn assistant_text(content: &[AssistantContent]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn openai_messages(context: &Context) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &context.system_prompt {
        messages.push(json!({"role": "system", "content": system}));
    }
    for message in &context.messages {
        match message {
            Message::User { content, .. } => {
                messages.push(json!({"role": "user", "content": user_content_text(content)}));
            }
            Message::Assistant { content, .. } => {
                messages.push(json!({"role": "assistant", "content": assistant_text(content)}));
            }
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let text = content
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": text,
                }));
            }
        }
    }
    messages
}

fn anthropic_messages(context: &Context) -> Vec<Value> {
    context
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::User { content, .. } => {
                Some(json!({"role": "user", "content": user_content_text(content)}))
            }
            Message::Assistant { content, .. } => {
                Some(json!({"role": "assistant", "content": assistant_text(content)}))
            }
            Message::ToolResult { .. } => None,
        })
        .collect()
}

pub fn build_openai_chat_request(
    model: &Model,
    context: &Context,
    options: Option<&StreamOptions>,
) -> Result<HttpChatRequest, String> {
    let api_key = resolve_api_key(options, "OPENAI_API_KEY")
        .ok_or_else(|| "missing API key: set options.api_key or OPENAI_API_KEY".to_string())?;
    Ok(HttpChatRequest {
        url: join_url(&model.base_url, "chat/completions"),
        headers: vec![
            ("Authorization".into(), format!("Bearer {api_key}")),
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "text/event-stream".into()),
        ],
        body: json!({
            "model": model.id,
            "messages": openai_messages(context),
            "stream": true,
        }),
    })
}

pub fn build_anthropic_messages_request(
    model: &Model,
    context: &Context,
    options: Option<&StreamOptions>,
) -> Result<HttpChatRequest, String> {
    let api_key = resolve_api_key(options, "ANTHROPIC_API_KEY")
        .ok_or_else(|| "missing API key: set options.api_key or ANTHROPIC_API_KEY".to_string())?;
    let mut body = json!({
        "model": model.id,
        "max_tokens": model.max_tokens,
        "messages": anthropic_messages(context),
        "stream": true,
    });
    if let Some(system) = &context.system_prompt {
        body["system"] = json!(system);
    }
    Ok(HttpChatRequest {
        url: join_url(&model.base_url, "v1/messages"),
        headers: vec![
            ("x-api-key".into(), api_key),
            ("anthropic-version".into(), "2023-06-01".into()),
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "text/event-stream".into()),
        ],
        body,
    })
}

fn error_events(model: &Model, message: impl Into<String>) -> Vec<AssistantMessageEvent> {
    let error = assistant_message(
        model,
        vec![AssistantContent::Text {
            text: String::new(),
        }],
        Usage::default(),
        StopReason::Error,
        Some(message.into()),
    );
    vec![
        AssistantMessageEvent::Start {
            partial: error.clone(),
        },
        AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error,
        },
    ]
}

fn map_openai_stop(reason: Option<&str>) -> StopReason {
    match reason {
        Some("length") => StopReason::Length,
        Some("tool_calls") => StopReason::ToolUse,
        Some("content_filter") => StopReason::Error,
        _ => StopReason::Stop,
    }
}

fn map_anthropic_stop(reason: Option<&str>) -> StopReason {
    match reason {
        Some("max_tokens") => StopReason::Length,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::Stop,
    }
}

fn success_events(
    model: &Model,
    deltas: &[String],
    stop: StopReason,
) -> Vec<AssistantMessageEvent> {
    let full = deltas.concat();
    let output_tokens = full.len() as i64;
    let message = assistant_message(
        model,
        vec![AssistantContent::Text { text: full }],
        Usage::with_tokens(0, output_tokens),
        stop,
        None,
    );
    let mut events = vec![AssistantMessageEvent::Start {
        partial: message.clone(),
    }];
    let mut acc = String::new();
    for delta in deltas {
        acc.push_str(delta);
        events.push(AssistantMessageEvent::TextDelta {
            delta: delta.clone(),
            partial: assistant_message(
                model,
                vec![AssistantContent::Text { text: acc.clone() }],
                Usage::default(),
                StopReason::Pending,
                None,
            ),
        });
    }
    events.push(AssistantMessageEvent::Done {
        reason: stop,
        message,
    });
    events
}

fn data_payloads(sse: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut event_name = String::new();
    for raw in sse.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            event_name.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = rest.trim().to_string();
            continue;
        }
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let data = rest.strip_prefix(' ').unwrap_or(rest).trim();
        if data.is_empty() {
            continue;
        }
        out.push((event_name.clone(), data.to_string()));
    }
    out
}

/// Parse an OpenAI-compatible `text/event-stream` body into stream events.
/// Never panics; HTTP/JSON failures become [`AssistantMessageEvent::Error`].
pub fn parse_openai_compatible_sse(sse: &str, model: &Model) -> Vec<AssistantMessageEvent> {
    let payloads = data_payloads(sse);
    if payloads.is_empty() {
        return error_events(model, "empty SSE stream");
    }

    let mut deltas = Vec::new();
    let mut stop = StopReason::Stop;
    let mut saw_terminal = false;
    let mut saw_chunk = false;

    for (_event, data) in payloads {
        if data == "[DONE]" {
            saw_terminal = true;
            break;
        }
        let chunk: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(error) => return error_events(model, format!("SSE JSON parse error: {error}")),
        };
        saw_chunk = true;
        let choice = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first());
        if let Some(reason) = choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(Value::as_str)
        {
            stop = map_openai_stop(Some(reason));
            saw_terminal = true;
        }
        if let Some(content) = choice
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(Value::as_str)
        {
            if !content.is_empty() {
                deltas.push(content.to_string());
            }
        }
    }

    if !saw_chunk && !saw_terminal {
        return error_events(model, "SSE stream contained no JSON chunks");
    }
    success_events(model, &deltas, stop)
}

/// Parse Anthropic Messages SSE (`event: content_block_delta`) into text deltas.
pub fn parse_anthropic_messages_sse(sse: &str, model: &Model) -> Vec<AssistantMessageEvent> {
    let payloads = data_payloads(sse);
    if payloads.is_empty() {
        return error_events(model, "empty SSE stream");
    }

    let mut deltas = Vec::new();
    let mut stop = StopReason::Stop;
    let mut saw_chunk = false;

    for (event_name, data) in payloads {
        let chunk: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(error) => return error_events(model, format!("SSE JSON parse error: {error}")),
        };
        let event_type = if event_name.is_empty() {
            chunk
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        } else {
            event_name
        };
        match event_type.as_str() {
            "content_block_delta" => {
                saw_chunk = true;
                let delta = chunk.get("delta");
                let is_text = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .map(|t| t == "text_delta")
                    .unwrap_or(true);
                if is_text {
                    if let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                        if !text.is_empty() {
                            deltas.push(text.to_string());
                        }
                    }
                }
            }
            "message_delta" => {
                saw_chunk = true;
                if let Some(reason) = chunk.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    stop = map_anthropic_stop(Some(reason));
                }
            }
            "message_stop"
            | "message_start"
            | "content_block_start"
            | "content_block_stop"
            | "ping" => {
                saw_chunk = true;
            }
            other if chunk.get("type").is_some() || !other.is_empty() => {
                saw_chunk = true;
            }
            _ => {}
        }
    }

    if !saw_chunk {
        return error_events(model, "SSE stream contained no JSON chunks");
    }
    success_events(model, &deltas, stop)
}

fn post_sse(request: &HttpChatRequest) -> Result<String, String> {
    let mut req = ureq::post(&request.url);
    for (name, value) in &request.headers {
        req = req.set(name, value);
    }
    match req.send_json(request.body.clone()) {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("read SSE body: {error}")),
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// POST `{base_url}/chat/completions` with `stream: true` and parse SSE deltas.
/// Failures become error events; this function never panics.
pub fn openai_compatible_stream(
    model: &Model,
    context: &Context,
    options: Option<StreamOptions>,
) -> Vec<AssistantMessageEvent> {
    let request = match build_openai_chat_request(model, context, options.as_ref()) {
        Ok(request) => request,
        Err(error) => return error_events(model, error),
    };
    match post_sse(&request) {
        Ok(body) => parse_openai_compatible_sse(&body, model),
        Err(error) => error_events(model, error),
    }
}

/// POST `{base_url}/v1/messages` with `stream: true` and parse Anthropic SSE.
pub fn anthropic_messages_stream(
    model: &Model,
    context: &Context,
    options: Option<StreamOptions>,
) -> Vec<AssistantMessageEvent> {
    let request = match build_anthropic_messages_request(model, context, options.as_ref()) {
        Ok(request) => request,
        Err(error) => return error_events(model, error),
    };
    match post_sse(&request) {
        Ok(body) => parse_anthropic_messages_sse(&body, model),
        Err(error) => error_events(model, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_model;

    fn fixture_model() -> Model {
        let mut model = test_model();
        model.base_url = "https://api.openai.com/v1".into();
        model
    }

    fn anthropic_model() -> Model {
        let mut model = test_model();
        model.api = "anthropic-messages".into();
        model.provider = "anthropic".into();
        model.base_url = "https://api.anthropic.com".into();
        model
    }

    fn empty_context() -> Context {
        Context {
            system_prompt: Some("sys".into()),
            messages: vec![Message::User {
                content: Value::String("hi".into()),
                timestamp: 0,
            }],
            tools: None,
        }
    }

    const OPENAI_SSE: &str = "\
data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\
\n\
data: [DONE]\n";

    const ANTHROPIC_SSE: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    fn text_deltas(events: &[AssistantMessageEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                AssistantMessageEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn openai_sse_fixture_emits_start_deltas_done() {
        let events = parse_openai_compatible_sse(OPENAI_SSE, &fixture_model());
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        assert_eq!(text_deltas(&events), vec!["Hello", " world"]);
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                ..
            })
        ));
        let Some(AssistantMessageEvent::Done { message, .. }) = events.last() else {
            panic!("expected done");
        };
        let Message::Assistant { content, .. } = message else {
            panic!("expected assistant");
        };
        assert_eq!(
            content,
            &vec![AssistantContent::Text {
                text: "Hello world".into()
            }]
        );
    }

    #[test]
    fn openai_sse_parse_failure_is_error_event() {
        let events = parse_openai_compatible_sse("data: {not-json\n", &fixture_model());
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Error {
                reason: StopReason::Error,
                ..
            })
        ));
    }

    #[test]
    fn openai_empty_sse_is_error_event() {
        let events = parse_openai_compatible_sse("", &fixture_model());
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Error { .. })
        ));
    }

    #[test]
    fn openai_request_posts_chat_completions_with_stream() {
        let options = StreamOptions {
            api_key: Some("sk-test".into()),
            ..StreamOptions::default()
        };
        let request =
            build_openai_chat_request(&fixture_model(), &empty_context(), Some(&options)).unwrap();
        assert_eq!(request.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(request.body["stream"], json!(true));
        assert_eq!(request.body["model"], json!("mock-1"));
        assert_eq!(
            request.body["messages"],
            json!([
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "hi"}
            ])
        );
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer sk-test"));
    }

    #[test]
    fn openai_api_key_prefers_options_over_missing_env() {
        let options = StreamOptions {
            api_key: Some("from-options".into()),
            ..StreamOptions::default()
        };
        assert_eq!(
            resolve_api_key(Some(&options), "OPENAI_API_KEY_UNLIKELY_TEST"),
            Some("from-options".into())
        );
        assert_eq!(resolve_api_key(None, "OPENAI_API_KEY_UNLIKELY_TEST"), None);
    }

    #[test]
    fn openai_missing_api_key_is_error_not_panic() {
        let options = StreamOptions::default();
        let err = build_openai_chat_request(&fixture_model(), &empty_context(), Some(&options));
        // May succeed if the environment actually has OPENAI_API_KEY; then skip.
        if err.is_err() {
            let events =
                openai_compatible_stream(&fixture_model(), &empty_context(), Some(options));
            assert!(matches!(
                events.last(),
                Some(AssistantMessageEvent::Error { .. })
            ));
        }
    }

    #[test]
    fn anthropic_content_block_delta_fixture_emits_text_deltas() {
        let events = parse_anthropic_messages_sse(ANTHROPIC_SSE, &anthropic_model());
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        assert_eq!(text_deltas(&events), vec!["Hello", " world"]);
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                ..
            })
        ));
    }

    #[test]
    fn anthropic_sse_parse_failure_is_error_event() {
        let events = parse_anthropic_messages_sse(
            "event: content_block_delta\ndata: not-json\n",
            &anthropic_model(),
        );
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Error { .. })
        ));
    }

    #[test]
    fn anthropic_request_is_streamed_messages() {
        let options = StreamOptions {
            api_key: Some("ant-key".into()),
            ..StreamOptions::default()
        };
        let request =
            build_anthropic_messages_request(&anthropic_model(), &empty_context(), Some(&options))
                .unwrap();
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(request.body["stream"], json!(true));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "x-api-key" && value == "ant-key"));
    }
}
