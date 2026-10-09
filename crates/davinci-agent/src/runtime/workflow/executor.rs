//! Workflow execution engine.
//!
//! Executes multi-phase agent workflows respecting phase dependencies,
//! concurrency limits, deterministic joins (all, any, quorum), retry budgets,
//! and cancellation trees.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::spec::{
    WorkflowJoin, WorkflowLaunch, WorkflowPhaseSpec, WorkflowSpec, WorkflowWorkerSpec,
};
use super::state::WorkflowStateStore;
use super::validate::{validate_workflow_with_capabilities, WorkflowValidationError};
use crate::runtime::cancellation::CancellationToken;
use crate::runtime::events::{AgentKind, AgentRecord, AgentState, RuntimeEvent};
use crate::runtime::ids::{AgentId, TaskId, WorkflowId};
use crate::runtime::{ChildExecutionContext, ChildExecutionKind, RuntimeHandle};
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
    /// `worker-id: error` for each failed worker, so a failed phase can say why.
    #[serde(default)]
    pub failure_reasons: Vec<String>,
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
    #[error(
        "phase execution failed: phase '{phase}' failed{}",
        reason_suffix(reasons)
    )]
    PhaseFailed { phase: String, reasons: Vec<String> },
    #[error("workflow was cancelled")]
    Cancelled,
    #[error("execution error: {0}")]
    ExecutionError(String),
}

fn reason_suffix(reasons: &[String]) -> String {
    if reasons.is_empty() {
        String::new()
    } else {
        format!(" ({})", reasons.join("; "))
    }
}

/// Distinct workers of `phase` that left an artifact. Artifacts name their
/// worker by the spec id in `value.worker`; one from a worker the phase does
/// not declare, or a second one from the same worker, counts for nothing.
fn workers_with_artifacts(
    phase: &WorkflowPhaseSpec,
    artifacts: &[super::state::WorkflowArtifact],
) -> usize {
    phase
        .workers
        .iter()
        .filter(|w| {
            artifacts
                .iter()
                .any(|a| a.value.get("worker").and_then(|v| v.as_str()) == Some(w.id.as_str()))
        })
        .count()
}

/// Whether persisted artifacts already satisfy a phase's join.
fn phase_satisfied_by(
    phase: &WorkflowPhaseSpec,
    artifacts: &[super::state::WorkflowArtifact],
) -> bool {
    let done = workers_with_artifacts(phase, artifacts);
    match phase.join {
        WorkflowJoin::All => !phase.workers.is_empty() && done == phase.workers.len(),
        WorkflowJoin::Any => done >= 1,
        WorkflowJoin::Quorum { required } => done >= required,
    }
}

const RETRY_BACKOFF_BASE_MS: u64 = 250;
const RETRY_BACKOFF_MAX_MS: u64 = 5_000;

/// Pause before retry number `retry` (1-based): exponential from 250 ms,
/// capped at 5 s, plus up to a quarter of that as jitter so workers that
/// failed together do not retry together.
fn retry_backoff(retry: usize) -> std::time::Duration {
    let exp = RETRY_BACKOFF_BASE_MS.saturating_mul(1u64 << retry.saturating_sub(1).min(16));
    let base = exp.min(RETRY_BACKOFF_MAX_MS);
    let jitter_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    std::time::Duration::from_millis(base + jitter_seed % (base / 4 + 1))
}

/// Sleep for `total`, waking early once any token is cancelled.
fn sleep_unless_cancelled(total: std::time::Duration, tokens: &[&CancellationToken]) {
    let end = std::time::Instant::now() + total;
    while std::time::Instant::now() < end && !tokens.iter().any(|t| t.is_cancelled()) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The prompt for a retry: the original task plus what the last attempt
/// died of, so the worker can change course instead of repeating itself.
fn retry_prompt(base: &str, retry: usize, last_error: &str) -> String {
    format!("{base}\n\n[Retry {retry}: the previous attempt failed with: {last_error}]")
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
    launches: Arc<RwLock<HashMap<WorkflowId, WorkflowLaunch>>>,
    /// Each worker's live activity (tool uses, tokens, recent calls) for
    /// the workflows view.
    progress: Arc<RwLock<HashMap<AgentId, crate::subagent_progress::ProgressReporter>>>,
    /// Each running worker's own token, so one agent can be stopped.
    agent_tokens: Arc<RwLock<HashMap<AgentId, CancellationToken>>>,
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
            launches: Arc::new(RwLock::new(HashMap::new())),
            progress: Arc::new(RwLock::new(HashMap::new())),
            agent_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Stop one running worker. It counts as failed, so a relaunch reruns it
    /// (Claude Code: stopping a single agent counts as failing). Returns
    /// false when that worker is not running.
    pub fn cancel_agent(&self, agent: &AgentId) -> bool {
        let token = self
            .agent_tokens
            .read()
            .ok()
            .and_then(|tokens| tokens.get(agent).cloned());
        match token {
            Some(token) => {
                self.runtime
                    .registry
                    .set_failure_reason(*agent, "stopped from the workflows view");
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// A run's phase ids in spec order (its state keeps them in a map).
    pub fn phase_order(&self, wf_id: &WorkflowId) -> Vec<String> {
        self.specs
            .read()
            .ok()
            .and_then(|specs| {
                specs
                    .get(wf_id)
                    .map(|spec| spec.phases.iter().map(|phase| phase.id.clone()).collect())
            })
            .unwrap_or_default()
    }

    /// While a run is paused no new worker starts; running ones continue.
    fn wait_while_paused(&self, wf_id: &WorkflowId, token: &CancellationToken) {
        while !token.is_cancelled()
            && self
                .get_state(wf_id)
                .is_some_and(|state| state.status == WorkflowStatus::Paused)
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// A worker's activity so far, for the workflows view.
    pub fn agent_progress(
        &self,
        agent: &AgentId,
    ) -> Option<crate::subagent_progress::SubagentProgress> {
        self.progress
            .read()
            .ok()
            .and_then(|map| map.get(agent).map(|reporter| reporter.snapshot()))
    }

    /// Retrieve the current execution state of a workflow.
    pub fn get_state(&self, wf_id: &WorkflowId) -> Option<WorkflowExecutionState> {
        let execs = self
            .executions
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        execs.get(wf_id).cloned()
    }

    /// Cancel a running workflow and all its active workers.
    pub fn cancel(&self, wf_id: &WorkflowId) -> Result<(), WorkflowExecutionError> {
        let token = {
            let tokens = self.tokens.read().unwrap_or_else(PoisonError::into_inner);
            tokens.get(wf_id).cloned()
        };
        if let Some(tok) = token {
            tok.cancel();
        }

        let mut execs = self
            .executions
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(state) = execs.get_mut(wf_id) {
            if !matches!(
                state.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
            ) {
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
        let mut execs = self
            .executions
            .write()
            .unwrap_or_else(PoisonError::into_inner);
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
        let execs = self
            .executions
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let mut list: Vec<_> = execs.values().cloned().collect();
        list.sort_by_key(|w| w.started_ms);
        list
    }

    /// Resume a paused workflow.
    pub fn resume(&self, wf_id: &WorkflowId) -> Result<(), WorkflowExecutionError> {
        let mut execs = self
            .executions
            .write()
            .unwrap_or_else(PoisonError::into_inner);
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
                    failure_reasons: Vec::new(),
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
            let mut execs = self
                .executions
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            execs.insert(wf_id, initial_state);
            let mut toks = self.tokens.write().unwrap_or_else(PoisonError::into_inner);
            toks.insert(wf_id, wf_token);
            let mut sps = self.specs.write().unwrap_or_else(PoisonError::into_inner);
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
        self.execute_with(spec, WorkflowLaunch::default())
    }

    pub fn execute_with(
        &self,
        spec: WorkflowSpec,
        launch: WorkflowLaunch,
    ) -> Result<WorkflowExecutionState, WorkflowExecutionError> {
        validate_workflow_with_capabilities(
            &spec,
            launch.parent_permission_mode,
            &[],
            &self.runtime.capability_registry,
        )?;
        if spec.max_cost_usd.is_some() {
            return Err(WorkflowExecutionError::ExecutionError(
                "max_cost_usd is not enforced yet; remove it from the spec".into(),
            ));
        }
        let wf_id = WorkflowId::new();
        // A synchronous run belongs to the turn that called it: Esc stops it.
        let wf_token = launch
            .turn_token
            .as_ref()
            .unwrap_or(&self.runtime.cancellation_token)
            .child_token();
        self.launches
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(wf_id, launch.clone());
        self.init_workflow_state(&spec, wf_id, wf_token.clone());
        self.run_phases(spec, wf_id, wf_token)
    }

    /// Execute a validated workflow asynchronously in a background thread.
    pub fn execute_background(
        &self,
        spec: WorkflowSpec,
    ) -> Result<WorkflowId, WorkflowExecutionError> {
        self.execute_background_with(spec, WorkflowLaunch::default())
    }

    pub fn execute_background_with(
        &self,
        spec: WorkflowSpec,
        launch: WorkflowLaunch,
    ) -> Result<WorkflowId, WorkflowExecutionError> {
        validate_workflow_with_capabilities(
            &spec,
            launch.parent_permission_mode,
            &[],
            &self.runtime.capability_registry,
        )?;
        if spec.max_cost_usd.is_some() {
            return Err(WorkflowExecutionError::ExecutionError(
                "max_cost_usd is not enforced yet; remove it from the spec".into(),
            ));
        }
        let wf_id = WorkflowId::new();
        // A background run outlives the turn that started it, like a
        // background agent: it descends from the session's team token, so
        // Esc on the lead leaves it running while `/workflow cancel` and a
        // session switch still stop it.
        let wf_token = self.runtime.team.session_token().child_token();
        self.launches
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(wf_id, launch.clone());
        self.init_workflow_state(&spec, wf_id, wf_token.clone());

        let this = self.clone();
        std::thread::Builder::new()
            .name(format!("wf-{wf_id}"))
            .spawn(move || {
                let name = spec.name.clone();
                let outcome = this.run_phases(spec, wf_id, wf_token);
                if launch.report_to_lead {
                    if let Err(problem) = this.report_completion(wf_id, &name, &outcome) {
                        eprintln!(
                            "[davinci-runtime] workflow {wf_id} completion notice: {problem}"
                        );
                    }
                }
            })
            .map_err(|e| self.abort_unlaunched(&wf_id, &e))?;

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
            let is_phase_satisfied = phase_satisfied_by(phase, &existing_artifacts);

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
                        failure_reasons: Vec::new(),
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
                        failure_reasons: Vec::new(),
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
            let mut execs = self
                .executions
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            execs.insert(wf_id, initial_state);
            let mut toks = self.tokens.write().unwrap_or_else(PoisonError::into_inner);
            toks.insert(wf_id, wf_token.clone());
            let mut sps = self.specs.write().unwrap_or_else(PoisonError::into_inner);
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
        let started = std::time::Instant::now();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct Finish(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Finish {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _finish = Finish(finished.clone());
        if let Some(ms) = spec.deadline_ms {
            let token = wf_token.clone();
            std::thread::Builder::new()
                .name(format!("wf-deadline-{wf_id}"))
                .spawn(move || {
                    while !finished.load(std::sync::atomic::Ordering::SeqCst)
                        && !token.is_cancelled()
                    {
                        if started.elapsed() >= std::time::Duration::from_millis(ms) {
                            token.cancel();
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                })
                .map_err(|e| WorkflowExecutionError::ExecutionError(e.to_string()))?;
        }
        while completed_phase_ids.len() < spec.phases.len() {
            if wf_token.is_cancelled() {
                return Err(self.cancelled_error(&wf_id, &spec, started));
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
                self.wait_while_paused(&wf_id, &wf_token);
                if wf_token.is_cancelled() {
                    return Err(self.cancelled_error(&wf_id, &spec, started));
                }

                self.runtime
                    .emit_observe(RuntimeEvent::WorkflowPhaseChanged {
                        workflow_id: wf_id,
                        phase: phase.id.clone(),
                    });

                {
                    let mut execs = self
                        .executions
                        .write()
                        .unwrap_or_else(PoisonError::into_inner);
                    if let Some(wf_state) = execs.get_mut(&wf_id) {
                        if let Some(p_state) = wf_state.phases.get_mut(&phase.id) {
                            p_state.status = PhaseStatus::Running;
                        }
                    }
                }

                let phase_success =
                    self.execute_phase(&wf_id, &phase, &wf_token)
                        .inspect_err(|error| {
                            self.fail_workflow(&wf_id, &error.to_string());
                        })?;
                if wf_token.is_cancelled() {
                    return Err(self.cancelled_error(&wf_id, &spec, started));
                }
                if !phase_success {
                    let reasons = self
                        .executions
                        .read()
                        .ok()
                        .and_then(|execs| {
                            execs
                                .get(&wf_id)
                                .and_then(|state| state.phases.get(&phase.id))
                                .map(|state| state.failure_reasons.clone())
                        })
                        .unwrap_or_default();
                    let err_msg = format!(
                        "Phase '{}' failed join requirements{}",
                        phase.id,
                        reason_suffix(&reasons)
                    );
                    self.fail_workflow(&wf_id, &err_msg);
                    return Err(WorkflowExecutionError::PhaseFailed {
                        phase: phase.id,
                        reasons,
                    });
                }

                completed_phase_ids.insert(phase.id.clone());
                {
                    let mut execs = self
                        .executions
                        .write()
                        .unwrap_or_else(PoisonError::into_inner);
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
            let mut execs = self
                .executions
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let state = execs.get_mut(&wf_id).unwrap();
            // A cancel that landed after the last phase already ended the run.
            if matches!(
                state.status,
                WorkflowStatus::Failed | WorkflowStatus::Cancelled
            ) {
                return Err(WorkflowExecutionError::Cancelled);
            }
            state.status = WorkflowStatus::Completed;
            state.finished_ms = Some(now_ms());
            self.runtime.emit_observe(RuntimeEvent::WorkflowEnded {
                workflow_id: wf_id,
                success: true,
            });
            state.clone()
        };

        Ok(final_state)
    }

    fn fail_workflow(&self, wf_id: &WorkflowId, error: &str) {
        let mut execs = self
            .executions
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        // A run that already ended keeps its status and its one WorkflowEnded.
        if let Some(state) = execs.get_mut(wf_id) {
            if matches!(
                state.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
            ) {
                return;
            }
            state.status = WorkflowStatus::Failed;
            state.finished_ms = Some(now_ms());
            state.error = Some(error.to_string());
            self.runtime.emit_observe(RuntimeEvent::WorkflowEnded {
                workflow_id: *wf_id,
                success: false,
            });
        }
    }

    /// A background run whose thread never started has no executor: fail it
    /// rather than leave it Running forever.
    fn abort_unlaunched(
        &self,
        wf_id: &WorkflowId,
        error: &std::io::Error,
    ) -> WorkflowExecutionError {
        let message = format!("workflow thread did not start: {error}");
        self.fail_workflow(wf_id, &message);
        WorkflowExecutionError::ExecutionError(message)
    }

    fn run_workflow_worker(
        &self,
        wf_id: &WorkflowId,
        phase: &WorkflowPhaseSpec,
        artifact_context: &str,
        identities: (AgentId, TaskId),
        worker: &WorkflowWorkerSpec,
        phase_token: &CancellationToken,
    ) -> Result<String, String> {
        let (aid, tid) = identities;
        let launch = self
            .launches
            .read()
            .unwrap()
            .get(wf_id)
            .cloned()
            .unwrap_or_default();
        let operation_adapter = self.runtime.child_operation_adapter();
        let lease = if worker.isolation.as_deref() == Some("worktree") {
            let manager = self
                .runtime
                .worktree_manager
                .clone()
                .ok_or("worktree isolation requested but no worktree manager is configured")?;
            Some(
                manager
                    .create_lease(self.runtime.run_id, aid, None)
                    .map_err(|e| format!("worktree isolation failed: {e}"))?,
            )
        } else {
            None
        };

        if let Some(lease) = &lease {
            self.runtime.registry.set_worktree(aid, lease.path.clone());
        }
        // The view reads snapshots; nothing listens to the events themselves.
        let reporter = crate::subagent_progress::ProgressReporter::new(
            format!("workflow:{wf_id}"),
            crate::EventSink(Arc::new(|_| {})),
            aid.to_string(),
            worker.id.clone(),
            (0, 1),
            "workflow",
        );
        reporter.started();
        if let Ok(mut map) = self.progress.write() {
            map.insert(aid, reporter.clone());
        }
        let agent_token = phase_token.child_token();
        if let Ok(mut tokens) = self.agent_tokens.write() {
            tokens.insert(aid, agent_token.clone());
        }
        let mut result = (|| {
            let effective_prompt = if !artifact_context.is_empty() {
                format!(
                    "{}\n\n[Context from previous phases]:\n{}",
                    worker.prompt, artifact_context
                )
            } else {
                worker.prompt.clone()
            };

            let child_token = agent_token.clone();
            let mut req = SubagentRequest {
                service_tier: launch.service_tier,
                progress: Some(reporter.clone()),
                max_turns: worker.max_turns,
                parent_tools: Some(
                    if launch.parent_tools.is_empty() && launch.parent_permission_mode.is_none() {
                        worker.tools.clone()
                    } else {
                        launch.parent_tools.clone()
                    },
                ),
                prompt: effective_prompt,
                tools: crate::subagent::scoped_tools_for_access(
                    Some(&worker.tools),
                    if launch.parent_tools.is_empty() && launch.parent_permission_mode.is_none() {
                        &worker.tools
                    } else {
                        &launch.parent_tools
                    },
                    crate::subagent::WorkerAccess::for_worker(
                        launch.parent_permission_mode,
                        lease.is_some(),
                    ),
                    &self.runtime.capability_registry,
                ),
                description: Some(worker.id.clone()),
                provider: launch.provider.clone(),
                model_id: launch.model_id.clone(),
                abort: Some(phase_token.as_atomic_bool()),
                cancellation_token: Some(child_token.clone()),
                agent: worker.agent_profile.clone(),
                mode: crate::subagent::AgentSpawnMode::Oneshot,
                model_override: worker.model.clone(),
                isolation: worker.isolation.clone(),
                instance_name: Some(worker.id.clone()),
                runtime_agent_id: Some(aid),
                runtime: Some(self.runtime.for_worker(aid, Some(child_token))?),
                parent_permission_mode: launch.parent_permission_mode,
                foreground_supervisor: launch.foreground_supervisor.clone(),
                sandbox: launch.sandbox.clone(),
                worktree_path: lease.as_ref().map(|l| l.path.clone()),
                contract_digest: None,
                active_contract: None,
            };

            let retry_limit = worker.retry_budget.unwrap_or(0);
            let base_prompt = req.prompt.clone();
            let mut attempt = 0;
            let mut outcome: Result<String, String> = Err("no runner configured".into());

            while attempt <= retry_limit {
                if phase_token.is_cancelled() {
                    outcome = Err("cancelled: workflow or phase joined".into());
                    break;
                }
                if agent_token.is_cancelled() {
                    outcome = Err("stopped from the workflows view".into());
                    break;
                }
                let operation = operation_adapter.as_ref().map(|adapter| {
                    let mut child = ChildExecutionContext::new(
                        format!(
                            "workflow:{wf_id}:{}:{}:attempt-{}",
                            phase.id,
                            worker.id,
                            attempt + 1
                        ),
                        &self.runtime,
                        Some(aid),
                    );
                    child.task_id = Some(tid);
                    child.workflow_id = Some(*wf_id);
                    child.host = "workflow_executor".to_owned();
                    let payload = serde_json::json!({
                        "workflow_id": wf_id,
                        "phase_id": phase.id,
                        "worker_id": worker.id,
                        "attempt": attempt + 1,
                        "tools": worker.tools,
                        "model": worker.model,
                        "prompt_digest": crate::runtime::operations::PayloadDigest::of_bytes(req.prompt.as_bytes()),
                    });
                    adapter
                        .start(ChildExecutionKind::WorkflowPhase, child, payload)
                });
                let operation = match operation {
                    Some(Ok(operation)) => Some(operation),
                    Some(Err(error)) => {
                        return Err(format!(
                            "workflow child launch was not durably admitted: {error}"
                        ))
                    }
                    None => None,
                };
                let run = || match &self.runner {
                    Some(r) => {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.run(&req)))
                            .unwrap_or_else(|_| Err("workflow worker panicked".into()))
                        {
                            Ok(result) => crate::tools::ToolResult {
                                content: result,
                                is_error: false,
                                details: None,
                            },
                            Err(error) => crate::tools::ToolResult {
                                content: error,
                                is_error: true,
                                details: None,
                            },
                        }
                    }
                    None => crate::tools::ToolResult {
                        content: "workflow runner is not configured".into(),
                        is_error: true,
                        details: None,
                    },
                };
                let result = match operation.as_ref() {
                    Some(operation) => operation
                        .execute(phase_token.is_cancelled(), || Ok(()), run)
                        .map_err(|error| error.to_string()),
                    None => Ok(run()),
                };
                outcome = result.and_then(|result| {
                    if result.is_error {
                        Err(result.content)
                    } else {
                        Ok(result.content)
                    }
                });
                if outcome.is_ok() {
                    break;
                }
                // A failed attempt may already have applied part of its
                // file changes, and nothing marks an operation boundary to
                // resume from. Rerunning it would repeat those effects.
                if attempt < retry_limit
                    && crate::subagent::tools_may_mutate(
                        &req.tools,
                        &self.runtime.capability_registry,
                    )
                {
                    if let Err(error) = &mut outcome {
                        error.push_str(
                            " (not retried: the worker can modify files and may have partly applied changes)",
                        );
                    }
                    break;
                }
                if attempt < retry_limit {
                    let last_error = outcome.as_ref().err().cloned().unwrap_or_default();
                    sleep_unless_cancelled(
                        retry_backoff(attempt + 1),
                        &[phase_token, &agent_token],
                    );
                    req.prompt = retry_prompt(&base_prompt, attempt + 1, &last_error);
                }
                attempt += 1;
            }

            outcome
        })();
        if let Ok(mut tokens) = self.agent_tokens.write() {
            tokens.remove(&aid);
        }
        reporter.finish(result.is_ok());
        if let (Some(manager), Some(lease)) = (&self.runtime.worktree_manager, &lease) {
            let cleanup = if result.is_ok() && !manager.is_dirty(lease) {
                manager
                    .release_lease(lease, false)
                    .map_err(|e| e.to_string())
            } else {
                Err("preserved for review".into())
            };
            if let Err(reason) = cleanup {
                let note = format!(
                    "\n\nworktree kept: {} (branch {}) — {reason}",
                    lease.path.display(),
                    lease.branch
                );
                result = result.map(|text| text + &note).map_err(|text| text + &note);
            }
        }
        result
    }

    fn cancelled_error(
        &self,
        id: &WorkflowId,
        spec: &WorkflowSpec,
        started: std::time::Instant,
    ) -> WorkflowExecutionError {
        if let Some(ms) = spec
            .deadline_ms
            .filter(|ms| started.elapsed() >= std::time::Duration::from_millis(*ms))
        {
            let error = format!("workflow deadline of {ms} ms exceeded");
            self.fail_workflow(id, &error);
            WorkflowExecutionError::ExecutionError(error)
        } else {
            let _ = self.cancel(id);
            WorkflowExecutionError::Cancelled
        }
    }

    /// Tell the lead a background run ended. The run's own state stays
    /// readable through `workflow_status`, so a lost notification is returned
    /// for the caller to report and never fails the run.
    fn report_completion(
        &self,
        id: WorkflowId,
        name: &str,
        outcome: &Result<WorkflowExecutionState, WorkflowExecutionError>,
    ) -> Result<(), String> {
        self.runtime
            .ensure_lead_registered("", "", std::path::Path::new("."));
        let sender = AgentId::new();
        let record = AgentRecord {
            id: sender,
            run_id: self.runtime.run_id,
            parent: Some(self.runtime.agent_id),
            kind: AgentKind::WorkflowWorker,
            name: format!("workflow:{name}"),
            provider: String::new(),
            model_id: String::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
            state: AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: now_ms(),
            updated_ms: now_ms(),
            failure_reason: None,
        };
        let mut problems = Vec::new();
        let registered = self.runtime.registry.register_agent(record);
        if let Err(error) = &registered {
            problems.push(format!("sender not registered: {error}"));
        }
        let content = match outcome {
            Ok(state) => format!(
                "status: completed

workflow '{name}' ({id}) {:?}. Use workflow_status for artifacts.",
                state.status
            ),
            Err(error) => format!(
                "status: failed

workflow '{name}' ({id}) failed: {error}"
            ),
        };
        if let Err(error) = self.runtime.mailbox.send(crate::runtime::AgentMessage::new(
            self.runtime.run_id,
            sender,
            self.runtime.agent_id,
            content,
        )) {
            problems.push(format!("not delivered to the lead: {error}"));
        }
        if registered.is_ok() {
            if let Err(error) = self.runtime.registry.transition(
                sender,
                if outcome.is_ok() {
                    AgentState::Completed
                } else {
                    AgentState::Failed
                },
            ) {
                problems.push(format!("sender not closed: {error}"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }

    /// Register a worker agent and give it a task it owns. On failure the
    /// worker is not launched; whatever this call registered is closed out.
    fn admit_worker(
        &self,
        aid: AgentId,
        phase_id: &str,
        worker: &WorkflowWorkerSpec,
    ) -> Result<TaskId, WorkflowExecutionError> {
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
            worktree: None,
            started_ms: now,
            updated_ms: now,
            failure_reason: None,
        };
        self.runtime.registry.register_agent(record).map_err(|e| {
            WorkflowExecutionError::ExecutionError(format!(
                "could not register worker '{}': {e}",
                worker.id
            ))
        })?;

        let task_rec = crate::runtime::tasks::TaskRecord::new(
            self.runtime.run_id,
            format!("{phase_id}:{}", worker.id),
        )
        .with_description(worker.prompt.clone());
        let admitted = self
            .runtime
            .task_registry
            .create_task(task_rec)
            .map_err(|e| e.to_string())
            .and_then(|tid| {
                self.runtime
                    .task_registry
                    .assign_task(tid, aid)
                    .map(|()| tid)
                    .map_err(|e| {
                        // Best effort: the assignment error is what gets reported.
                        let _ = self
                            .runtime
                            .task_registry
                            .fail_task(tid, Some(e.to_string()));
                        e.to_string()
                    })
            });
        admitted.map_err(|reason| {
            let message = format!(
                "could not assign a task to worker '{}': {reason}",
                worker.id
            );
            self.runtime.registry.set_failure_reason(aid, &message);
            // Cleanup of an agent that never ran; the error below is the report.
            let _ = self.runtime.registry.transition(aid, AgentState::Failed);
            WorkflowExecutionError::ExecutionError(message)
        })
    }

    /// Close out workers that were admitted but will never launch.
    fn abandon_admitted(&self, admitted: &[(AgentId, TaskId)], reason: &str) {
        for (aid, tid) in admitted {
            self.runtime.registry.set_failure_reason(*aid, reason);
            // Cleanup after a failed admission; the admission error is the report.
            let _ = self.runtime.registry.transition(*aid, AgentState::Failed);
            let _ = self
                .runtime
                .task_registry
                .fail_task(*tid, Some(reason.to_string()));
        }
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
            match self.admit_worker(aid, &phase.id, worker) {
                Ok(tid) => {
                    worker_agent_ids.push(aid);
                    worker_tasks.push((aid, tid, worker.clone()));
                }
                Err(error) => {
                    // Nothing has launched yet: close out the workers
                    // admitted so far instead of leaving them Running.
                    let admitted: Vec<_> = worker_tasks.iter().map(|(a, t, _)| (*a, *t)).collect();
                    self.abandon_admitted(&admitted, &error.to_string());
                    return Err(error);
                }
            }
        }

        if let Some(state) = self
            .executions
            .write()
            .unwrap()
            .get_mut(wf_id)
            .and_then(|s| s.phases.get_mut(&phase.id))
        {
            state.worker_agent_ids = worker_agent_ids;
            state.task_ids = worker_tasks.iter().map(|(_, tid, _)| *tid).collect();
        }
        // Execute workers using scheduler or runner
        let mut successful_workers: Vec<AgentId> = Vec::new();
        let mut failed_workers: Vec<AgentId> = Vec::new();
        let mut failure_reasons: Vec<String> = Vec::new();
        let requested = self
            .specs
            .read()
            .ok()
            .and_then(|specs| specs.get(wf_id).map(|s| s.max_parallel_agents))
            .unwrap_or(1);
        // The spec asks; the session ceiling (`workflowMaxConcurrentAgents`)
        // decides. A launch without one uses the default ceiling.
        let ceiling = self
            .launches
            .read()
            .ok()
            .and_then(|launches| launches.get(wf_id).and_then(|l| l.max_concurrent_agents))
            .unwrap_or(super::limits::DEFAULT_MAX_CONCURRENT_AGENTS);
        let limit = super::limits::WorkflowSettings {
            max_concurrent_agents: ceiling,
            ..super::limits::WorkflowSettings::default()
        }
        .concurrency_for(requested)
        .min(phase.workers.len().max(1));
        let phase_token = wf_token.child_token();
        let successes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let queue = std::sync::Mutex::new(worker_tasks.into_iter());
        let out = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..limit {
                handles.push(scope.spawn(|| loop {
                    self.wait_while_paused(wf_id, &phase_token);
                    let next = queue.lock().unwrap_or_else(|p| p.into_inner()).next();
                    let Some((aid, tid, worker)) = next else {
                        break;
                    };
                    let result = if phase_token.is_cancelled() {
                        Err("cancelled: phase already joined or workflow cancelled".to_string())
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            self.run_workflow_worker(
                                wf_id,
                                phase,
                                &artifact_context,
                                (aid, tid),
                                &worker,
                                &phase_token,
                            )
                        }))
                        .unwrap_or_else(|_| Err("workflow worker panicked".into()))
                    };
                    if result.is_ok() {
                        let done = successes.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        let joined_early = match phase.join {
                            WorkflowJoin::Any => done >= 1,
                            WorkflowJoin::Quorum { required } => done >= required,
                            WorkflowJoin::All => false,
                        };
                        if joined_early {
                            phase_token.cancel();
                        }
                    }
                    out.lock().unwrap_or_else(|p| p.into_inner()).push((
                        aid,
                        tid,
                        worker.id.clone(),
                        result,
                    ));
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
        });
        let mut results = out.into_inner().unwrap_or_else(|p| p.into_inner());
        results.sort_by_key(|(_, _, id, _)| {
            phase
                .workers
                .iter()
                .position(|w| &w.id == id)
                .unwrap_or(usize::MAX)
        });
        let joined_early = match phase.join {
            WorkflowJoin::Any => successes.load(std::sync::atomic::Ordering::SeqCst) > 0,
            WorkflowJoin::Quorum { required } => {
                successes.load(std::sync::atomic::Ordering::SeqCst) >= required
            }
            WorkflowJoin::All => false,
        };
        for (aid, tid, worker_id, outcome) in results {
            match outcome {
                Ok(output) => {
                    let _ = self.runtime.registry.transition(aid, AgentState::Completed);
                    let _ = self.runtime.task_registry.complete_task(tid, None);
                    successful_workers.push(aid);
                    self.store
                        .put_artifact(
                            *wf_id,
                            &phase.id,
                            aid,
                            serde_json::json!({"worker": worker_id, "output": output}),
                            None,
                        )
                        .map_err(|e| WorkflowExecutionError::ExecutionError(e.to_string()))?;
                }
                Err(error) => {
                    self.runtime.registry.set_failure_reason(aid, &error);
                    failure_reasons.push(format!("{worker_id}: {error}"));
                    if joined_early || wf_token.is_cancelled() {
                        let _ = self.runtime.registry.transition(aid, AgentState::Cancelled);
                        let _ = self.runtime.task_registry.cancel_task(tid);
                    } else {
                        let _ = self.runtime.registry.transition(aid, AgentState::Failed);
                        let _ = self.runtime.task_registry.fail_task(tid, Some(error));
                    }
                    failed_workers.push(aid);
                }
            }
        }

        // Update phase execution state
        {
            let mut execs = self
                .executions
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(wf_state) = execs.get_mut(wf_id) {
                if let Some(p_state) = wf_state.phases.get_mut(&phase.id) {
                    p_state.completed_workers = successful_workers.clone();
                    p_state.failed_workers = failed_workers.clone();
                    p_state.failure_reasons = failure_reasons.clone();
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
        let handle = handle.with_worktree_manager(super::super::test_worktree_manager(tmp.path()));
        let store = WorkflowStateStore::with_options(32 * 1024, tmp.path().to_path_buf());
        (
            WorkflowExecutor::new(
                handle,
                store,
                runner.or_else(|| Some(SubagentRunner::new(|_| Ok("fixture result".into())))),
            ),
            tmp,
        )
    }

    #[test]
    fn a_poisoned_state_lock_does_not_cascade_into_every_later_call() {
        let (executor, _tmp) = setup_executor(None);
        let executions = executor.executions.clone();
        let tokens = executor.tokens.clone();
        let _ = std::thread::spawn(move || {
            let _executions = executions.write().unwrap();
            let _tokens = tokens.write().unwrap();
            panic!("worker panicked while holding workflow state");
        })
        .join();
        assert!(executor.executions.is_poisoned());
        assert!(executor.tokens.is_poisoned());

        let unknown = WorkflowId::new();
        assert!(executor.get_state(&unknown).is_none());
        let _ = executor.cancel(&unknown);
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
    fn a_failed_phase_names_each_failed_worker_and_its_error() {
        let runner = SubagentRunner::new(|req| {
            if req.instance_name.as_deref() == Some("broken-worker") {
                Err("worktree isolation failed: git lock held".into())
            } else {
                Ok("fine".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let worker = |id: &str| WorkflowWorkerSpec {
            id: id.into(),
            prompt: "work".into(),
            agent_profile: None,
            model: None,
            tools: vec!["read".into()],
            isolation: None,
            max_turns: None,
            retry_budget: None,
        };
        let spec = WorkflowSpec {
            schema_version: 1,
            name: "reason-test".into(),
            max_parallel_agents: 2,
            max_total_agents: 2,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "implement".into(),
                depends_on: vec![],
                join: WorkflowJoin::All,
                workers: vec![worker("good-worker"), worker("broken-worker")],
            }],
        };
        let error = executor.execute(spec).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("phase 'implement' failed (broken-worker: worktree isolation failed: git lock held)"),
            "{message}"
        );
        assert!(!message.contains("good-worker"), "{message}");
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

    fn retrying_writer_spec(tool: &str) -> WorkflowSpec {
        WorkflowSpec {
            schema_version: 1,
            name: "retry-writer".into(),
            max_parallel_agents: 1,
            max_total_agents: 1,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "step".into(),
                depends_on: vec![],
                join: WorkflowJoin::All,
                workers: vec![WorkflowWorkerSpec {
                    id: "writer".into(),
                    prompt: "work".into(),
                    agent_profile: None,
                    model: None,
                    tools: vec![tool.into()],
                    isolation: None,
                    max_turns: None,
                    retry_budget: Some(2),
                }],
            }],
        }
    }

    fn writer_launch() -> WorkflowLaunch {
        WorkflowLaunch {
            parent_permission_mode: Some(crate::PermissionMode::AlwaysApprove),
            parent_tools: vec!["read".into(), "write".into()],
            ..Default::default()
        }
    }

    #[test]
    fn wor26_failed_mutating_worker_is_not_retried_after_a_partial_effect() {
        use std::io::Write as _;
        let (executor, tmp) = setup_executor(None);
        let target = tmp.path().join("effect.log");
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (att, path) = (Arc::clone(&attempts), target.clone());
        let runner = SubagentRunner::new(move |_| {
            att.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(file, "applied").unwrap();
            Err("crashed before the success receipt".into())
        });
        let executor = WorkflowExecutor::new(
            executor.runtime.clone(),
            WorkflowStateStore::with_options(32 * 1024, tmp.path().to_path_buf()),
            Some(runner),
        );
        let error = executor
            .execute_with(retrying_writer_spec("write"), writer_launch())
            .unwrap_err();
        assert!(error.to_string().contains("not retried"), "{error}");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "applied
"
        );
    }

    #[test]
    fn wor26_failed_read_only_worker_is_still_retried_under_a_writer_launch() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let att = Arc::clone(&attempts);
        let runner = SubagentRunner::new(move |_| {
            if att.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Err("transient glitch".into())
            } else {
                Ok("recovered".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let state = executor
            .execute_with(retrying_writer_spec("read"), writer_launch())
            .unwrap();
        assert_eq!(state.status, WorkflowStatus::Completed);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
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
        for _ in 0..500 {
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
        valid_fps.insert(format!("{wf_id}:write_code:writer-1"));

        let res = executor.resume_execution(wf_id, spec, &valid_fps);
        assert!(
            res.is_ok(),
            "resume should succeed with validated fingerprint"
        );
        assert_eq!(res.unwrap().status, WorkflowStatus::Completed);
    }
    fn two_worker_phase(isolation: Option<&str>, tools: &[&str]) -> WorkflowSpec {
        let worker = |id: &str| WorkflowWorkerSpec {
            id: id.into(),
            prompt: format!("do {id}"),
            agent_profile: None,
            model: None,
            tools: tools.iter().map(|t| t.to_string()).collect(),
            isolation: isolation.map(str::to_string),
            max_turns: Some(7),
            retry_budget: None,
        };
        WorkflowSpec {
            schema_version: 1,
            name: "pair".into(),
            max_parallel_agents: 2,
            max_total_agents: 2,
            max_cost_usd: None,
            deadline_ms: None,
            phases: vec![WorkflowPhaseSpec {
                id: "p".into(),
                depends_on: vec![],
                workers: vec![worker("a"), worker("b")],
                join: WorkflowJoin::All,
            }],
        }
    }

    #[test]
    fn phase_workers_run_concurrently() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let b = barrier.clone();
        let runner = SubagentRunner::new(move |_| {
            // Deadlocks (and the test times out) if workers run one by one.
            b.wait();
            Ok("ok".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(executor.execute(two_worker_phase(None, &["read"])));
        });
        let state = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("workers ran sequentially")
            .unwrap();
        assert_eq!(state.status, WorkflowStatus::Completed);
    }

    #[test]
    fn workers_inherit_parent_model_and_max_turns() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let runner = SubagentRunner::new(move |req| {
            s.lock().unwrap().push((
                req.provider.clone(),
                req.model_id.clone(),
                req.max_turns,
                req.parent_permission_mode,
            ));
            Ok("ok".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        executor
            .execute_with(
                two_worker_phase(None, &["read"]),
                WorkflowLaunch {
                    parent_permission_mode: Some(crate::PermissionMode::Ask),
                    provider: Some("anthropic".into()),
                    model_id: Some("claude-opus-5-5".into()),
                    ..WorkflowLaunch::default()
                },
            )
            .unwrap();
        for (provider, model, turns, mode) in seen.lock().unwrap().iter() {
            assert_eq!(provider.as_deref(), Some("anthropic"));
            assert_eq!(model.as_deref(), Some("claude-opus-5-5"));
            assert_eq!(*turns, Some(7));
            assert_eq!(*mode, Some(crate::PermissionMode::Ask));
        }
    }

    #[test]
    fn workflow_without_runner_fails_instead_of_fabricating_results() {
        let (mut executor, _tmp) = setup_executor(None);
        executor.runner = None;
        assert!(executor.execute(two_worker_phase(None, &["read"])).is_err());
        assert!(executor
            .executions
            .read()
            .unwrap()
            .values()
            .all(|state| state.status == WorkflowStatus::Failed));
    }

    #[test]
    fn empty_parent_tool_ceiling_is_enforced() {
        let (executor, _tmp) = setup_executor(Some(SubagentRunner::new(|req| {
            assert!(req.tools.is_empty());
            assert_eq!(req.parent_tools, Some(Vec::new()));
            Ok("scoped".into())
        })));
        executor
            .execute_with(
                two_worker_phase(None, &["read"]),
                WorkflowLaunch {
                    parent_permission_mode: Some(crate::PermissionMode::Ask),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    #[test]
    fn background_workflow_reports_preserved_real_worktrees() {
        let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = paths.clone();
        let (executor, _tmp) = setup_executor(Some(SubagentRunner::new(move |req| {
            let path = req.worktree_path.as_ref().unwrap();
            assert!(path.join(".git").is_file());
            assert!(req.tools.iter().any(|tool| tool == "write"));
            std::fs::write(path.join("result.txt"), "worker output").unwrap();
            capture.lock().unwrap().push(path.clone());
            Ok("edited".into())
        })));
        let id = executor
            .execute_background_with(
                two_worker_phase(Some("worktree"), &["write"]),
                WorkflowLaunch {
                    parent_permission_mode: Some(crate::PermissionMode::Edits),
                    parent_tools: vec!["write".into()],
                    report_to_lead: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let started = std::time::Instant::now();
        while executor
            .runtime
            .mailbox
            .pending_count(&executor.runtime.agent_id)
            == 0
        {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let reports = executor.runtime.take_labeled_messages(10).join("\n");
        assert!(reports.contains("kind=\"workflow_worker\""));
        assert!(reports.contains("status: completed"));
        let paths = paths.lock().unwrap();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0], paths[1]);
        let state = executor.get_state(&id).unwrap();
        for phase in state.phases.values() {
            let outputs = executor
                .store
                .list_phase_artifacts(id, &phase.id)
                .iter()
                .map(|artifact| artifact.value.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            for agent in &phase.worker_agent_ids {
                let path = executor
                    .runtime
                    .registry
                    .get(agent)
                    .unwrap()
                    .worktree
                    .unwrap();
                assert!(path.join("result.txt").exists());
                assert!(outputs.contains("worktree kept"));
            }
        }
    }

    #[test]
    fn plan_mode_parent_rejects_mutating_workflows() {
        let (executor, _tmp) = setup_executor(Some(SubagentRunner::new(|_| Ok("x".into()))));
        let err = executor
            .execute_with(
                two_worker_phase(Some("worktree"), &["read", "write"]),
                WorkflowLaunch {
                    parent_permission_mode: Some(crate::PermissionMode::ReadOnly),
                    ..WorkflowLaunch::default()
                },
            )
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("read-only")
                || err.to_string().to_lowercase().contains("permission")
        );
    }

    fn wait_until_cancelled_runner() -> SubagentRunner {
        SubagentRunner::new(|req| {
            let token = req.cancellation_token.clone().unwrap();
            let start = std::time::Instant::now();
            while !token.is_cancelled() {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    return Ok("never cancelled".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err("cancelled".into())
        })
    }

    fn wait_for_status(
        executor: &WorkflowExecutor,
        id: &WorkflowId,
        wanted: impl Fn(&WorkflowStatus) -> bool,
    ) -> WorkflowStatus {
        let start = std::time::Instant::now();
        loop {
            let status = executor.get_state(id).unwrap().status;
            if wanted(&status) || start.elapsed() > std::time::Duration::from_secs(8) {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn the_session_ceiling_bounds_phase_concurrency() {
        let running = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (r, p) = (running.clone(), peak.clone());
        let runner = SubagentRunner::new(move |_| {
            let now = r.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            p.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(60));
            r.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok("ok".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut spec = two_worker_phase(None, &["read"]);
        let template = spec.phases[0].workers[0].clone();
        spec.phases[0].workers = (0..6)
            .map(|index| WorkflowWorkerSpec {
                id: format!("w{index}"),
                ..template.clone()
            })
            .collect();
        spec.max_parallel_agents = 6;
        spec.max_total_agents = 6;
        let state = executor
            .execute_with(
                spec,
                WorkflowLaunch {
                    max_concurrent_agents: Some(2),
                    ..WorkflowLaunch::default()
                },
            )
            .unwrap();
        assert_eq!(state.status, WorkflowStatus::Completed);
        assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_paused_run_starts_no_new_agents_until_resumed() {
        let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = started.clone();
        let runner = SubagentRunner::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(80));
            Ok("ok".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut spec = two_worker_phase(None, &["read"]);
        spec.max_parallel_agents = 1;
        let id = executor.execute_background(spec).unwrap();
        let start = std::time::Instant::now();
        while started.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        executor.pause(&id).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);
        executor.resume(&id).unwrap();
        let status = wait_for_status(&executor, &id, |s| *s == WorkflowStatus::Completed);
        assert_eq!(status, WorkflowStatus::Completed);
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn one_agent_can_be_stopped_and_counts_as_failed() {
        let (executor, _tmp) = setup_executor(Some(wait_until_cancelled_runner()));
        let mut spec = two_worker_phase(None, &["read"]);
        spec.phases[0].workers.truncate(1);
        spec.max_total_agents = 1;
        let id = executor.execute_background(spec).unwrap();
        let start = std::time::Instant::now();
        let agent = loop {
            let running = executor
                .get_state(&id)
                .and_then(|state| state.phases.values().next().cloned())
                .and_then(|phase| phase.worker_agent_ids.first().copied());
            if let Some(agent) = running.filter(|agent| executor.cancel_agent(agent)) {
                break agent;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let status = wait_for_status(&executor, &id, |s| *s != WorkflowStatus::Running);
        assert_eq!(status, WorkflowStatus::Failed);
        let record = executor.runtime.registry.get(&agent).unwrap();
        assert_eq!(record.state, AgentState::Failed);
        assert!(
            !executor.cancel_agent(&agent),
            "a finished agent cannot be stopped"
        );
    }

    #[test]
    fn a_background_workflow_survives_the_lead_turn_being_interrupted() {
        let (executor, _tmp) = setup_executor(Some(wait_until_cancelled_runner()));
        let id = executor
            .execute_background(two_worker_phase(None, &["read"]))
            .unwrap();
        // Esc on the lead cancels the turn's runtime token.
        executor.runtime.cancellation_token.cancel();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert_eq!(
            executor.get_state(&id).unwrap().status,
            WorkflowStatus::Running,
            "Esc must not stop a background workflow"
        );
        executor.cancel(&id).unwrap();
        let status = wait_for_status(&executor, &id, |s| *s != WorkflowStatus::Running);
        assert_eq!(status, WorkflowStatus::Cancelled);
    }

    #[test]
    fn a_session_switch_stops_background_workflows() {
        let (executor, _tmp) = setup_executor(Some(wait_until_cancelled_runner()));
        let id = executor
            .execute_background(two_worker_phase(None, &["read"]))
            .unwrap();
        executor.runtime.team.shutdown_all();
        let status = wait_for_status(&executor, &id, |s| *s != WorkflowStatus::Running);
        assert_ne!(status, WorkflowStatus::Running);
        assert_ne!(status, WorkflowStatus::Completed);
    }

    #[test]
    fn a_synchronous_workflow_stops_with_the_calling_turn() {
        let (executor, _tmp) = setup_executor(Some(wait_until_cancelled_runner()));
        let turn = CancellationToken::new();
        let canceller = turn.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            canceller.cancel();
        });
        let started = std::time::Instant::now();
        let result = executor.execute_with(
            two_worker_phase(None, &["read"]),
            WorkflowLaunch {
                turn_token: Some(turn),
                ..WorkflowLaunch::default()
            },
        );
        assert_eq!(result.unwrap_err(), WorkflowExecutionError::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn deadline_cancels_a_running_workflow() {
        let runner = SubagentRunner::new(|req| {
            let token = req.cancellation_token.clone().unwrap();
            let start = std::time::Instant::now();
            while !token.is_cancelled() {
                assert!(start.elapsed() < std::time::Duration::from_secs(5));
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err("cancelled".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut spec = two_worker_phase(None, &["read"]);
        spec.deadline_ms = Some(150);
        let started = std::time::Instant::now();
        let result = executor.execute(spec);
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn early_any_join_cancels_the_rest() {
        let runner = SubagentRunner::new(|req| {
            if req.instance_name.as_deref() == Some("a") {
                return Ok("fast".into());
            }
            let token = req.cancellation_token.clone().unwrap();
            let start = std::time::Instant::now();
            while !token.is_cancelled() {
                assert!(
                    start.elapsed() < std::time::Duration::from_secs(5),
                    "never cancelled"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err("cancelled".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut spec = two_worker_phase(None, &["read"]);
        spec.phases[0].join = WorkflowJoin::Any;
        let state = executor.execute(spec).unwrap();
        assert_eq!(state.status, WorkflowStatus::Completed);
        let unfinished = executor
            .runtime
            .registry
            .get_by_run(&executor.runtime.run_id)
            .into_iter()
            .filter(|r| matches!(r.state, AgentState::Running | AgentState::Starting))
            .count();
        assert_eq!(unfinished, 0);
    }

    /// WOR-104 review: a phase whose worker output overflowed to disk in an
    /// earlier process is still recognised as done, not run again.
    #[test]
    fn resume_after_restart_reuses_an_overflowed_phase() {
        let (executor, tmp) = setup_executor(None);
        let spec: WorkflowSpec = serde_json::from_str(
            r#"{
            "schema_version": 1,
            "name": "overflow-resume",
            "max_parallel_agents": 1,
            "max_total_agents": 2,
            "phases": [
                {
                    "id": "research",
                    "join": "all",
                    "workers": [
                        {"id": "researcher-1", "prompt": "find", "tools": ["read"]}
                    ]
                }
            ]
        }"#,
        )
        .unwrap();
        let wf_id = WorkflowId::new();
        let earlier = WorkflowStateStore::with_options(32 * 1024, tmp.path().to_path_buf());
        let artifact = earlier
            .put_artifact(
                wf_id,
                "research",
                AgentId::new(),
                serde_json::json!({"worker": "researcher-1", "output": "z".repeat(40 * 1024)}),
                None,
            )
            .unwrap();
        assert!(artifact.is_overflow);
        drop(earlier);

        let state = executor
            .resume_execution(wf_id, spec, &HashSet::new())
            .expect("resume succeeds");
        assert_eq!(state.phases["research"].status, PhaseStatus::Completed);
        assert_eq!(
            executor.store.list_phase_artifacts(wf_id, "research").len(),
            1,
            "the overflowed phase ran again"
        );
    }

    fn phase_spec(join: &str, workers: &[&str]) -> WorkflowSpec {
        let workers: Vec<_> = workers
            .iter()
            .map(|id| serde_json::json!({"id": id, "prompt": "p", "tools": ["read"]}))
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "name": "resume-test",
            "max_parallel_agents": 4,
            "max_total_agents": 8,
            "phases": [{"id": "work", "join": join, "workers": workers}]
        }))
        .unwrap()
    }

    /// Resume `spec` over `artifacts` already stored for phase `work`, and
    /// count how many workers had to run.
    fn resume_runs(spec: WorkflowSpec, artifacts: &[&str]) -> usize {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let runner = SubagentRunner::new(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("ran".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let wf_id = WorkflowId::new();
        for worker in artifacts {
            executor
                .store
                .put_artifact(
                    wf_id,
                    "work",
                    AgentId::new(),
                    serde_json::json!({"worker": worker, "output": "done"}),
                    None,
                )
                .unwrap();
        }
        executor
            .resume_execution(wf_id, spec, &HashSet::new())
            .unwrap();
        runs.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn wor108_artifacts_the_executor_wrote_satisfy_an_all_phase_on_resume() {
        // The executor stores `{"worker": <spec id>, ...}` and the store keeps
        // that field on overflow, so the field resume reads does exist.
        assert_eq!(resume_runs(phase_spec("all", &["a", "b"]), &["a", "b"]), 0);
        // An artifact with no worker name cannot be tied to a worker.
        assert_eq!(resume_runs(phase_spec("all", &["a", "b"]), &["a"]), 2);
    }

    #[test]
    fn wor109_wor122_quorum_resume_counts_distinct_workers() {
        let mut spec = phase_spec("any", &["a", "b", "c"]);
        spec.phases[0].join = WorkflowJoin::Quorum { required: 2 };
        assert!(
            resume_runs(spec.clone(), &["a", "a"]) > 0,
            "one worker twice is not a quorum"
        );
        assert_eq!(resume_runs(spec, &["a", "b"]), 0);
    }

    #[test]
    fn wor110_wor123_any_resume_ignores_artifacts_from_undeclared_workers() {
        let spec = phase_spec("any", &["a", "b"]);
        assert!(resume_runs(spec.clone(), &["ghost"]) > 0);
        assert_eq!(resume_runs(spec, &["b"]), 0);
    }

    fn event_log(
        runner: Option<SubagentRunner>,
    ) -> (
        WorkflowExecutor,
        tempfile::TempDir,
        Arc<std::sync::Mutex<Vec<RuntimeEvent>>>,
    ) {
        struct Capture(Arc<std::sync::Mutex<Vec<RuntimeEvent>>>);
        impl crate::runtime::bus::RuntimeSubscriber for Capture {
            fn on_event(
                &self,
                event: &crate::runtime::events::RuntimeEventEnvelope,
            ) -> crate::runtime::bus::RuntimeDecision {
                self.0.lock().unwrap().push(event.payload.clone());
                crate::runtime::bus::RuntimeDecision::Continue
            }
        }
        let bus = crate::runtime::RuntimeBus::default();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(Capture(events.clone())));
        let handle = RuntimeHandle::new(crate::runtime::RunId::new(), AgentId::new(), bus);
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(32 * 1024, tmp.path().to_path_buf());
        let executor = WorkflowExecutor::new(
            handle,
            store,
            runner.or_else(|| Some(SubagentRunner::new(|_| Ok("ok".into())))),
        );
        (executor, tmp, events)
    }

    fn ended_events(events: &Arc<std::sync::Mutex<Vec<RuntimeEvent>>>) -> Vec<bool> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                RuntimeEvent::WorkflowEnded { success, .. } => Some(*success),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn wor135_a_cancelled_run_ends_once_and_stays_cancelled() {
        let (executor, _tmp, events) = event_log(None);
        let spec = phase_spec("all", &["a"]);
        let wf_id = WorkflowId::new();
        executor.init_workflow_state(&spec, wf_id, CancellationToken::new());
        executor.cancel(&wf_id).unwrap();
        // The runner reaches its own terminal calls after the cancel.
        executor.fail_workflow(&wf_id, "late failure");
        executor.cancel(&wf_id).unwrap();
        assert_eq!(
            executor.get_state(&wf_id).unwrap().status,
            WorkflowStatus::Cancelled
        );
        assert_eq!(ended_events(&events), vec![false]);
    }

    #[test]
    fn wor135_cancelling_a_paused_run_ends_it() {
        let (executor, _tmp, events) = event_log(None);
        let spec = phase_spec("all", &["a"]);
        let wf_id = WorkflowId::new();
        executor.init_workflow_state(&spec, wf_id, CancellationToken::new());
        executor.pause(&wf_id).unwrap();
        executor.cancel(&wf_id).unwrap();
        assert_eq!(
            executor.get_state(&wf_id).unwrap().status,
            WorkflowStatus::Cancelled
        );
        assert_eq!(ended_events(&events), vec![false]);
    }

    #[test]
    fn wor134_a_background_run_that_cannot_start_a_thread_ends_failed() {
        let (executor, _tmp, events) = event_log(None);
        let spec = phase_spec("all", &["a"]);
        let wf_id = WorkflowId::new();
        executor.init_workflow_state(&spec, wf_id, CancellationToken::new());
        let error = executor.abort_unlaunched(&wf_id, &std::io::Error::other("no threads"));
        assert!(error.to_string().contains("no threads"), "{error}");
        let state = executor.get_state(&wf_id).unwrap();
        assert_eq!(state.status, WorkflowStatus::Failed);
        assert!(state.error.unwrap().contains("did not start"));
        assert_eq!(ended_events(&events), vec![false]);
    }

    #[test]
    fn wor131_wor139_a_worker_that_cannot_be_registered_gets_no_task() {
        let (executor, _tmp) = setup_executor(None);
        let spec = phase_spec("all", &["a"]);
        let taken = AgentId::new();
        executor
            .admit_worker(taken, "work", &spec.phases[0].workers[0])
            .unwrap();
        let tasks_before = executor.runtime.task_registry.list_tasks(None).len();
        let error = executor
            .admit_worker(taken, "work", &spec.phases[0].workers[0])
            .unwrap_err();
        assert!(error.to_string().contains("could not register"), "{error}");
        assert_eq!(
            executor.runtime.task_registry.list_tasks(None).len(),
            tasks_before,
            "a task was created for an unregistered worker"
        );
    }

    /// A task store that accepts creation and refuses the commit that
    /// assigns an owner.
    struct RefuseAssignment;
    impl crate::runtime::task_store::TaskCommitSink for RefuseAssignment {
        fn commit(
            &self,
            changes: Vec<crate::runtime::task_store::TaskChange>,
        ) -> Result<(), crate::runtime::tasks::TaskError> {
            if changes
                .iter()
                .any(|c| c.record.assigned_to.is_some() && c.prior_revision.is_some())
            {
                return Err(crate::runtime::tasks::TaskError::Persistence(
                    "fixture: journal refused the assignment".into(),
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn wor130_wor140_a_refused_task_assignment_stops_the_phase_before_launch() {
        let launched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&launched);
        let runner = SubagentRunner::new(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("ran".into())
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut executor = executor;
        executor.runtime = executor.runtime.clone().with_task_registry(
            crate::runtime::tasks::TaskRegistry::new().with_commit_sink(Arc::new(RefuseAssignment)),
        );
        let error = executor
            .execute(phase_spec("all", &["a", "b"]))
            .unwrap_err();
        assert!(
            error.to_string().contains("could not assign a task"),
            "{error}"
        );
        assert_eq!(launched.load(std::sync::atomic::Ordering::SeqCst), 0);
        let still_running = executor
            .runtime
            .registry
            .get_by_run(&executor.runtime.run_id)
            .into_iter()
            .filter(|r| r.kind == AgentKind::Subagent && r.state == AgentState::Running)
            .count();
        assert_eq!(still_running, 0, "an unlaunched worker was left Running");
        let wf = executor.list_workflows().pop().unwrap();
        assert_eq!(wf.status, WorkflowStatus::Failed);
    }

    #[test]
    fn wor128_wor114_a_retry_waits_and_carries_the_last_error() {
        let calls = Arc::new(std::sync::Mutex::new(
            Vec::<(std::time::Instant, String)>::new(),
        ));
        let log = Arc::clone(&calls);
        let runner = SubagentRunner::new(move |req| {
            let mut log = log.lock().unwrap();
            log.push((std::time::Instant::now(), req.prompt.clone()));
            if log.len() == 1 {
                Err("provider overloaded".into())
            } else {
                Ok("recovered".into())
            }
        });
        let (executor, _tmp) = setup_executor(Some(runner));
        let mut spec = phase_spec("all", &["a"]);
        spec.phases[0].workers[0].retry_budget = Some(2);
        executor.execute(spec).unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1].0.duration_since(calls[0].0) >= std::time::Duration::from_millis(200),
            "retry fired with no backoff"
        );
        assert!(calls[1].1.starts_with("p"), "{}", calls[1].1);
        assert!(calls[1].1.contains("provider overloaded"), "{}", calls[1].1);
        assert!(!calls[0].1.contains("Retry"));
    }

    #[test]
    fn wor128_backoff_grows_and_is_capped() {
        let ms = |n| retry_backoff(n).as_millis() as u64;
        assert!((250..=313).contains(&ms(1)), "{}", ms(1));
        assert!(ms(2) >= 500);
        assert!(ms(30) <= RETRY_BACKOFF_MAX_MS + RETRY_BACKOFF_MAX_MS / 4);
    }

    #[test]
    fn wor128_a_cancelled_run_does_not_sit_out_the_backoff() {
        let token = CancellationToken::new();
        token.cancel();
        let started = std::time::Instant::now();
        sleep_unless_cancelled(std::time::Duration::from_secs(5), &[&token]);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn wor136_wor143_a_lost_completion_notice_is_reported_not_swallowed() {
        let (executor, _tmp, _events) = event_log(None);
        let id = WorkflowId::new();
        let outcome = Err(WorkflowExecutionError::Cancelled);
        executor.report_completion(id, "ok", &outcome).unwrap();
        // The lead has gone; the mailbox refuses mail for a finished agent.
        let lead = executor.runtime.agent_id;
        executor
            .runtime
            .registry
            .transition(lead, AgentState::Cancelled)
            .unwrap();
        let problem = executor
            .report_completion(id, "gone", &outcome)
            .unwrap_err();
        assert!(problem.contains("not delivered to the lead"), "{problem}");
    }
}
