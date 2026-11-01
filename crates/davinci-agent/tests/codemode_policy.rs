use davinci_agent::codemode::{resolve_script_alias, CapabilityPolicy, CodeModeMode};
use davinci_agent::runtime::{CapabilitySource, RuntimeCapability};
use davinci_agent::ToolClass;

fn capability(name: &str, read_only: bool) -> RuntimeCapability {
    RuntimeCapability::new(
        name,
        CapabilitySource::Builtin,
        ToolClass::Read,
        read_only,
        &serde_json::json!({"type":"object"}),
        None,
    )
}

#[test]
fn off_and_read_only_never_admit_mutations() {
    assert!(
        CapabilityPolicy::new(CodeModeMode::Off, vec![capability("read", true)])
            .unwrap()
            .tools()
            .is_empty()
    );
    assert!(
        CapabilityPolicy::new(CodeModeMode::ReadOnly, vec![capability("write", false)])
            .unwrap()
            .tools()
            .is_empty()
    );
}

#[test]
fn orchestrators_and_prototype_names_are_excluded() {
    for name in [
        "codemode",
        "batch",
        "agent",
        "graph_submit",
        "workflow_start",
        "constructor",
        "__proto__",
        "prototype",
        "agent_message",
        "session_fork",
    ] {
        assert!(
            CapabilityPolicy::new(CodeModeMode::Controlled, vec![capability(name, true)])
                .unwrap()
                .tools()
                .is_empty(),
            "{name}"
        );
    }
}

#[test]
fn ambiguous_aliases_are_excluded_without_disabling_other_tools() {
    let policy = CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![
            capability("mcp/server.tool", true),
            capability("mcp_server_tool", true),
            capability("read", true),
        ],
    )
    .unwrap();
    assert!(policy.resolve("mcp/server.tool").is_err());
    assert!(policy.resolve("mcp_server_tool").is_err());
    assert!(policy.resolve("read").is_ok());
    assert_eq!(policy.tools().len(), 1);
    // Exclusion is reported, and the refusal names the reason.
    assert_eq!(
        policy.alias_collisions(),
        vec!["mcp/server.tool".to_string(), "mcp_server_tool".to_string()]
    );
    assert!(policy
        .resolve("mcp_server_tool")
        .unwrap_err()
        .message
        .contains("collides"));
}

#[test]
fn duplicate_canonical_names_fail_before_discovery() {
    assert!(CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![capability("read", true), capability("read", true)]
    )
    .is_err());
}

#[test]
fn codemode_aliases_are_policy_bounded() {
    let full = CapabilityPolicy::new(
        CodeModeMode::Controlled,
        vec![
            capability("read", true),
            capability("grep", true),
            capability("find", true),
            capability("ls", true),
            capability("exec_command", false),
            capability("apply_patch", false),
        ],
    )
    .unwrap();
    for (alias, canonical) in [
        ("tools.Read", "read"),
        ("tools.Grep", "grep"),
        ("tools.Glob", "find"),
        ("tools.LS", "ls"),
        ("tools.Execute", "exec_command"),
        ("tools.ApplyPatch", "apply_patch"),
    ] {
        assert_eq!(
            resolve_script_alias(alias, CodeModeMode::Controlled, &full).unwrap(),
            canonical,
            "{alias}"
        );
    }
    // Read-only admits reads but never an effectful alias.
    let read_only = CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![
            capability("read", true),
            capability("exec_command", false),
            capability("apply_patch", false),
        ],
    )
    .unwrap();
    assert_eq!(
        resolve_script_alias("tools.Read", CodeModeMode::ReadOnly, &read_only).unwrap(),
        "read"
    );
    assert!(resolve_script_alias("tools.ApplyPatch", CodeModeMode::ReadOnly, &read_only).is_err());
    assert!(resolve_script_alias("tools.Execute", CodeModeMode::ReadOnly, &read_only).is_err());

    // Revoking the shell capability revokes `tools.Execute`; the alias never
    // synthesizes a grant.
    let no_shell = CapabilityPolicy::new(
        CodeModeMode::Controlled,
        vec![capability("read", true), capability("apply_patch", false)],
    )
    .unwrap();
    assert!(resolve_script_alias("tools.Execute", CodeModeMode::Controlled, &no_shell).is_err());

    // Unknown and unmapped names deny.
    assert!(resolve_script_alias("tools.Delete", CodeModeMode::Controlled, &full).is_err());
    assert!(resolve_script_alias("Read", CodeModeMode::Controlled, &full).is_err());
    assert!(resolve_script_alias("tools.Read", CodeModeMode::Off, &full).is_err());

    // A discovered tool whose script name is exactly the alias makes the
    // alias ambiguous, so it denies instead of silently binding one meaning.
    let ambiguous = CapabilityPolicy::new(
        CodeModeMode::Controlled,
        vec![capability("Read", true), capability("read", true)],
    )
    .unwrap();
    let error =
        resolve_script_alias("tools.Read", CodeModeMode::Controlled, &ambiguous).unwrap_err();
    assert_eq!(error.code, "DENIED");
    assert!(error.message.contains("collides"));

    // The alias snapshot is frozen for the Script's lifetime: a later policy
    // change cannot retarget it.
    let snapshot = full.script_aliases();
    assert_eq!(
        snapshot.get("tools.Execute").map(String::as_str),
        Some("exec_command")
    );
    assert_eq!(full.script_aliases(), snapshot);
}

#[test]
fn canonical_resolution_never_accepts_an_alias() {
    let policy = CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![capability("mcp/server.tool", true)],
    )
    .unwrap();
    assert!(policy.resolve("mcp/server.tool").is_ok());
    assert!(policy.resolve("mcp_server_tool").is_err());
}
