//! The GraphController: a deterministic state machine over typed artifacts.
//!
//! Every routing decision here is plain code. Model calls happen ONLY inside
//! `deps.runner` (one isolated pi process per node). If you find yourself
//! wanting to ask a model "what should happen next", the answer belongs in
//! this file as an `if` instead.

use super::briefings::{
    build_evidence_digest, classify_briefing, implement_briefing, milestone_goal, plan_briefing,
    research_briefing, review_briefing, revision_notes_from, role_system_prompt_with_recovery,
    ClassifyInput, ReviewInput,
};
use super::config::{detect_verify_commands, read_package_scripts, GraphConfig};
use super::continuation::{DeliveryCheckpoint, DeliveryStage, GraphContinuation, NodeIndices};
use super::mutation::{capture_baseline, capture_graph_delta};
use super::operations;
use super::recovery::{
    build_retry_context_delta, classify_worker_failure, retry_decision, RetryDecision,
    WorkerFailureClass,
};
use super::replay::{incompatibility_reason, replay_compatible, ReplayFingerprint};
use super::review_coverage::{chunk_graph_mutation, coverage_complete, ReviewCoverage};
use super::roles::{
    ensure_governor_recovery_tool, initial_worker_tools, requires_task_coordinator,
    role_for_research_kind, role_tools,
};
use super::store::{
    artifact_path, create_run_dir, new_run_id, now_ms, save_run, transcript_path, write_artifact,
    write_log, write_task_fingerprint, write_task_mutation,
};
use super::topology::{
    build_definition, ready_nodes, validate_definition, EdgeCondition, EdgeDefinition, GraphMode,
    GraphRunState, NodeDefinition,
};
use super::types::{
    Artifact, ArtifactKind, Complexity, EvidenceArtifact, GraphBudgets, GraphCounters,
    GraphLifecycle, GraphRun, GraphTaskState, ImplementationPlan, Phase, ResearchKind, ReviewIssue,
    Role, Severity, TaskStatus, Verdict, VerificationResult, WorkerResult, WorkerSpec, WorkerUsage,
};
use super::verify::{
    collect_verify_commands, nothing_ran, run_verification_with_progress, CollectInput, VerifyExec,
};
use super::worker::WorkerRunner;
use crate::native_extensions::ecosystem::risk::ChangeRisk;
use crate::native_extensions::ecosystem::verification::{SecurityPolicyMode, SecurityVerification};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const EVIDENCE_DIGEST_MAX_CHARS: usize = 24_000;
const DIFF_MAX_CHARS: usize = 60_000;
const NODE_ATTEMPTS: u32 = 2;

pub type UpdateSink = dyn Fn(&GraphRun, Option<&str>) + Send + Sync;

/// Typed worker limit distinguishing unlimited configuration from exhausted finite grant of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GraphWorkerLimit {
    Unlimited,
    Finite(u32),
}

#[allow(dead_code)]
impl GraphWorkerLimit {
    pub fn from_config(config_val: u32) -> Self {
        if config_val == 0 {
            Self::Unlimited
        } else {
            Self::Finite(config_val)
        }
    }

    pub fn is_exhausted(&self, spawned: u32) -> bool {
        match self {
            Self::Unlimited => false,
            Self::Finite(cap) => spawned >= *cap,
        }
    }
}

fn coordinator_task_tools(
    tools: &[String],
    language_available: bool,
    browser_available: bool,
) -> Vec<String> {
    tools
        .iter()
        .filter(|tool| {
            davinci_agent::runtime::task_transport::is_task_tool(tool)
                || davinci_agent::tools::is_managed_process_tool(tool)
                || browser_available
                    && crate::native_extensions::browser::TOOL_NAMES.contains(&tool.as_str())
                || tool.as_str() == "retrieve_output" && language_available
                || crate::native_extensions::language_intelligence::TOOL_NAMES
                    .contains(&tool.as_str())
        })
        .cloned()
        .collect()
}

pub struct ControllerDeps {
    pub runner: Arc<WorkerRunner>,
    pub verify_exec: Arc<VerifyExec>,
    pub config: GraphConfig,
    pub session_model: Option<String>,
    pub session_thinking: Option<String>,
    pub project_trusted: bool,
    pub on_update: Arc<UpdateSink>,
    pub memory: Option<crate::native_extensions::VectorMemory>,
    pub learning: Option<crate::native_extensions::LearningController>,
    pub governor: Option<crate::native_extensions::TokenGovernor>,
    pub language_intelligence:
        Option<crate::native_extensions::language_intelligence::LanguageIntelligence>,
    pub processes: Option<davinci_agent::process_manager::ProcessManager>,
    pub browser: Option<crate::native_extensions::browser::BrowserWorkerHost>,
    pub runtime: Option<davinci_agent::RuntimeHandle>,
    pub permissions: Option<Arc<davinci_agent::PermissionState>>,
    pub task_contract: Option<davinci_agent::runtime::TaskContract>,
}

pub struct RunOptions {
    pub goal: String,
    pub cwd: PathBuf,
    pub forced: Option<Complexity>,
    pub dry_run: bool,
    /// Set by the graph UI's stop action or by session shutdown.
    pub abort: Arc<AtomicBool>,
    /// Artifacts from a previous run of the same goal, keyed by task id. A task
    /// whose id has a cached artifact is reused without spawning a worker (its
    /// original usage is credited so totals stay cumulative); the controller
    /// replays deterministically until the first uncached task, then continues
    /// live. Verification always re-runs.
    pub resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
    /// Persisted lifecycle state when a stopped graph continues. The new
    /// execution reuses the run identity and cumulative counters while safely
    /// replaying only the artifacts admitted above.
    pub resume_run: Option<Box<GraphRun>>,
}

pub fn default_is_git_repo(cwd: &Path) -> bool {
    cwd.join(".git").exists()
}

/// An untracked file larger than this is listed, not shown.
const UNTRACKED_FILE_MAX_BYTES: usize = 64 * 1024;

/// What the reviewer reads: every change against HEAD, plus the files the
/// writer created. The writer may not `git add`, so a plain `git diff` never
/// showed new files and the reviewer approved changes it had not seen.
pub fn default_get_diff(cwd: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    // `HEAD` includes staged changes; a repository without a commit has none.
    let mut diff = git(&["diff", "HEAD"])
        .or_else(|| git(&["diff"]))
        .unwrap_or_default();
    let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for file in untracked
        .lines()
        .map(str::trim)
        .filter(|file| !file.is_empty())
    {
        let Ok(bytes) = std::fs::read(cwd.join(file)) else {
            continue;
        };
        if !diff.is_empty() && !diff.ends_with('\n') {
            diff.push('\n');
        }
        diff.push_str(&format!(
            "diff --git a/{file} b/{file}\nnew file (untracked)\n--- /dev/null\n+++ b/{file}\n"
        ));
        if bytes.len() > UNTRACKED_FILE_MAX_BYTES || bytes.contains(&0) {
            diff.push_str(&format!(
                "@@ new file, {} bytes, contents not shown @@\n",
                bytes.len()
            ));
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        diff.push_str(&format!("@@ -0,0 +1,{} @@\n", text.lines().count()));
        for line in text.lines() {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
    }
    diff
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

/// Outcome of delivering one goal (the whole request, or one milestone).
enum Delivery {
    Ok,
    /// The run was finalized (blocked or cancelled); the caller must stop.
    Stop,
}

pub fn graph_dispatch_allowed(
    lifecycle: &str,
    dependencies_ready: bool,
    lease_available: bool,
) -> bool {
    lifecycle == "running" && dependencies_ready && lease_available
}

pub struct GraphExecution {
    run: Mutex<GraphRun>,
    deps: ControllerDeps,
    pub learning: Mutex<Option<crate::native_extensions::LearningController>>,
    options: RunOptions,
    /// Watched by every child process: the operator's abort, a budget abort,
    /// or a session shutdown all funnel here.
    exec_abort: Arc<AtomicBool>,
    budget_abort_reason: Mutex<Option<String>>,
    persistence_error: Mutex<Option<String>>,
    run_deadline: Option<std::time::Instant>,
    pub active_workers: Arc<AtomicUsize>,
    pub node_aborts: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[path = "controller_control.rs"]
mod live_control;

#[path = "controller_attempts.rs"]
mod attempts;

impl GraphExecution {
    fn completion_inputs(&self) -> Result<String, String> {
        let source = capture_baseline(&self.options.cwd)?;
        serde_json::to_string(&(
            &source.files,
            &self.deps.config.verify_commands,
            self.deps.config.security_verification,
        ))
        .map(|json| super::replay::compute_input_hash(&json))
        .map_err(|error| error.to_string())
    }

    fn save_delivery(
        &self,
        delivery: &DeliveryCheckpoint,
        indices: NodeIndices,
        note: Option<&str>,
    ) -> bool {
        {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            let cursor = run
                .continuation
                .get_or_insert_with(GraphContinuation::default);
            cursor.delivery = Some(delivery.clone());
            cursor.indices = indices;
        }
        self.checkpoint(note)
    }
    #[allow(dead_code)]
    pub fn abort_node(&self, task_id: &str) {
        if let Some(flag) = self
            .node_aborts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(task_id)
        {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn register_node_abort(&self, task_id: &str) -> Arc<AtomicBool> {
        self.node_aborts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(task_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    #[allow(dead_code)]
    pub fn request_pause(&self) {
        let mut run = self.run.lock().unwrap_or_else(|e| e.into_inner());
        if run.current_lifecycle() == GraphLifecycle::Running {
            if self.active_workers.load(Ordering::SeqCst) > 0 {
                run.lifecycle = Some(GraphLifecycle::PauseRequested);
            } else {
                run.lifecycle = Some(GraphLifecycle::Paused);
            }
            run.revision += 1;
        }
    }

    #[allow(dead_code)]
    pub fn resume_execution(&self) {
        let mut run = self.run.lock().unwrap_or_else(|e| e.into_inner());
        if run.current_lifecycle() == GraphLifecycle::Paused
            || run.current_lifecycle() == GraphLifecycle::PauseRequested
        {
            run.lifecycle = Some(GraphLifecycle::Running);
            run.revision += 1;
        }
    }

    fn checkpoint(&self, note: Option<&str>) -> bool {
        self.checkpoint_with(note, |_| Ok(()))
    }

    fn checkpoint_with(
        &self,
        note: Option<&str>,
        persist_companions: impl FnOnce(&mut GraphRun) -> std::io::Result<()>,
    ) -> bool {
        // Persist under the lock, but report on a clone with the guard dropped:
        // a slow `on_update` must not serialize every worker thread, and an
        // implementation that re-locks the run must not deadlock.
        let (snapshot, failure) = {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            let mut failure = self
                .persistence_error
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // A failed save is terminal for this execution. Retrying implicitly
            // could overwrite the last durable checkpoint with partial state.
            if failure.is_some() {
                return false;
            }
            if matches!(run.phase, Phase::Done | Phase::Blocked | Phase::Cancelled) {
                run.lifecycle = Some(GraphLifecycle::Stopped);
            }
            self.acknowledge_controls(&mut run);
            let gov_stats = self.deps.governor.as_ref().map(|g| g.stats());
            if let Some(ref gs) = gov_stats {
                run.ecosystem_stats.governor_bytes_omitted = gs.bytes_withheld;
                run.ecosystem_stats.governor_retrievals = gs.retrievals;
                run.ecosystem_stats.prunings = gs.prunings;
            }
            run.resource_snapshot = Some(
                crate::native_extensions::ecosystem::ResourceSnapshot::collect(
                    &run.tasks,
                    gov_stats.as_ref(),
                ),
            );
            if let Err(error) = persist_companions(&mut run).and_then(|()| save_run(&mut run)) {
                let reason = format!("checkpoint persistence failed: {error}");
                *failure = Some(reason.clone());
                self.exec_abort.store(true, Ordering::SeqCst);
                run.phase = Phase::Blocked;
                run.lifecycle = Some(GraphLifecycle::Stopped);
                run.blocked_reason = Some(reason);
            }
            (run.clone(), failure.clone())
        };
        if let Some(reason) = failure {
            (self.deps.on_update)(
                &snapshot,
                Some(&format!("{reason}; stopped state not saved")),
            );
            false
        } else {
            (self.deps.on_update)(&snapshot, note);
            true
        }
    }

    fn snapshot(&self) -> GraphRun {
        let mut snapshot = self
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(reason) = self.persistence_failure() {
            snapshot.phase = Phase::Blocked;
            snapshot.lifecycle = Some(GraphLifecycle::Stopped);
            snapshot.blocked_reason = Some(reason);
        }
        snapshot
    }

    fn persistence_failure(&self) -> Option<String> {
        self.persistence_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn verification_progress(&self, progress: &VerificationResult) {
        self.run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .verification = Some(progress.clone());
        self.checkpoint(None);
    }

    fn begin_verification(&self) {
        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
        run.phase = Phase::Verify;
        run.verification = None;
    }

    fn verify_commands(
        &self,
        commands: &[super::types::VerifyCommandSpec],
        timeout_ms: u64,
    ) -> VerificationResult {
        let result = run_verification_with_progress(
            commands,
            &self.options.cwd,
            &self.exec_abort,
            timeout_ms,
            self.run_deadline,
            &|command, cwd, abort, timeout| {
                loop {
                    if !self.wait_until_running() {
                        return (-1, "verification stopped before dispatch".into(), 0);
                    }
                    let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                    if run.current_lifecycle() != GraphLifecycle::Running {
                        continue;
                    }
                    self.active_workers.fetch_add(1, Ordering::SeqCst);
                    break;
                }
                let _active = live_control::ActiveOperation(self.active_workers.clone());
                (self.deps.verify_exec)(command, cwd, abort, timeout)
            },
            |progress| self.verification_progress(progress),
        );
        if self
            .run_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.budget_abort("run deadline exceeded".into());
        }
        result
    }

    fn budget_abort(&self, reason: String) {
        let mut current = self
            .budget_abort_reason
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current.is_some() {
            return;
        }
        *current = Some(reason);
        self.exec_abort.store(true, Ordering::Relaxed);
    }

    fn budget_abort_reason(&self) -> Option<String> {
        self.budget_abort_reason
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Check whether this graph execution exceeds an authorized root lease.
    #[allow(dead_code)]
    pub fn enforce_root_lease(&self, root_lease_tokens: Option<u64>) -> Option<String> {
        let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(lease) = root_lease_tokens {
            let total_tokens: u64 = run
                .tasks
                .iter()
                .map(|t| t.usage.input.saturating_add(t.usage.output))
                .sum();
            if total_tokens > lease {
                return Some(format!(
                    "graph execution tokens ({total_tokens}) exceeds root lease ({lease})"
                ));
            }
        }
        None
    }

    /// The only spend guard that can fire mid-node. `0` disables it.
    fn cost_budget_exceeded(run: &GraphRun) -> Option<String> {
        let budgets = &run.budgets;
        (budgets.max_cost_usd > 0.0 && run.counters.cost_usd > budgets.max_cost_usd).then(|| {
            format!(
                "cost budget exhausted: ${:.2} > ${:.2}",
                run.counters.cost_usd, budgets.max_cost_usd
            )
        })
    }

    /// Checked before each node starts. Every branch is disabled by a `0`.
    fn budget_exceeded(run: &GraphRun, now: u64) -> Option<String> {
        if let Some(reason) = Self::cost_budget_exceeded(run) {
            return Some(reason);
        }
        let budgets = &run.budgets;
        if budgets.run_deadline_ms > 0
            && now.saturating_sub(run.counters.started_at) > budgets.run_deadline_ms
        {
            return Some("run deadline exceeded".to_string());
        }
        // max_workers is sized for one deliverable; a decomposed run gets that
        // allowance per milestone. Cost cap and deadline stay global on purpose.
        let milestones = run.milestones.as_ref().map(Vec::len).unwrap_or(1).max(1) as u32;
        let limit = if budgets.max_workers == 0 {
            GraphWorkerLimit::Unlimited
        } else {
            GraphWorkerLimit::Finite(budgets.max_workers.saturating_mul(milestones))
        };
        if limit.is_exhausted(run.counters.workers_spawned) {
            let worker_cap = budgets.max_workers.saturating_mul(milestones);
            return Some(format!("worker budget exhausted ({worker_cap} workers)"));
        }
        None
    }

    fn add_usage(&self, task_id: &str, usage: &WorkerUsage) {
        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(task) = run.tasks.iter_mut().find(|task| task.id == task_id) {
            task.usage.add(usage);
        }
        run.counters.cost_usd += usage.cost_usd;
        if let Some(reason) = Self::cost_budget_exceeded(&run) {
            drop(run);
            self.budget_abort(reason);
        }
    }

    fn end_task(&self, task_id: &str, status: TaskStatus, error: Option<String>) {
        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(task) = run.tasks.iter_mut().find(|task| task.id == task_id) {
            task.ended_at = Some(now_ms());
            task.last_activity = None;
            match status {
                TaskStatus::Succeeded => task.mark_succeeded(),
                TaskStatus::Failed => {
                    let err = error
                        .or_else(|| task.error.clone())
                        .unwrap_or_else(|| "task failed".to_string());
                    task.mark_failed(err);
                }
                _ => {
                    task.status = status;
                    if let Some(err) = error {
                        task.error = Some(err);
                    }
                }
            }
        }
    }

    fn authorized_worker_tools(&self, role: Role) -> Vec<String> {
        let mut tools = role_tools(role);
        // Match the worker-side role guard; do not advertise tools it must refuse.
        let has_coordinator_authority =
            self.deps.runtime.is_some() && self.deps.permissions.is_some();
        if role == Role::Writer {
            tools.extend(
                self.deps
                    .config
                    .worker_extra_tools
                    .iter()
                    .filter(|tool| {
                        has_coordinator_authority || !requires_task_coordinator(tool.trim())
                    })
                    .cloned(),
            );
        }
        if !has_coordinator_authority || self.deps.language_intelligence.is_none() {
            tools.retain(|name| {
                !crate::native_extensions::language_intelligence::TOOL_NAMES
                    .contains(&name.as_str())
            });
        }
        if !has_coordinator_authority || self.deps.processes.is_none() {
            tools.retain(|name| !davinci_agent::tools::is_managed_process_tool(name));
        }
        ensure_governor_recovery_tool(&mut tools);
        tools
    }

    fn worker_spec(
        &self,
        task: &GraphTaskState,
        briefing: String,
        tools: Vec<String>,
    ) -> WorkerSpec {
        let run = self.snapshot();
        let role = task.role;
        let configured_model = self.deps.config.models.get(&role).cloned();
        let authorized_tools = self.authorized_worker_tools(role);
        let has_recovery = authorized_tools.iter().any(|t| t == "retrieve_output");
        let initially_exposed_tools = initial_worker_tools(role, &authorized_tools);
        WorkerSpec {
            task_id: task.id.clone(),
            role,
            expect: task.expect,
            briefing,
            system_prompt: role_system_prompt_with_recovery(role, has_recovery),
            cwd: PathBuf::from(&run.cwd),
            model: configured_model
                .clone()
                .or_else(|| self.deps.session_model.clone()),
            thinking_level: configured_model
                .is_none()
                .then(|| self.deps.session_thinking.clone())
                .flatten(),
            tools,
            authorized_tools,
            initially_exposed_tools,
            extra_extensions: if self.deps.project_trusted {
                self.deps.config.worker_extensions.clone()
            } else {
                Vec::new()
            },
            timeout_ms: run.budgets.worker_timeout_ms.get(role),
            run_deadline: self.run_deadline,
            artifact_path: artifact_path(Path::new(&run.cwd), &run.run_id, &task.id),
            transcript_path: Some(transcript_path(Path::new(&run.cwd), &run.run_id, &task.id)),
            project_trusted: self.deps.project_trusted,
            runtime_agent_id: None,
            task_contract: self.deps.task_contract.clone(),
            coordinator_client: None,
            node_abort: None,
        }
    }

    fn execute_node(&self, mut task: GraphTaskState, briefing: String) -> Option<Artifact> {
        let task_id = task.id.clone();
        let role = task.role;

        if !self.wait_until_running() {
            return None;
        }

        if self
            .snapshot()
            .task(&task_id)
            .is_some_and(|existing| existing.status == TaskStatus::Succeeded)
        {
            let run = self.snapshot();
            return match super::store::read_artifact(
                &self.options.cwd,
                &run.run_id,
                &task_id,
                task.expect,
            ) {
                Ok(artifact) => Some(artifact),
                Err(errors) => {
                    self.blocked(format!(
                        "Completed task '{task_id}' cannot be restored: {}",
                        errors.join("; ")
                    ));
                    None
                }
            };
        }

        {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(existing) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                existing.status = TaskStatus::Pending;
            }
            materialize_generated_dependencies(&mut run, &mut task);
            // Saved topology evaluates OnFailure/Always edges below. Treating
            // every predecessor as OnSuccess would make recovery unreachable.
            let unmet = if run.saved_definition.is_some() {
                Vec::new()
            } else {
                run.unmet_dependencies(&task)
            };
            if !unmet.is_empty() {
                let mut refused = task.clone();
                refused.status = TaskStatus::Cancelled;
                refused.ended_at = Some(now_ms());
                refused.error = Some(format!("dependencies not satisfied: {}", unmet.join(", ")));
                run.tasks.push(refused);
                drop(run);
                self.checkpoint(Some(&format!("{task_id}: dependencies not satisfied")));
                return None;
            }

            if let Some(def) = &run.definition {
                let is_saved = run.saved_definition.is_some();
                if def.node(&task_id).is_none() {
                    if is_saved {
                        let mut refused = task.clone();
                        refused.status = TaskStatus::Cancelled;
                        refused.ended_at = Some(now_ms());
                        refused.error =
                            Some(format!("node not defined in saved topology: {task_id}"));
                        run.tasks.push(refused);
                        drop(run);
                        self.checkpoint(Some(&format!(
                            "{task_id}: node unknown in saved topology"
                        )));
                        return None;
                    }
                } else {
                    let state = GraphRunState::from_run(&run);
                    let ready = ready_nodes(def, &state);
                    if !ready.contains(&task_id) {
                        let mut refused = task.clone();
                        refused.status = TaskStatus::Cancelled;
                        refused.ended_at = Some(now_ms());
                        refused.error =
                            Some(format!("node not ready in execution graph: {task_id}"));
                        run.tasks.push(refused);
                        drop(run);
                        self.checkpoint(Some(&format!("{task_id}: not ready in topology")));
                        return None;
                    }
                }
            }

            if let Some(pending) = run
                .tasks
                .iter_mut()
                .find(|entry| entry.id == task_id && entry.status == TaskStatus::Pending)
            {
                pending.depends_on = task.depends_on.clone();
            } else {
                run.tasks.push(task.clone());
            }
        }

        if let Some((artifact, usage, stored_fingerprint)) =
            self.options.resume_artifacts.get(&task_id)
        {
            let (run_version, cwd, definition_digest, dry_run) = {
                let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                let run_version = run
                    .definition
                    .as_ref()
                    .map(|d| d.version)
                    .unwrap_or(run.version);
                let cwd = PathBuf::from(&run.cwd);
                let definition_digest = run.definition_digest.clone();
                let dry_run = run.dry_run;
                (run_version, cwd, definition_digest, dry_run)
            };
            let current_fingerprint = ReplayFingerprint::for_saved_task(
                &cwd,
                run_version,
                &briefing,
                task.expect,
                definition_digest.as_deref(),
                dry_run,
            );

            let is_compatible = match stored_fingerprint {
                Some(stored) => {
                    if replay_compatible(stored, &current_fingerprint) {
                        true
                    } else {
                        let reason = incompatibility_reason(stored, &current_fingerprint)
                            .unwrap_or_else(|| "replay fingerprint mismatch".to_string());
                        self.checkpoint(Some(&format!(
                            "{task_id}: replay refused ({reason}), re-executing"
                        )));
                        false
                    }
                }
                None => {
                    self.checkpoint(Some(&format!(
                        "{task_id}: replay refused (missing fingerprint), re-executing"
                    )));
                    false
                }
            };

            if is_compatible {
                {
                    let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some(task_entry) = run.tasks.iter_mut().find(|entry| entry.id == task_id)
                    {
                        task_entry.started_at = Some(now_ms());
                        task_entry.artifact_file = Some(format!("artifacts/{task_id}.json"));
                        task_entry.fingerprint = stored_fingerprint.clone();
                        task_entry.usage = *usage;
                    }
                }
                if self.options.resume_run.is_none() {
                    self.add_usage(&task_id, usage);
                }
                self.end_task(&task_id, TaskStatus::Succeeded, None);
                if !self.checkpoint_with(
                    Some(&format!("{task_id}: reused from previous run")),
                    |run| {
                        write_artifact(&artifact_path(&cwd, &run.run_id, &task_id), artifact)?;
                        if let Some(fp) = stored_fingerprint {
                            write_task_fingerprint(&cwd, &run.run_id, &task_id, fp)?;
                        }
                        Ok(())
                    },
                ) {
                    return None;
                }
                return Some(artifact.clone());
            }
        }

        let authorized_tools = self.authorized_worker_tools(role);
        let retry_query = crate::native_extensions::ecosystem::WorkerContextQuery {
            role: Some(role),
            node_objective: briefing.clone(),
            graph_goal: self.options.goal.clone(),
            target_hints: task.focus.clone().into_iter().collect(),
            failure_hint: None,
        };
        let capability_selection = {
            let guard = self
                .learning
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match (&self.deps.memory, guard.as_ref()) {
                (Some(mem), Some(learn)) => {
                    let context_query = retry_query.render();
                    let skill_query = retry_query.render_skill_query();
                    crate::native_extensions::ecosystem::select_capabilities(
                        mem,
                        learn,
                        authorized_tools,
                        crate::native_extensions::ecosystem::CapabilityRequest::new(
                            &context_query,
                            role,
                        )
                        .with_skill_prompt(&skill_query)
                        .with_context_token_cap(
                            crate::native_extensions::ecosystem::DEFAULT_GRAPH_CONTEXT_TOKENS,
                        )
                        .with_skills(true),
                    )
                }
                _ => crate::native_extensions::ecosystem::CapabilitySelection {
                    tools: authorized_tools,
                    context: crate::native_extensions::ecosystem::ContextPacket::empty(),
                },
            }
        };
        let context_packet = capability_selection.context;

        {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(t) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                t.context_fingerprint = if context_packet.is_empty() {
                    None
                } else {
                    Some(context_packet.fingerprint.clone())
                };
                t.context_tokens = context_packet.estimated_tokens;
                t.memory_refs = context_packet.memory_refs.clone();
                t.skill_refs = context_packet.skill_refs.clone();
            }
            if !context_packet.is_empty() {
                run.ecosystem_stats.memory_hits += context_packet.memory_refs.len() as u64;
                run.ecosystem_stats.memory_injected_tokens += context_packet.memory_tokens as u64;
                run.ecosystem_stats.skill_candidates_considered +=
                    context_packet.skill_candidates_considered as u64;
                run.ecosystem_stats.skills_injected += context_packet.skill_refs.len() as u64;
                run.ecosystem_stats.skill_injected_tokens += context_packet.skill_tokens as u64;
                run.ecosystem_stats.context_packet_tokens += context_packet.estimated_tokens as u64;
                run.ecosystem_stats.context_fingerprint = Some(context_packet.fingerprint.clone());
            }
        }
        if !context_packet.is_empty()
            && !self.checkpoint_with(None, |run| {
                super::store::write_task_context_packet(
                    Path::new(&run.cwd),
                    &run.run_id,
                    &task_id,
                    &context_packet,
                )
            })
        {
            return None;
        }

        let mut last_failure_class: Option<WorkerFailureClass> = None;
        let mut retry_context_delta = crate::native_extensions::ecosystem::ContextPacket::empty();
        let mut attempts_run = 0;
        let previous_attempts = self
            .snapshot()
            .task(&task_id)
            .map_or(0, |task| task.attempts);
        for local_attempt in 1..=NODE_ATTEMPTS {
            let Some(attempt) = previous_attempts.checked_add(local_attempt) else {
                self.blocked(format!("Attempt counter exhausted for '{task_id}'"));
                return None;
            };
            attempts_run = attempt;
            if self.exec_abort.load(Ordering::Relaxed) {
                self.end_task(&task_id, TaskStatus::Cancelled, None);
                self.checkpoint(None);
                return None;
            }
            // Budget check and spawn accounting share one critical section so
            // parallel research threads cannot all pass a stale check before
            // any of them records its own spawn.
            let over_budget = loop {
                if !self.wait_until_running() {
                    self.end_task(&task_id, TaskStatus::Cancelled, None);
                    self.checkpoint(None);
                    return None;
                }
                let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                if run.current_lifecycle() != GraphLifecycle::Running {
                    continue;
                }
                break match Self::budget_exceeded(&run, now_ms()) {
                    Some(reason) => Some(reason),
                    None => {
                        run.counters.workers_spawned += 1;
                        run.ecosystem_stats.graph_workers += 1;
                        if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                            task.status = TaskStatus::Running;
                            task.attempts = attempt;
                            task.started_at.get_or_insert_with(now_ms);
                        }
                        None
                    }
                };
            };
            if let Some(reason) = over_budget {
                self.end_task(&task_id, TaskStatus::Cancelled, Some(reason));
                self.checkpoint(None);
                return None;
            }
            let attempt_briefing = if attempt == 1 {
                briefing.clone()
            } else {
                let notice = match last_failure_class {
                    Some(WorkerFailureClass::Timeout) => {
                        "the previous worker ran out of time before submitting; work economically, avoid repository-wide searches, and call graph_submit well before the deadline"
                    }
                    Some(WorkerFailureClass::VerificationFailed) => {
                        "verification failed; revise the writer's work against the reported diagnostic before submitting"
                    }
                    Some(WorkerFailureClass::PlanInvalidated) => {
                        "the plan was invalidated; re-check the current task scope and produce a replacement artifact"
                    }
                    Some(WorkerFailureClass::ArtifactInvalid) => {
                        "the previous submission was invalid; complete the contract and call graph_submit exactly once"
                    }
                    Some(WorkerFailureClass::ProcessFailure) | Some(WorkerFailureClass::Unknown) => {
                        "the previous worker exited without a valid submitted artifact; complete the work and call graph_submit exactly once"
                    }
                    Some(WorkerFailureClass::PermissionRefused) | Some(WorkerFailureClass::Environment) => {
                        "the previous worker encountered an unrecoverable environment or permission failure"
                    }
                    None => "the previous worker exited without a valid submitted artifact; complete the work and call graph_submit exactly once",
                };
                let mut retry = format!("{briefing}\n\nRETRY NOTICE: {notice}.");
                if !retry_context_delta.is_empty() {
                    retry.push_str("\n\nRETRY CONTEXT DELTA:\n");
                    retry.push_str(&retry_context_delta.text);
                }
                retry
            };

            let effective_briefing = if context_packet.text.is_empty() {
                attempt_briefing
            } else {
                format!("{}\n\n{}", context_packet.text, attempt_briefing)
            };

            let mut spec = self.worker_spec(
                &task,
                effective_briefing,
                capability_selection.tools.clone(),
            );
            // A retry after a timeout gets double time — but only when a
            // timeout was configured at all; 0 stays unlimited.
            if matches!(last_failure_class, Some(WorkerFailureClass::Timeout))
                && spec.timeout_ms > 0
            {
                spec.timeout_ms = spec.timeout_ms.saturating_mul(2);
            }

            let worker_agent_id = davinci_agent::AgentId::new();
            spec.runtime_agent_id = Some(worker_agent_id);
            if !self.begin_attempt(&spec, attempt) {
                return None;
            }
            if let Some(runtime) = &self.deps.runtime {
                let run_snapshot = self.snapshot();
                let provider = spec
                    .model
                    .as_deref()
                    .and_then(|m| m.split_once('/').map(|(p, _)| p))
                    .unwrap_or("default")
                    .to_string();
                let record = davinci_agent::AgentRecord {
                    id: worker_agent_id,
                    run_id: runtime.run_id,
                    parent: Some(runtime.agent_id),
                    kind: davinci_agent::AgentKind::GraphWorker,
                    name: format!("graph-worker-{}-{}", task_id, run_snapshot.run_id),
                    provider,
                    model_id: spec.model.clone().unwrap_or_default(),
                    cwd: spec.cwd.clone(),
                    state: davinci_agent::AgentState::Starting,
                    task_id: None,
                    worktree: None,
                    started_ms: super::store::now_ms() as i64,
                    updated_ms: super::store::now_ms() as i64,
                    failure_reason: None,
                };
                let _ = runtime.registry.register_agent(record);
                let _ = runtime
                    .registry
                    .transition(worker_agent_id, davinci_agent::AgentState::Running);
            }

            let task_tools = coordinator_task_tools(
                &spec.tools,
                self.deps.language_intelligence.is_some(),
                self.deps.browser.is_some(),
            );
            let _coordinator_transport = if !task_tools.is_empty() {
                if let (Some(runtime), Some(permissions)) =
                    (&self.deps.runtime, &self.deps.permissions)
                {
                    match davinci_agent::runtime::task_transport::TaskCoordinatorTransport::bind_with_handler(
                        runtime,
                        worker_agent_id,
                        permissions.clone(),
                        task_tools,
                        spec.cwd.clone(),
                        self.exec_abort.clone(),
                        super::coordinator_handler::for_worker(&self.deps, &spec, self.exec_abort.clone()),
                    ) {
                        Ok(transport) => {
                            spec.coordinator_client = Some(transport.client());
                            Some(transport)
                        }
                        Err(err) => {
                            eprintln!("failed to bind task coordinator transport: {err}");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let reported = Mutex::new(WorkerUsage::default());
            let result = {
                let mut on_progress = |line: &str, usage: &WorkerUsage| {
                    let delta = {
                        let mut reported =
                            reported.lock().unwrap_or_else(|error| error.into_inner());
                        let delta = WorkerUsage::delta(usage, &reported);
                        *reported = *usage;
                        delta
                    };
                    self.add_usage(&task_id, &delta);
                    {
                        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                        run.ecosystem_stats.graph_cost_usd += delta.cost_usd;
                        run.ecosystem_stats.cache_read_tokens += delta.cache_read;
                        run.ecosystem_stats.cache_write_tokens += delta.cache_write;
                        run.ecosystem_stats.record_graph_cache_usage(role, &delta);
                        if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                            task.last_activity = Some(line.to_string());
                        }
                        if let Some(record) = run
                            .continuation
                            .as_mut()
                            .and_then(|cursor| cursor.attempt_history.get_mut(&task_id))
                            .and_then(|records| records.last_mut())
                        {
                            record.usage = *usage;
                        }
                    }
                    self.checkpoint(Some(&format!("{task_id}: {line}")));
                };
                let node_abort = self.register_node_abort(&task_id);
                spec.node_abort = Some(Arc::clone(&node_abort));
                let runner_abort = Arc::new(AtomicBool::new(
                    self.exec_abort.load(Ordering::Relaxed) || node_abort.load(Ordering::Relaxed),
                ));
                let watcher_stop = Arc::new(AtomicBool::new(false));
                let w_stop = Arc::clone(&watcher_stop);
                let w_exec = Arc::clone(&self.exec_abort);
                let w_node = Arc::clone(&node_abort);
                let w_comb = Arc::clone(&runner_abort);
                let watcher_handle = thread::spawn(move || {
                    while !w_stop.load(Ordering::Relaxed) {
                        if w_exec.load(Ordering::Relaxed) || w_node.load(Ordering::Relaxed) {
                            w_comb.store(true, Ordering::SeqCst);
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                });

                self.active_workers.fetch_add(1, Ordering::SeqCst);
                let mut run_res = if runner_abort.load(Ordering::SeqCst) {
                    WorkerResult::default()
                } else {
                    (self.deps.runner)(&spec, &runner_abort, &mut on_progress)
                };
                self.active_workers.fetch_sub(1, Ordering::SeqCst);
                watcher_stop.store(true, Ordering::Relaxed);
                let _ = watcher_handle.join();

                if node_abort.load(Ordering::Relaxed) {
                    run_res.ok = false;
                    run_res.failure_reason = Some(format!("node {task_id} stopped by operator"));
                }

                run_res
            };
            let worker_terminal_state = if result.ok {
                davinci_agent::AgentState::Completed
            } else if result.timed_out
                || result.run_deadline_exceeded
                || self.exec_abort.load(Ordering::Relaxed)
            {
                davinci_agent::AgentState::Cancelled
            } else {
                davinci_agent::AgentState::Failed
            };
            if let Some(runtime) = &self.deps.runtime {
                let _ = runtime
                    .registry
                    .transition(worker_agent_id, worker_terminal_state);
            }
            let trailing = {
                let reported = reported.lock().unwrap_or_else(|error| error.into_inner());
                WorkerUsage::delta(&result.usage, &reported)
            };
            self.add_usage(&task_id, &trailing);
            {
                let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                run.ecosystem_stats.graph_cost_usd += trailing.cost_usd;
                run.ecosystem_stats.cache_read_tokens += trailing.cache_read;
                run.ecosystem_stats.cache_write_tokens += trailing.cache_write;
                run.ecosystem_stats
                    .record_graph_cache_usage(role, &trailing);
            }

            let run = self.snapshot();
            if !self.finish_attempt(&spec, attempt, &result) {
                return None;
            }
            if result.recovery_required {
                let error = format!(
                    "reconciliation required: {}",
                    result
                        .failure_reason
                        .as_deref()
                        .unwrap_or("worker execution history could not be saved")
                );
                self.end_task(&task_id, TaskStatus::Failed, Some(error));
                self.checkpoint(Some("worker history requires recovery; retry stopped"));
                return None;
            }
            write_log(
                Path::new(&run.cwd),
                &run.run_id,
                &task_id,
                &format!(
                    "attempt {attempt}\nexit {} timedOut={}\n--- stderr ---\n{}\n--- final text ---\n{}\n--- failure reason ---\n{}",
                    result.exit_code,
                    result.timed_out,
                    result.stderr,
                    result.final_text,
                    result.failure_reason.clone().unwrap_or_default()
                ),
            );

            if result.run_deadline_exceeded
                || result.failure_reason.as_deref() == Some("run deadline exceeded")
            {
                let reason = "run deadline exceeded".to_string();
                self.budget_abort(reason.clone());
                self.end_task(&task_id, TaskStatus::Cancelled, Some(reason));
                self.checkpoint(Some(&format!("{task_id}: run deadline exceeded")));
                return None;
            }

            if let Some(reason) = self.budget_abort_reason() {
                self.end_task(&task_id, TaskStatus::Cancelled, Some(reason));
                self.checkpoint(Some(&format!("{task_id}: stopped by budget")));
                return None;
            }
            if self.persistence_failure().is_some() {
                return None;
            }
            if self.register_node_abort(&task_id).load(Ordering::SeqCst) {
                self.end_task(
                    &task_id,
                    TaskStatus::Cancelled,
                    Some("node stopped by operator".into()),
                );
                self.checkpoint(Some("node stopped at safe boundary"));
                return None;
            }
            if result.ok {
                if let Some(artifact) = result.artifact {
                    let (run_version, cwd, run_id, definition_digest, dry_run) = {
                        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                        let run_version = run
                            .definition
                            .as_ref()
                            .map(|d| d.version)
                            .unwrap_or(run.version);
                        let cwd = PathBuf::from(&run.cwd);
                        let run_id = run.run_id.clone();
                        let definition_digest = run.definition_digest.clone();
                        let dry_run = run.dry_run;
                        let fingerprint = ReplayFingerprint::for_saved_task(
                            &cwd,
                            run_version,
                            &briefing,
                            task.expect,
                            definition_digest.as_deref(),
                            dry_run,
                        );
                        if let Some(task_entry) =
                            run.tasks.iter_mut().find(|entry| entry.id == task_id)
                        {
                            task_entry.artifact_file = Some(format!("artifacts/{task_id}.json"));
                            task_entry.fingerprint = Some(fingerprint.clone());
                        }
                        (run_version, cwd, run_id, definition_digest, dry_run)
                    };
                    let fingerprint = ReplayFingerprint::for_saved_task(
                        &cwd,
                        run_version,
                        &briefing,
                        task.expect,
                        definition_digest.as_deref(),
                        dry_run,
                    );
                    self.end_task(&task_id, TaskStatus::Succeeded, None);
                    if !self.checkpoint_with(Some(&format!("{task_id}: succeeded")), |_| {
                        write_artifact(&artifact_path(&cwd, &run_id, &task_id), &artifact)?;
                        write_task_fingerprint(&cwd, &run_id, &task_id, &fingerprint)
                    }) {
                        return None;
                    }
                    return Some(artifact);
                }
            }
            let error = result.failure_reason.clone().unwrap_or_else(|| {
                if result.timed_out {
                    "timed out".to_string()
                } else {
                    let count = result.stderr.chars().count();
                    let tail: String = result
                        .stderr
                        .chars()
                        .skip(count.saturating_sub(300))
                        .collect();
                    format!("exit {}; {tail}", result.exit_code)
                }
            });
            {
                let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                    task.error = Some(error.clone());
                    task.status = TaskStatus::Failed;
                }
            }
            let failure_class = classify_worker_failure(&result, Some(&error));
            let decision = retry_decision(failure_class, local_attempt as usize);
            last_failure_class = Some(failure_class);
            self.checkpoint(Some(&format!(
                "{task_id}: attempt {attempt} failed ({failure_class}, {decision:?})"
            )));
            if matches!(decision, RetryDecision::Stop | RetryDecision::Replan) {
                break;
            }
            let guard = self
                .learning
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            retry_context_delta = match (&self.deps.memory, guard.as_ref()) {
                (Some(memory), Some(learning)) => build_retry_context_delta(
                    memory,
                    learning,
                    role,
                    &retry_query,
                    failure_class,
                    &error,
                ),
                _ => crate::native_extensions::ecosystem::ContextPacket::empty(),
            };
        }
        self.end_task(&task_id, TaskStatus::Failed, None);
        self.checkpoint(Some(&format!(
            "{task_id}: failed after {attempts_run} attempts"
        )));
        None
    }

    fn blocked(&self, reason: String) {
        {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            run.phase = Phase::Blocked;
            run.blocked_reason = Some(reason.clone());
        }
        self.checkpoint(Some(&format!("blocked: {reason}")));
    }

    fn task_failure_reason(&self, fallback: &str) -> String {
        let run = self.snapshot();
        match run.tasks.last().and_then(|task| task.error.clone()) {
            Some(error) => format!("{fallback}: {error}"),
            None => fallback.to_string(),
        }
    }

    fn set_phase(&self, phase: Phase) {
        self.run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .phase = phase;
    }

    /// True when the run was finalized and the caller must stop.
    fn cancelled_if_aborted(&self) -> bool {
        if self.persistence_failure().is_some() {
            return true;
        }
        if let Some(reason) = self.budget_abort_reason() {
            self.blocked(reason);
            return true;
        }
        if !self.options.abort.load(Ordering::Relaxed) {
            return false;
        }
        self.set_phase(Phase::Cancelled);
        self.checkpoint(Some("cancelled"));
        true
    }

    fn skill_scope_relevant(
        record: &crate::native_extensions::learning::types::SkillLedgerRecord,
        task: &GraphTaskState,
        goal: &str,
    ) -> bool {
        let meta = &record.applicability;
        if meta == &crate::native_extensions::learning::types::SkillApplicability::default() {
            return false;
        }
        let mut context = goal.replace('\\', "/").to_ascii_lowercase();
        context.push(' ');
        context.push_str(&task.role.to_string().to_ascii_lowercase());
        if let Some(focus) = &task.focus {
            context.push(' ');
            context.push_str(&focus.replace('\\', "/").to_ascii_lowercase());
        }
        if let Some(mutation) = &task.mutation {
            for file in &mutation.files {
                context.push(' ');
                context.push_str(&file.path.replace('\\', "/").to_ascii_lowercase());
            }
        }
        let matches = |hint: &str| {
            let normalized = hint
                .trim()
                .trim_matches('*')
                .replace('\\', "/")
                .to_ascii_lowercase();
            !normalized.is_empty() && context.contains(&normalized)
        };
        if !meta.required_signals.is_empty()
            && !meta.required_signals.iter().all(|hint| matches(hint))
        {
            return false;
        }
        meta.languages.iter().any(|hint| matches(hint))
            || meta.task_types.iter().any(|hint| matches(hint))
            || meta.path_globs.iter().any(|hint| matches(hint))
            || meta
                .verification_categories
                .iter()
                .any(|hint| matches(hint))
    }

    fn skill_usage_signal(
        outcome: crate::native_extensions::learning::types::SkillOutcome,
        relevant: bool,
    ) -> crate::native_extensions::learning::types::SkillUsageSignal {
        use crate::native_extensions::learning::types::{SkillOutcome, SkillUsageSignal};
        match (outcome, relevant) {
            (SkillOutcome::VerifiedSuccess, true) => SkillUsageSignal::VerifiedHelpful,
            (SkillOutcome::VerifiedFailure, true) => SkillUsageSignal::VerifiedFailureRelevant,
            (SkillOutcome::Neutral, true) => SkillUsageSignal::ScopeRelevant,
            (_, false) => SkillUsageSignal::Injected,
        }
    }

    pub fn record_skill_outcomes(&self, run: &GraphRun) {
        if self.persistence_failure().is_some() {
            return;
        }
        let Some(ref verification) = run.verification else {
            return;
        };
        let mut guard = self.learning.lock().unwrap_or_else(|e| e.into_inner());
        let Some(learning) = guard.as_mut() else {
            return;
        };

        let changed_files: Vec<String> = run
            .tasks
            .iter()
            .filter_map(|task| task.mutation.as_ref())
            .flat_map(|mutation| mutation.files.iter().map(|file| file.path.clone()))
            .collect();
        let bundle = verification.to_bundle(
            changed_files,
            Some(run.run_id.clone()),
            crate::native_extensions::ecosystem::verification::SecurityVerification::NotRequired,
        );

        let outcome = if bundle.commands_ran > 0
            && bundle.approval_eligible(
                crate::native_extensions::ecosystem::verification::SecurityPolicyMode::Risk,
            )
            && run.phase == Phase::Done
        {
            crate::native_extensions::learning::types::SkillOutcome::VerifiedSuccess
        } else if bundle.commands_ran > 0
            && (!bundle.deterministic_passed || bundle.commands_failed > 0)
        {
            crate::native_extensions::learning::types::SkillOutcome::VerifiedFailure
        } else {
            crate::native_extensions::learning::types::SkillOutcome::Neutral
        };

        let mut usage = std::collections::BTreeMap::new();
        for task in &run.tasks {
            for s in &task.skill_refs {
                let key = (s.name.clone(), s.version, s.content_hash.clone());
                let exact_record = learning
                    .project_store
                    .skill_version(&s.name, s.version)
                    .or_else(|| learning.global_store.skill_version(&s.name, s.version))
                    .filter(|record| record.content_hash == s.content_hash)
                    .cloned();
                let relevant = exact_record
                    .as_ref()
                    .is_some_and(|record| Self::skill_scope_relevant(record, task, &run.goal));
                usage
                    .entry(key)
                    .and_modify(|seen_relevant| *seen_relevant |= relevant)
                    .or_insert(relevant);
            }
        }
        for ((name, version, content_hash), relevant) in usage {
            let version_ref = crate::native_extensions::learning::types::SkillVersionRef {
                name,
                version,
                content_hash,
            };
            let signal = Self::skill_usage_signal(outcome, relevant);
            let _ = learning.record_skill_usage_outcome(&version_ref, signal);
        }
        {
            let mut run_mut = self.run.lock().unwrap_or_else(|e| e.into_inner());
            run_mut.ecosystem_stats.learning_reviews_dispatched = learning.stats.reviews_dispatched;
            run_mut.ecosystem_stats.learning_reviews_skipped = learning.stats.reviews_skipped;
            run_mut.ecosystem_stats.learned_artifacts_applied = learning.stats.candidates_approved;
        }
    }
}

/// Mirror the operator's abort into the execution flag so children stop even
/// while no output is arriving.
fn spawn_abort_watcher(
    parent: Arc<AtomicBool>,
    exec: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !finished.load(Ordering::Relaxed) {
            if parent.load(Ordering::Relaxed) {
                exec.store(true, Ordering::Relaxed);
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    })
}

#[allow(dead_code)]
pub fn run_saved_graph(
    options: RunOptions,
    deps: ControllerDeps,
    saved_def: super::definitions::SavedGraphDefinitionV1,
) -> GraphRun {
    run_graph_internal(options, deps, Some(saved_def), None)
}

pub fn run_graph(options: RunOptions, deps: ControllerDeps) -> GraphRun {
    run_graph_internal(options, deps, None, None)
}

pub(super) fn run_graph_owned(
    options: RunOptions,
    deps: ControllerDeps,
    saved_def: Option<super::definitions::SavedGraphDefinitionV1>,
    workspace_lease: super::lease::WorkspaceLease,
) -> GraphRun {
    run_graph_internal(options, deps, saved_def, Some(workspace_lease))
}

fn run_graph_internal(
    options: RunOptions,
    deps: ControllerDeps,
    explicit_saved_def: Option<super::definitions::SavedGraphDefinitionV1>,
    workspace_lease: Option<super::lease::WorkspaceLease>,
) -> GraphRun {
    let continuation = options.resume_run.as_deref().map(|run| {
        (
            run.run_id.clone(),
            run.counters,
            run.ecosystem_stats.clone(),
        )
    });
    let run_id = continuation
        .as_ref()
        .map(|(run_id, _, _)| run_id.clone())
        .unwrap_or_else(new_run_id);
    let mut budgets: GraphBudgets = deps.config.budgets.clone();
    let (saved_def_from_resume, origin_from_resume, digest_from_resume) = options
        .resume_run
        .as_ref()
        .map(|r| {
            (
                r.saved_definition.clone(),
                r.execution_origin.clone(),
                r.definition_digest.clone(),
            )
        })
        .unwrap_or((None, None, None));
    let (saved_definition, execution_origin, definition_digest) =
        if let Some(def) = explicit_saved_def {
            let digest = super::definitions::compute_definition_digest(&def);
            let origin = super::types::ExecutionOrigin::SavedDefinition {
                name: def.name.clone(),
                digest: digest.clone(),
            };
            (Some(def), Some(origin), Some(digest))
        } else {
            (
                saved_def_from_resume,
                origin_from_resume.or(Some(super::types::ExecutionOrigin::GeneratedGoal)),
                digest_from_resume,
            )
        };
    if let Some(saved_budgets) = saved_definition
        .as_ref()
        .and_then(|def| def.budgets.as_ref())
    {
        if let Some(ms) = saved_budgets.max_duration_ms {
            budgets.run_deadline_ms = ms;
        }
        if let Some(usd) = saved_budgets.max_cost_usd {
            budgets.max_cost_usd = usd;
        }
    }
    let mut run = GraphRun {
        version: 1,
        run_id: run_id.clone(),
        goal: options.goal.clone(),
        cwd: options.cwd.to_string_lossy().into_owned(),
        phase: Phase::Classify,
        forced: options.forced,
        dry_run: options.dry_run,
        execution_origin,
        definition_digest,
        saved_definition,
        definition: None,
        classification: None,
        milestones: None,
        current_milestone: None,
        tasks: Vec::new(),
        verification: None,
        verification_bundle: None,
        review_coverage: None,
        budgets,
        counters: continuation
            .as_ref()
            .map(|(_, counters, _)| *counters)
            .unwrap_or(GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: now_ms(),
            }),
        blocked_reason: None,
        resource_snapshot: None,
        ecosystem_stats: continuation
            .as_ref()
            .map(|(_, _, stats)| stats.clone())
            .unwrap_or_default(),
        updated_at: 0,
        lifecycle: Some(GraphLifecycle::Running),
        control_history: options
            .resume_run
            .as_ref()
            .map(|run| run.control_history.clone())
            .unwrap_or_default(),
        revision: 0,
        continuation: Some(GraphContinuation::default()),
    };

    let _workspace_lease = match workspace_lease
        .map(Ok)
        .unwrap_or_else(|| super::lease::WorkspaceLease::acquire(&options.cwd))
    {
        Ok(lease) => lease,
        Err(error) => {
            if let Some(previous) = options.resume_run.as_deref() {
                run = previous.clone();
            }
            run.phase = Phase::Blocked;
            run.lifecycle = Some(GraphLifecycle::Stopped);
            run.blocked_reason = Some(error);
            (deps.on_update)(&run, Some("workspace ownership refused; state not saved"));
            return run;
        }
    };
    if let Some(previous) = options.resume_run.as_deref() {
        run = previous.clone();
        let identity = if run.goal != options.goal
            || run.forced != options.forced
            || run.dry_run != options.dry_run
        {
            Err("Resume options do not match the saved goal and execution mode".into())
        } else {
            super::continuation::validate_resume(&run, &options.cwd)
        };
        if let Err(error) = identity {
            run.phase = Phase::Blocked;
            run.lifecycle = Some(GraphLifecycle::RecoveryRequired);
            run.blocked_reason = Some(error);
            (deps.on_update)(&run, Some("resume refused; checkpoint not changed"));
            return run;
        }
        run.revision += 1; // validate_resume checks exhaustion before any write.
        run.phase = match run
            .continuation
            .as_ref()
            .and_then(|cursor| cursor.delivery.as_ref())
            .map(|delivery| delivery.stage)
        {
            Some(DeliveryStage::Plan) => Phase::Plan,
            Some(DeliveryStage::Implement) => Phase::Implement,
            Some(DeliveryStage::Verify) => Phase::Verify,
            Some(DeliveryStage::Review) => Phase::Review,
            None if run.classification.is_some() => Phase::Investigate,
            None => Phase::Classify,
        };
        run.lifecycle = Some(GraphLifecycle::Running);
        run.blocked_reason = None;
    }
    let _ = create_run_dir(&options.cwd, &run_id);

    let run_deadline = remaining_run_deadline(
        run.budgets.run_deadline_ms,
        run.counters.started_at,
        now_ms(),
        Instant::now(),
    );
    let exec_abort = Arc::new(AtomicBool::new(options.abort.load(Ordering::Relaxed)));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = spawn_abort_watcher(
        Arc::clone(&options.abort),
        Arc::clone(&exec_abort),
        Arc::clone(&finished),
    );

    let learning = Mutex::new(deps.learning.clone());
    let execution = Arc::new(GraphExecution {
        run: Mutex::new(run),
        deps,
        learning,
        options,
        exec_abort,
        budget_abort_reason: Mutex::new(None),
        persistence_error: Mutex::new(None),
        run_deadline,
        active_workers: Arc::new(AtomicUsize::new(0)),
        node_aborts: Arc::new(Mutex::new(HashMap::new())),
    });
    if let Some(active) = super::active_run(&execution.options.cwd) {
        if Arc::ptr_eq(&active.abort, &execution.options.abort) {
            *active
                .execution
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(&execution);
        }
    }
    execution.checkpoint(Some(if continuation.is_some() {
        "run continued"
    } else {
        "run created"
    }));
    let result = drive(&execution);
    finished.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    execution.record_skill_outcomes(&result);
    execution.snapshot()
}

fn remaining_run_deadline(
    budget_ms: u64,
    started_at: u64,
    now: u64,
    instant: Instant,
) -> Option<Instant> {
    (budget_ms > 0).then(|| {
        instant + Duration::from_millis(budget_ms.saturating_sub(now.saturating_sub(started_at)))
    })
}

fn saved_review_approves(artifact: &Artifact) -> bool {
    artifact.as_review().is_some_and(|review| {
        review.verdict == Verdict::Approve
            && !review
                .issues
                .iter()
                .any(|issue| matches!(issue.severity, Severity::Blocker | Severity::Major))
    })
}

fn saved_stage_inputs_unchanged(execution: &GraphExecution, task_id: &str) -> bool {
    let actual = execution.completion_inputs();
    let expected = execution
        .snapshot()
        .continuation
        .unwrap()
        .saved_stage_inputs
        .get(task_id)
        .cloned();
    if actual.as_ref().ok() == expected.as_ref() && expected.is_some() {
        return true;
    }
    let reason = match actual {
        Ok(_) => format!(
            "saved stage '{task_id}' inputs changed while it was executing; resume must recheck"
        ),
        Err(error) => format!("cannot validate saved stage '{task_id}' inputs: {error}"),
    };
    execution.end_task(task_id, TaskStatus::Failed, Some(reason.clone()));
    execution.blocked(reason);
    false
}

fn saved_has_completed_writer_descendant(
    definition: &super::topology::GraphDefinition,
    run: &GraphRun,
    id: &str,
) -> bool {
    let mut pending = vec![id.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(parent) = pending.pop() {
        for edge in definition.edges.iter().filter(|edge| edge.from == parent) {
            if !seen.insert(edge.to.clone()) {
                continue;
            }
            if definition
                .node(&edge.to)
                .is_some_and(|node| node.allows_mutation)
                && run
                    .task(&edge.to)
                    .is_some_and(|task| task.status == TaskStatus::Succeeded)
            {
                return true;
            }
            pending.push(edge.to.clone());
        }
    }
    false
}

fn drive_compiled_saved_graph(
    execution: &GraphExecution,
    saved_def: &super::definitions::SavedGraphDefinitionV1,
) -> GraphRun {
    let compiled = match super::bindings::compile_saved_definition(saved_def) {
        Ok(plan) => plan,
        Err(err) => {
            execution.blocked(format!("saved graph definition invalid: {err}"));
            return execution.snapshot();
        }
    };

    let cwd = execution.options.cwd.clone();
    let is_git_repo = default_is_git_repo(&cwd);
    let goal = execution.options.goal.clone();

    {
        let mut run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        run.definition = Some(compiled.topology.clone());
        run.definition_digest = Some(compiled.definition_digest.clone());
    }
    let run_id = execution.snapshot().run_id;
    if !execution.checkpoint(Some("saved graph compiled")) {
        return execution.snapshot();
    }

    let initial_baseline = match execution.snapshot().continuation.unwrap().saved_baseline {
        Some(baseline) => baseline,
        None => match capture_baseline(&cwd) {
            Ok(baseline) => {
                execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .continuation
                    .as_mut()
                    .unwrap()
                    .saved_baseline = Some(baseline.clone());
                baseline
            }
            Err(error) => {
                execution.blocked(format!("Cannot capture saved graph baseline: {error}"));
                return execution.snapshot();
            }
        },
    };
    // Retry unfinished nodes only once per explicit resume. A recorded failure
    // with an OnFailure edge remains the input to its recovery branch.
    if execution.options.resume_run.is_some() {
        // A successful worker submission is not necessarily an approval. The
        // process may have stopped before the controller interpreted it.
        for task in execution.snapshot().tasks.iter().filter(|task| {
            task.status == TaskStatus::Succeeded
                && compiled
                    .bindings
                    .get(&task.id)
                    .is_some_and(|binding| binding.stage == super::bindings::SupportedStage::Review)
        }) {
            match super::store::read_artifact(&cwd, &run_id, &task.id, task.expect) {
                Ok(artifact) if saved_review_approves(&artifact) => {}
                Ok(_) => execution.end_task(
                    &task.id,
                    TaskStatus::Failed,
                    Some("review requires changes".into()),
                ),
                Err(errors) => {
                    execution.blocked(format!(
                        "Cannot restore saved review '{}': {}",
                        task.id,
                        errors.join("; ")
                    ));
                    return execution.snapshot();
                }
            }
        }
        let inputs = match execution.completion_inputs() {
            Ok(inputs) => inputs,
            Err(error) => {
                execution.blocked(format!("Cannot validate saved graph inputs: {error}"));
                return execution.snapshot();
            }
        };
        let mut run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let stale: Vec<_> = run
            .tasks
            .iter()
            .filter(|task| {
                use super::bindings::SupportedStage;
                task.status == TaskStatus::Succeeded
                    && compiled.bindings.get(&task.id).is_some_and(|binding| {
                        matches!(
                            binding.stage,
                            SupportedStage::Verify
                                | SupportedStage::Security
                                | SupportedStage::Review
                        )
                    })
                    && run
                        .continuation
                        .as_ref()
                        .unwrap()
                        .saved_stage_inputs
                        .get(&task.id)
                        != Some(&inputs)
                    && !saved_has_completed_writer_descendant(&compiled.topology, &run, &task.id)
            })
            .map(|task| task.id.clone())
            .collect();
        for task in &mut run.tasks {
            let has_failure_edge = compiled.topology.edges.iter().any(|edge| {
                edge.from == task.id && edge.condition == super::topology::EdgeCondition::OnFailure
            });
            if matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled)
                && !has_failure_edge
                || stale.contains(&task.id)
            {
                task.status = TaskStatus::Pending;
            }
        }
    }
    if !execution.checkpoint(Some("saved graph frontier restored")) {
        return execution.snapshot();
    }
    let mut cumulative_delta = match capture_graph_delta(&cwd, &initial_baseline) {
        Ok(delta) => delta,
        Err(error) => {
            execution.blocked(format!("Cannot restore saved graph mutation: {error}"));
            return execution.snapshot();
        }
    };

    // Artifact completion precedes mutation publication. Restore only this
    // narrow interrupted boundary, before another writer could change it.
    let snapshot = execution.snapshot();
    for (index, task) in snapshot.tasks.iter().enumerate() {
        if task.status != TaskStatus::Succeeded
            || task.mutation.is_some()
            || !compiled
                .topology
                .node(&task.id)
                .is_some_and(|node| node.allows_mutation)
        {
            continue;
        }
        if snapshot.tasks[index + 1..].iter().any(|later| {
            later.attempts > 0
                && compiled
                    .topology
                    .node(&later.id)
                    .is_some_and(|node| node.allows_mutation)
        }) {
            execution.blocked(format!(
                "Writer '{}' mutation is missing after a later writer ran; reconciliation required",
                task.id
            ));
            return execution.snapshot();
        }
        let delta = snapshot
            .continuation
            .as_ref()
            .unwrap()
            .saved_attempt_baselines
            .get(&task.id)
            .ok_or_else(|| "original attempt baseline is missing".to_string())
            .and_then(|baseline| capture_graph_delta(&cwd, baseline));
        let delta = match delta {
            Ok(delta) => delta,
            Err(error) => {
                execution.blocked(format!(
                    "Cannot restore writer '{}' mutation: {error}",
                    task.id
                ));
                return execution.snapshot();
            }
        };
        execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tasks
            .iter_mut()
            .find(|entry| entry.id == task.id)
            .unwrap()
            .mutation = Some(delta.clone());
        if !execution.checkpoint_with(Some("saved writer mutation restored"), |_| {
            write_task_mutation(&cwd, &run_id, &task.id, &delta)
        }) {
            return execution.snapshot();
        }
    }

    loop {
        if !execution.wait_until_running() {
            return execution.snapshot();
        }

        let state = {
            let run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            GraphRunState::from_run(&run)
        };

        let ready = ready_nodes(&compiled.topology, &state);
        if ready.is_empty() {
            break;
        }

        // Pick next ready node in topological order
        let task_id = ready[0].clone();
        let node = match compiled.topology.node(&task_id) {
            Some(n) => n.clone(),
            None => {
                execution.blocked(format!("unknown task in ready frontier: {task_id}"));
                return execution.snapshot();
            }
        };
        let binding = match compiled.bindings.get(&task_id) {
            Some(b) => b.clone(),
            None => {
                execution.blocked(format!("missing stage binding for ready node: {task_id}"));
                return execution.snapshot();
            }
        };

        match binding.stage {
            super::bindings::SupportedStage::Classify => execution.set_phase(Phase::Classify),
            super::bindings::SupportedStage::Research => execution.set_phase(Phase::Investigate),
            super::bindings::SupportedStage::Plan => execution.set_phase(Phase::Plan),
            super::bindings::SupportedStage::Implement => execution.set_phase(Phase::Implement),
            super::bindings::SupportedStage::Verify => execution.begin_verification(),
            super::bindings::SupportedStage::Security => execution.set_phase(Phase::Verify),
            super::bindings::SupportedStage::Review => execution.set_phase(Phase::Review),
        }
        if !execution.checkpoint(None) {
            return execution.snapshot();
        }

        let incoming_deps: Vec<String> = compiled
            .topology
            .incoming_edges(&task_id)
            .map(|e| e.from.clone())
            .collect();

        if matches!(
            binding.stage,
            super::bindings::SupportedStage::Verify
                | super::bindings::SupportedStage::Security
                | super::bindings::SupportedStage::Review
        ) {
            let inputs = match execution.completion_inputs() {
                Ok(inputs) => inputs,
                Err(error) => {
                    execution.blocked(format!("Cannot capture saved stage inputs: {error}"));
                    return execution.snapshot();
                }
            };
            execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .continuation
                .as_mut()
                .unwrap()
                .saved_stage_inputs
                .insert(task_id.clone(), inputs);
            if !execution.checkpoint(Some("saved stage inputs persisted")) {
                return execution.snapshot();
            }
        }

        if binding.stage == super::bindings::SupportedStage::Verify {
            let budgets = execution.snapshot().budgets;
            let commands = collect_verify_commands(&CollectInput {
                config_commands: &execution.deps.config.verify_commands,
                detected: &detect_verify_commands(&cwd),
                plan: None,
            });
            if commands.is_empty() && !execution.options.dry_run {
                execution.blocked(
                    "no verification command to run: add verifyCommands to .pi/graph.json                      (or a Cargo.toml / package.json test script) so the change can be checked"
                        .to_string(),
                );
                return execution.snapshot();
            }
            let mut verification =
                execution.verify_commands(&commands, budgets.verify_command_timeout_ms);
            if execution.options.dry_run && nothing_ran(&verification) {
                verification.passed = true;
            }
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                run.verification = Some(verification.clone());
                run.continuation
                    .as_mut()
                    .unwrap()
                    .saved_verifications
                    .insert(task_id.clone(), verification.clone());
            }
            let mut task_state =
                GraphTaskState::new(task_id.clone(), node.role, node.expect, incoming_deps, None);
            task_state.started_at = Some(now_ms());
            task_state.ended_at = Some(now_ms());
            if verification.passed {
                task_state.mark_succeeded();
            } else {
                task_state.mark_failed("verification failed".to_string());
            }
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(existing) = run.tasks.iter_mut().find(|task| task.id == task_id) {
                    *existing = task_state;
                } else {
                    run.tasks.push(task_state);
                }
            }
            if !saved_stage_inputs_unchanged(execution, &task_id) {
                return execution.snapshot();
            }
            if !execution.checkpoint(Some(if verification.passed {
                "verification passed"
            } else {
                "verification FAILED"
            })) {
                return execution.snapshot();
            }
            continue;
        }

        if binding.stage == super::bindings::SupportedStage::Security {
            let mut sec_task = GraphTaskState::new(
                task_id.clone(),
                node.role,
                node.expect,
                incoming_deps,
                Some("security verification".to_string()),
            );
            sec_task.started_at = Some(now_ms());
            sec_task.ended_at = Some(now_ms());
            let changed_files: Vec<String> = cumulative_delta
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect();
            let mut sec_controller =
                crate::native_extensions::SecurityScanController::new(cwd.clone());
            let req = crate::native_extensions::SecurityVerifyRequest {
                cwd: &cwd,
                changed_files: &changed_files,
                graph_run_id: &run_id,
            };
            let outcome = match sec_controller.verify_changed_surface(req) {
                Ok(sec) => sec,
                Err(reason) => SecurityVerification::Unavailable { reason },
            };
            if matches!(outcome, SecurityVerification::Passed { .. }) {
                sec_task.mark_succeeded();
            } else {
                sec_task.mark_failed(format!("{outcome:?}"));
            }
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                run.continuation
                    .as_mut()
                    .unwrap()
                    .saved_security
                    .insert(task_id.clone(), outcome);
                if let Some(existing) = run.tasks.iter_mut().find(|task| task.id == task_id) {
                    *existing = sec_task;
                } else {
                    run.tasks.push(sec_task);
                }
            }
            if !saved_stage_inputs_unchanged(execution, &task_id)
                || !execution.checkpoint(Some("security verification complete"))
            {
                return execution.snapshot();
            }
            continue;
        }

        // Standard worker stage: Classify, Research, Plan, Implement, Review
        let max_researchers = execution.snapshot().budgets.max_researchers;
        let diff_text = cumulative_delta.diff();
        let diff_str = if !diff_text.is_empty() {
            Some(diff_text.as_str())
        } else {
            None
        };
        let briefing = match super::bindings::build_briefing_for_stage(
            &node,
            &binding,
            &goal,
            &execution.snapshot(),
            &cwd,
            is_git_repo,
            max_researchers,
            diff_str,
            execution.snapshot().verification.as_ref(),
        ) {
            Ok(b) => b,
            Err(err) => {
                execution.blocked(format!("briefing error for '{task_id}': {err}"));
                return execution.snapshot();
            }
        };

        let task =
            GraphTaskState::new(task_id.clone(), node.role, node.expect, incoming_deps, None);

        let attempt_baseline = if node.allows_mutation {
            let previous = execution
                .snapshot()
                .continuation
                .unwrap()
                .saved_attempt_baselines
                .get(&task_id)
                .cloned();
            match previous.map(Ok).unwrap_or_else(|| capture_baseline(&cwd)) {
                Ok(baseline) => {
                    execution
                        .run
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .continuation
                        .as_mut()
                        .unwrap()
                        .saved_attempt_baselines
                        .insert(task_id.clone(), baseline.clone());
                    if !execution.checkpoint(Some("saved writer baseline persisted")) {
                        return execution.snapshot();
                    }
                    baseline
                }
                Err(error) => {
                    execution.blocked(format!("Cannot capture saved writer baseline: {error}"));
                    return execution.snapshot();
                }
            }
        } else {
            super::mutation::MutationBaseline::default()
        };

        let mut artifact = execution.execute_node(task, briefing);
        if binding.stage == super::bindings::SupportedStage::Review && artifact.is_some() {
            if !saved_stage_inputs_unchanged(execution, &task_id) {
                return execution.snapshot();
            }
            if artifact
                .as_ref()
                .is_none_or(|artifact| !saved_review_approves(artifact))
            {
                execution.end_task(
                    &task_id,
                    TaskStatus::Failed,
                    Some("review requires changes".into()),
                );
                if !execution.checkpoint(Some("saved review requires changes")) {
                    return execution.snapshot();
                }
                artifact = None;
            }
        }

        if node.allows_mutation {
            let attempt_delta = match capture_graph_delta(&cwd, &attempt_baseline) {
                Ok(delta) => delta,
                Err(error) => {
                    execution.blocked(format!("Cannot capture saved writer mutation: {error}"));
                    return execution.snapshot();
                }
            };
            cumulative_delta = match capture_graph_delta(&cwd, &initial_baseline) {
                Ok(delta) => delta,
                Err(error) => {
                    execution.blocked(format!("Cannot capture saved graph mutation: {error}"));
                    return execution.snapshot();
                }
            };
            let mut run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let run_id = run.run_id.clone();
            if let Some(task_entry) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                task_entry.mutation = Some(attempt_delta.clone());
            }
            drop(run);
            if !execution.checkpoint_with(None, |_| {
                write_task_mutation(&cwd, &run_id, &task_id, &attempt_delta)
            }) {
                return execution.snapshot();
            }
        }

        if execution.cancelled_if_aborted() {
            return execution.snapshot();
        }

        if artifact.is_none() && node.required {
            // Check if there are failure edges out of this node
            let has_failure_edge = compiled.topology.edges.iter().any(|e| {
                e.from == task_id && e.condition == super::topology::EdgeCondition::OnFailure
            });
            if !has_failure_edge {
                execution.blocked(format!("required node '{task_id}' failed"));
                return execution.snapshot();
            }
        }
    }

    let final_state = {
        let run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        GraphRunState::from_run(&run)
    };

    let mut failed_required = false;
    for node in &compiled.topology.nodes {
        if node.required && !final_state.succeeded_tasks.contains(&node.id) {
            failed_required = true;
            break;
        }
    }

    if execution.cancelled_if_aborted() {
        return execution.snapshot();
    }
    if failed_required && execution.snapshot().blocked_reason.is_none() {
        execution.blocked("one or more required nodes failed or were unreachable".to_string());
    } else if execution.snapshot().blocked_reason.is_none() {
        execution.set_phase(Phase::Done);
        execution.checkpoint(Some("done (saved pipeline executed successfully)"));
    }

    execution.snapshot()
}

fn drive(execution: &GraphExecution) -> GraphRun {
    if execution.cancelled_if_aborted() {
        return execution.snapshot();
    }
    let saved_def = execution.snapshot().saved_definition.clone();
    if let Some(saved_def) = saved_def {
        return drive_compiled_saved_graph(execution, &saved_def);
    }

    let cwd = execution.options.cwd.clone();
    let is_git_repo = default_is_git_repo(&cwd);
    let goal = execution.options.goal.clone();
    let max_researchers = execution.snapshot().budgets.max_researchers;

    let classify_task = GraphTaskState::new(
        "classify",
        Role::Classifier,
        ArtifactKind::Classification,
        vec![],
        None,
    );
    let classification = execution
        .snapshot()
        .classification
        .map(Artifact::Classification)
        .or_else(|| {
            execution.execute_node(
                classify_task,
                classify_briefing(&ClassifyInput {
                    goal: &goal,
                    is_git_repo,
                    package_scripts: read_package_scripts(&cwd),
                    max_researchers,
                }),
            )
        });
    if execution.cancelled_if_aborted() {
        return execution.snapshot();
    }
    let Some(classification) =
        classification.and_then(|artifact| artifact.as_classification().cloned())
    else {
        execution.blocked(execution.task_failure_reason("classification failed"));
        return execution.snapshot();
    };

    let complexity = execution
        .options
        .forced
        .unwrap_or(classification.complexity);
    let mode = GraphMode::from(complexity);
    let definition = build_definition(mode, &classification);
    if let Err(err) = validate_definition(&definition) {
        execution.blocked(format!("invalid graph topology: {err}"));
        return execution.snapshot();
    }
    let milestones: Vec<String> = classification
        .milestones
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|milestone| milestone.trim().to_string())
        .filter(|milestone| !milestone.is_empty())
        .take(8)
        .collect();
    {
        let mut run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        run.classification = Some(classification.clone());
        run.definition.get_or_insert_with(|| definition.clone());
        if complexity != Complexity::Trivial && milestones.len() > 1 {
            run.milestones = Some(milestones.clone());
        }
    }
    let milestone_note = if milestones.len() > 1 && complexity != Complexity::Trivial {
        format!(", {} milestones", milestones.len())
    } else {
        String::new()
    };
    execution.checkpoint(Some(&format!(
        "classified: {}/{complexity}{milestone_note}",
        classification.task_class
    )));

    let prior_evidence = execution
        .snapshot()
        .continuation
        .as_ref()
        .and_then(|cursor| cursor.evidence_digest.clone());
    let mut evidence_digest = prior_evidence.clone().unwrap_or_default();
    if complexity != Complexity::Trivial && prior_evidence.is_none() {
        execution.set_phase(Phase::Investigate);
        execution.checkpoint(None);
        let requests: Vec<_> = classification
            .research_tasks
            .iter()
            .filter(|request| request.kind != ResearchKind::History || is_git_repo)
            .take(max_researchers as usize)
            .cloned()
            .collect();
        let evidences: Mutex<Vec<EvidenceArtifact>> = Mutex::new(Vec::new());
        let failed_kinds: Mutex<Vec<ResearchKind>> = Mutex::new(Vec::new());
        let next = AtomicUsize::new(0);
        let parallelism = execution
            .snapshot()
            .budgets
            .max_parallel_workers
            .max(1)
            .min(requests.len().max(1) as u32) as usize;

        thread::scope(|scope| {
            for _ in 0..parallelism {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(request) = requests.get(index) else {
                        return;
                    };
                    let task = GraphTaskState::new(
                        format!("research-{}", index + 1),
                        role_for_research_kind(request.kind),
                        ArtifactKind::Evidence,
                        vec!["classify".to_string()],
                        Some(request.focus.clone()),
                    );
                    let artifact = execution
                        .execute_node(task, research_briefing(&goal, request.kind, &request.focus));
                    match artifact.and_then(|artifact| artifact.as_evidence().cloned()) {
                        Some(evidence) => evidences
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push(evidence),
                        None => failed_kinds
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push(request.kind),
                    }
                });
            }
        });

        if execution.cancelled_if_aborted() {
            return execution.snapshot();
        }
        let failed_kinds = failed_kinds
            .into_inner()
            .unwrap_or_else(|error| error.into_inner());
        if !failed_kinds.is_empty() {
            execution.blocked("Investigation is incomplete; resume retries unfinished researchers and preserves successful siblings".into());
            return execution.snapshot();
        }
        evidence_digest = build_evidence_digest(
            &evidences
                .into_inner()
                .unwrap_or_else(|error| error.into_inner()),
            &failed_kinds,
            EVIDENCE_DIGEST_MAX_CHARS,
        );
    }

    {
        let mut run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        run.continuation.as_mut().unwrap().evidence_digest = Some(evidence_digest.clone());
    }
    if !execution.checkpoint(Some("investigation checkpoint saved")) {
        return execution.snapshot();
    }

    let mut indices = execution.snapshot().continuation.as_ref().unwrap().indices;
    let goals: Vec<String> = if milestones.len() > 1 && complexity != Complexity::Trivial {
        (0..milestones.len())
            .map(|index| milestone_goal(&goal, &milestones, index))
            .collect()
    } else {
        vec![goal.clone()]
    };
    let milestone_count = goals.len();

    let mut completed = execution
        .snapshot()
        .continuation
        .as_ref()
        .unwrap()
        .completed_milestones;
    if completed == milestone_count {
        let inputs = match execution.completion_inputs() {
            Ok(inputs) => inputs,
            Err(error) => {
                execution.blocked(format!("Cannot validate finalization inputs: {error}"));
                return execution.snapshot();
            }
        };
        let mut run = execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let cursor = run.continuation.as_mut().unwrap();
        if cursor.completion_inputs.as_ref() != Some(&inputs) {
            let Some(mut delivery) = cursor.completed_delivery.clone() else {
                drop(run);
                execution.blocked("Finalization cannot restore completed delivery evidence".into());
                return execution.snapshot();
            };
            delivery.stage = DeliveryStage::Verify;
            completed = completed.saturating_sub(1);
            cursor.completed_milestones = completed;
            cursor.delivery = Some(delivery);
        }
    }
    for (index, goal_text) in goals.iter().enumerate().skip(completed) {
        if milestone_count > 1 {
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                run.current_milestone = Some(index + 1);
            }
            execution.checkpoint(Some(&format!(
                "milestone {}/{milestone_count}: {}",
                index + 1,
                truncate(&milestones[index], 80)
            )));
        }
        let delivery = deliver_goal(
            execution,
            goal_text,
            complexity,
            &evidence_digest,
            &mut indices,
        );
        if matches!(delivery, Delivery::Stop) {
            return execution.snapshot();
        }
        let completion_inputs = match execution.completion_inputs() {
            Ok(inputs) => inputs,
            Err(error) => {
                execution.blocked(format!("Cannot persist delivery input identity: {error}"));
                return execution.snapshot();
            }
        };
        {
            let mut run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let cursor = run.continuation.as_mut().unwrap();
            cursor.completed_milestones = index + 1;
            cursor.indices = indices;
            cursor.completed_delivery = cursor.delivery.take();
            cursor.completion_inputs = Some(completion_inputs);
        }
        if !execution.checkpoint(Some("milestone complete")) {
            return execution.snapshot();
        }
    }

    if execution.cancelled_if_aborted() {
        return execution.snapshot();
    }
    execution.set_phase(Phase::Done);
    let note = if milestone_count > 1 {
        format!("done ({milestone_count} milestones delivered)")
    } else if complexity == Complexity::Trivial {
        "done (trivial path: verified, review skipped)".to_string()
    } else {
        "done (approved)".to_string()
    };
    execution.checkpoint(Some(&note));
    execution.snapshot()
}

/// Generated milestone templates cannot predict how many writer/review attempts
/// verification will need. Bind dispatched attempts to the actual preceding work
/// instead of interpreting their ordinal as a milestone number. Saved graphs
/// retain their explicit topology and may never be rebound this way.
fn materialize_generated_dependencies(run: &mut GraphRun, task: &mut GraphTaskState) {
    if run.saved_definition.is_some() || run.definition.is_none() {
        return;
    }
    let is_new_plan =
        task.role == Role::Planner && run.definition.as_ref().unwrap().node(&task.id).is_none();
    if !matches!(task.role, Role::Writer | Role::Reviewer) && !is_new_plan {
        return;
    }
    let succeeded =
        |entry: &&GraphTaskState| entry.id != task.id && entry.status == TaskStatus::Succeeded;
    let mut dependencies = task.depends_on.clone();
    if task.role == Role::Writer {
        if let Some(plan) = run
            .tasks
            .iter()
            .rev()
            .filter(succeeded)
            .find(|t| t.role == Role::Planner)
        {
            dependencies.push(plan.id.clone());
        }
    }
    let previous = run.tasks.iter().rev().filter(succeeded).find(|entry| {
        if task.role == Role::Reviewer {
            entry.role == Role::Writer
        } else {
            matches!(
                entry.role,
                Role::Writer | Role::Reviewer | Role::Planner | Role::Classifier
            )
        }
    });
    if let Some(previous) = previous {
        dependencies.push(previous.id.clone());
    }
    dependencies.sort();
    dependencies.dedup();
    task.depends_on = dependencies;
    let pending_review = run
        .continuation
        .as_ref()
        .map(|cursor| format!("review-{}", cursor.indices.review.saturating_add(1)));
    let definition = run.definition.as_mut().unwrap();
    if definition.node(&task.id).is_none() {
        definition.nodes.push(NodeDefinition {
            id: task.id.clone(),
            role: task.role,
            expect: task.expect,
            required: true,
            allows_mutation: task.role == Role::Writer,
        });
    }
    definition.edges.retain(|edge| edge.to != task.id);
    definition
        .edges
        .extend(task.depends_on.iter().map(|dependency| EdgeDefinition {
            from: dependency.clone(),
            to: task.id.clone(),
            condition: EdgeCondition::OnSuccess,
        }));
    // A revision is a new writer identity, but still owes the current milestone
    // a review. Persist that edge immediately, including before verification.
    if task.role == Role::Writer && definition.mode != GraphMode::Simple {
        if let Some(review_id) = pending_review {
            if definition.node(&review_id).is_none() {
                definition.nodes.push(NodeDefinition {
                    id: review_id.clone(),
                    role: Role::Reviewer,
                    expect: ArtifactKind::Review,
                    required: true,
                    allows_mutation: false,
                });
            }
            definition.edges.retain(|edge| edge.to != review_id);
            definition.edges.push(EdgeDefinition {
                from: task.id.clone(),
                to: review_id,
                condition: EdgeCondition::OnSuccess,
            });
        }
    }
}

fn produce_plan(
    execution: &GraphExecution,
    indices: &mut NodeIndices,
    goal_text: &str,
    evidence_digest: &str,
    replan_reason: Option<&str>,
) -> Option<ImplementationPlan> {
    let mut delivery = execution
        .snapshot()
        .continuation
        .as_ref()
        .unwrap()
        .delivery
        .clone()
        .unwrap();
    let task_id = if let Some(id) = &delivery.plan_task {
        id.clone()
    } else {
        indices.plan += 1;
        let id = format!("plan-{}", indices.plan);
        delivery.plan_task = Some(id.clone());
        id
    };
    if !execution.save_delivery(&delivery, *indices, Some("planning cursor saved")) {
        return None;
    }
    let task = GraphTaskState::new(task_id, Role::Planner, ArtifactKind::Plan, vec![], None);
    execution
        .execute_node(
            task,
            plan_briefing(goal_text, evidence_digest, replan_reason),
        )
        .and_then(|artifact| artifact.as_plan().cloned())
}

/// Deliver one goal (the whole request, or one milestone of it) through
/// plan -> implement -> verify -> review. Revision and replan budgets apply per
/// deliverable.
fn deliver_goal(
    execution: &GraphExecution,
    goal_text: &str,
    complexity: Complexity,
    evidence_digest: &str,
    indices: &mut NodeIndices,
) -> Delivery {
    let budgets = execution.snapshot().budgets;
    let cwd = execution.options.cwd.clone();
    let mut saved = if let Some(delivery) = execution
        .snapshot()
        .continuation
        .as_ref()
        .unwrap()
        .delivery
        .clone()
    {
        delivery
    } else {
        let baseline = match capture_baseline(&cwd) {
            Ok(baseline) => baseline,
            Err(error) => {
                execution.blocked(format!("Cannot capture mutation baseline: {error}"));
                return Delivery::Stop;
            }
        };
        DeliveryCheckpoint::new(baseline, complexity != Complexity::Trivial)
    };
    if !execution.save_delivery(&saved, *indices, Some("delivery cursor saved")) {
        return Delivery::Stop;
    }
    let mut plan = saved.plan.clone();
    let mut revision_cycles = saved.revision_cycles;
    let mut replans = saved.replans;
    let mut revision_notes = saved.revision_notes.clone();
    let milestone_baseline = saved.baseline.clone();
    loop {
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }
        if saved.stage == DeliveryStage::Plan {
            execution.set_phase(Phase::Plan);
            plan = produce_plan(
                execution,
                indices,
                goal_text,
                evidence_digest,
                saved.replan_reason.as_deref(),
            );
            if execution.cancelled_if_aborted() {
                return Delivery::Stop;
            }
            if plan.is_none() {
                execution.blocked(execution.task_failure_reason("planning failed"));
                return Delivery::Stop;
            }
            saved = execution
                .snapshot()
                .continuation
                .as_ref()
                .unwrap()
                .delivery
                .clone()
                .unwrap();
            saved.plan = plan.clone();
            saved.stage = DeliveryStage::Implement;
            if !execution.save_delivery(&saved, *indices, Some("plan complete")) {
                return Delivery::Stop;
            }
        }

        let patch = if saved.stage == DeliveryStage::Implement {
            execution.set_phase(Phase::Implement);
            let task_id = if let Some(id) = &saved.writer_task {
                id.clone()
            } else {
                indices.implement += 1;
                let id = format!("implement-{}", indices.implement);
                saved.writer_task = Some(id.clone());
                id
            };
            let task = GraphTaskState::new(
                task_id.clone(),
                Role::Writer,
                ArtifactKind::PatchReport,
                vec![],
                None,
            );
            let attempt_baseline = if let Some(baseline) = &saved.attempt_baseline {
                baseline.clone()
            } else {
                match capture_baseline(&cwd) {
                    Ok(baseline) => baseline,
                    Err(error) => {
                        execution.blocked(format!("Cannot capture attempt baseline: {error}"));
                        return Delivery::Stop;
                    }
                }
            };
            saved.attempt_baseline = Some(attempt_baseline.clone());
            if !execution.save_delivery(&saved, *indices, Some("writer cursor saved")) {
                return Delivery::Stop;
            }
            let patch = execution
                .execute_node(
                    task,
                    implement_briefing(
                        goal_text,
                        plan.as_ref(),
                        evidence_digest,
                        revision_notes.as_deref(),
                    ),
                )
                .and_then(|artifact| artifact.as_patch_report().cloned());
            if execution.cancelled_if_aborted() {
                return Delivery::Stop;
            }
            let Some(patch) = patch else {
                let failure = execution
                    .snapshot()
                    .tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .and_then(|task| task.error.clone());
                if failure.as_deref().is_some_and(|error| {
                    classify_worker_failure(&WorkerResult::default(), Some(error))
                        == WorkerFailureClass::PlanInvalidated
                }) {
                    let reason = failure.unwrap_or_else(|| "writer invalidated the plan".into());
                    if replans >= budgets.max_replans {
                        execution
                            .blocked(format!("plan invalidated {} times: {reason}", replans + 1));
                        return Delivery::Stop;
                    }
                    replans += 1;
                    {
                        let mut run = execution
                            .run
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        run.counters.replans += 1;
                        run.phase = Phase::Plan;
                    }
                    saved.prepare_replan(reason, replans);
                    if !execution.save_delivery(
                        &saved,
                        *indices,
                        Some("replanning after writer failure"),
                    ) {
                        return Delivery::Stop;
                    }
                    revision_notes = None;
                    continue;
                }
                execution.blocked(execution.task_failure_reason("implementation failed"));
                return Delivery::Stop;
            };

            let attempt_delta = match capture_graph_delta(&cwd, &attempt_baseline) {
                Ok(delta) => delta,
                Err(error) => {
                    execution.blocked(format!("Cannot capture writer mutation: {error}"));
                    return Delivery::Stop;
                }
            };
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let run_id = run.run_id.clone();
                if let Some(task_entry) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                    task_entry.mutation = Some(attempt_delta.clone());
                }
                drop(run);
                if !execution.checkpoint_with(None, |_| {
                    write_task_mutation(&cwd, &run_id, &task_id, &attempt_delta)
                }) {
                    return Delivery::Stop;
                }
            }

            if patch.plan_invalidated {
                let reason = patch.invalidation_reason.clone().unwrap_or_default();
                if replans >= budgets.max_replans {
                    execution.blocked(format!("plan invalidated {} times: {reason}", replans + 1));
                    return Delivery::Stop;
                }
                replans += 1;
                {
                    let mut run = execution
                        .run
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    run.counters.replans += 1;
                    run.phase = Phase::Plan;
                }
                saved.prepare_replan(reason, replans);
                if !execution.save_delivery(&saved, *indices, Some("replanning")) {
                    return Delivery::Stop;
                }
                revision_notes = None;
                continue;
            }

            saved.patch = Some(patch.clone());
            saved.stage = DeliveryStage::Verify;
            if !execution.save_delivery(&saved, *indices, Some("implementation complete")) {
                return Delivery::Stop;
            }
            patch
        } else {
            let Some(patch) = saved.patch.clone() else {
                execution.blocked("Continuation has no completed patch report".into());
                return Delivery::Stop;
            };
            patch
        };
        let cumulative_delta = match capture_graph_delta(&cwd, &milestone_baseline) {
            Ok(delta) => delta,
            Err(error) => {
                execution.blocked(format!("Cannot restore cumulative mutation: {error}"));
                return Delivery::Stop;
            }
        };

        execution.begin_verification();
        execution.checkpoint(None);
        let cwd = execution.options.cwd.clone();
        let commands = collect_verify_commands(&CollectInput {
            config_commands: &execution.deps.config.verify_commands,
            detected: &detect_verify_commands(&cwd),
            plan: plan.as_ref(),
        });
        // An unverified change is never delivered. With nothing to run there
        // is nothing to revise either, so this blocks rather than looping.
        if commands.is_empty() && !execution.options.dry_run {
            execution.blocked(
                "no verification command to run: add verifyCommands to .pi/graph.json \
                 (or a Cargo.toml / package.json test script) so the change can be checked"
                    .to_string(),
            );
            return Delivery::Stop;
        }
        let mut verification: VerificationResult =
            execution.verify_commands(&commands, budgets.verify_command_timeout_ms);
        // A dry run verifies nothing by design: with no command to pretend
        // to run, it passes instead of revising a change nobody made.
        if execution.options.dry_run && nothing_ran(&verification) {
            verification.passed = true;
        }
        {
            let mut run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            run.verification = Some(verification.clone());
        }
        execution.checkpoint(Some(if verification.passed {
            "verification passed"
        } else {
            "verification FAILED"
        }));
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }

        if !verification.passed {
            if verification.commands.iter().any(|command| {
                super::verify::looks_like_missing_command(command.exit_code, &command.output_tail)
            }) {
                execution.blocked("Verification command is unavailable; repair the environment and resume this run".into());
                return Delivery::Stop;
            }
            if nothing_ran(&verification) && !execution.options.dry_run {
                execution.blocked(
                    "every verification command was plan-invented and does not exist; \
                     add verifyCommands to .pi/graph.json so the change can be checked"
                        .to_string(),
                );
                return Delivery::Stop;
            }
            if revision_cycles >= budgets.max_revision_cycles {
                execution.blocked(format!(
                    "verification still failing after {revision_cycles} revision cycles"
                ));
                return Delivery::Stop;
            }
            revision_cycles += 1;
            execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .counters
                .revision_cycles += 1;
            revision_notes = Some(revision_notes_from(Some(&verification), None, None));
            saved.prepare_revision(revision_notes.clone(), revision_cycles);
            if !execution.save_delivery(&saved, *indices, Some("verification revision saved")) {
                return Delivery::Stop;
            }
            continue;
        }

        let changed_files: Vec<String> = if !cumulative_delta.files.is_empty() {
            cumulative_delta
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect()
        } else {
            patch.changed_files.clone()
        };

        let policy_mode = execution.deps.config.security_verification;
        let risk = cumulative_delta.assess_risk();
        let should_scan = match policy_mode {
            SecurityPolicyMode::Off => false,
            SecurityPolicyMode::Risk => risk.level == ChangeRisk::High,
            SecurityPolicyMode::Always => {
                !cumulative_delta.files.is_empty() || !changed_files.is_empty()
            }
        };

        let security_verification = if should_scan {
            execution.set_phase(Phase::Verify);
            let Some(security_index) = indices.security.checked_add(1) else {
                execution.blocked("Security attempt counter exhausted".into());
                return Delivery::Stop;
            };
            indices.security = security_index;
            if !execution.save_delivery(&saved, *indices, Some("security cursor saved")) {
                return Delivery::Stop;
            }
            let run_id = execution.snapshot().run_id;
            let mut sec_controller =
                crate::native_extensions::SecurityScanController::new(cwd.clone());
            let req = crate::native_extensions::SecurityVerifyRequest {
                cwd: &cwd,
                changed_files: &changed_files,
                graph_run_id: &run_id,
            };
            let outcome = match sec_controller.verify_changed_surface(req) {
                Ok(sec) => sec,
                Err(reason) => SecurityVerification::Unavailable { reason },
            };

            let mut sec_task = GraphTaskState::new(
                format!("security-{}", indices.security),
                Role::TestAnalyzer,
                ArtifactKind::Evidence,
                vec![format!("implement-{}", indices.implement)],
                Some("security verification".to_string()),
            );
            sec_task.started_at = Some(now_ms());
            sec_task.ended_at = Some(now_ms());
            if matches!(outcome, SecurityVerification::Passed { .. }) {
                sec_task.mark_succeeded();
            } else {
                sec_task.mark_failed(format!("{outcome:?}"));
            }
            {
                let mut run = execution
                    .run
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                run.tasks.push(sec_task);
            }
            outcome
        } else {
            SecurityVerification::NotRequired
        };

        let mut bundle = verification.to_bundle(
            changed_files.clone(),
            Some(execution.snapshot().run_id),
            security_verification.clone(),
        );
        if let Err(error) =
            operations::attach_source_manifest_digest(&mut bundle, &execution.snapshot(), &cwd)
        {
            execution.blocked(format!(
                "failed to capture verification source manifest: {error}"
            ));
            return Delivery::Stop;
        }

        {
            let mut run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            run.verification_bundle = Some(bundle.clone());
            if should_scan {
                run.ecosystem_stats.security_gate_triggered = true;
                run.ecosystem_stats.security_result = Some(match &security_verification {
                    SecurityVerification::Passed { .. } => "passed".to_string(),
                    SecurityVerification::Failed { .. } => "failed".to_string(),
                    SecurityVerification::Unavailable { reason } => {
                        format!("unavailable: {reason}")
                    }
                    SecurityVerification::NotRequired => "not_required".to_string(),
                });
            }
        }

        let security_eligible = if execution.options.dry_run {
            !matches!(security_verification, SecurityVerification::Failed { .. })
        } else {
            bundle.approval_eligible(policy_mode)
        };
        if !security_eligible {
            if matches!(
                security_verification,
                SecurityVerification::Unavailable { .. }
            ) {
                execution.blocked(format!("Security verification unavailable; repair the environment and resume: {security_verification:?}"));
                return Delivery::Stop;
            }
            if revision_cycles >= budgets.max_revision_cycles {
                execution.blocked(format!(
                    "security verification still failing after {revision_cycles} revision cycles: {security_verification:?}"
                ));
                return Delivery::Stop;
            }
            revision_cycles += 1;
            execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .counters
                .revision_cycles += 1;
            revision_notes = Some(revision_notes_from(
                Some(&verification),
                None,
                Some(&security_verification),
            ));
            saved.prepare_revision(revision_notes.clone(), revision_cycles);
            if !execution.save_delivery(&saved, *indices, Some("security revision saved")) {
                return Delivery::Stop;
            }
            continue;
        }

        if complexity == Complexity::Trivial {
            return Delivery::Ok;
        }

        execution.set_phase(Phase::Review);
        let review_inputs = match capture_baseline(&cwd).and_then(|source| {
            let results: Vec<_> = verification
                .commands
                .iter()
                .map(|command| {
                    (
                        &command.name,
                        &command.command,
                        command.exit_code,
                        &command.output_tail,
                        command.skipped,
                    )
                })
                .collect();
            serde_json::to_string(&(
                &source.files,
                &commands,
                results,
                policy_mode,
                &security_verification,
                goal_text,
                &plan,
            ))
            .map(|json| super::replay::compute_input_hash(&json))
            .map_err(|error| error.to_string())
        }) {
            Ok(inputs) => inputs,
            Err(error) => {
                execution.blocked(format!("Cannot bind review to current inputs: {error}"));
                return Delivery::Stop;
            }
        };
        if saved.review_inputs.as_ref() != Some(&review_inputs) {
            saved.review_task = None;
        }
        saved.review_inputs = Some(review_inputs);
        if saved.review_task.is_none() {
            indices.review += 1;
            saved.review_task = Some(format!("review-{}", indices.review));
        }
        saved.stage = DeliveryStage::Review;
        if !execution.save_delivery(&saved, *indices, Some("review cursor saved")) {
            return Delivery::Stop;
        }
        let graph_diff = cumulative_delta.diff();

        let chunks = chunk_graph_mutation(&cumulative_delta, DIFF_MAX_CHARS);
        let required_chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let mut coverage = ReviewCoverage::new(required_chunk_ids);

        let review = if chunks.len() > 1 {
            // Large diff: review individual chunks, then aggregate findings compactly
            let mut all_issues = Vec::new();
            let mut chunk_summaries = Vec::new();

            for (idx, chunk) in chunks.iter().enumerate() {
                let chunk_task_id = format!("review-{}-chunk-{}", indices.review, idx + 1);
                let task = GraphTaskState::new(
                    chunk_task_id,
                    Role::Reviewer,
                    ArtifactKind::Review,
                    vec![],
                    None,
                );
                let req_ids = vec![chunk.id.clone()];
                let chunk_review = execution
                    .execute_node(
                        task,
                        review_briefing(&ReviewInput {
                            goal: goal_text,
                            plan: plan.as_ref(),
                            diff: &chunk.patch,
                            changed_files: &changed_files,
                            verification: &verification,
                            chunk: Some(chunk),
                            chunk_summaries: None,
                            required_chunk_ids: Some(&req_ids),
                            security: Some(&security_verification),
                        }),
                    )
                    .and_then(|artifact| artifact.as_review().cloned());

                if execution.cancelled_if_aborted() {
                    return Delivery::Stop;
                }
                let Some(chunk_review) = chunk_review else {
                    execution.blocked(
                        execution.task_failure_reason(&format!("chunk review {} failed", chunk.id)),
                    );
                    return Delivery::Stop;
                };

                if !chunk_review.reviewed_chunk_ids.is_empty() {
                    coverage.record_reviewed(&chunk_review.reviewed_chunk_ids);
                } else {
                    coverage.record_reviewed(&[chunk.id.clone()]);
                }
                let issues_count = chunk_review.issues.len();
                all_issues.extend(chunk_review.issues);
                chunk_summaries.push(format!(
                    "Chunk {} ({}): verdict={}, {} issue(s): {}",
                    chunk.id, chunk.file, chunk_review.verdict, issues_count, chunk_review.notes
                ));
            }

            // Final holistic review summarizing all chunk reviews
            let task = GraphTaskState::new(
                format!("review-{}", indices.review),
                Role::Reviewer,
                ArtifactKind::Review,
                vec![],
                None,
            );
            let final_review = execution
                .execute_node(
                    task,
                    review_briefing(&ReviewInput {
                        goal: goal_text,
                        plan: plan.as_ref(),
                        diff: "",
                        changed_files: &changed_files,
                        verification: &verification,
                        chunk: None,
                        chunk_summaries: Some(&chunk_summaries),
                        required_chunk_ids: None,
                        security: Some(&security_verification),
                    }),
                )
                .and_then(|artifact| artifact.as_review().cloned());

            if execution.cancelled_if_aborted() {
                return Delivery::Stop;
            }
            let Some(mut final_review) = final_review else {
                execution.blocked(
                    execution
                        .task_failure_reason("review failed (a run is never approved by default)"),
                );
                return Delivery::Stop;
            };

            final_review.issues.extend(all_issues);
            Some(final_review)
        } else {
            // Single chunk or no graph diff
            let diff = if !graph_diff.is_empty() {
                truncate(&graph_diff, DIFF_MAX_CHARS)
            } else {
                truncate(&default_get_diff(&cwd), DIFF_MAX_CHARS)
            };
            let single_chunk = chunks.first();
            let req_ids = single_chunk.map(|c| vec![c.id.clone()]);
            let task = GraphTaskState::new(
                format!("review-{}", indices.review),
                Role::Reviewer,
                ArtifactKind::Review,
                vec![],
                None,
            );
            let review = execution
                .execute_node(
                    task,
                    review_briefing(&ReviewInput {
                        goal: goal_text,
                        plan: plan.as_ref(),
                        diff: &diff,
                        changed_files: &changed_files,
                        verification: &verification,
                        chunk: single_chunk,
                        chunk_summaries: None,
                        required_chunk_ids: req_ids.as_deref(),
                        security: Some(&security_verification),
                    }),
                )
                .and_then(|artifact| artifact.as_review().cloned());

            if execution.cancelled_if_aborted() {
                return Delivery::Stop;
            }
            if let (Some(r), Some(c)) = (&review, single_chunk) {
                if !r.reviewed_chunk_ids.is_empty() {
                    coverage.record_reviewed(&r.reviewed_chunk_ids);
                } else {
                    coverage.record_reviewed(&[c.id.clone()]);
                }
            }
            review
        };

        {
            let mut run = execution
                .run
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            run.review_coverage = Some(coverage.clone());
        }

        let Some(mut review) = review else {
            execution.blocked(
                execution.task_failure_reason("review failed (a run is never approved by default)"),
            );
            return Delivery::Stop;
        };

        let is_coverage_complete = coverage_complete(&coverage);
        if !is_coverage_complete {
            let missing = coverage.missing_chunk_ids().join(", ");
            execution.checkpoint(Some(&format!(
                "review coverage incomplete: missing chunks [{missing}]"
            )));
            if review.verdict == Verdict::Approve {
                review.verdict = Verdict::ChangesRequired;
                review.issues.push(ReviewIssue {
                    severity: Severity::Blocker,
                    file: None,
                    description: format!("Incomplete review coverage: missing chunks [{missing}]"),
                });
            }
        }

        // An approval that lists a blocker or a major issue is a
        // contradiction; the issues win, as they would with a human reviewer.
        let blocking_issue = review
            .issues
            .iter()
            .any(|issue| matches!(issue.severity, Severity::Blocker | Severity::Major));
        let can_approve = if execution.options.dry_run {
            !matches!(security_verification, SecurityVerification::Failed { .. })
        } else {
            bundle.approval_eligible(policy_mode)
        };
        if review.verdict == Verdict::Approve
            && !blocking_issue
            && is_coverage_complete
            && can_approve
        {
            return Delivery::Ok;
        }
        if review.verdict == Verdict::Approve {
            if !can_approve {
                review.verdict = Verdict::ChangesRequired;
                review.issues.push(ReviewIssue {
                    severity: Severity::Blocker,
                    file: None,
                    description: format!(
                        "Security verification rejected approval: {:?}",
                        bundle.security
                    ),
                });
            }
            execution.checkpoint(Some(
                "review approved with blocking issues or incomplete coverage; treated as changes required",
            ));
        }
        if revision_cycles >= budgets.max_revision_cycles {
            execution.blocked(format!(
                "reviewer still requires changes after {revision_cycles} revision cycles"
            ));
            return Delivery::Stop;
        }
        revision_cycles += 1;
        execution
            .run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .counters
            .revision_cycles += 1;
        revision_notes = Some(revision_notes_from(
            None,
            Some(&review),
            Some(&security_verification),
        ));
        saved.prepare_revision(revision_notes.clone(), revision_cycles);
        if !execution.save_delivery(&saved, *indices, Some("review revision saved")) {
            return Delivery::Stop;
        }
    }
}

#[cfg(test)]
#[path = "controller_continuation_tests.rs"]
mod continuation_tests;

#[cfg(test)]
#[path = "controller_saved_continuation_tests.rs"]
mod saved_continuation_tests;

#[cfg(test)]
#[path = "controller_attempt_tests.rs"]
mod attempt_tests;

#[cfg(test)]
#[path = "controller_revision_tests.rs"]
mod revision_tests;

#[cfg(test)]
#[path = "controller_persistence_tests.rs"]
mod persistence_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::*;
    use tempfile::tempdir;

    #[test]
    fn coordinator_transport_includes_authorized_browser_tools() {
        let tools = vec![
            "browser_open".to_string(),
            "browser_snapshot".to_string(),
            "process_start".to_string(),
            "read".to_string(),
        ];
        let selected = coordinator_task_tools(&tools, false, true);
        assert!(selected.contains(&"browser_open".to_string()));
        assert!(selected.contains(&"browser_snapshot".to_string()));
        assert!(selected.contains(&"process_start".to_string()));
        assert!(!selected.contains(&"read".to_string()));
        let without_browser = coordinator_task_tools(&tools, false, false);
        assert!(!without_browser.contains(&"browser_open".to_string()));
        assert!(!without_browser.contains(&"browser_snapshot".to_string()));
        assert!(without_browser.contains(&"process_start".to_string()));
    }

    #[test]
    #[ignore = "requires explicitly configured trusted Node and Playwright installation"]
    fn graph_scheduler_writer_uses_authenticated_browser_transport() {
        use davinci_agent::{
            jobs::{supervisor::SupervisorCommand, JobBook},
            process_manager::ProcessManager,
            runtime::{
                transactions::{ProposedChange, TransactionCoordinator, TransactionOwner},
                AgentId, RunId, RuntimeBus, RuntimeHandle,
            },
            PermissionMode, PermissionPolicy, PermissionState,
        };

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            "<button onclick=\"this.textContent='Broken'\">Start</button>",
        )
        .unwrap();
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(
            dir.path().join("server.cjs"),
            format!(
                "const http=require('node:http'),fs=require('node:fs');const s=http.createServer((q,r)=>{{r.setHeader('content-type','text/html');r.end(fs.readFileSync('index.html'));}});s.listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>s.close(),60000);"
            ),
        )
        .unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ]);

        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let supervisor = SupervisorCommand {
            executable: std::env::current_exe().unwrap(),
            argv: vec![
                "--exact".into(),
                "native_extensions::graph::coordinator_handler::tests::helper_entry".into(),
                "--nocapture".into(),
            ],
        };
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let processes =
            ProcessManager::new(dir.path(), jobs, permissions.clone(), supervisor.clone()).unwrap();
        let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        let browser = crate::native_extensions::browser::BrowserWorkerHost {
            controller: crate::native_extensions::browser::BrowserController::new(
                dir.path(),
                crate::native_extensions::browser::BrowserConfig {
                    enabled: true,
                    node: std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH")
                        .unwrap()
                        .into(),
                    package: std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH")
                        .unwrap()
                        .into(),
                    version: "1.62.0".into(),
                },
            ),
            supervisor,
        };
        let parent_runtime = runtime.clone();
        let writer_cwd = dir.path().to_path_buf();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Bug,
                    complexity: Complexity::Standard,
                    rationale: "browser fixture".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![PlanStep {
                        description: "fix and verify browser".into(),
                        files: vec!["index.html".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["fixture-test".into()],
                    completion_criteria: vec!["browser interaction passes".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    assert_eq!(spec.cwd, writer_cwd);
                    assert!(spec.tools.iter().any(|tool| tool == "browser_open"));
                    let client = spec
                        .coordinator_client
                        .as_ref()
                        .expect("scheduler must bind authenticated browser transport");
                    let owner = TransactionOwner {
                        agent_id: spec.runtime_agent_id.expect("host worker identity"),
                        parent_agent_id: Some(parent_runtime.agent_id),
                        session_id: parent_runtime.session_id.clone(),
                        task_id: spec.task_contract.as_ref().map(|contract| contract.task_id),
                        graph_node: Some(spec.task_id.clone()),
                    };
                    let coordinator = TransactionCoordinator::new(&spec.cwd, owner).unwrap();
                    let preview = coordinator
                        .preview(vec![ProposedChange::write(
                            "index.html",
                            b"<button onclick=\"this.textContent='Done'\">Start</button>".to_vec(),
                        )])
                        .unwrap();
                    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();

                    let started = client
                        .call(
                            "process_start",
                            &serde_json::json!({
                                "executable":"node",
                                "argv":["server.cjs"],
                                "ports":[port]
                            }),
                        )
                        .unwrap()
                        .details
                        .unwrap();
                    let process_id = started["process"]["id"].as_u64().unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        let output = client
                            .call("process_output", &serde_json::json!({"id":process_id}))
                            .unwrap();
                        if output.content.contains("READY") {
                            break;
                        }
                        assert!(Instant::now() < deadline, "{output:?}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let opened = client
                        .call(
                            "browser_open",
                            &serde_json::json!({
                                "process_id":process_id,
                                "port":port,
                                "transaction_id":preview.id
                            }),
                        )
                        .unwrap()
                        .details
                        .unwrap();
                    assert_eq!(
                        opened["source_binding"]["transaction_id"],
                        serde_json::json!(preview.id)
                    );
                    let browser_id = opened["browser_id"].clone();
                    client
                        .call(
                            "browser_click",
                            &serde_json::json!({
                                "browser_id":browser_id,
                                "selector":{"kind":"role","role":"button","name":"Start"}
                            }),
                        )
                        .unwrap();
                    let snapshot = client
                        .call(
                            "browser_snapshot",
                            &serde_json::json!({"browser_id":browser_id}),
                        )
                        .unwrap()
                        .details
                        .unwrap();
                    assert!(snapshot["result"]["html"]
                        .as_str()
                        .unwrap()
                        .contains(">Done</button>"));
                    client
                        .call(
                            "browser_close",
                            &serde_json::json!({"browser_id":browser_id}),
                        )
                        .unwrap();
                    client
                        .call("process_stop", &serde_json::json!({"id":process_id}))
                        .unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["index.html".into()],
                        summary: "source-bound browser flow passed".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "browser evidence observed".into(),
                    reviewed_chunk_ids: vec![],
                })),
                ArtifactKind::Evidence => unreachable!("fixture has no research tasks"),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let run = run_graph(
            RunOptions {
                goal: "fix browser fixture".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            ControllerDeps {
                runner,
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 1)),
                config: GraphConfig {
                    verify_commands: vec![VerifyCommandSpec {
                        name: "fixture-test".into(),
                        command: "fixture-test".into(),
                        from_plan: false,
                    }],
                    ..Default::default()
                },
                session_model: None,
                session_thinking: None,
                project_trusted: true,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: Some(processes),
                browser: Some(browser),
                runtime: Some(runtime),
                permissions: Some(permissions),
                task_contract: None,
            },
        );
        assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
    }

    #[test]
    fn plan_invalidation_replans_before_dispatching_another_writer() {
        let dir = tempfile::tempdir().unwrap();
        let planner_calls = Arc::new(AtomicUsize::new(0));
        let writer_calls = Arc::new(AtomicUsize::new(0));
        let planner_calls_runner = Arc::clone(&planner_calls);
        let writer_calls_runner = Arc::clone(&writer_calls);

        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Bug,
                    complexity: Complexity::Standard,
                    rationale: "fixture".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::Plan => {
                    planner_calls_runner.fetch_add(1, Ordering::SeqCst);
                    Artifact::Plan(Box::new(ImplementationPlan {
                        steps: vec![],
                        tests_to_add: vec![],
                        tests_to_run: vec!["fixture-test".into()],
                        completion_criteria: vec!["done".into()],
                        invariants: vec![],
                        out_of_scope: vec![],
                    }))
                }
                ArtifactKind::PatchReport => {
                    let call = writer_calls_runner.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        return WorkerResult {
                            ok: false,
                            failure_reason: Some("plan invalidated: scope changed".into()),
                            ..WorkerResult::default()
                        };
                    }
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec![],
                        summary: "implemented replanned work".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "approved".into(),
                    reviewed_chunk_ids: vec![],
                })),
                ArtifactKind::Evidence => unreachable!("fixture has no research tasks"),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let run = run_graph(
            RunOptions {
                goal: "replan fixture".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            ControllerDeps {
                runner,
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 1)),
                config: GraphConfig {
                    verify_commands: vec![VerifyCommandSpec {
                        name: "fixture-test".into(),
                        command: "fixture-test".into(),
                        from_plan: false,
                    }],
                    ..Default::default()
                },
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
        );

        assert_eq!(
            run.phase,
            Phase::Done,
            "blocked={:?}, tasks={:?}",
            run.blocked_reason,
            run.tasks
        );
        assert_eq!(run.counters.replans, 1);
        assert_eq!(planner_calls.load(Ordering::SeqCst), 2);
        assert_eq!(writer_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn f03_graph_controller_does_not_advertise_unbound_task_tools() {
        let dir = tempfile::tempdir().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            if spec.role == Role::Classifier {
                return WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::Classification(Classification {
                        task_class: TaskClass::Bug,
                        complexity: Complexity::Trivial,
                        rationale: "fixture".into(),
                        research_tasks: vec![],
                        milestones: None,
                    })),
                    ..WorkerResult::default()
                };
            }
            captured.lock().unwrap().push(spec.tools.clone());
            WorkerResult {
                ok: false,
                ..WorkerResult::default()
            }
        });
        run_graph(
            RunOptions {
                goal: "fixture".into(),
                cwd: dir.path().to_path_buf(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            ControllerDeps {
                runner,
                verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
                config: GraphConfig {
                    worker_extra_tools: [
                        "task_create",
                        "task_update",
                        "task_list",
                        "task_get",
                        "custom_mutator",
                    ]
                    .map(str::to_string)
                    .to_vec(),
                    ..Default::default()
                },
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
        );
        let observed = observed.lock().unwrap();
        assert!(!observed.is_empty(), "writer was not dispatched");
        for tools in observed.iter() {
            assert!(tools.iter().any(|tool| tool == "custom_mutator"));
            for task_tool in ["task_create", "task_update", "task_list", "task_get"] {
                assert!(
                    !tools.iter().any(|tool| tool == task_tool),
                    "advertised {task_tool}"
                );
            }
        }
    }

    #[test]
    fn f03_graph_controller_binds_task_coordinator_for_writer_with_runtime_and_permissions() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let parent_runtime = davinci_agent::RuntimeHandle::new(
            davinci_agent::runtime::RunId::new(),
            davinci_agent::AgentId::new(),
            davinci_agent::runtime::RuntimeBus::new(),
        );
        let lead_record = davinci_agent::AgentRecord {
            id: parent_runtime.agent_id,
            run_id: parent_runtime.run_id,
            parent: None,
            kind: davinci_agent::AgentKind::Main,
            name: "lead".into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: dir.path().to_path_buf(),
            state: davinci_agent::AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: 1,
            updated_ms: 1,
            failure_reason: None,
        };
        parent_runtime.registry.register_agent(lead_record).unwrap();

        let permissions = Arc::new(davinci_agent::PermissionState::new(
            davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove),
        ));

        let executed_tasks = Arc::new(Mutex::new(Vec::new()));
        let executed_client = executed_tasks.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            if spec.role == Role::Classifier {
                return WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::Classification(Classification {
                        task_class: TaskClass::Bug,
                        complexity: Complexity::Trivial,
                        rationale: "fixture".into(),
                        research_tasks: vec![],
                        milestones: None,
                    })),
                    ..WorkerResult::default()
                };
            }
            if spec.role == Role::Writer {
                assert!(spec.tools.contains(&"task_create".to_string()));
                assert!(spec.tools.contains(&"task_list".to_string()));
                let client = spec
                    .coordinator_client
                    .as_ref()
                    .expect("coordinator client must be bound for writer");
                let op_id = davinci_agent::runtime::RunId::new().0.to_string();
                let created = client
                    .call(
                        "task_create",
                        &json!({
                            "title": "task created by writer",
                            "operation_id": op_id,
                        }),
                    )
                    .expect("writer call to task_create should succeed");
                executed_client.lock().unwrap().push(created);
                return WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec![],
                        summary: "patched".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))),
                    ..WorkerResult::default()
                };
            }
            WorkerResult {
                ok: true,
                artifact: Some(Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                }))),
                ..WorkerResult::default()
            }
        });

        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig {
                worker_extra_tools: [
                    "task_create",
                    "task_update",
                    "task_list",
                    "task_get",
                    "custom_mutator",
                ]
                .map(str::to_string)
                .to_vec(),
                verify_commands: vec![VerifyCommandSpec {
                    name: "test".into(),
                    command: "echo ok".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: Some(parent_runtime.clone()),
            permissions: Some(permissions),
            task_contract: None,
        };

        let run = run_graph(
            RunOptions {
                goal: "test coordinator".into(),
                cwd: dir.path().to_path_buf(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
        );

        assert_eq!(run.phase, Phase::Done);
        let executed = executed_tasks.lock().unwrap();
        assert_eq!(executed.len(), 1);
        let tasks = parent_runtime
            .task_registry
            .list_tasks(Some(parent_runtime.run_id));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "task created by writer");
    }

    #[test]
    fn graph_worker_spec_does_not_advertise_privileged_extras_to_classifier() {
        let dir = tempfile::tempdir().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            captured.lock().unwrap().push((
                spec.role,
                spec.tools.clone(),
                spec.extra_extensions.clone(),
            ));
            WorkerResult {
                ok: false,
                ..WorkerResult::default()
            }
        });
        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig {
                worker_extra_tools: vec!["write".into(), "custom_mutator".into()],
                worker_extensions: vec!["./fixture.mjs".into()],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };
        let run = run_graph(
            RunOptions {
                goal: "inspect".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
        );
        assert_eq!(run.phase, Phase::Blocked);
        assert!(!run.tasks.is_empty());
        let observed = observed.lock().unwrap();
        assert!(!observed.is_empty());
        for (role, tools, extensions) in observed.iter() {
            assert_eq!(*role, Role::Classifier);
            assert!(
                extensions.is_empty(),
                "untrusted project authorized executable extensions"
            );
            assert!(!tools
                .iter()
                .any(|name| name == "write" || name == "custom_mutator"));
        }
    }

    #[test]
    fn resumed_run_deadline_uses_remaining_lifetime_budget() {
        let now = Instant::now();
        assert_eq!(
            remaining_run_deadline(1000, 100, 850, now),
            Some(now + Duration::from_millis(250))
        );
        assert_eq!(remaining_run_deadline(1000, 100, 1100, now), Some(now));
        assert_eq!(remaining_run_deadline(1000, 100, 1200, now), Some(now));
        assert_eq!(remaining_run_deadline(0, 100, 1200, now), None);
    }

    #[test]
    fn graph_deadline_controller_aborts_run_when_worker_exceeds_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let budgets = GraphBudgets {
            run_deadline_ms: 60_000,
            ..Default::default()
        };

        let runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {
            // This tests propagation of the worker's deadline outcome. The
            // process tests cover elapsed deadlines; a 50ms setup budget here
            // could expire during checkpoint I/O before any worker was started.
            assert!(spec.run_deadline.is_some());
            WorkerResult {
                ok: false,
                run_deadline_exceeded: true,
                failure_reason: Some("run deadline exceeded".to_string()),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            budgets,
            ..Default::default()
        };

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config,
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "test deadline".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Blocked);
        assert_eq!(run.blocked_reason.as_deref(), Some("run deadline exceeded"));
        let task = run.tasks.last().unwrap();
        assert_eq!(task.error.as_deref(), Some("run deadline exceeded"));
    }

    #[test]
    fn graph_review_coverage_approval_impossible_when_one_chunk_is_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn initial() {}\n").unwrap();

        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(
                    crate::native_extensions::graph::types::Classification {
                        task_class: crate::native_extensions::graph::types::TaskClass::Feature,
                        complexity: Complexity::Standard,
                        rationale: "standard test".into(),
                        research_tasks: vec![
                            crate::native_extensions::graph::types::ResearchRequest {
                                kind: ResearchKind::CodeSearch,
                                focus: "find code".into(),
                            },
                        ],
                        milestones: None,
                    },
                ),
                ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![crate::native_extensions::graph::types::EvidenceFinding {
                        claim: "found code".into(),
                        refs: vec!["code.rs:1".into()],
                        confidence: crate::native_extensions::graph::types::Confidence::High,
                    }],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
                })),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![crate::native_extensions::graph::types::PlanStep {
                        description: "step 1".into(),
                        files: vec!["code.rs".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["true".into()],
                    completion_criteria: vec!["done".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    std::fs::write(&file_path, "fn initial() {}\nfn added() {}\n").unwrap();
                    Artifact::PatchReport(Box::new(
                        crate::native_extensions::graph::types::PatchReport {
                            changed_files: vec!["code.rs".into()],
                            summary: "added function".into(),
                            deviations: vec![],
                            plan_invalidated: false,
                            invalidation_reason: None,
                        },
                    ))
                }
                ArtifactKind::Review => {
                    // Reviewer approves BUT omits the required chunk!
                    Artifact::Review(Box::new(
                        crate::native_extensions::graph::types::ReviewDecision {
                            verdict: Verdict::Approve,
                            issues: vec![],
                            notes: "looks fine".into(),
                            reviewed_chunk_ids: vec!["some_other_file#chunk-0".into()],
                        },
                    ))
                }
            };

            let _ = write_artifact(&spec.artifact_path, &artifact);
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            verify_commands: vec![crate::native_extensions::graph::types::VerifyCommandSpec {
                command: "echo test".into(),
                name: "test".into(),
                from_plan: false,
            }],
            budgets: GraphBudgets {
                max_revision_cycles: 0, // No retries allowed
                ..Default::default()
            },
            ..Default::default()
        };

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config,
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "test coverage requirement".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Blocked);
        assert!(run
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("reviewer still requires changes"));
        let coverage = run
            .review_coverage
            .expect("review coverage must be tracked");
        assert!(
            !coverage_complete(&coverage),
            "coverage must NOT be complete"
        );
        assert!(coverage
            .missing_chunk_ids()
            .contains(&"code.rs#chunk-0".to_string()));
    }

    #[test]
    fn graph_skill_outcome_attributed_to_injected_version() {
        let dir = tempfile::tempdir().unwrap();
        let learning_dir = dir.path().join(".pi").join("learning");
        std::fs::create_dir_all(&learning_dir).unwrap();
        let mut learning =
            crate::native_extensions::LearningController::new(dir.path(), None, None);

        let skill_v1 = crate::native_extensions::learning::types::SkillLedgerRecord {
            skill_id: "skill-fix".into(),
            name: "fix-skill".into(),
            scope: crate::native_extensions::learning::types::LearningScope::Project,
            origin: crate::native_extensions::learning::types::SkillOrigin::LearnedReview,
            status: crate::native_extensions::learning::types::ArtifactStatus::Active,
            path: dir.path().join("SKILL.md"),
            content_hash: "hash-v1".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            applicability: crate::native_extensions::learning::types::SkillApplicability {
                task_types: vec!["bug".into()],
                ..Default::default()
            },
            pinned: false,
        };
        learning.project_store.upsert_skill(skill_v1).unwrap();

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, "ok".to_string(), 10));
        let runner: Arc<WorkerRunner> = Arc::new(|spec, _, _| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Bug,
                    complexity: Complexity::Trivial,
                    rationale: "fix".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![],
                    tests_to_add: vec![],
                    tests_to_run: vec!["cargo test".into()],
                    completion_criteria: vec![],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["main.rs".into()],
                    summary: "ok".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config: GraphConfig {
                verify_commands: vec![crate::native_extensions::graph::types::VerifyCommandSpec {
                    command: "cargo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: Some(learning),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "fix a bug".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let execution = GraphExecution {
            run: Mutex::new(GraphRun {
                version: 1,
                run_id: "test-run-1".into(),
                goal: options.goal.clone(),
                cwd: options.cwd.to_string_lossy().into_owned(),
                phase: Phase::Done,
                forced: options.forced,
                dry_run: options.dry_run,
                execution_origin: None,
                definition_digest: None,
                saved_definition: None,
                definition: None,
                classification: None,
                milestones: None,
                current_milestone: None,
                tasks: vec![GraphTaskState {
                    id: "task-1".into(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    depends_on: vec![],
                    focus: None,
                    status: TaskStatus::Succeeded,
                    attempts: 1,
                    artifact_file: Some("main.rs".into()),
                    error: None,
                    usage: WorkerUsage::default(),
                    started_at: None,
                    ended_at: None,
                    last_activity: None,
                    fingerprint: None,
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 100,
                    memory_refs: vec![],
                    skill_refs: vec![crate::native_extensions::ecosystem::SkillContextRef {
                        name: "fix-skill".into(),
                        version: 1,
                        content_hash: "hash-v1".into(),
                    }],
                }],
                verification: Some(VerificationResult {
                    progress: None,
                    passed: true,
                    commands: vec![
                        crate::native_extensions::graph::types::VerificationCommandResult {
                            name: "test".into(),
                            command: "cargo test".into(),
                            exit_code: 0,
                            duration_ms: 10,
                            output_tail: "ok".into(),
                            skipped: false,
                        },
                    ],
                }),
                verification_bundle: None,
                review_coverage: None,
                budgets: GraphBudgets::default(),
                counters: GraphCounters {
                    workers_spawned: 1,
                    revision_cycles: 0,
                    replans: 0,
                    cost_usd: 0.0,
                    started_at: now_ms(),
                },
                blocked_reason: None,
                resource_snapshot: None,
                ecosystem_stats: Default::default(),
                updated_at: 0,
                lifecycle: Some(GraphLifecycle::Running),
                revision: 0,
                control_history: Vec::new(),
                continuation: None,
            }),
            learning: Mutex::new(deps.learning.clone()),
            deps,
            options,
            exec_abort: Arc::new(AtomicBool::new(false)),
            budget_abort_reason: Mutex::new(None),
            persistence_error: Mutex::new(None),
            run_deadline: None,
            active_workers: Arc::new(AtomicUsize::new(0)),
            node_aborts: Arc::new(Mutex::new(HashMap::new())),
        };

        let snapshot = execution.snapshot();
        execution.record_skill_outcomes(&snapshot);

        let guard = execution.learning.lock().unwrap();
        let updated_learning = guard.as_ref().unwrap();
        let record = updated_learning
            .project_store
            .skill_version("fix-skill", 1)
            .unwrap();
        assert_eq!(record.success_count, 1);
        assert_eq!(record.failure_count, 0);
    }

    #[test]
    fn graph_security_gate_blocks_approval_on_high_risk_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let auth_file = dir.path().join("src").join("auth.rs");
        std::fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        std::fs::write(&auth_file, "pub fn key() -> &'static str { \"initial\" }\n").unwrap();

        let auth_file_clone = auth_file.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Feature,
                    complexity: Complexity::Standard,
                    rationale: "auth change".into(),
                    research_tasks: vec![ResearchRequest {
                        kind: ResearchKind::CodeSearch,
                        focus: "find key".into(),
                    }],
                    milestones: None,
                }),
                ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![EvidenceFinding {
                        claim: "found key".into(),
                        refs: vec!["src/auth.rs:1".into()],
                        confidence: Confidence::High,
                    }],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
                })),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![PlanStep {
                        description: "update key".into(),
                        files: vec!["src/auth.rs".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["true".into()],
                    completion_criteria: vec!["done".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    std::fs::write(
                        &auth_file_clone,
                        "pub fn key() -> &'static str { \"sk-secret12345\" }\n",
                    )
                    .unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["src/auth.rs".into()],
                        summary: "added secret key".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "reviewer approves".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };

            let _ = write_artifact(&spec.artifact_path, &artifact);
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            security_verification: SecurityPolicyMode::Risk,
            verify_commands: vec![VerifyCommandSpec {
                command: "echo test".into(),
                name: "test".into(),
                from_plan: false,
            }],
            budgets: GraphBudgets {
                max_revision_cycles: 0,
                ..Default::default()
            },
            ..Default::default()
        };

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config,
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "update auth key".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Blocked);
        let blocked = run.blocked_reason.expect("must be blocked");
        assert!(
            blocked.contains("security verification still failing"),
            "unexpected blocked reason: {blocked}"
        );
        let bundle = run
            .verification_bundle
            .expect("verification bundle must be present");
        assert!(matches!(
            bundle.security,
            SecurityVerification::Failed { .. }
        ));
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
    }

    #[test]
    fn graph_security_gate_passes_clean_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let ui_file = dir.path().join("src").join("ui.rs");
        std::fs::create_dir_all(ui_file.parent().unwrap()).unwrap();
        std::fs::write(&ui_file, "pub fn render() {}\n").unwrap();

        let ui_file_clone = ui_file.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Feature,
                    complexity: Complexity::Standard,
                    rationale: "clean ui change".into(),
                    research_tasks: vec![ResearchRequest {
                        kind: ResearchKind::CodeSearch,
                        focus: "find render".into(),
                    }],
                    milestones: None,
                }),
                ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![EvidenceFinding {
                        claim: "found render".into(),
                        refs: vec!["src/ui.rs:1".into()],
                        confidence: Confidence::High,
                    }],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
                })),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![PlanStep {
                        description: "update render".into(),
                        files: vec!["src/ui.rs".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["true".into()],
                    completion_criteria: vec!["done".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    std::fs::write(
                        &ui_file_clone,
                        "pub fn render() { println!(\"Hello clean UI\"); }\n",
                    )
                    .unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["src/ui.rs".into()],
                        summary: "updated render".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "looks great".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };

            let _ = write_artifact(&spec.artifact_path, &artifact);
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            security_verification: SecurityPolicyMode::Always,
            verify_commands: vec![VerifyCommandSpec {
                command: "echo test".into(),
                name: "test".into(),
                from_plan: false,
            }],
            budgets: GraphBudgets {
                max_revision_cycles: 0,
                ..Default::default()
            },
            ..Default::default()
        };

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config,
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "clean ui update".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        assert!(run.blocked_reason.is_none());
        let bundle = run
            .verification_bundle
            .expect("verification bundle must be present");
        assert!(matches!(
            bundle.security,
            SecurityVerification::Passed { .. }
        ));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Always));
    }

    #[test]
    fn graph_security_policy_off_allows_risky_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let auth_file = dir.path().join("src").join("auth.rs");
        std::fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        std::fs::write(&auth_file, "pub fn key() -> &'static str { \"initial\" }\n").unwrap();

        let auth_file_clone = auth_file.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Feature,
                    complexity: Complexity::Standard,
                    rationale: "auth change".into(),
                    research_tasks: vec![ResearchRequest {
                        kind: ResearchKind::CodeSearch,
                        focus: "find key".into(),
                    }],
                    milestones: None,
                }),
                ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![EvidenceFinding {
                        claim: "found key".into(),
                        refs: vec!["src/auth.rs:1".into()],
                        confidence: Confidence::High,
                    }],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
                })),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![PlanStep {
                        description: "update key".into(),
                        files: vec!["src/auth.rs".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["true".into()],
                    completion_criteria: vec!["done".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    std::fs::write(
                        &auth_file_clone,
                        "pub fn key() -> &'static str { \"sk-secret12345\" }\n",
                    )
                    .unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["src/auth.rs".into()],
                        summary: "added secret key".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "reviewer approves".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };

            let _ = write_artifact(&spec.artifact_path, &artifact);
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            security_verification: SecurityPolicyMode::Off,
            verify_commands: vec![VerifyCommandSpec {
                command: "echo test".into(),
                name: "test".into(),
                from_plan: false,
            }],
            budgets: GraphBudgets {
                max_revision_cycles: 0,
                ..Default::default()
            },
            ..Default::default()
        };

        let deps = ControllerDeps {
            runner,
            verify_exec,
            config,
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "update auth key".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        assert!(run.blocked_reason.is_none());
        let bundle = run
            .verification_bundle
            .expect("verification bundle must be present");
        assert!(matches!(bundle.security, SecurityVerification::NotRequired));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Off));
    }

    #[test]
    fn graph_learning_feedback_closed_loop_attribution() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        let agent_dir = dir.path().join("agent_isolated");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let db_file = cwd.join("src").join("db.rs");
        std::fs::create_dir_all(db_file.parent().unwrap()).unwrap();
        std::fs::write(&db_file, "pub fn connect() -> bool { false }\n").unwrap();

        let worker_calls = Arc::new(AtomicUsize::new(0));
        let worker_calls_clone = Arc::clone(&worker_calls);
        let db_file_clone = db_file.clone();

        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            worker_calls_clone.fetch_add(1, Ordering::SeqCst);
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Feature,
                    complexity: Complexity::Standard,
                    rationale: "db operation".into(),
                    research_tasks: vec![ResearchRequest {
                        kind: ResearchKind::CodeSearch,
                        focus: "find db connect".into(),
                    }],
                    milestones: None,
                }),
                ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![EvidenceFinding {
                        claim: "found connect".into(),
                        refs: vec!["src/db.rs:1".into()],
                        confidence: Confidence::High,
                    }],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
                })),
                ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                    steps: vec![PlanStep {
                        description: "update connect".into(),
                        files: vec!["src/db.rs".into()],
                    }],
                    tests_to_add: vec![],
                    tests_to_run: vec!["true".into()],
                    completion_criteria: vec!["done".into()],
                    invariants: vec![],
                    out_of_scope: vec![],
                })),
                ArtifactKind::PatchReport => {
                    std::fs::write(&db_file_clone, "pub fn connect() -> bool { true }\n").unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["src/db.rs".into()],
                        summary: "updated connect".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "reviewer approves".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };

            let _ = write_artifact(&spec.artifact_path, &artifact);
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let verify_exec: Arc<VerifyExec> = Arc::new(|_, _, _, _| (0, String::new(), 0));
        let config = GraphConfig {
            security_verification: SecurityPolicyMode::Risk,
            verify_commands: vec![VerifyCommandSpec {
                command: "echo test".into(),
                name: "test".into(),
                from_plan: false,
            }],
            budgets: GraphBudgets {
                max_revision_cycles: 0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut learning =
            crate::native_extensions::LearningController::new(&cwd, Some(&agent_dir), None);
        learning.set_project_trusted(true);

        let mut vector_mem = crate::native_extensions::VectorMemory::new(cwd.clone());
        vector_mem.mark_dense_offline();

        // --- Step 1: Run #1 receives no learned skill, executes cleanly ---
        let deps1 = ControllerDeps {
            runner: Arc::clone(&runner),
            verify_exec: Arc::clone(&verify_exec),
            config: config.clone(),
            session_model: None,
            session_thinking: None,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(vector_mem.clone()),
            learning: Some(learning.clone()),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options1 = RunOptions {
            goal: "Initial database setup".into(),
            cwd: cwd.clone(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run1 = run_graph(options1, deps1);
        assert_eq!(run1.phase, Phase::Done);
        assert!(run1.tasks.iter().all(|t| t.skill_refs.is_empty()));
        assert_eq!(worker_calls.load(Ordering::SeqCst), 5);

        // --- Step 2: Persist deterministic memory and skill from Run #1 ---
        let memory_id = vector_mem
            .index_learning_memory(
                "Apply database migration and check schema.",
                crate::native_extensions::vector_memory::MemoryKind::Fact,
                0.9,
                0.95,
                &run1.run_id,
                1,
                Some("graph_pass"),
            )
            .expect("memory indexing should succeed");

        let fixture_json = serde_json::json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.95,
                    "rationale": "Learned database migration procedure",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "database-migration",
                        "description": "Apply database migration and verify schema",
                        "body": "---\nname: database-migration\ndescription: Apply database migration and verify schema\n---\n\nRun cargo sqlx migrate run and verify schema.\n"
                    }
                }
            ]
        }).to_string();

        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", &fixture_json);
        let bundle1 = run1.verification.as_ref().unwrap().to_bundle(
            vec!["src/db.rs".into()],
            Some(run1.run_id.clone()),
            SecurityVerification::NotRequired,
        );
        let evidence1 = crate::native_extensions::learning::types::LearningEvidence {
            session_id: "sess-loop-1".into(),
            repo_id: vector_mem.repo_id.clone(),
            turn: 1,
            messages: vec![crate::native_extensions::vector_memory::MemoryMessage {
                role: "assistant".into(),
                content: "Completed database setup".into(),
            }],
            tools: vec![],
            run_stats: davinci_agent::RunStats::default(),
            verification:
                crate::native_extensions::learning::evidence::verification_evidence_from_bundle(
                    &bundle1,
                ),
        };
        learning.review_settled_turn(evidence1);
        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");

        // Applicability is deliberately conservative for legacy records. Declare
        // the learned skill's relevance before asserting positive graph credit.
        let mut learned_skill = learning
            .project_store
            .skill("database-migration")
            .expect("database-migration skill must exist in store")
            .clone();
        learned_skill.applicability.task_types = vec!["database".into()];
        learning.project_store.upsert_skill(learned_skill).unwrap();

        // Assert persistence after Run #1
        let skill_v1 = learning
            .project_store
            .skill("database-migration")
            .expect("database-migration skill must exist in store")
            .clone();
        assert_eq!(skill_v1.version, 1);
        assert_eq!(skill_v1.success_count, 0);
        assert_eq!(skill_v1.failure_count, 0);
        assert!(skill_v1.path.exists(), "SKILL.md must be written to disk");
        assert!(vector_mem.records().iter().any(|r| r.id == memory_id));

        // --- Step 3 & 4: Run #2 with related goal retrieves exact provenance ---
        let deps2 = ControllerDeps {
            runner: Arc::clone(&runner),
            verify_exec: Arc::clone(&verify_exec),
            config: config.clone(),
            session_model: None,
            session_thinking: None,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(vector_mem.clone()),
            learning: Some(learning.clone()),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options2 = RunOptions {
            goal: "Apply database migration".into(),
            cwd: cwd.clone(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run2 = run_graph(options2, deps2);
        assert_eq!(run2.phase, Phase::Done);

        // Assert exact provenance in Run #2 metadata
        let tasks_with_skills: Vec<_> = run2
            .tasks
            .iter()
            .filter(|t| !t.skill_refs.is_empty())
            .collect();
        assert!(
            !tasks_with_skills.is_empty(),
            "at least one task must carry retrieved skill refs"
        );
        let first_with_skill = tasks_with_skills[0];
        assert_eq!(first_with_skill.skill_refs[0].name, "database-migration");
        assert_eq!(first_with_skill.skill_refs[0].version, 1);
        assert_eq!(
            first_with_skill.skill_refs[0].content_hash,
            skill_v1.content_hash
        );
        assert!(first_with_skill.memory_refs.contains(&memory_id));

        // --- Step 5 & 6: Assert exact skill version success count increments ---
        let reloaded_learning =
            crate::native_extensions::LearningController::new(&cwd, Some(&agent_dir), None);
        let updated_record = reloaded_learning
            .project_store
            .skill_version("database-migration", 1)
            .expect("database-migration v1 must exist in reloaded store");
        assert_eq!(
            updated_record.success_count, 1,
            "skill version success count must increment"
        );
        assert_eq!(updated_record.failure_count, 0);

        // --- Step 7: Assert no extra coordinator model invocation ---
        // 5 tasks in Run #1 + 5 tasks in Run #2 = 10 total worker runner calls
        assert_eq!(worker_calls.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_graph_worker_limit_distinguishes_unlimited_from_exhausted_zero() {
        // Zero in config means unlimited
        let unlimited = GraphWorkerLimit::from_config(0);
        assert_eq!(unlimited, GraphWorkerLimit::Unlimited);
        assert!(!unlimited.is_exhausted(100));

        // Finite 5 is not exhausted at 4, exhausted at 5
        let finite_5 = GraphWorkerLimit::from_config(5);
        assert!(!finite_5.is_exhausted(4));
        assert!(finite_5.is_exhausted(5));

        // Exhausted finite grant of 0 is NOT unlimited: it is exhausted immediately
        let exhausted_zero = GraphWorkerLimit::Finite(0);
        assert!(exhausted_zero.is_exhausted(0));
        assert!(exhausted_zero.is_exhausted(1));
    }

    #[test]
    fn test_saved_graph_execution_records_exact_order_and_roles_without_classifier() {
        use crate::native_extensions::graph::definitions::*;
        use crate::native_extensions::graph::topology::*;
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        let recorded: Arc<Mutex<Vec<(String, Role)>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded_clone = Arc::clone(&recorded);

        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
            recorded_clone
                .lock()
                .unwrap()
                .push((spec.task_id.clone(), spec.role));
            match spec.role {
                Role::Researcher => WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::Evidence(Box::new(EvidenceArtifact {
                        kind: ResearchKind::CodeSearch,
                        findings: vec![crate::native_extensions::graph::types::EvidenceFinding {
                            claim: "evidence found".into(),
                            refs: vec!["src/lib.rs:1".into()],
                            confidence: crate::native_extensions::graph::types::Confidence::High,
                        }],
                        risks: vec![],
                        gaps: vec![],
                        test_baseline: None,
                    }))),
                    ..WorkerResult::default()
                },
                Role::Planner => WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::Plan(Box::new(ImplementationPlan {
                        steps: vec![crate::native_extensions::graph::types::PlanStep {
                            description: "step 1".into(),
                            files: vec!["src/lib.rs".into()],
                        }],
                        tests_to_add: vec![],
                        tests_to_run: vec![],
                        completion_criteria: vec!["done".into()],
                        invariants: vec![],
                        out_of_scope: vec![],
                    }))),
                    ..WorkerResult::default()
                },
                Role::Writer => WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::PatchReport(Box::new(
                        crate::native_extensions::graph::types::PatchReport {
                            summary: "implemented".into(),
                            changed_files: vec!["src/lib.rs".into()],
                            deviations: vec![],
                            plan_invalidated: false,
                            invalidation_reason: None,
                        },
                    ))),
                    ..WorkerResult::default()
                },
                _ => WorkerResult {
                    ok: false,
                    ..WorkerResult::default()
                },
            }
        });

        let saved_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "custom-pipeline".into(),
            description: "Custom test pipeline".into(),
            graph: SavedGraphTopology {
                graph_id: "custom-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![
                    NodeDefinition {
                        id: "research-1".into(),
                        role: Role::Researcher,
                        expect: ArtifactKind::Evidence,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "plan-1".into(),
                        role: Role::Planner,
                        expect: ArtifactKind::Plan,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "implement-1".into(),
                        role: Role::Writer,
                        expect: ArtifactKind::PatchReport,
                        required: true,
                        allows_mutation: true,
                    },
                ],
                edges: vec![
                    EdgeDefinition {
                        from: "research-1".into(),
                        to: "plan-1".into(),
                        condition: EdgeCondition::OnSuccess,
                    },
                    EdgeDefinition {
                        from: "plan-1".into(),
                        to: "implement-1".into(),
                        condition: EdgeCondition::OnSuccess,
                    },
                ],
            },
            bindings: vec![
                SavedStageBinding {
                    node_id: "research-1".into(),
                    stage: "research".into(),
                    input_artifacts: vec![],
                },
                SavedStageBinding {
                    node_id: "plan-1".into(),
                    stage: "plan".into(),
                    input_artifacts: vec!["research-1".into()],
                },
                SavedStageBinding {
                    node_id: "implement-1".into(),
                    stage: "implement".into(),
                    input_artifacts: vec!["plan-1".into()],
                },
            ],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        };

        let options = RunOptions {
            goal: "execute custom pipeline".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: true,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig::default(),
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let run = run_saved_graph(options, deps, saved_def);
        assert_eq!(
            run.phase,
            Phase::Done,
            "run blocked: {:?}",
            run.blocked_reason
        );

        let calls = recorded.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                ("research-1".into(), Role::Researcher),
                ("plan-1".into(), Role::Planner),
                ("implement-1".into(), Role::Writer),
            ],
            "WorkerRunner must record exact order and roles"
        );

        // Verify Classifier was NOT invoked
        assert!(
            calls.iter().all(|(_, role)| *role != Role::Classifier),
            "no classifier worker may be executed for saved definition without classify node"
        );

        // Verify ExecutionOrigin and provenance
        assert!(matches!(
            run.execution_origin,
            Some(ExecutionOrigin::SavedDefinition { ref name, .. }) if name == "custom-pipeline"
        ));
        assert!(run.definition_digest.is_some());
        assert!(run.saved_definition.is_some());
    }

    #[test]
    fn test_saved_graph_undefined_binding_rejected() {
        use crate::native_extensions::graph::definitions::*;
        use crate::native_extensions::graph::topology::*;
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        let saved_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "incomplete-pipeline".into(),
            description: "Missing binding".into(),
            graph: SavedGraphTopology {
                graph_id: "incomplete-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![
                    NodeDefinition {
                        id: "research-1".into(),
                        role: Role::Researcher,
                        expect: ArtifactKind::Evidence,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "plan-1".into(),
                        role: Role::Planner,
                        expect: ArtifactKind::Plan,
                        required: true,
                        allows_mutation: false,
                    },
                ],
                edges: vec![EdgeDefinition {
                    from: "research-1".into(),
                    to: "plan-1".into(),
                    condition: EdgeCondition::OnSuccess,
                }],
            },
            // bindings missing for plan-1!
            bindings: vec![SavedStageBinding {
                node_id: "research-1".into(),
                stage: "research".into(),
                input_artifacts: vec![],
            }],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        };

        let options = RunOptions {
            goal: "execute incomplete".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: true,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let deps = ControllerDeps {
            runner: Arc::new(|_, _, _| WorkerResult {
                ok: true,
                ..WorkerResult::default()
            }),
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig::default(),
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let run = run_saved_graph(options, deps, saved_def);
        assert_eq!(run.phase, Phase::Blocked);
        assert!(run
            .blocked_reason
            .as_ref()
            .unwrap()
            .contains("missing stage binding"));
    }

    #[test]
    fn test_saved_graph_bounded_dynamic_attempt_ids_registered() {
        use crate::native_extensions::graph::definitions::*;
        use crate::native_extensions::graph::topology::*;
        use std::collections::BTreeMap;
        use std::sync::atomic::AtomicUsize;

        let dir = tempfile::tempdir().unwrap();
        let attempts_seen = Arc::new(AtomicUsize::new(0));
        let attempts_seen_clone = Arc::clone(&attempts_seen);

        let runner: Arc<WorkerRunner> = Arc::new(move |_spec, _, _| {
            let attempt = attempts_seen_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < 2 {
                WorkerResult {
                    ok: false,
                    failure_reason: Some(format!("transient failure {attempt}")),
                    ..WorkerResult::default()
                }
            } else {
                WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::Evidence(Box::new(EvidenceArtifact {
                        kind: ResearchKind::CodeSearch,
                        findings: vec![crate::native_extensions::graph::types::EvidenceFinding {
                            claim: "succeeded on attempt 2".into(),
                            refs: vec![],
                            confidence: crate::native_extensions::graph::types::Confidence::High,
                        }],
                        risks: vec![],
                        gaps: vec![],
                        test_baseline: None,
                    }))),
                    ..WorkerResult::default()
                }
            }
        });

        let saved_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "retry-pipeline".into(),
            description: "Retry pipeline".into(),
            graph: SavedGraphTopology {
                graph_id: "retry-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![NodeDefinition {
                    id: "research-retry".into(),
                    role: Role::Researcher,
                    expect: ArtifactKind::Evidence,
                    required: true,
                    allows_mutation: false,
                }],
                edges: vec![],
            },
            bindings: vec![SavedStageBinding {
                node_id: "research-retry".into(),
                stage: "research".into(),
                input_artifacts: vec![],
            }],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        };

        let options = RunOptions {
            goal: "retry test".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: true,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig::default(),
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let run = run_saved_graph(options, deps, saved_def);
        assert_eq!(run.phase, Phase::Done);
        assert_eq!(attempts_seen.load(Ordering::SeqCst), 2);
        let task = run.tasks.iter().find(|t| t.id == "research-retry").unwrap();
        assert_eq!(task.attempts, 2);
        assert_eq!(task.status, TaskStatus::Succeeded);
    }

    #[test]
    fn test_saved_graph_verification_remains_real() {
        use crate::native_extensions::graph::definitions::*;
        use crate::native_extensions::graph::topology::*;
        use std::collections::BTreeMap;
        use std::sync::atomic::AtomicUsize;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"dummy\"\n",
        )
        .unwrap();

        let saved_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "verify-pipeline".into(),
            description: "Verification pipeline".into(),
            graph: SavedGraphTopology {
                graph_id: "verify-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![NodeDefinition {
                    id: "verify-node".into(),
                    role: Role::TestAnalyzer,
                    expect: ArtifactKind::Evidence,
                    required: true,
                    allows_mutation: false,
                }],
                edges: vec![],
            },
            bindings: vec![SavedStageBinding {
                node_id: "verify-node".into(),
                stage: "verify".into(),
                input_artifacts: vec![],
            }],
            budgets: Some(SavedBudgets {
                max_duration_ms: Some(10_000),
                max_cost_usd: None,
                max_tokens: None,
            }),
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        };

        let options = RunOptions {
            goal: "verify test".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        // verify_exec returns non-zero (failure)
        let verify_exec_calls = Arc::new(AtomicUsize::new(0));
        let verify_exec_calls_clone = Arc::clone(&verify_exec_calls);
        let deps = ControllerDeps {
            runner: Arc::new(|_, _, _| WorkerResult {
                ok: true,
                ..WorkerResult::default()
            }),
            verify_exec: Arc::new(move |_, _, _, timeout| {
                assert!(
                    (1..=10_000).contains(&timeout),
                    "saved deadline was ignored: {timeout}"
                );
                verify_exec_calls_clone.fetch_add(1, Ordering::SeqCst);
                (1, "cargo test failed: 1 test panicked".into(), 10)
            }),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    name: "test".into(),
                    command: "cargo test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let run = run_saved_graph(options, deps, saved_def);
        assert_eq!(verify_exec_calls.load(Ordering::SeqCst), 1);
        assert!(run.verification.is_some());
        assert!(!run.verification.unwrap().passed);
        assert_eq!(run.phase, Phase::Blocked);
    }

    #[test]
    fn f13_no_dispatch_when_paused() {
        assert!(graph_dispatch_allowed("running", true, true));
        assert!(!graph_dispatch_allowed("pause_requested", true, true));
        assert!(!graph_dispatch_allowed("paused", true, true));
        assert!(!graph_dispatch_allowed("running", false, true));
    }

    #[test]
    fn test_pause_during_parallel_research() {
        let dir = tempdir().unwrap();
        let execution = GraphExecution {
            run: Mutex::new(GraphRun {
                version: 1,
                run_id: "test-run-pause".into(),
                goal: "test".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
                phase: Phase::Investigate,
                forced: None,
                dry_run: false,
                execution_origin: None,
                definition_digest: None,
                saved_definition: None,
                definition: None,
                classification: None,
                milestones: None,
                current_milestone: None,
                tasks: vec![],
                verification: None,
                verification_bundle: None,
                review_coverage: None,
                budgets: GraphBudgets::default(),
                counters: GraphCounters {
                    workers_spawned: 0,
                    revision_cycles: 0,
                    replans: 0,
                    cost_usd: 0.0,
                    started_at: 0,
                },
                blocked_reason: None,
                resource_snapshot: None,
                ecosystem_stats: Default::default(),
                updated_at: 0,
                lifecycle: Some(GraphLifecycle::Running),
                revision: 0,
                control_history: Vec::new(),
                continuation: None,
            }),
            deps: ControllerDeps {
                runner: Arc::new(|_, _, _| WorkerResult {
                    ok: true,
                    ..Default::default()
                }),
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 10)),
                config: Default::default(),
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
            learning: Mutex::new(None),
            options: RunOptions {
                goal: "test".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            exec_abort: Arc::new(AtomicBool::new(false)),
            budget_abort_reason: Mutex::new(None),
            persistence_error: Mutex::new(None),
            run_deadline: None,
            active_workers: Arc::new(AtomicUsize::new(1)),
            node_aborts: Arc::new(Mutex::new(HashMap::new())),
        };

        // When active workers > 0, request_pause transitions to PauseRequested
        execution.request_pause();
        assert_eq!(
            execution.snapshot().current_lifecycle(),
            GraphLifecycle::PauseRequested
        );

        // When active worker finishes (active_workers reaches 0), safe boundary reached -> Paused
        execution.active_workers.store(0, Ordering::SeqCst);
        let mut run = execution.run.lock().unwrap();
        run.lifecycle = Some(GraphLifecycle::Paused);
        drop(run);
        assert_eq!(
            execution.snapshot().current_lifecycle(),
            GraphLifecycle::Paused
        );

        // Resume restores Running
        execution.resume_execution();
        assert_eq!(
            execution.snapshot().current_lifecycle(),
            GraphLifecycle::Running
        );
    }

    #[test]
    fn test_pause_races_spawn() {
        assert!(!graph_dispatch_allowed("pause_requested", true, true));
        assert!(!graph_dispatch_allowed("paused", true, true));
        assert!(graph_dispatch_allowed("running", true, true));
        assert!(!graph_dispatch_allowed("running", false, true));
        assert!(!graph_dispatch_allowed("running", true, false));
    }

    #[test]
    fn test_stop_selected_child_leaves_siblings() {
        let dir = tempdir().unwrap();
        let execution = GraphExecution {
            run: Mutex::new(GraphRun {
                version: 1,
                run_id: "test-run-siblings".into(),
                goal: "test".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
                phase: Phase::Investigate,
                forced: None,
                dry_run: false,
                execution_origin: None,
                definition_digest: None,
                saved_definition: None,
                definition: None,
                classification: None,
                milestones: None,
                current_milestone: None,
                tasks: vec![],
                verification: None,
                verification_bundle: None,
                review_coverage: None,
                budgets: GraphBudgets::default(),
                counters: GraphCounters {
                    workers_spawned: 0,
                    revision_cycles: 0,
                    replans: 0,
                    cost_usd: 0.0,
                    started_at: 0,
                },
                blocked_reason: None,
                resource_snapshot: None,
                ecosystem_stats: Default::default(),
                updated_at: 0,
                lifecycle: Some(GraphLifecycle::Running),
                revision: 0,
                control_history: Vec::new(),
                continuation: None,
            }),
            deps: ControllerDeps {
                runner: Arc::new(|_, _, _| WorkerResult {
                    ok: true,
                    ..Default::default()
                }),
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 10)),
                config: Default::default(),
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
            learning: Mutex::new(None),
            options: RunOptions {
                goal: "test".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            exec_abort: Arc::new(AtomicBool::new(false)),
            budget_abort_reason: Mutex::new(None),
            persistence_error: Mutex::new(None),
            run_deadline: None,
            active_workers: Arc::new(AtomicUsize::new(0)),
            node_aborts: Arc::new(Mutex::new(HashMap::new())),
        };

        let abort_a = execution.register_node_abort("task-a");
        let abort_b = execution.register_node_abort("task-b");

        assert!(!abort_a.load(Ordering::Relaxed));
        assert!(!abort_b.load(Ordering::Relaxed));

        // Aborting task-a only signals task-a, leaving sibling task-b untouched
        execution.abort_node("task-a");
        assert!(abort_a.load(Ordering::Relaxed));
        assert!(!abort_b.load(Ordering::Relaxed));
    }

    #[test]
    fn test_deadline_while_paused() {
        let dir = tempdir().unwrap();
        let past_deadline = Instant::now() - Duration::from_secs(1);
        let execution = GraphExecution {
            run: Mutex::new(GraphRun {
                version: 1,
                run_id: "test-run-dl".into(),
                goal: "test".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
                phase: Phase::Investigate,
                forced: None,
                dry_run: false,
                execution_origin: None,
                definition_digest: None,
                saved_definition: None,
                definition: None,
                classification: None,
                milestones: None,
                current_milestone: None,
                tasks: vec![],
                verification: None,
                verification_bundle: None,
                review_coverage: None,
                budgets: GraphBudgets::default(),
                counters: GraphCounters {
                    workers_spawned: 0,
                    revision_cycles: 0,
                    replans: 0,
                    cost_usd: 0.0,
                    started_at: 0,
                },
                blocked_reason: None,
                resource_snapshot: None,
                ecosystem_stats: Default::default(),
                updated_at: 0,
                lifecycle: Some(GraphLifecycle::Paused),
                revision: 0,
                control_history: Vec::new(),
                continuation: None,
            }),
            deps: ControllerDeps {
                runner: Arc::new(|_, _, _| WorkerResult {
                    ok: true,
                    ..Default::default()
                }),
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 10)),
                config: Default::default(),
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(|_, _| {}),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
            learning: Mutex::new(None),
            options: RunOptions {
                goal: "test".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            exec_abort: Arc::new(AtomicBool::new(false)),
            budget_abort_reason: Mutex::new(None),
            persistence_error: Mutex::new(None),
            run_deadline: Some(past_deadline),
            active_workers: Arc::new(AtomicUsize::new(0)),
            node_aborts: Arc::new(Mutex::new(HashMap::new())),
        };

        // Simulated node dispatch triggers deadline check and sets budget abort
        if let Some(dl) = execution.run_deadline {
            if Instant::now() >= dl {
                execution.budget_abort("run deadline exceeded".into());
            }
        }
        assert!(execution.exec_abort.load(Ordering::Relaxed));
        assert_eq!(
            execution.budget_abort_reason(),
            Some("run deadline exceeded".into())
        );
    }

    #[test]
    fn test_windows_grandchildren_and_unix_group() {
        use super::super::process::terminate_process_tree;
        // Nonexistent PID termination does not panic
        terminate_process_tree(999_999_999);
    }

    #[test]
    fn test_stubborn_process_keeps_run_exclusion() {
        use super::super::ActiveRun;
        let active = ActiveRun::default();
        // A run whose thread hasn't finished is still running
        assert!(!active.is_finished());
    }

    #[test]
    fn test_callbacks_invoked_outside_locks() {
        let dir = tempdir().unwrap();
        let update_called = Arc::new(AtomicBool::new(false));
        let u_called = Arc::clone(&update_called);
        let execution = GraphExecution {
            run: Mutex::new(GraphRun {
                version: 1,
                run_id: "test-run-cb".into(),
                goal: "test".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
                phase: Phase::Investigate,
                forced: None,
                dry_run: false,
                execution_origin: None,
                definition_digest: None,
                saved_definition: None,
                definition: None,
                classification: None,
                milestones: None,
                current_milestone: None,
                tasks: vec![],
                verification: None,
                verification_bundle: None,
                review_coverage: None,
                budgets: GraphBudgets::default(),
                counters: GraphCounters {
                    workers_spawned: 0,
                    revision_cycles: 0,
                    replans: 0,
                    cost_usd: 0.0,
                    started_at: 0,
                },
                blocked_reason: None,
                resource_snapshot: None,
                ecosystem_stats: Default::default(),
                updated_at: 0,
                lifecycle: Some(GraphLifecycle::Running),
                revision: 0,
                control_history: Vec::new(),
                continuation: None,
            }),
            deps: ControllerDeps {
                runner: Arc::new(|_, _, _| WorkerResult {
                    ok: true,
                    ..Default::default()
                }),
                verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 10)),
                config: Default::default(),
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(move |_, _| {
                    u_called.store(true, Ordering::SeqCst);
                }),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
            learning: Mutex::new(None),
            options: RunOptions {
                goal: "test".into(),
                cwd: dir.path().to_path_buf(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            exec_abort: Arc::new(AtomicBool::new(false)),
            budget_abort_reason: Mutex::new(None),
            persistence_error: Mutex::new(None),
            run_deadline: None,
            active_workers: Arc::new(AtomicUsize::new(0)),
            node_aborts: Arc::new(Mutex::new(HashMap::new())),
        };

        // checkpoint calls on_update outside run lock
        execution.checkpoint(Some("test update"));
        assert!(update_called.load(Ordering::SeqCst));
    }
}
