//! One node execution = one isolated `pi` child process.
//!
//! ```text
//! pi --mode json -p --no-session --no-extensions --no-skills
//!    --no-prompt-templates --tools <role allowlist>
//!    [--model provider/id] [--thinking level] [-a]
//!    --append-system-prompt <role prompt file> @<briefing file>
//! ```
//!
//! The child's `graph_submit` tool (see `worker_hooks.rs`) writes the artifact
//! to `spec.artifact_path`; "exit 0 AND a valid artifact file" is the only
//! definition of success. stdout JSON lines are folded for usage accounting,
//! the live transcript, and diagnostics.

use super::process::run_child_with_deadline;
pub use super::process::WorkerDeadline;
use super::store::{iso8601_utc, now_ms, write_artifact};
pub use super::types::WorkerError;
use super::types::{
    Artifact, ArtifactKind, Classification, Complexity, EvidenceArtifact, EvidenceFinding,
    ImplementationPlan, PatchReport, PlanStep, ResearchKind, ResearchRequest, ReviewDecision,
    TaskClass, Verdict, WorkerResult, WorkerSpec, WorkerUsage,
};
use super::validate::validate_artifact;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const STDERR_TAIL_CHARS: usize = 8000;
const FINAL_TEXT_TRANSCRIPT_CHARS: usize = 2000;

#[derive(Debug, Default)]
pub struct WorkerEventState {
    pub usage: WorkerUsage,
    pub final_text: String,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
    /// Last observed tool call or turn, for the live view.
    pub activity: Option<String>,
    /// Bumped whenever `activity` or `usage` changes.
    pub activity_seq: u64,
    /// Full assistant message from the last `message_update` done/error event.
    /// `message_end` only carries `{role, content}`, so usage, stopReason and
    /// errorMessage have to be taken from here.
    pending_done_message: Option<Value>,
}

/// Short human hint for a tool call, e.g. `bash: cargo test` or `edit: worker.rs`.
pub fn describe_tool_call(tool_name: &str, args: &Value) -> String {
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            args.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
    };
    let mut hint = pick(&["command", "cmd"]).or_else(|| pick(&["path", "file_path", "filePath"]));
    if let Some(text) = &hint {
        if text.contains('/') || text.contains('\\') {
            hint = text
                .rsplit(['/', '\\'])
                .next()
                .map(str::to_string)
                .or(hint.clone());
        }
    }
    let hint = hint.or_else(|| pick(&["pattern", "query", "url"]));
    let Some(hint) = hint else {
        return tool_name.to_string();
    };
    let collapsed = hint.split_whitespace().collect::<Vec<_>>().join(" ");
    let shortened = if collapsed.chars().count() > 48 {
        let head: String = collapsed.chars().take(47).collect();
        format!("{head}…")
    } else {
        collapsed
    };
    format!("{tool_name}: {shortened}")
}

/// Fold one JSON event line from the child into the running worker state.
pub fn parse_worker_event(
    line: &str,
    state: &mut WorkerEventState,
    mut on_transcript: impl FnMut(&str),
) {
    if line.trim().is_empty() {
        return;
    }
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if event_type == "tool_execution_start" {
        if let Some(tool_name) = event.get("toolName").and_then(Value::as_str) {
            let hint = describe_tool_call(tool_name, event.get("args").unwrap_or(&Value::Null));
            state.activity = Some(hint.clone());
            state.activity_seq += 1;
            on_transcript(&format!("→ {hint}"));
        }
        return;
    }
    if event_type == "tool_execution_end" {
        if let Some(tool_name) = event.get("toolName").and_then(Value::as_str) {
            // The Rust child sends `result` as the text itself; the TS shape
            // is `{ content: [{ type: "text", text }] }`. Read both, or a
            // failed `graph_submit` shows as a bare ERROR with no reason.
            let excerpt = match event.get("result") {
                Some(Value::String(text)) => text.clone(),
                Some(result) => result
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|block| {
                                block.get("type").and_then(Value::as_str) == Some("text")
                            })
                            .filter_map(|block| block.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
                None => String::new(),
            };
            let excerpt: String = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
            let excerpt: String = excerpt.chars().take(200).collect();
            let error = if event.get("isError").and_then(Value::as_bool) == Some(true) {
                " ERROR"
            } else {
                ""
            };
            let suffix = if excerpt.is_empty() {
                String::new()
            } else {
                format!(": {excerpt}")
            };
            on_transcript(&format!("← {tool_name}{error}{suffix}"));
        }
        return;
    }
    if event_type == "message_update" {
        // The done/error stream event carries the only serialization of the
        // assistant message that still has usage/stopReason/errorMessage on
        // it; stash it for the matching `message_end`.
        let inner_type = event
            .pointer("/assistantMessageEvent/type")
            .and_then(Value::as_str);
        if matches!(inner_type, Some("done" | "error")) {
            if let Some(message) = event.pointer("/assistantMessageEvent/message") {
                if message.get("role").and_then(Value::as_str) == Some("assistant") {
                    state.pending_done_message = Some(message.clone());
                }
            }
        }
        return;
    }
    if event_type != "message_end" {
        return;
    }
    let Some(end_message) = event.get("message") else {
        return;
    };
    if end_message.get("role").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let pending = state.pending_done_message.take();
    let message = pending.as_ref().unwrap_or(end_message);

    state.usage.turns += 1;
    state.activity = Some(format!("turn {} done", state.usage.turns));
    state.activity_seq += 1;
    let number = |path: &str| -> u64 {
        message
            .pointer(path)
            .and_then(Value::as_u64)
            .unwrap_or_default()
    };
    state.usage.input += number("/usage/input");
    state.usage.output += number("/usage/output");
    state.usage.cache_read += number("/usage/cacheRead");
    state.usage.cache_write += number("/usage/cacheWrite");
    state.usage.cost_usd += message
        .pointer("/usage/cost/total")
        .and_then(Value::as_f64)
        .unwrap_or_default();

    let text = message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if !text.is_empty() {
        state.final_text = text.clone();
    }
    if let Some(stop_reason) = message.get("stopReason").and_then(Value::as_str) {
        state.stop_reason = Some(stop_reason.to_string());
    }
    if let Some(error_message) = message.get("errorMessage").and_then(Value::as_str) {
        state.error_message = Some(error_message.to_string());
    }

    if !text.is_empty() {
        let excerpt: String = text.chars().take(FINAL_TEXT_TRANSCRIPT_CHARS).collect();
        on_transcript(&excerpt);
    }
    let usage = &state.usage;
    let error = state
        .error_message
        .as_ref()
        .map(|message| format!(" — ERROR: {message}"))
        .unwrap_or_default();
    on_transcript(&format!(
        "■ turn {} — {}↑ {}↓ ${:.4}{error}",
        usage.turns, usage.input, usage.output, usage.cost_usd
    ));
}

pub fn build_worker_args(
    spec: &WorkerSpec,
    briefing_file: &Path,
    system_prompt_file: &Path,
) -> Vec<String> {
    // A worker is a `--print` child: nobody can answer a permission prompt
    // in it, and the product's default mode (`ask`) fails closed there — a
    // writer could not write, and no role could even `graph_submit`. The
    // worker's gate is the graph's own: the per-role `--tools` allowlist and
    // the bash policy in `worker_hooks`, both of which the parent set. Deny
    // rules from the user's settings still win in `auto`.
    let mut args = vec![
        "--mode".to_string(),
        "json".to_string(),
        "-p".to_string(),
        "--no-session".to_string(),
        "--no-extensions".to_string(),
        "--permission-mode".to_string(),
        "auto".to_string(),
    ];
    for extension in &spec.extra_extensions {
        args.push("-e".to_string());
        args.push(extension.clone());
    }
    args.push("--no-skills".to_string());
    args.push("--no-prompt-templates".to_string());
    args.push("--tools".to_string());
    args.push(spec.tools.join(","));
    if let Some(model) = &spec.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(thinking) = &spec.thinking_level {
        args.push("--thinking".to_string());
        args.push(thinking.clone());
    }
    if spec.project_trusted {
        args.push("-a".to_string());
    }
    args.push("--append-system-prompt".to_string());
    args.push(system_prompt_file.to_string_lossy().into_owned());
    args.push(format!("@{}", briefing_file.to_string_lossy()));
    args
}

/// A worker runner: given a spec and an abort flag, produce a result.
pub type WorkerRunner = dyn Fn(&WorkerSpec, &Arc<AtomicBool>, &mut dyn FnMut(&str, &WorkerUsage)) -> WorkerResult
    + Send
    + Sync;

fn append_transcript(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{text}");
    }
}

/// The `pi` a worker runs as. Normally this process, so a run always uses the
/// build that started it; `PI_GRAPH_WORKER_EXECUTABLE` overrides that for tests
/// and for pointing workers at a specific build.
fn worker_executable() -> std::io::Result<PathBuf> {
    match std::env::var_os("PI_GRAPH_WORKER_EXECUTABLE") {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => std::env::current_exe(),
    }
}

fn temp_dir_for(task_id: &str) -> PathBuf {
    let unique = format!(
        "pi-graph-{task_id}-{}-{}",
        std::process::id(),
        super::store::now_ms()
    );
    std::env::temp_dir().join(unique)
}

/// Spawn one real `pi` child for this node.
pub fn run_worker(
    spec: &WorkerSpec,
    abort: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(&str, &WorkerUsage),
) -> WorkerResult {
    let _ = fs::remove_file(&spec.artifact_path);

    let temp_dir = temp_dir_for(&spec.task_id);
    if let Err(error) = fs::create_dir_all(&temp_dir) {
        return WorkerResult {
            failure_reason: Some(format!("could not create the worker scratch dir: {error}")),
            ..WorkerResult::default()
        };
    }
    let briefing_file = temp_dir.join("briefing.md");
    let system_prompt_file = temp_dir.join("system.md");
    // The role prompt plus the artifact contract: the child's `graph_submit`
    // repeats the contract on the tool, but a model reads its system prompt
    // first and plans its whole turn around it.
    let system_prompt = format!(
        "{}\n\n{}",
        spec.system_prompt,
        super::validate::artifact_contract(spec.expect)
    );
    if fs::write(&briefing_file, &spec.briefing).is_err()
        || fs::write(&system_prompt_file, &system_prompt).is_err()
    {
        let _ = fs::remove_dir_all(&temp_dir);
        return WorkerResult {
            failure_reason: Some("could not stage the worker briefing files".into()),
            ..WorkerResult::default()
        };
    }

    let Ok(executable) = worker_executable() else {
        let _ = fs::remove_dir_all(&temp_dir);
        return WorkerResult {
            failure_reason: Some("could not resolve the pi executable for a worker".into()),
            ..WorkerResult::default()
        };
    };
    let cache_key = crate::native_extensions::ecosystem::cache_affinity::derive_worker_cache_key(
        &spec.cwd.to_string_lossy(),
        1,
        spec.role,
        spec.model.as_deref(),
        &spec.tools,
        &spec.system_prompt,
        spec.expect,
    );
    let mut command = Command::new(executable);
    command
        .args(build_worker_args(spec, &briefing_file, &system_prompt_file))
        .current_dir(&spec.cwd)
        .env("PI_GRAPH_ROLE", spec.role.as_str())
        .env("PI_GRAPH_EXPECT", spec.expect.as_str())
        .env("PI_GRAPH_ARTIFACT_PATH", &spec.artifact_path)
        .env("PI_GRAPH_EXTRA_TOOLS", spec.tools.join(","))
        .env("PI_GRAPH_CACHE_KEY", &cache_key);

    if let Some(path) = &spec.transcript_path {
        let model = spec
            .model
            .as_ref()
            .map(|model| format!(" — {model}"))
            .unwrap_or_default();
        append_transcript(
            path,
            &format!(
                "══ {} ({}) — {}{model}",
                spec.task_id,
                spec.role,
                iso8601_utc(now_ms())
            ),
        );
    }

    let mut state = WorkerEventState::default();
    let mut stderr = String::new();
    let mut reported_seq = 0;
    let transcript = spec.transcript_path.clone();

    let deadline = WorkerDeadline {
        run_deadline: spec.run_deadline,
        role_timeout: (spec.timeout_ms > 0)
            .then(|| std::time::Duration::from_millis(spec.timeout_ms)),
    };

    let outcome = {
        let state = &mut state;
        let stderr = &mut stderr;
        run_child_with_deadline(
            command,
            abort,
            deadline,
            |line| {
                parse_worker_event(line, state, |text| {
                    if let Some(path) = &transcript {
                        append_transcript(path, text);
                    }
                });
                if state.activity_seq != reported_seq {
                    reported_seq = state.activity_seq;
                    let activity = state
                        .activity
                        .clone()
                        .unwrap_or_else(|| format!("turn {}", state.usage.turns));
                    on_progress(&activity, &state.usage);
                }
            },
            |line| {
                stderr.push_str(line);
                stderr.push('\n');
            },
        )
    };
    let _ = fs::remove_dir_all(&temp_dir);

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return WorkerResult {
                stderr: format!("spawn error: {error}"),
                failure_reason: Some(format!("worker could not be spawned: {error}")),
                ..WorkerResult::default()
            }
        }
    };

    if let Some(path) = &spec.transcript_path {
        let suffix = match (
            outcome.run_deadline_exceeded,
            outcome.timed_out,
            outcome.aborted,
        ) {
            (true, _, _) => " (run deadline exceeded)",
            (_, true, _) => " (timed out)",
            (_, _, true) => " (aborted)",
            _ => "",
        };
        append_transcript(path, &format!("══ exited {}{suffix}", outcome.exit_code));
    }

    let stderr_tail: String = {
        let count = stderr.chars().count();
        stderr
            .chars()
            .skip(count.saturating_sub(STDERR_TAIL_CHARS))
            .collect()
    };
    let base = WorkerResult {
        ok: false,
        exit_code: outcome.exit_code,
        artifact: None,
        final_text: state.final_text.clone(),
        stderr: stderr_tail,
        usage: state.usage,
        timed_out: outcome.timed_out,
        run_deadline_exceeded: outcome.run_deadline_exceeded,
        failure_reason: None,
        child_pid: Some(outcome.pid),
    };

    if outcome.run_deadline_exceeded {
        return WorkerResult {
            failure_reason: Some("run deadline exceeded".to_string()),
            ..base
        };
    }

    // The artifact is the node's deliverable. One that reached disk and
    // validates is accepted even when the child then hit a provider error
    // or the deadline while still talking after `graph_submit`: a retry
    // would only buy the same artifact again at full cost. An abort still
    // cancels the node.
    if !outcome.aborted {
        if let Some(artifact) = fs::read_to_string(&spec.artifact_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| validate_artifact(spec.expect, &value).ok())
        {
            return WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..base
            };
        }
    }

    if outcome.aborted
        || outcome.timed_out
        || outcome.exit_code != 0
        || state.stop_reason.as_deref() == Some("error")
        || state.error_message.is_some()
    {
        let failure_reason = if outcome.aborted {
            "worker aborted".to_string()
        } else if outcome.timed_out {
            format!("worker timed out after {}ms", spec.timeout_ms)
        } else if let Some(message) = &state.error_message {
            format!(
                "worker {}: {message}",
                state.stop_reason.as_deref().unwrap_or("failed")
            )
        } else if state.stop_reason.as_deref() == Some("error") {
            "worker reported an error".to_string()
        } else {
            format!("worker exited with code {}", outcome.exit_code)
        };
        return WorkerResult {
            failure_reason: Some(failure_reason),
            ..base
        };
    }

    let Ok(raw) = fs::read_to_string(&spec.artifact_path) else {
        let detail = if state.final_text.trim().is_empty() {
            String::new()
        } else {
            let trimmed = state.final_text.trim();
            let count = trimmed.chars().count();
            let tail: String = trimmed.chars().skip(count.saturating_sub(500)).collect();
            format!(" Final response: {tail}")
        };
        return WorkerResult {
            failure_reason: Some(format!(
                "worker exited successfully without submitting a graph artifact.{detail}"
            )),
            ..base
        };
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return WorkerResult {
            failure_reason: Some("worker wrote an invalid JSON graph artifact".into()),
            ..base
        };
    };
    match validate_artifact(spec.expect, &parsed) {
        Ok(artifact) => WorkerResult {
            ok: true,
            artifact: Some(artifact),
            ..base
        },
        Err(errors) => {
            let failure_reason = format!("artifact invalid: {}", errors.join("; "));
            WorkerResult {
                stderr: format!("{}\n{failure_reason}", base.stderr),
                failure_reason: Some(failure_reason),
                ..base
            }
        }
    }
}

/// Zero-token fake execution: canned artifacts that satisfy the validators.
/// Lets a human watch the full graph run (`/graph --dry-run ...`) and gives the
/// test suite an end-to-end wiring check.
pub fn canned_artifact(expect: ArtifactKind) -> Artifact {
    match expect {
        ArtifactKind::Classification => Artifact::Classification(Classification {
            task_class: TaskClass::Bug,
            complexity: Complexity::Standard,
            rationale: "dry-run canned classification".into(),
            research_tasks: vec![ResearchRequest {
                kind: ResearchKind::CodeSearch,
                focus: "dry-run canned research focus".into(),
            }],
            milestones: None,
        }),
        ArtifactKind::Evidence => Artifact::Evidence(Box::new(EvidenceArtifact {
            kind: ResearchKind::CodeSearch,
            findings: vec![EvidenceFinding {
                claim: "dry-run canned finding".into(),
                refs: vec!["README.md:1".into()],
                confidence: super::types::Confidence::Low,
            }],
            risks: vec!["dry-run: no real investigation happened".into()],
            gaps: vec![],
            test_baseline: None,
        })),
        ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
            steps: vec![PlanStep {
                description: "dry-run canned step".into(),
                files: vec!["README.md".into()],
            }],
            tests_to_add: vec![],
            tests_to_run: vec![],
            completion_criteria: vec!["dry-run completes".into()],
            invariants: vec!["nothing is actually modified".into()],
            out_of_scope: vec!["everything real".into()],
        })),
        ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
            changed_files: vec![],
            summary: "dry-run: no files were changed".into(),
            deviations: vec![],
            plan_invalidated: false,
            invalidation_reason: None,
        })),
        ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
            verdict: Verdict::Approve,
            issues: vec![],
            notes: "dry-run canned approval".into(),
            reviewed_chunk_ids: vec![],
        })),
    }
}

pub fn run_dry_worker(
    spec: &WorkerSpec,
    _abort: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(&str, &WorkerUsage),
) -> WorkerResult {
    let usage = WorkerUsage::default();
    on_progress(&format!("{}: dry-run", spec.task_id), &usage);
    let artifact = canned_artifact(spec.expect);
    let _ = write_artifact(&spec.artifact_path, &artifact);
    WorkerResult {
        ok: true,
        exit_code: 0,
        artifact: Some(artifact),
        final_text: "(dry-run)".into(),
        stderr: String::new(),
        usage,
        timed_out: false,
        run_deadline_exceeded: false,
        failure_reason: None,
        child_pid: None,
    }
}

/// Run a fixture child process that sleeps beyond the deadline to verify
/// active process-tree termination.
#[allow(dead_code)]
pub fn run_fixture_worker_with_deadline(
    deadline: std::time::Duration,
) -> Result<WorkerResult, WorkerError> {
    let dir = std::env::temp_dir().join(format!(
        "pi-graph-fixture-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = fs::create_dir_all(&dir);
    let sleeper = if cfg!(windows) {
        "ping -n 20 127.0.0.1 > nul"
    } else {
        "sleep 20"
    };
    let abort = Arc::new(AtomicBool::new(false));
    let command = super::process::shell_command(sleeper, &dir);
    let deadline_spec = WorkerDeadline {
        run_deadline: Some(std::time::Instant::now() + deadline),
        role_timeout: None,
    };
    let outcome =
        super::process::run_child_with_deadline(command, &abort, deadline_spec, |_| {}, |_| {})
            .map_err(|e| WorkerError::SpawnFailed(e.to_string()))?;
    let _ = fs::remove_dir_all(&dir);

    // Assert that the child process tree was actively killed and is no longer alive
    assert!(
        !super::process::is_pid_alive(outcome.pid),
        "child process {} must be terminated when run deadline expires",
        outcome.pid
    );

    let result = WorkerResult {
        ok: false,
        exit_code: outcome.exit_code,
        artifact: None,
        final_text: String::new(),
        stderr: String::new(),
        usage: WorkerUsage::default(),
        timed_out: outcome.timed_out,
        run_deadline_exceeded: outcome.run_deadline_exceeded,
        failure_reason: if outcome.run_deadline_exceeded {
            Some("run deadline exceeded".to_string())
        } else if outcome.timed_out {
            Some("timed out".to_string())
        } else if outcome.aborted {
            Some("aborted".to_string())
        } else {
            None
        },
        child_pid: Some(outcome.pid),
    };
    result.into_result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::Role;
    use serde_json::json;

    fn spec() -> WorkerSpec {
        WorkerSpec {
            task_id: "review-1".into(),
            role: Role::Reviewer,
            expect: ArtifactKind::Review,
            briefing: "brief".into(),
            system_prompt: "system".into(),
            cwd: PathBuf::from("."),
            model: Some("openai/gpt".into()),
            thinking_level: Some("high".into()),
            tools: vec!["read".into(), "graph_submit".into()],
            extra_extensions: vec!["governor".into()],
            timeout_ms: 0,
            run_deadline: None,
            artifact_path: PathBuf::from("artifact.json"),
            transcript_path: None,
            project_trusted: true,
        }
    }

    #[test]
    fn active_run_deadline_kills_running_worker() {
        let started = std::time::Instant::now();
        let result = run_fixture_worker_with_deadline(std::time::Duration::from_millis(50));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "execution must terminate promptly on deadline"
        );
        assert!(matches!(result, Err(WorkerError::RunDeadlineExceeded)));
    }

    #[test]
    fn graph_deadline_kills_running_worker() {
        active_run_deadline_kills_running_worker();
    }

    #[test]
    fn worker_args_isolate_the_child_and_carry_the_allowlist() {
        let args = build_worker_args(&spec(), Path::new("brief.md"), Path::new("system.md"));
        let joined = args.join(" ");
        assert!(joined.contains("--mode json"));
        assert!(joined.contains("--no-session"));
        assert!(joined.contains("--no-extensions"));
        assert!(joined.contains("--no-skills"));
        assert!(joined.contains("--no-prompt-templates"));
        assert!(joined.contains("--tools read,graph_submit"));
        assert!(joined.contains("--model openai/gpt"));
        assert!(joined.contains("--thinking high"));
        assert!(joined.contains("-e governor"));
        assert!(joined.contains("-a"));
        assert!(args.last().unwrap().starts_with('@'));
    }

    #[test]
    fn an_untrusted_project_never_gets_the_approve_flag() {
        let mut spec = spec();
        spec.project_trusted = false;
        spec.model = None;
        spec.thinking_level = None;
        let args = build_worker_args(&spec, Path::new("brief.md"), Path::new("system.md"));
        assert!(!args.iter().any(|arg| arg == "-a"));
        assert!(!args.iter().any(|arg| arg == "--model"));
    }

    #[test]
    fn usage_and_final_text_are_folded_from_message_end_events() {
        let mut state = WorkerEventState::default();
        let event = json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "the answer"}],
                "usage": {"input": 10, "output": 4, "cacheRead": 2, "cacheWrite": 1,
                          "cost": {"total": 0.25}},
            }
        });
        parse_worker_event(&event.to_string(), &mut state, |_| {});
        assert_eq!(state.usage.input, 10);
        assert_eq!(state.usage.output, 4);
        assert_eq!(state.usage.cache_read, 2);
        assert_eq!(state.usage.turns, 1);
        assert!((state.usage.cost_usd - 0.25).abs() < f64::EPSILON);
        assert_eq!(state.final_text, "the answer");
    }

    #[test]
    fn usage_comes_from_the_message_update_done_event_in_the_live_stream() {
        // A real `pi --mode json` child emits `message_end` with only
        // `{role, content}`; usage/stopReason ride on the preceding
        // `message_update` done event.
        let mut state = WorkerEventState::default();
        let done = json!({
            "type": "message_update",
            "usage": null,
            "assistantMessageEvent": {
                "type": "done",
                "reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "the answer"}],
                    "stopReason": "stop",
                    "usage": {"input": 10, "output": 4, "cacheRead": 2, "cacheWrite": 1,
                              "cost": {"input": 0.1, "output": 0.1, "cacheRead": 0.03,
                                       "cacheWrite": 0.02, "total": 0.25}},
                }
            }
        });
        parse_worker_event(&done.to_string(), &mut state, |_| {});
        // Usage is not folded until the message actually ends.
        assert_eq!(state.usage.turns, 0);
        let end = json!({
            "type": "message_end",
            "message": {"role": "assistant",
                        "content": [{"type": "text", "text": "the answer"}]}
        });
        parse_worker_event(&end.to_string(), &mut state, |_| {});
        assert_eq!(state.usage.turns, 1);
        assert_eq!(state.usage.input, 10);
        assert_eq!(state.usage.output, 4);
        assert!((state.usage.cost_usd - 0.25).abs() < f64::EPSILON);
        assert_eq!(state.stop_reason.as_deref(), Some("stop"));
        assert_eq!(state.final_text, "the answer");
    }

    #[test]
    fn a_provider_error_on_a_turn_is_captured() {
        let mut state = WorkerEventState::default();
        let event = json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [], "stopReason": "error",
                        "errorMessage": "fetch failed"}
        });
        let mut transcript = Vec::new();
        parse_worker_event(&event.to_string(), &mut state, |text| {
            transcript.push(text.to_string())
        });
        assert_eq!(state.error_message.as_deref(), Some("fetch failed"));
        assert_eq!(state.stop_reason.as_deref(), Some("error"));
        assert!(transcript
            .iter()
            .any(|line| line.contains("ERROR: fetch failed")));
    }

    #[test]
    fn tool_events_become_short_transcript_lines() {
        let mut state = WorkerEventState::default();
        let mut transcript = Vec::new();
        let start = json!({
            "type": "tool_execution_start", "toolCallId": "1", "toolName": "bash",
            "args": {"command": "cargo test --workspace"}
        });
        parse_worker_event(&start.to_string(), &mut state, |text| {
            transcript.push(text.to_string())
        });
        let end = json!({
            "type": "tool_execution_end", "toolCallId": "1", "toolName": "bash",
            "isError": true, "result": {"content": [{"type": "text", "text": "boom"}]}
        });
        parse_worker_event(&end.to_string(), &mut state, |text| {
            transcript.push(text.to_string())
        });
        assert_eq!(transcript[0], "→ bash: cargo test --workspace");
        assert_eq!(transcript[1], "← bash ERROR: boom");
        assert_eq!(
            state.activity.as_deref(),
            Some("bash: cargo test --workspace")
        );
    }

    #[test]
    fn garbage_lines_are_ignored_rather_than_fatal() {
        let mut state = WorkerEventState::default();
        parse_worker_event("not json at all", &mut state, |_| {});
        parse_worker_event("", &mut state, |_| {});
        assert_eq!(state.usage.turns, 0);
    }

    #[test]
    fn a_file_path_argument_is_shortened_to_its_basename() {
        let hint = describe_tool_call("edit", &json!({"path": "crates/pi/src/deep/file.rs"}));
        assert_eq!(hint, "edit: file.rs");
    }

    /// End-to-end against a real `pi` child. `current_exe()` is the test
    /// harness here, so this only runs when `PI_GRAPH_WORKER_EXECUTABLE` points
    /// at a built binary:
    ///
    /// ```text
    /// PI_GRAPH_WORKER_EXECUTABLE=target/debug/pi cargo test -p davinci-coding-agent \
    ///     --lib graph::worker::tests::a_real_child
    /// ```
    ///
    /// Offline, the child answers in prose and never calls graph_submit, which
    /// is precisely the failure the parent must not mistake for success.
    #[test]
    fn a_real_child_that_never_submits_is_reported_as_a_failure() {
        let Some(executable) = std::env::var_os("PI_GRAPH_WORKER_EXECUTABLE") else {
            return;
        };
        if !Path::new(&executable).is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut spec = spec();
        spec.cwd = dir.path().to_path_buf();
        spec.artifact_path = dir.path().join("artifact.json");
        spec.transcript_path = Some(dir.path().join("live.log"));
        spec.model = None;
        spec.thinking_level = None;
        spec.extra_extensions = Vec::new();
        spec.tools = vec!["read".into(), "graph_submit".into()];

        let abort = Arc::new(AtomicBool::new(false));
        let mut progress = |_: &str, _: &WorkerUsage| {};
        let result = run_worker(&spec, &abort, &mut progress);

        assert!(!result.ok, "a child that never submitted must not be ok");
        assert!(!spec.artifact_path.exists());
        let reason = result.failure_reason.expect("a reason");
        assert!(
            reason.contains("without submitting a graph artifact"),
            "unexpected reason: {reason}"
        );
        let transcript = std::fs::read_to_string(spec.transcript_path.unwrap()).unwrap();
        assert!(transcript.contains("══ review-1 (reviewer)"));
        assert!(transcript.contains("══ exited"));
    }

    #[test]
    fn every_canned_artifact_satisfies_its_own_validator() {
        for kind in ArtifactKind::ALL {
            let artifact = canned_artifact(*kind);
            let value = serde_json::to_value(&artifact).expect("serializes");
            validate_artifact(*kind, &value)
                .unwrap_or_else(|errors| panic!("{kind} canned artifact invalid: {errors:?}"));
        }
    }

    #[test]
    fn worker_args_and_env_establish_cache_affinity_without_session() {
        let dir = tempfile::tempdir().unwrap();
        let briefing_file = dir.path().join("briefing.md");
        let system_file = dir.path().join("system.md");
        let mut test_spec = spec();
        test_spec.tools = vec!["read".into(), "grep".into(), "graph_submit".into()];
        let args = build_worker_args(&test_spec, &briefing_file, &system_file);

        // Child process must be explicitly told not to create or persist sessions
        assert!(args.contains(&"--no-session".to_string()));
        assert!(args.contains(&"--no-skills".to_string()));
        assert!(args.contains(&"--no-extensions".to_string()));

        // Derive the cache key for this spec
        let key = crate::native_extensions::ecosystem::cache_affinity::derive_worker_cache_key(
            &test_spec.cwd.to_string_lossy(),
            1,
            test_spec.role,
            test_spec.model.as_deref(),
            &test_spec.tools,
            &test_spec.system_prompt,
            test_spec.expect,
        );
        assert!(!key.is_empty());
        assert!(key.starts_with("gw-reviewer-"));

        // Simulate the child worker environment where PI_GRAPH_CACHE_KEY is passed
        let simulated_options = davinci_ai::StreamOptions {
            session_id: None, // Because child has --no-session
            cache_key: Some(key.clone()),
            ..davinci_ai::StreamOptions::default()
        };
        assert_eq!(simulated_options.session_id, None);
        assert_eq!(simulated_options.cache_key.as_deref(), Some(key.as_str()));
        assert_eq!(
            davinci_ai::cache::effective_prompt_cache_key(&simulated_options),
            Some(key.as_str())
        );
    }
}
