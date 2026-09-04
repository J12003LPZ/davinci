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
    pub pre_tool: Vec<Vec<String>>,
    #[serde(default)]
    pub post_tool: Vec<Vec<String>>,
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
        file.pre_tool.extend(project.pre_tool);
        file.post_tool.extend(project.post_tool);
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

fn run_one(
    argv: &[String],
    kind: &str,
    tool: &str,
    args: &Value,
    result: Option<&str>,
) -> Option<String> {
    let program = argv.first()?;
    if std::env::var("PI_HOOKS_DRY_RUN").is_ok() {
        return None;
    }
    let payload = serde_json::json!({
        "kind": kind,
        "tool": tool,
        "args": args,
        "result": result,
    })
    .to_string();
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
        // A hook that never reads stdin closes it; the write fails, which
        // is fine.
        let _ = stdin.write_all(payload.as_bytes());
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
}
