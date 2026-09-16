use davinci_agent::{execute_tool_with, Agent};
use davinci_agent::mcp::McpRegistry;
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use serde_json::{json, Value};
use std::path::Path;

fn http_fixture(body: &str) -> (tempfile::TempDir, davinci_mcp::ConfigFile) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-fixture.json");
    std::fs::write(&path, body).unwrap();
    let url = format!("fixture:{}", path.display());
    let config = davinci_mcp::parse_config(&format!(
        r#"{{"mcpServers":{{"memory":{{"url":{}}}}}}}"#,
        serde_json::to_string(&url).unwrap()
    ))
    .unwrap();
    (dir, config)
}

fn agent_with_runtime() -> Agent {
    let mut agent = Agent::new("test");
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    agent.runtime = Some(runtime.clone());
    agent.tool_context.runtime = Some(runtime);
    agent
}

#[test]
fn mcp_large_catalog_uses_progressive_schema_exposure() {
    let tools = (0..200)
        .map(|index| {
            json!({
                "name": format!("catalog_tool_{index}"),
                "description": format!("catalog fixture tool {index}"),
                "inputSchema": {"type": "object"}
            })
        })
        .collect::<Vec<Value>>();
    let body = json!({
        "initialize": {
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fixture", "version": "0"}
        },
        "tools/list": {"tools": tools},
        "resources/list": {"resources": []}
    })
    .to_string();
    let (dir, config) = http_fixture(&body);
    let registry = McpRegistry::connect(&config, dir.path());
    assert_eq!(registry.tool_names().len(), 200);

    let mut agent = agent_with_runtime();
    agent.attach_mcp(registry);
    let target = "mcp__memory__catalog_tool_123";
    assert!(agent.tools.iter().any(|name| name == target));

    let initial = agent.provider_tool_specs();
    assert!(
        !initial.iter().any(|tool| tool.name == target),
        "large MCP catalogs must stay deferred until discovery"
    );

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"query": "catalog_tool_123"}),
        &agent.tool_context,
    )
    .unwrap();
    let activated = result
        .details
        .as_ref()
        .and_then(|details| details.get("activated"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(activated, &vec![Value::String(target.to_string())]);

    let visible_mcp = agent
        .provider_tool_specs()
        .into_iter()
        .filter(|tool| tool.name.starts_with("mcp__"))
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(visible_mcp, vec![target.to_string()]);
}

#[test]
fn tool_search_cannot_activate_denied_mcp_tool() {
    let body = json!({
        "initialize": {
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fixture", "version": "0"}
        },
        "tools/list": {
            "tools": [{
                "name": "dangerous_write",
                "description": "mutation fixture",
                "inputSchema": {"type": "object"}
            }]
        },
        "resources/list": {"resources": []}
    })
    .to_string();
    let (dir, config) = http_fixture(&body);
    let registry = McpRegistry::connect(&config, dir.path());

    let mut agent = agent_with_runtime();
    agent.attach_mcp(registry);
    let denied = "mcp__memory__dangerous_write";
    agent.tools = vec!["read".to_string(), "tool_search".to_string()];
    let before = agent.provider_tool_specs();
    assert!(!before.iter().any(|tool| tool.name == denied));

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"query": "dangerous_write"}),
        &agent.tool_context,
    )
    .unwrap();
    let activated = result
        .details
        .as_ref()
        .and_then(|details| details.get("activated"))
        .and_then(Value::as_array)
        .unwrap();
    assert!(activated.is_empty());
    assert!(!agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == denied));
}
