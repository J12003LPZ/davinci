//! Offline validation of CLI modes and existing-session exports.
use davinci_coding_agent::args::{parse_args, Mode};
use davinci_session::{JsonlSession, SessionEntry};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn davinci(root: &Path) -> Command {
    let config = root.join("config");
    let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(root)
        .args(["--offline", "--no-extensions", "--no-skills", "--no-mcp"])
        .arg("--session-dir")
        .arg(root.join("sessions"))
        .env("HOME", &config)
        .env("USERPROFILE", &config)
        .env("PI_CODING_AGENT_DIR", &config)
        .env("DAVINCI_CODING_AGENT_DIR", &config)
        .env("PI_OFFLINE", "1")
        .env("PI_DISABLE_NETWORK", "1")
        .stdin(Stdio::null());
    command
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn session(root: &Path) -> JsonlSession {
    let cwd = davinci_agent::strip_verbatim_prefix(&root.canonicalize().unwrap());
    let mut session =
        JsonlSession::create(&root.join("sessions"), &cwd.to_string_lossy(), None).unwrap();
    session
        .append_entry(SessionEntry::message(
            "user",
            json!([{"type":"text","text":"export regression marker"}]),
        ))
        .unwrap();
    session
}

#[test]
fn export_missing_input_fails_without_creating_a_session_or_output() {
    for reference in ["missing-session.jsonl", "missing-session-id"] {
        let root = tempfile::tempdir().unwrap();
        let output = davinci(root.path())
            .args(["--export", reference, "out.html"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Session not found"), "{}", describe(&output));
        assert!(stderr.contains(reference), "{}", describe(&output));
        assert!(output.stdout.is_empty(), "{}", describe(&output));
        assert!(!root.path().join("out.html").exists());
        assert!(!root.path().join("sessions").exists());
        assert!(!root.path().join(reference).exists());
    }
}

#[test]
fn export_missing_input_never_falls_back_to_other_session_flags() {
    let root = tempfile::tempdir().unwrap();
    let source = session(root.path());
    let original = fs::read(&source.path).unwrap();
    let export = root.path().join("out.html");
    fs::write(&export, "keep existing export").unwrap();
    let source_path = source.path.to_str().unwrap();
    for flags in [
        vec!["--continue"],
        vec!["--session", source_path],
        vec!["--session-id", "must-not-be-created"],
        vec!["--fork", source_path],
    ] {
        let output = davinci(root.path())
            .args(flags)
            .args(["--export", "missing-session.jsonl", "out.html"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
        assert!(String::from_utf8_lossy(&output.stderr).contains("missing-session.jsonl"));
        assert_eq!(fs::read_to_string(&export).unwrap(), "keep existing export");
        assert_eq!(fs::read(&source.path).unwrap(), original);
        assert_eq!(
            davinci_session::discover_sessions(&root.path().join("sessions"), None)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn export_accepts_existing_paths_ids_and_unambiguous_prefixes() {
    let root = tempfile::tempdir().unwrap();
    let source = session(root.path());
    let original = fs::read(&source.path).unwrap();
    fs::copy(&source.path, root.path().join("source.jsonl")).unwrap();
    fs::create_dir_all(root.path().join("config")).unwrap();
    fs::copy(&source.path, root.path().join("config/source.jsonl")).unwrap();
    for reference in [
        source.path.to_str().unwrap(),
        "source.jsonl",
        "~/source.jsonl",
        source.header.id.as_str(),
        &source.header.id[..8],
    ] {
        let output = davinci(root.path())
            .args(["--export", reference, "out.jsonl"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", describe(&output));
        let exported = fs::read_to_string(root.path().join("out.jsonl")).unwrap();
        assert!(exported.contains(&source.header.id), "{exported}");
        assert!(exported.contains("export regression marker"), "{exported}");
        assert_eq!(fs::read(&source.path).unwrap(), original);
    }
    let output = davinci(root.path())
        .args(["--export", "source.jsonl", "out.html"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", describe(&output));
    assert!(fs::read_to_string(root.path().join("out.html"))
        .unwrap()
        .contains("id=\"session-data\""));
}

#[test]
fn invalid_mode_fails_before_starting_a_session() {
    let root = tempfile::tempdir().unwrap();
    let output = davinci(root.path())
        .args(["--mode", "banana", "-p", "hello"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid mode \"banana\""), "{}", describe(&output));
    assert!(stderr.contains("text, json, rpc"), "{}", describe(&output));
    assert!(output.stdout.is_empty(), "{}", describe(&output));
    assert!(!root.path().join("sessions").exists());
}

#[test]
fn mode_accepts_text_json_and_rpc_in_spaced_and_equals_forms() {
    for (value, expected) in [("text", Mode::Text), ("json", Mode::Json), ("rpc", Mode::Rpc)] {
        for raw in [
            vec!["--mode".to_string(), value.to_string()],
            vec![format!("--mode={value}")],
        ] {
            let parsed = parse_args(&raw);
            assert_eq!(parsed.mode, Some(expected));
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            assert!(parsed.unknown_flags.is_empty());
        }
    }
}

#[test]
fn invalid_or_missing_modes_report_errors_without_swallowing_flags() {
    for raw in [
        vec!["--mode", "banana"],
        vec!["--mode=banana"],
        vec!["--mode"],
        vec!["--mode="],
        vec!["--mode", "--print", "hello"],
        vec!["--mode", "json", "--mode", "banana"],
    ] {
        let parsed = parse_args(&raw.iter().map(|value| value.to_string()).collect::<Vec<_>>());
        assert!(
            parsed.diagnostics.iter().any(|diagnostic| diagnostic.kind == "error"
                && diagnostic.message.contains("text, json, rpc")),
            "{raw:?}: {:?}",
            parsed.diagnostics
        );
        assert!(!parsed.unknown_flags.contains_key("mode"));
    }
    let parsed = parse_args(&["--mode".into(), "--print".into(), "hello".into()]);
    assert!(parsed.print);
    assert_eq!(parsed.messages, ["hello"]);

    let literal = parse_args(&["--".into(), "--mode".into(), "banana".into()]);
    assert!(literal.diagnostics.is_empty());
    assert_eq!(literal.mode, None);
    assert_eq!(literal.messages, ["--mode", "banana"]);
}
