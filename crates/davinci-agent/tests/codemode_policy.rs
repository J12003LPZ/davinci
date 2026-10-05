use davinci_agent::codemode::{CapabilityPolicy, CodeModeMode};
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
fn canonical_resolution_never_accepts_an_alias() {
    let policy = CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![capability("mcp/server.tool", true)],
    )
    .unwrap();
    assert!(policy.resolve("mcp/server.tool").is_ok());
    assert!(policy.resolve("mcp_server_tool").is_err());
}
