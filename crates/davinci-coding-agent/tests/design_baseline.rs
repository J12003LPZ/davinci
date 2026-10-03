//! Characterization gates recorded before design integration.
use davinci_agent::Agent;
use davinci_coding_agent::native_extensions::NativeExtensionHost;

#[test]
fn design_disabled_keeps_tools_unchanged() {
    let expected: Vec<String> =
        serde_json::from_str(include_str!("fixtures/design/baseline-tools.json")).unwrap();
    assert_eq!(NativeExtensionHost::default().tool_names(), expected);
}

#[test]
fn design_disabled_keeps_prompt_projection_unchanged() {
    let agent = Agent::new("Design baseline: ordinary conversation.");
    assert_eq!(
        agent.provider_system_prompt(),
        "Design baseline: ordinary conversation."
    );
}

#[test]
fn design_requires_no_node_when_disabled() {
    // A default host has no supervisor, Node path or browser package configured.
    let host = NativeExtensionHost::default();
    assert!(!host.visual_verification_available());
    assert!(!host
        .tool_names()
        .iter()
        .any(|name| name.starts_with("design_")));
    assert!(!host.has_tool("design_create"));
}
