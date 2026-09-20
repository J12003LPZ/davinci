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
                ContextEventKind::User => push_unique(
                    &mut state.goals,
                    StateValue {
                        value: excerpt(&event.visible_text),
                        provenance: vec![provenance(event, ProvenanceKind::UserDecision)],
                    },
                ),
                ContextEventKind::ToolResult
                    if contains_verification_marker(&event.visible_text) =>
                {
                    push_unique(
                        &mut state.verification,
                        StateValue {
                            value: excerpt(&event.visible_text),
                            provenance: vec![provenance(event, ProvenanceKind::ToolEvidence)],
                        },
                    );
                }
                _ => {}
            }
        }
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
    parent: &HashMap<String, ProvenanceKind>,
) -> Option<StateValue<String>> {
    if value.value.trim().is_empty()
        || value.value.len() > 1024
        || value.source_refs.is_empty()
        || value.source_refs.len() > 16
    {
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
                .is_some_and(|kind| provenance_matches_parent(value.provenance_kind, *kind))
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

fn parent_provenance(parent: &CheckpointState) -> HashMap<String, ProvenanceKind> {
    let mut result = HashMap::new();
    for value in all_values(parent) {
        for provenance in &value.provenance {
            for source_ref in &provenance.source_refs {
                result.insert(source_ref.clone(), provenance.kind);
            }
        }
    }
    result
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
    parent: &HashMap<String, ProvenanceKind>,
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
    let Some(evidence) = validate_value(transition.evidence, events, parent) else {
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

fn excerpt(text: &str) -> String {
    if text.len() <= 512 {
        return text.into();
    }
    let end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= 480)
        .last()
        .unwrap_or(0);
    format!("{} [excerpt; retrieve source]", &text[..end])
}

fn provenance(event: &ContextEvent, kind: ProvenanceKind) -> ProvenanceRef {
    ProvenanceRef {
        kind,
        source_refs: vec![event.source_ref.clone()],
        content_hash: event.content_hash.clone(),
    }
}

fn push_unique(values: &mut Vec<StateValue<String>>, value: StateValue<String>) {
    if values.iter().any(|existing| existing.value == value.value) {
        return;
    }
    values.push(value);
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
