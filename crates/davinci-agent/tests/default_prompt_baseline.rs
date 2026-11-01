use davinci_agent::default_system_prompt;
use sha2::{Digest, Sha256};

#[test]
fn default_prompt_v1_matches_committed_baseline() {
    let actual = default_system_prompt();
    let expected = include_str!("fixtures/default_system_prompt_v1.txt").replace("\r\n", "\n");

    assert_eq!(actual, expected);

    let hash = format!(
        "{:x}",
        davinci_sys::hex::Lower(&Sha256::digest(actual.as_bytes()))
    );
    assert!(!hash.is_empty());
}

#[test]
fn script_prompt_only_in_script_mode() {
    use davinci_agent::codemode::CodeModeMode;
    use davinci_agent::prompt::script_mode_module;
    use std::collections::BTreeMap;

    let mut aliases = BTreeMap::new();
    aliases.insert("tools.Read".to_string(), "read".to_string());
    aliases.insert("tools.Execute".to_string(), "exec_command".to_string());
    // Off and read-only never add a hidden Script prompt.
    assert!(script_mode_module(&CodeModeMode::Off, &aliases).is_none());
    assert!(script_mode_module(&CodeModeMode::ReadOnly, &aliases).is_none());
    // Nothing to teach when no alias is bound.
    assert!(script_mode_module(&CodeModeMode::Controlled, &BTreeMap::new()).is_none());

    let module = script_mode_module(&CodeModeMode::Controlled, &aliases).unwrap();
    for expected in [
        "tools.Read",
        "tools.Execute",
        "// @input",
        "text(",
        "notify(",
    ] {
        assert!(module.body.contains(expected), "{expected}");
    }
    // Phantom controls never appear: risk is host-owned and WaitForScript is
    // not a model tool yet.
    for phantom in ["riskLevel", "WaitForScript"] {
        assert!(!module.body.contains(phantom), "{phantom}");
    }
    // Only actually bound aliases are advertised.
    assert!(!module.body.contains("tools.ApplyPatch"));
}
