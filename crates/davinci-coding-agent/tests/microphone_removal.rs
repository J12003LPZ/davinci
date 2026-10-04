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

#[test]
fn removed_voice_subsystem_is_absent_from_workspace() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = crate_dir
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap();

    assert!(
        !workspace.join("crates/davinci-voice").exists(),
        "the retired microphone/STT crate must stay removed"
    );

    for manifest in [
        workspace.join("Cargo.toml"),
        workspace.join("crates/davinci-coding-agent/Cargo.toml"),
        workspace.join("crates/davinci-tui/Cargo.toml"),
    ] {
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            !text.contains("davinci-voice"),
            "{} must not depend on the retired voice subsystem",
            manifest.display()
        );
    }
}
