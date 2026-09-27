use davinci_agent::mcp::McpRegistry;
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_agent::{execute_tool_with, Agent};
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

    let page = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"query": "catalog_tool_", "limit": 20}),
        &agent.tool_context,
    )
    .unwrap();
    let page_details = page.details.as_ref().unwrap();
    assert_eq!(page_details["matching_count"], 200);
    assert_eq!(page_details["matches"].as_array().unwrap().len(), 20);
    assert_eq!(page_details["activated"].as_array().unwrap().len(), 20);
    assert_eq!(page_details["next_cursor"], "20");

    let next_page = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({
            "query": "catalog_tool_",
            "cursor": page_details["next_cursor"].as_str().unwrap(),
            "limit": 20
        }),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(
        next_page.details.as_ref().unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
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

#[test]
fn runtime_less_search_keeps_mcp_fallback_available() {
    let body = json!({
        "initialize": {
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fixture", "version": "0"}
        },
        "tools/list": {
            "tools": [{
                "name": "lookup",
                "description": "read-only fallback fixture",
                "inputSchema": {"type": "object"}
            }]
        },
        "resources/list": {"resources": []}
    })
    .to_string();
    let (dir, config) = http_fixture(&body);
    let registry = McpRegistry::connect(&config, dir.path());
    let mut agent = Agent::new("runtime-less");
    agent.attach_mcp(registry);

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode": "exact", "query": "mcp__memory__lookup"}),
        &agent.tool_context,
    )
    .unwrap();
    let details = result.details.as_ref().unwrap();
    assert_eq!(details["matching_count"], 1);
    assert_eq!(details["matches"], json!(["mcp__memory__lookup"]));
    assert_eq!(details["activated"], json!(["mcp__memory__lookup"]));
    assert!(agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "mcp__memory__lookup"));
}

#[test]
fn real_mcp_registration_exposes_known_families_and_server_namespaces() {
    let body = json!({
        "initialize": {"protocolVersion":"2025-03-26", "capabilities":{"tools":{}},
            "serverInfo":{"name":"fixture", "version":"0"}},
        "tools/list": {"tools":[
            {"name":"git_branch_diff", "inputSchema":{"type":"object"}},
            {"name":"browser_snapshot", "inputSchema":{"type":"object"}},
            {"name":"lookup", "inputSchema":{"type":"object"}}
        ]},
        "resources/list": {"resources":[]}
    })
    .to_string();
    let (dir, config) = http_fixture(&body);
    for runtime_enabled in [false, true] {
        let mut agent = if runtime_enabled {
            agent_with_runtime()
        } else {
            Agent::new("no runtime")
        };
        agent.attach_mcp(McpRegistry::connect(&config, dir.path()));
        for (family, tool) in [
            ("git", "git_branch_diff"),
            ("browser", "browser_snapshot"),
            ("mcp:memory", "lookup"),
        ] {
            let result = execute_tool_with(
                Path::new("."),
                "tool_search",
                &json!({"mode":"family", "query":family}),
                &agent.tool_context,
            )
            .unwrap();
            let name = format!("mcp__memory__{tool}");
            let details = result.details.as_ref().unwrap();
            assert!(details["activated"]
                .as_array()
                .unwrap()
                .contains(&json!(name)));
            assert_eq!(details["families"][&name], family);
            assert!(agent
                .provider_tool_specs()
                .iter()
                .any(|tool| tool.name == name));
        }
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(davinci_agent::PermissionRule::bare("mcp__memory__lookup"));
        agent.sync_tool_authorization();
        let denied = execute_tool_with(
            Path::new("."),
            "tool_search",
            &json!({"mode":"family", "query":"mcp:memory"}),
            &agent.tool_context,
        )
        .unwrap();
        assert_eq!(denied.details.as_ref().unwrap()["matching_count"], 0);
        assert!(!agent.is_tool_visible("mcp__memory__lookup"));
    }
}

#[test]
fn runtime_less_discovery_activates_builtin_schemas_without_inventing_native_tools() {
    let agent = Agent::new("no runtime");
    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode":"exact", "query":"edit"}),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(
        result.details.as_ref().unwrap()["activated"],
        json!(["edit"])
    );
    assert!(agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "edit"));
    let unavailable = execute_tool_with(
        Path::new("."),
        "tool_search",
        &json!({"mode":"family", "query":"git"}),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(unavailable.details.as_ref().unwrap()["matching_count"], 0);
}

#[test]
fn mcp_server_family_uses_exact_handshake_namespace_without_relaxing_trust() {
    let body = json!({
        "initialize": {"protocolVersion":"2025-03-26", "capabilities":{"tools":{}},
            "serverInfo":{"name":"fixture", "version":"0"}},
        "tools/list": {"tools":[{"name":"navigate", "inputSchema":{"type":"object"},
            "annotations":{"readOnlyHint":true}}]},
        "resources/list": {"resources":[]}
    })
    .to_string();
    let (dir, mut config) = http_fixture(&body);
    let server = config.mcp_servers.remove("memory").unwrap();
    config
        .mcp_servers
        .insert("playwright".into(), server.clone());
    config.mcp_servers.insert("browserless".into(), server);
    let registry = McpRegistry::connect(&config, dir.path());
    let capabilities = registry.capabilities();
    let playwright = capabilities
        .iter()
        .find(|cap| cap.name == "mcp__playwright__navigate")
        .unwrap();
    assert_eq!(playwright.family.as_deref(), Some("browser"));
    assert!(
        !playwright.read_only,
        "family classification cannot trust a remote annotation"
    );
    assert_eq!(playwright.tool_class, davinci_agent::ToolClass::Other);
    let other = capabilities
        .iter()
        .find(|cap| cap.name == "mcp__browserless__navigate")
        .unwrap();
    assert_eq!(other.family.as_deref(), Some("mcp:browserless"));
}
