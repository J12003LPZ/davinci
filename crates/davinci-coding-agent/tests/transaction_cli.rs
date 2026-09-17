//! Offline checks through the packaged CLI, its policy, and Graph worker adapter.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run(root: &Path, tool: &str, args: Value, node: Option<&str>, enabled: bool) -> (bool, String) {
    run_with_identity(root, tool, args, node, enabled, None)
}

fn run_with_identity(
    root: &Path,
    tool: &str,
    args: Value,
    node: Option<&str>,
    enabled: bool,
    identity: Option<&str>,
) -> (bool, String) {
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        json!({"editingTransactions":{"enabled":enabled},"processManager":{"enabled":false}})
            .to_string(),
    )
    .unwrap();
    let stdout = root.join("stdout.jsonl");
    let stderr = root.join("stderr.txt");
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
            "--permission-mode",
            "always-approve",
            "--tools",
            "write,patch_preview,patch_apply,patch_status,patch_rollback,exec_command",
            "--mode",
            "json",
            "--print",
            "transaction fixture",
        ])
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
            json!({"name":tool,"arguments":args}).to_string(),
        )
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    if let Some(node) = node {
        command
            .env("PI_GRAPH_ROLE", "writer")
            .env("PI_GRAPH_EXPECT", "patch-report")
            .env("PI_GRAPH_NODE_ID", node)
            .env("PI_GRAPH_ARTIFACT_PATH", root.join("artifact.json"))
            .env("PI_GRAPH_EFFECT_REPORT", root.join("effects.jsonl"));
    }
    if let Some(identity) = identity {
        command.env("DAVINCI_AGENT_ID", identity);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("transaction CLI fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = fs::read_to_string(stdout)
        .unwrap()
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .is_some_and(|event| event["type"] == "tool_execution_end")
        })
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    let errors: String = fs::read_to_string(stderr)
        .unwrap()
        .chars()
        .take(4096)
        .collect();
    (status.success(), format!("{output}\n{errors}"))
}

#[test]
fn ordinary_cli_foreground_command_works_with_process_manager_disabled() {
    let root = tempfile::tempdir().unwrap();
    let command = if cfg!(windows) {
        "[Console]::Out.Write('FOREGROUND_OK'); exit 7"
    } else {
        "printf FOREGROUND_OK; exit 7"
    };
    let (ok, output) = run(
        root.path(),
        "exec_command",
        json!({"command":command}),
        None,
        false,
    );
    assert!(ok, "{output}");
    let event: Value = serde_json::from_str(output.lines().next().unwrap()).unwrap();
    assert!(output.contains("FOREGROUND_OK"), "{output}");
    assert_eq!(event["isError"], true, "{event}");
}

#[test]
fn graph_cli_rejects_invalid_parent_agent_identity_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (ok, output) = run_with_identity(
        root.path(),
        "write",
        json!({"path":"a.txt","content":"forbidden"}),
        Some("writer-1"),
        true,
        Some("invalid-identity"),
    );
    assert!(!ok, "{output}");
    assert!(
        output.contains("invalid Graph worker agent identity"),
        "{output}"
    );
    assert!(!root.path().join("a.txt").exists());
    assert!(!root.path().join(".davinci-transactions").exists());
}

#[test]
fn graph_cli_write_has_host_node_provenance_and_effect_handoff() {
    let root = tempfile::tempdir().unwrap();
    let (ok, output) = run(
        root.path(),
        "write",
        json!({"path":"a.txt","content":"created"}),
        Some("writer-17"),
        true,
    );
    assert!(ok, "{output}");
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"created");
    let records: Vec<Value> = fs::read_dir(root.path().join(".davinci-transactions"))
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().is_some_and(|ext| ext == "json"))
                .then(|| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
        })
        .collect();
    assert_eq!(records.len(), 1, "{output}");
    assert_eq!(records[0]["summary"]["owner"]["graph_node"], "writer-17");
    assert_eq!(records[0]["summary"]["state"], "applied");
    let effects = fs::read_to_string(root.path().join("effects.jsonl")).unwrap();
    assert_eq!(effects.lines().count(), 1);
}

#[test]
fn graph_cli_disabled_tools_and_invalid_context_cannot_mutate() {
    let root = tempfile::tempdir().unwrap();
    let (ok, output) = run(
        root.path(),
        "patch_preview",
        json!({"input":"*** Begin Patch\n*** Add File: a.txt\n+created\n*** End Patch"}),
        Some("writer-17"),
        false,
    );
    assert!(ok, "{output}");
    assert!(
        !root.path().join(".davinci-transactions").exists(),
        "{output}"
    );
    assert!(!root.path().join("a.txt").exists());
    assert!(output.contains("Unknown tool: patch_preview"), "{output}");
    let (ok, output) = run(
        root.path(),
        "write",
        json!({"path":"a.txt","content":"forbidden"}),
        Some(""),
        true,
    );
    assert!(!ok, "{output}");
    assert!(output.contains("invalid Graph worker context"), "{output}");
    assert!(!root.path().join("a.txt").exists());
}

#[test]
fn ordinary_cli_write_keeps_transaction_safety_when_explicit_tools_are_disabled() {
    let root = tempfile::tempdir().unwrap();
    let (ok, output) = run(
        root.path(),
        "write",
        json!({"path":"a.txt","content":"ordinary"}),
        None,
        false,
    );
    assert!(ok, "{output}");
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"ordinary");
    let paths: Vec<_> = fs::read_dir(root.path().join(".davinci-transactions"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert_eq!(paths.len(), 1, "{output}");
    let record: Value = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
    assert!(record["summary"]["owner"]["graph_node"].is_null());
    assert_eq!(record["summary"]["state"], "applied");
}
