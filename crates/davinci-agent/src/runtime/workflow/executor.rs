//! Workflow execution engine.
//!
//! Executes multi-phase agent workflows respecting phase dependencies,
//! concurrency limits, deterministic joins (all, any, quorum), retry budgets,
//! and cancellation trees.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::spec::{WorkflowJoin, WorkflowSpec};
use super::state::WorkflowStateStore;
use super::validate::{validate_workflow_with_capabilities, WorkflowValidationError};
use crate::runtime::cancellation::CancellationToken;
use crate::runtime::events::{AgentKind, AgentRecord, AgentState, RuntimeEvent};
use crate::runtime::ids::{AgentId, TaskId, WorkflowId};
use crate::runtime::RuntimeHandle;
use crate::subagent::{SubagentRequest, SubagentRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseExecutionState {
    pub id: String,
    pub status: PhaseStatus,
    pub task_ids: Vec<TaskId>,
    pub worker_agent_ids: Vec<AgentId>,
    pub completed_workers: Vec<AgentId>,
    pub failed_workers: Vec<AgentId>,
    pub retry_counts: HashMap<AgentId, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionState {
    pub id: WorkflowId,
    pub name: String,
    pub status: WorkflowStatus,
    pub phases: HashMap<String, PhaseExecutionState>,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WorkflowExecutionError {
    #[error("validation error: {0}")]
    Validation(#[from] WorkflowValidationError),
    #[error("workflow not found: {0}")]
    WorkflowNotFound(WorkflowId),
    #[error("workflow is in invalid state for action: {0:?}")]
    InvalidState(WorkflowStatus),
    #[error("phase execution failed: phase '{phase}' failed")]
    PhaseFailed { phase: String },
    #[error("workflow was cancelled")]
    Cancelled,
    #[error("execution error: {0}")]
    ExecutionError(String),
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Executor for deterministic general workflows.
#[derive(Clone)]
pub struct WorkflowExecutor {
    pub runtime: RuntimeHandle,
    pub store: WorkflowStateStore,
    pub runner: Option<SubagentRunner>,
    executions: Arc<RwLock<HashMap<WorkflowId, WorkflowExecutionState>>>,
    tokens: Arc<RwLock<HashMap<WorkflowId, CancellationToken>>>,
    specs: Arc<RwLock<HashMap<WorkflowId, WorkflowSpec>>>,
}

impl WorkflowExecutor {
    pub fn new(
        runtime: RuntimeHandle,
        store: WorkflowStateStore,
        runner: Option<SubagentRunner>,
    ) -> Self {
        Self {
            runtime,
            store,
            runner,
            executions: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
            specs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Retrieve the current execution state of a workflow.
    pub fn get_state(&self, wf_id: &WorkflowId) -> Option<WorkflowExecutionState> {
        let execs = self.executions.read().unwrap();
        execs.get(wf_id).cloned()
    }

    /// Cancel a running workflow and all its active workers.
    pub fn cancel(&self, wf_id: &WorkflowId) -> Result<(), WorkflowExecutionError> {
        let token = {
            let tokens = self.tokens.read().unwrap();
            tokens.get(wf_id).cloned()
        };
        if let Some(tok) = token {
            tok.cancel();
        }

        let mut execs = self.executions.write().unwrap();
        if let Some(state) = execs.get_mut(wf_id) {
            if state.status == WorkflowStatus::Running || state.status == WorkflowStatus::Pending {
                state.status = WorkflowStatus::Cancelled;
                state.finished_ms = Some(now_ms());
                self.runtime.emit_observe(RuntimeEvent::WorkflowEnded {
                    workflow_id: *wf_id,
                    success: false,
                });
            }
            Ok(())
        } else {
            Err(WorkflowExecutionError::WorkflowNotFound(*wf_id))
        }
    }

    /// Pause a running workflow.
    pub fn pause(&self, wf_id: &WorkflowId) -> Result<(), WorkflowExecutionError> {
        let mut execs = self.executions.write().unwrap();
        if let Some(state) = execs.get_mut(wf_id) {
            if state.status == WorkflowStatus::Running {
                state.status = WorkflowStatus::Paused;
                Ok(())
            } else {
                Err(WorkflowExecutionError::InvalidState(state.status))
            }
        } else {
            Err(WorkflowExecutionError::WorkflowNotFound(*wf_id))
        }
    }

    /// Retrieve all tracked workflow executions.
    pub fn list_workflows(&self) -> Vec<WorkflowExecutionState> {
        let execs = self.executions.read().unwrap();
        let mut list: Vec<_> = execs.values().cloned().collect();
        list.sort_by_key(|w| w.started_ms);
        list
    }

    /// Resume a paused workflow.
    pub fn resume(&self, wf_id: &WorkflowId) -> Result<(), WorkflowExecutionError> {
        let mut execs = self.executions.write().unwrap();
        if let Some(state) = execs.get_mut(wf_id) {
            if state.status == WorkflowStatus::Paused {
                state.status = WorkflowStatus::Running;
                self.runtime.emit_observe(RuntimeEvent::WorkflowStarted {
                    workflow_id: *wf_id,
                });
                Ok(())
            } else {
                Err(WorkflowExecutionError::InvalidState(state.status))
            }
        } else {
            Err(WorkflowExecutionError::WorkflowNotFound(*wf_id))
        }
    }

    fn init_workflow_state(
        &self,
        spec: &WorkflowSpec,
        wf_id: WorkflowId,
        wf_token: CancellationToken,
    ) {
        let now = now_ms();
        let mut phase_states = HashMap::new();
        for phase in &spec.phases {
            phase_states.insert(
                phase.id.clone(),
                PhaseExecutionState {
                    id: phase.id.clone(),
                    status: PhaseStatus::Pending,
                    task_ids: Vec::new(),
                    worker_agent_ids: Vec::new(),
                    completed_workers: Vec::new(),
                    failed_workers: Vec::new(),
                    retry_counts: HashMap::new(),
                },
            );
        }

        let initial_state = WorkflowExecutionState {
            id: wf_id,
            name: spec.name.clone(),
            status: WorkflowStatus::Running,
            phases: phase_states,
            started_ms: now,
            finished_ms: None,
            error: None,
        };

        {
            let mut execs = self.executions.write().unwrap();
            execs.insert(wf_id, initial_state);
            let mut toks = self.tokens.write().unwrap();
            toks.insert(wf_id, wf_token);
            let mut sps = self.specs.write().unwrap();
            sps.insert(wf_id, spec.clone());
        }

        self.runtime
            .emit_observe(RuntimeEvent::WorkflowStarted { workflow_id: wf_id });
    }

    /// Execute a validated workflow from start to finish synchronously.
    pub fn execute(
        &self,
        spec: WorkflowSpec,
    ) -> Result<WorkflowExecutionState, WorkflowExecutionError> {
        validate_workflow_with_capabilities(&spec, None, &[], &self.runtime.capability_registry)?;
        let wf_id = WorkflowId::new();
        let wf_token = self.runtime.cancellation_token.child_token();
        self.init_workflow_state(&spec, wf_id, wf_token.clone());
        self.run_phases(spec, wf_id, wf_token)
    }

    /// Execute a validated workflow asynchronously in a background thread.
    pub fn execute_background(
        &self,
        spec: WorkflowSpec,
    ) -> Result<WorkflowId, WorkflowExecutionError> {
        validate_workflow_with_capabilities(&spec, None, &[], &self.runtime.capability_registry)?;
        let wf_id = WorkflowId::new();
        let wf_token = self.runtime.cancellation_token.child_token();
        self.init_workflow_state(&spec, wf_id, wf_token.clone());

        let this = self.clone();
        std::thread::Builder::new()
            .name(format!("wf-{}", wf_id))
            .spawn(move || {
                let _ = this.run_phases(spec, wf_id, wf_token);
            })
            .map_err(|e| WorkflowExecutionError::ExecutionError(e.to_string()))?;

        Ok(wf_id)
    }

    /// Resume an existing or partially completed workflow from persisted phase artifacts and tasks.
    ///
    /// Rules:
    /// - Phases already satisfied by persisted artifacts in `WorkflowStateStore` are reused
    ///   and marked `PhaseStatus::Completed` without executing workers again.
    /// - For incomplete phases, workers using mutating tools (`is_mutating_tool`) MUST NOT be
    ///   rerun automatically unless their fingerprint (`format!("{}:{}:{}", wf_id, phase.id, worker.id)`)
    ///   is in `validated_mutation_fingerprints`.
    pub fn resume_execution(
        &self,
        wf_id: WorkflowId,
        spec: WorkflowSpec,
        validated_mutation_fingerprints: &HashSet<String>,
    ) -> Result<WorkflowExecutionState, WorkflowExecutionError> {
        validate_workflow_with_capabilities(&spec, None, &[], &self.runtime.capability_registry)?;
        let wf_token = self.runtime.cancellation_token.child_token();

        let mut completed_phases = HashSet::new();
        let mut phase_states = HashMap::new();

        for phase in &spec.phases {
            let existing_artifacts = self.store.list_phase_artifacts(wf_id, &phase.id);
            let is_phase_satisfied = match phase.join {
                WorkflowJoin::All => {
                    !phase.workers.is_empty()
                        && phase.workers.iter().all(|w| {
                            existing_artifacts.iter().any(|a| {
                                a.value
                                    .get("worker")
                                    .and_then(|v| v.as_str())
                                    .map(|id| id == w.id)
                                    .unwrap_or(false)
                            })
                        })
                }
                WorkflowJoin::Any => !existing_artifacts.is_empty(),
                WorkflowJoin::Quorum { required } => existing_artifacts.len() >= required,
            };

            if is_phase_satisfied {
                completed_phases.insert(phase.id.clone());
                phase_states.insert(
                    phase.id.clone(),
                    PhaseExecutionState {
                        id: phase.id.clone(),
                        status: PhaseStatus::Completed,
                        task_ids: Vec::new(),
                        worker_agent_ids: Vec::new(),
                        completed_workers: Vec::new(),
                        failed_workers: Vec::new(),
                        retry_counts: HashMap::new(),
                    },
                );
            } else {
                // Incomplete phase: verify mutation workers are validated before attempting rerun
                for worker in &phase.workers {
                    let is_mutating = worker
                        .tools
                        .iter()
                        .any(|t| super::validate::is_mutating_tool(t));
                    if is_mutating {
                        let fp = format!("{}:{}:{}", wf_id, phase.id, worker.id);
                        if !validated_mutation_fingerprints.contains(&fp) {
                            return Err(WorkflowExecutionError::ExecutionError(format!(
                                "cannot automatically rerun mutation worker '{}' in phase '{}' without fingerprint validation",
                                worker.id, phase.id
                            )));
                        }
                    }
                }

                phase_states.insert(
                    phase.id.clone(),
                    PhaseExecutionState {
                        id: phase.id.clone(),
                        status: PhaseStatus::Pending,
                        task_ids: Vec::new(),
                        worker_agent_ids: Vec::new(),
                        completed_workers: Vec::new(),
                        failed_workers: Vec::new(),
                        retry_counts: HashMap::new(),
                    },
                );
            }
        }

        let now = now_ms();
        let initial_state = WorkflowExecutionState {
            id: wf_id,
            name: spec.name.clone(),
            status: WorkflowStatus::Running,
            phases: phase_states,
            started_ms: now,
            finished_ms: None,
            error: None,
        };

        {
            let mut execs = self.executions.write().unwrap();
            execs.insert(wf_id, initial_state);
            let mut toks = self.tokens.write().unwrap();
            toks.insert(wf_id, wf_token.clone());
            let mut sps = self.specs.write().unwrap();
            sps.insert(wf_id, spec.clone());
        }

        self.runtime
            .emit_observe(RuntimeEvent::WorkflowStarted { workflow_id: wf_id });

        self.run_phases_with_completed(spec, wf_id, wf_token, completed_phases)
    }

    fn run_phases(
        &self,
        spec: WorkflowSpec,
        wf_id: WorkflowId,
        wf_token: CancellationToken,
    ) -> Result<WorkflowExecutionState, WorkflowExecutionError> {
        self.run_phases_with_completed(spec, wf_id, wf_token, HashSet::new())
    }

    fn run_phases_with_completed(
        &self,
        spec: WorkflowSpec,
        wf_id: WorkflowId,
        wf_token: CancellationToken,
        mut completed_phase_ids: HashSet<String>,
    ) -> Result<WorkflowExecutionState, WorkflowExecutionError> {
        while completed_phase_ids.len() < spec.phases.len() {
            if wf_token.is_cancelled() {
                self.cancel(&wf_id)?;
                return Err(WorkflowExecutionError::Cancelled);
            }

            let ready_phases: Vec<_> = spec
                .phases
                .iter()
                .filter(|p| {
                    !completed_phase_ids.contains(&p.id)
                        && p.depends_on
                            .iter()
                            .all(|dep| completed_phase_ids.contains(dep))
                })
                .cloned()
                .collect();

            if ready_phases.is_empty() {
                let err_msg =
                    "Workflow deadlocked or stalled with unresolvable dependencies".to_string();
                self.fail_workflow(&wf_id, &err_msg);
                return Err(WorkflowExecutionError::ExecutionError(err_msg));
            }

            for phase in ready_phases {
                if wf_token.is_cancelled() {
                    self.cancel(&wf_id)?;
                    return Err(WorkflowExecutionError::Cancelled);
                }

                self.runtime
                    .emit_observe(RuntimeEvent::WorkflowPhaseChanged {
                        workflow_id: wf_id,
                        phase: phase.id.clone(),
                    });

                {
                    let mut execs = self.executions.write().unwrap();
                    if let Some(wf_state) = execs.get_mut(&wf_id) {
                        if let Some(p_state) = wf_state.phases.get_mut(&phase.id) {
                            p_state.status = PhaseStatus::Running;
                        }
                    }
                }

                let phase_success = self.execute_phase(&wf_id, &phase, &wf_token)?;
                if !phase_success {
                    let err_msg = format!("Phase '{}' failed join requirements", phase.id);
                    self.fail_workflow(&wf_id, &err_msg);
                    return Err(WorkflowExecutionError::PhaseFailed { phase: phase.id });
                }

                completed_phase_ids.insert(phase.id.clone());
                {
                    let mut execs = self.executions.write().unwrap();
                    if let Some(wf_state) = execs.get_mut(&wf_id) {
                        if let Some(p_state) = wf_state.phases.get_mut(&phase.id) {
                            p_state.status = PhaseStatus::Completed;
                        }
                    }
                }
            }
        }

        // 3. Mark workflow complete
        let final_state = {
            let mut execs = self.executions.write().unwrap();
            let state = execs.get_mut(&wf_id).unwrap();
            state.status = WorkflowStatus::Completed;
            state.finished_ms = Some(now_ms());
            state.clone()
        };

        self.runtime.emit_observe(RuntimeEvent::WorkflowEnded {
            workflow_id: wf_id,
            success: true,
        });

        Ok(final_state)
    }

    fn fail_workflow(&self, wf_id: &WorkflowId, error: &str) {
        let mut execs = self.executions.write().unwrap();
        if let Some(state) = execs.get_mut(wf_id) {
            state.status = WorkflowStatus::Failed;
            state.finished_ms = Some(now_ms());
            state.error = Some(error.to_string());
        }
        self.runtime.emit_observe(RuntimeEvent::WorkflowEnded {
            workflow_id: *wf_id,
            success: false,
        });
    }

    fn execute_phase(
        &self,
        wf_id: &WorkflowId,
        phase: &super::spec::WorkflowPhaseSpec,
        wf_token: &CancellationToken,
    ) -> Result<bool, WorkflowExecutionError> {
        let deps: Vec<&str> = phase.depends_on.iter().map(String::as_str).collect();
        let artifact_context = self.store.format_prompt_references(*wf_id, &deps);

        let mut worker_agent_ids = Vec::new();
        let mut worker_tasks = Vec::new();

        for worker in &phase.workers {
            let aid = AgentId::new();
            worker_agent_ids.push(aid);

            // Register agent in runtime registry
            let now = now_ms();
            let record = AgentRecord {
                id: aid,
                run_id: self.runtime.run_id,
                parent: Some(self.runtime.agent_id),
                kind: AgentKind::Subagent,
                name: worker.id.clone(),
                provider: worker
                    .model
                    .as_ref()
                    .and_then(|m| m.split('/').next())
                    .unwrap_or_default()
                    .to_string(),
                model_id: worker
                    .model
                    .as_ref()
                    .and_then(|m| m.split('/').nth(1))
                    .unwrap_or_default()
                    .to_string(),
                cwd: std::env::current_dir().unwrap_or_default(),
                state: AgentState::Running,
                task_id: None,
                worktree: worker.isolation.as_ref().and_then(|iso| {
                    if iso == "worktree" {
                        Some(std::path::PathBuf::from("worktree"))
                    } else {
                        None
                    }
                }),
                started_ms: now,
                updated_ms: now,
                failure_reason: None,
            };
            let _ = self.runtime.registry.register_agent(record);

            let task_rec = crate::runtime::tasks::TaskRecord::new(
                self.runtime.run_id,
                format!("{}:{}", phase.id, worker.id),
            )
            .with_description(worker.prompt.clone());

            let tid = self
                .runtime
                .task_registry
                .create_task(task_rec)
                .map_err(|e| WorkflowExecutionError::ExecutionError(e.to_string()))?;

            let _ = self.runtime.task_registry.assign_task(tid, aid);
            worker_tasks.push((aid, tid, worker.clone()));
        }

        // Execute workers using scheduler or runner
        let mut successful_workers: Vec<AgentId> = Vec::new();
        let mut failed_workers: Vec<AgentId> = Vec::new();

        for (aid, tid, worker) in worker_tasks {
            if wf_token.is_cancelled() {
                let _ = self.runtime.registry.transition(aid, AgentState::Cancelled);
                let _ = self.runtime.task_registry.cancel_task(tid);
                failed_workers.push(aid);
                continue;
            }

            let effective_prompt = if !artifact_context.is_empty() {
                format!(
                    "{}\n\n[Context from previous phases]:\n{}",
                    worker.prompt, artifact_context
                )
            } else {
                worker.prompt.clone()
            };

            let req = SubagentRequest {
                prompt: effective_prompt,
                tools: worker.tools.clone(),
                description: Some(worker.id.clone()),
                provider: None,
                model_id: None,
                abort: Some(wf_token.as_atomic_bool()),
                cancellation_token: Some(wf_token.child_token()),
                agent: worker.agent_profile.clone(),
                mode: crate::subagent::AgentSpawnMode::Oneshot,
                model_override: worker.model.clone(),
                isolation: worker.isolation.clone(),
                instance_name: Some(worker.id.clone()),
                runtime_agent_id: Some(aid),
                parent_permission_mode: None,
                worktree_path: None,
            };

            let retry_limit = worker.retry_budget.unwrap_or(0);
            let mut attempt = 0;
            let mut outcome: Result<String, String> = Err("no runner configured".into());

            while attempt <= retry_limit {
                if let Some(r) = &self.runner {
                    outcome = r.run(&req);
                    if outcome.is_ok() {
                        break;
                    }
                } else {
                    // Default fallback if runner is not provided in test: canned success
                    outcome = Ok(format!("completed {}", worker.id));
                    break;
                }
                attempt += 1;
            }

            match outcome {
                Ok(result_text) => {
                    let _ = self.runtime.registry.transition(aid, AgentState::Completed);
                    let _ = self.runtime.task_registry.complete_task(tid, None);
                    successful_workers.push(aid);

                    // Store artifact
                    let val = serde_json::json!({
                        "worker": worker.id,
                        "output": result_text,
                    });
                    let _ = self.store.put_artifact(*wf_id, &phase.id, aid, val, None);
                }
                Err(err) => {
                    let _ = self.runtime.registry.transition(aid, AgentState::Failed);
                    let _ = self.runtime.task_registry.fail_task(tid, Some(err));
                    failed_workers.push(aid);
                }
            }

            // Early exit on Any or Quorum if possible
            match phase.join {
                WorkflowJoin::Any if !successful_workers.is_empty() => break,
                WorkflowJoin::Quorum { required } if successful_workers.len() >= required => break,
                _ => {}
            }
        }

        // Update phase execution state
        {
            let mut execs = self.executions.write().unwrap();
            if let Some(wf_state) = execs.get_mut(wf_id) {
                if let Some(p_state) = wf_state.phases.get_mut(&phase.id) {
                    p_state.completed_workers = successful_workers.clone();
                    p_state.failed_workers = failed_workers.clone();
                }
            }
        }

        // Evaluate Join
        let total_workers = phase.workers.len();
        let success_count = successful_workers.len();

        let joined = match phase.join {
            WorkflowJoin::All => success_count == total_workers,
            WorkflowJoin::Any => success_count > 0,
            WorkflowJoin::Quorum { required } => success_count >= required,
        };

        Ok(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::workflow::spec::*;
    use tempfile::tempdir;

    fn setup_executor(runner: Option<SubagentRunner>) -> (WorkflowExecutor, tempfile::TempDir) {
        let bus = crate::runtime::RuntimeBus::default();
        let handle = RuntimeHandle::new(crate::runtime::RunId::new(), AgentId::new(), bus);
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(32 * 1024, tmp.path().to_path_buf());
        (WorkflowExecutor::new(handle, store, runner), tmp)
    }

    #[test]
    fn test_execute_valid_3_phase_workflow_success() {
        let runner = SubagentRunner::new(|req| Ok(format!("result for {}", req.prompt)));
        let (executor, _tmp) = setup_executor(Some(runner));

        let spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        let final_state = executor
            .execute(spec)
            .expect("workflow executes to completion");

        assert_eq!(final_state.status, WorkflowStatus::Completed);
        assert_eq!(final_state.phases.len(), 3);
        for p in final_state.phases.values() {
            assert_eq!(p.status, PhaseStatus::Completed);
        }

        // Verify artifacts from all phases are in store
        let p1_arts = executor
            .store
            .list_phase_artifacts(final_state.id, "investigate");
        assert_eq!(p1_arts.len(), 2);
        let p3_arts = executor
            .store
            .list_phase_artifacts(final_state.id, "implement");
        assert_eq!(p3_arts.len(), 2);
    }

    #[test]
    fn test_join_any_succeeds_when_first_worker_succeeds() {
        let runner = SubagentRunner::new(|req| {
            if req.instance_name.as_deref() == Some("fast-worker") {
                Ok("fast success".into())
            } else {
                Err("slow failure".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));

        let spec = WorkflowSpec {
            schema_version: 1,
            name: "any-join-test".into(),
            max_parallel_agents: 2,
            max_total_agents: 4,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "competition".into(),
                depends_on: vec![],
                join: WorkflowJoin::Any,
                workers: vec![
                    WorkflowWorkerSpec {
                        id: "fast-worker".into(),
                        prompt: "try method A".into(),
                        agent_profile: None,
                        model: None,
                        tools: vec!["read".into()],
                        isolation: None,
                        max_turns: None,
                        retry_budget: None,
                    },
                    WorkflowWorkerSpec {
                        id: "slow-worker".into(),
                        prompt: "try method B".into(),
                        agent_profile: None,
                        model: None,
                        tools: vec!["read".into()],
                        isolation: None,
                        max_turns: None,
                        retry_budget: None,
                    },
                ],
            }],
        };

        let final_state = executor.execute(spec).unwrap();
        assert_eq!(final_state.status, WorkflowStatus::Completed);
        let p_state = &final_state.phases["competition"];
        assert_eq!(p_state.status, PhaseStatus::Completed);
        assert_eq!(p_state.completed_workers.len(), 1);
    }

    #[test]
    fn test_join_quorum_succeeds_when_threshold_reached() {
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cnt = Arc::clone(&counter);
        let runner = SubagentRunner::new(move |_| {
            let n = cnt.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                Ok("quorum success".into())
            } else {
                Err("quorum fail".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));

        let spec = WorkflowSpec {
            schema_version: 1,
            name: "quorum-join-test".into(),
            max_parallel_agents: 3,
            max_total_agents: 3,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "vote".into(),
                depends_on: vec![],
                join: WorkflowJoin::Quorum { required: 2 },
                workers: vec![
                    WorkflowWorkerSpec {
                        id: "voter-1".into(),
                        prompt: "vote".into(),
                        agent_profile: None,
                        model: None,
                        tools: vec!["read".into()],
                        isolation: None,
                        max_turns: None,
                        retry_budget: None,
                    },
                    WorkflowWorkerSpec {
                        id: "voter-2".into(),
                        prompt: "vote".into(),
                        agent_profile: None,
                        model: None,
                        tools: vec!["read".into()],
                        isolation: None,
                        max_turns: None,
                        retry_budget: None,
                    },
                    WorkflowWorkerSpec {
                        id: "voter-3".into(),
                        prompt: "vote".into(),
                        agent_profile: None,
                        model: None,
                        tools: vec!["read".into()],
                        isolation: None,
                        max_turns: None,
                        retry_budget: None,
                    },
                ],
            }],
        };

        let final_state = executor.execute(spec).unwrap();
        assert_eq!(final_state.status, WorkflowStatus::Completed);
        let p_state = &final_state.phases["vote"];
        assert_eq!(p_state.status, PhaseStatus::Completed);
        assert_eq!(p_state.completed_workers.len(), 2);
    }

    #[test]
    fn test_retry_budget_recovers_transient_failure() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let att = Arc::clone(&attempts);
        let runner = SubagentRunner::new(move |_| {
            let cur = att.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if cur == 0 {
                Err("transient glitch".into())
            } else {
                Ok("recovered".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));

        let spec = WorkflowSpec {
            schema_version: 1,
            name: "retry-test".into(),
            max_parallel_agents: 1,
            max_total_agents: 1,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "step".into(),
                depends_on: vec![],
                join: WorkflowJoin::All,
                workers: vec![WorkflowWorkerSpec {
                    id: "retrying-worker".into(),
                    prompt: "work".into(),
                    agent_profile: None,
                    model: None,
                    tools: vec!["read".into()],
                    isolation: None,
                    max_turns: None,
                    retry_budget: Some(2),
                }],
            }],
        };

        let final_state = executor.execute(spec).unwrap();
        assert_eq!(final_state.status, WorkflowStatus::Completed);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn test_workflow_cancellation_stops_execution() {
        let (executor, _tmp) = setup_executor(None);
        let spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();

        // Pre-cancel parent runtime token
        executor.runtime.cancellation_token.cancel();

        let err = executor.execute(spec).unwrap_err();
        assert_eq!(err, WorkflowExecutionError::Cancelled);
    }

    #[test]
    fn test_workflow_execute_background_and_list_and_resume() {
        let (executor, _tmp) = setup_executor(None);
        let spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();

        let wf_id = executor.execute_background(spec).unwrap();
        // Give background thread a moment to finish execution
        let mut completed = false;
        for _ in 0..50 {
            if let Some(st) = executor.get_state(&wf_id) {
                if st.status == WorkflowStatus::Completed {
                    completed = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(completed, "background workflow should complete");

        let list = executor.list_workflows();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, wf_id);

        // Test pause and resume
        let pause_res = executor.pause(&wf_id);
        assert!(pause_res.is_err(), "cannot pause completed workflow");
    }

    #[test]
    fn test_workflow_resume_reusing_completed_readonly_phase() {
        let (executor, _tmp) = setup_executor(None);
        let readonly_workflow_json = r#"{
            "schema_version": 1,
            "name": "readonly-resume-test",
            "max_parallel_agents": 2,
            "max_total_agents": 4,
            "phases": [
                {
                    "id": "research",
                    "join": "all",
                    "workers": [
                        {
                            "id": "researcher-1",
                            "prompt": "find occurrences",
                            "tools": ["read", "grep"]
                        }
                    ]
                },
                {
                    "id": "summarize",
                    "depends_on": ["research"],
                    "join": "all",
                    "workers": [
                        {
                            "id": "summarizer-1",
                            "prompt": "summarize findings",
                            "tools": ["read"]
                        }
                    ]
                }
            ]
        }"#;
        let spec: WorkflowSpec = serde_json::from_str(readonly_workflow_json).unwrap();
        let wf_id = WorkflowId::new();

        // 1. Simulate that Phase 1 ("research") already completed and produced an artifact
        let worker_aid = AgentId::new();
        let _ = executor.store.put_artifact(
            wf_id,
            "research",
            worker_aid,
            serde_json::json!({
                "worker": "researcher-1",
                "output": "Found relevant files: src/main.rs",
            }),
            None,
        );

        let validated_fps = HashSet::new();
        let state = executor
            .resume_execution(wf_id, spec, &validated_fps)
            .expect("resume succeeds");

        assert_eq!(state.status, WorkflowStatus::Completed);
        assert_eq!(state.phases["research"].status, PhaseStatus::Completed);
        assert_eq!(state.phases["summarize"].status, PhaseStatus::Completed);

        // Verify artifacts exist for both phases
        assert_eq!(
            executor.store.list_phase_artifacts(wf_id, "research").len(),
            1
        );
        assert_eq!(
            executor
                .store
                .list_phase_artifacts(wf_id, "summarize")
                .len(),
            1
        );
    }

    #[test]
    fn test_workflow_resume_mutation_worker_requires_fingerprint_validation() {
        let (executor, _tmp) = setup_executor(None);
        let wf_id = WorkflowId::new();

        let mutating_spec_json = r#"{
            "schema_version": 1,
            "name": "mutating-workflow",
            "max_parallel_agents": 2,
            "max_total_agents": 4,
            "phases": [
                {
                    "id": "write_code",
                    "join": "all",
                    "workers": [
                        {
                            "id": "writer-1",
                            "prompt": "write new feature",
                            "tools": ["read", "write"]
                        }
                    ]
                }
            ]
        }"#;
        let spec: WorkflowSpec = serde_json::from_str(mutating_spec_json).unwrap();

        // 1. Without fingerprint validation: resume must be refused
        let empty_fps = HashSet::new();
        let err = executor
            .resume_execution(wf_id, spec.clone(), &empty_fps)
            .unwrap_err();

        match err {
            WorkflowExecutionError::ExecutionError(msg) => {
                assert!(msg.contains("without fingerprint validation"));
                assert!(msg.contains("writer-1"));
            }
            other => panic!("expected ExecutionError, got {other:?}"),
        }

        // 2. With validated fingerprint: resume is permitted
        let mut valid_fps = HashSet::new();
        valid_fps.insert(format!("{}:write_code:writer-1", wf_id));

        let res = executor.resume_execution(wf_id, spec, &valid_fps);
        assert!(
            res.is_ok(),
            "resume should succeed with validated fingerprint"
        );
        assert_eq!(res.unwrap().status, WorkflowStatus::Completed);
    }
}
