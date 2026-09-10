//! Structured clarification decision contracts.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::LivingPlan;

const MAX_TEXT_BYTES: usize = 8_192;
const MAX_ID_BYTES: usize = 64;
const MAX_EVIDENCE_REFS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Architecture,
    Scope,
    Behavior,
    Permissions,
    Cost,
    Persistence,
    Irreversible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionState {
    Open,
    AnsweredByUser,
    Deferred,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAnswerSource {
    UserInteraction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionAnswer {
    pub source: DecisionAnswerSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_text: Option<String>,
    pub answered_at_ms: u64,
    pub host_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOptionInput {
    pub id: String,
    pub label: String,
    pub explanation: String,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionQuestionInput {
    pub id: String,
    pub kind: DecisionKind,
    pub title: String,
    pub question: String,
    pub materiality: String,
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub options: Vec<DecisionOptionInput>,
    #[serde(default)]
    pub allow_custom: bool,
    #[serde(default)]
    pub custom_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionQuestion {
    pub id: String,
    pub kind: DecisionKind,
    pub title: String,
    pub question: String,
    pub materiality: String,
    pub evidence_refs: Vec<String>,
    pub evidence_fingerprints: BTreeMap<String, String>,
    pub options: Vec<DecisionOptionInput>,
    pub allow_custom: bool,
    pub custom_only: bool,
    pub plan_revision: u64,
    pub state: DecisionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<DecisionAnswer>,
}

pub fn decision_transition(state: &str, event: &str, trusted_user: bool) -> Option<&'static str> {
    match (state, event, trusted_user) {
        ("Open", "recommend", _) => Some("Open"),
        ("Open", "answer", true) => Some("AnsweredByUser"),
        ("Open", "defer", true) => Some("Deferred"),
        ("Open", "cancel", true) => Some("Cancelled"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostDecisionAction {
    AnswerChoice(String),
    AnswerCustom(String),
    Defer,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDecisionReply {
    pub decision_id: String,
    pub expected_plan_revision: u64,
    pub expected_question_revision: u64,
    pub host_event_id: String,
    pub answered_at_ms: u64,
    pub action: HostDecisionAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionApplyOutcome {
    Applied,
    Duplicate,
}

pub(crate) fn apply_host_decision(
    plan: &LivingPlan,
    reply: &HostDecisionReply,
    cwd: &Path,
) -> Result<(LivingPlan, DecisionApplyOutcome), String> {
    valid_id(&reply.decision_id, "decision id")?;
    valid_id(&reply.host_event_id, "host event id")?;

    let current = plan
        .structured_decisions
        .get(&reply.decision_id)
        .ok_or_else(|| format!("Unknown structured decision: {}", reply.decision_id))?;

    if let Some(answer) = &current.answer {
        if answer.host_event_id == reply.host_event_id {
            let exact_duplicate = match &reply.action {
                HostDecisionAction::AnswerChoice(choice_id) => {
                    answer.choice_id.as_deref() == Some(choice_id.as_str())
                        && answer.custom_text.is_none()
                }
                HostDecisionAction::AnswerCustom(text) => {
                    answer.custom_text.as_deref() == Some(text.as_str())
                        && answer.choice_id.is_none()
                }
                HostDecisionAction::Defer | HostDecisionAction::Cancel => false,
            };
            return if exact_duplicate {
                Ok((plan.clone(), DecisionApplyOutcome::Duplicate))
            } else {
                Err("Decision reply idempotency key was reused with different content".into())
            };
        }
    }

    if reply.expected_plan_revision != plan.revision {
        return Err(format!(
            "Decision reply is stale: expected plan revision {}, current revision {}",
            reply.expected_plan_revision, plan.revision
        ));
    }
    if reply.expected_question_revision != current.plan_revision {
        return Err(format!(
            "Decision reply is stale: expected question revision {}, current question revision {}",
            reply.expected_question_revision, current.plan_revision
        ));
    }
    if current.plan_revision != plan.revision {
        return Err(format!(
            "Decision question belongs to stale plan revision {} rather than current revision {}",
            current.plan_revision, plan.revision
        ));
    }

    for (evidence_ref, issued_fingerprint) in &current.evidence_fingerprints {
        let now = plan.decision_evidence_fingerprint(evidence_ref, cwd)?;
        if &now != issued_fingerprint {
            return Err(format!(
                "Decision evidence is stale for {evidence_ref}; ask the question again"
            ));
        }
    }

    let next_state = match &reply.action {
        HostDecisionAction::AnswerChoice(_) | HostDecisionAction::AnswerCustom(_) => {
            decision_transition(state_name(current.state), "answer", true)
        }
        HostDecisionAction::Defer => decision_transition(state_name(current.state), "defer", true),
        HostDecisionAction::Cancel => {
            decision_transition(state_name(current.state), "cancel", true)
        }
    }
    .ok_or_else(|| format!("Decision cannot transition from {:?}", current.state))?;

    let answer = match &reply.action {
        HostDecisionAction::AnswerChoice(choice_id) => {
            valid_id(choice_id, "choice id")?;
            if current.custom_only {
                return Err("Custom-only decision cannot accept a listed option".into());
            }
            if !current.options.iter().any(|option| option.id == *choice_id) {
                return Err(format!("Unknown decision option: {choice_id}"));
            }
            Some(DecisionAnswer {
                source: DecisionAnswerSource::UserInteraction,
                choice_id: Some(choice_id.clone()),
                custom_text: None,
                answered_at_ms: reply.answered_at_ms,
                host_event_id: reply.host_event_id.clone(),
            })
        }
        HostDecisionAction::AnswerCustom(text) => {
            if !current.allow_custom {
                return Err("This decision does not allow a custom answer".into());
            }
            bounded_text(text, "custom answer")?;
            Some(DecisionAnswer {
                source: DecisionAnswerSource::UserInteraction,
                choice_id: None,
                custom_text: Some(text.clone()),
                answered_at_ms: reply.answered_at_ms,
                host_event_id: reply.host_event_id.clone(),
            })
        }
        HostDecisionAction::Defer | HostDecisionAction::Cancel => None,
    };

    let mut next = plan.clone();
    next.revision = plan
        .revision
        .checked_add(1)
        .ok_or("Plan revision exhausted")?;
    next.approved_revision =
        crate::living_plan::approval_after_decision(plan.approved_revision, true);
    let decision = next
        .structured_decisions
        .get_mut(&reply.decision_id)
        .expect("decision cloned from validated plan");
    decision.state = match next_state {
        "AnsweredByUser" => DecisionState::AnsweredByUser,
        "Deferred" => DecisionState::Deferred,
        "Cancelled" => DecisionState::Cancelled,
        _ => return Err("Unexpected decision transition".into()),
    };
    decision.plan_revision = next.revision;
    decision.answer = answer;
    next.changes.push(format!(
        "decision {} updated to {:?}",
        reply.decision_id, decision.state
    ));
    next.invalidate_dependent_step_decisions(&reply.decision_id);
    Ok((next, DecisionApplyOutcome::Applied))
}

fn state_name(state: DecisionState) -> &'static str {
    match state {
        DecisionState::Open => "Open",
        DecisionState::AnsweredByUser => "AnsweredByUser",
        DecisionState::Deferred => "Deferred",
        DecisionState::Cancelled => "Cancelled",
    }
}

pub fn valid_question_shape(
    options: usize,
    recommended: usize,
    material: bool,
    has_evidence: bool,
    custom_only: bool,
) -> bool {
    material
        && has_evidence
        && recommended <= 1
        && if custom_only {
            options == 0 && recommended == 0
        } else {
            (2..=4).contains(&options)
        }
}

/// Validate model-proposed question structure against host-owned plan evidence.
/// Evidence paths/fingerprints are resolved from the current Living Plan; model
/// input cannot manufacture verified evidence or its digest.
pub fn validate_question_for_plan(
    input: DecisionQuestionInput,
    plan: &LivingPlan,
    cwd: &Path,
) -> Result<DecisionQuestion, String> {
    valid_id(&input.id, "question id")?;
    bounded_text(&input.title, "question title")?;
    bounded_text(&input.question, "question")?;
    bounded_text(&input.materiality, "materiality")?;

    if input.evidence_refs.is_empty() || input.evidence_refs.len() > MAX_EVIDENCE_REFS {
        return Err(format!(
            "A material decision needs 1-{MAX_EVIDENCE_REFS} current evidence references"
        ));
    }

    let recommended = input
        .options
        .iter()
        .filter(|option| option.recommended)
        .count();
    if !valid_question_shape(
        input.options.len(),
        recommended,
        !input.materiality.trim().is_empty(),
        !input.evidence_refs.is_empty(),
        input.custom_only,
    ) {
        return Err("Question must have 2-4 options (or a valid custom-only exception), at most one recommendation, materiality, and evidence".into());
    }
    if input.custom_only && !input.allow_custom {
        return Err("Custom-only questions must enable the custom-answer route".into());
    }

    let mut option_ids = BTreeSet::new();
    for option in &input.options {
        valid_id(&option.id, "option id")?;
        if !option_ids.insert(option.id.clone()) {
            return Err(format!("Duplicate option id: {}", option.id));
        }
        bounded_text(&option.label, "option label")?;
        bounded_text(&option.explanation, "option explanation")?;
    }

    let mut evidence_ids = BTreeSet::new();
    let mut evidence_fingerprints = BTreeMap::new();
    for evidence_ref in &input.evidence_refs {
        if !evidence_ids.insert(evidence_ref.clone()) {
            return Err(format!("Duplicate evidence reference: {evidence_ref}"));
        }
        let fingerprint = plan.decision_evidence_fingerprint(evidence_ref, cwd)?;
        evidence_fingerprints.insert(evidence_ref.clone(), fingerprint);
    }

    Ok(DecisionQuestion {
        id: input.id,
        kind: input.kind,
        title: input.title,
        question: input.question,
        materiality: input.materiality,
        evidence_refs: input.evidence_refs,
        evidence_fingerprints,
        options: input.options,
        allow_custom: input.allow_custom,
        custom_only: input.custom_only,
        plan_revision: plan.revision,
        state: DecisionState::Open,
        answer: None,
    })
}

fn valid_id(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "{field} must be 1-{MAX_ID_BYTES} ASCII letters, digits, hyphens or underscores"
        ));
    }
    Ok(())
}

fn bounded_text(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "{field} must contain 1-{MAX_TEXT_BYTES} bytes of meaningful text"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn f02_question_bounds() {
        assert!(valid_question_shape(3, 1, true, true, false));
        assert!(!valid_question_shape(3, 2, true, true, false));
        assert!(!valid_question_shape(1, 0, true, true, false));
        assert!(valid_question_shape(0, 0, true, true, true));
    }

    #[test]
    fn f02_recommendation_not_answer() {
        assert_eq!(
            decision_transition("Open", "recommend", false),
            Some("Open")
        );
        assert_eq!(decision_transition("Open", "answer", false), None);
        assert_eq!(
            decision_transition("Open", "answer", true),
            Some("AnsweredByUser")
        );
        assert_eq!(decision_transition("AnsweredByUser", "answer", true), None);
        assert_eq!(decision_transition("Open", "cancel", false), None);
        assert_eq!(
            decision_transition("Open", "cancel", true),
            Some("Cancelled")
        );
    }

    fn fixture() -> (tempfile::TempDir, LivingPlan) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
        let mut plan = LivingPlan::default();
        plan.update(
            &json!({
                "expected_revision": 0,
                "goal": "Choose cache behavior",
                "evidence": [{"path":"src.rs","finding":"Existing process-local cache"}],
                "steps": [{
                    "id":"cache",
                    "change":"Implement cache",
                    "files":["src.rs"],
                    "why":"Required behavior",
                    "depends_on":[],
                    "verify":["cargo test --offline"]
                }]
            }),
            dir.path(),
        )
        .unwrap();
        (dir, plan)
    }

    fn question() -> DecisionQuestionInput {
        DecisionQuestionInput {
            id: "cache-scope".into(),
            kind: DecisionKind::Persistence,
            title: "Cache scope".into(),
            question: "Which caching strategy should this feature use?".into(),
            materiality: "Persistence semantics change materially.".into(),
            evidence_refs: vec!["src.rs".into()],
            options: vec![
                DecisionOptionInput {
                    id: "memory".into(),
                    label: "In-memory LRU".into(),
                    explanation: "No new infrastructure".into(),
                    recommended: true,
                },
                DecisionOptionInput {
                    id: "sqlite".into(),
                    label: "SQLite".into(),
                    explanation: "Persists across restarts".into(),
                    recommended: false,
                },
            ],
            allow_custom: true,
            custom_only: false,
        }
    }

    #[test]
    fn duplicate_ids_and_two_recommendations_are_rejected() {
        let (dir, plan) = fixture();
        let mut input = question();
        input.options[1].id = input.options[0].id.clone();
        assert!(validate_question_for_plan(input, &plan, dir.path())
            .unwrap_err()
            .contains("Duplicate option"));

        let mut input = question();
        input.options[1].recommended = true;
        assert!(validate_question_for_plan(input, &plan, dir.path()).is_err());
    }

    #[test]
    fn missing_and_stale_evidence_are_rejected() {
        let (dir, plan) = fixture();
        let mut missing = question();
        missing.evidence_refs = vec!["missing.rs".into()];
        assert!(validate_question_for_plan(missing, &plan, dir.path()).is_err());

        std::fs::write(dir.path().join("src.rs"), "fn changed() {}\n").unwrap();
        assert!(validate_question_for_plan(question(), &plan, dir.path())
            .unwrap_err()
            .contains("stale"));
    }

    #[test]
    fn materiality_option_and_custom_only_bounds_are_enforced() {
        let (dir, plan) = fixture();
        let mut empty_materiality = question();
        empty_materiality.materiality = "  ".into();
        assert!(validate_question_for_plan(empty_materiality, &plan, dir.path()).is_err());

        let mut fifth = question();
        fifth.options.extend([
            DecisionOptionInput {
                id: "redis".into(),
                label: "Redis".into(),
                explanation: "External service".into(),
                recommended: false,
            },
            DecisionOptionInput {
                id: "disk".into(),
                label: "Disk".into(),
                explanation: "Local files".into(),
                recommended: false,
            },
            DecisionOptionInput {
                id: "other".into(),
                label: "Other".into(),
                explanation: "Another concrete choice".into(),
                recommended: false,
            },
        ]);
        assert!(validate_question_for_plan(fifth, &plan, dir.path()).is_err());

        let mut custom = question();
        custom.options.clear();
        custom.custom_only = true;
        custom.allow_custom = false;
        assert!(validate_question_for_plan(custom.clone(), &plan, dir.path()).is_err());
        custom.allow_custom = true;
        assert!(validate_question_for_plan(custom, &plan, dir.path()).is_ok());
    }

    #[test]
    fn overlength_unicode_input_is_rejected_by_bytes() {
        let (dir, plan) = fixture();
        let mut input = question();
        input.question = "é".repeat(4_097);
        assert!(input.question.len() > MAX_TEXT_BYTES);
        assert!(validate_question_for_plan(input, &plan, dir.path()).is_err());
    }

    #[test]
    fn model_input_denies_unknown_fields() {
        for forbidden in [
            "answer",
            "state",
            "actor",
            "approved_revision",
            "permission_mode",
        ] {
            let mut value = json!({
                "id":"q","kind":"scope","title":"t","question":"q","materiality":"m",
                "evidence_refs":["src.rs"],"options":[],"allow_custom":true,"custom_only":true
            });
            value[forbidden] = json!("forged");
            assert!(
                serde_json::from_value::<DecisionQuestionInput>(value).is_err(),
                "model input unexpectedly accepted authority field {forbidden}"
            );
        }
    }

    #[test]
    fn validated_question_reserves_typed_authenticated_answer_provenance() {
        let (dir, plan) = fixture();
        let mut validated = validate_question_for_plan(question(), &plan, dir.path()).unwrap();
        assert_eq!(validated.state, DecisionState::Open);
        assert!(validated.answer.is_none());
        validated.answer = Some(DecisionAnswer {
            source: DecisionAnswerSource::UserInteraction,
            choice_id: Some("sqlite".into()),
            custom_text: None,
            answered_at_ms: 42,
            host_event_id: "host-event-1".into(),
        });
        assert_eq!(
            validated.answer.as_ref().unwrap().source,
            DecisionAnswerSource::UserInteraction
        );
    }

    fn plan_with_question() -> (tempfile::TempDir, LivingPlan) {
        let (dir, mut plan) = fixture();
        let q = validate_question_for_plan(question(), &plan, dir.path()).unwrap();
        plan.structured_decisions.insert(q.id.clone(), q);
        (dir, plan)
    }

    fn reply(action: HostDecisionAction) -> HostDecisionReply {
        HostDecisionReply {
            decision_id: "cache-scope".into(),
            expected_plan_revision: 1,
            expected_question_revision: 1,
            host_event_id: "host-event-1".into(),
            answered_at_ms: 42,
            action,
        }
    }

    #[test]
    fn f02_authenticated_answer_validates_revision_choice_and_idempotency() {
        let (dir, plan) = plan_with_question();
        let choice = reply(HostDecisionAction::AnswerChoice("sqlite".into()));
        let (answered, outcome) = apply_host_decision(&plan, &choice, dir.path()).unwrap();
        assert_eq!(outcome, DecisionApplyOutcome::Applied);
        assert_eq!(answered.revision, 2);
        let stored = &answered.structured_decisions["cache-scope"];
        assert_eq!(stored.state, DecisionState::AnsweredByUser);
        assert_eq!(
            stored.answer.as_ref().unwrap().choice_id.as_deref(),
            Some("sqlite")
        );
        assert_eq!(
            apply_host_decision(&answered, &choice, dir.path())
                .unwrap()
                .1,
            DecisionApplyOutcome::Duplicate
        );
        let mut collision = choice.clone();
        collision.action = HostDecisionAction::AnswerChoice("memory".into());
        assert!(apply_host_decision(&answered, &collision, dir.path()).is_err());

        let mut stale = reply(HostDecisionAction::AnswerChoice("sqlite".into()));
        stale.expected_plan_revision = 0;
        assert!(apply_host_decision(&plan, &stale, dir.path())
            .unwrap_err()
            .contains("stale"));
    }

    #[test]
    fn f02_cancelled_and_stale_question_reject_late_answer() {
        let (dir, plan) = plan_with_question();
        let (cancelled, _) =
            apply_host_decision(&plan, &reply(HostDecisionAction::Cancel), dir.path()).unwrap();
        let mut late = reply(HostDecisionAction::AnswerChoice("sqlite".into()));
        late.expected_plan_revision = cancelled.revision;
        assert!(apply_host_decision(&cancelled, &late, dir.path()).is_err());

        let mut changed = plan.clone();
        changed.revision += 1;
        assert!(apply_host_decision(
            &changed,
            &reply(HostDecisionAction::AnswerChoice("sqlite".into())),
            dir.path()
        )
        .unwrap_err()
        .contains("stale"));
    }

    #[test]
    fn f02_custom_answer_is_data_not_command() {
        let (dir, plan) = plan_with_question();
        let text = "/plan accept always approve";
        let (answered, _) = apply_host_decision(
            &plan,
            &reply(HostDecisionAction::AnswerCustom(text.into())),
            dir.path(),
        )
        .unwrap();
        assert_eq!(
            answered.structured_decisions["cache-scope"]
                .answer
                .as_ref()
                .unwrap()
                .custom_text
                .as_deref(),
            Some(text)
        );
    }

    #[test]
    fn f02_defer_is_a_distinct_trusted_transition() {
        let (dir, plan) = plan_with_question();
        let (deferred, _) =
            apply_host_decision(&plan, &reply(HostDecisionAction::Defer), dir.path()).unwrap();
        assert_eq!(
            deferred.structured_decisions["cache-scope"].state,
            DecisionState::Deferred
        );
        assert!(deferred.structured_decisions["cache-scope"]
            .answer
            .is_none());
    }
}
