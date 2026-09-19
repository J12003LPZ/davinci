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
            goals: validate(proposal.goals),
            constraints: validate(proposal.constraints),
            completed: validate(proposal.completed),
            in_progress: validate(proposal.in_progress),
            blockers: validate(proposal.blockers),
            decisions: validate(proposal.decisions),
            modified_files: validate(proposal.modified_files),
            verification: validate(proposal.verification),
            narrative: proposal
                .narrative
                .and_then(|value| validate_value(value, &event_map, &parent_map)),
        };
        // A partial proposal is allowed to update one field without erasing
        // state that was already proven in an earlier checkpoint. There is no
        // delete operation in the fold contract, so replacement is expressed
        // by a later value with the same semantic slot/source in the proposal.
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
        };
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
                        value: event.visible_text.clone(),
                        provenance: vec![provenance(event, ProvenanceKind::UserDecision)],
                    },
                ),
                ContextEventKind::ToolResult
                    if contains_verification_marker(&event.visible_text) =>
                {
                    push_unique(
                        &mut state.verification,
                        StateValue {
                            value: event.visible_text.clone(),
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

fn validate_value(
    value: ProposedStateValue,
    events: &HashMap<&str, &ContextEvent>,
    parent: &HashMap<String, ProvenanceKind>,
) -> Option<StateValue<String>> {
    if value.value.trim().is_empty() || value.source_refs.is_empty() {
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
