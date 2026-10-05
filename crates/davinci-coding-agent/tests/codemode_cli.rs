//! Codemode through the real CLI entry point, without a Node runtime.
use std::process::{Command, Stdio};

/// Codemode records operations in the session journal, so a sessionless run
/// must refuse it plainly instead of failing later or dropping the tool.
#[test]
fn sessionless_codemode_is_refused_with_its_reason() {
    let home = tempfile::tempdir().unwrap();
    let agent_dir = home.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let outside = home.path().join("runtime");
    std::fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({"codemode": {
            "mode": "read-only",
            "nodePath": outside.join("node.exe"),
            "hostPath": outside.join("host"),
        }})
        .to_string(),
    )
    .unwrap();
    let workspace = home.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .current_dir(&workspace)
        .env("DAVINCI_CODING_AGENT_DIR", &agent_dir)
        .env("PI_CODING_AGENT_DIR", &agent_dir)
        .env("PI_OFFLINE", "1")
        .args(["--no-extensions", "--no-session", "--offline", "-p", "hi"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("remove --no-session"), "{text}");
}
