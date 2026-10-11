use super::super::cache::digest;
use super::super::context::{wrap_untrusted_data, ContextItem, ContextPacket};
use super::super::context_manifest::{ContextManifestEntry, ProvenanceKind};
use super::{
    CheckpointState, ContextEvent, ContextEventKind, ContextImage, ContextImageEntry,
    ContextObject, ContextObjectStore, ContextPageRef, ContextRoot, StateValue,
};
use davinci_ai::ChatMessage;
use serde_json::Value;
use std::collections::HashSet;

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

        let mut checkpoint_state = None;
        if let Some(checkpoint) = &request.root.checkpoint {
            let object = self.load_page(checkpoint)?;
            let content = object_content(&object)?;
            let entry = page_entry(checkpoint, "checkpoint", content, true);
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
            if let ContextObject::Checkpoint(state) = object {
                checkpoint_state = Some(state);
            }
        }

        // Content that changes on routine turns goes after the hot events, so
        // it never splits the prefix the provider cached last turn: the
        // delta, and optional broker items not marked stable. The whole delta
        // page is reserved here; what is rendered, after the hot events are
        // chosen, is only the part the model cannot see elsewhere.
        let mut deltas = Vec::new();
        for page in &request.root.deltas {
            let object = self.load_page(page)?;
            let content = object_content(&object)?;
            let mut entry = page_entry(page, "delta", content, true);
            entry.stable_for_cache = false;
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            deltas.push((entry, object));
        }
        let mut volatile_broker = Vec::new();

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

        // Admission order is not message order (WOR-72). After the required
        // entries come optional broker items at or above
        // PRIORITY_OVER_RECENT_TURNS (the living plan), then recent turns,
        // newest first: the hot set is already capped by the hot window, so
        // it is the recency reserve. Episode descriptors and the remaining
        // broker items share what is left. The messages keep their
        // cache-friendly order: pages, stable broker, hot, then volatile.
        let mut optional_remaining = optional_limit.saturating_sub(used_tokens);
        let mut broker = request
            .broker_packet
            .items
            .iter()
            .map(|item| {
                let entry = broker_entry(item);
                (!entry.mandatory).then_some((item.priority, entry))
            })
            .collect::<Vec<_>>();
        let mut admitted = vec![false; broker.len()];
        let mut admit_broker = |high: bool, remaining: &mut u64| {
            for (slot, candidate) in broker.iter().enumerate() {
                let Some((priority, entry)) = candidate else {
                    continue;
                };
                if admitted[slot]
                    || (*priority >= PRIORITY_OVER_RECENT_TURNS) != high
                    || entry.estimated_tokens > *remaining
                {
                    continue;
                }
                *remaining -= entry.estimated_tokens;
                admitted[slot] = true;
            }
        };
        admit_broker(true, &mut optional_remaining);
        let mut selected_from_tail = Vec::new();
        let mut hot_tokens = 0u64;
        while let Some(event) = selected_hot.pop() {
            let estimate = event_entry(event).estimated_tokens;
            if required(event) || estimate <= optional_remaining {
                if !required(event) {
                    optional_remaining -= estimate;
                }
                hot_tokens = hot_tokens.saturating_add(estimate);
                selected_from_tail.push(event);
            }
        }
        selected_from_tail.reverse();

        for page in &request.root.episodes {
            // Episodes are optional: a descriptor is never smaller than its empty skeleton,
            // so when even that cannot fit, skip without touching the object store.
            if episode_descriptor_floor_tokens(page) > optional_remaining {
                continue;
            }
            let object = self.load_page(page)?;
            let content = episode_descriptor(&page.id, &object)?;
            let entry = page_entry(page, "episode", content, false);
            if entry.estimated_tokens > optional_remaining {
                continue;
            }
            optional_remaining -= entry.estimated_tokens;
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }

        admit_broker(false, &mut optional_remaining);
        for (slot, candidate) in broker.iter_mut().enumerate() {
            let Some((_, entry)) = candidate.take().filter(|_| admitted[slot]) else {
                continue;
            };
            used_tokens = used_tokens.saturating_add(entry.estimated_tokens);
            if entry.stable_for_cache {
                messages.push(ChatMessage::text("custom", entry.content.clone()));
                entries.push(entry);
            } else {
                volatile_broker.push(entry);
            }
        }

        used_tokens = used_tokens.saturating_add(hot_tokens);
        let visible = selected_from_tail
            .iter()
            .map(|event| event.source_ref.as_str())
            .collect::<HashSet<_>>();
        for event in selected_from_tail {
            let mut entry = event_entry(event);
            entry.mandatory |= required(event);
            messages.push(event_message(event));
            entries.push(entry);
        }
        // A rendered view keeps the whole page's charge, so the image's
        // estimate admits the same image again; an empty view is omitted
        // along with its charge.
        for (mut entry, object) in deltas {
            if let ContextObject::Delta(delta) = &object {
                let Some(view) =
                    state_update_view(&delta.checkpoint_patch, checkpoint_state.as_ref(), &visible)
                else {
                    used_tokens = used_tokens.saturating_sub(entry.estimated_tokens);
                    continue;
                };
                let content = wrap_untrusted_data(&entry.source_ref, &view);
                entry.content_hash = digest(view.as_bytes());
                entry.content = content;
            }
            messages.push(ChatMessage::text("custom", entry.content.clone()));
            entries.push(entry);
        }
        for entry in volatile_broker {
            messages.push(ChatMessage::text("custom", entry.content.clone()));
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

/// What one hot event costs the compiler's budget.
pub(super) fn hot_entry_tokens(event: &ContextEvent) -> u64 {
    crate::provider_budget::message_token_ceiling(&event_message(event))
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
        estimated_tokens: hot_entry_tokens(event),
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
    } else if !event.images.is_empty() {
        ChatMessage {
            role: role.into(),
            content: event.user_content(),
            ..Default::default()
        }
    } else {
        ChatMessage::text(role, &event.visible_text)
    }
}

/// The current state the model cannot already see: values that are neither
/// in the checkpoint at the head of the image nor fully backed by recent
/// events it carries verbatim. Recent values come from those events, so this
/// view changes when events leave the window rather than on every turn, and
/// it stays out of the way of the cached prefix. None when nothing is left.
/// The full delta page stays retrievable by its id.
fn state_update_view(
    state: &CheckpointState,
    base: Option<&CheckpointState>,
    visible: &HashSet<&str>,
) -> Option<String> {
    let empty = CheckpointState::default();
    let base = base.unwrap_or(&empty);
    let shown_verbatim = |value: &StateValue<String>| {
        let mut sources = value
            .provenance
            .iter()
            .flat_map(|provenance| &provenance.source_refs)
            .peekable();
        sources.peek().is_some() && sources.all(|source| visible.contains(source.as_str()))
    };
    let unseen = |values: &[StateValue<String>], base: &[StateValue<String>]| {
        values
            .iter()
            .filter(|value| {
                !base.iter().any(|known| known.value == value.value) && !shown_verbatim(value)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut view = serde_json::Map::new();
    for (name, values, known) in [
        ("goals", &state.goals, &base.goals),
        ("constraints", &state.constraints, &base.constraints),
        ("completed", &state.completed, &base.completed),
        ("in_progress", &state.in_progress, &base.in_progress),
        ("blockers", &state.blockers, &base.blockers),
        ("decisions", &state.decisions, &base.decisions),
        (
            "modified_files",
            &state.modified_files,
            &base.modified_files,
        ),
        ("verification", &state.verification, &base.verification),
    ] {
        let values = unseen(values, known);
        if !values.is_empty() {
            view.insert(name.into(), serde_json::to_value(values).ok()?);
        }
    }
    if let Some(narrative) = state
        .narrative
        .as_ref()
        .filter(|narrative| base.narrative.as_ref() != Some(*narrative))
        .filter(|narrative| !shown_verbatim(narrative))
    {
        view.insert("narrative".into(), serde_json::to_value(narrative).ok()?);
    }
    let retired = state
        .retired
        .iter()
        .filter(|retired| !base.retired.contains(retired))
        .collect::<Vec<_>>();
    if !retired.is_empty() {
        view.insert("retired".into(), serde_json::to_value(retired).ok()?);
    }
    if view.is_empty() {
        return None;
    }
    let mut rendered = serde_json::Map::new();
    rendered.insert("type".into(), "state_update".into());
    rendered.extend(view);
    serde_json::to_string(&rendered).ok()
}

fn object_content(object: &ContextObject) -> Result<String, String> {
    serde_json::to_string(object).map_err(|error| format!("context page render failed: {error}"))
}

/// How the model recovers a folded episode. It names the tool and the exact
/// argument, so the placeholder is actionable without any prompt text.
fn episode_recovery_hint(page_id: &str) -> String {
    format!(
        "call retrieve_context with page={page_id} for the full episode, \
         or sourceRef=<one of source_refs> for an exact source"
    )
}

fn episode_descriptor(page_id: &str, object: &ContextObject) -> Result<String, String> {
    let ContextObject::Episode(episode) = object else {
        return object_content(object);
    };
    let recover = episode_recovery_hint(page_id);
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
            "recover": &recover,
        });
        let rendered = serde_json::to_string(&value)
            .map_err(|error| format!("context episode render failed: {error}"))?;
        // Keep the descriptor bounded; the compiler accounts for its envelope. The
        // complete Episode page remains available through retrieve_context, and
        // the recovery hint is never trimmed.
        if rendered.len() <= 512 + recover.len() {
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

/// Lower bound on the estimated tokens `episode_descriptor` can produce for `page`,
/// computed without loading it: the descriptor with every variable field empty.
fn episode_descriptor_floor_tokens(page: &ContextPageRef) -> u64 {
    let skeleton = serde_json::json!({
        "type": "episode",
        "title": "",
        "outcome": "",
        "source_refs": Vec::<String>::new(),
        "artifact_refs": Vec::<String>::new(),
        "recover": episode_recovery_hint(&page.id),
    })
    .to_string();
    let source_ref = format!("context_vm:{}", page.id);
    estimate_tokens(wrap_untrusted_data(&source_ref, &skeleton).len())
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

/// Optional broker items at or above this priority are admitted before
/// recent conversation. The living plan uses 500; repository files 100.
const PRIORITY_OVER_RECENT_TURNS: i32 = 500;

fn estimate_tokens(bytes: usize) -> u64 {
    // Includes custom-message envelope and provider framing.
    bytes as u64 + 128
}
