use super::super::cache::digest;
use super::super::context::{wrap_untrusted_data, ContextItem, ContextPacket};
use super::super::context_manifest::{ContextManifestEntry, ProvenanceKind};
use super::{
    ContextEvent, ContextEventKind, ContextImage, ContextImageEntry, ContextObject,
    ContextObjectStore, ContextPageRef, ContextRoot,
};
use davinci_ai::ChatMessage;
use serde_json::Value;

pub struct ContextCompileRequest<'a> {
    pub root: &'a ContextRoot,
    pub hot_events: &'a [ContextEvent],
    pub broker_packet: &'a ContextPacket,
    pub max_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ContextCompiler {
    store: ContextObjectStore,
}

impl ContextCompiler {
    pub fn new(store: ContextObjectStore) -> Self {
        Self { store }
    }

    pub fn compile(&self, request: ContextCompileRequest<'_>) -> Result<ContextImage, String> {
        let mut entries = Vec::new();
        let mut messages = Vec::new();
        let mut used_tokens = 0u64;

        if let Some(checkpoint) = &request.root.checkpoint {
            let object = self.load_page(checkpoint)?;
            let content = object_content(&object)?;
            let entry = page_entry(checkpoint, "checkpoint", content, true);
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        for page in &request.root.deltas {
            let object = self.load_page(page)?;
            let content = object_content(&object)?;
            let entry = page_entry(page, "delta", content, true);
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        let latest_user = request
            .hot_events
            .iter()
            .rev()
            .find(|event| event.kind == ContextEventKind::User);
        let newest = request.hot_events.last();
        let required = |event: &ContextEvent| {
            latest_user.is_some_and(|last| last.source_ref == event.source_ref)
                || newest.is_some_and(|last| last.source_ref == event.source_ref)
        };
        let mut selected_hot = request
            .hot_events
            .iter()
            .filter(|event| {
                required(event)
                    || request.root.hot_event_refs.is_empty()
                    || request
                        .root
                        .hot_event_refs
                        .iter()
                        .any(|source| source == &event.source_ref)
            })
            .collect::<Vec<_>>();
        let required_tokens = selected_hot
            .iter()
            .filter(|event| required(event))
            .map(|event| event_entry(event).estimated_tokens)
            .fold(0u64, u64::saturating_add);
        if used_tokens.saturating_add(required_tokens) > request.max_tokens {
            return Err(super::CONTEXT_BUDGET_EXCEEDED.into());
        }
        let optional_limit = request.max_tokens - required_tokens;

        for entry in request
            .broker_packet
            .items
            .iter()
            .map(broker_entry)
            .filter(|e| e.mandatory)
        {
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            if used_tokens > optional_limit {
                return Err(super::CONTEXT_BUDGET_EXCEEDED.into());
            }
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        for page in &request.root.episodes {
            let object = self.load_page(page)?;
            let content = episode_descriptor(&object)?;
            let entry = page_entry(page, "episode", content, false);
            if used_tokens.saturating_add(entry.estimated_tokens) > optional_limit {
                continue;
            }
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        for item in &request.broker_packet.items {
            let entry = broker_entry(item);
            if entry.mandatory {
                continue;
            }
            if used_tokens.saturating_add(entry.estimated_tokens) > optional_limit {
                continue;
            }
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        let mut selected_from_tail = Vec::new();
        let mut optional_remaining = optional_limit.saturating_sub(used_tokens);
        while let Some(event) = selected_hot.pop() {
            let estimate = event_entry(event).estimated_tokens;
            if required(event) || estimate <= optional_remaining {
                if !required(event) {
                    optional_remaining -= estimate;
                }
                used_tokens = used_tokens.saturating_add(estimate);
                selected_from_tail.push(event);
            }
        }
        selected_from_tail.reverse();
        for event in selected_from_tail {
            let mut entry = event_entry(event);
            entry.mandatory |= required(event);
            messages.push(event_message(event));
            entries.push(entry);
        }

        // Required state and the newest event must never be silently truncated.
        // Budget rejection is distinct from recoverable derived-cache errors.
        if used_tokens > request.max_tokens {
            return Err(super::CONTEXT_BUDGET_EXCEEDED.into());
        }

        let prefix_digest = digest(
            entries
                .iter()
                .filter(|entry| entry.stable_for_cache)
                .map(|entry| format!("{}:{}", entry.id, entry.content_hash))
                .collect::<Vec<_>>()
                .join("|")
                .as_bytes(),
        );
        Ok(ContextImage {
            root: request.root.clone(),
            entries,
            messages,
            estimated_tokens: used_tokens,
            prefix_digest,
        })
    }

    pub fn manifest_entries(&self, image: &ContextImage) -> Vec<ContextManifestEntry> {
        image
            .entries
            .iter()
            .map(|entry| {
                ContextManifestEntry::new(
                    entry.id.clone(),
                    entry.category.clone(),
                    entry.provenance_kind,
                    entry.source_ref.clone(),
                    entry.content_hash.clone(),
                    entry.estimated_tokens,
                    true,
                    Some("context_vm_selected".into()),
                    entry.mandatory,
                    "derived",
                    Some(entry.source_ref.clone()),
                )
            })
            .collect()
    }

    fn load_page(&self, page: &ContextPageRef) -> Result<ContextObject, String> {
        self.store.load(page).map_err(|error| error.to_string())
    }
}

fn page_entry(
    page: &ContextPageRef,
    category: &str,
    content: String,
    mandatory: bool,
) -> ContextImageEntry {
    let source_ref = format!("context_vm:{}", page.id);
    let wrapped = wrap_untrusted_data(&source_ref, &content);
    ContextImageEntry {
        id: page.id.clone(),
        category: category.into(),
        provenance_kind: ProvenanceKind::AgentInference,
        source_ref: source_ref.clone(),
        content_hash: page.content_hash.clone(),
        estimated_tokens: estimate_tokens(wrapped.len()),
        content: wrapped,
        mandatory,
        stable_for_cache: true,
    }
}

fn broker_entry(item: &ContextItem) -> ContextImageEntry {
    let source_ref = item.source.clone();
    ContextImageEntry {
        id: item.stable_id(),
        category: "broker_context".into(),
        provenance_kind: broker_provenance(item),
        source_ref: source_ref.clone(),
        content_hash: ContextManifestEntry::hash_content(&item.content),
        content: wrap_untrusted_data(&source_ref, &item.content),
        estimated_tokens: estimate_tokens(wrap_untrusted_data(&source_ref, &item.content).len()),
        mandatory: item.is_mandatory(),
        stable_for_cache: item.stable_for_cache,
    }
}

fn event_entry(event: &ContextEvent) -> ContextImageEntry {
    let category = match event.kind {
        ContextEventKind::User => "hot_user",
        ContextEventKind::Assistant => "hot_assistant",
        ContextEventKind::ToolResult => "hot_tool_result",
        ContextEventKind::Custom => "hot_custom",
    };
    ContextImageEntry {
        id: event.source_ref.clone(),
        category: category.into(),
        provenance_kind: event.provenance_kind,
        source_ref: event.source_ref.clone(),
        content_hash: event.content_hash.clone(),
        content: wrap_untrusted_data(&event.source_ref, &event.visible_text),
        estimated_tokens: crate::provider_budget::message_token_ceiling(&event_message(event)),
        mandatory: matches!(event.kind, ContextEventKind::User),
        stable_for_cache: false,
    }
}

fn event_message(event: &ContextEvent) -> ChatMessage {
    let role = match event.kind {
        ContextEventKind::User => "user",
        ContextEventKind::Assistant => "assistant",
        ContextEventKind::ToolResult => "custom",
        ContextEventKind::Custom => "custom",
    };
    if matches!(
        event.kind,
        ContextEventKind::ToolResult | ContextEventKind::Custom
    ) {
        ChatMessage::text(
            role,
            wrap_untrusted_data(&event.source_ref, &event.visible_text),
        )
    } else {
        ChatMessage::text(role, &event.visible_text)
    }
}

fn object_content(object: &ContextObject) -> Result<String, String> {
    serde_json::to_string(object).map_err(|error| format!("context page render failed: {error}"))
}

fn episode_descriptor(object: &ContextObject) -> Result<String, String> {
    let ContextObject::Episode(episode) = object else {
        return object_content(object);
    };
    let mut title = truncate_utf8(&episode.title, 96);
    let mut outcome = truncate_utf8(&episode.outcome, 192);
    let mut source_refs = episode
        .source_refs
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let mut artifact_refs = episode
        .artifact_refs
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>();

    loop {
        let value = serde_json::json!({
            "type": "episode",
            "title": &title,
            "outcome": &outcome,
            "source_refs": &source_refs,
            "artifact_refs": &artifact_refs,
        });
        let rendered = serde_json::to_string(&value)
            .map_err(|error| format!("context episode render failed: {error}"))?;
        // Keep the descriptor bounded; the compiler accounts for its envelope. The
        // complete Episode page remains available through retrieve_context.
        if rendered.len() <= 512 {
            return Ok(rendered);
        }
        if outcome.len() > 16 {
            outcome = truncate_utf8(&outcome, outcome.len().saturating_sub(16));
        } else if title.len() > 16 {
            title = truncate_utf8(&title, title.len().saturating_sub(16));
        } else if !artifact_refs.is_empty() {
            artifact_refs.pop();
        } else if !source_refs.is_empty() {
            source_refs.pop();
        } else {
            return Ok(rendered);
        }
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let suffix = "...";
    let limit = max_bytes.saturating_sub(suffix.len());
    let end = value
        .char_indices()
        .take_while(|(index, _)| *index <= limit)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    format!("{}{}", &value[..end], suffix)
}

fn broker_provenance(item: &ContextItem) -> ProvenanceKind {
    item.provenance
        .get("provenance_kind")
        .and_then(Value::as_str)
        .and_then(|value| match value {
            "user_decision" => Some(ProvenanceKind::UserDecision),
            "repository_fact" => Some(ProvenanceKind::RepositoryFact),
            "tool_evidence" => Some(ProvenanceKind::ToolEvidence),
            "mandatory_policy" => Some(ProvenanceKind::MandatoryPolicy),
            "agent_inference" => Some(ProvenanceKind::AgentInference),
            _ => None,
        })
        .unwrap_or(ProvenanceKind::RepositoryFact)
}

fn estimate_tokens(bytes: usize) -> u64 {
    // Includes custom-message envelope and provider framing.
    bytes as u64 + 128
}
