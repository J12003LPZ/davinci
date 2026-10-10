use super::super::cache::digest;
use super::super::context_manifest::ProvenanceKind;
use super::{
    CheckpointState, ContextEvent, ContextEventKind, ProvenanceRef, StateDelta, StateValue,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedStateValue {
    pub value: String,
    pub source_refs: Vec<String>,
    pub provenance_kind: ProvenanceKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointProposal {
    pub goals: Vec<ProposedStateValue>,
    pub constraints: Vec<ProposedStateValue>,
    pub completed: Vec<ProposedStateValue>,
    pub in_progress: Vec<ProposedStateValue>,
    pub blockers: Vec<ProposedStateValue>,
    pub decisions: Vec<ProposedStateValue>,
    pub modified_files: Vec<ProposedStateValue>,
    pub verification: Vec<ProposedStateValue>,
    pub narrative: Option<ProposedStateValue>,
    pub transitions: Vec<StateTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSlot {
    Goal,
    Constraint,
    Strategy,
    Blocker,
    Decision,
    Verification,
    ModifiedFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Resolve,
    Supersede,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition {
    pub slot: StateSlot,
    pub kind: TransitionKind,
    /// Exact previous value; a proposal cannot delete an unidentified fact.
    pub previous: String,
    pub evidence: ProposedStateValue,
    #[serde(default)]
    pub replacement: Option<ProposedStateValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredState {
    pub slot: StateSlot,
    pub kind: TransitionKind,
    pub previous: StateValue<String>,
    pub evidence: StateValue<String>,
}

pub struct ContextStateReducer;

impl ContextStateReducer {
    pub fn validate_proposal(
        parent: &CheckpointState,
        events: &[ContextEvent],
        proposal: CheckpointProposal,
    ) -> CheckpointState {
        let event_map: HashMap<&str, &ContextEvent> = events
            .iter()
            .map(|event| (event.source_ref.as_str(), event))
            .collect();
        let parent_map = parent_provenance(parent);
        let validate = |values: Vec<ProposedStateValue>| {
            values
                .into_iter()
                .filter_map(|value| validate_value(value, &event_map, &parent_map))
                .collect::<Vec<_>>()
        };

        let proposed = CheckpointState {
            through_seq: parent.through_seq,
            goals: validate(
                proposal
                    .goals
                    .into_iter()
                    .filter(is_user_authority)
                    .collect(),
            ),
            constraints: validate(
                proposal
                    .constraints
                    .into_iter()
                    .filter(is_user_authority)
                    .collect(),
            ),
            completed: validate(proposal.completed),
            in_progress: validate(proposal.in_progress),
            blockers: validate(proposal.blockers),
            decisions: validate(proposal.decisions),
            modified_files: validate(proposal.modified_files),
            verification: validate(proposal.verification),
            narrative: proposal
                .narrative
                .filter(|value| value.provenance_kind == ProvenanceKind::AgentInference)
                .and_then(|value| validate_value(value, &event_map, &parent_map)),
            retired: Vec::new(),
        };
        // Partial proposals preserve proven state. Retirement is permitted
        // only through explicit lifecycle transitions with newer evidence.
        let mut state = CheckpointState {
            through_seq: parent.through_seq,
            goals: merge_values(&parent.goals, proposed.goals),
            constraints: merge_values(&parent.constraints, proposed.constraints),
            completed: merge_values(&parent.completed, proposed.completed),
            in_progress: merge_values(&parent.in_progress, proposed.in_progress),
            blockers: merge_values(&parent.blockers, proposed.blockers),
            decisions: merge_values(&parent.decisions, proposed.decisions),
            modified_files: merge_values(&parent.modified_files, proposed.modified_files),
            verification: merge_values(&parent.verification, proposed.verification),
            narrative: proposed.narrative.or_else(|| parent.narrative.clone()),
            retired: parent.retired.clone(),
        };
        for transition in proposal.transitions {
            apply_transition(&mut state, transition, &event_map, &parent_map);
        }
        if state.retired.len() > 32 {
            state.retired.drain(..state.retired.len() - 32);
        }
        let accepted_refs = state_provenance_refs(&state);
        state.through_seq = events
            .iter()
            .filter(|event| accepted_refs.contains(&event.source_ref))
            .map(|event| event.seq)
            .max()
            .unwrap_or(parent.through_seq)
            .max(parent.through_seq);
        state
    }

    pub fn deterministic_delta(parent: &CheckpointState, events: &[ContextEvent]) -> StateDelta {
        let mut state = parent.clone();
        for event in events {
            match event.kind {
                // The exact user text is authoritative. Keeping it as a goal preserves the
                // prompt reference without asking the deterministic path to infer intent.
                // An image-only message has no words to keep as a goal.
                ContextEventKind::User if event.visible_text.trim().is_empty() => {}
                ContextEventKind::User => push_unique(
                    &mut state.goals,
                    StateValue {
                        value: user_excerpt(&event.visible_text),
                        provenance: vec![provenance(event, ProvenanceKind::UserDecision)],
                    },
                ),
                ContextEventKind::ToolResult
                    if contains_verification_marker(&event.visible_text) =>
                {
                    // A repeated result is a fresh confirmation: it moves to
                    // the recent end, so `keep_recent` keeps what was
                    // confirmed last rather than what was seen first.
                    push_refreshed(
                        &mut state.verification,
                        StateValue {
                            value: evidence_excerpt(&event.visible_text),
                            provenance: vec![provenance(event, ProvenanceKind::ToolEvidence)],
                        },
                    );
                }
                _ => {}
            }
        }
        // Without a summarizer nothing merges these excerpts, so a long run
        // would grow the checkpoint by one goal per prompt forever. Keep the
        // original request and the most recent ones; every dropped value stays
        // retrievable by its source_ref.
        keep_first_directives_and_recent(&mut state.goals, FALLBACK_GOAL_LIMIT);
        keep_recent(&mut state.verification, FALLBACK_VERIFICATION_LIMIT);
        state.through_seq = state
            .through_seq
            .max(events.iter().map(|event| event.seq).max().unwrap_or(0));
        StateDelta {
            from_seq: parent.through_seq,
            through_seq: state.through_seq,
            checkpoint_patch: state,
        }
    }
}

/// Source refs shown per ledger value; the full list stays in the state.
const LEDGER_REFS_PER_VALUE: usize = 2;

/// The task ledger as the model reads it after a compaction: every slot of
/// the validated state, each value with the sources it was traced to.
/// Values are already bounded and deduplicated by the reducer.
pub fn render_ledger(state: &CheckpointState) -> String {
    fn refs(value: &StateValue<String>) -> String {
        let refs = value
            .provenance
            .iter()
            .flat_map(|provenance| &provenance.source_refs)
            .take(LEDGER_REFS_PER_VALUE)
            .map(String::as_str)
            .collect::<Vec<_>>();
        if refs.is_empty() {
            String::new()
        } else {
            format!(" [{}]", refs.join(", "))
        }
    }
    let mut out = String::from(
        "Task ledger kept by the Context VM. Each value is quoted from or traced to \
         the sources in brackets; retrieve_context with sourceRef=<ref> returns a \
         source exactly. Only Verification is backed by tool output; anything else \
         claimed done is unverified.\n",
    );
    for (title, values) in [
        ("Goals (user)", &state.goals),
        ("Constraints (user or policy)", &state.constraints),
        ("Decisions", &state.decisions),
        ("In progress", &state.in_progress),
        ("Completed", &state.completed),
        ("Blockers", &state.blockers),
        ("Modified files", &state.modified_files),
        (
            "Verification (tool output, newest last)",
            &state.verification,
        ),
    ] {
        if values.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {title}\n"));
        for value in values {
            out.push_str(&format!("- {}{}\n", value.value, refs(value)));
        }
    }
    if !state.retired.is_empty() {
        out.push_str("\n## Superseded or resolved (do not act on the old value)\n");
        for retired in &state.retired {
            let kind = match retired.kind {
                TransitionKind::Resolve => "resolved",
                TransitionKind::Supersede => "superseded",
                TransitionKind::Reject => "rejected",
            };
            out.push_str(&format!(
                "- {kind}: {} -> {}{}\n",
                retired.previous.value,
                retired.evidence.value,
                refs(&retired.evidence)
            ));
        }
    }
    if let Some(narrative) = &state.narrative {
        out.push_str(&format!(
            "\n## Narrative (agent inference)\n{}{}\n",
            narrative.value,
            refs(narrative)
        ));
    }
    out
}

pub fn parse_checkpoint_proposal(text: &str) -> Result<CheckpointProposal, String> {
    serde_json::from_str(text).map_err(|error| format!("invalid context fold JSON: {error}"))
}

fn is_user_authority(value: &ProposedStateValue) -> bool {
    matches!(
        value.provenance_kind,
        ProvenanceKind::UserDecision | ProvenanceKind::MandatoryPolicy
    )
}

fn validate_value(
    value: ProposedStateValue,
    events: &HashMap<&str, &ContextEvent>,
    parent: &ParentSources,
) -> Option<StateValue<String>> {
    validate_cited_value(value, events, parent, true)
}

/// `grounded` is false only for the evidence of a supersede, whose grounded
/// replacement already carries the user's words. That evidence records why a
/// value retired rather than quoting the user, so it is kept out of later
/// grounding text.
fn validate_cited_value(
    value: ProposedStateValue,
    events: &HashMap<&str, &ContextEvent>,
    parent: &ParentSources,
    grounded: bool,
) -> Option<StateValue<String>> {
    if value.value.trim().is_empty()
        || value.value.len() > 1024
        || value.source_refs.is_empty()
        || value.source_refs.len() > 16
    {
        return None;
    }
    // A matching source type is not enough for authority: the summarizer
    // could pin any claim to a real user message. User and policy values
    // must be grounded in the text they cite (WOR-62).
    if grounded && is_user_authority(&value) && !grounded_in_sources(&value, events, parent) {
        return None;
    }
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for source_ref in value.source_refs {
        let valid = if let Some(event) = events.get(source_ref.as_str()) {
            provenance_matches(value.provenance_kind, event.kind, event.provenance_kind)
        } else {
            parent
                .get(&source_ref)
                .is_some_and(|(kind, _)| provenance_matches_parent(value.provenance_kind, *kind))
        };
        if !valid {
            return None;
        }
        if seen.insert(source_ref.clone()) {
            sources.push(source_ref);
        }
    }
    if sources.is_empty() {
        return None;
    }
    Some(StateValue {
        provenance: vec![ProvenanceRef {
            kind: value.provenance_kind,
            content_hash: digest(value.value.as_bytes()),
            source_refs: sources,
        }],
        value: value.value,
    })
}

fn provenance_matches(
    requested: ProvenanceKind,
    event_kind: ContextEventKind,
    event_provenance: ProvenanceKind,
) -> bool {
    match requested {
        ProvenanceKind::UserDecision => event_kind == ContextEventKind::User,
        ProvenanceKind::ToolEvidence | ProvenanceKind::RepositoryFact => {
            event_kind == ContextEventKind::ToolResult
        }
        ProvenanceKind::AgentInference => {
            matches!(
                event_kind,
                ContextEventKind::Assistant | ContextEventKind::Custom
            )
        }
        ProvenanceKind::MandatoryPolicy => event_provenance == ProvenanceKind::MandatoryPolicy,
    }
}

fn provenance_matches_parent(requested: ProvenanceKind, existing: ProvenanceKind) -> bool {
    match requested {
        ProvenanceKind::RepositoryFact => {
            matches!(
                existing,
                ProvenanceKind::RepositoryFact | ProvenanceKind::ToolEvidence
            )
        }
        _ => requested == existing,
    }
}

/// Source refs already accepted into the parent checkpoint: their provenance
/// kind and the accepted values citing them. Folded events are no longer in
/// the event list, so a new value citing one is grounded in those values.
type ParentSources = HashMap<String, (ProvenanceKind, String)>;

fn parent_provenance(parent: &CheckpointState) -> ParentSources {
    let mut result: ParentSources = HashMap::new();
    // Retirement evidence may be an ungrounded description, so it lends its
    // refs' provenance kind but never its text as grounding for a new value.
    let evidence: HashSet<*const StateValue<String>> = parent
        .retired
        .iter()
        .map(|retired| &retired.evidence as *const _)
        .collect();
    for value in all_values(parent) {
        let grounding = !evidence.contains(&(value as *const _));
        for provenance in &value.provenance {
            for source_ref in &provenance.source_refs {
                let (kind, text) = result
                    .entry(source_ref.clone())
                    .or_insert_with(|| (provenance.kind, String::new()));
                *kind = provenance.kind;
                if grounding {
                    text.push('\n');
                    text.push_str(&value.value);
                }
            }
        }
    }
    result
}

/// Words that carry no claim of their own; they may be dropped from a quote.
/// Negations are deliberately absent: "do not delete" never equals "delete".
const UNGROUNDED_WORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "at",
    "by",
    "for",
    "from",
    "with",
    "as",
    "is",
    "are",
    "be",
    "was",
    "were",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "user",
    "users",
    "wants",
    "want",
    "asked",
    "asks",
    "requires",
    "require",
    "requested",
    "should",
    "must",
    "please",
];

/// Whitespace-separated words, lowercased, with surrounding punctuation
/// trimmed. Inner punctuation stays, so `src/main.rs` and `.env` remain
/// distinct from `main` and `env`.
fn content_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|word| {
            let word = word.trim_end_matches(|c: char| !c.is_alphanumeric());
            let start = word.find(|c: char| c.is_alphanumeric())?;
            // Keep the dot of a dotfile name: ".env" is not "env".
            let start = if start > 0 && word[..start].ends_with('.') {
                start - 1
            } else {
                start
            };
            Some(word[start..].to_lowercase())
        })
        .filter(|word| !UNGROUNDED_WORDS.contains(&word.as_str()))
        .collect()
}

/// Clauses end at `;`, `!`, `?`, a line break, or a period that ends a
/// sentence (followed by whitespace or the end), never at the dot in a name.
/// Each clause is returned with whether it is a question.
fn clauses(text: &str) -> Vec<(&str, bool)> {
    let mut clauses = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let ends = match c {
            ';' | '!' | '?' | '\n' => true,
            '.' => chars.peek().is_none_or(|(_, next)| next.is_whitespace()),
            _ => false,
        };
        if ends {
            clauses.push((&text[start..index], c == '?'));
            start = index + c.len_utf8();
        }
    }
    clauses.push((&text[start..], false));
    clauses
}

/// True when `value` reproduces whole clauses of one cited source: its content
/// words equal those of one or more consecutive clauses, none of them a
/// question. The summarizer may drop function words and punctuation, and
/// choose which sentences matter, but cannot cut a clause. Cutting is what
/// forges intent: "Should we drop the table?", "Deleting the tests is not
/// acceptable", "I was going to force push but changed my mind" and "my
/// coworker said to disable auth" all contain a command that is not the
/// user's. Word-level checks cannot tell those apart; clause boundaries can.
/// A rejected value costs only the summarizer's copy: the deterministic
/// checkpoint keeps the user's own text.
fn grounded_in_sources(
    value: &ProposedStateValue,
    events: &HashMap<&str, &ContextEvent>,
    parent: &ParentSources,
) -> bool {
    let quote = content_words(&value.value);
    if quote.is_empty() {
        return false;
    }
    value.source_refs.iter().any(|source_ref| {
        let text = match events.get(source_ref.as_str()) {
            Some(event) => event.visible_text.as_str(),
            None => match parent.get(source_ref) {
                Some((_, text)) => text.as_str(),
                None => return false,
            },
        };
        let clauses = clauses(text)
            .into_iter()
            .map(|(clause, question)| (content_words(clause), question))
            .filter(|(words, _)| !words.is_empty())
            .collect::<Vec<_>>();
        (0..clauses.len()).any(|first| {
            let mut joined = Vec::new();
            for (words, question) in &clauses[first..] {
                if *question {
                    return false;
                }
                joined.extend(words.iter().cloned());
                if joined.len() >= quote.len() {
                    return joined == quote;
                }
            }
            false
        })
    })
}

fn state_provenance_refs(state: &CheckpointState) -> HashSet<String> {
    all_values(state)
        .flat_map(|value| {
            value
                .provenance
                .iter()
                .flat_map(|provenance| provenance.source_refs.iter().cloned())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn all_values(state: &CheckpointState) -> impl Iterator<Item = &StateValue<String>> {
    state
        .goals
        .iter()
        .chain(&state.constraints)
        .chain(&state.completed)
        .chain(&state.in_progress)
        .chain(&state.blockers)
        .chain(&state.decisions)
        .chain(&state.modified_files)
        .chain(&state.verification)
        .chain(state.narrative.iter())
        .chain(
            state
                .retired
                .iter()
                .flat_map(|v| [&v.previous, &v.evidence]),
        )
}

fn slot_values(state: &mut CheckpointState, slot: StateSlot) -> &mut Vec<StateValue<String>> {
    match slot {
        StateSlot::Goal => &mut state.goals,
        StateSlot::Constraint => &mut state.constraints,
        StateSlot::Strategy => &mut state.in_progress,
        StateSlot::Blocker => &mut state.blockers,
        StateSlot::Decision => &mut state.decisions,
        StateSlot::Verification => &mut state.verification,
        StateSlot::ModifiedFile => &mut state.modified_files,
    }
}

fn apply_transition(
    state: &mut CheckpointState,
    transition: StateTransition,
    events: &HashMap<&str, &ContextEvent>,
    parent: &ParentSources,
) {
    let Some(previous) = slot_values(state, transition.slot)
        .iter()
        .find(|v| v.value == transition.previous)
        .cloned()
    else {
        return;
    };
    let previous_seq = previous
        .provenance
        .iter()
        .flat_map(|p| &p.source_refs)
        .filter_map(|r| events.get(r.as_str()))
        .map(|e| e.seq)
        .max()
        .unwrap_or(state.through_seq);
    // Lifecycle changes require newer observed evidence, never an assistant's
    // unsupported claim. Constraints can only be changed by the user/policy.
    let policy = previous
        .provenance
        .iter()
        .any(|p| p.kind == ProvenanceKind::MandatoryPolicy);
    let is_new_authority = |r: &String| {
        events.get(r.as_str()).is_some_and(|e| {
            e.seq > previous_seq
                && if policy {
                    e.provenance_kind == ProvenanceKind::MandatoryPolicy
                } else {
                    match transition.slot {
                        StateSlot::Constraint => {
                            e.kind == ContextEventKind::User
                                || e.provenance_kind == ProvenanceKind::MandatoryPolicy
                        }
                        StateSlot::Goal if transition.kind != TransitionKind::Resolve => {
                            e.kind == ContextEventKind::User
                        }
                        StateSlot::Strategy => true,
                        _ => matches!(
                            e.kind,
                            ContextEventKind::User | ContextEventKind::ToolResult
                        ),
                    }
                }
        })
    };
    let new_authority = transition.evidence.source_refs.iter().all(is_new_authority);
    if !new_authority {
        return;
    }
    // A reject or resolve stands on its evidence alone (and a resolved goal
    // stores it as completed work), so that evidence must quote the user. A
    // supersede is justified by its replacement, which is grounded below, so
    // its evidence may describe the change ("latest user correction").
    let evidence_grounded = transition.replacement.is_none();
    let Some(evidence) =
        validate_cited_value(transition.evidence, events, parent, evidence_grounded)
    else {
        return;
    };
    let replacement = match transition.replacement {
        Some(value) => match value
            .source_refs
            .iter()
            .all(is_new_authority)
            .then(|| validate_value(value, events, parent))
            .flatten()
        {
            Some(value) => Some(value),
            None => return,
        },
        None => None,
    };
    if transition.kind == TransitionKind::Supersede && replacement.is_none() {
        return;
    }
    let values = slot_values(state, transition.slot);
    values.retain(|v| v.value != transition.previous);
    if let Some(value) = replacement {
        push_unique(values, value);
    }
    if transition.slot == StateSlot::Goal && transition.kind == TransitionKind::Resolve {
        push_unique(&mut state.completed, evidence.clone());
    }
    state.retired.push(RetiredState {
        slot: transition.slot,
        kind: transition.kind,
        previous,
        evidence,
    });
}

/// Deterministic values kept whole up to this size.
const WHOLE_VALUE_BYTES: usize = 512;
/// Bound on a deterministic excerpt, marker included. Proposals are held to
/// the same bound.
const EXCERPT_BYTES: usize = 1024;
const EXCERPT_MARKER: &str = " [excerpt; retrieve source]";
const EXCERPT_GAP: &str = " [...] ";

/// The longest prefix of `text` within `max` bytes, on a char boundary.
fn head(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The longest suffix of `text` within `max` bytes, on a char boundary.
fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Words that mark a clause as an instruction rather than narrative: what
/// must, must not, or should instead happen. Matched on whole words.
const DIRECTIVE_WORDS: &[&str] = &[
    "never",
    "must",
    "always",
    "don't",
    "dont",
    "do not",
    "only",
    "required",
    "requirement",
    "avoid",
    "instead",
    "correction",
    "ensure",
    "make sure",
    "not allowed",
    "forbidden",
    "cannot",
    "can't",
    "shouldn't",
    "should not",
    "keep",
    "prefer",
];

fn is_directive(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    let words = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    DIRECTIVE_WORDS.iter().any(|directive| {
        let parts = directive.split(' ').collect::<Vec<_>>();
        words
            .windows(parts.len())
            .any(|window| window == parts.as_slice())
    })
}

/// A user message for the deterministic goal list. Short messages stay
/// whole. A long one keeps its opening, then every clause that reads as an
/// instruction, in order, as far as the bound allows: a constraint buried in
/// a pasted design note survives, where a head-only cut dropped it. The
/// source stays retrievable either way.
fn user_excerpt(text: &str) -> String {
    if text.len() <= WHOLE_VALUE_BYTES {
        return text.into();
    }
    let budget = EXCERPT_BYTES - EXCERPT_MARKER.len();
    let mut out = head(text, 256).to_string();
    let rest = &text[out.len()..];
    // A clause the head cut in two is not quoted from its middle.
    let cut = !out.ends_with(['.', ';', '!', '?', '\n']);
    for (clause, _) in clauses(rest).into_iter().skip(usize::from(cut)) {
        // Keep the clause's own terminator: the user's words stay exact.
        let end = clause.as_ptr() as usize - rest.as_ptr() as usize + clause.len();
        let ended = rest[end..]
            .chars()
            .next()
            .filter(|c| matches!(c, '.' | ';' | '!' | '?'))
            .map_or(end, |c| end + c.len_utf8());
        let clause = rest[end - clause.len()..ended].trim();
        if clause.is_empty() || !is_directive(clause) {
            continue;
        }
        if out.len() + EXCERPT_GAP.len() + clause.len() > budget {
            continue;
        }
        out.push_str(EXCERPT_GAP);
        out.push_str(clause);
    }
    out.push_str(EXCERPT_MARKER);
    out
}

/// Tool evidence for the deterministic verification list: its head and its
/// tail, where test runners and builds print their verdict.
fn evidence_excerpt(text: &str) -> String {
    if text.len() <= WHOLE_VALUE_BYTES {
        return text.into();
    }
    let side = (EXCERPT_BYTES - EXCERPT_MARKER.len() - EXCERPT_GAP.len()) / 2;
    format!(
        "{}{EXCERPT_GAP}{}{EXCERPT_MARKER}",
        head(text, side),
        tail(text, side)
    )
}

fn provenance(event: &ContextEvent, kind: ProvenanceKind) -> ProvenanceRef {
    ProvenanceRef {
        kind,
        source_refs: vec![event.source_ref.clone()],
        content_hash: event.content_hash.clone(),
    }
}

const FALLBACK_GOAL_LIMIT: usize = 16;
const FALLBACK_VERIFICATION_LIMIT: usize = 8;

/// Bound the deterministic goal list. The original request and the newest
/// half stay. Older goals in between go oldest first, but a goal that states
/// an instruction ("never touch migrations") outlasts ones that do not
/// ("continue with step 7"); when only instructions remain, the oldest goes.
/// Every dropped value stays retrievable by its source_ref.
fn keep_first_directives_and_recent(values: &mut Vec<StateValue<String>>, limit: usize) {
    if limit < 2 {
        return;
    }
    let recent = limit / 2;
    while values.len() > limit {
        let middle = 1..values.len() - recent;
        let drop = middle
            .clone()
            .find(|&index| !is_directive(&values[index].value))
            .unwrap_or(middle.start);
        values.remove(drop);
    }
}

fn keep_recent(values: &mut Vec<StateValue<String>>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
}

/// Provenance kept per value: the first observation and the most recent
/// ones, so repeated confirmations cannot grow a value without bound.
const MAX_PROVENANCE_PER_VALUE: usize = 8;

/// Add `value`, or, when the same text is already there, add the new
/// observation's provenance to it. Dropping that provenance lost the newer
/// evidence, and transitions that require it.
fn push_unique(values: &mut Vec<StateValue<String>>, value: StateValue<String>) {
    match values
        .iter_mut()
        .find(|existing| existing.value == value.value)
    {
        Some(existing) => merge_provenance(&mut existing.provenance, value.provenance),
        None => values.push(value),
    }
}

/// Like `push_unique`, but a repeated value moves to the end.
fn push_refreshed(values: &mut Vec<StateValue<String>>, mut value: StateValue<String>) {
    if let Some(index) = values
        .iter()
        .position(|existing| existing.value == value.value)
    {
        let mut existing = values.remove(index);
        merge_provenance(
            &mut existing.provenance,
            std::mem::take(&mut value.provenance),
        );
        value = existing;
    }
    values.push(value);
}

fn merge_provenance(provenance: &mut Vec<ProvenanceRef>, newer: Vec<ProvenanceRef>) {
    for reference in newer {
        if !provenance.contains(&reference) {
            provenance.push(reference);
        }
    }
    if provenance.len() > MAX_PROVENANCE_PER_VALUE {
        provenance.drain(1..provenance.len() - (MAX_PROVENANCE_PER_VALUE - 1));
    }
}

fn merge_values(
    parent: &[StateValue<String>],
    proposed: Vec<StateValue<String>>,
) -> Vec<StateValue<String>> {
    let mut values = parent.to_vec();
    for value in proposed {
        push_unique(&mut values, value);
    }
    values
}

fn contains_verification_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "test",
        "verified",
        "verification",
        "passed",
        "failed",
        "failure",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use crate::runtime::context_vm::events_from_messages;
    use davinci_ai::ChatMessage;

    fn sources(value: &StateValue<String>) -> Vec<String> {
        value
            .provenance
            .iter()
            .flat_map(|reference| reference.source_refs.clone())
            .collect()
    }

    #[test]
    fn a_repeated_observation_keeps_its_newer_provenance() {
        let mut events = events_from_messages(&[
            ChatMessage::text("user", "ship it"),
            ChatMessage::text("user", "ship it"),
        ]);
        for (index, event) in events.iter_mut().enumerate() {
            event.source_ref = format!("session:goal-{index}");
        }
        let mut verification = events[0].clone();
        verification.kind = ContextEventKind::ToolResult;
        verification.visible_text = "all tests passed".into();
        let mut later = verification.clone();
        verification.source_ref = "session:run-1".into();
        later.source_ref = "session:run-2".into();
        later.seq += 10;
        later.content_hash = "later".into();
        events.push(verification);
        events.push(later);
        let state = ContextStateReducer::deterministic_delta(&CheckpointState::default(), &events)
            .checkpoint_patch;
        assert_eq!(state.goals.len(), 1);
        assert_eq!(
            sources(&state.goals[0]),
            ["session:goal-0", "session:goal-1"]
        );
        assert_eq!(state.verification.len(), 1);
        assert_eq!(
            sources(&state.verification[0]),
            ["session:run-1", "session:run-2"]
        );
    }

    #[test]
    fn provenance_per_value_is_bounded_keeping_first_and_latest() {
        let mut values = Vec::new();
        for n in 0..50 {
            push_unique(
                &mut values,
                StateValue {
                    value: "same".to_string(),
                    provenance: vec![ProvenanceRef {
                        kind: ProvenanceKind::UserDecision,
                        source_refs: vec![format!("session:{n}")],
                        content_hash: format!("{n}"),
                    }],
                },
            );
        }
        let refs = sources(&values[0]);
        assert_eq!(refs.len(), MAX_PROVENANCE_PER_VALUE);
        assert_eq!(refs.first().map(String::as_str), Some("session:0"));
        assert_eq!(refs.last().map(String::as_str), Some("session:49"));
    }

    #[test]
    fn a_reconfirmed_verification_survives_the_recent_window() {
        let value = |text: &str, source: &str| StateValue {
            value: text.to_string(),
            provenance: vec![ProvenanceRef {
                kind: ProvenanceKind::ToolEvidence,
                source_refs: vec![source.to_string()],
                content_hash: source.to_string(),
            }],
        };
        let mut values = Vec::new();
        push_refreshed(&mut values, value("old pass", "a"));
        for n in 0..FALLBACK_VERIFICATION_LIMIT {
            push_refreshed(&mut values, value(&format!("other {n}"), &format!("o{n}")));
        }
        push_refreshed(&mut values, value("old pass", "b"));
        keep_recent(&mut values, FALLBACK_VERIFICATION_LIMIT);
        assert_eq!(values.last().unwrap().value, "old pass");
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    use crate::runtime::context_vm::events_from_messages;
    use davinci_ai::ChatMessage;

    fn goals(messages: &[ChatMessage]) -> Vec<String> {
        ContextStateReducer::deterministic_delta(
            &CheckpointState::default(),
            &events_from_messages(messages),
        )
        .checkpoint_patch
        .goals
        .into_iter()
        .map(|goal| goal.value)
        .collect()
    }

    #[test]
    fn a_requirement_buried_in_a_long_paste_survives_word_for_word() {
        let paste = format!(
            "Design note:\n{}\nAll public functions must keep their current signatures.\n{}",
            "The scheduler stores jobs in a queue. ".repeat(60),
            "Workers report progress. ".repeat(60)
        );
        let goal = goals(&[ChatMessage::text("user", paste)]).remove(0);
        assert!(
            goal.contains("All public functions must keep their current signatures."),
            "{goal}"
        );
        assert!(goal.starts_with("Design note:"), "{goal}");
        assert!(goal.ends_with("[excerpt; retrieve source]"));
        assert!(goal.len() <= EXCERPT_BYTES);
    }

    #[test]
    fn a_long_test_log_keeps_its_verdict() {
        let log = format!(
            "running 40 tests\n{}test result: FAILED. 39 passed; 1 failed",
            "test parser::case ... ok\n".repeat(200)
        );
        let mut event = events_from_messages(&[ChatMessage::text("user", "x")]).remove(0);
        event.kind = ContextEventKind::ToolResult;
        event.visible_text = log;
        let state = ContextStateReducer::deterministic_delta(&CheckpointState::default(), &[event])
            .checkpoint_patch;
        let verdict = &state.verification[0].value;
        assert!(verdict.starts_with("running 40 tests"), "{verdict}");
        assert!(verdict.contains("test result: FAILED. 39 passed; 1 failed"));
        assert!(verdict.len() <= EXCERPT_BYTES);
    }

    #[test]
    fn an_old_instruction_outlasts_newer_routine_goals() {
        let mut messages = vec![
            ChatMessage::text("user", "Build the scheduler."),
            ChatMessage::text("user", "Never modify files under migrations/."),
        ];
        for step in 0..30 {
            messages.push(ChatMessage::text(
                "user",
                format!("Continue with step {step}."),
            ));
        }
        let goals = goals(&messages);
        assert_eq!(goals.len(), FALLBACK_GOAL_LIMIT);
        assert_eq!(goals[0], "Build the scheduler.");
        assert!(goals.contains(&"Never modify files under migrations/.".to_string()));
        assert_eq!(goals.last().unwrap(), "Continue with step 29.");
    }

    #[test]
    fn the_ledger_shows_every_slot_with_sources_and_what_was_superseded() {
        let value = |text: &str, source: &str| StateValue {
            value: text.to_string(),
            provenance: vec![ProvenanceRef {
                kind: ProvenanceKind::UserDecision,
                source_refs: vec![source.to_string()],
                content_hash: String::new(),
            }],
        };
        let state = CheckpointState {
            constraints: vec![value("Use tokio.", "session:b")],
            verification: vec![value("test result: ok", "session:c")],
            retired: vec![RetiredState {
                slot: StateSlot::Constraint,
                kind: TransitionKind::Supersede,
                previous: value("Use async-std.", "session:a"),
                evidence: value("Use tokio.", "session:b"),
            }],
            ..CheckpointState::default()
        };
        let ledger = render_ledger(&state);
        assert!(ledger.contains("## Constraints (user or policy)\n- Use tokio. [session:b]"));
        assert!(ledger.contains("- test result: ok [session:c]"));
        assert!(ledger.contains("- superseded: Use async-std. -> Use tokio. [session:b]"));
        assert!(!ledger.contains("## Goals"), "empty slots are omitted");
    }

    #[test]
    fn directive_words_match_whole_words_only() {
        assert!(is_directive("Do not delete the tests"));
        assert!(is_directive("you can't push to main"));
        assert!(!is_directive(
            "The scheduler keeps jobs and musters workers"
        ));
        assert!(!is_directive("Continue with step 4"));
    }
}
