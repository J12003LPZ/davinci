//! Offline checks for `--approval-policy abort|deny-continue` and
//! `--fail-on-denied`, driven through the packaged binary so the exit codes
//! and the stdout/stderr contract are what gets tested. `PI_OFFLINE_TOOL_CALL`
//! makes the first reply of every prompt a `write` that needs approval in
//! `manual` mode; the reply after the tool result is the offline text stub.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn davinci(root: &Path, json_mode: bool, extra: &[&str], prompts: &[&str]) -> Output {
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        json!({"processManager":{"enabled":false}}).to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(root)
        .args([
            "--offline",
            "--no-session",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-mcp",
            "--permission-mode",
            "manual",
        ])
        .args(extra);
    if json_mode {
        command.args(["--mode", "json"]);
    }
    command
        .arg("--print")
        .args(prompts)
        .env("HOME", &config)
        .env("USERPROFILE", &config)
        .env("PI_CODING_AGENT_DIR", &config)
        .env("DAVINCI_CODING_AGENT_DIR", &config)
        .env("PI_OFFLINE", "1")
        .env("DAVINCI_OFFLINE", "1")
        .env("PI_DISABLE_NETWORK", "1")
        .env("PI_HOOKS_DRY_RUN", "1")
        .env(
            "PI_OFFLINE_TOOL_CALL",
            json!({"name": "write", "arguments": {
                "path": "must-not-exist.txt",
                "content": "fixture",
            }})
            .to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.output().unwrap()
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn json_rows(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("every stdout line must be JSON"))
        .collect()
}

fn rows_of<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> {
    rows.iter().filter(|row| row["type"] == kind).collect()
}

#[test]
fn default_policy_still_stops_at_the_first_approval() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(root.path(), true, &[], &["first prompt", "second prompt"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let rows = json_rows(&output);
    assert!(rows_of(&rows, "denied_actions").is_empty());
    let required = rows_of(&rows, "approval_required");
    assert_eq!(required.len(), 1, "{}", describe(&output));
    assert_eq!(rows.last().unwrap()["type"], "approval_required");
    assert!(!root.path().join("must-not-exist.txt").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("second prompt"));
}

#[test]
fn deny_continue_finishes_and_reports_the_denied_action_in_json() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(
        root.path(),
        true,
        &["--approval-policy", "deny-continue"],
        &["write it"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let rows = json_rows(&output);
    assert!(rows_of(&rows, "approval_required").is_empty());
    let tool_end = rows
        .iter()
        .find(|row| row["type"] == "tool_execution_end")
        .expect("the denied call still ends");
    assert_eq!(tool_end["isError"], true);
    assert!(tool_end["result"]
        .to_string()
        .contains("this non-interactive run cannot ask"));
    let denied = rows_of(&rows, "denied_actions");
    assert_eq!(denied.len(), 1, "{}", describe(&output));
    assert_eq!(rows.last().unwrap()["type"], "denied_actions");
    let actions = denied[0]["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["action"], "write");
    assert_eq!(actions[0]["permission_mode"], "ask");
    assert!(actions[0]["target"]
        .as_str()
        .unwrap()
        .ends_with("must-not-exist.txt"));
    assert!(!root.path().join("must-not-exist.txt").exists());
}

#[test]
fn deny_continue_text_mode_prints_the_reply_and_one_summary_line() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(
        root.path(),
        false,
        &["--approval-policy=deny-continue"],
        &["write it"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(offline) received"),
        "{}",
        describe(&output)
    );
    assert!(!stdout.contains("approval_required"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary: Vec<_> = stderr
        .lines()
        .filter(|line| line.starts_with("Denied "))
        .collect();
    assert_eq!(summary.len(), 1, "{}", describe(&output));
    assert!(summary[0].starts_with("Denied 1 action needing approval"));
    assert!(summary[0].contains("must-not-exist.txt"));
}

#[test]
fn fail_on_denied_turns_a_finished_run_into_exit_3() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(
        root.path(),
        true,
        &["--approval-policy", "deny-continue", "--fail-on-denied"],
        &["write it"],
    );
    assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
    assert_eq!(rows_of(&json_rows(&output), "denied_actions").len(), 1);

    // Nothing denied (the later mode wins and allows the write): the flag
    // leaves a clean run at 0 and the json stream without denied_actions.
    let clean = tempfile::tempdir().unwrap();
    let output = davinci(
        clean.path(),
        true,
        &[
            "--approval-policy",
            "deny-continue",
            "--fail-on-denied",
            "--permission-mode",
            "always-approve",
        ],
        &["write it"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(rows_of(&json_rows(&output), "denied_actions").is_empty());
    assert!(clean.path().join("must-not-exist.txt").exists());
}

#[test]
fn deny_continue_aborts_when_one_action_is_denied_three_times() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(
        root.path(),
        true,
        &["--approval-policy", "deny-continue", "--fail-on-denied"],
        &["first", "second", "third", "fourth"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let rows = json_rows(&output);
    let required = rows_of(&rows, "approval_required");
    assert_eq!(required.len(), 1, "{}", describe(&output));
    assert_eq!(rows.last().unwrap()["type"], "approval_required");
    assert_eq!(required[0]["action"], "write");
    let denied = rows_of(&rows, "denied_actions");
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0]["actions"].as_array().unwrap().len(), 2);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("fourth"));
    assert!(!root.path().join("must-not-exist.txt").exists());
}
