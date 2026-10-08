//! Structured clarification decision contracts.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::LivingPlan;

const MAX_TEXT_BYTES: usize = 8_192;
const MAX_ID_BYTES: usize = 64;
const MAX_EVIDENCE_REFS: usize = 16;
/// Most questions one `ask_user_question` call may put to the user at once.
pub const MAX_BATCH_QUESTIONS: usize = 4;

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
    /// The one option picked on a single-select question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    /// Every option checked on a multi-select question, in option order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choice_ids: Vec<String>,
    /// Free text. On a single-select question it replaces the options; on a
    /// multi-select question it is one more answer next to `choice_ids`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_text: Option<String>,
    pub answered_at_ms: u64,
    pub host_event_id: String,
}

impl DecisionAnswer {
    /// The selected option ids, whichever shape of question was answered.
    pub fn selected_ids(&self) -> Vec<&str> {
        match &self.choice_id {
            Some(id) => vec![id.as_str()],
            None => self.choice_ids.iter().map(String::as_str).collect(),
        }
    }
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
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
}

/// The tool input: 1-4 questions shown together in one dialog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionBatchInput {
    questions: QuestionList,
}

/// Some models send a nested array as a JSON string; both spellings parse.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum QuestionList {
    Items(Vec<DecisionQuestionInput>),
    Encoded(String),
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
    /// Absent in sessions saved before multi-select existed: single-select.
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
    pub plan_revision: u64,
    pub state: DecisionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<DecisionAnswer>,
}

impl DecisionQuestion {
    /// How many answers a submission must carry, inclusive. A filled-in
    /// custom answer counts as one. Single-select is always exactly one.
    pub fn selection_bounds(&self) -> (usize, usize) {
        selection_bounds(
            self.multi_select,
            self.min_selections,
            self.max_selections,
            self.options.len(),
            self.allow_custom,
        )
    }
}

fn selection_bounds(
    multi_select: bool,
    min: Option<usize>,
    max: Option<usize>,
    options: usize,
    allow_custom: bool,
) -> (usize, usize) {
    if !multi_select {
        return (1, 1);
    }
    let capacity = options + usize::from(allow_custom);
    (min.unwrap_or(1), max.unwrap_or(capacity))
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
    /// A multi-select answer: the checked options plus optional free text.
    AnswerChoices {
        choice_ids: Vec<String>,
        custom_text: Option<String>,
    },
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

/// One user submission answering every question of a dialog at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDecisionBatchReply {
    pub answers: BTreeMap<String, HostDecisionAction>,
    pub expected_plan_revision: u64,
    pub expected_question_revision: u64,
    pub host_event_id: String,
    pub answered_at_ms: u64,
}

pub(crate) fn apply_host_decision(
    plan: &LivingPlan,
    reply: &HostDecisionReply,
    cwd: &Path,
) -> Result<(LivingPlan, DecisionApplyOutcome), String> {
    valid_id(&reply.decision_id, "decision id")?;
    apply_host_decision_batch(
        plan,
        &HostDecisionBatchReply {
            answers: BTreeMap::from([(reply.decision_id.clone(), reply.action.clone())]),
            expected_plan_revision: reply.expected_plan_revision,
            expected_question_revision: reply.expected_question_revision,
            host_event_id: reply.host_event_id.clone(),
            answered_at_ms: reply.answered_at_ms,
        },
        cwd,
    )
}

/// Apply every answer of one submission or none of them. Every check runs
/// before the plan is cloned, so an invalid answer to question four leaves
/// questions one to three open too.
pub(crate) fn apply_host_decision_batch(
    plan: &LivingPlan,
    reply: &HostDecisionBatchReply,
    cwd: &Path,
) -> Result<(LivingPlan, DecisionApplyOutcome), String> {
    valid_id(&reply.host_event_id, "host event id")?;
    if reply.answers.is_empty() || reply.answers.len() > MAX_BATCH_QUESTIONS {
        return Err(format!(
            "A decision reply must answer 1-{MAX_BATCH_QUESTIONS} questions"
        ));
    }
    let mut current = Vec::with_capacity(reply.answers.len());
    for (id, action) in &reply.answers {
        valid_id(id, "decision id")?;
        let decision = plan
            .structured_decisions
            .get(id)
            .ok_or_else(|| format!("Unknown structured decision: {id}"))?;
        current.push((decision, action));
    }

    // A replayed submission is recognised by its idempotency key, and only
    // if it carries exactly the answers already stored under that key.
    let replayed = current
        .iter()
        .filter(|(decision, _)| {
            decision
                .answer
                .as_ref()
                .is_some_and(|answer| answer.host_event_id == reply.host_event_id)
        })
        .count();
    if replayed > 0 {
        let exact_duplicate = replayed == current.len()
            && current.iter().all(|(decision, action)| {
                let stored = decision.answer.as_ref().expect("counted above");
                matches!(
                    answer_content(decision, action),
                    Ok(Some(content)) if content.matches(stored)
                )
            });
        return if exact_duplicate {
            Ok((plan.clone(), DecisionApplyOutcome::Duplicate))
        } else {
            Err("Decision reply idempotency key was reused with different content".into())
        };
    }

    if reply.expected_plan_revision != plan.revision {
        return Err(format!(
            "Decision reply is stale: expected plan revision {}, current revision {}",
            reply.expected_plan_revision, plan.revision
        ));
    }
    let mut resolved = Vec::with_capacity(current.len());
    for (decision, action) in &current {
        if reply.expected_question_revision != decision.plan_revision {
            return Err(format!(
                "Decision reply is stale: expected question revision {}, current question revision {}",
                reply.expected_question_revision, decision.plan_revision
            ));
        }
        if decision.plan_revision != plan.revision {
            return Err(format!(
                "Decision question belongs to stale plan revision {} rather than current revision {}",
                decision.plan_revision, plan.revision
            ));
        }
        for (evidence_ref, issued_fingerprint) in &decision.evidence_fingerprints {
            let now = plan.decision_evidence_fingerprint(evidence_ref, cwd)?;
            if &now != issued_fingerprint {
                return Err(format!(
                    "Decision evidence is stale for {evidence_ref}; ask the question again"
                ));
            }
        }
        let event = match action {
            HostDecisionAction::Defer => "defer",
            HostDecisionAction::Cancel => "cancel",
            _ => "answer",
        };
        let next_state = match decision_transition(state_name(decision.state), event, true) {
            Some("AnsweredByUser") => DecisionState::AnsweredByUser,
            Some("Deferred") => DecisionState::Deferred,
            Some("Cancelled") => DecisionState::Cancelled,
            _ => {
                return Err(format!(
                    "Decision {} cannot transition from {:?}",
                    decision.id, decision.state
                ))
            }
        };
        let answer = answer_content(decision, action)
            .map_err(|error| format!("Decision {}: {error}", decision.id))?
            .map(|content| DecisionAnswer {
                source: DecisionAnswerSource::UserInteraction,
                choice_id: content.choice_id,
                choice_ids: content.choice_ids,
                custom_text: content.custom_text,
                answered_at_ms: reply.answered_at_ms,
                host_event_id: reply.host_event_id.clone(),
            });
        resolved.push((decision.id.clone(), next_state, answer));
    }

    let mut next = plan.clone();
    next.revision = plan
        .revision
        .checked_add(1)
        .ok_or("Plan revision exhausted")?;
    next.approved_revision =
        crate::living_plan::approval_after_decision(plan.approved_revision, true);
    for (id, state, answer) in resolved {
        let decision = next
            .structured_decisions
            .get_mut(&id)
            .expect("decision cloned from validated plan");
        decision.state = state;
        decision.plan_revision = next.revision;
        decision.answer = answer;
        next.changes
            .push(format!("decision {id} updated to {state:?}"));
        next.invalidate_dependent_step_decisions(&id);
    }
    Ok((next, DecisionApplyOutcome::Applied))
}

/// What an answer says, before provenance is attached.
#[derive(Debug, PartialEq, Eq)]
struct AnswerContent {
    choice_id: Option<String>,
    choice_ids: Vec<String>,
    custom_text: Option<String>,
}

impl AnswerContent {
    fn matches(&self, stored: &DecisionAnswer) -> bool {
        self.choice_id == stored.choice_id
            && self.choice_ids == stored.choice_ids
            && self.custom_text == stored.custom_text
    }
}

/// Validate one host action against its question and normalise it. `None`
/// for defer and cancel, which carry no answer.
fn answer_content(
    question: &DecisionQuestion,
    action: &HostDecisionAction,
) -> Result<Option<AnswerContent>, String> {
    let (choice_ids, custom_text) = match action {
        HostDecisionAction::Defer | HostDecisionAction::Cancel => return Ok(None),
        HostDecisionAction::AnswerChoice(choice_id) => (vec![choice_id.clone()], None),
        HostDecisionAction::AnswerCustom(text) => (Vec::new(), Some(text.clone())),
        HostDecisionAction::AnswerChoices {
            choice_ids,
            custom_text,
        } => {
            if !question.multi_select {
                return Err(
                    "A single-select decision takes one choice or one custom answer".into(),
                );
            }
            (choice_ids.clone(), custom_text.clone())
        }
    };
    if let Some(text) = &custom_text {
        if !question.allow_custom {
            return Err("This decision does not allow a custom answer".into());
        }
        bounded_text(text, "custom answer")?;
    }
    let mut seen = BTreeSet::new();
    for choice_id in &choice_ids {
        valid_id(choice_id, "choice id")?;
        if question.custom_only {
            return Err("Custom-only decision cannot accept a listed option".into());
        }
        if !question
            .options
            .iter()
            .any(|option| option.id == *choice_id)
        {
            return Err(format!("Unknown decision option: {choice_id}"));
        }
        if !seen.insert(choice_id.as_str()) {
            return Err(format!("Option {choice_id} was selected twice"));
        }
    }

    if !question.multi_select {
        // Exactly one of the two, by construction of the actions above.
        return Ok(Some(AnswerContent {
            choice_id: choice_ids.into_iter().next(),
            choice_ids: Vec::new(),
            custom_text,
        }));
    }
    let (min, max) = question.selection_bounds();
    let count = choice_ids.len() + usize::from(custom_text.is_some());
    if count < min {
        return Err(format!("Select at least {min} answer(s); got {count}"));
    }
    if count > max {
        return Err(format!("Select at most {max} answer(s); got {count}"));
    }
    // Stored in option order, so a replay in a different order still matches.
    let ordered = question
        .options
        .iter()
        .filter(|option| seen.contains(option.id.as_str()))
        .map(|option| option.id.clone())
        .collect();
    Ok(Some(AnswerContent {
        choice_id: None,
        choice_ids: ordered,
        custom_text,
    }))
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
    // Several options can each be worth checking on a multi-select question,
    // so the one-recommendation rule is a single-select rule.
    if !valid_question_shape(
        input.options.len(),
        if input.multi_select { 0 } else { recommended },
        !input.materiality.trim().is_empty(),
        !input.evidence_refs.is_empty(),
        input.custom_only,
    ) {
        return Err("Question must have 2-4 options (or a valid custom-only exception), at most one recommendation, materiality, and evidence".into());
    }
    if input.custom_only && !input.allow_custom {
        return Err("Custom-only questions must enable the custom-answer route".into());
    }
    if input.multi_select {
        if input.custom_only {
            return Err("A multi-select question needs options; custom_only cannot be combined with multi_select".into());
        }
        let (min, max) = selection_bounds(
            true,
            input.min_selections,
            input.max_selections,
            input.options.len(),
            input.allow_custom,
        );
        let capacity = input.options.len() + usize::from(input.allow_custom);
        if max == 0 || min > max || max > capacity {
            return Err(format!(
                "Selection bounds must satisfy 0 <= min_selections <= max_selections <= {capacity} with max_selections >= 1; got {min}..={max}"
            ));
        }
        if recommended > max {
            return Err(format!(
                "{recommended} recommended options exceed max_selections {max}"
            ));
        }
    } else if input.min_selections.is_some() || input.max_selections.is_some() {
        return Err(
            "min_selections and max_selections apply only to multi_select questions".into(),
        );
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
        multi_select: input.multi_select,
        min_selections: input.min_selections,
        max_selections: input.max_selections,
        plan_revision: plan.revision,
        state: DecisionState::Open,
        answer: None,
    })
}

/// Read the tool input: `{"questions": [...]}` with 1-4 questions, or the
/// original flat single-question object, which stays valid.
pub fn parse_question_batch(
    input: &serde_json::Value,
) -> Result<Vec<DecisionQuestionInput>, String> {
    let questions = if input.get("questions").is_some() {
        let batch: DecisionBatchInput = serde_json::from_value(input.clone())
            .map_err(|error| format!("Invalid structured question batch: {error}"))?;
        match batch.questions {
            QuestionList::Items(items) => items,
            QuestionList::Encoded(text) => serde_json::from_str(&text)
                .map_err(|error| format!("Invalid structured question batch: {error}"))?,
        }
    } else {
        vec![serde_json::from_value(input.clone())
            .map_err(|error| format!("Invalid structured question: {error}"))?]
    };
    if questions.is_empty() || questions.len() > MAX_BATCH_QUESTIONS {
        return Err(format!(
            "Ask 1-{MAX_BATCH_QUESTIONS} questions per call; got {}",
            questions.len()
        ));
    }
    Ok(questions)
}

/// Validate every question of one dialog against the same plan snapshot, so
/// they share a revision and are answered, or rejected, as one unit.
pub fn validate_question_batch(
    inputs: Vec<DecisionQuestionInput>,
    plan: &LivingPlan,
    cwd: &Path,
) -> Result<Vec<DecisionQuestion>, String> {
    let mut ids = BTreeSet::new();
    let mut questions = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !ids.insert(input.id.clone()) {
            return Err(format!("Duplicate question id: {}", input.id));
        }
        let id = input.id.clone();
        questions.push(
            validate_question_for_plan(input, plan, cwd)
                .map_err(|error| format!("Question {id}: {error}"))?,
        );
    }
    Ok(questions)
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

    #[test]
    fn the_tool_schema_offers_exactly_the_kinds_the_validator_accepts() {
        // Regression: the schema offered `approach`, `tradeoff`, ... which the
        // validator rejected, so a model following the schema always failed.
        let spec = crate::tools::tool_specs()
            .into_iter()
            .find(|tool| tool.name == "ask_user_question")
            .expect("ask_user_question spec");
        let offered = spec.parameters["properties"]["questions"]["items"]["properties"]["kind"]
            ["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let accepted = [
            DecisionKind::Architecture,
            DecisionKind::Scope,
            DecisionKind::Behavior,
            DecisionKind::Permissions,
            DecisionKind::Cost,
            DecisionKind::Persistence,
            DecisionKind::Irreversible,
        ]
        .map(|kind| {
            serde_json::to_value(kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        });
        assert_eq!(offered, accepted);
        for kind in &offered {
            assert!(
                serde_json::from_value::<DecisionKind>(json!(kind)).is_ok(),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_question_outside_a_plan_cites_workspace_files() {
        // Regression: with no living plan every evidence reference was
        // "Unknown plan evidence", so the question tool never worked outside
        // Plan Mode.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn a() {}
",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "TOKEN=x
",
        )
        .unwrap();
        let plan = LivingPlan::default();
        let ask = |refs: Vec<&str>| {
            validate_question_for_plan(
                DecisionQuestionInput {
                    kind: DecisionKind::Cost,
                    evidence_refs: refs.into_iter().map(String::from).collect(),
                    ..question()
                },
                &plan,
                dir.path(),
            )
        };
        let asked = ask(vec!["src/lib.rs"]).expect("a read workspace file is evidence");
        assert_eq!(asked.evidence_fingerprints.len(), 1);
        // Missing, sensitive or outside-workspace files are not evidence.
        for bad in ["src/missing.rs", ".env", "../outside.rs"] {
            let error = ask(vec![bad]).unwrap_err();
            assert!(error.contains(bad), "{bad}: {error}");
        }
        // A changed file makes the issued question stale.
        let issued = &asked.evidence_fingerprints["src/lib.rs"];
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn b() {}
",
        )
        .unwrap();
        let now = plan
            .decision_evidence_fingerprint("src/lib.rs", dir.path())
            .unwrap();
        assert_ne!(&now, issued);
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
            multi_select: false,
            min_selections: None,
            max_selections: None,
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
            choice_ids: Vec::new(),
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

    fn features() -> DecisionQuestionInput {
        DecisionQuestionInput {
            id: "features".into(),
            kind: DecisionKind::Scope,
            title: "Features".into(),
            question: "Which features should ship first?".into(),
            materiality: "Each feature adds a module and tests.".into(),
            evidence_refs: vec!["src.rs".into()],
            options: ["auth", "cache", "logs"]
                .into_iter()
                .map(|id| DecisionOptionInput {
                    id: id.into(),
                    label: id.to_uppercase(),
                    explanation: format!("Ship {id}"),
                    recommended: id != "cache",
                })
                .collect(),
            allow_custom: true,
            custom_only: false,
            multi_select: true,
            min_selections: Some(1),
            max_selections: Some(3),
        }
    }

    fn plan_with_batch() -> (tempfile::TempDir, LivingPlan) {
        let (dir, mut plan) = fixture();
        for q in validate_question_batch(vec![question(), features()], &plan, dir.path()).unwrap() {
            plan.structured_decisions.insert(q.id.clone(), q);
        }
        (dir, plan)
    }

    fn batch_reply(answers: Vec<(&str, HostDecisionAction)>) -> HostDecisionBatchReply {
        HostDecisionBatchReply {
            answers: answers
                .into_iter()
                .map(|(id, action)| (id.to_string(), action))
                .collect(),
            expected_plan_revision: 1,
            expected_question_revision: 1,
            host_event_id: "host-batch-1".into(),
            answered_at_ms: 7,
        }
    }

    fn choices(ids: &[&str], custom: Option<&str>) -> HostDecisionAction {
        HostDecisionAction::AnswerChoices {
            choice_ids: ids.iter().map(|id| id.to_string()).collect(),
            custom_text: custom.map(String::from),
        }
    }

    #[test]
    fn the_flat_legacy_input_and_the_questions_array_both_parse() {
        let flat = serde_json::to_value(question()).unwrap();
        assert_eq!(parse_question_batch(&flat).unwrap(), vec![question()]);

        let batch = json!({"questions": [question(), features()]});
        assert_eq!(
            parse_question_batch(&batch).unwrap(),
            vec![question(), features()]
        );
        // A model that stringifies the nested array is still understood.
        let encoded = json!({"questions": serde_json::to_string(&[question()]).unwrap()});
        assert_eq!(parse_question_batch(&encoded).unwrap(), vec![question()]);
    }

    #[test]
    fn a_batch_holds_one_to_four_questions_and_nothing_else() {
        assert!(parse_question_batch(&json!({"questions": []}))
            .unwrap_err()
            .contains("1-4"));
        let five: Vec<_> = (0..5)
            .map(|n| DecisionQuestionInput {
                id: format!("q{n}"),
                ..question()
            })
            .collect();
        assert!(parse_question_batch(&json!({ "questions": five }))
            .unwrap_err()
            .contains("got 5"));
        // Authority or legacy fields next to `questions` are refused.
        for extra in ["answer", "id", "state"] {
            let mut value = json!({"questions": [question()]});
            value[extra] = json!("forged");
            assert!(parse_question_batch(&value).is_err(), "{extra}");
        }
        // And inside each question too.
        let mut forged = serde_json::to_value(question()).unwrap();
        forged["answer"] = json!({"choice_id": "sqlite"});
        assert!(parse_question_batch(&json!({ "questions": [forged] })).is_err());
    }

    #[test]
    fn batch_validation_names_the_failing_question_and_rejects_duplicate_ids() {
        let (dir, plan) = fixture();
        let error =
            validate_question_batch(vec![question(), question()], &plan, dir.path()).unwrap_err();
        assert!(
            error.contains("Duplicate question id: cache-scope"),
            "{error}"
        );

        let mut broken = features();
        broken.options.truncate(1);
        let error =
            validate_question_batch(vec![question(), broken], &plan, dir.path()).unwrap_err();
        assert!(error.starts_with("Question features:"), "{error}");

        let questions =
            validate_question_batch(vec![question(), features()], &plan, dir.path()).unwrap();
        assert!(questions.iter().all(|q| q.plan_revision == plan.revision));
        assert!(questions[1].multi_select);
        assert_eq!(questions[1].selection_bounds(), (1, 3));
        assert_eq!(questions[0].selection_bounds(), (1, 1));
    }

    #[test]
    fn multi_select_shape_rules() {
        let (dir, plan) = fixture();
        let check =
            |input: DecisionQuestionInput| validate_question_for_plan(input, &plan, dir.path());

        // Several recommendations are fine on multi-select, not on single.
        assert!(check(features()).is_ok());
        let mut single = features();
        single.multi_select = false;
        single.min_selections = None;
        single.max_selections = None;
        assert!(check(single.clone()).is_err());

        // Bounds belong to multi-select only.
        single.options[0].recommended = false;
        single.options[2].recommended = false;
        assert!(check(single.clone()).is_ok());
        single.max_selections = Some(1);
        assert!(check(single).unwrap_err().contains("only to multi_select"));

        for (min, max) in [(Some(3), Some(2)), (None, Some(0)), (None, Some(5))] {
            let mut bad = features();
            bad.min_selections = min;
            bad.max_selections = max;
            assert!(
                check(bad).unwrap_err().contains("Selection bounds"),
                "{min:?} {max:?}"
            );
        }
        // Capacity counts the custom answer: 3 options + custom = 4.
        let mut four = features();
        four.max_selections = Some(4);
        assert!(check(four.clone()).is_ok());
        four.allow_custom = false;
        assert!(check(four).is_err());
        // Zero minimum means "none of these" is a valid answer.
        let mut optional = features();
        optional.min_selections = Some(0);
        assert!(check(optional).is_ok());
        // Recommending more options than may be selected is contradictory.
        let mut too_many = features();
        too_many.max_selections = Some(1);
        assert!(check(too_many).unwrap_err().contains("exceed"));
        // custom_only has no options to multi-select.
        let mut custom_multi = features();
        custom_multi.options.clear();
        custom_multi.custom_only = true;
        assert!(check(custom_multi).is_err());
    }

    #[test]
    fn one_submission_answers_every_question_in_one_revision() {
        let (dir, plan) = plan_with_batch();
        let reply = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerChoice("sqlite".into()),
            ),
            (
                "features",
                choices(&["logs", "auth"], Some("metrics export")),
            ),
        ]);
        let (next, outcome) = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap();
        assert_eq!(outcome, DecisionApplyOutcome::Applied);
        assert_eq!(next.revision, plan.revision + 1, "one bump for the batch");
        let single = &next.structured_decisions["cache-scope"];
        let multi = &next.structured_decisions["features"];
        for decision in [single, multi] {
            assert_eq!(decision.state, DecisionState::AnsweredByUser);
            assert_eq!(decision.plan_revision, next.revision);
            let answer = decision.answer.as_ref().unwrap();
            assert_eq!(answer.host_event_id, "host-batch-1");
            assert_eq!(answer.source, DecisionAnswerSource::UserInteraction);
        }
        assert_eq!(single.answer.as_ref().unwrap().selected_ids(), ["sqlite"]);
        let multi_answer = multi.answer.as_ref().unwrap();
        // Stored in option order, whatever order the host sent.
        assert_eq!(multi_answer.choice_ids, ["auth", "logs"]);
        assert_eq!(multi_answer.choice_id, None);
        assert_eq!(multi_answer.custom_text.as_deref(), Some("metrics export"));
        assert!(next
            .render()
            .contains("[selected: auth, logs; custom: metrics export]"));
    }

    #[test]
    fn an_invalid_answer_anywhere_rejects_the_whole_submission() {
        let (dir, plan) = plan_with_batch();
        let cases = [
            (choices(&[], None), "at least 1"),
            (
                choices(&["auth", "cache", "logs"], Some("more")),
                "at most 3",
            ),
            (choices(&["auth", "auth"], None), "selected twice"),
            (choices(&["redis"], None), "Unknown decision option"),
            (choices(&["auth"], Some("   ")), "custom answer"),
        ];
        for (bad, expected) in cases {
            let reply = batch_reply(vec![
                (
                    "cache-scope",
                    HostDecisionAction::AnswerChoice("sqlite".into()),
                ),
                ("features", bad),
            ]);
            let error = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap_err();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        // A single-select question cannot be given several choices.
        let reply = batch_reply(vec![
            ("cache-scope", choices(&["sqlite", "memory"], None)),
            ("features", choices(&["auth"], None)),
        ]);
        assert!(apply_host_decision_batch(&plan, &reply, dir.path())
            .unwrap_err()
            .contains("single-select"));
        // Legacy single actions still answer a multi-select question.
        let reply = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerCustom("Redis".into()),
            ),
            ("features", HostDecisionAction::AnswerChoice("cache".into())),
        ]);
        let (next, _) = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap();
        assert_eq!(
            next.structured_decisions["features"]
                .answer
                .as_ref()
                .unwrap()
                .choice_ids,
            ["cache"]
        );
    }

    #[test]
    fn a_replayed_submission_is_a_duplicate_only_when_identical() {
        let (dir, plan) = plan_with_batch();
        let reply = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerChoice("memory".into()),
            ),
            ("features", choices(&["logs", "auth"], None)),
        ]);
        let (answered, _) = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap();
        let reordered = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerChoice("memory".into()),
            ),
            ("features", choices(&["auth", "logs"], None)),
        ]);
        let (same, outcome) = apply_host_decision_batch(&answered, &reordered, dir.path()).unwrap();
        assert_eq!(outcome, DecisionApplyOutcome::Duplicate);
        assert_eq!(same, answered);

        let changed = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerChoice("memory".into()),
            ),
            ("features", choices(&["auth"], None)),
        ]);
        assert!(apply_host_decision_batch(&answered, &changed, dir.path())
            .unwrap_err()
            .contains("idempotency"));
    }

    #[test]
    fn stale_or_unknown_batch_replies_are_refused() {
        let (dir, plan) = plan_with_batch();
        let mut stale = batch_reply(vec![("features", choices(&["auth"], None))]);
        stale.expected_plan_revision = 0;
        assert!(apply_host_decision_batch(&plan, &stale, dir.path())
            .unwrap_err()
            .contains("stale"));
        let unknown = batch_reply(vec![("ghost", choices(&["auth"], None))]);
        assert!(apply_host_decision_batch(&plan, &unknown, dir.path())
            .unwrap_err()
            .contains("Unknown structured decision"));
        let empty = batch_reply(vec![]);
        assert!(apply_host_decision_batch(&plan, &empty, dir.path()).is_err());
        // Evidence edited while the dialog was open makes every answer stale.
        std::fs::write(dir.path().join("src.rs"), "fn edited() {}\n").unwrap();
        let reply = batch_reply(vec![("features", choices(&["auth"], None))]);
        assert!(apply_host_decision_batch(&plan, &reply, dir.path())
            .unwrap_err()
            .contains("stale"));
    }

    #[test]
    fn deferring_a_batch_defers_every_question() {
        let (dir, plan) = plan_with_batch();
        let reply = batch_reply(vec![
            ("cache-scope", HostDecisionAction::Defer),
            ("features", HostDecisionAction::Defer),
        ]);
        let (next, _) = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap();
        for id in ["cache-scope", "features"] {
            assert_eq!(next.structured_decisions[id].state, DecisionState::Deferred);
            assert!(next.structured_decisions[id].answer.is_none());
        }
    }

    #[test]
    fn sessions_saved_before_multi_select_load_and_new_answers_round_trip() {
        let (dir, plan) = plan_with_batch();
        let mut old = serde_json::to_value(&plan.structured_decisions["cache-scope"]).unwrap();
        for field in ["multi_select", "min_selections", "max_selections"] {
            old.as_object_mut().unwrap().remove(field);
        }
        let restored: DecisionQuestion = serde_json::from_value(old).unwrap();
        assert!(!restored.multi_select);
        assert_eq!(restored.selection_bounds(), (1, 1));
        // An old single-choice answer has no `choice_ids` key and still reads.
        let legacy_answer = json!({
            "source": "user_interaction", "choice_id": "sqlite",
            "answered_at_ms": 1, "host_event_id": "e"
        });
        let answer: DecisionAnswer = serde_json::from_value(legacy_answer).unwrap();
        assert_eq!(answer.selected_ids(), ["sqlite"]);

        let reply = batch_reply(vec![
            (
                "cache-scope",
                HostDecisionAction::AnswerChoice("sqlite".into()),
            ),
            ("features", choices(&["auth", "logs"], Some("audit"))),
        ]);
        let (next, _) = apply_host_decision_batch(&plan, &reply, dir.path()).unwrap();
        let encoded = serde_json::to_string(&next).unwrap();
        let decoded: LivingPlan = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, next);
        assert_eq!(
            decoded.structured_decisions["features"]
                .answer
                .as_ref()
                .unwrap()
                .choice_ids,
            ["auth", "logs"]
        );
    }
}
