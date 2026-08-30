//! The GraphController: a deterministic state machine over typed artifacts.
//!
//! Every routing decision here is plain code. Model calls happen ONLY inside
//! `deps.runner` (one isolated pi process per node). If you find yourself
//! wanting to ask a model "what should happen next", the answer belongs in
//! this file as an `if` instead.

use super::briefings::{
    build_evidence_digest, classify_briefing, implement_briefing, milestone_goal, plan_briefing,
    research_briefing, review_briefing, revision_notes_from, role_system_prompt, ClassifyInput,
    ReviewInput,
};
use super::config::{detect_verify_commands, read_package_scripts, GraphConfig};
use super::roles::{role_for_research_kind, role_tools};
use super::store::{
    artifact_path, create_run_dir, new_run_id, now_ms, save_run, transcript_path, write_artifact,
    write_log,
};
use super::types::{
    Artifact, ArtifactKind, Complexity, EvidenceArtifact, GraphBudgets, GraphCounters, GraphRun,
    GraphTaskState, ImplementationPlan, Phase, ResearchKind, Role, TaskStatus, Verdict,
    VerificationResult, WorkerSpec, WorkerUsage,
};
use super::verify::{collect_verify_commands, run_verification, CollectInput, VerifyExec};
use super::worker::WorkerRunner;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const EVIDENCE_DIGEST_MAX_CHARS: usize = 24_000;
const DIFF_MAX_CHARS: usize = 60_000;
const NODE_ATTEMPTS: u32 = 2;

pub type UpdateSink = dyn Fn(&GraphRun, Option<&str>) + Send + Sync;

pub struct ControllerDeps {
    pub runner: Arc<WorkerRunner>,
    pub verify_exec: Arc<VerifyExec>,
    pub config: GraphConfig,
    pub session_model: Option<String>,
    pub session_thinking: Option<String>,
    pub project_trusted: bool,
    pub on_update: Arc<UpdateSink>,
}

pub struct RunOptions {
    pub goal: String,
    pub cwd: PathBuf,
    pub forced: Option<Complexity>,
    pub dry_run: bool,
    /// Set by the operator (`/graph-abort`) or by session shutdown.
    pub abort: Arc<AtomicBool>,
    /// Artifacts from a previous run of the same goal, keyed by task id. A task
    /// whose id has a cached artifact is reused without spawning a worker (its
    /// original usage is credited so totals stay cumulative); the controller
    /// replays deterministically until the first uncached task, then continues
    /// live. Verification always re-runs.
    pub resume_artifacts: HashMap<String, (Artifact, WorkerUsage)>,
}

pub fn default_is_git_repo(cwd: &Path) -> bool {
    cwd.join(".git").exists()
}

pub fn default_get_diff(cwd: &Path) -> String {
    Command::new("git")
        .arg("diff")
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
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

pub struct GraphExecution {
    run: Mutex<GraphRun>,
    deps: ControllerDeps,
    options: RunOptions,
    /// Watched by every child process: the operator's abort, a budget abort,
    /// or a session shutdown all funnel here.
    exec_abort: Arc<AtomicBool>,
    budget_abort_reason: Mutex<Option<String>>,
}

impl GraphExecution {
    fn checkpoint(&self, note: Option<&str>) {
        // Persist under the lock, but report on a clone with the guard dropped:
        // a slow `on_update` must not serialize every worker thread, and an
        // implementation that re-locks the run must not deadlock.
        let snapshot = {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            let _ = save_run(&mut run);
            run.clone()
        };
        (self.deps.on_update)(&snapshot, note);
    }

    fn snapshot(&self) -> GraphRun {
        self.run
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
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
            return Some(format!(
                "run deadline exceeded ({} minutes)",
                budgets.run_deadline_ms / 60_000
            ));
        }
        // max_workers is sized for one deliverable; a decomposed run gets that
        // allowance per milestone. Cost cap and deadline stay global on purpose.
        let milestones = run.milestones.as_ref().map(Vec::len).unwrap_or(1).max(1) as u32;
        let worker_cap = budgets.max_workers.saturating_mul(milestones);
        if worker_cap > 0 && run.counters.workers_spawned >= worker_cap {
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
            task.status = status;
            task.ended_at = Some(now_ms());
            task.last_activity = None;
            if error.is_some() {
                task.error = error;
            }
        }
    }

    fn worker_spec(&self, task: &GraphTaskState, briefing: String) -> WorkerSpec {
        let run = self.snapshot();
        let role = task.role;
        let configured_model = self.deps.config.models.get(&role).cloned();
        let mut tools = role_tools(role);
        tools.extend(self.deps.config.worker_extra_tools.iter().cloned());
        WorkerSpec {
            task_id: task.id.clone(),
            role,
            expect: task.expect,
            briefing,
            system_prompt: role_system_prompt(role),
            cwd: PathBuf::from(&run.cwd),
            model: configured_model
                .clone()
                .or_else(|| self.deps.session_model.clone()),
            thinking_level: configured_model
                .is_none()
                .then(|| self.deps.session_thinking.clone())
                .flatten(),
            tools,
            extra_extensions: self.deps.config.worker_extensions.clone(),
            timeout_ms: run.budgets.worker_timeout_ms.get(role),
            artifact_path: artifact_path(Path::new(&run.cwd), &run.run_id, &task.id),
            transcript_path: Some(transcript_path(Path::new(&run.cwd), &run.run_id, &task.id)),
            project_trusted: self.deps.project_trusted,
        }
    }

    fn execute_node(&self, task: GraphTaskState, briefing: String) -> Option<Artifact> {
        let task_id = task.id.clone();
        let role = task.role;
        {
            let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            let unmet = run.unmet_dependencies(&task);
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
            run.tasks.push(task.clone());
        }

        if let Some((artifact, usage)) = self.options.resume_artifacts.get(&task_id) {
            {
                let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                let cwd = PathBuf::from(&run.cwd);
                let run_id = run.run_id.clone();
                if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                    task.started_at = Some(now_ms());
                    task.artifact_file = Some(format!("artifacts/{task_id}.json"));
                }
                let _ = write_artifact(&artifact_path(&cwd, &run_id, &task_id), artifact);
            }
            self.add_usage(&task_id, usage);
            self.end_task(&task_id, TaskStatus::Succeeded, None);
            self.checkpoint(Some(&format!("{task_id}: reused from previous run")));
            return Some(artifact.clone());
        }

        let mut last_failure: Option<String> = None;
        for attempt in 1..=NODE_ATTEMPTS {
            if self.exec_abort.load(Ordering::Relaxed) {
                self.end_task(&task_id, TaskStatus::Cancelled, None);
                self.checkpoint(None);
                return None;
            }
            // Budget check and spawn accounting share one critical section so
            // parallel research threads cannot all pass a stale check before
            // any of them records its own spawn.
            let over_budget = {
                let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                match Self::budget_exceeded(&run, now_ms()) {
                    Some(reason) => Some(reason),
                    None => {
                        run.counters.workers_spawned += 1;
                        if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                            task.status = TaskStatus::Running;
                            task.attempts = attempt;
                            task.started_at.get_or_insert_with(now_ms);
                        }
                        None
                    }
                }
            };
            if let Some(reason) = over_budget {
                self.end_task(&task_id, TaskStatus::Cancelled, Some(reason));
                self.checkpoint(None);
                return None;
            }
            self.checkpoint(Some(&format!("{task_id}: attempt {attempt} ({role})")));

            let timed_out_before = last_failure
                .as_ref()
                .is_some_and(|failure| failure.contains("timed out"));
            let attempt_briefing = if attempt == 1 {
                briefing.clone()
            } else if timed_out_before {
                format!(
                    "{briefing}\n\nRETRY NOTICE: the previous worker ran out of time before submitting. \
                     You have twice the time now, but work economically: rely on the evidence already in this \
                     briefing, avoid repository-wide searches, and call graph_submit well before the deadline."
                )
            } else {
                format!(
                    "{briefing}\n\nRETRY NOTICE: the previous worker exited without a valid submitted artifact. \
                     Complete the work and call graph_submit exactly once before stopping."
                )
            };

            let mut spec = self.worker_spec(&task, attempt_briefing);
            // A retry after a timeout gets double time — but only when a
            // timeout was configured at all; 0 stays unlimited.
            if timed_out_before && spec.timeout_ms > 0 {
                spec.timeout_ms *= 2;
            }

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
                        if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                            task.last_activity = Some(line.to_string());
                        }
                    }
                    (self.deps.on_update)(&self.snapshot(), Some(&format!("{task_id}: {line}")));
                };
                (self.deps.runner)(&spec, &self.exec_abort, &mut on_progress)
            };
            let trailing = {
                let reported = reported.lock().unwrap_or_else(|error| error.into_inner());
                WorkerUsage::delta(&result.usage, &reported)
            };
            self.add_usage(&task_id, &trailing);

            let run = self.snapshot();
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

            if let Some(reason) = self.budget_abort_reason() {
                self.end_task(&task_id, TaskStatus::Cancelled, Some(reason));
                self.checkpoint(Some(&format!("{task_id}: stopped by budget")));
                return None;
            }
            if result.ok {
                if let Some(artifact) = result.artifact {
                    {
                        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                        if let Some(task) = run.tasks.iter_mut().find(|entry| entry.id == task_id) {
                            task.artifact_file = Some(format!("artifacts/{task_id}.json"));
                        }
                    }
                    self.end_task(&task_id, TaskStatus::Succeeded, None);
                    self.checkpoint(Some(&format!("{task_id}: succeeded")));
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
                }
            }
            last_failure = Some(error);
            self.checkpoint(Some(&format!("{task_id}: attempt {attempt} failed")));
        }
        self.end_task(&task_id, TaskStatus::Failed, None);
        self.checkpoint(Some(&format!(
            "{task_id}: failed after {NODE_ATTEMPTS} attempts"
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

pub fn run_graph(options: RunOptions, deps: ControllerDeps) -> GraphRun {
    let run_id = new_run_id();
    let _ = create_run_dir(&options.cwd, &run_id);
    let budgets: GraphBudgets = deps.config.budgets.clone();
    let run = GraphRun {
        version: 1,
        run_id: run_id.clone(),
        goal: options.goal.clone(),
        cwd: options.cwd.to_string_lossy().into_owned(),
        phase: Phase::Classify,
        forced: options.forced,
        dry_run: options.dry_run,
        classification: None,
        milestones: None,
        current_milestone: None,
        tasks: Vec::new(),
        verification: None,
        budgets,
        counters: GraphCounters {
            workers_spawned: 0,
            revision_cycles: 0,
            replans: 0,
            cost_usd: 0.0,
            started_at: now_ms(),
        },
        blocked_reason: None,
        updated_at: 0,
    };

    let exec_abort = Arc::new(AtomicBool::new(options.abort.load(Ordering::Relaxed)));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = spawn_abort_watcher(
        Arc::clone(&options.abort),
        Arc::clone(&exec_abort),
        Arc::clone(&finished),
    );

    let execution = GraphExecution {
        run: Mutex::new(run),
        deps,
        options,
        exec_abort,
        budget_abort_reason: Mutex::new(None),
    };
    execution.checkpoint(Some("run created"));
    let result = drive(&execution);
    finished.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    result
}

fn drive(execution: &GraphExecution) -> GraphRun {
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
    let classification = execution.execute_node(
        classify_task,
        classify_briefing(&ClassifyInput {
            goal: &goal,
            is_git_repo,
            package_scripts: read_package_scripts(&cwd),
            max_researchers,
        }),
    );
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

    let mut evidence_digest = String::new();
    if complexity != Complexity::Trivial {
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
        evidence_digest = build_evidence_digest(
            &evidences
                .into_inner()
                .unwrap_or_else(|error| error.into_inner()),
            &failed_kinds
                .into_inner()
                .unwrap_or_else(|error| error.into_inner()),
            EVIDENCE_DIGEST_MAX_CHARS,
        );
    }

    let mut indices = NodeIndices::default();
    let goals: Vec<String> = if milestones.len() > 1 && complexity != Complexity::Trivial {
        (0..milestones.len())
            .map(|index| milestone_goal(&goal, &milestones, index))
            .collect()
    } else {
        vec![goal.clone()]
    };
    let milestone_count = goals.len();

    for (index, goal_text) in goals.iter().enumerate() {
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

#[derive(Default)]
struct NodeIndices {
    plan: u32,
    implement: u32,
    review: u32,
}

fn produce_plan(
    execution: &GraphExecution,
    indices: &mut NodeIndices,
    goal_text: &str,
    evidence_digest: &str,
    replan_reason: Option<&str>,
) -> Option<ImplementationPlan> {
    indices.plan += 1;
    let task = GraphTaskState::new(
        format!("plan-{}", indices.plan),
        Role::Planner,
        ArtifactKind::Plan,
        vec![],
        None,
    );
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
    let mut plan: Option<ImplementationPlan> = None;
    let mut revision_cycles = 0;
    let mut replans = 0;

    if complexity != Complexity::Trivial {
        execution.set_phase(Phase::Plan);
        execution.checkpoint(None);
        plan = produce_plan(execution, indices, goal_text, evidence_digest, None);
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }
        if plan.is_none() {
            execution.blocked(execution.task_failure_reason("planning failed"));
            return Delivery::Stop;
        }
    }

    let mut revision_notes: Option<String> = None;
    loop {
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }

        execution.set_phase(Phase::Implement);
        indices.implement += 1;
        let task = GraphTaskState::new(
            format!("implement-{}", indices.implement),
            Role::Writer,
            ArtifactKind::PatchReport,
            vec![],
            None,
        );
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
            execution.blocked(execution.task_failure_reason("implementation failed"));
            return Delivery::Stop;
        };

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
            execution.checkpoint(Some("replanning"));
            plan = produce_plan(
                execution,
                indices,
                goal_text,
                evidence_digest,
                Some(&reason),
            );
            if execution.cancelled_if_aborted() {
                return Delivery::Stop;
            }
            if plan.is_none() {
                execution.blocked(execution.task_failure_reason("replanning failed"));
                return Delivery::Stop;
            }
            revision_notes = None;
            continue;
        }

        execution.set_phase(Phase::Verify);
        execution.checkpoint(None);
        let cwd = execution.options.cwd.clone();
        let commands = collect_verify_commands(&CollectInput {
            config_commands: &execution.deps.config.verify_commands,
            detected: &detect_verify_commands(&cwd),
            plan: plan.as_ref(),
        });
        let verification: VerificationResult = run_verification(
            &commands,
            &cwd,
            &execution.exec_abort,
            budgets.verify_command_timeout_ms,
            execution.deps.verify_exec.as_ref(),
        );
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
            revision_notes = Some(revision_notes_from(Some(&verification), None));
            continue;
        }

        if complexity == Complexity::Trivial {
            return Delivery::Ok;
        }

        execution.set_phase(Phase::Review);
        indices.review += 1;
        let diff = truncate(&default_get_diff(&cwd), DIFF_MAX_CHARS);
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
                    changed_files: &patch.changed_files,
                    verification: &verification,
                }),
            )
            .and_then(|artifact| artifact.as_review().cloned());
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }
        let Some(review) = review else {
            execution.blocked(
                execution.task_failure_reason("review failed (a run is never approved by default)"),
            );
            return Delivery::Stop;
        };

        if review.verdict == Verdict::Approve {
            return Delivery::Ok;
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
        revision_notes = Some(revision_notes_from(None, Some(&review)));
    }
}
