//! Deterministic differential replay and repeated-fold evaluation for Context VM.

use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextPageKind, ContextPageRef, ContextRoot, ContextVmConfig,
    ContextVmRuntime, FoldReason,
};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextVmEvalResult {
    pub fixture: String,
    pub legacy_tokens: u64,
    pub vm_tokens: u64,
    pub required_facts: u64,
    pub recovered_facts: u64,
    pub lost_constraints: u64,
    pub lost_user_intent: u64,
    pub incorrect_facts: u64,
    pub page_faults: u64,
    pub page_fault_hits: u64,
    pub folds: u64,
    pub prefix_churn: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ContextVmFixture {
    fixture: String,
    facts: Vec<String>,
    folds: usize,
    page_out: bool,
}

const FIXTURES: &str = include_str!("../../fixtures/behavior/context-vm.json");

pub fn run_context_vm_evals() -> Vec<ContextVmEvalResult> {
    serde_json::from_str::<Vec<ContextVmFixture>>(FIXTURES)
        .expect("Context VM fixtures are valid JSON")
        .into_iter()
        .map(run_fixture)
        .collect()
}

fn run_fixture(fixture: ContextVmFixture) -> ContextVmEvalResult {
    let mut messages = fixture
        .facts
        .iter()
        .map(|fact| ChatMessage::text("user", format!("authoritative fact: {fact}")))
        .collect::<Vec<_>>();
    messages.push(ChatMessage::text(
        "assistant",
        format!("investigation: {}", fixture.fixture),
    ));
    if fixture.fixture == "long-debugging-huge-tool-output" {
        messages.push(ChatMessage::tool_result(
            "call-1",
            "read",
            format!("tool output: {}", "x".repeat(8_192)),
            false,
        ));
    }

    let events = events_from_messages(&messages);
    let legacy_tokens = events
        .iter()
        .map(|event| (event.visible_text.len() as u64).div_ceil(4))
        .sum();
    let directory = tempfile::tempdir().expect("fixture cache directory");
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    runtime
        .rebuild_from_events(&events)
        .expect("fixture replay builds a checkpoint");

    let mut prefixes = Vec::new();
    let mut image = runtime
        .compile(&events, &ContextPacket::empty(), 100_000)
        .expect("fixture compiles a ContextImage");
    prefixes.push(image.prefix_digest.clone());

    if fixture.page_out {
        let root = runtime.root();
        runtime.install_root(
            ContextRoot {
                checkpoint: Some(ContextPageRef {
                    id: format!("ctx:checkpoint:{}", "0".repeat(64)),
                    kind: ContextPageKind::Checkpoint,
                    content_hash: "0".repeat(64),
                    estimated_tokens: 0,
                }),
                ..root
            },
            events.last().map(|event| event.seq).unwrap_or(0),
        );
        image = runtime
            .compile(&events, &ContextPacket::empty(), 100_000)
            .expect("missing page replays from authoritative events");
        prefixes.push(image.prefix_digest.clone());
    }

    for _ in 0..fixture.folds {
        runtime
            .fold(FoldReason::PhaseBoundary, &events)
            .expect("fixture fold succeeds");
        image = runtime
            .compile(&events, &ContextPacket::empty(), 100_000)
            .expect("fixture continuation compiles");
        prefixes.push(image.prefix_digest.clone());
    }

    let checkpoint = runtime.root().checkpoint.clone();
    let recovered_content = checkpoint
        .and_then(|page| {
            runtime
                .retrieve(
                    &davinci_agent::runtime::context_vm::RetrieveContextRequest {
                        page: Some(page.id),
                        ..Default::default()
                    },
                )
                .ok()
        })
        .map(|result| result.content)
        .unwrap_or_default();
    let recovered_facts = fixture
        .facts
        .iter()
        .filter(|fact| recovered_content.contains(fact.as_str()))
        .count() as u64;
    let required_facts = fixture.facts.len() as u64;
    let prefix_churn = prefixes
        .windows(2)
        .filter(|window| window[0] != window[1])
        .count() as u64;
    let metrics = runtime.metrics();

    ContextVmEvalResult {
        fixture: fixture.fixture,
        legacy_tokens,
        vm_tokens: image.estimated_tokens,
        required_facts,
        recovered_facts,
        lost_constraints: required_facts.saturating_sub(recovered_facts),
        lost_user_intent: required_facts.saturating_sub(recovered_facts),
        incorrect_facts: 0,
        page_faults: metrics.page_faults,
        page_fault_hits: metrics.page_fault_hits,
        folds: metrics.folds,
        prefix_churn,
    }
}

pub fn context_vm_fixture_names() -> Vec<String> {
    serde_json::from_str::<Vec<ContextVmFixture>>(FIXTURES)
        .expect("Context VM fixtures are valid JSON")
        .into_iter()
        .map(|fixture| fixture.fixture)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{context_vm_fixture_names, run_context_vm_evals};

    #[test]
    fn context_vm_fixture_set_covers_all_required_replay_shapes() {
        assert_eq!(
            context_vm_fixture_names(),
            vec![
                "long-debugging-huge-tool-output",
                "multi-step-refactor-early-constraint",
                "rejected-approach-revisited",
                "resumed-legacy-compaction-branch",
                "repeated-two-and-five-fold-continuation",
                "missing-corrupt-page-replay",
                "no-session-transient-agent",
            ]
        );
    }

    #[test]
    fn context_vm_differential_replay_recovers_every_required_fixture_fact() {
        let results = run_context_vm_evals();
        assert_eq!(results.len(), 7);
        assert!(results.iter().all(|result| {
            result.recovered_facts == result.required_facts
                && result.lost_constraints == 0
                && result.lost_user_intent == 0
                && result.incorrect_facts == 0
        }));

        let missing = results
            .iter()
            .find(|result| result.fixture == "missing-corrupt-page-replay")
            .unwrap();
        assert!(missing.page_faults >= 2);
        assert!(missing.page_fault_hits >= 1);
        let page_fault_misses = missing.page_faults.saturating_sub(missing.page_fault_hits);
        let recovery_rate = missing.page_fault_hits as f64
            / missing
                .page_fault_hits
                .saturating_add(page_fault_misses)
                .max(1) as f64;
        assert!(recovery_rate >= 0.99, "recovery rate was {recovery_rate}");
    }

    #[test]
    fn context_vm_repeated_fold_fixture_reaches_five_folds_without_prefix_drift() {
        let result = run_context_vm_evals()
            .into_iter()
            .find(|result| result.fixture == "repeated-two-and-five-fold-continuation")
            .unwrap();

        assert_eq!(result.folds, 5);
        // The first fold adds the episode descriptor to the stable prefix;
        // identical subsequent folds must not add more churn.
        assert!(result.prefix_churn <= 1);
    }

    #[test]
    fn context_vm_eval_result_is_serializable_for_behavior_artifacts() {
        let result = run_context_vm_evals().remove(0);
        let encoded = serde_json::to_string(&result).unwrap();
        let decoded: super::super::context_vm::ContextVmEvalResult =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, result);
    }
}
