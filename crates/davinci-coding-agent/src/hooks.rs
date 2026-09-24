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
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Maximum allowed hook payload size for stdin and captured stdout/stderr streams.
pub const MAX_HOOK_STREAM_BYTES: usize = 64 * 1024;

/// Default hook timeout when not overridden by rule or settings.
#[allow(dead_code)]
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// Legacy fallback timeout.
const HOOK_TIMEOUT: Duration = Duration::from_secs(60);

/// Explicit failure policy for hook rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HookFailurePolicy {
    #[default]
    Block,
    Warn,
    Ignore,
}

/// Structured deterministic hook rule supporting typed event, tool, and path glob filters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPolicyRule {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, rename = "path", skip_serializing_if = "Option::is_none")]
    pub path_pattern: Option<String>,
    #[serde(default)]
    pub action: Vec<String>,
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, rename = "onFailure")]
    pub on_failure: HookFailurePolicy,
}

impl HookPolicyRule {
    pub fn matches(&self, event: &str, tool: &str, path: Option<&Path>) -> bool {
        let normalized_rule_event = normalize_event_name(&self.event);
        let normalized_target_event = normalize_event_name(event);
        if normalized_rule_event != normalized_target_event {
            return false;
        }
        if let Some(rule_tool) = &self.tool {
            if !rule_tool.is_empty() && !rule_tool.eq_ignore_ascii_case(tool) {
                return false;
            }
        }
        if let Some(pattern) = &self.path_pattern {
            let Some(p) = path else {
                return false;
            };
            if !matches_path(pattern, p) {
                return false;
            }
        }
        true
    }
}

/// Settings configuration for Hook Policy Engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPolicyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default)]
    pub default_failure_policy: HookFailurePolicy,
}

fn default_true() -> bool {
    true
}

fn default_max_depth() -> usize {
    3
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Default for HookPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 3,
            default_timeout_ms: 10_000,
            default_failure_policy: HookFailurePolicy::Block,
        }
    }
}

/// Global telemetry recording hook executions and outcomes.
#[derive(Default)]
pub struct HookTelemetry {
    pub executed: AtomicUsize,
    pub blocked: AtomicUsize,
    pub warned: AtomicUsize,
    pub ignored: AtomicUsize,
    pub timed_out: AtomicUsize,
    pub recursion_stopped: AtomicUsize,
}

pub static GLOBAL_HOOK_TELEMETRY: HookTelemetry = HookTelemetry {
    executed: AtomicUsize::new(0),
    blocked: AtomicUsize::new(0),
    warned: AtomicUsize::new(0),
    ignored: AtomicUsize::new(0),
    timed_out: AtomicUsize::new(0),
    recursion_stopped: AtomicUsize::new(0),
};

std::thread_local! {
    static HOOK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// RAII guard bounding self-trigger recursion across hook invocations.
#[derive(Debug)]
pub struct HookDepthGuard;

impl HookDepthGuard {
    pub fn enter(max_depth: usize) -> Result<Self, String> {
        let env_depth = std::env::var("DAVINCI_HOOK_DEPTH")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let current_depth = HOOK_DEPTH.with(|d| d.get()).max(env_depth);
        if current_depth >= max_depth {
            GLOBAL_HOOK_TELEMETRY
                .recursion_stopped
                .fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "hook recursion depth limit exceeded (depth: {current_depth} >= max: {max_depth})"
            ));
        }
        HOOK_DEPTH.with(|d| d.set(current_depth + 1));
        Ok(Self)
    }
}

impl Drop for HookDepthGuard {
    fn drop(&mut self) {
        HOOK_DEPTH.with(|d| {
            let depth = d.get();
            if depth > 0 {
                d.set(depth - 1);
            }
        });
    }
}

pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn matches_path(pattern: &str, candidate: &Path) -> bool {
    let path_str = candidate.to_string_lossy().replace('\\', "/");
    let pattern_str = pattern.replace('\\', "/");

    if pattern_str.starts_with("*.") {
        let ext = &pattern_str[1..];
        if path_str.ends_with(ext) {
            return true;
        }
    }

    if let Ok(glob) = globset::Glob::new(&pattern_str) {
        if glob.compile_matcher().is_match(Path::new(&path_str)) {
            return true;
        }
    }

    if let Some(file_name) = candidate.file_name().and_then(|n| n.to_str()) {
        if let Ok(glob) = globset::Glob::new(&pattern_str) {
            if glob.compile_matcher().is_match(Path::new(file_name)) {
                return true;
            }
        }
    }
    false
}

pub fn normalize_event_name(raw: &str) -> &'static str {
    let cleaned: String = raw.chars().filter(|c| c.is_alphanumeric()).collect();
    let lower = cleaned.to_lowercase();
    match lower.as_str() {
        "beforewrite" => "beforeWrite",
        "afterwrite" => "afterWrite",
        "beforeprocessstart" => "beforeProcessStart",
        "afterprocessexit" => "afterProcessExit",
        "beforetest" => "beforeTest",
        "aftertest" => "afterTest",
        "beforecommit" => "beforeCommit",
        "aftercommit" => "afterCommit",
        "beforecompletion" => "beforeCompletion",
        "beforetool" | "pretool" => "beforeTool",
        "aftertool" | "posttool" => "afterTool",
        "posttoolfailure" => "postToolFailure",
        "posttoolbatch" => "postToolBatch",
        "sessionstart" => "sessionStart",
        "sessionend" => "sessionEnd",
        "stop" => "stop",
        "userpromptsubmit" => "userPromptSubmit",
        "permissionrequest" => "permissionRequest",
        "subagentstart" => "subagentStart",
        "subagentstop" => "subagentStop",
        "taskcreated" => "taskCreated",
        "taskcompleted" => "taskCompleted",
        "precompact" => "preCompact",
        "postcompact" => "postCompact",
        "premodelswitch" => "preModelSwitch",
        "postmodelswitch" => "postModelSwitch",
        _ => "unknown",
    }
}

pub fn rule_event_for(event: &davinci_agent::RuntimeEvent) -> Option<&'static str> {
    match event {
        davinci_agent::RuntimeEvent::SessionStarted { .. } => Some("sessionStart"),
        davinci_agent::RuntimeEvent::SessionEnded { .. } => Some("sessionEnd"),
        davinci_agent::RuntimeEvent::UserPromptSubmitted => Some("userPromptSubmit"),
        davinci_agent::RuntimeEvent::PreToolUse { .. } => Some("beforeTool"),
        davinci_agent::RuntimeEvent::PostToolUse {
            is_error: false, ..
        } => Some("afterTool"),
        davinci_agent::RuntimeEvent::PostToolUse { is_error: true, .. } => Some("postToolFailure"),
        davinci_agent::RuntimeEvent::PostToolBatch { .. } => Some("postToolBatch"),
        davinci_agent::RuntimeEvent::BeforeWrite { .. } => Some("beforeWrite"),
        davinci_agent::RuntimeEvent::AfterWrite { .. } => Some("afterWrite"),
        davinci_agent::RuntimeEvent::BeforeProcessStart { .. } => Some("beforeProcessStart"),
        davinci_agent::RuntimeEvent::AfterProcessExit { .. } => Some("afterProcessExit"),
        davinci_agent::RuntimeEvent::BeforeTest { .. } => Some("beforeTest"),
        davinci_agent::RuntimeEvent::AfterTest { .. } => Some("afterTest"),
        davinci_agent::RuntimeEvent::BeforeCommit { .. } => Some("beforeCommit"),
        davinci_agent::RuntimeEvent::AfterCommit { .. } => Some("afterCommit"),
        davinci_agent::RuntimeEvent::BeforeCompletion { .. } => Some("beforeCompletion"),
        davinci_agent::RuntimeEvent::TaskCompletionRequested { .. } => Some("beforeCompletion"),
        davinci_agent::RuntimeEvent::TaskCompleted { .. } => Some("taskCompleted"),
        davinci_agent::RuntimeEvent::PreCompact { .. } => Some("preCompact"),
        davinci_agent::RuntimeEvent::PostCompact { .. } => Some("postCompact"),
        davinci_agent::RuntimeEvent::PreModelSwitch { .. } => Some("preModelSwitch"),
        davinci_agent::RuntimeEvent::PostModelSwitch { .. } => Some("postModelSwitch"),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

    #[serde(default, alias = "policies")]
    pub rules: Vec<HookPolicyRule>,

    #[serde(skip)]
    pub project_path: Option<PathBuf>,
    #[serde(skip)]
    pub content_hash: Option<String>,
    #[serde(skip)]
    pub project_trusted: bool,
}

impl HooksFile {
    pub fn validate_trust_and_integrity(&self, cwd: &Path, agent_dir: &Path) -> Result<(), String> {
        let Some(project_path) = &self.project_path else {
            return Ok(());
        };

        if !self.project_trusted {
            return Err(format!(
                "untrusted project hook execution blocked for {}",
                project_path.display()
            ));
        }

        let store = crate::trust::ProjectTrustStore::open(agent_dir);
        if let Some(decision) = store.get(cwd) {
            if !decision {
                return Err(format!(
                    "project trust has been revoked for {}",
                    project_path.display()
                ));
            }
        }

        let Ok(current_bytes) = std::fs::read(project_path) else {
            return Err(format!(
                "project hooks file missing or unreadable: {}",
                project_path.display()
            ));
        };
        let current_hash = compute_sha256(&current_bytes);
        if let Some(expected_hash) = &self.content_hash {
            if &current_hash != expected_hash {
                return Err(format!(
                    "project hooks file was modified on disk; revalidation required for {}",
                    project_path.display()
                ));
            }
        }
        Ok(())
    }

    pub fn get_legacy_commands(&self, kind: &str) -> Vec<&Vec<String>> {
        match kind {
            "sessionStart" => self.session_start.iter().collect(),
            "userPromptSubmit" => self.user_prompt_submit.iter().collect(),
            "preTool" => self.pre_tool.iter().collect(),
            "permissionRequest" => self.permission_request.iter().collect(),
            "postTool" => self.post_tool.iter().collect(),
            "postToolFailure" => self
                .post_tool_failure
                .iter()
                .chain(self.post_tool.iter())
                .collect(),
            "postToolBatch" => self.post_tool_batch.iter().collect(),
            "subagentStart" => self.subagent_start.iter().collect(),
            "subagentStop" => self.subagent_stop.iter().collect(),
            "taskCreated" => self.task_created.iter().collect(),
            "taskCompleted" => self.task_completed.iter().collect(),
            "preCompact" => self.pre_compact.iter().collect(),
            "postCompact" => self.post_compact.iter().collect(),
            "preModelSwitch" => self.pre_model_switch.iter().collect(),
            "postModelSwitch" => self.post_model_switch.iter().collect(),
            "sessionEnd" => self.session_end.iter().chain(self.stop.iter()).collect(),
            _ => Vec::new(),
        }
    }
}

pub fn load(agent_dir: &Path, cwd: &Path, trusted: bool) -> HooksFile {
    if let Ok(path) = std::env::var("PI_HOOKS_CONFIG") {
        let mut file = load_path(Path::new(&path));
        file.project_trusted = true;
        return file;
    }
    let mut file = load_path(&agent_dir.join("hooks.json"));
    file.project_trusted = trusted;
    if trusted {
        let project_path = crate::project_config::resolve(cwd, "hooks.json");
        if let Some(p) = project_path {
            if let Ok(bytes) = std::fs::read(&p) {
                file.content_hash = Some(compute_sha256(&bytes));
                file.project_path = Some(p.clone());
                let project = load_path(&p);
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
                file.rules.extend(project.rules);
            }
        }
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
    match run_supervised_hook(
        argv, kind, tool, None, args, result, envelope, None, None, 3,
    ) {
        Ok(()) => None,
        Err(err) => Some(err),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_rule(
    rule: &HookPolicyRule,
    kind: &str,
    tool: &str,
    path: Option<&Path>,
    args: &Value,
    result: Option<&str>,
    envelope: Option<&davinci_agent::RuntimeEventEnvelope>,
    timeout_ms: Option<u64>,
    cwd: Option<&Path>,
    max_depth: usize,
) -> Result<(), String> {
    run_supervised_hook(
        &rule.action,
        kind,
        tool,
        path,
        args,
        result,
        envelope,
        timeout_ms,
        cwd,
        max_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_supervised_hook(
    argv: &[String],
    kind: &str,
    tool: &str,
    path: Option<&Path>,
    args: &Value,
    result: Option<&str>,
    envelope: Option<&davinci_agent::RuntimeEventEnvelope>,
    timeout_ms: Option<u64>,
    cwd: Option<&Path>,
    max_depth: usize,
) -> Result<(), String> {
    let program = argv
        .first()
        .ok_or_else(|| "empty hook command".to_string())?;

    if std::env::var("PI_HOOKS_DRY_RUN").is_ok() {
        return Ok(());
    }

    let _depth_guard = HookDepthGuard::enter(max_depth)?;

    GLOBAL_HOOK_TELEMETRY
        .executed
        .fetch_add(1, Ordering::Relaxed);

    let mut payload = serde_json::json!({
        "kind": kind,
        "tool": tool,
        "path": path.map(|p| p.to_string_lossy()),
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
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }

    cmd.env_clear();
    for var in [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    cmd.env("PI_HOOK_KIND", kind);
    cmd.env("PI_HOOK_TOOL", tool);
    cmd.env("PI_HOOK_EVENT", kind);
    let current_depth = HOOK_DEPTH.with(|d| d.get());
    cmd.env("DAVINCI_HOOK_DEPTH", current_depth.to_string());

    if let Some(c) = cwd {
        cmd.current_dir(c);
    }

    let timeout = timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(HOOK_TIMEOUT);
    let output = davinci_sys::process::run_bounded(
        cmd,
        Some(payload_str.into_bytes()),
        davinci_sys::process::RunLimits {
            timeout,
            output_cap: MAX_HOOK_STREAM_BYTES,
        },
        &|| false,
    )
    .map_err(|err| {
        GLOBAL_HOOK_TELEMETRY
            .blocked
            .fetch_add(1, Ordering::Relaxed);
        format!("hook `{program}` failed: {err}")
    })?;

    if output.timed_out {
        GLOBAL_HOOK_TELEMETRY
            .timed_out
            .fetch_add(1, Ordering::Relaxed);
        GLOBAL_HOOK_TELEMETRY
            .blocked
            .fetch_add(1, Ordering::Relaxed);
        return Err(format!(
            "hook `{program}` timed out after {}s",
            timeout.as_secs()
        ));
    }

    if output.status.is_some_and(|status| status.success()) {
        return Ok(());
    }

    GLOBAL_HOOK_TELEMETRY
        .blocked
        .fetch_add(1, Ordering::Relaxed);
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stdout).into_owned();
    }
    Err(format!("hook `{program}` blocked {tool}: {}", text.trim()))
}

pub fn status_report(cwd: &Path) -> Value {
    let hooks = load(&davinci_session::default_agent_dir(), cwd, true);
    serde_json::json!({
        "trusted": hooks.project_trusted,
        "projectPath": hooks.project_path.as_ref().map(|p| p.to_string_lossy()),
        "contentHash": hooks.content_hash,
        "rulesCount": hooks.rules.len(),
        "legacyHooksCount": {
            "preTool": hooks.pre_tool.len(),
            "postTool": hooks.post_tool.len(),
            "sessionStart": hooks.session_start.len(),
            "sessionEnd": hooks.session_end.len(),
            "stop": hooks.stop.len(),
        },
        "telemetry": {
            "executed": GLOBAL_HOOK_TELEMETRY.executed.load(Ordering::Relaxed),
            "blocked": GLOBAL_HOOK_TELEMETRY.blocked.load(Ordering::Relaxed),
            "warned": GLOBAL_HOOK_TELEMETRY.warned.load(Ordering::Relaxed),
            "ignored": GLOBAL_HOOK_TELEMETRY.ignored.load(Ordering::Relaxed),
            "timedOut": GLOBAL_HOOK_TELEMETRY.timed_out.load(Ordering::Relaxed),
            "recursionStopped": GLOBAL_HOOK_TELEMETRY.recursion_stopped.load(Ordering::Relaxed),
        }
    })
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
    fn a_hook_that_ignores_large_stdin_still_times_out() {
        let command = if cfg!(windows) {
            vec![
                "powershell".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ]
        } else {
            vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()]
        };
        let payload = serde_json::json!({"output": "x".repeat(4 * 1024 * 1024)});
        let started = std::time::Instant::now();
        let result = run_supervised_hook(
            &command,
            "preTool",
            "read",
            None,
            &payload,
            None,
            None,
            Some(250),
            None,
            3,
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
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
