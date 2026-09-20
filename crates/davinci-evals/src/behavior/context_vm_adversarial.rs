//! Mixed-role state lifecycle evaluations. Expected answers are independent
//! fixtures; proposals exercise the same runtime validation boundary as a fold.
use super::ContextVmEvalResult;
use davinci_agent::runtime::{context_manifest::ProvenanceKind, context_vm::*};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;
use serde::Deserialize;

#[derive(Deserialize)]
struct Message {
    id: String,
    role: String,
    text: String,
}
#[derive(Deserialize)]
struct Pageable {
    source: String,
    text: String,
}
#[derive(Deserialize)]
struct Case {
    fixture: String,
    messages: Vec<Message>,
    proposal: CheckpointProposal,
    resident: Vec<String>,
    pageable: Vec<Pageable>,
    forbidden: Vec<String>,
}
const CORPUS: &str = include_str!("../../fixtures/behavior/context-vm-adversarial.json");

pub(super) fn names() -> Vec<String> {
    cases().into_iter().map(|case| case.fixture).collect()
}
fn cases() -> Vec<Case> {
    serde_json::from_str(CORPUS).expect("adversarial corpus")
}

pub(super) fn run() -> Vec<ContextVmEvalResult> {
    cases()
        .into_iter()
        .map(|case| {
            let messages: Vec<_> = case
                .messages
                .iter()
                .map(|message| {
                    if message.role == "toolResult" {
                        ChatMessage::tool_result(&message.id, "fixture", &message.text, false)
                    } else {
                        ChatMessage::text(
                            if message.role == "policy" {
                                "custom"
                            } else {
                                &message.role
                            },
                            &message.text,
                        )
                    }
                })
                .collect();
            let mut events = events_from_messages(&messages);
            assert_eq!(events.len(), case.messages.len());
            for (event, source) in events.iter_mut().zip(&case.messages) {
                event.source_ref = format!("fixture:{}", source.id);
                if source.role == "policy" {
                    event.provenance_kind = ProvenanceKind::MandatoryPolicy;
                }
            }
            let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
            vm.fold_with_proposal(FoldReason::Manual, &events, Some(case.proposal))
                .unwrap();
            let state = vm.load_state_from_root().unwrap();
            let active: Vec<_> = state
                .goals
                .iter()
                .chain(&state.constraints)
                .chain(&state.in_progress)
                .chain(&state.blockers)
                .chain(&state.decisions)
                .chain(&state.modified_files)
                .chain(&state.verification)
                .collect();
            let missing = case
                .resident
                .iter()
                .filter(|required| !active.iter().any(|value| &value.value == *required))
                .count();
            let incorrect = case
                .forbidden
                .iter()
                .filter(|forbidden| active.iter().any(|value| &value.value == *forbidden))
                .count();
            assert!(active.iter().all(|value| value.provenance.iter().all(|p| !p
                .source_refs
                .is_empty()
                && p.source_refs
                    .iter()
                    .all(|r| events.iter().any(|e| &e.source_ref == r)))));
            let retrieved = case
                .pageable
                .iter()
                .filter(|required| {
                    vm.retrieve(&RetrieveContextRequest {
                        source_ref: Some(required.source.clone()),
                        ..Default::default()
                    })
                    .is_ok_and(|result| result.content.contains(&required.text))
                })
                .count();
            let image = vm
                .compile(&events, &ContextPacket::empty(), 100_000)
                .unwrap();
            let required = case.resident.len() + case.pageable.len();
            let metrics = vm.metrics();
            ContextVmEvalResult {
                fixture: case.fixture,
                legacy_tokens: messages
                    .iter()
                    .map(davinci_agent::provider_budget::message_token_ceiling)
                    .sum(),
                vm_tokens: image.estimated_tokens,
                required_facts: required as u64,
                recovered_facts: (case.resident.len() - missing + retrieved) as u64,
                lost_constraints: missing as u64,
                lost_user_intent: missing as u64,
                incorrect_facts: incorrect as u64,
                page_faults: metrics.page_faults,
                page_fault_hits: metrics.page_fault_hits,
                folds: metrics.folds,
                prefix_churn: 0,
            }
        })
        .collect()
}
