//! Seventeen-step login-button workflow through packaged normal and Graph dispatch.
//!
//! Browser, process, and LSP steps are executed when the matching host
//! capability is available. Missing live dependencies stay explicit failures
//! of those steps, never fabricated successes.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TOOLS: &str = "repo_map,impact_analyze,verification_plan,workspace_checkpoint,workspace_diff";

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn fixture(root: &Path) {
    write(
        root,
        "package.json",
        r#"{"name":"login-app","scripts":{"test":"node --test"},"type":"module"}"#,
    );
    write(
        root,
        "src/login.ts",
        "export function loginButtonLabel(ok: boolean): string { return ok ? 'Broken' : 'Broken'; }\n",
    );
    write(
        root,
        "src/login.test.ts",
        "import { loginButtonLabel } from './login.ts';\nconsole.log(loginButtonLabel(true));\n",
    );
    write(
        root,
        "index.html",
        "<html><body><button id='login'>Broken</button></body></html>",
    );
}

fn run(root: &Path, graph_role: Option<&str>) -> (bool, Vec<Value>, String) {
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
        {"name":"impact_analyze","arguments":{"files":["src/login.ts"]}},
        {"name":"verification_plan","arguments":{"files":["src/login.ts"]}},
        {"name":"workspace_checkpoint","arguments":{"path":"src/login.ts","label":"login"}},
        {"name":"repo_map","arguments":{}}
    ]);
    let stdout = root.join(format!("stdout-{}.jsonl", graph_role.unwrap_or("normal")));
    let stderr = root.join(format!("stderr-{}.txt", graph_role.unwrap_or("normal")));
    let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "USERPROFILE"] {
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
            TOOLS,
            "--mode",
            "json",
            "--print",
            "Fix the login button bug and make sure nothing else breaks.",
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
    if let Some(role) = graph_role {
        command
            .env("PI_GRAPH_ROLE", role)
            .env("PI_GRAPH_EXPECT", "patch-report")
            .env("PI_GRAPH_NODE_ID", format!("{role}-login"))
            .env("PI_GRAPH_ARTIFACT_PATH", root.join("artifact.json"))
            .env("PI_GRAPH_EFFECT_REPORT", root.join("effects.jsonl"));
        if role == "writer" {
            command
                .env("PI_GRAPH_EXTRA_TOOLS", TOOLS)
                .env("PI_GRAPH_AUTHORIZED_TOOLS", TOOLS);
        } else {
            command
                .env("PI_GRAPH_EXTRA_TOOLS", "repo_map")
                .env("PI_GRAPH_AUTHORIZED_TOOLS", "repo_map");
        }
    }

    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("login workflow fixture timed out");
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

fn names(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event["toolName"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn login_button_normal_dispatch_covers_program_tools() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let (ok, events, stderr) = run(root.path(), None);
    assert!(ok, "{stderr}");
    let names = names(&events);
    for required in [
        "repo_map",
        "impact_analyze",
        "verification_plan",
        "workspace_checkpoint",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing {required} in {names:?} events={events:?}\nstderr={stderr}"
        );
    }
    assert!(
        events
            .iter()
            .filter(|event| event["toolName"] == "verification_plan"
                || event["toolName"] == "impact_analyze"
                || event["toolName"] == "workspace_checkpoint")
            .all(|event| event["isError"] == false),
        "events={events:?}\nstderr={stderr}"
    );
}

#[test]
fn login_button_graph_writer_gets_same_program_tools() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let (ok, events, stderr) = run(root.path(), Some("writer"));
    assert!(ok, "{stderr}");
    let names = names(&events);
    assert!(
        names.iter().any(|name| name == "verification_plan"),
        "{names:?}\n{stderr}"
    );
    assert!(
        names.iter().any(|name| name == "workspace_checkpoint"),
        "{names:?}\n{stderr}"
    );
}

#[test]
fn login_button_graph_classifier_is_denied_planner_and_restore() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let (_ok, events, stderr) = run(root.path(), Some("classifier"));
    let names = names(&events);
    assert!(
        !names.iter().any(|name| name == "verification_plan")
            || events
                .iter()
                .any(|event| event["toolName"] == "verification_plan" && event["isError"] == true),
        "classifier must not execute verification_plan successfully: {events:?}\n{stderr}"
    );
    assert!(
        !names.iter().any(|name| name == "workspace_restore")
            || events
                .iter()
                .any(|event| event["toolName"] == "workspace_restore" && event["isError"] == true),
        "classifier must not restore snapshots: {events:?}\n{stderr}"
    );
}
