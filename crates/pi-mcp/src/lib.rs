//! Native MCP client: JSON-RPC 2.0 over stdio and streamable HTTP.
//!
//! No TypeScript counterpart (vendor `pi` has no MCP). Phase 4 spec:
//! `docs/superpowers/specs/2026-09-01-native-mcp-design.md`.

mod config;
mod http;
mod jsonrpc;
mod stdio;
mod types;

pub use config::{load_path, merge, parse as parse_config, File as ConfigFile, ServerConfig};
pub use jsonrpc::{RpcError, RpcId};
pub use stdio::StdioTransport;
pub use types::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, Resource, ServerEntry,
    ToolAnnotations, ToolSpec,
};

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Child;
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2025-03-26";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Protocol(String),
    #[error("mcp io: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Http(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// How a server is started.
#[derive(Debug, Clone)]
pub enum TransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

enum Transport {
    Stdio(StdioTransport),
    Http(http::HttpTransport),
}

/// One connected MCP server.
pub struct Client {
    pub name: String,
    transport: Transport,
    pub initialize: InitializeResult,
    pub tools: Vec<ToolSpec>,
    pub resources: Vec<Resource>,
    child: Option<Child>,
}

impl Client {
    pub fn connect(name: &str, config: TransportConfig, cwd: &Path) -> Result<Self> {
        match config {
            TransportConfig::Stdio { command, args, env } => {
                let (mut transport, child) = StdioTransport::spawn(&command, &args, &env, cwd)?;
                let (initialize, tools, resources) = handshake(&mut transport)?;
                Ok(Self {
                    name: name.to_string(),
                    transport: Transport::Stdio(transport),
                    initialize,
                    tools,
                    resources,
                    child: Some(child),
                })
            }
            TransportConfig::Http { url, headers } => {
                let mut transport = http::HttpTransport::new(&url, headers)?;
                let (initialize, tools, resources) = handshake(&mut transport)?;
                Ok(Self {
                    name: name.to_string(),
                    transport: Transport::Http(transport),
                    initialize,
                    tools,
                    resources,
                    child: None,
                })
            }
        }
    }

    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<CallToolResult> {
        let result = match &mut self.transport {
            Transport::Stdio(t) => t.call(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )?,
            Transport::Http(t) => t.call(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )?,
        };
        serde_json::from_value(result).map_err(|err| Error::Protocol(format!("tools/call: {err}")))
    }

    pub fn read_resource(&mut self, uri: &str) -> Result<String> {
        let result = match &mut self.transport {
            Transport::Stdio(t) => t.call("resources/read", json!({ "uri": uri }))?,
            Transport::Http(t) => t.call("resources/read", json!({ "uri": uri }))?,
        };
        contents_text(&result)
    }

    pub fn agent_tool_name(&self, tool: &str) -> String {
        format!("mcp__{}__{tool}", self.name)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

trait Rpc {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    fn notify(&mut self, method: &str, params: Value) -> Result<()>;
}

fn handshake(rpc: &mut dyn Rpc) -> Result<(InitializeResult, Vec<ToolSpec>, Vec<Resource>)> {
    let init = rpc.call(
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "pi", "version": env!("CARGO_PKG_VERSION") }
        }),
    )?;
    let initialize: InitializeResult = serde_json::from_value(init)
        .map_err(|err| Error::Protocol(format!("initialize: {err}")))?;
    rpc.notify("notifications/initialized", json!({}))?;
    let tools = match rpc.call("tools/list", json!({})) {
        Ok(value) => parse_tools(value),
        Err(_) => Vec::new(),
    };
    let resources = match rpc.call("resources/list", json!({})) {
        Ok(value) => parse_resources(value),
        Err(_) => Vec::new(),
    };
    Ok((initialize, tools, resources))
}

fn parse_tools(value: Value) -> Vec<ToolSpec> {
    value
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| serde_json::from_value(tool.clone()).ok())
        .filter(|tool: &ToolSpec| is_ident(&tool.name))
        .collect()
}

fn parse_resources(value: Value) -> Vec<Resource> {
    value
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|resource| serde_json::from_value(resource.clone()).ok())
        .collect()
}

fn contents_text(result: &Value) -> Result<String> {
    let contents = result
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = String::new();
    for item in contents {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    Ok(out)
}

/// Server and tool names are `mcp__<server>__<tool>`.
pub fn is_ident(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub fn split_agent_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if is_ident(server) && is_ident(tool) {
        Some((server, tool))
    } else {
        None
    }
}

pub const CALL_TIMEOUT_SECS: u64 = CALL_TIMEOUT.as_secs();

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_exe() -> std::path::PathBuf {
        for key in ["CARGO_BIN_EXE_mcp_fixture", "CARGO_BIN_EXE_mcp-fixture"] {
            if let Ok(path) = std::env::var(key) {
                return std::path::PathBuf::from(path);
            }
        }
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.push("target");
        path.push(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        });
        path.push(format!("mcp-fixture{}", std::env::consts::EXE_SUFFIX));
        path
    }

    #[test]
    fn names_round_trip() {
        assert!(is_ident("server-memory"));
        assert!(!is_ident("has space"));
        assert_eq!(
            split_agent_tool_name("mcp__memory__echo"),
            Some(("memory", "echo"))
        );
        assert_eq!(split_agent_tool_name("echo"), None);
    }

    #[test]
    fn a_stdio_fixture_lists_and_calls_a_tool() {
        let exe = fixture_exe();
        assert!(exe.is_file(), "missing fixture bin at {}", exe.display());
        let mut client = Client::connect(
            "memory",
            TransportConfig::Stdio {
                command: exe.to_string_lossy().into_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            Path::new("."),
        )
        .expect("fixture");
        assert_eq!(client.initialize.protocol_version, PROTOCOL_VERSION);
        assert_eq!(client.tools.len(), 1);
        assert_eq!(client.tools[0].name, "echo");
        assert!(client.tools[0].read_only());
        assert_eq!(client.agent_tool_name("echo"), "mcp__memory__echo");
        let result = client
            .call_tool("echo", json!({"text": "hi"}))
            .expect("call");
        assert_eq!(result.text(), "hi");
        assert_eq!(client.resources.len(), 1);
        assert_eq!(
            client.read_resource("fixture://note").expect("read"),
            "a note"
        );
    }

    #[test]
    fn an_http_fixture_answers_without_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"ping","inputSchema":{"type":"object"}}]},
              "tools/call": {"content":[{"type":"text","text":"pong"}]},
              "resources/list": {"resources":[]}
            }"#,
        )
        .unwrap();
        std::env::set_var("PI_MCP_FIXTURE", &path);
        let mut client = Client::connect(
            "docs",
            TransportConfig::Http {
                url: "https://example.invalid/mcp".into(),
                headers: BTreeMap::new(),
            },
            Path::new("."),
        )
        .expect("http fixture");
        std::env::remove_var("PI_MCP_FIXTURE");
        assert_eq!(client.tools[0].name, "ping");
        assert_eq!(client.call_tool("ping", json!({})).unwrap().text(), "pong");
    }
}
