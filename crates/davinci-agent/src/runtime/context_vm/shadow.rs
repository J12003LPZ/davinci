use super::super::cache::digest;
use super::{ContextEvent, ContextEventKind, ContextImage};
use davinci_ai::{content_text, ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowComparison {
    pub legacy_estimated_tokens: u64,
    pub vm_estimated_tokens: u64,
    pub missing_user_refs: Vec<String>,
    pub missing_tool_refs: Vec<String>,
    pub prefix_changed: bool,
}

pub fn compare_shadow_views(
    legacy: &[ChatMessage],
    image: &ContextImage,
    events: &[ContextEvent],
) -> ShadowComparison {
    let image_refs: HashSet<&str> = image
        .entries
        .iter()
        .map(|entry| entry.source_ref.as_str())
        .collect();
    let missing_user_refs = events
        .iter()
        .filter(|event| {
            event.kind == ContextEventKind::User && !image_refs.contains(event.source_ref.as_str())
        })
        .map(|event| event.source_ref.clone())
        .collect();
    let missing_tool_refs = events
        .iter()
        .filter(|event| {
            event.kind == ContextEventKind::ToolResult
                && !image_refs.contains(event.source_ref.as_str())
        })
        .map(|event| event.source_ref.clone())
        .collect();
    ShadowComparison {
        legacy_estimated_tokens: legacy.iter().map(message_tokens).sum(),
        vm_estimated_tokens: image.estimated_tokens,
        missing_user_refs,
        missing_tool_refs,
        prefix_changed: legacy_prefix_changed(legacy, image),
    }
}

fn message_tokens(message: &ChatMessage) -> u64 {
    let mut text = content_text(&message.content);
    for block in &message.content {
        if let MessageContent::ToolCall {
            name, arguments, ..
        } = block
        {
            text.push_str(name);
            text.push_str(&serde_json::to_string(arguments).unwrap_or_default());
        }
    }
    text.len().div_ceil(4).max(1) as u64
}

fn legacy_prefix_changed(legacy: &[ChatMessage], image: &ContextImage) -> bool {
    let legacy_prefix = legacy
        .first()
        .map(|message| digest(content_text(&message.content).as_bytes()))
        .unwrap_or_default();
    let image_prefix = image
        .messages
        .first()
        .map(|message| digest(content_text(&message.content).as_bytes()))
        .unwrap_or_default();
    !legacy.is_empty() && !image.messages.is_empty() && legacy_prefix != image_prefix
}
