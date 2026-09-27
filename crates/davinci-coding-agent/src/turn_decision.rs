//! Bounded, ready-only decision intelligence for coding submissions.
//!
//! A submission enqueues bounded background work after installing the user
//! message. The shared agent loop consumes current-turn results at request
//! boundaries, so both print and interactive request 1 stay deterministic.

use davinci_agent::decision::provider::DecisionError;
use davinci_agent::decision::response::{DecisionAnswer, DecisionResponse};
use davinci_agent::decision::{DecisionAdviceKey, DecisionRuntime};
use davinci_agent::effort;
use davinci_agent::Agent;
use davinci_protocol::ThinkingLevel as ProtocolThinkingLevel;
use serde_json::json;

use crate::decision_state::{
    requirement_question_id, DecisionMetadata, DecisionState, RequirementLedger,
};

#[derive(Debug, Clone)]
pub struct DecisionSnapshot {
    pub request_id: String,
    pub task: String,
    pub decision_class: davinci_agent::decision::risk::DecisionClass,
    pub metadata: DecisionMetadata,
    pub evidence_revision: u64,
    pub mutation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionAdmission {
    Enqueued,
    Disabled,
    Busy,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviceAbsence {
    NoReadyResult,
    Stale,
    NotApplicable,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortAdviceOutcome {
    Ready(ProtocolThinkingLevel),
    Absent(AdviceAbsence),
}

pub use davinci_agent::decision::advice::{RequirementAdvice, RequirementJudgment};

/// Map only validated requirement Choice answers. Evidence references remain
/// empty until the host supplies concrete current checks; a provider answer
/// alone can never authorize completion.
pub fn requirement_advice(
    response: &DecisionResponse,
    key: &DecisionAdviceKey,
    ledger: &RequirementLedger,
) -> Vec<RequirementAdvice> {
    ledger
        .entries
        .iter()
        .filter_map(|requirement| {
            let answer = response
                .answers
                .get(&requirement_question_id(&requirement.id))?;
            let DecisionAnswer::Choice {
                choice, confidence, ..
            } = answer
            else {
                return None;
            };
            let judgment = match choice.as_str() {
                "supported" => RequirementJudgment::Supported,
                "possibly_missing" => RequirementJudgment::PossiblyMissing,
                "uncertain" => RequirementJudgment::Uncertain,
                _ => return None,
            };
            Some(RequirementAdvice {
                requirement_id: requirement.id.clone(),
                judgment,
                confidence: *confidence,
                evidence_references: Vec::new(),
                key: key.clone(),
            })
        })
        .collect()
}

/// Admit work for the current real user turn. Call after `prompt_user_with`.
/// Disabled paths never construct a request; enabled preparation runs on the
/// runtime's single bounded worker, with no filesystem work on submission.
pub fn prepare_turn_decision(agent: &mut Agent, snapshot: DecisionSnapshot) -> DecisionAdmission {
    use davinci_agent::decision::advice::DecisionRequestFactory;
    use std::sync::Arc;
    let Some(runtime) = agent.decision_runtime() else {
        agent.set_decision_effort_advice(None);
        return DecisionAdmission::Disabled;
    };
    let features = agent.decision_advice_features();
    if !runtime.is_enabled() || !features.any() {
        agent.set_decision_effort_advice(None);
        return DecisionAdmission::Disabled;
    }
    let key = DecisionAdviceKey {
        request_id: snapshot.request_id,
        generation: runtime.generation(),
        evidence_revision: snapshot.evidence_revision,
        mutation_revision: snapshot.mutation_revision,
    };
    let factory = DecisionRequestFactory(Arc::new(move |key, completion| {
        let is_completion = completion.is_some();
        let mut additional_state = serde_json::Map::new();
        if let Some(completion) = completion {
            // Evidence is a bounded projection of checks/paths already held
            // by the host. Apply the same secret redactor as task context.
            let encoded = serde_json::to_string(&completion).unwrap_or_default();
            let redacted = davinci_agent::runtime::contracts::redact_secrets(&encoded);
            let value = serde_json::from_str::<serde_json::Value>(&redacted)
                .unwrap_or_else(|_| json!({"status": "unavailable"}));
            additional_state.insert("completionEvidence".into(), value);
        }
        additional_state.insert(
            "__davinci_decision_key".to_owned(),
            json!({
                "requestId": key.request_id,
                "generation": key.generation,
                "evidenceRevision": key.evidence_revision,
                "mutationRevision": key.mutation_revision,
            }),
        );
        DecisionState::from_task_with_metadata(&snapshot.task, snapshot.metadata.clone())
            .request_selected(
                key.request_id,
                snapshot.decision_class,
                |id| {
                    if is_completion {
                        features.completion && id.starts_with("requirement_")
                    } else if matches!(id, "regression_risk" | "verification_scope") {
                        features.effort
                    } else {
                        features.tool_families && id.ends_with("_relevant")
                    }
                },
                is_completion,
                &additional_state,
            )
    }));
    match agent.prepare_decision_advice(key, factory) {
        Ok(()) => DecisionAdmission::Enqueued,
        Err(error) => admission_for_error(error),
    }
}

pub fn try_effort_advice(
    runtime: &DecisionRuntime,
    key: &DecisionAdviceKey,
    base: ProtocolThinkingLevel,
    deterministic: ProtocolThinkingLevel,
) -> EffortAdviceOutcome {
    let Some(ready) = runtime.take_ready_shadow() else {
        return EffortAdviceOutcome::Absent(AdviceAbsence::NoReadyResult);
    };
    if !runtime.is_enabled() || ready.key != *key || ready.key.generation != runtime.generation() {
        return EffortAdviceOutcome::Absent(AdviceAbsence::Stale);
    }
    if ready.request.request_id != key.request_id || !request_carries_key(&ready.request.state, key)
    {
        return EffortAdviceOutcome::Absent(AdviceAbsence::Stale);
    }
    let Some(recommended) = recommended_effort(&ready.response) else {
        return EffortAdviceOutcome::Absent(AdviceAbsence::NotApplicable);
    };
    let recommended = effort::favor_high_effort(recommended);
    if effort_rank(recommended) > effort_rank(deterministic) {
        EffortAdviceOutcome::Ready(recommended)
    } else if effort_rank(base) > effort_rank(deterministic) {
        // A deterministic failure escalation remains the source of truth.
        EffortAdviceOutcome::Absent(AdviceAbsence::NotApplicable)
    } else {
        EffortAdviceOutcome::Absent(AdviceAbsence::NotApplicable)
    }
}

fn request_carries_key(state: &serde_json::Value, key: &DecisionAdviceKey) -> bool {
    let Some(value) = state.get("__davinci_decision_key") else {
        return false;
    };
    value.get("requestId").and_then(|value| value.as_str()) == Some(key.request_id.as_str())
        && value.get("generation").and_then(|value| value.as_u64()) == Some(key.generation)
        && value
            .get("evidenceRevision")
            .and_then(|value| value.as_u64())
            == Some(key.evidence_revision)
        && value
            .get("mutationRevision")
            .and_then(|value| value.as_u64())
            == Some(key.mutation_revision)
}

fn recommended_effort(response: &DecisionResponse) -> Option<ProtocolThinkingLevel> {
    let risk = match response.answers.get("regression_risk") {
        Some(DecisionAnswer::Score { score, .. }) => *score >= 2.0,
        _ => false,
    };
    let broad_verification = matches!(
        response.answers.get("verification_scope"),
        Some(DecisionAnswer::Choice { choice, .. }) if choice == "full"
    );
    (risk || broad_verification).then_some(ProtocolThinkingLevel::Medium)
}

fn effort_rank(level: ProtocolThinkingLevel) -> u8 {
    match level {
        ProtocolThinkingLevel::Off => 0,
        ProtocolThinkingLevel::Minimal => 1,
        ProtocolThinkingLevel::Low => 2,
        ProtocolThinkingLevel::Medium => 3,
        ProtocolThinkingLevel::High => 4,
        ProtocolThinkingLevel::Xhigh => 5,
        ProtocolThinkingLevel::Max => 6,
    }
}

fn admission_for_error(error: DecisionError) -> DecisionAdmission {
    match error {
        DecisionError::Busy => DecisionAdmission::Busy,
        DecisionError::Disabled => DecisionAdmission::Disabled,
        DecisionError::InvalidRequest(_) => DecisionAdmission::Invalid,
        DecisionError::CredentialInvalid
        | DecisionError::RateLimited
        | DecisionError::Overloaded
        | DecisionError::Unavailable(_)
        | DecisionError::SchemaMismatch(_)
        | DecisionError::StaleResponse
        | DecisionError::HttpStatus(_) => DecisionAdmission::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use davinci_agent::decision::response::DecisionAnswer;

    fn key(revision: u64) -> DecisionAdviceKey {
        DecisionAdviceKey {
            request_id: "turn-1".into(),
            generation: 4,
            evidence_revision: revision,
            mutation_revision: 2,
        }
    }

    #[test]
    fn advice_requires_all_freshness_material() {
        let state = json!({
            "__davinci_decision_key": {
                "requestId": "turn-1",
                "generation": 4,
                "evidenceRevision": 8,
                "mutationRevision": 2
            }
        });
        assert!(request_carries_key(&state, &key(8)));
        assert!(!request_carries_key(&state, &key(9)));
    }

    #[test]
    fn risky_answer_is_the_only_advice_that_can_raise_effort() {
        let risky = DecisionResponse {
            answers: BTreeMap::from([(
                "regression_risk".into(),
                DecisionAnswer::Score {
                    score: 2.0,
                    levels: 4,
                    probabilities: BTreeMap::new(),
                    confidence: 0.9,
                },
            )]),
            model: None,
            input_tokens: None,
            output_tokens: None,
        };
        assert_eq!(
            recommended_effort(&risky),
            Some(ProtocolThinkingLevel::Medium)
        );
    }

    #[test]
    fn requirement_advice_keeps_the_freshness_key_and_never_invents_evidence() {
        let ledger = RequirementLedger::from_task("Must preserve the API");
        let id = ledger.entries[0].id.clone();
        let response = DecisionResponse {
            answers: BTreeMap::from([(
                requirement_question_id(&id),
                DecisionAnswer::Choice {
                    choice: "possibly_missing".into(),
                    probabilities: BTreeMap::from([
                        ("supported".into(), 0.1),
                        ("possibly_missing".into(), 0.8),
                        ("uncertain".into(), 0.1),
                    ]),
                    confidence: 0.8,
                },
            )]),
            model: Some("fixture".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
        };
        let advice = requirement_advice(&response, &key(8), &ledger);
        assert_eq!(advice.len(), 1);
        assert_eq!(advice[0].requirement_id, id);
        assert_eq!(advice[0].judgment, RequirementJudgment::PossiblyMissing);
        assert!(advice[0].evidence_references.is_empty());
        assert_eq!(advice[0].key, key(8));
    }
}
