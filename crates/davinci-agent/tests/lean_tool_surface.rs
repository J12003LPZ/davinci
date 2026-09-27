use davinci_agent::runtime::{
    AgentId, CapabilitySource, RunId, RuntimeBus, RuntimeCapability, RuntimeHandle,
};
use davinci_agent::{execute_tool_with, Agent, ToolClass, ToolSurface};
use serde_json::json;
use std::path::Path;

fn lean_agent() -> Agent {
    let mut agent = Agent::new("lean surface fixture");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.turn_context_placement_override =
        Some(davinci_agent::turn_context::TurnContextPlacement::Appended);
    agent.tool_surface = ToolSurface::Lean;
    agent.freeze_tools_for_cache();
    agent
}

fn specialist_capability(name: &str, family: &str) -> RuntimeCapability {
    RuntimeCapability::new(
        name,
        CapabilitySource::Mcp,
        ToolClass::Read,
        true,
        &json!({"type": "object", "properties": {"value": {"type": "string"}}}),
        None,
    )
    .with_family(family)
}

#[test]
fn lean_surface_exposes_core_and_defers_specialists() {
    let agent = lean_agent();
    let visible = agent.visible_tool_names();
    for name in [
        "read",
        "grep",
        "find",
        "ls",
        "exec_command",
        "write_stdin",
        "apply_patch",
        "batch",
        "tool_search",
        "update_plan",
        "propose_plan",
        "ask_user_question",
        "agent",
        "job_output",
        "job_kill",
    ] {
        assert!(visible.contains(name), "missing lean core tool {name}");
    }
    for name in [
        "edit",
        "write",
        "powershell",
        "web_search",
        "web_fetch",
        "todo",
    ] {
        assert!(
            !visible.contains(name),
            "specialist or legacy tool leaked: {name}"
        );
    }
}

#[test]
fn family_discovery_activates_all_members_and_pages_names() {
    let mut agent = lean_agent();
    let names = (0..10)
        .map(|index| format!("family_tool_{index}"))
        .collect::<Vec<_>>();
    agent
        .runtime
        .as_ref()
        .expect("runtime")
        .capability_registry
        .register_all(
            names
                .iter()
                .map(|name| specialist_capability(name, "fixture-lsp")),
        );
    agent.apply_extension_tools(&names);
    agent.freeze_tools_for_cache();

    assert!(names.iter().all(|name| !agent.is_tool_visible(name)));
    let first = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "family", "query": "fixture-lsp", "limit": 5}),
        &agent.tool_context,
    )
    .unwrap();
    let details = first.details.as_ref().unwrap();
    assert_eq!(details["matching_count"], 10);
    assert_eq!(details["activated_count"], 10);
    assert_eq!(details["matches"].as_array().unwrap().len(), 5);
    assert_eq!(details["next_cursor"], "5");
    assert!(names.iter().all(|name| agent.is_tool_visible(name)));

    let repeat = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "family", "query": "fixture-lsp", "limit": 20}),
        &agent.tool_context,
    )
    .unwrap();
    let repeat_details = repeat.details.as_ref().unwrap();
    assert_eq!(repeat_details["activated_count"], 10);
    assert_eq!(repeat_details["matches"].as_array().unwrap().len(), 10);
}

#[test]
fn search_pagination_and_exact_mode_are_stable_and_authorized() {
    let mut agent = lean_agent();
    let names = (0..7)
        .map(|index| format!("browser_fixture_{index}"))
        .collect::<Vec<_>>();
    agent
        .runtime
        .as_ref()
        .expect("runtime")
        .capability_registry
        .register_all(
            names
                .iter()
                .map(|name| specialist_capability(name, "browser")),
        );
    agent.apply_extension_tools(&names);

    let first = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"query": "browser_fixture", "limit": 3}),
        &agent.tool_context,
    )
    .unwrap();
    let first_details = first.details.as_ref().unwrap();
    assert_eq!(first_details["matching_count"], 7);
    assert_eq!(first_details["matches"].as_array().unwrap().len(), 3);
    let cursor = first_details["next_cursor"].as_str().unwrap();

    let second = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"query": "browser_fixture", "cursor": cursor, "limit": 20}),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(
        second.details.as_ref().unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    let exact = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "exact", "query": names[3]}),
        &agent.tool_context,
    )
    .unwrap();
    let exact_details = exact.details.as_ref().unwrap();
    assert_eq!(exact_details["matching_count"], 1);
    assert_eq!(exact_details["matches"], json!([names[3].clone()]));
    assert_eq!(exact_details["activated"], json!([names[3].clone()]));

    let unauthorized = names[6].clone();
    agent.set_active_tools_by_name(&["read".into(), "tool_search".into()]);
    let denied = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "exact", "query": unauthorized}),
        &agent.tool_context,
    )
    .unwrap();
    let denied_details = denied.details.as_ref().unwrap();
    assert_eq!(denied_details["matching_count"], 0);
    assert!(!agent.is_tool_visible(&unauthorized));
}

#[test]
fn family_discovery_separates_authorization_from_schema_availability() {
    let mut agent = lean_agent();
    let allowed = specialist_capability("family_allowed", "fixture-family");
    let schema_less = RuntimeCapability::with_raw_hash(
        "family_schema_less",
        CapabilitySource::Mcp,
        ToolClass::Read,
        true,
        "fixture-schema-less",
        None,
    )
    .with_family("fixture-family");
    let denied = specialist_capability("family_denied", "fixture-family");
    agent
        .runtime
        .as_ref()
        .expect("runtime")
        .capability_registry
        .register_all([allowed, schema_less, denied]);
    agent.apply_extension_tools(&["family_allowed".into(), "family_schema_less".into()]);

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "family", "query": "fixture-family"}),
        &agent.tool_context,
    )
    .unwrap();
    let details = result.details.as_ref().unwrap();
    assert_eq!(details["matching_count"], 2);
    assert_eq!(details["activated_count"], 1);
    assert!(agent.is_tool_visible("family_allowed"));
    assert!(!agent.is_tool_visible("family_schema_less"));
    assert!(!agent.is_tool_visible("family_denied"));
}
