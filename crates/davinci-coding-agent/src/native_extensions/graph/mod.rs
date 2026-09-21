//! Typed graph-engineer runtime: run a coding task as an explicit execution
//! graph of isolated, least-privileged worker processes.
//!
//! ```text
//! /graph <goal> [--simple|--complex] [--dry-run]   start a run in the background
//! /graph                                            watch, or continue if stopped
//! ```
//!
//! The controller is deterministic code: model calls happen only inside worker
//! child processes, one per node. Artifacts are the only data that crosses a
//! node boundary, and every artifact is schema-validated before it is accepted.
//!
//! Budgets that bound time or money are OFF by default — a run continues until
//! it finishes, blocks, or the operator aborts it. See [`types::GraphBudgets`].

pub(crate) mod bindings;
pub(crate) mod briefings;
pub(crate) mod config;
pub(crate) mod control;
pub(crate) mod controller;
mod coordinator_handler;
pub(crate) mod definitions;
pub(crate) mod export;
pub(crate) mod history;
mod lease;
pub(crate) mod mutation;
pub(crate) mod operations;
pub(crate) mod preflight;
pub(crate) mod process;
pub(crate) mod recovery;
pub(crate) mod render;
pub(crate) mod replay;
pub(crate) mod review_coverage;
pub(crate) mod roles;
pub(crate) mod store;
pub(crate) mod topology;
pub(crate) mod types;
pub(crate) mod validate;
pub(crate) mod verify;
pub(crate) mod worker;
pub(crate) mod worker_hooks;

#[allow(unused_imports)]
pub use control::*;
#[allow(unused_imports)]
pub use mutation::{
    capture_baseline, capture_graph_delta, ChangedFile, FileFingerprint, GraphMutation,
    MutationBaseline, PatchChunk,
};
#[allow(unused_imports)]
pub use recovery::{
    build_retry_context_delta, classify_worker_failure, retry_decision, RetryDecision,
    WorkerFailureClass, RETRY_CONTEXT_DELTA_TOKENS,
};
#[allow(unused_imports)]
pub use replay::{incompatibility_reason, replay_compatible, ReplayFingerprint};
#[allow(unused_imports)]
pub use review_coverage::{chunk_graph_mutation, coverage_complete, ReviewChunk, ReviewCoverage};
#[allow(unused_imports)]
pub use topology::{
    build_definition, ready_nodes, validate_definition, EdgeCondition, EdgeDefinition,
    GraphDefinition, GraphMode, GraphTopologyError, NodeDefinition,
};
pub use types::*;

use crate::native_extensions::ecosystem::verification::{SecurityPolicyMode, SecurityVerification};
use config::load_config;
#[allow(unused_imports)]
pub use controller::{run_graph, run_saved_graph, ControllerDeps, RunOptions};
use davinci_agent::{ToolError, ToolResult};
#[allow(unused_imports)]
pub use render::{
    graph_command_kind, parse_graph_args, parse_graph_command, render_now, render_run_summary,
    GraphCommand, ParsedGraphArgs,
};
use serde_json::{json, Value};
use store::{list_runs, load_run, transcript_path};
use verify::{contracted_verify_exec, default_verify_exec, dry_run_verify_exec};
use worker::{run_dry_worker, run_worker};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

pub use roles::GRAPH_SUBMIT_TOOL;
pub use worker_hooks::GraphWorkerContext;

/// How long `/graph` waits for the first checkpoint so it can report the run id.
const START_REPORT_WAIT: Duration = Duration::from_millis(2000);

/// How long session shutdown waits for aborted run threads to leave before
/// letting the process exit with them detached.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub(crate) struct ActiveRun {
    abort: Arc<AtomicBool>,
    snapshot: Mutex<Option<GraphRun>>,
    /// Set once the run thread has left, whatever its outcome. A run whose
    /// thread died without reaching a terminal phase is finished all the same.
    finished: AtomicBool,
    /// The background run thread, joined on session shutdown.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    pub(crate) control_tracker: Mutex<control::ControlTracker>,
}

impl ActiveRun {
    pub(crate) fn snapshot(&self) -> Option<GraphRun> {
        self.snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

/// Flags a run finished when the thread that owns it unwinds or returns.
struct FinishedOnDrop(Arc<ActiveRun>);

impl Drop for FinishedOnDrop {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Relaxed);
    }
}

/// Active runs are process-wide, not per-controller: the CLI builds a fresh
/// extension host for every slash command, so opening `/graph` again must find
/// the run that `/graph <goal>` started.
fn active_runs() -> &'static Mutex<HashMap<PathBuf, Arc<ActiveRun>>> {
    static ACTIVE: OnceLock<Mutex<HashMap<PathBuf, Arc<ActiveRun>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn active_run(cwd: &Path) -> Option<Arc<ActiveRun>> {
    let runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    runs.get(cwd).cloned().filter(|run| {
        run.snapshot()
            .map(|run| !run.phase.as_str().eq("done"))
            .unwrap_or(true)
    })
}

fn is_running(cwd: &Path) -> bool {
    active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(cwd)
        .map(|run| {
            // No snapshot yet means the run thread has not reached its first
            // checkpoint — starting still counts as running. A run whose
            // abort was requested is running until its thread has left:
            // its workers are still being terminated.
            !run.is_finished()
                && run
                    .snapshot()
                    .map(|snapshot| is_live(&snapshot))
                    .unwrap_or(true)
        })
        .unwrap_or(false)
}

/// `/graph <goal>` and `graph_run` refuse while a run is live. One
/// that is stopping is still live: starting another would put two runs on
/// the same working tree.
fn refuse_if_active(cwd: &Path) -> Result<(), String> {
    if !is_running(cwd) {
        return Ok(());
    }
    let stopping = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(cwd)
        .is_some_and(|run| run.abort.load(Ordering::Relaxed));
    Err(if stopping {
        "The previous graph run is still stopping; wait a moment and retry.".into()
    } else {
        "A graph run is already active. Open /graph to watch it first.".into()
    })
}

/// Make `active` the project's run, joining the thread of the finished run
/// it replaces so nothing is left dangling.
fn register_run(cwd: &Path, active: Arc<ActiveRun>) {
    let previous = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(cwd.to_path_buf(), active);
    if let Some(previous) = previous {
        if previous.is_finished() {
            if let Some(handle) = previous
                .handle
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = handle.join();
            }
        }
    }
}

/// Stop every run this process started. Called on session shutdown so a
/// background run never outlives the session that asked for it.
pub fn abort_all_runs() {
    let runs: Vec<Arc<ActiveRun>> = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .cloned()
        .collect();
    for run in &runs {
        run.abort.store(true, Ordering::Relaxed);
    }
    // Bounded wait for the run threads (whose child processes watch the abort
    // flag) to leave, so exiting the CLI does not strand live workers.
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    for run in &runs {
        while !run.finished.load(Ordering::Relaxed) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        if run.finished.load(Ordering::Relaxed) {
            if let Some(handle) = run
                .handle
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = handle.join();
            }
        }
    }
}

fn is_live(run: &GraphRun) -> bool {
    !matches!(
        run.phase,
        types::Phase::Done | types::Phase::Blocked | types::Phase::Cancelled
    )
}

fn saved_definition_from_run(
    run: &GraphRun,
    name: &str,
) -> Result<definitions::SavedGraphDefinitionV1, String> {
    if let Some(def) = &run.saved_definition {
        let mut def = def.clone();
        def.name = name.to_string();
        return Ok(def);
    }
    let Some(graph_def) = &run.definition else {
        return Err(format!(
            "Run '{}' does not contain a validated graph definition to export.",
            run.run_id
        ));
    };
    let topology = definitions::SavedGraphTopology {
        graph_id: format!("{name}-graph"),
        version: graph_def.version,
        mode: graph_def.mode,
        nodes: graph_def.nodes.clone(),
        edges: graph_def.edges.clone(),
    };
    let bindings = graph_def
        .nodes
        .iter()
        .map(|node| definitions::SavedStageBinding {
            node_id: node.id.clone(),
            stage: node.role.to_string(),
            input_artifacts: vec![],
        })
        .collect();
    Ok(definitions::SavedGraphDefinitionV1 {
        schema_version: 1,
        name: name.to_string(),
        description: format!("Exported from run {}", run.run_id),
        graph: topology,
        bindings,
        budgets: Some(definitions::SavedBudgets {
            max_duration_ms: (run.budgets.run_deadline_ms > 0)
                .then_some(run.budgets.run_deadline_ms),
            max_cost_usd: (run.budgets.max_cost_usd > 0.0).then_some(run.budgets.max_cost_usd),
            max_tokens: None,
        }),
        verification_policy: Some(definitions::SavedVerificationPolicy {
            command_profile: "default".into(),
            required: true,
        }),
        artifact_contract_versions: std::collections::BTreeMap::new(),
        parameters: vec![],
    })
}

#[derive(Debug, Clone)]
pub struct GraphController {
    cwd: PathBuf,
    session_model: Option<String>,
    session_thinking: Option<String>,
    session_role_models: Option<std::collections::BTreeMap<Role, String>>,
    project_trusted: bool,
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

impl Default for GraphController {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl GraphController {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            session_model: None,
            session_thinking: None,
            session_role_models: None,
            project_trusted: false,
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_runtime(mut self, runtime: davinci_agent::RuntimeHandle) -> Self {
        self.runtime = Some(runtime);
        self
    }

    #[allow(dead_code)]
    pub fn set_runtime(&mut self, runtime: Option<davinci_agent::RuntimeHandle>) {
        self.runtime = runtime;
    }

    #[allow(dead_code)]
    pub fn with_permissions(mut self, permissions: Arc<davinci_agent::PermissionState>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    #[allow(dead_code)]
    pub fn set_permissions(&mut self, permissions: Option<Arc<davinci_agent::PermissionState>>) {
        if let Some(language) = &self.language_intelligence {
            language.set_permissions(permissions.clone());
        }
        self.permissions = permissions;
    }

    pub fn set_task_contract(&mut self, contract: Option<davinci_agent::runtime::TaskContract>) {
        self.task_contract = contract;
    }

    /// Workers inherit the session's model and thinking level unless the
    /// project pins a per-role model in `.pi/graph.json`.
    pub fn set_session_context(
        &mut self,
        model: Option<String>,
        thinking: Option<String>,
        project_trusted: bool,
    ) {
        self.session_model = model;
        self.session_thinking = thinking;
        self.project_trusted = project_trusted;
    }

    pub fn role_models(&self) -> std::collections::BTreeMap<Role, String> {
        self.session_role_models.clone().unwrap_or_else(|| {
            if self.project_trusted {
                load_config(&self.cwd).config.models
            } else {
                Default::default()
            }
        })
    }

    /// Explicit interactive choices override project defaults for this session.
    pub fn set_role_models(&mut self, models: std::collections::BTreeMap<Role, String>) {
        self.session_role_models = Some(models);
    }

    /// Returns the dependencies plus any `graph.json` complaints worth showing.
    fn deps(&self, dry_run: bool, active: &Arc<ActiveRun>) -> (ControllerDeps, Vec<String>) {
        // A malformed graph.json is reported, then ignored: the run proceeds
        // on defaults rather than refusing to start.
        // Repository configuration is not explicit user authorization to run
        // extensions or verification commands. Ignore it until trust is granted.
        let mut loaded = if self.project_trusted {
            load_config(&self.cwd)
        } else {
            config::LoadedConfig::default()
        };
        if let Some(models) = &self.session_role_models {
            loaded.config.models = models.clone();
        }
        let sink = Arc::clone(active);
        let deps = ControllerDeps {
            runner: if dry_run {
                Arc::new(run_dry_worker)
            } else {
                Arc::new(run_worker)
            },
            verify_exec: if dry_run {
                Arc::new(dry_run_verify_exec)
            } else {
                match self.task_contract.is_some() {
                    true => Arc::new(contracted_verify_exec),
                    false => Arc::new(default_verify_exec),
                }
            },
            config: loaded.config,
            session_model: self.session_model.clone(),
            session_thinking: self.session_thinking.clone(),
            project_trusted: self.project_trusted,
            on_update: Arc::new(move |run: &GraphRun, _note: Option<&str>| {
                *sink
                    .snapshot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(run.clone());
            }),
            memory: self.memory.clone(),
            learning: self.learning.clone(),
            governor: self.governor.clone(),
            language_intelligence: self.language_intelligence.clone(),
            processes: self.processes.clone(),
            browser: self.browser.clone(),
            runtime: self.runtime.clone(),
            permissions: self.permissions.clone(),
            task_contract: self.task_contract.clone(),
        };
        (deps, loaded.errors)
    }

    fn options(
        &self,
        parsed: &ParsedGraphArgs,
        abort: Arc<AtomicBool>,
        resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
        resume_run: Option<Box<types::GraphRun>>,
    ) -> RunOptions {
        RunOptions {
            goal: parsed.goal.clone(),
            cwd: self.cwd.clone(),
            forced: parsed.forced,
            dry_run: parsed.dry_run,
            abort,
            resume_artifacts,
            resume_run,
        }
    }

    /// Start a run on a background thread and report what we know so far.
    fn start_background(
        &self,
        parsed: ParsedGraphArgs,
        resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
        resume_run: Option<Box<types::GraphRun>>,
        workspace_lease: Option<lease::WorkspaceLease>,
    ) -> Result<Value, String> {
        if parsed.goal.trim().is_empty() {
            return Err("Usage: /graph <goal> [--simple|--complex] [--dry-run]".into());
        }
        refuse_if_active(&self.cwd)?;
        let workspace_lease = workspace_lease
            .map(Ok)
            .unwrap_or_else(|| lease::WorkspaceLease::acquire(&self.cwd))?;
        let active = Arc::new(ActiveRun::default());
        let (deps, config_errors) = self.deps(parsed.dry_run, &active);
        let options = self.options(
            &parsed,
            Arc::clone(&active.abort),
            resume_artifacts,
            resume_run,
        );
        let resumed = options.resume_artifacts.len();
        register_run(&self.cwd, Arc::clone(&active));

        let finished = Arc::clone(&active);
        let handle = thread::spawn(move || {
            // Marks the run finished however this thread leaves — normal exit
            // or a panic — so a crashed run cannot wedge `/graph` behind a
            // permanent "already active" for this project.
            let _guard = FinishedOnDrop(finished);
            let _ = controller::run_graph_owned(options, deps, None, workspace_lease);
        });
        active
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(handle);

        // Wait briefly for the first checkpoint so the operator gets a run id
        // to poll rather than an anonymous "started".
        let deadline = Instant::now() + START_REPORT_WAIT;
        let mut snapshot = active.snapshot();
        while snapshot.is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
            snapshot = active.snapshot();
        }
        Ok(json!({
            "started": true,
            "goal": parsed.goal,
            "dryRun": parsed.dry_run,
            "resumedTasks": resumed,
            "runId": snapshot.as_ref().map(|run| run.run_id.clone()),
            "configErrors": config_errors,
            "status": snapshot.as_ref().map(render_now).unwrap_or_default(),
        }))
    }

    /// Run to completion on the calling thread and return the full summary.
    /// Used by the `graph_run` tool, where the caller wants the outcome, not a
    /// handle to poll.
    fn run_to_completion(&self, parsed: ParsedGraphArgs) -> Result<GraphRun, String> {
        if parsed.goal.trim().is_empty() {
            return Err("graph goal cannot be empty".into());
        }
        refuse_if_active(&self.cwd)?;
        let workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
        let active = Arc::new(ActiveRun::default());
        let (deps, _config_errors) = self.deps(parsed.dry_run, &active);
        let options = self.options(&parsed, Arc::clone(&active.abort), HashMap::new(), None);
        register_run(&self.cwd, Arc::clone(&active));
        let _guard = FinishedOnDrop(Arc::clone(&active));
        Ok(controller::run_graph_owned(
            options,
            deps,
            None,
            workspace_lease,
        ))
    }

    fn resume(&self, wanted: &str) -> Result<Value, String> {
        let workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
        let runs = list_runs(&self.cwd);
        let summary = if wanted.is_empty() {
            runs.iter()
                .find(|run| run.phase != "done" && run.phase != "cancelled")
                .or_else(|| runs.first())
        } else {
            runs.iter().find(|run| run.run_id == wanted)
        };
        let Some(summary) = summary else {
            return Err(if wanted.is_empty() {
                "No graph runs in this project.".to_string()
            } else {
                format!("No graph run \"{wanted}\" in this project.")
            });
        };
        let Some(mut old_run) = load_run(&self.cwd, &summary.run_id) else {
            return Err(format!("Could not load state for run {}.", summary.run_id));
        };
        if old_run.phase == types::Phase::Done {
            return Err(format!(
                "Run {} already finished (done). Start a new /graph instead.",
                old_run.run_id
            ));
        }

        // Reconcile running workers using restored_worker_state so crashed "running" workers
        // become "reconciliation_required" unless live identity is verified.
        let mut reconciled = false;
        for task in &mut old_run.tasks {
            let state = store::restored_worker_state(task.status.as_str(), false);
            if state == "reconciliation_required" {
                task.status = types::TaskStatus::Failed;
                task.error =
                    Some("worker stopped unexpectedly; reconciliation required".to_string());
                reconciled = true;
            }
        }
        if reconciled && old_run.current_lifecycle() == types::GraphLifecycle::Running {
            old_run.lifecycle = Some(types::GraphLifecycle::RecoveryRequired);
            store::save_run(&mut old_run)
                .map_err(|e| format!("Failed to persist reconciled run: {e}"))?;
            return Ok(json!({
                "resumed": false,
                "reconciliationRequired": true,
                "runId": old_run.run_id,
                "status": render_now(&old_run),
                "message": "Run workers crashed and require reconciliation before resuming.",
            }));
        }

        // Bare /graph reopens an explicitly Paused run without auto-resuming it
        if wanted.is_empty() && old_run.current_lifecycle() == types::GraphLifecycle::Paused {
            return Ok(json!({
                "resumed": false,
                "paused": true,
                "runId": old_run.run_id,
                "status": render_now(&old_run),
                "message": "Run is paused. Use /graph-control or 'p' to resume.",
            }));
        }
        // A run that revised or replanned holds several succeeded plan-N /
        // implement-N / review-N attempts, and the resumed run numbers its own
        // nodes from 1 again — so replaying by task id would hand back the very
        // attempt that verification or review rejected. Only investigation
        // results, which no later attempt supersedes, are safe to reuse there.
        let superseded = old_run.counters.revision_cycles > 0 || old_run.counters.replans > 0;
        let mut resume_artifacts = HashMap::new();
        for task in &old_run.tasks {
            if task.status != types::TaskStatus::Succeeded || task.artifact_file.is_none() {
                continue;
            }
            let is_investigation = task.id == "classify" || task.id.starts_with("research-");
            if superseded && !is_investigation {
                continue;
            }
            if let Ok(artifact) =
                store::read_artifact(&self.cwd, &old_run.run_id, &task.id, task.expect)
            {
                let fp = task
                    .fingerprint
                    .clone()
                    .or_else(|| store::read_task_fingerprint(&self.cwd, &old_run.run_id, &task.id));
                resume_artifacts.insert(task.id.clone(), (artifact, task.usage, fp));
            }
        }
        // A dry run resumes as a dry run: its canned artifacts must never be
        // replayed as real node outputs in front of real verification.
        self.start_background(
            ParsedGraphArgs {
                goal: old_run.goal.clone(),
                forced: old_run.forced,
                dry_run: old_run.dry_run,
            },
            resume_artifacts,
            Some(Box::new(old_run)),
            Some(workspace_lease),
        )
    }

    fn status(&self, run_id: Option<&str>) -> Value {
        let active = active_run(&self.cwd).and_then(|run| run.snapshot());
        let explicit = run_id
            .filter(|id| !id.is_empty())
            .and_then(|id| load_run(&self.cwd, id));
        let current = explicit.or_else(|| {
            active.clone().or_else(|| {
                list_runs(&self.cwd)
                    .first()
                    .and_then(|s| load_run(&self.cwd, &s.run_id))
            })
        });
        let recent: Vec<Value> = list_runs(&self.cwd)
            .into_iter()
            .take(5)
            .map(|summary| {
                json!({
                    "runId": summary.run_id,
                    "phase": summary.phase,
                    "goal": summary.goal,
                    "costUsd": summary.cost_usd,
                    "workersSpawned": summary.workers_spawned,
                })
            })
            .collect();
        json!({
            "active": active.is_some() && is_running(&self.cwd),
            "run": current,
            "status": current.as_ref().map(render_now).unwrap_or_default(),
            "summary": current.as_ref().map(render_run_summary),
            "recent": recent,
        })
    }

    fn view(&self, task_id: &str) -> Value {
        let Some(run) = active_run(&self.cwd)
            .and_then(|run| run.snapshot())
            .or_else(|| {
                list_runs(&self.cwd)
                    .first()
                    .and_then(|summary| load_run(&self.cwd, &summary.run_id))
            })
        else {
            return json!({"error": "No graph runs in this project."});
        };
        if run.tasks.is_empty() {
            return json!({"error": "The run has no worker tasks yet.", "runId": run.run_id});
        }
        let tasks: Vec<Value> = run
            .tasks
            .iter()
            .map(|task| {
                json!({
                    "id": task.id,
                    "role": task.role,
                    "status": task.status,
                    "usage": task.usage,
                    "lastActivity": task.last_activity,
                })
            })
            .collect();
        let chosen = run
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .or_else(|| {
                task_id
                    .is_empty()
                    .then(|| {
                        run.tasks
                            .iter()
                            .rev()
                            .find(|task| task.started_at.is_some())
                    })
                    .flatten()
            });
        let Some(chosen) = chosen else {
            return json!({"runId": run.run_id, "tasks": tasks,
                           "error": format!("No task \"{task_id}\" in run {}", run.run_id)});
        };
        let path = transcript_path(Path::new(&run.cwd), &run.run_id, &chosen.id);
        let tail = store::read_transcript_tail(&path);
        json!({
            "runId": run.run_id,
            "taskId": chosen.id,
            "role": chosen.role,
            "status": chosen.status,
            "tasks": tasks,
            "transcript": if tail.is_empty() { vec!["(no transcript yet)".to_string()] } else { tail },
        })
    }

    fn abort(&self) -> Value {
        // Only a run that is still going can be aborted; one that already
        // blocked or was cancelled is finished, not stoppable.
        let live = is_running(&self.cwd)
            .then(|| active_run(&self.cwd))
            .flatten();
        match live {
            Some(run) => {
                run.abort.store(true, Ordering::Relaxed);
                json!({"aborted": true,
                       "message": "Abort requested; workers are being terminated."})
            }
            None => json!({"aborted": false, "message": "No active graph run."}),
        }
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        match name {
            "graph_run" => {
                let goal = args.get("goal").and_then(Value::as_str).unwrap_or_default();
                let parsed = ParsedGraphArgs {
                    goal: goal.trim().to_string(),
                    forced: args
                        .get("mode")
                        .and_then(Value::as_str)
                        .and_then(types::Complexity::parse),
                    dry_run: args.get("dryRun").and_then(Value::as_bool).unwrap_or(false),
                };
                let run = self.run_to_completion(parsed).map_err(ToolError::Failed)?;
                Ok(ToolResult {
                    content: render_run_summary(&run),
                    is_error: run.phase != types::Phase::Done,
                    details: Some(json!({"graph": run})),
                })
            }
            "graph_status" => {
                let status = self.status(args.get("runId").and_then(Value::as_str));
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&status).unwrap_or_else(|_| "{}".into()),
                    is_error: false,
                    details: Some(status),
                })
            }
            GRAPH_SUBMIT_TOOL => {
                let context = GraphWorkerContext::from_env().ok_or_else(|| {
                    ToolError::Failed(
                        "graph_submit is only available inside a graph worker process".into(),
                    )
                })?;
                let message = context.submit(args).map_err(ToolError::Failed)?;
                Ok(ToolResult {
                    content: message,
                    is_error: false,
                    details: Some(json!({})),
                })
            }
            _ => Err(ToolError::Unknown(name.to_string())),
        }
    }

    pub fn save_command(&self, name: &str, overwrite: bool) -> Result<Value, String> {
        let run = active_run(&self.cwd)
            .and_then(|r| r.snapshot())
            .or_else(|| {
                list_runs(&self.cwd)
                    .first()
                    .and_then(|s| load_run(&self.cwd, &s.run_id))
            });
        let Some(run) = run else {
            return Err("No graph run found in this project to save.".into());
        };
        if run.phase != types::Phase::Done {
            return Err(format!(
                "Cannot save graph run '{}': only runs with phase 'done' may be saved (current phase: {})",
                run.run_id, run.phase
            ));
        }
        let saved_def = if let Some(ref def) = run.saved_definition {
            let mut def = def.clone();
            def.name = name.to_string();
            def.description = format!("Saved from run {}", run.run_id);
            def
        } else if let Some(ref graph_def) = run.definition {
            let topology = definitions::SavedGraphTopology {
                graph_id: format!("{name}-graph"),
                version: graph_def.version,
                mode: graph_def.mode,
                nodes: graph_def.nodes.clone(),
                edges: graph_def.edges.clone(),
            };
            let bindings = graph_def
                .nodes
                .iter()
                .map(|n| definitions::SavedStageBinding {
                    node_id: n.id.clone(),
                    stage: n.role.to_string(),
                    input_artifacts: vec![],
                })
                .collect();
            definitions::SavedGraphDefinitionV1 {
                schema_version: 1,
                name: name.to_string(),
                description: format!("Saved from run {}", run.run_id),
                graph: topology,
                bindings,
                budgets: Some(definitions::SavedBudgets {
                    max_duration_ms: None,
                    max_cost_usd: if run.budgets.max_cost_usd > 0.0 {
                        Some(run.budgets.max_cost_usd)
                    } else {
                        None
                    },
                    max_tokens: None,
                }),
                verification_policy: Some(definitions::SavedVerificationPolicy {
                    command_profile: "default".into(),
                    required: true,
                }),
                artifact_contract_versions: std::collections::BTreeMap::new(),
                parameters: vec![],
            }
        } else {
            return Err(format!(
                "Run '{}' does not contain a validated graph definition to save.",
                run.run_id
            ));
        };

        definitions::validate_saved_definition(&saved_def)?;
        let dir = definitions::project_graphs_dir(&self.cwd);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create graphs directory: {e}"))?;
        let saved_path = definitions::save_graph_definition_to_dir(
            &dir,
            name,
            &saved_def,
            overwrite,
            Some(&self.cwd),
        )?;
        Ok(json!({
            "saved": true,
            "name": name,
            "path": saved_path.display().to_string(),
            "runId": run.run_id,
        }))
    }

    pub fn run_saved_command(
        &self,
        name: &str,
        params: HashMap<String, String>,
        dry_run: bool,
    ) -> Result<Value, String> {
        let def = definitions::resolve_and_load_graph_definition(name, &self.cwd)
            .map_err(|e| format!("Could not load graph definition '{name}': {e}"))?;
        definitions::validate_saved_definition(&def)
            .map_err(|e| format!("Graph definition '{name}' failed validation: {e}"))?;
        let bound_params = definitions::bind_parameters(&def, &params)?;
        self.start_saved_background(def, bound_params, dry_run)
    }

    pub fn start_saved_background(
        &self,
        def: definitions::SavedGraphDefinitionV1,
        bound_params: HashMap<String, String>,
        dry_run: bool,
    ) -> Result<Value, String> {
        refuse_if_active(&self.cwd)?;
        let workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
        let active = Arc::new(ActiveRun::default());
        let (deps, config_errors) = self.deps(dry_run, &active);
        let options = RunOptions {
            goal: def.description.clone(),
            cwd: self.cwd.clone(),
            forced: None,
            dry_run,
            abort: Arc::clone(&active.abort),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };
        register_run(&self.cwd, Arc::clone(&active));
        let finished = Arc::clone(&active);
        let name = def.name.clone();
        let digest = definitions::compute_definition_digest(&def);
        let origin = ExecutionOrigin::SavedDefinition {
            name: name.clone(),
            digest: digest.clone(),
        };
        let _ = origin;
        let handle = thread::spawn(move || {
            let _guard = FinishedOnDrop(finished);
            let _ = controller::run_graph_owned(options, deps, Some(def), workspace_lease);
        });
        active
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(handle);

        let deadline = Instant::now() + START_REPORT_WAIT;
        let mut snapshot = active.snapshot();
        while snapshot.is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
            snapshot = active.snapshot();
        }
        Ok(json!({
            "started": true,
            "savedGraph": name,
            "digest": digest,
            "dryRun": dry_run,
            "params": bound_params,
            "runId": snapshot.as_ref().map(|run| run.run_id.clone()),
            "configErrors": config_errors,
            "status": snapshot.as_ref().map(render_now).unwrap_or_default(),
        }))
    }

    #[allow(dead_code)]
    pub fn run_saved_to_completion(
        &self,
        def: definitions::SavedGraphDefinitionV1,
        params: HashMap<String, String>,
        dry_run: bool,
    ) -> Result<GraphRun, String> {
        refuse_if_active(&self.cwd)?;
        definitions::validate_saved_definition(&def)?;
        let _bound = definitions::bind_parameters(&def, &params)?;
        let workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
        let active = Arc::new(ActiveRun::default());
        let (deps, _config_errors) = self.deps(dry_run, &active);
        let options = RunOptions {
            goal: def.description.clone(),
            cwd: self.cwd.clone(),
            forced: None,
            dry_run,
            abort: Arc::clone(&active.abort),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };
        register_run(&self.cwd, Arc::clone(&active));
        let _guard = FinishedOnDrop(Arc::clone(&active));
        Ok(controller::run_graph_owned(
            options,
            deps,
            Some(def),
            workspace_lease,
        ))
    }

    pub fn handle_advanced_command(
        &self,
        cmd: render::GraphAdvancedCommand,
    ) -> Result<Option<Value>, String> {
        match cmd {
            render::GraphAdvancedCommand::Diff { revision } => {
                let report = self.diff_command(revision)?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::Explain { node_id } => {
                let report = self.explain_command(node_id.as_deref())?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::DryRun { name } => {
                let report = self.dry_run_preflight_command(name.as_deref())?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::Fork {
                node_id,
                strategy,
                authorized,
            } => {
                let report = self.fork_command(&node_id, strategy.as_deref(), authorized)?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::Rewind {
                node_id,
                authorized,
            } => {
                let report = self.rewind_command(&node_id, authorized)?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::Verify => {
                let report = self.verify_command()?;
                Ok(Some(serde_json::to_value(report).unwrap_or_default()))
            }
            render::GraphAdvancedCommand::Budget {
                resource,
                value,
                expected_revision,
                authorized,
            } => self.budget_command(
                resource.as_deref(),
                value.as_deref(),
                expected_revision,
                authorized,
            ),
            render::GraphAdvancedCommand::Export { name, overwrite } => {
                self.export_command(name.as_deref(), overwrite)
            }
        }
    }

    pub fn diff_command(
        &self,
        revision: Option<u64>,
    ) -> Result<operations::GraphDiffReport, String> {
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to diff.".to_string())?;
        let current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        let prior = if let Some(rev) = revision {
            list_runs(&self.cwd)
                .iter()
                .filter_map(|s| load_run(&self.cwd, &s.run_id))
                .find(|r| r.revision == rev)
        } else if current.revision > 1 {
            list_runs(&self.cwd)
                .iter()
                .filter_map(|s| load_run(&self.cwd, &s.run_id))
                .find(|r| r.revision == current.revision - 1)
        } else {
            None
        };
        Ok(operations::generate_graph_diff(
            &current,
            prior.as_ref(),
            Some(&self.cwd),
            &[],
        ))
    }

    pub fn explain_command(
        &self,
        node_id: Option<&str>,
    ) -> Result<operations::GraphExplainReport, String> {
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to explain.".to_string())?;
        let current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        Ok(operations::generate_graph_explain(&current, node_id))
    }

    pub fn dry_run_preflight_command(
        &self,
        name: Option<&str>,
    ) -> Result<preflight::PreflightReport, String> {
        let current = list_runs(&self.cwd)
            .first()
            .and_then(|s| load_run(&self.cwd, &s.run_id));
        preflight::run_preflight(
            &self.cwd,
            name,
            current.as_ref(),
            self.project_trusted,
            "auto",
        )
    }

    pub fn fork_command(
        &self,
        node_id: &str,
        strategy_str: Option<&str>,
        authorized: bool,
    ) -> Result<operations::ForkPreview, String> {
        let _workspace_lease = authorized
            .then(|| lease::WorkspaceLease::acquire(&self.cwd))
            .transpose()?;
        if is_running(&self.cwd) {
            return Err(
                "fork requires a quiescent graph; pause or stop the active run first".into(),
            );
        }
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to fork.".to_string())?;
        let current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        let strategy = match strategy_str {
            Some(raw) => operations::ForkStrategy::parse(raw)
                .ok_or_else(|| format!("unknown fork strategy '{raw}'"))?,
            None => operations::ForkStrategy::RepairMinimal,
        };
        let preview =
            operations::generate_fork_preview(&current, node_id, strategy, Some(&self.cwd))?;
        if !authorized {
            return Ok(preview);
        }
        let _new_run = operations::execute_fork(&current, &preview, &self.cwd)?;
        Ok(operations::ForkPreview {
            applied: true,
            ..preview
        })
    }

    pub fn rewind_command(
        &self,
        node_id: &str,
        authorized: bool,
    ) -> Result<operations::GraphRewindPreview, String> {
        let _workspace_lease = authorized
            .then(|| lease::WorkspaceLease::acquire(&self.cwd))
            .transpose()?;
        if is_running(&self.cwd) {
            return Err(
                "rewind requires a quiescent graph; pause or stop the active run first".into(),
            );
        }
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to rewind.".to_string())?;
        let mut current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        let preview = operations::generate_rewind_preview(&current, node_id, &self.cwd, &[])?;
        if !authorized {
            return Ok(preview);
        }
        let _result =
            operations::execute_rewind_with_runtime(&mut current, &preview, &self.cwd, true, true)?;
        Ok(operations::GraphRewindPreview {
            applied: true,
            ..preview
        })
    }

    pub fn verify_command(&self) -> Result<operations::VerifyOnlyReport, String> {
        let _workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to verify.".to_string())?;
        let mut current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;

        let commands = config::detect_verify_commands(&self.cwd);
        let abort = Arc::new(AtomicBool::new(false));
        let exec = verify::default_verify_exec;
        let source_before = operations::source_manifest_for_run(&current, &self.cwd)?;
        let source_current = current
            .verification_bundle
            .as_ref()
            .map(|bundle| operations::verification_source_current(bundle, source_before.as_ref()))
            .unwrap_or(source_before.is_none());

        let policy_mode = load_config(&self.cwd).config.security_verification;
        let security_available = match policy_mode {
            SecurityPolicyMode::Off => true,
            SecurityPolicyMode::Risk | SecurityPolicyMode::Always => {
                source_current
                    && current.verification_bundle.as_ref().is_some_and(|bundle| {
                        matches!(
                            bundle.security,
                            SecurityVerification::Passed { .. } | SecurityVerification::NotRequired
                        )
                    })
            }
        };
        let complexity = current
            .forced
            .or_else(|| current.classification.as_ref().map(|item| item.complexity));
        let review_complete = match complexity {
            Some(types::Complexity::Trivial) => source_current,
            _ => {
                current
                    .review_coverage
                    .as_ref()
                    .map(review_coverage::coverage_complete)
                    .unwrap_or(false)
                    && source_current
            }
        };

        let mut report = operations::execute_verify_only(
            &mut current,
            &self.cwd,
            &commands,
            &exec,
            &abort,
            None,
            security_available,
            review_complete,
        );
        let source_after = operations::source_manifest_for_run(&current, &self.cwd)?;
        let source_changed = match (source_before.as_ref(), source_after.as_ref()) {
            (Some(before), Some(after)) => !before.is_current(after),
            (None, None) => false,
            _ => true,
        };
        if source_changed {
            report.passed = false;
            report.security_passed = false;
            report.review_passed = false;
            report.message =
                "Verification rejected: source changed during verification".to_string();
        }
        store::save_run(&mut current)
            .map_err(|error| format!("failed to persist verification result: {error}"))?;
        Ok(report)
    }

    pub fn budget_command(
        &self,
        resource: Option<&str>,
        value: Option<&str>,
        expected_revision: Option<u64>,
        authorized: bool,
    ) -> Result<Option<Value>, String> {
        let _workspace_lease = value
            .map(|_| lease::WorkspaceLease::acquire(&self.cwd))
            .transpose()?;
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to inspect.".to_string())?;
        if is_running(&self.cwd) && value.is_some() {
            return Err("budget updates are serialized at run boundaries; pause the active graph before updating".into());
        }
        let mut current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        if resource.is_none() && value.is_none() {
            let report = operations::graph_budget_report(&current);
            return Ok(Some(
                serde_json::to_value(report).map_err(|error| error.to_string())?,
            ));
        }
        let (Some(resource), Some(value)) = (resource, value) else {
            return Err(
                "Usage: /graph budget [set] <resource> <value> [--revision N] [--authorize]".into(),
            );
        };
        if !self.project_trusted {
            return Err("budget adjustments require explicit project trust".into());
        }
        if expected_revision.is_none() {
            return Err("budget adjustments require an expected graph revision".into());
        }
        let report = operations::apply_budget_update(
            &mut current,
            resource,
            value,
            expected_revision,
            authorized,
        )?;
        store::save_run(&mut current)
            .map_err(|error| format!("failed to save budget update: {error}"))?;
        Ok(Some(
            serde_json::to_value(report).map_err(|error| error.to_string())?,
        ))
    }

    pub fn export_command(
        &self,
        name: Option<&str>,
        overwrite: bool,
    ) -> Result<Option<Value>, String> {
        let latest = list_runs(&self.cwd)
            .first()
            .cloned()
            .ok_or_else(|| "No graph run found in this project to export.".to_string())?;
        let current = load_run(&self.cwd, &latest.run_id)
            .ok_or_else(|| format!("Could not load run {}", latest.run_id))?;
        let export_name = name.unwrap_or("graph-export");
        let definition = saved_definition_from_run(&current, export_name)?;
        let document = export::export_definition(&definition)?;
        let rendered = export::render_export_document(&document)?;
        let checksum = export::export_checksum(&rendered);
        if let Some(name) = name {
            let path = export::write_project_export(&self.cwd, name, &document, overwrite)?;
            return Ok(Some(json!({
                "exported": true,
                "name": name,
                "path": path.display().to_string(),
                "runId": current.run_id,
                "checksum": checksum,
            })));
        }
        Ok(Some(json!({
            "exported": true,
            "runId": current.run_id,
            "checksum": checksum,
            "document": serde_json::from_str::<Value>(&rendered).map_err(|error| error.to_string())?,
        })))
    }

    pub fn command(&self, name: &str, args: &str) -> Result<Option<Value>, String> {
        let value = match name {
            "graph" => {
                match render::parse_advanced_graph_command(args) {
                    Ok(Some(adv)) => return self.handle_advanced_command(adv),
                    Ok(None) => {}
                    Err(error) => return Err(error),
                }
                let cmd = parse_graph_command(args)?;
                match cmd {
                    GraphCommand::Current => {
                        if is_running(&self.cwd) {
                            self.status(None)
                        } else {
                            let Some(latest) = list_runs(&self.cwd).first().cloned() else {
                                return Err(
                                    "Usage: /graph <goal> [--simple|--complex] [--dry-run]".into(),
                                );
                            };
                            let Some(run) = load_run(&self.cwd, &latest.run_id) else {
                                return Err(format!(
                                    "Could not load state for run {}.",
                                    latest.run_id
                                ));
                            };
                            if run.phase == types::Phase::Done
                                || run.current_lifecycle() == types::GraphLifecycle::Paused
                            {
                                self.status(Some(&run.run_id))
                            } else {
                                self.resume(&run.run_id)?
                            }
                        }
                    }
                    GraphCommand::Save { name, overwrite } => {
                        self.save_command(&name, overwrite)?
                    }
                    GraphCommand::RunSaved {
                        name,
                        params,
                        dry_run,
                    } => self.run_saved_command(&name, params, dry_run)?,
                    GraphCommand::Goal(parsed) => {
                        if parsed.goal.trim().is_empty() {
                            return Err(
                                "Usage: /graph <goal> [--simple|--complex] [--dry-run]".into()
                            );
                        }
                        self.start_background(parsed, HashMap::new(), None, None)?
                    }
                }
            }
            "graph-control" => {
                let control: control::GraphControl = serde_json::from_str(args.trim())
                    .map_err(|e| format!("Invalid graph control JSON: {e}"))?;
                if let Some(active) = active_run(&self.cwd).filter(|active| !active.is_finished()) {
                    let mut snap = active.snapshot.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(ref mut run) = *snap {
                        if run.run_id == control.run_id {
                            let mut tracker = active
                                .control_tracker
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            let receipt =
                                control::reduce_control(run, &control, &mut tracker, 0, true);
                            if receipt.state == control::ControlReceiptState::Applied {
                                store::save_run(run).map_err(|e| {
                                    format!("Failed to persist graph run checkpoint: {e}")
                                })?;
                            }
                            return Ok(Some(serde_json::to_value(&receipt).unwrap_or_default()));
                        }
                    }
                }
                let _workspace_lease = lease::WorkspaceLease::acquire(&self.cwd)?;
                let Some(mut run) = load_run(&self.cwd, &control.run_id) else {
                    return Err(format!("Run '{}' not found", control.run_id));
                };
                let mut tracker = control::ControlTracker::new();
                let receipt = control::reduce_control(&mut run, &control, &mut tracker, 0, true);
                if receipt.state == control::ControlReceiptState::Applied {
                    store::save_run(&mut run)
                        .map_err(|e| format!("Failed to persist graph run checkpoint: {e}"))?;
                }
                serde_json::to_value(&receipt).map_err(|e| e.to_string())?
            }
            "graph-resume" => self.resume(args.trim())?,
            "graph-status" => self.status(Some(args.trim())),
            "graph-view" => self.view(args.trim()),
            "graph-abort" => self.abort(),
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use types::{ArtifactKind, Phase, TaskStatus};

    /// The active-run registry and `abort_all_runs` are process-wide by
    /// design (one process is one session), so tests that touch them run one
    /// at a time rather than racing each other's runs.
    static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn registry_guard() -> std::sync::MutexGuard<'static, ()> {
        REGISTRY_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn controller(cwd: &Path) -> GraphController {
        let mut controller = GraphController::new(cwd.to_path_buf());
        controller.set_session_context(None, None, false);
        controller
    }

    #[test]
    fn workspace_lease_blocks_dispatch_resume_and_idle_mutations() {
        let _registry = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let owner = lease::WorkspaceLease::acquire(dir.path()).unwrap();
        let active = Arc::new(ActiveRun::default());
        let (mut deps, _) = controller.deps(true, &active);
        deps.runner = Arc::new(|_, _, _| panic!("another owner must prevent dispatch"));
        let parsed = ParsedGraphArgs {
            goal: "lease fixture".into(),
            forced: Some(Complexity::Trivial),
            dry_run: true,
        };
        let options = controller.options(&parsed, active.abort.clone(), HashMap::new(), None);
        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Blocked);
        assert!(run
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("ownership unavailable"));
        assert!(list_runs(dir.path()).is_empty());
        assert!(controller
            .start_background(parsed.clone(), HashMap::new(), None, None)
            .unwrap_err()
            .contains("ownership unavailable"));
        assert!(controller
            .resume("")
            .unwrap_err()
            .contains("ownership unavailable"));
        assert!(controller
            .verify_command()
            .unwrap_err()
            .contains("ownership unavailable"));
        assert!(controller
            .rewind_command("writer", true)
            .unwrap_err()
            .contains("ownership unavailable"));
        assert!(controller
            .fork_command("writer", None, true)
            .unwrap_err()
            .contains("ownership unavailable"));
        assert!(controller
            .budget_command(Some("workers"), Some("2"), Some(0), true)
            .unwrap_err()
            .contains("ownership unavailable"));
        let control = serde_json::json!({"operationId":"stop", "runId":"owned", "expectedRunRevision":0, "action":"stop_graph"});
        assert!(controller
            .command("graph-control", &control.to_string())
            .unwrap_err()
            .contains("ownership unavailable"));
        drop(owner);
        let (deps, _) = controller.deps(true, &active);
        let options = controller.options(&parsed, active.abort.clone(), HashMap::new(), None);
        assert_eq!(run_graph(options, deps).phase, Phase::Done);
    }

    #[test]
    fn interactive_role_models_reach_worker_dependencies_without_project_trust() {
        let dir = tempdir().unwrap();
        let mut controller = controller(dir.path());
        controller.set_session_context(Some("provider/chat".into()), Some("high".into()), false);
        let models = std::collections::BTreeMap::from([
            (Role::Researcher, "provider/luna".into()),
            (Role::Planner, "provider/sol".into()),
        ]);
        controller.set_role_models(models.clone());
        let active = Arc::new(ActiveRun::default());
        let (deps, errors) = controller.deps(true, &active);
        assert!(errors.is_empty());
        assert_eq!(deps.config.models, models);
        assert_eq!(deps.session_model.as_deref(), Some("provider/chat"));
        assert!(!deps.project_trusted);
        assert!(deps.config.worker_extensions.is_empty());
        controller.set_role_models(Default::default());
        assert!(controller.deps(true, &active).0.config.models.is_empty());
    }

    fn drain_active(cwd: &Path) {
        // A dry run finishes in milliseconds; wait for it so the next test in
        // the same directory is not refused as "already active".
        for _ in 0..200 {
            if !is_running(cwd) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(cwd);
    }

    #[test]
    fn security_untrusted_project_config_cannot_authorize_execution() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".davinci")).unwrap();
        std::fs::write(dir.path().join(".davinci/graph.json"),
            r#"{"workerExtensions":["./fixture.mjs"],"workerExtraTools":["custom_mutator"],"verifyCommands":[{"name":"fixture","command":"echo fixture"}],"securityVerification":"off"}"#).unwrap();
        let mut controller = controller(dir.path());
        let active = Arc::new(ActiveRun::default());
        let (deps, _) = controller.deps(true, &active);
        assert!(deps.config.worker_extensions.is_empty());
        assert!(deps.config.worker_extra_tools.is_empty());
        assert!(deps.config.verify_commands.is_empty());
        assert_eq!(
            deps.config.security_verification,
            config::GraphConfig::default().security_verification
        );
        controller.set_session_context(None, None, true);
        let (trusted, _) = controller.deps(true, &active);
        assert_eq!(trusted.config.worker_extensions, vec!["./fixture.mjs"]);
        assert_eq!(trusted.config.verify_commands.len(), 1);
    }

    #[test]
    fn a_dry_run_walks_the_whole_pipeline_without_a_model() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run improve the parser"))
            .expect("runs");
        assert_eq!(run.phase, Phase::Done, "blocked: {:?}", run.blocked_reason);
        assert_eq!(run.counters.cost_usd, 0.0);
        let ids: Vec<&str> = run.tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "classify",
                "research-1",
                "plan-1",
                "implement-1",
                "review-1"
            ]
        );
        assert!(run
            .tasks
            .iter()
            .all(|task| task.status == TaskStatus::Succeeded));
        drain_active(dir.path());
    }

    #[test]
    fn test_graph_dry_run_registers_runtime_workers_with_zero_model_calls() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();

        let run_id = davinci_agent::RunId::new();
        let root_agent_id = davinci_agent::AgentId::new();
        let bus = davinci_agent::RuntimeBus::default();
        let runtime = davinci_agent::RuntimeHandle::new(run_id, root_agent_id, bus);

        let root_record = davinci_agent::AgentRecord {
            id: root_agent_id,
            run_id,
            parent: None,
            kind: davinci_agent::AgentKind::Main,
            name: "root-agent".into(),
            provider: "mock".into(),
            model_id: "mock-model".into(),
            cwd: dir.path().to_path_buf(),
            state: davinci_agent::AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: 1000,
            updated_ms: 1000,
            failure_reason: None,
        };
        runtime.registry.register_agent(root_record).unwrap();

        let controller = controller(dir.path()).with_runtime(runtime.clone());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run test runtime registration"))
            .expect("runs");

        assert_eq!(run.phase, Phase::Done);
        assert_eq!(run.counters.cost_usd, 0.0);

        let workers: Vec<_> = runtime
            .registry
            .snapshot()
            .into_iter()
            .filter(|r| r.kind == davinci_agent::AgentKind::GraphWorker)
            .collect();

        assert!(
            !workers.is_empty(),
            "expected GraphWorker records in runtime registry"
        );
        for worker in &workers {
            assert_eq!(worker.parent, Some(root_agent_id));
            assert_eq!(worker.run_id, run_id);
            assert_eq!(worker.state, davinci_agent::AgentState::Completed);
            assert!(worker.name.starts_with("graph-worker-"));
        }

        drain_active(dir.path());
    }

    #[test]
    fn a_dry_run_persists_state_and_artifacts_for_later_inspection() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run persist me"))
            .expect("runs");
        let reloaded = load_run(dir.path(), &run.run_id).expect("state.json written");
        assert_eq!(reloaded.goal, "persist me");
        assert_eq!(reloaded.phase, Phase::Done);
        assert!(
            reloaded.definition.is_some(),
            "graph definition must be persisted"
        );
        let task = reloaded.task("classify").expect("classify task exists");
        assert!(
            task.fingerprint.is_some(),
            "replay fingerprint must be persisted"
        );
        assert_eq!(task.error, None, "no stale errors on success");
        let artifact =
            store::read_artifact(dir.path(), &run.run_id, "review-1", ArtifactKind::Review)
                .expect("review artifact validates");
        assert!(artifact.as_review().is_some());
        drain_active(dir.path());
    }

    #[test]
    fn forcing_simple_skips_planning_and_review() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run --simple tiny tweak"))
            .expect("runs");
        assert_eq!(run.phase, Phase::Done);
        let ids: Vec<&str> = run.tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(ids, vec!["classify", "implement-1"]);
        drain_active(dir.path());
    }

    #[test]
    fn an_empty_goal_is_refused_with_usage() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let error = controller.command("graph", "  --dry-run  ").unwrap_err();
        assert!(error.contains("Usage: /graph"));
        drain_active(dir.path());
    }

    #[test]
    fn bare_graph_continues_a_stopped_run_with_the_same_identity_and_counters() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let mut stopped = controller
            .run_to_completion(parse_graph_args("--dry-run continue me"))
            .expect("initial run completes");
        drain_active(dir.path());
        stopped.phase = Phase::Cancelled;
        stopped.counters.workers_spawned = 41;
        store::save_run(&mut stopped).expect("stopped state persists");

        let started = controller.command("graph", "").unwrap().unwrap();
        assert_eq!(started["started"], true);
        assert_eq!(started["runId"], stopped.run_id);
        drain_active(dir.path());

        let continued = load_run(dir.path(), &stopped.run_id).expect("continued state persists");
        assert_eq!(continued.run_id, stopped.run_id);
        assert!(continued.counters.workers_spawned >= 41);
        assert_eq!(list_runs(dir.path()).len(), 1, "continuation is one run");
    }

    #[test]
    fn status_and_view_report_a_finished_run_from_disk() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        controller
            .run_to_completion(parse_graph_args("--dry-run inspect me"))
            .expect("runs");
        drain_active(dir.path());

        let status = controller.command("graph-status", "").unwrap().unwrap();
        assert_eq!(status["active"], false);
        assert_eq!(status["recent"][0]["phase"], "done");
        assert!(status["summary"].as_str().unwrap().contains("inspect me"));

        let view = controller
            .command("graph-view", "review-1")
            .unwrap()
            .unwrap();
        assert_eq!(view["taskId"], "review-1");
        assert!(view["tasks"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn viewing_an_unknown_task_lists_the_real_ones() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        controller
            .run_to_completion(parse_graph_args("--dry-run list tasks"))
            .expect("runs");
        drain_active(dir.path());
        let view = controller.command("graph-view", "nope").unwrap().unwrap();
        assert!(view["error"].as_str().unwrap().contains("No task \"nope\""));
        assert!(!view["tasks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn aborting_without_a_run_is_reported_not_an_error() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let value = controller.command("graph-abort", "").unwrap().unwrap();
        assert_eq!(value["aborted"], false);
    }

    #[test]
    fn aborting_and_session_shutdown_both_reach_the_run_that_is_live() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let active = Arc::new(ActiveRun::default());
        active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(dir.path().to_path_buf(), Arc::clone(&active));

        assert!(!active.abort.load(Ordering::Relaxed));
        let value = controller.command("graph-abort", "").unwrap().unwrap();
        assert_eq!(value["aborted"], true);
        assert!(active.abort.load(Ordering::Relaxed));

        active.abort.store(false, Ordering::Relaxed);
        abort_all_runs();
        assert!(
            active.abort.load(Ordering::Relaxed),
            "session shutdown must stop a background run"
        );
        active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(dir.path());
    }

    #[test]
    fn resuming_a_finished_run_is_refused() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        controller
            .run_to_completion(parse_graph_args("--dry-run finished already"))
            .expect("runs");
        drain_active(dir.path());
        let error = controller.command("graph-resume", "").unwrap_err();
        assert!(error.contains("already finished"));
    }

    #[test]
    fn resuming_without_any_runs_says_so() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let error = controller.command("graph-resume", "").unwrap_err();
        assert_eq!(error, "No graph runs in this project.");
    }

    #[test]
    fn an_unknown_command_is_not_claimed() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        assert!(controller.command("memory-status", "").unwrap().is_none());
    }

    #[test]
    fn the_graph_run_tool_returns_a_readable_summary() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let result = controller
            .execute_tool("graph_run", &json!({"goal": "tool path", "dryRun": true}))
            .expect("tool runs");
        assert!(!result.is_error);
        assert!(result.content.contains("## Graph run"));
        assert!(result.content.contains("- outcome: done"));
        drain_active(dir.path());
    }

    #[test]
    fn graph_submit_is_refused_outside_a_worker_process() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let error = controller
            .execute_tool(GRAPH_SUBMIT_TOOL, &json!({"artifact": {}}))
            .unwrap_err();
        assert!(matches!(error, ToolError::Failed(_)));
    }

    #[test]
    fn a_project_budget_override_reaches_the_run() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".pi")).unwrap();
        std::fs::write(
            dir.path().join(".pi/graph.json"),
            r#"{"budgets":{"maxCostUsd":5,"runDeadlineMs":600000}}"#,
        )
        .unwrap();
        let mut controller = controller(dir.path());
        // Project overrides are applied only after explicit project trust.
        controller.set_session_context(None, None, true);
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run budgeted"))
            .expect("runs");
        assert_eq!(run.budgets.max_cost_usd, 5.0);
        assert_eq!(run.budgets.run_deadline_ms, 600_000);
        drain_active(dir.path());
    }

    #[test]
    fn an_unlimited_default_run_records_no_caps() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run unbounded"))
            .expect("runs");
        assert_eq!(run.budgets.max_cost_usd, 0.0);
        assert_eq!(run.budgets.run_deadline_ms, 0);
        assert_eq!(run.budgets.max_workers, 0);
        assert_eq!(run.budgets.verify_command_timeout_ms, 0);
        drain_active(dir.path());
    }

    #[test]
    fn test_command_save_and_run_saved_roundtrip() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());

        // 1. Initial dry-run to completion
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run audit code"))
            .expect("runs");
        assert_eq!(run.phase, types::Phase::Done);

        // 2. Save the successful run
        let save_res = controller
            .command("graph", "save security-audit")
            .unwrap()
            .expect("save result");
        assert_eq!(save_res["saved"], true);
        assert_eq!(save_res["name"], "security-audit");

        let graph_file = dir
            .path()
            .join(".davinci")
            .join("graphs")
            .join("security-audit.yaml");
        assert!(graph_file.exists());

        // 3. Second save without overwrite fails
        let err = controller
            .command("graph", "save security-audit")
            .unwrap_err();
        assert!(err.contains("already exists"));

        // 4. Second save with overwrite succeeds
        let save_res2 = controller
            .command("graph", "save security-audit --overwrite")
            .unwrap()
            .expect("overwrite save result");
        assert_eq!(save_res2["saved"], true);

        // 5. Run saved graph
        let run_res = controller
            .command("graph", "run security-audit --dry-run")
            .unwrap()
            .expect("run saved result");
        assert_eq!(run_res["started"], true);
        assert_eq!(run_res["savedGraph"], "security-audit");

        drain_active(dir.path());
    }

    #[test]
    fn test_command_save_without_completed_run_fails() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());

        let err = controller.command("graph", "save no-run").unwrap_err();
        assert!(err.contains("No graph run found"));
    }

    #[test]
    fn blocked_graph_can_resume_with_same_identity_and_preserved_spend() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let controller = controller(dir.path());
        let mut run = controller
            .run_to_completion(parse_graph_args("--dry-run --simple resume regression"))
            .unwrap();
        assert_eq!(run.lifecycle, Some(types::GraphLifecycle::Stopped));
        run.phase = Phase::Blocked;
        run.lifecycle = Some(types::GraphLifecycle::Running); // Older saved runs.
        run.blocked_reason = Some("verification still failing after 3 revision cycles".into());
        run.counters.revision_cycles = 3;
        run.counters.cost_usd = 6.83;
        assert_eq!(run.current_lifecycle(), types::GraphLifecycle::Stopped);
        store::save_run(&mut run).unwrap();
        let response = controller
            .command("graph-resume", &run.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(response["started"], true);
        drain_active(dir.path());
        let resumed = load_run(dir.path(), &run.run_id).unwrap();
        assert_eq!(resumed.run_id, run.run_id);
        assert_eq!(resumed.goal, run.goal);
        assert_eq!(resumed.phase, Phase::Done);
        assert_eq!(resumed.lifecycle, Some(types::GraphLifecycle::Stopped));
        assert!(resumed.blocked_reason.is_none());
        assert!(resumed.counters.cost_usd >= 6.83);
        assert!(resumed.counters.revision_cycles >= 3);
    }

    #[test]
    fn test_resume_bare_paused_does_not_unpause() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let run_id = store::new_run_id();
        store::create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = types::GraphRun {
            version: 1,
            run_id: run_id.clone(),
            goal: "paused run".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            phase: types::Phase::Implement,
            forced: None,
            dry_run: true,
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
            budgets: types::GraphBudgets::default(),
            counters: types::GraphCounters {
                workers_spawned: 1,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: store::now_ms(),
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: Some(types::GraphLifecycle::Paused),
            revision: 1,
        };
        store::save_run(&mut run).unwrap();

        let controller = controller(dir.path());
        let res = controller.command("graph-resume", "").unwrap().unwrap();
        assert_eq!(res["resumed"], false);
        assert_eq!(res["paused"], true);
        assert_eq!(res["runId"], run_id);
    }

    #[test]
    fn test_resume_reconciles_crashed_running_workers() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let run_id = store::new_run_id();
        store::create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = types::GraphRun {
            version: 1,
            run_id: run_id.clone(),
            goal: "crashed run".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            phase: types::Phase::Implement,
            forced: None,
            dry_run: true,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![types::GraphTaskState {
                id: "implement-1".into(),
                role: types::Role::Writer,
                expect: types::ArtifactKind::PatchReport,
                depends_on: vec![],
                focus: None,
                status: types::TaskStatus::Running,
                attempts: 1,
                artifact_file: None,
                error: None,
                usage: types::WorkerUsage::default(),
                started_at: Some(10),
                ended_at: None,
                last_activity: None,
                fingerprint: None,
                mutation: None,
                context_fingerprint: None,
                context_tokens: 0,
                memory_refs: vec![],
                skill_refs: vec![],
            }],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: types::GraphBudgets::default(),
            counters: types::GraphCounters {
                workers_spawned: 1,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: store::now_ms(),
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: Some(types::GraphLifecycle::Running),
            revision: 1,
        };
        store::save_run(&mut run).unwrap();

        let controller = controller(dir.path());
        let _ = controller.command("graph-resume", &run_id);
        drain_active(dir.path());

        // Re-read run from disk: task should have been marked Failed with reconciliation required
        let reloaded = store::load_run(dir.path(), &run_id).unwrap();
        assert_eq!(reloaded.tasks[0].status, types::TaskStatus::Failed);
        assert!(reloaded.tasks[0]
            .error
            .as_ref()
            .unwrap()
            .contains("reconciliation required"));
        assert_eq!(
            reloaded.current_lifecycle(),
            types::GraphLifecycle::RecoveryRequired
        );
    }

    #[test]
    fn f14_budget_inspection_update_and_declarative_export() {
        let _guard = registry_guard();
        let dir = tempdir().unwrap();
        let mut controller = controller(dir.path());
        let run = controller
            .run_to_completion(parse_graph_args("--dry-run budget and export"))
            .expect("runs");
        drain_active(dir.path());

        let inspected = controller.command("graph", "budget").unwrap().unwrap();
        assert_eq!(inspected["runId"], run.run_id);
        assert_eq!(inspected["ceilings"]["maxCostUsd"], 0.0);

        let unauthorized = controller
            .command(
                "graph",
                &format!(
                    "budget set max-workers 20 --revision {} --authorize",
                    run.revision
                ),
            )
            .unwrap_err();
        assert!(unauthorized.contains("project trust"));

        controller.set_session_context(None, None, true);
        let updated = controller
            .command(
                "graph",
                &format!(
                    "budget set max-workers 20 --revision {} --authorize",
                    run.revision
                ),
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated["ceilings"]["maxWorkers"], 20);

        let review = controller.command("graph", "export").unwrap().unwrap();
        assert_eq!(review["exported"], true);
        assert!(review["document"].get("runId").is_none());
        assert!(review["document"].get("resourceSnapshot").is_none());

        let written = controller
            .command("graph", "export review --overwrite")
            .unwrap()
            .unwrap();
        let path = written["path"].as_str().unwrap();
        assert!(std::path::Path::new(path).exists());
    }
}
