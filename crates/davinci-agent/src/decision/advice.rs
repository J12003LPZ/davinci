//! Ready-only advice for one real user turn. No consumer waits for Jev.
//!
//! Task advice describes an immutable task/metadata snapshot. New tool outputs
//! do not change those inputs; a mutation, settings generation, or user turn
//! does. Completion advice additionally binds the exact public evidence revision.

use std::sync::Arc;

use serde_json::{json, Value};

use super::provider::DecisionError;
use super::request::DecisionRequest;
use super::response::{DecisionAnswer, DecisionResponse};
use super::{DecisionAdviceKey, ReadyDecision};
use crate::{effort, Agent, CompletionEvidence};
use davinci_protocol::ThinkingLevel;

#[derive(Debug, Clone, Copy, Default)]
pub struct AdviceFeatures {
    pub effort: bool,
    pub completion: bool,
    pub tool_families: bool,
}

impl AdviceFeatures {
    pub fn any(self) -> bool {
        self.effort || self.completion || self.tool_families
    }
}

#[derive(Clone)]
pub struct DecisionRequestFactory(
    pub Arc<dyn Fn(DecisionAdviceKey, Option<Value>) -> DecisionRequest + Send + Sync>,
);

impl std::fmt::Debug for DecisionRequestFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DecisionRequestFactory")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementJudgment {
    Supported,
    PossiblyMissing,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementAdvice {
    pub requirement_id: String,
    pub judgment: RequirementJudgment,
    pub confidence: f32,
    pub evidence_references: Vec<String>,
    pub key: DecisionAdviceKey,
}

/// Observations, not completion authority. No calibrated omission-reminder
/// policy is installed, so even confident support cannot waive a hard check.
#[derive(Debug, Clone)]
pub struct CompletionAdvice {
    pub key: DecisionAdviceKey,
    pub requirements: Vec<RequirementAdvice>,
    pub requirements_truncated: bool,
    pub deterministic_evidence: CompletionEvidence,
}

#[derive(Debug, Clone)]
pub(crate) struct TurnAdvice {
    pub task_key: DecisionAdviceKey,
    pub factory: DecisionRequestFactory,
    pub pending_completion: bool,
    pub last_completion_revision: Option<(u64, u64)>,
    pub effort_mutation_revision: Option<u64>,
    pub completion: Option<CompletionAdvice>,
}

pub fn request_carries_key(state: &Value, key: &DecisionAdviceKey) -> bool {
    let Some(value) = state.get("__davinci_decision_key") else {
        return false;
    };
    value.get("requestId").and_then(Value::as_str) == Some(key.request_id.as_str())
        && value.get("generation").and_then(Value::as_u64) == Some(key.generation)
        && value.get("evidenceRevision").and_then(Value::as_u64) == Some(key.evidence_revision)
        && value.get("mutationRevision").and_then(Value::as_u64) == Some(key.mutation_revision)
}

pub fn recommended_effort(response: &DecisionResponse) -> Option<ThinkingLevel> {
    let risk = matches!(response.answers.get("regression_risk"),
        Some(DecisionAnswer::Score { score, .. }) if *score >= 2.0);
    let broad = matches!(response.answers.get("verification_scope"),
        Some(DecisionAnswer::Choice { choice, .. }) if choice == "full");
    (risk || broad).then_some(effort::favor_high_effort(ThinkingLevel::Medium))
}

pub fn advised_tool_families(response: &DecisionResponse) -> Vec<String> {
    [
        ("browser_relevant", "browser"),
        ("git_history_relevant", "git"),
        ("package_intelligence_relevant", "package"),
        ("test_impact_relevant", "tests"),
        ("change_impact_relevant", "impact"),
        ("verification_planner_relevant", "verification"),
    ]
    .into_iter()
    .filter_map(|(question, family)| {
        matches!(response.answers.get(question), Some(DecisionAnswer::Noul { value }) if *value >= 0.85)
            .then(|| family.to_owned())
    })
    .collect()
}

impl Agent {
    pub fn decision_advice_features(&self) -> AdviceFeatures {
        AdviceFeatures {
            effort: self.decision_effort_advice_enabled,
            completion: self.decision_completion_advice_enabled,
            tool_families: self.decision_tool_family_advice_enabled,
        }
    }

    pub fn set_decision_completion_advice_enabled(&mut self, enabled: bool) {
        if self.decision_completion_advice_enabled != enabled {
            self.clear_turn_decision();
        }
        self.decision_completion_advice_enabled = enabled;
    }

    pub fn set_decision_tool_family_advice_enabled(&mut self, enabled: bool) {
        if self.decision_tool_family_advice_enabled != enabled {
            self.clear_turn_decision();
        }
        self.decision_tool_family_advice_enabled = enabled;
    }

    pub fn completion_advice(&self) -> Option<&CompletionAdvice> {
        self.decision_turn_advice.as_ref()?.completion.as_ref()
    }

    pub(crate) fn clear_turn_decision(&mut self) {
        self.decision_effort_advice = None;
        self.decision_advice_key = None;
        self.decision_turn_advice = None;
    }

    /// Called after the real user message is installed. Preparation, including
    /// any metadata projection, runs on the bounded shadow worker.
    pub fn prepare_decision_advice(
        &mut self,
        key: DecisionAdviceKey,
        factory: DecisionRequestFactory,
    ) -> Result<(), DecisionError> {
        self.clear_turn_decision();
        if key.request_id.trim().is_empty() {
            return Err(DecisionError::InvalidRequest(
                "advice requires a current turn id".into(),
            ));
        }
        let Some(runtime) = self.decision_runtime() else {
            return Err(DecisionError::Disabled);
        };
        if !runtime.is_enabled() || !self.decision_advice_features().any() {
            return Err(DecisionError::Disabled);
        }
        // Discard a previous turn's completed result. An in-flight previous
        // worker can still finish; consumption also checks the full key.
        let _ = runtime.take_ready_shadow();
        self.decision_turn_advice = Some(TurnAdvice {
            task_key: key.clone(),
            factory: factory.clone(),
            pending_completion: false,
            last_completion_revision: None,
            effort_mutation_revision: None,
            completion: None,
        });
        if !self.decision_effort_advice_enabled && !self.decision_tool_family_advice_enabled {
            // Completion questions require post-tool evidence, never an empty
            // submit-time snapshot. The loop will enqueue them after tools.
            return Ok(());
        }
        let worker_key = key.clone();
        runtime.enqueue_shadow_with_result(move || {
            let request = (factory.0)(worker_key.clone(), None);
            (request, worker_key)
        })?;
        self.decision_advice_key = Some(key);
        Ok(())
    }

    /// Request construction boundary, shared by interactive and print modes.
    /// `after_first_request` is false for request 1, even if Jev is already ready.
    pub fn poll_decision_advice(&mut self, after_first_request: bool) {
        if self.abort_requested() {
            self.clear_turn_decision();
            return;
        }
        if !after_first_request {
            return;
        }
        let Some(runtime) = self.decision_runtime() else {
            return;
        };
        if !runtime.is_enabled() || !self.decision_advice_features().any() {
            self.clear_turn_decision();
            let _ = runtime.take_ready_shadow();
            return;
        }
        if self.decision_turn_advice.is_none() {
            if runtime.take_ready_shadow().is_some() {
                runtime.telemetry().record_stale_advice();
            }
            return;
        }
        let mutation_revision = self.mutation_verification_state().mutation_generation;
        if self
            .decision_turn_advice
            .as_ref()
            .unwrap()
            .effort_mutation_revision
            != Some(mutation_revision)
        {
            self.decision_effort_advice = None;
        }
        let Some(ready) = runtime.take_ready_shadow() else {
            return;
        };
        let turn = self.decision_turn_advice.as_ref().unwrap();
        let valid = runtime.is_enabled()
            && ready.key.generation == runtime.generation()
            && self.decision_advice_key.as_ref() == Some(&ready.key)
            && ready.key.request_id == turn.task_key.request_id
            && ready.request.request_id == ready.key.request_id
            && ready.key.mutation_revision == mutation_revision
            && request_carries_key(&ready.request.state, &ready.key)
            && if turn.pending_completion {
                ready.key.evidence_revision == self.messages.len() as u64
            } else {
                ready.key.evidence_revision == turn.task_key.evidence_revision
            };
        if !valid {
            runtime.telemetry().record_stale_advice();
            return;
        }
        runtime.telemetry().record_ready_advice();
        let completion = turn.pending_completion;
        self.decision_advice_key = None;
        if completion {
            self.record_completion_advice(ready);
        } else {
            if self.decision_effort_advice_enabled {
                if let Some(level) = recommended_effort(&ready.response) {
                    self.decision_effort_advice = Some(level);
                    self.decision_turn_advice
                        .as_mut()
                        .unwrap()
                        .effort_mutation_revision = Some(mutation_revision);
                }
            }
            if self.decision_tool_family_advice_enabled {
                let families = advised_tool_families(&ready.response);
                let previously_visible = self.visible_tool_names();
                for tool in self.activate_tool_families(&families) {
                    if !previously_visible.contains(&tool) {
                        runtime.telemetry().record_addition();
                    }
                }
            }
        }
    }

    pub(crate) fn record_decision_request_effort(&self) {
        let deterministic = effort::request_level(
            self.effort_policy,
            self.thinking_level,
            self.effort_signals(),
        );
        if self.request_thinking_level() != deterministic {
            if let Some(runtime) = self.decision_runtime() {
                runtime.telemetry().record_effort_advice_applied();
            }
        }
    }

    /// Snapshot only already-held public evidence; no repository reads or scans.
    /// Each revision is attempted once and busy admission remains a dropped sample.
    pub(crate) fn enqueue_completion_advice(&mut self) {
        if !self.decision_completion_advice_enabled {
            return;
        }
        let Some(runtime) = self.decision_runtime() else {
            return;
        };
        if !runtime.is_enabled() {
            return;
        }
        let evidence = self.mutation_verification_state();
        let revision = (self.messages.len() as u64, evidence.mutation_generation);
        let Some(turn) = self.decision_turn_advice.as_ref() else {
            return;
        };
        if turn.last_completion_revision == Some(revision) {
            return;
        }
        let key = DecisionAdviceKey {
            request_id: turn.task_key.request_id.clone(),
            generation: runtime.generation(),
            evidence_revision: revision.0,
            mutation_revision: revision.1,
        };
        let factory = turn.factory.clone();
        let status = self.completion_evidence();
        // Serialization and redaction are performed by the bounded worker.
        let cwd = self.cwd.clone();
        let worker_key = key.clone();
        let result = runtime.enqueue_shadow_with_result(move || {
            let relative = |path: &std::path::Path| path.strip_prefix(&cwd).unwrap_or(path).to_string_lossy().chars().take(240).collect::<String>();
            let public = json!({
                "status": status,
                "mutationGeneration": evidence.mutation_generation,
                "verifiedGeneration": evidence.verified_generation,
                "unknownMutationPaths": evidence.unknown_mutation_paths,
                "changedPaths": evidence.mutation_paths.iter().take(16).map(|path| relative(path)).collect::<Vec<_>>(),
                "pathsTruncated": evidence.mutation_paths.len() > 16,
                "latestCheck": evidence.latest_evidence.as_ref().map(|check| json!({
                    "reference": format!("verification:{}", check.generation),
                    "generation": check.generation,
                    "succeeded": check.succeeded,
                    "coverage": check.coverage,
                    "command": check.command.chars().take(512).collect::<String>(),
                    "targets": check.verification_targets.iter().take(16).map(|target| target.chars().take(240).collect::<String>()).collect::<Vec<_>>(),
                    "targetsTruncated": check.verification_targets.len() > 16,
                })),
            });
            let request = (factory.0)(worker_key.clone(), Some(public));
            (request, worker_key)
        });
        if let Some(turn) = self.decision_turn_advice.as_mut() {
            turn.last_completion_revision = Some(revision);
            turn.completion = None;
            if result.is_ok() {
                turn.pending_completion = true;
                self.decision_advice_key = Some(key);
            }
        }
    }

    fn record_completion_advice(&mut self, ready: ReadyDecision) {
        if !self.decision_completion_advice_enabled {
            return;
        }
        let requirements = ready
            .request
            .state
            .pointer("/requirements/entries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?;
                let DecisionAnswer::Choice {
                    choice, confidence, ..
                } = ready.response.answers.get(&format!("requirement_{id}"))?
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
                    requirement_id: id.to_owned(),
                    judgment,
                    confidence: *confidence,
                    // A general check is not proof of a specific requirement.
                    evidence_references: Vec::new(),
                    key: ready.key.clone(),
                })
            })
            .collect();
        let observation = CompletionAdvice {
            key: ready.key,
            requirements,
            requirements_truncated: ready
                .request
                .state
                .pointer("/requirements/truncated")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            deterministic_evidence: self.completion_evidence(),
        };
        if let Some(runtime) = self.decision_runtime() {
            runtime.telemetry().record_completion_advice_observed();
        }
        if let Some(turn) = self.decision_turn_advice.as_mut() {
            turn.completion = Some(observation);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::super::provider::DecisionProvider;
    use super::super::request::DecisionQuestion;
    use super::super::{DecisionRuntime, ReadyDecision};
    use super::*;

    struct NoNetwork;
    impl DecisionProvider for NoNetwork {
        fn name(&self) -> &'static str {
            "fixture"
        }
        fn model(&self) -> &'static str {
            "fixture"
        }
        fn evaluate(
            &self,
            _: &DecisionRequest,
            _: Duration,
        ) -> Result<DecisionResponse, DecisionError> {
            panic!("unit fixture must not dispatch a provider")
        }
    }

    fn pending_agent() -> (Agent, Arc<DecisionRuntime>, DecisionAdviceKey) {
        let runtime = Arc::new(DecisionRuntime::new(Arc::new(NoNetwork)));
        runtime.enable();
        let mut agent = Agent::new("fixture");
        agent.set_decision_runtime(runtime.clone());
        agent.set_decision_effort_advice_enabled(true);
        agent.set_decision_completion_advice_enabled(true);
        agent.thinking_level = ThinkingLevel::Medium;
        agent.effort_policy = effort::EffortPolicy::Adaptive;
        agent.prompt("Must preserve the API");
        let key = DecisionAdviceKey {
            request_id: "turn".into(),
            generation: runtime.generation(),
            evidence_revision: agent.messages.len() as u64,
            mutation_revision: 0,
        };
        agent.decision_turn_advice = Some(TurnAdvice {
            task_key: key.clone(),
            factory: DecisionRequestFactory(Arc::new(|_, _| panic!("no prepare"))),
            pending_completion: false,
            last_completion_revision: None,
            effort_mutation_revision: None,
            completion: None,
        });
        agent.set_decision_advice_key(Some(key.clone()));
        (agent, runtime, key)
    }

    fn effort_ready(key: &DecisionAdviceKey) -> ReadyDecision {
        ReadyDecision {
            key: key.clone(),
            request: DecisionRequest::new(
                key.request_id.clone(),
                super::super::risk::DecisionRisk::Planning,
                json!({
                    "__davinci_decision_key": {"requestId":key.request_id,"generation":key.generation,"evidenceRevision":key.evidence_revision,"mutationRevision":key.mutation_revision}
                }),
                "fixture",
                [(
                    "regression_risk".into(),
                    DecisionQuestion::score("risk", ["low", "moderate", "high"]),
                )],
            ),
            response: DecisionResponse {
                answers: BTreeMap::from([(
                    "regression_risk".into(),
                    DecisionAnswer::Score {
                        score: 2.0,
                        levels: 3,
                        probabilities: BTreeMap::from([
                            ("0".into(), 0.0),
                            ("1".into(), 0.0),
                            ("2".into(), 1.0),
                        ]),
                        confidence: 1.0,
                    },
                )]),
                model: None,
                input_tokens: None,
                output_tokens: None,
            },
        }
    }

    #[test]
    fn mutation_after_admission_rejects_task_advice() {
        let (mut agent, runtime, key) = pending_agent();
        *runtime.ready_shadow.lock().unwrap() = Some(effort_ready(&key));
        agent.record_successful_mutation_paths(vec!["parser.rs".into()]);
        agent.poll_decision_advice(true);
        assert_eq!(runtime.telemetry().snapshot().stale_advice, 1);
        assert_eq!(runtime.telemetry().snapshot().ready_advice, 0);
        assert!(agent.decision_effort_advice.is_none());
    }

    #[test]
    fn replacing_runtime_with_the_same_generation_invalidates_accepted_advice() {
        let (mut agent, runtime, key) = pending_agent();
        *runtime.ready_shadow.lock().unwrap() = Some(effort_ready(&key));
        agent.poll_decision_advice(true);
        assert_eq!(agent.request_thinking_level(), ThinkingLevel::High);
        let replacement = Arc::new(DecisionRuntime::new(Arc::new(NoNetwork)));
        replacement.enable();
        assert_eq!(runtime.generation(), replacement.generation());
        agent.set_decision_runtime(replacement);
        assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
        assert!(agent.decision_turn_advice.is_none());
    }

    #[test]
    fn ready_advice_cannot_change_fixed_or_off_effort() {
        for (policy, base) in [
            (effort::EffortPolicy::Fixed, ThinkingLevel::Low),
            (effort::EffortPolicy::Adaptive, ThinkingLevel::Off),
        ] {
            let (mut agent, runtime, key) = pending_agent();
            agent.effort_policy = policy;
            agent.thinking_level = base;
            *runtime.ready_shadow.lock().unwrap() = Some(effort_ready(&key));
            agent.poll_decision_advice(true);
            assert_eq!(agent.request_thinking_level(), base);
            agent.record_decision_request_effort();
            assert_eq!(runtime.telemetry().snapshot().effort_advice_applied, 0);
        }
    }

    #[test]
    fn completion_requires_exact_current_evidence_revision() {
        let (mut agent, runtime, key) = pending_agent();
        agent
            .decision_turn_advice
            .as_mut()
            .unwrap()
            .pending_completion = true;
        *runtime.ready_shadow.lock().unwrap() = Some(effort_ready(&key));
        agent.messages.push(davinci_ai::ChatMessage::tool_result(
            "check",
            "bash",
            "new evidence",
            false,
        ));
        agent.poll_decision_advice(true);
        assert!(agent.completion_advice().is_none());
        assert_eq!(runtime.telemetry().snapshot().stale_advice, 1);
    }

    #[test]
    fn family_advice_expands_authorized_schemas_only_after_request_one() {
        use crate::runtime::{
            AgentId, CapabilitySource, RunId, RuntimeBus, RuntimeCapability, RuntimeHandle,
        };
        for enabled in [false, true] {
            let (mut agent, runtime, key) = pending_agent();
            agent.decision_tool_family_advice_enabled = enabled;
            agent.tool_surface = crate::ToolSurface::Lean;
            agent.turn_context_placement_override =
                Some(crate::turn_context::TurnContextPlacement::Appended);
            let host = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
            for name in ["browser_allowed", "browser_not_authorized"] {
                host.capability_registry.register(
                    RuntimeCapability::new(
                        name,
                        CapabilitySource::Mcp,
                        crate::ToolClass::Read,
                        true,
                        &json!({"type":"object","properties":{}}),
                        None,
                    )
                    .with_family("browser"),
                );
            }
            agent.set_runtime(host);
            agent.apply_extension_tools(&["browser_allowed".into()]);
            agent.freeze_tools_for_cache();
            let core = agent.visible_tool_names();
            let mut ready = effort_ready(&key);
            ready.request.questions = BTreeMap::from([(
                "browser_relevant".into(),
                DecisionQuestion::noul("browser relevance"),
            )]);
            ready.response.answers = BTreeMap::from([(
                "browser_relevant".into(),
                DecisionAnswer::Noul { value: 0.9 },
            )]);
            *runtime.ready_shadow.lock().unwrap() = Some(ready);
            agent.poll_decision_advice(false);
            assert!(!agent.is_tool_visible("browser_allowed"));
            agent.poll_decision_advice(true);
            assert_eq!(agent.is_tool_visible("browser_allowed"), enabled);
            assert!(!agent.is_tool_visible("browser_not_authorized"));
            assert!(core.is_subset(&agent.visible_tool_names()));
            let added = agent.visible_tool_names().difference(&core).count() as u64;
            assert_eq!(runtime.telemetry().snapshot().additions, added);
            assert_eq!(added > 0, enabled);
        }
    }

    #[test]
    fn family_advice_is_admitted_with_its_actual_schemas_before_dispatch() {
        use crate::runtime::{
            context_vm::ContextVmMode, AgentId, CapabilitySource, RunId, RuntimeBus,
            RuntimeCapability, RuntimeHandle,
        };

        for schema_bytes in [0, 64_000] {
            let (mut agent, runtime, key) = pending_agent();
            agent.decision_completion_advice_enabled = false;
            agent.decision_tool_family_advice_enabled = true;
            agent.auto_compaction = false;
            agent.auto_verify = false;
            agent.tool_surface = crate::ToolSurface::Lean;
            agent.turn_context_placement_override =
                Some(crate::turn_context::TurnContextPlacement::Appended);
            agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("public.txt"), "public evidence").unwrap();
            agent.cwd = root.path().into();
            let host = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
            host.capability_registry.register(
                RuntimeCapability::new(
                    "browser_large",
                    CapabilitySource::Mcp,
                    crate::ToolClass::Read,
                    true,
                    &json!({"type":"object", "description":"x".repeat(schema_bytes)}),
                    None,
                )
                .with_family("browser"),
            );
            agent.set_runtime(host);
            agent.apply_extension_tools(&["browser_large".into()]);
            agent.freeze_tools_for_cache();
            agent.set_context_vm_mode(ContextVmMode::Active);
            agent.set_provider_output_limit(Some(1024));
            agent.set_provider_context_overhead_estimator(|agent| {
                serde_json::to_vec(&agent.provider_tool_specs()).unwrap().len() as u64 + 128
            });
            agent.context_window = agent.provider_context_budget().reserved() + 16_000;
            let initial_tools = agent.visible_tool_names();
            let initial_budget = agent.provider_context_budget().tools;
            let initial_effort = agent.request_thinking_level();

            // Publish after the response-boundary poll, during the first tool
            // round. Request 2 must account for these ready schema additions.
            let mut ready = effort_ready(&key);
            ready.request.questions = BTreeMap::from([(
                "browser_relevant".into(),
                DecisionQuestion::noul("browser relevance"),
            )]);
            ready.response.answers = BTreeMap::from([(
                "browser_relevant".into(),
                DecisionAnswer::Noul { value: 0.9 },
            )]);
            let ready_runtime = Arc::clone(&runtime);
            agent.post_tool = Some(crate::PostToolHook(Arc::new(move |_, _, _, _, result| {
                *ready_runtime.ready_shadow.lock().unwrap() = Some(ready.clone());
                result
            })));

            let mut requests = 0;
            let result = agent.run_loop(|current| {
                requests += 1;
                if requests == 1 {
                    assert_eq!(current.visible_tool_names(), initial_tools);
                    assert_eq!(current.provider_context_budget().tools, initial_budget);
                    assert_eq!(current.request_thinking_level(), initial_effort);
                } else {
                    assert_eq!(schema_bytes, 0, "oversized schemas reached the provider");
                    assert!(current.is_tool_visible("browser_large"));
                    assert!(current.provider_context_budget().tools > initial_budget);
                }
                // The actual provider projection must be admitted, never the
                // legacy reader fallback for an image rejected after the gate.
                assert!(current.prepared_context_image().is_ok());
                Ok(serde_json::from_value::<davinci_ai::AssistantMessage>(json!({
                    "id":format!("response-{requests}"), "role":"assistant", "model":"fixture",
                    "content": if requests == 1 {
                        json!([{"type":"toolCall", "id":"read", "name":"read", "arguments":{"path":"public.txt"}}])
                    } else {
                        json!([{"type":"text", "text":"done"}])
                    },
                    "stopReason": if requests == 1 {"toolUse"} else {"stop"}
                })).unwrap())
            });
            assert!(agent.is_tool_visible("browser_large"));
            assert!(agent.provider_context_budget().tools > initial_budget);
            if schema_bytes == 0 {
                result.unwrap();
                assert_eq!(requests, 2);
            } else {
                assert!(result.unwrap_err().contains("compilation token budget"));
                assert_eq!(requests, 1);
                assert_eq!(agent.run_stats().model_turns, 1);
            }
        }
    }

    #[test]
    fn labeled_completion_observations_never_replace_hard_evidence() {
        for label in ["failed", "stale", "partial", "unknown", "complete"] {
            let (mut agent, runtime, mut key) = pending_agent();
            agent.record_successful_mutation_paths(vec!["parser.rs".into(), "tests.rs".into()]);
            match label {
                "failed" => agent.record_verification_result(false),
                "complete" => agent.record_verification_result(true),
                "stale" => {
                    agent.record_verification_result(true);
                    agent.record_successful_mutation_paths(vec!["later.rs".into()]);
                }
                "partial" => {
                    let mut state = agent.mutation_verification.lock().unwrap();
                    state.covered_paths = vec!["parser.rs".into()];
                    state.latest_evidence = Some(crate::VerificationEvidence {
                        generation: state.mutation_generation,
                        command: "public check".into(),
                        succeeded: true,
                        mutation_paths: vec!["parser.rs".into()],
                        verification_targets: vec!["parser.rs".into()],
                        coverage: crate::VerificationCoverage::Targeted,
                    });
                }
                _ => {}
            }
            let before = agent.mutation_verification_state();
            let status = agent.completion_evidence();
            key.mutation_revision = before.mutation_generation;
            agent
                .decision_turn_advice
                .as_mut()
                .unwrap()
                .pending_completion = true;
            agent.set_decision_advice_key(Some(key.clone()));
            let mut ready = effort_ready(&key);
            ready.request.state["requirements"] = json!({"entries":[{"id":"api", "wording":"Must preserve the API"}], "truncated":false});
            ready.response.answers = BTreeMap::from([(
                "requirement_api".into(),
                DecisionAnswer::Choice {
                    choice: "supported".into(),
                    probabilities: BTreeMap::from([
                        ("supported".into(), 1.0),
                        ("possibly_missing".into(), 0.0),
                        ("uncertain".into(), 0.0),
                    ]),
                    confidence: 1.0,
                },
            )]);
            *runtime.ready_shadow.lock().unwrap() = Some(ready);
            agent.poll_decision_advice(true);
            assert_eq!(agent.mutation_verification_state(), before, "{label}");
            assert_eq!(agent.completion_evidence(), status, "{label}");
            assert_eq!(
                agent.completion_advice().unwrap().requirements[0].judgment,
                RequirementJudgment::Supported
            );
            assert!(agent.completion_advice().unwrap().requirements[0]
                .evidence_references
                .is_empty());
            assert_eq!(runtime.telemetry().snapshot().completion_advice_observed, 1);
            agent.poll_decision_advice(true);
            assert_eq!(
                runtime.telemetry().snapshot().completion_advice_observed,
                1,
                "no duplicates"
            );
        }
    }
}
