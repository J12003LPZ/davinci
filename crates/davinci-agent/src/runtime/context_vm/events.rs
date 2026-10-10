use super::super::cache::digest;
use super::super::context_manifest::ProvenanceKind;
use super::types::ArtifactRef;
use davinci_ai::{content_text, ChatMessage, MessageContent};
use davinci_session::SessionEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextEventKind {
    User,
    Assistant,
    ToolResult,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEvent {
    pub source_ref: String,
    pub seq: u64,
    pub kind: ContextEventKind,
    pub provenance_kind: ProvenanceKind,
    pub content_hash: String,
    pub visible_text: String,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRef>,
    /// Images a user attached. They are model input like the text, so they
    /// reach the provider and count in `content_hash` (WOR-58).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<EventImage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventImage {
    pub mime_type: String,
    pub data: String,
}

impl ContextEvent {
    /// The provider content of a user event: its text, then its images.
    pub fn user_content(&self) -> Vec<MessageContent> {
        let text = (!self.visible_text.is_empty()).then(|| MessageContent::Text {
            text: self.visible_text.clone(),
        });
        text.into_iter()
            .chain(self.images.iter().map(|image| MessageContent::Image {
                data: image.data.clone(),
                mime_type: image.mime_type.clone(),
            }))
            .collect()
    }
}

pub fn events_from_session_branch(
    entries: &[SessionEntry],
    leaf_id: Option<&str>,
) -> Vec<ContextEvent> {
    davinci_session::branch_entries(entries, leaf_id)
        .into_iter()
        .filter_map(event_from_session_entry)
        .collect()
}

pub(super) fn event_from_session_entry(entry: &SessionEntry) -> Option<ContextEvent> {
    let message = match entry.entry_type.as_str() {
        "message" | "custom_message" => crate::entry_to_chat(entry)?,
        // Older extensions may store a complete chat message in a custom
        // entry. Metadata-only custom entries still have no provider content.
        "custom" => serde_json::from_value(entry.message.as_ref()?.clone()).ok()?,
        _ => return None,
    };
    event_from_message(&message, format!("session:{}", entry.id), entry.seq)
}

pub fn events_from_messages(messages: &[ChatMessage]) -> Vec<ContextEvent> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let visible = visible_text(message);
            let images = user_images(message);
            if visible.is_empty() && images.is_empty() {
                return None;
            }
            let content_hash = content_hash(&visible, &images);
            let source_ref = format!(
                "transient:{index}:{}",
                &content_hash[..content_hash.len().min(16)]
            );
            event_from_message(message, source_ref, index as u64 + 1)
        })
        .collect()
}

pub(super) fn event_from_message(
    message: &ChatMessage,
    source_ref: String,
    seq: u64,
) -> Option<ContextEvent> {
    if crate::is_legacy_verification_notice(message) {
        return None;
    }
    let visible_text = visible_text(message);
    let images = user_images(message);
    if visible_text.is_empty() && images.is_empty() {
        return None;
    }
    // A compaction or branch summary rides in a user message, but its words
    // are the summarizer's: never a user requirement.
    let summary = visible_text.starts_with(crate::COMPACTION_SUMMARY_PREFIX)
        || visible_text.starts_with(crate::BRANCH_SUMMARY_PREFIX);
    let (kind, provenance_kind) = match message.role.as_str() {
        "user" if !summary => (ContextEventKind::User, ProvenanceKind::UserDecision),
        "assistant" => (ContextEventKind::Assistant, ProvenanceKind::AgentInference),
        "toolResult" | "tool_result" => {
            (ContextEventKind::ToolResult, ProvenanceKind::ToolEvidence)
        }
        _ => (ContextEventKind::Custom, ProvenanceKind::AgentInference),
    };
    Some(ContextEvent {
        source_ref: source_ref.clone(),
        seq,
        kind,
        provenance_kind,
        content_hash: content_hash(&visible_text, &images),
        visible_text,
        artifact_refs: artifact_refs_from_message(message, &source_ref),
        images,
    })
}

fn user_images(message: &ChatMessage) -> Vec<EventImage> {
    if message.role != "user" {
        return Vec::new();
    }
    message
        .content
        .iter()
        .filter_map(|block| match block {
            MessageContent::Image { data, mime_type } => Some(EventImage {
                mime_type: mime_type.clone(),
                data: data.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Text-only events keep their existing hash, so stored roots stay valid.
fn content_hash(visible_text: &str, images: &[EventImage]) -> String {
    if images.is_empty() {
        return digest(visible_text.as_bytes());
    }
    let mut identity = visible_text.to_string();
    for image in images {
        identity.push_str(&format!(
            "\n[image {} {}]",
            image.mime_type,
            digest(image.data.as_bytes())
        ));
    }
    digest(identity.as_bytes())
}

fn artifact_refs_from_message(message: &ChatMessage, source_ref: &str) -> Vec<ArtifactRef> {
    let Some(governor) = message
        .extra
        .get("tokenGovernor")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let Some(output_id) = governor.get("outputId").and_then(Value::as_str) else {
        return Vec::new();
    };
    let uri = governor
        .get("reference")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("governor://"))
        .map(str::to_string)
        .unwrap_or_else(|| format!("governor://output/{output_id}"));
    vec![ArtifactRef {
        uri,
        content_hash: governor
            .get("contentHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        source_ref: source_ref.to_string(),
    }]
}

fn visible_text(message: &ChatMessage) -> String {
    let mut parts = Vec::new();
    let text = content_text(&message.content);
    if !text.is_empty() {
        parts.push(text);
    }
    for block in &message.content {
        if let MessageContent::ToolCall {
            name, arguments, ..
        } = block
        {
            let arguments = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".into());
            parts.push(format!("tool_call {name}({arguments})"));
        }
    }
    parts.join("\n")
}
