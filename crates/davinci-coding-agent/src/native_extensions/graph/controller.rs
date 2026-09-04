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
use super::mutation::{capture_baseline, capture_graph_delta, GraphMutation};
use super::replay::{incompatibility_reason, replay_compatible, ReplayFingerprint};
use super::review_coverage::{chunk_graph_mutation, coverage_complete, ReviewCoverage};
use super::roles::{role_for_research_kind, role_tools};
use super::store::{
    artifact_path, create_run_dir, new_run_id, now_ms, save_run, transcript_path, write_artifact,
    write_graph_definition, write_log, write_task_fingerprint, write_task_mutation,
};
use super::topology::{
    build_definition, ready_nodes, validate_definition, GraphMode, GraphRunState,
};
use super::types::{
    Artifact, ArtifactKind, Complexity, EvidenceArtifact, GraphBudgets, GraphCounters, GraphRun,
    GraphTaskState, ImplementationPlan, Phase, ResearchKind, ReviewIssue, Role, Severity,
    TaskStatus, Verdict, VerificationResult, WorkerSpec, WorkerUsage,
};
use super::verify::{
    collect_verify_commands, nothing_ran, run_verification, CollectInput, VerifyExec,
};
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
    pub resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
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

pub struct GraphExecution {
    run: Mutex<GraphRun>,
    deps: ControllerDeps,
    options: RunOptions,
    /// Watched by every child process: the operator's abort, a budget abort,
    /// or a session shutdown all funnel here.
    exec_abort: Arc<AtomicBool>,
    budget_abort_reason: Mutex<Option<String>>,
    run_deadline: Option<std::time::Instant>,
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
            return Some("run deadline exceeded".to_string());
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
            run_deadline: self.run_deadline,
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

            if let Some(def) = &run.definition {
                if def.node(&task_id).is_some() {
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

            run.tasks.push(task.clone());
        }

        if let Some((artifact, usage, stored_fingerprint)) =
            self.options.resume_artifacts.get(&task_id)
        {
            let (run_version, cwd) = {
                let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                (
                    run.definition
                        .as_ref()
                        .map(|d| d.version)
                        .unwrap_or(run.version),
                    PathBuf::from(&run.cwd),
                )
            };
            let current_fingerprint =
                ReplayFingerprint::for_task(&cwd, run_version, &briefing, task.expect);

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
                    let run_id = run.run_id.clone();
                    if let Some(task_entry) = run.tasks.iter_mut().find(|entry| entry.id == task_id)
                    {
                        task_entry.started_at = Some(now_ms());
                        task_entry.artifact_file = Some(format!("artifacts/{task_id}.json"));
                        task_entry.fingerprint = stored_fingerprint.clone();
                    }
                    let _ = write_artifact(&artifact_path(&cwd, &run_id, &task_id), artifact);
                    if let Some(fp) = stored_fingerprint {
                        let _ = write_task_fingerprint(&cwd, &run_id, &task_id, fp);
                    }
                }
                self.add_usage(&task_id, usage);
                self.end_task(&task_id, TaskStatus::Succeeded, None);
                self.checkpoint(Some(&format!("{task_id}: reused from previous run")));
                return Some(artifact.clone());
            }
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
            if result.ok {
                if let Some(artifact) = result.artifact {
                    let (run_version, cwd, run_id) = {
                        let mut run = self.run.lock().unwrap_or_else(|error| error.into_inner());
                        let run_version = run
                            .definition
                            .as_ref()
                            .map(|d| d.version)
                            .unwrap_or(run.version);
                        let cwd = PathBuf::from(&run.cwd);
                        let run_id = run.run_id.clone();
                        let fingerprint =
                            ReplayFingerprint::for_task(&cwd, run_version, &briefing, task.expect);
                        if let Some(task_entry) =
                            run.tasks.iter_mut().find(|entry| entry.id == task_id)
                        {
                            task_entry.artifact_file = Some(format!("artifacts/{task_id}.json"));
                            task_entry.fingerprint = Some(fingerprint.clone());
                        }
                        (run_version, cwd, run_id)
                    };
                    let fingerprint =
                        ReplayFingerprint::for_task(&cwd, run_version, &briefing, task.expect);
                    let _ = write_task_fingerprint(&cwd, &run_id, &task_id, &fingerprint);
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
    let run_deadline = (budgets.run_deadline_ms > 0)
        .then(|| std::time::Instant::now() + Duration::from_millis(budgets.run_deadline_ms));
    let run = GraphRun {
        version: 1,
        run_id: run_id.clone(),
        goal: options.goal.clone(),
        cwd: options.cwd.to_string_lossy().into_owned(),
        phase: Phase::Classify,
        forced: options.forced,
        dry_run: options.dry_run,
        definition: None,
        classification: None,
        milestones: None,
        current_milestone: None,
        tasks: Vec::new(),
        verification: None,
        review_coverage: None,
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
        run_deadline,
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
        run.definition = Some(definition.clone());
        if complexity != Complexity::Trivial && milestones.len() > 1 {
            run.milestones = Some(milestones.clone());
        }
    }
    let run_id = execution.snapshot().run_id;
    let _ = write_graph_definition(&cwd, &run_id, &definition);
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
    let cwd = PathBuf::from(&execution.options.cwd);
    let milestone_baseline = capture_baseline(&cwd).unwrap_or_default();
    #[allow(unused_assignments)]
    let mut cumulative_delta = GraphMutation::default();
    loop {
        if execution.cancelled_if_aborted() {
            return Delivery::Stop;
        }

        execution.set_phase(Phase::Implement);
        indices.implement += 1;
        let task_id = format!("implement-{}", indices.implement);
        let task = GraphTaskState::new(
            task_id.clone(),
            Role::Writer,
            ArtifactKind::PatchReport,
            vec![],
            None,
        );
        let attempt_baseline = capture_baseline(&cwd).unwrap_or_default();
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

        let attempt_delta = capture_graph_delta(&cwd, &attempt_baseline).unwrap_or_default();
        cumulative_delta = capture_graph_delta(&cwd, &milestone_baseline).unwrap_or_default();
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
            let _ = write_task_mutation(&cwd, &run_id, &task_id, &attempt_delta);
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
        let mut verification: VerificationResult = run_verification(
            &commands,
            &cwd,
            &execution.exec_abort,
            budgets.verify_command_timeout_ms,
            execution.deps.verify_exec.as_ref(),
        );
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
            revision_notes = Some(revision_notes_from(Some(&verification), None));
            continue;
        }

        if complexity == Complexity::Trivial {
            return Delivery::Ok;
        }

        execution.set_phase(Phase::Review);
        indices.review += 1;
        let graph_diff = cumulative_delta.diff();
        let changed_files: Vec<String> = if !cumulative_delta.files.is_empty() {
            cumulative_delta
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect()
        } else {
            patch.changed_files.clone()
        };

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
        if review.verdict == Verdict::Approve && !blocking_issue && is_coverage_complete {
            return Delivery::Ok;
        }
        if review.verdict == Verdict::Approve {
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
        revision_notes = Some(revision_notes_from(None, Some(&review)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::WorkerResult;

    #[test]
    fn graph_deadline_controller_aborts_run_when_worker_exceeds_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let budgets = GraphBudgets {
            run_deadline_ms: 50,
            ..Default::default()
        };

        let runner: Arc<WorkerRunner> = Arc::new(|_spec, _abort, _on_progress| {
            std::thread::sleep(Duration::from_millis(60));
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
        };

        let options = RunOptions {
            goal: "test deadline".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
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
        };

        let options = RunOptions {
            goal: "test coverage requirement".into(),
            cwd: dir.path().to_path_buf(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
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
}
