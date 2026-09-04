//! Typed graph-engineer runtime: run a coding task as an explicit execution
//! graph of isolated, least-privileged worker processes.
//!
//! ```text
//! /graph <goal> [--simple|--complex] [--dry-run]   start a run in the background
//! /graph-status                                    active + recent runs
//! /graph-view [taskId]                             tail a worker's live transcript
//! /graph-resume [runId]                            re-run, reusing finished workers
//! /graph-abort                                     stop the active run
//! ```
//!
//! The controller is deterministic code: model calls happen only inside worker
//! child processes, one per node. Artifacts are the only data that crosses a
//! node boundary, and every artifact is schema-validated before it is accepted.
//!
//! Budgets that bound time or money are OFF by default — a run continues until
//! it finishes, blocks, or the operator aborts it. See [`types::GraphBudgets`].

pub(crate) mod briefings;
pub(crate) mod config;
pub(crate) mod controller;
pub(crate) mod mutation;
pub(crate) mod process;
pub(crate) mod render;
pub(crate) mod roles;
pub(crate) mod replay;
pub(crate) mod store;
pub(crate) mod topology;
pub(crate) mod types;
pub(crate) mod validate;
pub(crate) mod verify;
pub(crate) mod worker;
pub(crate) mod worker_hooks;

#[allow(unused_imports)]
pub use mutation::{
    capture_baseline, capture_graph_delta, ChangedFile, FileFingerprint, GraphMutation,
    MutationBaseline, PatchChunk,
};
#[allow(unused_imports)]
pub use replay::{incompatibility_reason, replay_compatible, ReplayFingerprint};
#[allow(unused_imports)]
pub use topology::{
    build_definition, ready_nodes, validate_definition, EdgeCondition, EdgeDefinition,
    GraphDefinition, GraphMode, GraphTopologyError, NodeDefinition,
};

use config::load_config;
use controller::{run_graph, ControllerDeps, RunOptions};
use davinci_agent::{ToolError, ToolResult};
use render::{parse_graph_args, render_now, render_run_summary, ParsedGraphArgs};
use serde_json::{json, Value};
use store::{list_runs, load_run, read_transcript, transcript_path};
use types::{Artifact, GraphRun, WorkerUsage};
use verify::{default_verify_exec, dry_run_verify_exec};
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
struct ActiveRun {
    abort: Arc<AtomicBool>,
    snapshot: Mutex<Option<GraphRun>>,
    /// Set once the run thread has left, whatever its outcome. A run whose
    /// thread died without reaching a terminal phase is finished all the same.
    finished: AtomicBool,
    /// The background run thread, joined on session shutdown.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ActiveRun {
    fn snapshot(&self) -> Option<GraphRun> {
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
/// extension host for every slash command, so `/graph-status` and
/// `/graph-abort` must find the run that `/graph` started.
fn active_runs() -> &'static Mutex<HashMap<PathBuf, Arc<ActiveRun>>> {
    static ACTIVE: OnceLock<Mutex<HashMap<PathBuf, Arc<ActiveRun>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_run(cwd: &Path) -> Option<Arc<ActiveRun>> {
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

/// `/graph`, `/graph-resume` and `graph_run` refuse while a run is live. One
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
        "A graph run is already active. Use /graph-abort or /graph-status first.".into()
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

#[derive(Debug, Clone)]
pub struct GraphController {
    cwd: PathBuf,
    session_model: Option<String>,
    session_thinking: Option<String>,
    project_trusted: bool,
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
            project_trusted: false,
        }
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

    /// Returns the dependencies plus any `graph.json` complaints worth showing.
    fn deps(&self, dry_run: bool, active: &Arc<ActiveRun>) -> (ControllerDeps, Vec<String>) {
        // A malformed graph.json is reported, then ignored: the run proceeds
        // on defaults rather than refusing to start.
        let loaded = load_config(&self.cwd);
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
                Arc::new(default_verify_exec)
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
        };
        (deps, loaded.errors)
    }

    fn options(
        &self,
        parsed: &ParsedGraphArgs,
        abort: Arc<AtomicBool>,
        resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
    ) -> RunOptions {
        RunOptions {
            goal: parsed.goal.clone(),
            cwd: self.cwd.clone(),
            forced: parsed.forced,
            dry_run: parsed.dry_run,
            abort,
            resume_artifacts,
        }
    }

    /// Start a run on a background thread and report what we know so far.
    fn start_background(
        &self,
        parsed: ParsedGraphArgs,
        resume_artifacts: HashMap<String, (Artifact, WorkerUsage, Option<ReplayFingerprint>)>,
    ) -> Result<Value, String> {
        if parsed.goal.trim().is_empty() {
            return Err("Usage: /graph <goal> [--simple|--complex] [--dry-run]".into());
        }
        refuse_if_active(&self.cwd)?;
        let active = Arc::new(ActiveRun::default());
        let (deps, config_errors) = self.deps(parsed.dry_run, &active);
        let options = self.options(&parsed, Arc::clone(&active.abort), resume_artifacts);
        let resumed = options.resume_artifacts.len();
        register_run(&self.cwd, Arc::clone(&active));

        let finished = Arc::clone(&active);
        let handle = thread::spawn(move || {
            // Marks the run finished however this thread leaves — normal exit
            // or a panic — so a crashed run cannot wedge `/graph` behind a
            // permanent "already active" for this project.
            let _guard = FinishedOnDrop(finished);
            let _ = run_graph(options, deps);
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
        let active = Arc::new(ActiveRun::default());
        let (deps, _config_errors) = self.deps(parsed.dry_run, &active);
        let options = self.options(&parsed, Arc::clone(&active.abort), HashMap::new());
        register_run(&self.cwd, Arc::clone(&active));
        let _guard = FinishedOnDrop(Arc::clone(&active));
        Ok(run_graph(options, deps))
    }

    fn resume(&self, wanted: &str) -> Result<Value, String> {
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
        let Some(old_run) = load_run(&self.cwd, &summary.run_id) else {
            return Err(format!("Could not load state for run {}.", summary.run_id));
        };
        if old_run.phase == types::Phase::Done {
            return Err(format!(
                "Run {} already finished (done). Start a new /graph instead.",
                old_run.run_id
            ));
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
        let transcript = read_transcript(&path);
        let tail: Vec<String> = transcript.iter().rev().take(200).rev().cloned().collect();
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

    pub fn command(&self, name: &str, args: &str) -> Result<Option<Value>, String> {
        let value = match name {
            "graph" => self.start_background(parse_graph_args(args), HashMap::new())?,
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
        let controller = controller(dir.path());
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
}
