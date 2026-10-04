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
fn ambiguous_aliases_fail_before_discovery() {
    assert!(CapabilityPolicy::new(
        CodeModeMode::ReadOnly,
        vec![
            capability("mcp/server.tool", true),
            capability("mcp_server_tool", true)
        ]
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
