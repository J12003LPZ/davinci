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
    /// A capability or completion reminder the harness queued in the user
    /// role. The model must read it as an instruction in the user turn, but it
    /// is not a user requirement: no goal, not mandatory, no user budget.
    HarnessReminder,
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

impl EventImage {
    /// Digest of the image data; the same one `content_hash` folds in.
    pub fn digest(&self) -> String {
        digest(self.data.as_bytes())
    }

    /// The durable pointer to this image that outlives its bytes in the hot
    /// window: media type, a digest prefix and the way back to the source.
    pub fn reference(&self, source_ref: &str) -> String {
        let digest = self.digest();
        format!(
            "[image {} sha256:{}... - recover with retrieve_context {source_ref}]",
            self.mime_type,
            &digest[..digest.len().min(IMAGE_DIGEST_CHARS)]
        )
    }
}

/// Hex characters of an image digest shown in a reference.
const IMAGE_DIGEST_CHARS: usize = 16;

impl ContextEvent {
    /// One reference line per image the event carries (see
    /// [`EventImage::reference`]); empty for a text-only event.
    pub fn image_references(&self) -> Vec<String> {
        self.images
            .iter()
            .map(|image| image.reference(&self.source_ref))
            .collect()
    }

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
        "user" if !summary => match internal_user_message(message) {
            Some(InternalUserMessage::Reminder) => (
                ContextEventKind::HarnessReminder,
                ProvenanceKind::AgentInference,
            ),
            Some(internal) => (ContextEventKind::Custom, internal.provenance_kind()),
            None => (ContextEventKind::User, ProvenanceKind::UserDecision),
        },
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

/// `extra` field on the reminder the harness queues when a capability gate
/// blocks completion (`queue_capability_reminder` in turn.rs).
pub(crate) const CAPABILITY_REMINDER_FIELD: &str = "davinciCapabilityReminder";

/// A user-role message the harness wrote rather than a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InternalUserMessage {
    /// A finished background job (`customType: backgroundJob`).
    JobNotice,
    /// A capability or completion reminder.
    Reminder,
    /// Any other `customType`.
    Custom(String),
    /// A prompt the harness created and marked at creation
    /// (`INTERNAL_PROMPT_FIELD`): mailbox text, validator repair prompts.
    Marked(ProvenanceKind),
}

impl InternalUserMessage {
    pub(crate) fn provenance_kind(&self) -> ProvenanceKind {
        match self {
            Self::JobNotice => ProvenanceKind::ToolEvidence,
            Self::Reminder | Self::Custom(_) => ProvenanceKind::AgentInference,
            Self::Marked(kind) => *kind,
        }
    }
}

/// Whether a user-role message came from the harness. Job notices, capability
/// and completion reminders and any other `customType` message ride in the
/// user role so the model reads them, but they are never user requirements:
/// they must not become goals, be mandatory in the image, or get the user
/// excerpt budget. A message marked with `REAL_USER_ORIGIN_FIELD` is a
/// person's whatever else it carries; unmarked plain messages stay user
/// messages (`prompt_with` creates them).
pub(crate) fn internal_user_message(message: &ChatMessage) -> Option<InternalUserMessage> {
    if message.role != "user" || message.extra_bool(crate::REAL_USER_ORIGIN_FIELD) {
        return None;
    }
    if message.extra.contains_key(CAPABILITY_REMINDER_FIELD)
        || message
            .extra
            .contains_key(crate::completion::COMPLETION_REMINDER_FIELD)
    {
        return Some(InternalUserMessage::Reminder);
    }
    match message
        .extra
        .get(crate::INTERNAL_PROMPT_FIELD)
        .and_then(Value::as_str)
    {
        Some("tool_evidence") => {
            return Some(InternalUserMessage::Marked(ProvenanceKind::ToolEvidence))
        }
        Some(_) => return Some(InternalUserMessage::Marked(ProvenanceKind::AgentInference)),
        None => {}
    }
    match message.extra.get("customType").and_then(Value::as_str) {
        Some(crate::JOB_NOTICE_TYPE) => Some(InternalUserMessage::JobNotice),
        Some(other) => Some(InternalUserMessage::Custom(other.to_string())),
        None => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_with(extra: Value) -> ChatMessage {
        let mut message = ChatMessage::text("user", "Background job 3 finished.");
        message.extra = extra.as_object().cloned().unwrap_or_default();
        message
    }

    fn classify(message: &ChatMessage) -> (ContextEventKind, ProvenanceKind) {
        let event = event_from_message(message, "session:x".into(), 1).unwrap();
        (event.kind, event.provenance_kind)
    }

    #[test]
    fn image_reference_names_mime_digest_and_source() {
        let image = EventImage {
            mime_type: "image/png".into(),
            data: "AAAA".into(),
        };
        let line = image.reference("session:e1");
        assert!(line.starts_with("[image image/png sha256:"), "{line}");
        assert!(line.contains(&image.digest()[..16]), "{line}");
        assert!(
            line.ends_with("recover with retrieve_context session:e1]"),
            "{line}"
        );
    }

    #[test]
    fn plain_user_message_is_a_user_decision() {
        assert_eq!(
            classify(&ChatMessage::text("user", "fix the bug")),
            (ContextEventKind::User, ProvenanceKind::UserDecision)
        );
    }

    #[test]
    fn job_notice_is_tool_evidence_not_a_user_message() {
        let message = user_with(json!({"customType": crate::JOB_NOTICE_TYPE, "jobId": 3}));
        assert_eq!(
            classify(&message),
            (ContextEventKind::Custom, ProvenanceKind::ToolEvidence)
        );
    }

    #[test]
    fn harness_reminders_are_agent_inference() {
        for field in [
            CAPABILITY_REMINDER_FIELD,
            crate::completion::COMPLETION_REMINDER_FIELD,
        ] {
            let message = user_with(json!({ field: "verification_required" }));
            assert_eq!(
                classify(&message),
                (
                    ContextEventKind::HarnessReminder,
                    ProvenanceKind::AgentInference
                ),
                "{field}"
            );
        }
    }

    #[test]
    fn any_custom_type_on_a_user_role_message_is_not_user() {
        let message = user_with(json!({"customType": "extension-note"}));
        assert_eq!(
            classify(&message),
            (ContextEventKind::Custom, ProvenanceKind::AgentInference)
        );
    }

    #[test]
    fn real_user_origin_marker_overrides_internal_markers() {
        let message = user_with(json!({
            crate::REAL_USER_ORIGIN_FIELD: true,
            "customType": "extension-note",
        }));
        assert_eq!(
            classify(&message),
            (ContextEventKind::User, ProvenanceKind::UserDecision)
        );
    }
}
