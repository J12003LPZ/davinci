//! Connected MCP servers the agent can call.
//!
//! No TypeScript counterpart. Phase 4 spec:
//! `docs/superpowers/specs/2026-09-01-native-mcp-design.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::tools::{AgentTool, ToolError, ToolResult};

/// One connected (or failed) MCP server, as `/mcp` lists it.
#[derive(Debug, Clone)]
pub struct McpServerRow {
    pub name: String,
    pub transport: String,
    pub status: String,
    pub tools: usize,
    pub error: Option<String>,
    /// Tool names the server listed that cannot become `mcp__<server>__<tool>`
    /// (not `[A-Za-z0-9_-]+`); `/mcp` names them.
    pub skipped: Vec<String>,
}

/// One connected server behind its own lock, so a slow round trip on one
/// server never holds up a call to another (or the registry itself). The
/// scheduler runs read-only MCP tools in the parallel lane; this is what
/// lets them actually overlap.
type SharedClient = Arc<Mutex<davinci_mcp::Client>>;

#[derive(Default)]
struct Inner {
    clients: BTreeMap<String, SharedClient>,
    rows: Vec<McpServerRow>,
}

/// Connected MCP servers, shared with the tool thread and the shell.
#[derive(Clone, Default)]
pub struct McpRegistry {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for McpRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpRegistry")
    }
}

impl McpRegistry {
    /// Spawn and handshake every enabled server. Failures become an error row;
    /// they do not abort the session.
    pub fn connect(config: &davinci_mcp::ConfigFile, cwd: &Path) -> Self {
        let registry = Self::default();
        {
            let mut inner = registry.lock();
            for (name, server) in &config.mcp_servers {
                inner.connect_one(name, server, cwd);
            }
        }
        registry
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Kill every stdio server this process spawned, for an exit that runs
    /// no destructors (a signal, a hung turn). Does not take any registry
    /// lock: one may be held by the call that hung.
    pub fn shutdown_all() {
        davinci_mcp::kill_every_server();
    }

    pub fn rows(&self) -> Vec<McpServerRow> {
        self.lock().rows.clone()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.specs().into_iter().map(|tool| tool.name).collect()
    }

    pub fn read_only_names(&self) -> BTreeSet<String> {
        let inner = self.lock();
        inner
            .clients
            .values()
            .flat_map(|client| {
                let client = client.lock().unwrap_or_else(|err| err.into_inner());
                client
                    .tools
                    .iter()
                    .filter(|tool| tool.read_only())
                    .map(|tool| client.agent_tool_name(&tool.name))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn specs(&self) -> Vec<AgentTool> {
        let inner = self.lock();
        inner
            .clients
            .values()
            .flat_map(|client| {
                let client = client.lock().unwrap_or_else(|err| err.into_inner());
                client
                    .tools
                    .iter()
                    .map(|tool| AgentTool {
                        name: client.agent_tool_name(&tool.name),
                        description: format!(
                            "mcp:{}. {}",
                            client.name,
                            tool.description.as_deref().unwrap_or("")
                        ),
                        parameters: tool.input_schema.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Resources currently listed, for `mcp_read`'s description.
    pub fn mcp_read_description(&self) -> String {
        let inner = self.lock();
        let mut listed = Vec::new();
        for client in inner.clients.values() {
            let client = client.lock().unwrap_or_else(|err| err.into_inner());
            for resource in &client.resources {
                let label = resource.name.as_deref().unwrap_or("");
                if label.is_empty() {
                    listed.push(format!("{} {}", client.name, resource.uri));
                } else {
                    listed.push(format!("{} {} ({label})", client.name, resource.uri));
                }
            }
        }
        if listed.is_empty() {
            "Read a resource from a connected MCP server. Pass { server, uri }.".into()
        } else {
            format!(
                "Read a resource from a connected MCP server. Pass {{ server, uri }}. Listed: {}.",
                listed.join(", ")
            )
        }
    }

    /// The server's own handle, fetched under the registry lock and used
    /// after it is released.
    fn client(&self, server: &str) -> Result<SharedClient, ToolError> {
        self.lock()
            .clients
            .get(server)
            .cloned()
            .ok_or_else(|| ToolError::Failed(format!("no MCP server named `{server}`")))
    }

    pub fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<ToolResult, ToolError> {
        let client = self.client(server)?;
        let result = client
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .call_tool(tool, arguments.clone());
        match result {
            Ok(result) => Ok(ToolResult {
                content: result.text(),
                is_error: result.is_error.unwrap_or(false),
                details: None,
            }),
            Err(err) => self.lock().failed(server, err),
        }
    }

    pub fn read(&self, server: &str, uri: &str) -> Result<ToolResult, ToolError> {
        let client = self.client(server)?;
        let result = client
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .read_resource(uri);
        match result {
            Ok(text) => Ok(ToolResult {
                content: text,
                is_error: false,
                details: None,
            }),
            Err(err) => self.lock().failed(server, err),
        }
    }
}

impl Inner {
    /// A JSON-RPC error reply is the server's answer to this one call:
    /// `is_error` on the result, server kept. Anything else (closed pipe,
    /// timeout, decode) means the server is gone: it is dropped and its row
    /// turns into an error.
    fn failed(&mut self, server: &str, err: davinci_mcp::Error) -> Result<ToolResult, ToolError> {
        if err.is_rpc() {
            return Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
                details: None,
            });
        }
        self.drop_server(server, err.to_string());
        Err(ToolError::Failed(err.to_string()))
    }

    fn connect_one(&mut self, name: &str, server: &davinci_mcp::ServerConfig, cwd: &Path) {
        let transport_label = if server.url.is_some() {
            "http"
        } else {
            "stdio"
        };
        if !davinci_mcp::is_ident(name) {
            self.rows.push(McpServerRow {
                name: name.to_string(),
                transport: transport_label.into(),
                status: "error".into(),
                tools: 0,
                error: Some("name is not [A-Za-z0-9_-]+".into()),
                skipped: Vec::new(),
            });
            return;
        }
        if server.disabled {
            self.rows.push(McpServerRow {
                name: name.to_string(),
                transport: transport_label.into(),
                status: "disabled".into(),
                tools: 0,
                error: None,
                skipped: Vec::new(),
            });
            return;
        }
        let transport = match server.transport() {
            Ok(transport) => transport,
            Err(err) => {
                self.rows.push(McpServerRow {
                    name: name.to_string(),
                    transport: transport_label.into(),
                    status: "error".into(),
                    tools: 0,
                    error: Some(err.to_string()),
                    skipped: Vec::new(),
                });
                return;
            }
        };
        match davinci_mcp::Client::connect(name, transport, cwd) {
            Ok(client) => {
                let tools = client.tools.len();
                let skipped = client.skipped.clone();
                self.clients
                    .insert(name.to_string(), Arc::new(Mutex::new(client)));
                self.rows.push(McpServerRow {
                    name: name.to_string(),
                    transport: transport_label.into(),
                    status: "connected".into(),
                    tools,
                    error: None,
                    skipped,
                });
            }
            Err(err) => {
                self.rows.push(McpServerRow {
                    name: name.to_string(),
                    transport: transport_label.into(),
                    status: "error".into(),
                    tools: 0,
                    error: Some(err.to_string()),
                    skipped: Vec::new(),
                });
            }
        }
    }

    fn drop_server(&mut self, name: &str, error: String) {
        self.clients.remove(name);
        if let Some(row) = self.rows.iter_mut().find(|row| row.name == name) {
            row.status = "error".into();
            row.tools = 0;
            row.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn a_fixture_server_becomes_one_agent_tool() {
        let (_dir, config) = http_fixture(
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"echo","description":"echo text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}},"annotations":{"readOnlyHint":true}}]},
              "tools/call": {"content":[{"type":"text","text":"hi"}]},
              "resources/list": {"resources":[{"uri":"fixture://note","name":"note"}]},
              "resources/read": {"contents":[{"uri":"fixture://note","text":"a note"}]}
            }"#,
        );
        let registry = McpRegistry::connect(&config, Path::new("."));
        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "mcp__memory__echo");
        assert!(specs[0].description.starts_with("mcp:memory."));
        assert!(registry.read_only_names().contains("mcp__memory__echo"));
        let result = registry
            .call("memory", "echo", &json!({"text": "hi"}))
            .unwrap();
        assert_eq!(result.content, "hi");
        let read = registry.read("memory", "fixture://note").unwrap();
        assert_eq!(read.content, "a note");
        assert!(registry.mcp_read_description().contains("fixture://note"));
        let rows = registry.rows();
        assert_eq!(rows[0].status, "connected");
        assert_eq!(rows[0].transport, "http");
        assert_eq!(rows[0].tools, 1);

        let mut agent = crate::Agent::new("test");
        agent.attach_mcp(registry);
        assert!(agent.tools.iter().any(|name| name == "mcp__memory__echo"));
        let specs = agent.builtin_and_mcp_specs();
        assert!(specs.iter().any(|tool| tool.name == "mcp__memory__echo"));
        let result = crate::execute_tool_with(
            Path::new("."),
            "mcp__memory__echo",
            &json!({"text": "hi"}),
            &agent.tool_context,
        )
        .unwrap();
        assert_eq!(result.content, "hi");
        let read = crate::execute_tool_with(
            Path::new("."),
            "mcp_read",
            &json!({"server": "memory", "uri": "fixture://note"}),
            &agent.tool_context,
        )
        .unwrap();
        assert_eq!(read.content, "a note");
    }

    #[test]
    fn an_rpc_error_is_the_tools_answer_and_keeps_the_server() {
        let (_dir, config) = http_fixture(
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"echo","inputSchema":{"type":"object"}}]},
              "tools/call": {"$rpcError":{"code":-32602,"message":"invalid params"}},
              "resources/list": {"resources":[{"uri":"fixture://note"}]},
              "resources/read": {"contents":[{"uri":"fixture://note","text":"a note"}]}
            }"#,
        );
        let registry = McpRegistry::connect(&config, Path::new("."));
        let result = registry
            .call("memory", "echo", &json!({}))
            .expect("an rpc error is a tool result, not a failure");
        assert!(result.is_error);
        assert_eq!(result.content, "mcp -32602: invalid params");
        // Still connected: listed, callable, readable.
        let rows = registry.rows();
        assert_eq!(rows[0].status, "connected");
        assert_eq!(rows[0].tools, 1);
        assert_eq!(registry.tool_names(), vec!["mcp__memory__echo".to_string()]);
        assert_eq!(
            registry.read("memory", "fixture://note").unwrap().content,
            "a note"
        );
    }

    #[test]
    fn a_failed_list_is_an_error_row_and_skipped_names_are_kept() {
        let (_dir, config) = http_fixture(
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"$rpcError":{"code":-32603,"message":"list broke"}}
            }"#,
        );
        let registry = McpRegistry::connect(&config, Path::new("."));
        let rows = registry.rows();
        assert_eq!(rows[0].status, "error");
        assert_eq!(rows[0].tools, 0);
        let error = rows[0].error.as_deref().unwrap();
        assert!(error.contains("list broke"), "{error}");
        assert!(registry.specs().is_empty());

        let (_dir, config) = http_fixture(
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"echo"},{"name":"has space"},{"name":"a.b"}]},
              "resources/list": {"resources":[]}
            }"#,
        );
        let registry = McpRegistry::connect(&config, Path::new("."));
        let rows = registry.rows();
        assert_eq!(rows[0].status, "connected");
        assert_eq!(rows[0].tools, 1);
        assert_eq!(
            rows[0].skipped,
            vec!["has space".to_string(), "a.b".to_string()]
        );
        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].parameters, davinci_mcp::default_input_schema());
    }

    #[test]
    fn a_disabled_server_is_listed_and_not_spawned() {
        let config = davinci_mcp::parse_config(
            r#"{"mcpServers":{"off":{"command":"does-not-exist","disabled":true}}}"#,
        )
        .unwrap();
        let registry = McpRegistry::connect(&config, Path::new("."));
        assert!(registry.specs().is_empty());
        assert_eq!(registry.rows()[0].status, "disabled");
    }
}
