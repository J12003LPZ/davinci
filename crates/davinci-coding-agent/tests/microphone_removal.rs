//! Microphone removal must not change the ordinary CLI help path.
use std::process::{Command, Stdio};

#[test]
fn help_no_longer_advertises_local_microphone_commands() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .arg("--help")
        .current_dir(root.path())
        .env("PI_CODING_AGENT_DIR", root.path())
        .env("DAVINCI_CODING_AGENT_DIR", root.path())
        .env("PI_OFFLINE", "1")
        .env("PI_DISABLE_NETWORK", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage:"));
    assert!(help.contains("--print"));
    assert!(
        !help.contains("davinci voice"),
        "removed microphone commands must not appear in help"
    );
}
