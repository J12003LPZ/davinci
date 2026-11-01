//! Codemode through the real CLI entry point, without a Node runtime.
use std::process::{Command, Stdio};

/// Codemode records operations in the session journal, so a sessionless run
/// cannot use it. An explicit `--codemode` request fails; the `/config`
/// switch is a preference, so the run continues without Codemode and says why.
#[test]
fn sessionless_codemode_explains_itself_and_only_the_flag_is_fatal() {
    let home = tempfile::tempdir().unwrap();
    let agent_dir = home.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({"codemode": {"mode": "read-only"}}).to_string(),
    )
    .unwrap();
    let workspace = home.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_davinci"))
            .current_dir(&workspace)
            .env("DAVINCI_CODING_AGENT_DIR", &agent_dir)
            .env("PI_CODING_AGENT_DIR", &agent_dir)
            .env("PI_OFFLINE", "1")
            .args(["--no-extensions", "--no-session", "--offline"])
            .args(extra)
            .args(["-p", "hi"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success(), text)
    };
    let (ok, text) = run(&[]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Codemode is on in /config but stays off"),
        "{text}"
    );
    assert!(text.contains("remove --no-session"), "{text}");
    let (ok, text) = run(&["--codemode", "read-only"]);
    assert!(!ok, "{text}");
    assert!(text.contains("remove --no-session"), "{text}");
}

/// `controlled` is a trusted setting, not a script option, and it stays behind
/// the recovery release gate until S05/S15 pass. A bad mode is refused rather
/// than silently mapped to the default.
#[test]
fn controlled_codemode_reports_the_release_gate() {
    let home = tempfile::tempdir().unwrap();
    let agent_dir = home.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({"codemode": {"mode": "controlled"}}).to_string(),
    )
    .unwrap();
    let workspace = home.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_davinci"))
            .current_dir(&workspace)
            .env("DAVINCI_CODING_AGENT_DIR", &agent_dir)
            .env("PI_CODING_AGENT_DIR", &agent_dir)
            .env("PI_OFFLINE", "1")
            .args(["--no-extensions", "--offline"])
            .args(extra)
            .args(["-p", "hi"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success(), text)
    };
    let gate = "Controlled Codemode is unavailable until its recovery acceptance gates pass";
    let (ok, text) = run(&[]);
    assert!(ok, "{text}");
    assert!(text.contains(gate), "{text}");
    let (ok, text) = run(&["--codemode", "controlled"]);
    assert!(!ok, "{text}");
    assert!(text.contains(gate), "{text}");
    let (ok, text) = run(&["--codemode", "bogus"]);
    assert!(!ok, "{text}");
    assert!(
        text.contains("--codemode requires off, read-only, or controlled"),
        "{text}"
    );
}
