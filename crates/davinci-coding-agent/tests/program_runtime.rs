//! Packaged CLI acceptance for the program's normal and Graph execution paths.
//!
//! These checks deliberately use read-only native tools so Windows filesystem
//! ACLs cannot turn a runtime smoke test into a transaction-journal test.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run(root: &Path, graph: bool) -> (bool, Vec<Value>, String) {
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        json!({
            "cache": {"enabled": false},
            "changeImpact": {"enabled": true},
            "verificationPlanner": {"enabled": true},
            "workspaceSnapshots": {"enabled": true, "maxFiles": 8},
            "processManager": {"enabled": false}
        })
        .to_string(),
    )
    .unwrap();

    let calls = json!([
        {"name":"impact_analyze","arguments":{"files":["src/value.ts"]}},
        {"name":"verification_plan","arguments":{"files":["src/value.ts"]}},
        {"name":"workspace_checkpoint","arguments":{"path":"src/value.ts","label":"runtime"}}
    ]);
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
            "impact_analyze,verification_plan,workspace_checkpoint,workspace_diff,workspace_restore",
            "--mode",
            "json",
            "--print",
            "program runtime fixture",
        ])
        .env("HOME", &config)
        .env("USERPROFILE", &config)
        .env("PI_CODING_AGENT_DIR", &config)
        .env("DAVINCI_CODING_AGENT_DIR", &config)
        .env("PI_OFFLINE", "1")
        .env("DAVINCI_OFFLINE", "1")
        .env("PI_DISABLE_NETWORK", "1")
        .env("PI_HOOKS_DRY_RUN", "1")
        .env("PI_OFFLINE_TOOL_CALL", calls.to_string())
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    if graph {
        command
            .env("PI_GRAPH_ROLE", "writer")
            .env("PI_GRAPH_EXPECT", "patch-report")
            .env("PI_GRAPH_NODE_ID", "writer-runtime")
            .env("PI_GRAPH_ARTIFACT_PATH", root.join("artifact.json"))
            .env("PI_GRAPH_EFFECT_REPORT", root.join("effects.jsonl"))
            .env(
                "PI_GRAPH_EXTRA_TOOLS",
                "impact_analyze,verification_plan,workspace_checkpoint,workspace_diff,workspace_restore",
            )
            .env(
                "PI_GRAPH_AUTHORIZED_TOOLS",
                "impact_analyze,verification_plan,workspace_checkpoint,workspace_diff,workspace_restore",
            );
    }

    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("packaged program runtime fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr_text = fs::read_to_string(stderr).unwrap();
    let events = fs::read_to_string(stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "tool_execution_end")
        .collect();
    (status.success(), events, stderr_text)
}

fn assert_runtime(graph: bool) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "src/value.ts", "export const value = 1;");
    let (ok, events, stderr) = run(root.path(), graph);
    assert!(ok, "{stderr}");
    assert_eq!(events.len(), 3, "events={events:?}\nstderr={stderr}");
    assert!(
        events.iter().all(|event| event["isError"] == false),
        "events={events:?}\nstderr={stderr}"
    );
    assert_eq!(events[0]["toolName"], "impact_analyze");
    assert_eq!(events[1]["toolName"], "verification_plan");
    assert_eq!(events[2]["toolName"], "workspace_checkpoint");
    let planner = events[1]["result"]
        .as_str()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .or_else(|| events[1].get("details").cloned())
        .unwrap_or_else(|| events[1].clone());
    assert_eq!(
        planner["enabled"], true,
        "events={events:?}\nstderr={stderr}"
    );
}

#[test]
fn packaged_cli_runs_program_tools_in_normal_mode() {
    assert_runtime(false);
}

#[test]
fn packaged_cli_runs_program_tools_in_graph_writer_mode() {
    assert_runtime(true);
}
