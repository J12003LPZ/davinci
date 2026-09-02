//! User hooks: commands run around tools and at stop.
//!
//! No TypeScript counterpart. Phase 6 spec:
//! `docs/superpowers/specs/2026-09-01-hooks-and-observability-design.md`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

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
    serde_json::from_str(&body).unwrap_or_default()
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

pub fn append_event(session_path: Option<&PathBuf>, kind: &str, tool: &str) {
    let Some(path) = session_path else {
        return;
    };
    let file = path.with_extension("events.jsonl");
    let row = serde_json::json!({
        "ts": pi_session::now_ms(),
        "kind": kind,
        "tool": tool,
    });
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
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
        .env("PI_HOOK_KIND", kind)
        .env("PI_HOOK_TOOL", tool)
        .env("PI_HOOK_ARGS", args.to_string());
    if let Some(result) = result {
        cmd.env("PI_HOOK_RESULT", result);
    }
    match cmd.output() {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
            if text.trim().is_empty() {
                text = String::from_utf8_lossy(&output.stdout).into_owned();
            }
            Some(format!("hook `{program}` blocked {tool}: {}", text.trim()))
        }
        Err(err) => Some(format!("hook `{program}` failed: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
}
