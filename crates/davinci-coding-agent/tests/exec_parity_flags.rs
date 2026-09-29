//! Offline checks for the Codex `exec` parity flags `-o/--output-last-message`
//! and `-C/--cd`, driven through the packaged binary so the ordering inside
//! `run` (cwd change before settings, trust and the session directory) is what
//! gets tested.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

/// The offline stub reply for a prompt of `chars` characters.
fn offline_reply(chars: usize) -> String {
    format!("(offline) received {chars} characters")
}

fn davinci(launch_dir: &Path, config: &Path) -> Command {
    fs::create_dir_all(config).unwrap();
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
        .current_dir(launch_dir)
        .args([
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-mcp",
        ])
        .env("HOME", config)
        .env("USERPROFILE", config)
        .env("PI_CODING_AGENT_DIR", config)
        .env("DAVINCI_CODING_AGENT_DIR", config)
        .env("PI_OFFLINE", "1")
        .env("DAVINCI_OFFLINE", "1")
        .env("PI_DISABLE_NETWORK", "1")
        .env("PI_HOOKS_DRY_RUN", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(command: &mut Command) -> Output {
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

#[test]
fn output_last_message_writes_the_final_reply_in_text_and_json_modes() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");

    let text_file = root.path().join("text.txt");
    let output = run(davinci(root.path(), &config).args([
        "--no-session",
        "-o",
        text_file.to_str().unwrap(),
        "-p",
        "hello",
    ]));
    assert!(output.status.success(), "{}", describe(&output));
    assert_eq!(fs::read_to_string(&text_file).unwrap(), offline_reply(5));
    // stdout still carries the reply.
    assert!(String::from_utf8_lossy(&output.stdout).contains(&offline_reply(5)));

    let json_file = root.path().join("json.txt");
    let output = run(davinci(root.path(), &config).args([
        "--no-session",
        "--mode",
        "json",
        "--output-last-message",
        json_file.to_str().unwrap(),
        "-p",
        "hello there",
    ]));
    assert!(output.status.success(), "{}", describe(&output));
    assert_eq!(fs::read_to_string(&json_file).unwrap(), offline_reply(11));
    // json stdout stays a pure event stream.
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        serde_json::from_str::<Value>(line).unwrap_or_else(|_| panic!("not json: {line}"));
    }
}

#[test]
fn output_last_message_is_written_when_the_run_is_blocked() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let file = root.path().join("last.txt");
    fs::write(&file, "stale reply from an earlier run").unwrap();
    let command = if cfg!(windows) {
        "Remove-Item -Recurse nothing-here"
    } else {
        "rm -rf nothing-here"
    };
    let output = run(davinci(root.path(), &config)
        .args([
            "--no-session",
            "--permission-mode",
            "manual",
            "-o",
            file.to_str().unwrap(),
            "-p",
            "clean up",
        ])
        .env(
            "PI_OFFLINE_TOOL_CALL",
            json!({"name": if cfg!(windows) { "powershell" } else { "bash" },
                   "arguments": {"command": command}})
            .to_string(),
        ));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("approval_required"),
        "{}",
        describe(&output)
    );
    // The blocked run produced no reply text: the file is rewritten, not left stale.
    assert_eq!(fs::read_to_string(&file).unwrap(), "");
}

#[test]
fn output_last_message_keeps_the_runs_own_failure_code() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");

    // The shutdown fixture ends the run with 143 before any turn: the file is
    // still written, empty.
    let file = root.path().join("last.txt");
    let output = run(davinci(root.path(), &config)
        .args(["--no-session", "-o", file.to_str().unwrap(), "-p", "hi"])
        .env("PI_SHUTDOWN_SIGNAL", "TERM"));
    assert_eq!(output.status.code(), Some(143), "{}", describe(&output));
    assert_eq!(fs::read_to_string(&file).unwrap(), "");

    // A failed write does not replace 143 with 1.
    let unwritable = root.path().join("missing-dir").join("last.txt");
    let output = run(davinci(root.path(), &config)
        .args([
            "--no-session",
            "-o",
            unwritable.to_str().unwrap(),
            "-p",
            "hi",
        ])
        .env("PI_SHUTDOWN_SIGNAL", "TERM"));
    assert_eq!(output.status.code(), Some(143), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not write"));
}

#[test]
fn output_last_message_write_failure_fails_a_successful_run() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let unwritable = root.path().join("missing-dir").join("last.txt");
    let output = run(davinci(root.path(), &config).args([
        "--no-session",
        "-o",
        unwritable.to_str().unwrap(),
        "-p",
        "hello",
    ]));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: could not write --output-last-message file"),
        "{}",
        describe(&output)
    );
    // The reply itself was still delivered.
    assert!(String::from_utf8_lossy(&output.stdout).contains(&offline_reply(5)));
    assert!(!unwritable.exists());
}

#[test]
fn output_last_message_is_rejected_outside_print_modes() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let file = root.path().join("last.txt");

    let output = run(davinci(root.path(), &config).args([
        "--no-session",
        "--mode",
        "rpc",
        "-o",
        file.to_str().unwrap(),
    ]));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--output-last-message is not supported with --mode rpc"));

    let output = run(davinci(root.path(), &config)
        .args(["--no-session", "-o", file.to_str().unwrap()])
        .env("PI_FORCE_INTERACTIVE", "1"));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--output-last-message needs a non-interactive run"));
    assert!(!file.exists());
}

#[test]
fn cd_runs_the_agent_and_its_session_in_the_named_directory() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let work = root.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let output = run(davinci(root.path(), &config)
        .args([
            "--permission-mode",
            "always-approve",
            "--tools",
            "write",
            "--mode",
            "json",
            "-C",
            "work",
            // Relative `-o` resolves against the launch directory, as in Codex.
            "-o",
            "last.txt",
            "-p",
            "write the marker",
        ])
        .env(
            "PI_OFFLINE_TOOL_CALL",
            json!({"name": "write", "arguments": {"path": "marker.txt", "content": "here"}})
                .to_string(),
        ));
    assert!(output.status.success(), "{}", describe(&output));

    // The relative tool path landed in the --cd directory.
    assert_eq!(fs::read_to_string(work.join("marker.txt")).unwrap(), "here");
    assert!(!root.path().join("marker.txt").exists());
    assert!(root.path().join("last.txt").exists());
    assert!(!work.join("last.txt").exists());

    // The session header records the --cd directory as its cwd.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let header: Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(header["kind"], "header", "{header}");
    let cwd = header["cwd"].as_str().unwrap();
    assert!(!cwd.starts_with(r"\\?\"), "{cwd}");
    assert_eq!(Path::new(cwd).file_name().unwrap(), "work", "{cwd}");
    assert_eq!(
        fs::canonicalize(cwd).unwrap(),
        fs::canonicalize(&work).unwrap()
    );
    // ...and the session file lives under the directory encoded from it.
    let sessions = config.join("sessions");
    let encoded: Vec<String> = fs::read_dir(&sessions)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        encoded.iter().any(|name| name.ends_with("-work--")),
        "{encoded:?}"
    );
}

#[test]
fn cd_rejects_missing_and_non_directory_targets() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::write(root.path().join("file.txt"), "not a dir").unwrap();

    let output = run(davinci(root.path(), &config).args(["--cd", "no-such-dir", "-p", "hi"]));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Error: --cd: directory does not exist: no-such-dir"),
        "{}",
        describe(&output)
    );

    let output = run(davinci(root.path(), &config).args(["-C", "file.txt", "-p", "hi"]));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Error: --cd: not a directory: file.txt"),
        "{}",
        describe(&output)
    );

    let output = run(davinci(root.path(), &config).args(["-p", "hi", "-C"]));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--cd requires a directory"));
}

#[test]
fn add_dir_grants_only_the_requested_sibling_and_respects_denies() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let shared = root.path().join("shared");
    let other = root.path().join("other");
    let config = root.path().join("config");
    for dir in [&work, &shared, &other] {
        fs::create_dir(dir).unwrap();
    }
    let target = shared.join("marker.txt");
    let call = |path: &Path| {
        json!({"name":"write", "arguments":{"path":path, "content":"marker"}}).to_string()
    };
    let mut plain = davinci(&work, &config);
    let output = run(plain
        .args([
            "--no-session",
            "--permission-mode",
            "accept-edits",
            "--tools",
            "write",
            "--mode",
            "json",
            "-p",
            "write marker",
        ])
        .env("PI_OFFLINE_TOOL_CALL", call(&target)));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(!target.exists());

    let mut added = davinci(root.path(), &config);
    let output = run(added
        .args([
            "--no-session",
            "--permission-mode",
            "accept-edits",
            "--tools",
            "write",
            "--mode",
            "json",
            "-C",
            "work",
            "--add-dir",
            "../shared",
            "-p",
            "write marker",
        ])
        .env("PI_OFFLINE_TOOL_CALL", call(&target)));
    assert!(output.status.success(), "{}", describe(&output));
    assert!(target.exists(), "{}", describe(&output));
    assert_eq!(fs::read_to_string(&target).unwrap(), "marker");
    let relative = shared.join("relative.txt");
    let output = run(davinci(&work, &config)
        .args(["--no-session", "--permission-mode", "accept-edits", "--tools", "write,edit", "--mode", "json", "--add-dir", "../shared", "-p", "write and edit"])
        .env("PI_OFFLINE_TOOL_CALL", json!([
            {"name":"write","arguments":{"path":"../shared/relative.txt","content":"marker"}},
            {"name":"edit","arguments":{"path":"../shared/relative.txt","oldText":"marker","newText":"edited"}}
        ]).to_string()));
    assert!(output.status.success(), "{}", describe(&output));
    assert_eq!(
        fs::read_to_string(relative).unwrap(),
        "edited",
        "{}",
        describe(&output)
    );

    let outside = other.join("marker.txt");
    let mut unlisted = davinci(&work, &config);
    let output = run(unlisted
        .args([
            "--no-session",
            "--permission-mode",
            "accept-edits",
            "--tools",
            "write",
            "--mode",
            "json",
            "--add-dir",
            "../shared",
            "-p",
            "write marker",
        ])
        .env("PI_OFFLINE_TOOL_CALL", call(&outside)));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(!outside.exists());

    let denied = shared.join("denied.txt");
    let mut explicit = davinci(&work, &config);
    let subject = davinci_agent::strip_verbatim_prefix(&denied)
        .to_string_lossy()
        .replace('\\', "/");
    fs::write(config.join("settings.json"), json!({"processManager":{"enabled":false},"permissions":{"deny":[format!("write({subject})")]}}).to_string()).unwrap();
    let output = run(explicit
        .args([
            "--no-session",
            "--permission-mode",
            "accept-edits",
            "--tools",
            "write",
            "--mode",
            "json",
            "--add-dir",
            "../shared",
            "-p",
            "write marker",
        ])
        .env("PI_OFFLINE_TOOL_CALL", call(&denied)));
    assert!(!denied.exists(), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("deny rule"),
        "{}",
        describe(&output)
    );
}

#[test]
fn invalid_add_dir_fails_before_any_offline_tool_runs() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let marker = root.path().join("should-not-exist.txt");
    let output = run(davinci(root.path(), &config)
        .args(["--no-session", "--add-dir", "absent", "-p", "write"])
        .env(
            "PI_OFFLINE_TOOL_CALL",
            json!({"name":"write","arguments":{"path":marker,"content":"bad"}}).to_string(),
        ));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(!marker.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--add-dir"));
}
