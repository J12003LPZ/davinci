use crate::types::*;

pub fn faux_text(text: impl Into<String>) -> ContentBlock {
    ContentBlock::Text(TextContent::new(text))
}

pub fn faux_thinking(thinking: impl Into<String>) -> ContentBlock {
    ContentBlock::Thinking(ThinkingContent::new(thinking))
}

pub fn faux_tool_call(
    name: impl Into<String>,
    args: serde_json::Value,
    id: Option<String>,
) -> ContentBlock {
    ContentBlock::ToolCall(ToolCall {
        content_type: "toolCall".to_string(),
        id: id.unwrap_or_else(|| "call_faux_1".to_string()),
        name: name.into(),
        arguments: args,
        thought_signature: None,
        namespace: None,
    })
}

pub fn faux_assistant_message(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    error_message: Option<String>,
) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".to_string(),
        content,
        api: "faux".to_string(),
        provider: "faux".to_string(),
        model: "faux-1".to_string(),
        response_model: None,
        response_id: Some("faux_msg_1".to_string()),
        usage: Usage::default(),
        stop_reason,
        deferred: None,
        error_message,
        raw_stop_reason: None,
        end_turn: Some(true),
        timestamp: now_ms(),
    }
}
