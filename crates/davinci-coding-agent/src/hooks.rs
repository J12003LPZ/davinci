//! User hooks: commands run around tools and at stop.
//!
//! No TypeScript counterpart. Phase 6 spec:
//! `docs/superpowers/specs/2026-09-01-hooks-and-observability-design.md`.
//!
//! A hook is an argv. It gets `PI_HOOK_KIND` and `PI_HOOK_TOOL` in its
//! environment and one JSON document on stdin — `{kind, tool, args, result}`
//! — because a `write` of a large file does not fit the environment block
//! (32 KB on Windows) and a hook's input should not be readable by every
//! process inspector for as long as the hook runs. A `preTool` hook that
//! exits non-zero blocks the call with its stderr (or stdout) as the reason;
//! `postTool` and `stop` hooks are run for their effect only.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

/// The longest a hook may run before it is killed and reported. A hung
/// `preTool` hook would otherwise wedge the turn; a hung `stop` hook, the
/// exit.
const HOOK_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksFile {
    #[serde(default)]
    pub session_start: Vec<Vec<String>>,
    #[serde(default)]
    pub user_prompt_submit: Vec<Vec<String>>,
    #[serde(default)]
    pub pre_tool: Vec<Vec<String>>,
    #[serde(default)]
    pub permission_request: Vec<Vec<String>>,
    #[serde(default)]
    pub post_tool: Vec<Vec<String>>,
    #[serde(default)]
    pub post_tool_failure: Vec<Vec<String>>,
    #[serde(default)]
    pub post_tool_batch: Vec<Vec<String>>,
    #[serde(default)]
    pub subagent_start: Vec<Vec<String>>,
    #[serde(default)]
    pub subagent_stop: Vec<Vec<String>>,
    #[serde(default)]
    pub task_created: Vec<Vec<String>>,
    #[serde(default)]
    pub task_completed: Vec<Vec<String>>,
    #[serde(default)]
    pub pre_compact: Vec<Vec<String>>,
    #[serde(default)]
    pub post_compact: Vec<Vec<String>>,
    #[serde(default)]
    pub pre_model_switch: Vec<Vec<String>>,
    #[serde(default)]
    pub post_model_switch: Vec<Vec<String>>,
    #[serde(default)]
    pub session_end: Vec<Vec<String>>,
    #[serde(default)]
    pub stop: Vec<Vec<String>>,
}

pub fn load(agent_dir: &Path, cwd: &Path, trusted: bool) -> HooksFile {
    if let Ok(path) = std::env::var("PI_HOOKS_CONFIG") {
        return load_path(Path::new(&path));
    }
    let mut file = load_path(&agent_dir.join("hooks.json"));
    if trusted {
        let project = load_path(&cwd.join(".pi").join("hooks.json"));
        file.session_start.extend(project.session_start);
        file.user_prompt_submit.extend(project.user_prompt_submit);
        file.pre_tool.extend(project.pre_tool);
        file.permission_request.extend(project.permission_request);
        file.post_tool.extend(project.post_tool);
        file.post_tool_failure.extend(project.post_tool_failure);
        file.post_tool_batch.extend(project.post_tool_batch);
        file.subagent_start.extend(project.subagent_start);
        file.subagent_stop.extend(project.subagent_stop);
        file.task_created.extend(project.task_created);
        file.task_completed.extend(project.task_completed);
        file.pre_compact.extend(project.pre_compact);
        file.post_compact.extend(project.post_compact);
        file.pre_model_switch.extend(project.pre_model_switch);
        file.post_model_switch.extend(project.post_model_switch);
        file.session_end.extend(project.session_end);
        file.stop.extend(project.stop);
    }
    file
}

fn load_path(path: &Path) -> HooksFile {
    let Ok(body) = std::fs::read_to_string(path) else {
        return HooksFile::default();
    };
    match serde_json::from_str(&body) {
        Ok(file) => file,
        Err(err) => {
            // Hooks that silently switch off are a guard the user believes
            // is up.
            eprintln!("pi: ignoring {}: {err}", path.display());
            HooksFile::default()
        }
    }
}

pub fn hook_kind_for(event: &davinci_agent::RuntimeEvent) -> Option<&'static str> {
    match event {
        davinci_agent::RuntimeEvent::SessionStarted { .. } => Some("sessionStart"),
        davinci_agent::RuntimeEvent::UserPromptSubmitted => Some("userPromptSubmit"),
        davinci_agent::RuntimeEvent::PreToolUse { .. } => Some("preTool"),
        davinci_agent::RuntimeEvent::PermissionRequested { .. } => Some("permissionRequest"),
        davinci_agent::RuntimeEvent::PostToolUse { is_error: true, .. } => Some("postToolFailure"),
        davinci_agent::RuntimeEvent::PostToolUse {
            is_error: false, ..
        } => Some("postTool"),
        davinci_agent::RuntimeEvent::PostToolBatch { .. } => Some("postToolBatch"),
        davinci_agent::RuntimeEvent::AgentStarted {
            record:
                davinci_agent::AgentRecord {
                    kind: davinci_agent::AgentKind::Subagent,
                    ..
                },
        } => Some("subagentStart"),
        davinci_agent::RuntimeEvent::AgentStateChanged {
            to:
                davinci_agent::AgentState::Completed
                | davinci_agent::AgentState::Failed
                | davinci_agent::AgentState::Cancelled,
            ..
        } => Some("subagentStop"),
        davinci_agent::RuntimeEvent::TaskCreated { .. } => Some("taskCreated"),
        // Keep the existing hook as the pre-completion gate; do not execute it
        // twice when the successful commit observation follows the proposal.
        davinci_agent::RuntimeEvent::TaskCompletionRequested { .. }
        | davinci_agent::RuntimeEvent::TaskCompleted { success: false, .. } => {
            Some("taskCompleted")
        }
        davinci_agent::RuntimeEvent::PreCompact { .. } => Some("preCompact"),
        davinci_agent::RuntimeEvent::PostCompact { .. } => Some("postCompact"),
        davinci_agent::RuntimeEvent::PreModelSwitch { .. } => Some("preModelSwitch"),
        davinci_agent::RuntimeEvent::PostModelSwitch { .. } => Some("postModelSwitch"),
        davinci_agent::RuntimeEvent::SessionEnded { .. } => Some("sessionEnd"),
        _ => None,
    }
}

/// Run `preTool` hooks. A non-zero exit returns the stderr/stdout as a block
/// reason.
pub fn run_pre_tool(hooks: &HooksFile, tool: &str, args: &Value) -> Option<String> {
    for argv in &hooks.pre_tool {
        if let Some(reason) = run_one(argv, "preTool", tool, args, None) {
            return Some(reason);
        }
    }
    None
}

pub fn run_post_tool(hooks: &HooksFile, tool: &str, args: &Value, result: &str) {
    for argv in &hooks.post_tool {
        let _ = run_one(argv, "postTool", tool, args, Some(result));
    }
}

/// One row of `<session>.events.jsonl`: when, what kind (`tool`, `denied`),
/// which tool, and — for a tool row — the call id and whether it succeeded.
pub fn append_event(
    session_path: Option<&PathBuf>,
    kind: &str,
    tool: &str,
    tool_call_id: Option<&str>,
    ok: Option<bool>,
) {
    let Some(path) = session_path else {
        return;
    };
    let file = path.with_extension("events.jsonl");
    let mut row = serde_json::json!({
        "ts": davinci_session::now_ms(),
        "kind": kind,
        "tool": tool,
    });
    if let Some(id) = tool_call_id {
        row["toolCallId"] = Value::String(id.to_string());
    }
    if let Some(ok) = ok {
        row["ok"] = Value::Bool(ok);
    }
    if let Ok(mut out) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(out, "{row}");
    }
}

pub fn run_stop(hooks: &HooksFile) {
    for argv in &hooks.stop {
        let _ = run_one(argv, "stop", "", &Value::Null, None);
    }
}

pub fn run_one(
    argv: &[String],
    kind: &str,
    tool: &str,
    args: &Value,
    result: Option<&str>,
) -> Option<String> {
    run_one_envelope(argv, kind, tool, args, result, None)
}

pub fn run_one_envelope(
    argv: &[String],
    kind: &str,
    tool: &str,
    args: &Value,
    result: Option<&str>,
    envelope: Option<&davinci_agent::RuntimeEventEnvelope>,
) -> Option<String> {
    let program = argv.first()?;
    if std::env::var("PI_HOOKS_DRY_RUN").is_ok() {
        return None;
    }
    let mut payload = serde_json::json!({
        "kind": kind,
        "tool": tool,
        "args": args,
        "result": result,
    });
    if let Some(env) = envelope {
        payload["schemaVersion"] = serde_json::json!(env.schema_version);
        payload["runId"] = serde_json::json!(env.run_id.to_string());
        if let Some(agent_id) = env.agent_id {
            payload["agentId"] = serde_json::json!(agent_id.to_string());
        }
        if let Some(session_id) = &env.session_id {
            payload["sessionId"] = serde_json::json!(session_id);
        }
        payload["event"] = serde_json::to_value(&env.payload).unwrap_or(Value::Null);
        // Shell hooks retain their existing pre-completion input contract.
        // Only this compatibility adapter uses the old name: runtime observers
        // and persistence receive TaskCompletionRequested, never a false fact.
        if matches!(
            env.payload,
            davinci_agent::RuntimeEvent::TaskCompletionRequested { .. }
        ) {
            payload["event"]["kind"] = serde_json::json!("task_completed");
            payload["event"]["success"] = serde_json::json!(true);
            payload["event"]["phase"] = serde_json::json!("proposal");
        }
    }
    let payload_str = payload.to_string();
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
        .env("PI_HOOK_KIND", kind)
        .env("PI_HOOK_TOOL", tool)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return Some(format!("hook `{program}` failed: {err}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_str.as_bytes());
    }
    let stdout = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
            buf
        })
    });
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
            buf
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= HOOK_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Some(format!(
                    "hook `{program}` timed out after {}s",
                    HOOK_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(err) => return Some(format!("hook `{program}` failed: {err}")),
        }
    };
    let stdout = stdout
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    if status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&stderr).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&stdout).into_owned();
    }
    Some(format!("hook `{program}` blocked {tool}: {}", text.trim()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn f03_completion_hook_runs_for_proposal_not_success_observation() {
        use davinci_agent::{RuntimeEvent, TaskId};
        let task_id = TaskId::new();
        assert_eq!(
            super::hook_kind_for(&RuntimeEvent::TaskCompletionRequested {
                task_id,
                expected_revision: 3,
            }),
            Some("taskCompleted")
        );
        assert_eq!(
            super::hook_kind_for(&RuntimeEvent::TaskCompleted {
                task_id,
                success: true
            }),
            None
        );
        // Preserve the historical failure notification hook.
        assert_eq!(
            super::hook_kind_for(&RuntimeEvent::TaskCompleted {
                task_id,
                success: false
            }),
            Some("taskCompleted")
        );
    }
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// A hook that exits with `code` after echoing its stdin to a file.
    fn shell_hook(code: i32, capture: &Path) -> Vec<String> {
        let capture = capture.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            vec![
                "powershell".into(),
                "-NoProfile".into(),
                "-Command".into(),
                format!("$input | Out-File -Encoding utf8 '{capture}'; exit {code}"),
            ]
        } else {
            vec![
                "sh".into(),
                "-c".into(),
                format!("cat > '{capture}'; exit {code}"),
            ]
        }
    }

    #[test]
    fn an_untrusted_project_file_is_ignored() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HOOKS_CONFIG");
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(project.join(".pi")).unwrap();
        std::fs::write(agent.join("hooks.json"), r#"{"preTool":[["echo","user"]]}"#).unwrap();
        std::fs::write(
            project.join(".pi").join("hooks.json"),
            r#"{"preTool":[["echo","project"]]}"#,
        )
        .unwrap();
        let untrusted = load(&agent, &project, false);
        assert_eq!(untrusted.pre_tool.len(), 1);
        assert_eq!(untrusted.pre_tool[0][1], "user");
        let trusted = load(&agent, &project, true);
        assert_eq!(trusted.pre_tool.len(), 2);
    }

    #[test]
    fn pi_hooks_config_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.json");
        std::fs::write(&path, r#"{"stop":[["true"]]}"#).unwrap();
        std::env::set_var("PI_HOOKS_CONFIG", &path);
        let loaded = load(Path::new("/nope"), Path::new("/nope"), true);
        std::env::remove_var("PI_HOOKS_CONFIG");
        assert_eq!(loaded.stop.len(), 1);
    }

    #[test]
    fn a_malformed_file_loads_as_no_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = load_path(&path);
        assert!(loaded.pre_tool.is_empty() && loaded.stop.is_empty());
    }

    #[test]
    fn a_failing_pre_tool_hook_blocks_and_gets_the_call_on_stdin() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("seen.json");
        let big = "x".repeat(64 * 1024);
        let args = serde_json::json!({ "path": "notes.md", "content": big });
        let hooks = HooksFile {
            pre_tool: vec![shell_hook(3, &capture)],
            ..HooksFile::default()
        };
        let reason = run_pre_tool(&hooks, "write", &args).expect("blocked");
        assert!(reason.contains("blocked write"), "{reason}");
        let seen = std::fs::read_to_string(&capture).unwrap();
        let seen = seen.trim_start_matches('\u{feff}');
        let seen: Value = serde_json::from_str(seen.trim()).unwrap();
        assert_eq!(seen["kind"], "preTool");
        assert_eq!(seen["tool"], "write");
        assert_eq!(seen["args"]["path"], "notes.md");
        assert_eq!(seen["args"]["content"].as_str().unwrap().len(), 64 * 1024);

        let passing = HooksFile {
            pre_tool: vec![shell_hook(0, &capture)],
            ..HooksFile::default()
        };
        assert!(run_pre_tool(&passing, "write", &args).is_none());
    }

    #[test]
    fn dry_run_skips_every_hook() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("PI_HOOKS_DRY_RUN", "1");
        let hooks = HooksFile {
            pre_tool: vec![vec!["definitely-not-a-program".into()]],
            ..HooksFile::default()
        };
        let blocked = run_pre_tool(&hooks, "bash", &Value::Null);
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        assert!(blocked.is_none());
    }

    #[test]
    fn event_rows_carry_the_call_id_and_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("s.jsonl");
        append_event(Some(&session), "tool", "read", Some("call_1"), Some(true));
        append_event(
            Some(&session),
            "denied",
            "bash",
            Some("call_2"),
            Some(false),
        );
        let rows = std::fs::read_to_string(session.with_extension("events.jsonl")).unwrap();
        let rows: Vec<Value> = rows
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows[0]["kind"], "tool");
        assert_eq!(rows[0]["toolCallId"], "call_1");
        assert_eq!(rows[0]["ok"], true);
        assert_eq!(rows[1]["kind"], "denied");
        assert_eq!(rows[1]["ok"], false);
    }

    #[test]
    fn old_config_compatibility_and_defaults() {
        let json = r#"{
            "preTool": [["echo", "pre"]],
            "postTool": [["echo", "post"]],
            "stop": [["echo", "stop"]]
        }"#;
        let hooks: HooksFile = serde_json::from_str(json).unwrap();
        assert_eq!(hooks.pre_tool.len(), 1);
        assert_eq!(hooks.post_tool.len(), 1);
        assert_eq!(hooks.stop.len(), 1);
        assert!(hooks.session_start.is_empty());
        assert!(hooks.pre_compact.is_empty());
        assert!(hooks.post_compact.is_empty());
        assert!(hooks.task_completed.is_empty());
    }

    #[test]
    fn trusted_project_loads_new_hook_vectors() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HOOKS_CONFIG");
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(project.join(".pi")).unwrap();
        std::fs::write(
            agent.join("hooks.json"),
            r#"{"preCompact":[["echo","agent_compact"]]}"#,
        )
        .unwrap();
        std::fs::write(
            project.join(".pi").join("hooks.json"),
            r#"{"preCompact":[["echo","project_compact"]]}"#,
        )
        .unwrap();

        let untrusted = load(&agent, &project, false);
        assert_eq!(untrusted.pre_compact.len(), 1);
        assert_eq!(untrusted.pre_compact[0][1], "agent_compact");

        let trusted = load(&agent, &project, true);
        assert_eq!(trusted.pre_compact.len(), 2);
        assert_eq!(trusted.pre_compact[1][1], "project_compact");
    }

    #[test]
    fn f03_legacy_completion_hook_can_still_deny_by_payload() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous_dry_run = std::env::var_os("PI_HOOKS_DRY_RUN");
        let previous_v2 = std::env::var_os("DAVINCI_RUNTIME_HOOKS_V2");
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        std::env::set_var("DAVINCI_RUNTIME_HOOKS_V2", "1");
        let command = if cfg!(windows) {
            vec!["powershell".into(), "-NoProfile".into(), "-Command".into(),
                "$p = $input | ConvertFrom-Json; if ($p.event.kind -eq 'task_completed' -and $p.event.success -eq $true) { Write-Output 'legacy completion denied'; exit 1 }; exit 0".into()]
        } else {
            vec!["sh".into(), "-c".into(),
                r#"payload=$(cat); case "$payload" in *'"kind":"task_completed"'*) case "$payload" in *'"success":true'*) echo 'legacy completion denied'; exit 1;; esac;; esac; exit 0"#.into()]
        };
        let bus = davinci_agent::RuntimeBus::new();
        bus.subscribe(std::sync::Arc::new(
            crate::runtime_host::HooksRuntimeSubscriber::new(HooksFile {
                task_completed: vec![command],
                ..Default::default()
            }),
        ));
        let registry = davinci_agent::TaskRegistry::with_bus(bus);
        let id = registry
            .create_task(davinci_agent::TaskRecord::new(
                davinci_agent::RunId::new(),
                "legacy gate",
            ))
            .unwrap();
        let result = registry.complete_task(id, None);
        if let Some(value) = previous_dry_run {
            std::env::set_var("PI_HOOKS_DRY_RUN", value);
        }
        match previous_v2 {
            Some(value) => std::env::set_var("DAVINCI_RUNTIME_HOOKS_V2", value),
            None => std::env::remove_var("DAVINCI_RUNTIME_HOOKS_V2"),
        }
        assert!(
            matches!(result, Err(davinci_agent::TaskError::CompletionRefused(reason)) if reason.contains("legacy completion denied"))
        );
        assert_eq!(
            registry.get_task(&id).unwrap().state,
            davinci_agent::TaskState::Ready
        );
    }

    #[test]
    fn f03_completion_proposal_preserves_legacy_hook_payload() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous_dry_run = std::env::var_os("PI_HOOKS_DRY_RUN");
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("completion.json");
        let task_id = davinci_agent::TaskId::new();
        let envelope = davinci_agent::RuntimeEventEnvelope::new(
            1,
            davinci_agent::RunId::new(),
            None,
            None,
            None,
            davinci_agent::RuntimeEvent::TaskCompletionRequested {
                task_id,
                expected_revision: 7,
            },
        );
        let reason = run_one_envelope(
            &shell_hook(0, &capture),
            "taskCompleted",
            "",
            &Value::Null,
            None,
            Some(&envelope),
        );
        if let Some(value) = previous_dry_run {
            std::env::set_var("PI_HOOKS_DRY_RUN", value);
        }
        assert!(reason.is_none());
        let text = std::fs::read_to_string(capture).unwrap();
        let payload: Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).unwrap();
        assert_eq!(payload["kind"], "taskCompleted");
        assert_eq!(payload["event"]["kind"], "task_completed");
        assert_eq!(payload["event"]["success"], true);
        assert_eq!(payload["event"]["task_id"], task_id.to_string());
        assert_eq!(payload["event"]["expected_revision"], 7);
        assert_eq!(payload["event"]["phase"], "proposal");
    }

    #[test]
    fn new_event_dispatch_and_stdin_payload() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("envelope_seen.json");
        let run_id = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();
        let envelope = davinci_agent::RuntimeEventEnvelope::new(
            7,
            run_id,
            Some("session_xyz".into()),
            Some(agent_id),
            None,
            davinci_agent::RuntimeEvent::PreCompact {
                estimated_tokens: 150_000,
            },
        );

        let argv = shell_hook(0, &capture);
        let reason = run_one_envelope(
            &argv,
            "preCompact",
            "",
            &serde_json::Value::Null,
            None,
            Some(&envelope),
        );
        assert!(reason.is_none());

        let seen = std::fs::read_to_string(&capture).unwrap();
        let seen = seen.trim_start_matches('\u{feff}');
        let val: Value = serde_json::from_str(seen.trim()).unwrap();
        assert_eq!(val["kind"], "preCompact");
        assert_eq!(val["schemaVersion"], 1);
        assert_eq!(val["runId"], run_id.to_string());
        assert_eq!(val["agentId"], agent_id.to_string());
        assert_eq!(val["sessionId"], "session_xyz");
        assert_eq!(val["event"]["kind"], "pre_compact");
        assert_eq!(val["event"]["estimated_tokens"], 150_000);
    }

    #[test]
    fn hooks_runtime_subscriber_decision_and_kill_switch() {
        use crate::runtime_host::HooksRuntimeSubscriber;
        use davinci_agent::{RuntimeDecision, RuntimeSubscriber};

        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HOOKS_DRY_RUN");
        std::env::remove_var("DAVINCI_RUNTIME_HOOKS_V2");
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("decision_seen.json");

        let hooks = HooksFile {
            pre_tool: vec![shell_hook(2, &capture)],
            post_tool: vec![shell_hook(2, &capture)],
            ..HooksFile::default()
        };

        let subscriber = HooksRuntimeSubscriber::new(hooks);

        // PreToolUse is a decision event: failing exit status blocks with Deny
        let pre_event = davinci_agent::RuntimeEventEnvelope::new(
            1,
            davinci_agent::RunId::new(),
            None,
            Some(davinci_agent::AgentId::new()),
            None,
            davinci_agent::RuntimeEvent::PreToolUse {
                call_id: "c1".into(),
                tool: "write".into(),
                args: serde_json::json!({"path": "foo.txt"}),
            },
        );
        let dec = subscriber.on_event(&pre_event);
        assert!(matches!(dec, RuntimeDecision::Deny { .. }));

        // PostToolUse is an observe-only event: failing exit status fails open with Continue
        let post_event = davinci_agent::RuntimeEventEnvelope::new(
            2,
            davinci_agent::RunId::new(),
            None,
            Some(davinci_agent::AgentId::new()),
            None,
            davinci_agent::RuntimeEvent::PostToolUse {
                call_id: "c1".into(),
                tool: "write".into(),
                is_error: false,
            },
        );
        let dec_post = subscriber.on_event(&post_event);
        assert_eq!(dec_post, RuntimeDecision::Continue);

        // With kill switch DAVINCI_RUNTIME_HOOKS_V2=0, pre_event is skipped and returns Continue
        std::env::set_var("DAVINCI_RUNTIME_HOOKS_V2", "0");
        let dec_killed = subscriber.on_event(&pre_event);
        std::env::remove_var("DAVINCI_RUNTIME_HOOKS_V2");
        assert_eq!(dec_killed, RuntimeDecision::Continue);
    }
}
