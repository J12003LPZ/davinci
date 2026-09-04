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
pub use http::parse_http_body;
pub use jsonrpc::{RpcError, RpcId};
pub use stdio::{resolve_command_in, StdioTransport, STDERR_TAIL_BYTES};
pub use types::{
    base64_decoded_len, default_input_schema, CallToolResult, ContentBlock, Implementation,
    InitializeResult, Resource, ServerEntry, ToolAnnotations, ToolSpec,
};

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2025-03-26";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// How many `nextCursor` pages a list may span before we stop believing it.
const MAX_LIST_PAGES: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server spoke, but not the shape we expected (handshake, decode).
    #[error("{0}")]
    Protocol(String),
    #[error("mcp io: {0}")]
    Io(#[from] io::Error),
    /// The channel to the server failed: spawn, closed pipe, timeout, HTTP
    /// status. The server is unusable afterwards.
    #[error("{0}")]
    Transport(String),
    /// A JSON-RPC error reply. The server is fine; this one call was not.
    #[error("mcp {code}: {message}")]
    Rpc { code: i64, message: String },
}

impl Error {
    /// A JSON-RPC error reply, as opposed to a broken connection.
    pub fn is_rpc(&self) -> bool {
        matches!(self, Error::Rpc { .. })
    }

    /// Whether the registry should forget the server after this error.
    pub fn drops_server(&self) -> bool {
        !self.is_rpc()
    }
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

/// Every stdio child still alive, so a `process::exit` path can reap them
/// without reaching the registries that own the clients (a registry lock
/// may be held by a call in flight).
static CHILDREN: Mutex<Vec<Weak<Mutex<Option<Child>>>>> = Mutex::new(Vec::new());

fn track_child(child: &Arc<Mutex<Option<Child>>>) {
    let mut children = CHILDREN.lock().unwrap_or_else(|err| err.into_inner());
    children.retain(|weak| weak.strong_count() > 0);
    children.push(Arc::downgrade(child));
}

/// Kill and reap every stdio server this process spawned. Clients still
/// holding one see a transport error on their next call.
pub fn kill_every_server() {
    let children = std::mem::take(&mut *CHILDREN.lock().unwrap_or_else(|err| err.into_inner()));
    for weak in children {
        if let Some(slot) = weak.upgrade() {
            kill_child(&slot);
        }
    }
}

fn kill_child(slot: &Mutex<Option<Child>>) {
    if let Some(mut child) = slot.lock().unwrap_or_else(|err| err.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// One connected MCP server.
pub struct Client {
    pub name: String,
    transport: Transport,
    pub initialize: InitializeResult,
    pub tools: Vec<ToolSpec>,
    /// Tool names the server listed that are not `[A-Za-z0-9_-]+` and so
    /// cannot become `mcp__<server>__<tool>`; `/mcp` names them.
    pub skipped: Vec<String>,
    pub resources: Vec<Resource>,
    child: Option<Arc<Mutex<Option<Child>>>>,
}

impl Client {
    pub fn connect(name: &str, config: TransportConfig, cwd: &Path) -> Result<Self> {
        Self::connect_with_timeout(name, config, cwd, CALL_TIMEOUT)
    }

    /// `connect` with a per-call deadline other than [`CALL_TIMEOUT_SECS`];
    /// tests use it so a hung fixture fails in a second, not a minute.
    pub fn connect_with_timeout(
        name: &str,
        config: TransportConfig,
        cwd: &Path,
        call_timeout: Duration,
    ) -> Result<Self> {
        let (mut transport, child) = match config {
            TransportConfig::Stdio { command, args, env } => {
                let (mut transport, child) = StdioTransport::spawn(&command, &args, &env, cwd)?;
                transport.set_call_timeout(call_timeout);
                (Transport::Stdio(transport), Some(child))
            }
            TransportConfig::Http { url, headers } => {
                let mut transport = http::HttpTransport::new(&url, headers)?;
                transport.set_call_timeout(call_timeout);
                (Transport::Http(transport), None)
            }
        };
        let handshake = match handshake(transport.rpc()) {
            Ok(handshake) => handshake,
            Err(err) => {
                if let Some(mut child) = child {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(err);
            }
        };
        let child = child.map(|child| {
            let slot = Arc::new(Mutex::new(Some(child)));
            track_child(&slot);
            slot
        });
        Ok(Self {
            name: name.to_string(),
            transport,
            initialize: handshake.initialize,
            tools: handshake.tools,
            skipped: handshake.skipped,
            resources: handshake.resources,
            child,
        })
    }

    pub fn set_call_timeout(&mut self, timeout: Duration) {
        match &mut self.transport {
            Transport::Stdio(t) => t.set_call_timeout(timeout),
            Transport::Http(t) => t.set_call_timeout(timeout),
        }
    }

    /// The bounded tail of a stdio server's stderr; `None` over HTTP.
    pub fn stderr_tail(&self) -> Option<String> {
        match &self.transport {
            Transport::Stdio(t) => Some(t.stderr_tail()),
            Transport::Http(_) => None,
        }
    }

    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<CallToolResult> {
        let result = self.transport.rpc().call(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        serde_json::from_value(result).map_err(|err| Error::Protocol(format!("tools/call: {err}")))
    }

    pub fn read_resource(&mut self, uri: &str) -> Result<String> {
        let result = self
            .transport
            .rpc()
            .call("resources/read", json!({ "uri": uri }))?;
        contents_text(&result)
    }

    pub fn agent_tool_name(&self, tool: &str) -> String {
        format!("mcp__{}__{tool}", self.name)
    }
}

impl Transport {
    fn rpc(&mut self) -> &mut dyn Rpc {
        match self {
            Transport::Stdio(t) => t,
            Transport::Http(t) => t,
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(slot) = self.child.take() {
            kill_child(&slot);
        }
    }
}

trait Rpc {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    fn notify(&mut self, method: &str, params: Value) -> Result<()>;
}

struct Handshake {
    initialize: InitializeResult,
    tools: Vec<ToolSpec>,
    skipped: Vec<String>,
    resources: Vec<Resource>,
}

fn handshake(rpc: &mut dyn Rpc) -> Result<Handshake> {
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
    let (tools, skipped) = parse_tools(
        list_pages(rpc, "tools/list", "tools").map_err(|err| named("tools/list", err))?,
    );
    let resources = if declares(&initialize.capabilities, "resources") {
        match list_pages(rpc, "resources/list", "resources") {
            Ok(items) => parse_resources(items),
            // A server that names no `resources` capability may still refuse
            // the method; that is "no resources", not a broken server.
            Err(Error::Rpc { code: -32601, .. })
                if initialize.capabilities.get("resources").is_none() =>
            {
                Vec::new()
            }
            Err(err) => return Err(named("resources/list", err)),
        }
    } else {
        Vec::new()
    };
    Ok(Handshake {
        initialize,
        tools,
        skipped,
        resources,
    })
}

/// A capability is declared when the server names it, or when it sent no
/// capabilities object at all (older servers) — only an explicit set that
/// omits it says no.
fn declares(capabilities: &Value, name: &str) -> bool {
    match capabilities.as_object() {
        Some(map) if !map.is_empty() => map.contains_key(name),
        _ => true,
    }
}

/// A handshake failure names the method so the row reads
/// `error · tools/list: …`. Transport errors keep their class so the
/// caller still knows the server is gone.
fn named(method: &str, err: Error) -> Error {
    match err {
        Error::Transport(text) => Error::Transport(format!("{method}: {text}")),
        other => Error::Protocol(format!("{method}: {other}")),
    }
}

/// Every item of a paginated list, following `nextCursor` until the server
/// stops sending one.
fn list_pages(rpc: &mut dyn Rpc, method: &str, key: &str) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_LIST_PAGES {
        let params = match &cursor {
            Some(cursor) => json!({ "cursor": cursor }),
            None => json!({}),
        };
        let page = rpc.call(method, params)?;
        if let Some(list) = page.get(key).and_then(Value::as_array) {
            items.extend(list.iter().cloned());
        }
        let next = page
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|next| !next.is_empty())
            .map(str::to_string);
        if next.is_none() || next == cursor {
            return Ok(items);
        }
        cursor = next;
    }
    Err(Error::Protocol(format!(
        "{method}: more than {MAX_LIST_PAGES} pages"
    )))
}

/// Tools whose names can become agent tools, and the names of those that
/// cannot.
fn parse_tools(items: Vec<Value>) -> (Vec<ToolSpec>, Vec<String>) {
    let mut tools = Vec::new();
    let mut skipped = Vec::new();
    for item in items {
        let Ok(tool) = serde_json::from_value::<ToolSpec>(item) else {
            continue;
        };
        if is_ident(&tool.name) {
            tools.push(tool.normalize());
        } else {
            skipped.push(tool.name);
        }
    }
    (tools, skipped)
}

fn parse_resources(items: Vec<Value>) -> Vec<Resource> {
    items
        .into_iter()
        .filter_map(|resource| serde_json::from_value(resource).ok())
        .collect()
}

/// The text of a `resources/read` result. Binary contents become one
/// `[blob <mimeType>, N bytes]` line each so the model knows they exist.
fn contents_text(result: &Value) -> Result<String> {
    let contents = result
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = String::new();
    for item in contents {
        let line = if let Some(text) = item.get("text").and_then(Value::as_str) {
            text.to_string()
        } else if let Some(blob) = item.get("blob").and_then(Value::as_str) {
            format!(
                "[blob {}, {} bytes]",
                item.get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream"),
                base64_decoded_len(blob)
            )
        } else {
            continue;
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
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

    /// The in-tree fixture server. Cargo only builds a package's binaries
    /// for integration tests, so a unit test builds it once itself rather
    /// than trusting whatever `target/` last saw.
    fn fixture_exe() -> std::path::PathBuf {
        static BUILT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        BUILT
            .get_or_init(|| {
                for key in ["CARGO_BIN_EXE_mcp_fixture", "CARGO_BIN_EXE_mcp-fixture"] {
                    if let Ok(path) = std::env::var(key) {
                        return std::path::PathBuf::from(path);
                    }
                }
                let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
                let profile = if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                };
                let mut build = std::process::Command::new(cargo);
                build
                    .args(["build", "-p", "davinci-mcp", "--bin", "mcp-fixture", "--quiet"])
                    .current_dir(env!("CARGO_MANIFEST_DIR"));
                if profile == "release" {
                    build.arg("--release");
                }
                let status = build.status().expect("run cargo to build mcp-fixture");
                assert!(status.success(), "building mcp-fixture failed");
                let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                path.pop();
                path.pop();
                path.push("target");
                path.push(profile);
                path.push(format!("mcp-fixture{}", std::env::consts::EXE_SUFFIX));
                path
            })
            .clone()
    }

    fn stdio_config(args: &[&str]) -> TransportConfig {
        let exe = fixture_exe();
        assert!(exe.is_file(), "missing fixture bin at {}", exe.display());
        TransportConfig::Stdio {
            command: exe.to_string_lossy().into_owned(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: BTreeMap::new(),
        }
    }

    fn fixture_client(args: &[&str]) -> Client {
        Client::connect_with_timeout(
            "memory",
            stdio_config(args),
            Path::new("."),
            Duration::from_secs(5),
        )
        .expect("fixture")
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
        let mut client = fixture_client(&[]);
        assert_eq!(client.initialize.protocol_version, PROTOCOL_VERSION);
        assert_eq!(client.tools.len(), 1);
        assert_eq!(client.tools[0].name, "echo");
        assert!(client.tools[0].read_only());
        assert!(client.skipped.is_empty());
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
    fn an_rpc_error_is_reported_and_the_server_keeps_answering() {
        let mut client = fixture_client(&["--rpc-error"]);
        match client.call_tool("echo", json!({})) {
            Err(Error::Rpc { code, message }) => {
                assert_eq!(code, -32602);
                assert_eq!(message, "text is required");
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }
        let err = client.call_tool("echo", json!({})).unwrap_err();
        assert!(err.is_rpc());
        assert!(!err.drops_server());
        assert_eq!(err.to_string(), "mcp -32602: text is required");
        // The connection survived the error.
        let ok = client
            .call_tool("echo", json!({"text": "still here"}))
            .unwrap();
        assert_eq!(ok.text(), "still here");
    }

    #[test]
    fn stdout_log_lines_are_skipped() {
        let mut client = fixture_client(&["--log-stdout"]);
        assert_eq!(client.tools[0].name, "echo");
        let ok = client.call_tool("echo", json!({"text": "logged"})).unwrap();
        assert_eq!(ok.text(), "logged");
    }

    #[test]
    fn server_requests_are_refused_and_notifications_ignored() {
        let mut client = fixture_client(&["--sampling"]);
        // The fixture sends a notification and a `sampling/createMessage`
        // request whose id collides with ours before answering; it echoes
        // the error code our refusal carried.
        let ok = client.call_tool("echo", json!({"text": "x"})).unwrap();
        assert_eq!(ok.text(), "x refused:-32601");
    }

    #[test]
    fn a_hung_call_times_out_with_the_stderr_tail() {
        let mut client = Client::connect_with_timeout(
            "memory",
            stdio_config(&["--hang"]),
            Path::new("."),
            Duration::from_millis(600),
        )
        .expect("fixture");
        let started = std::time::Instant::now();
        let err = client.call_tool("echo", json!({"text": "x"})).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(err.drops_server());
        let text = err.to_string();
        assert!(text.contains("timed out"), "{text}");
        assert!(text.contains("hanging on purpose"), "{text}");
        assert!(client.stderr_tail().unwrap().contains("hanging on purpose"));
    }

    #[test]
    fn a_dying_server_quotes_its_last_stderr_lines() {
        let mut client = fixture_client(&["--die"]);
        let err = client.call_tool("echo", json!({"text": "x"})).unwrap_err();
        assert!(err.drops_server());
        let text = err.to_string();
        assert!(
            text.contains("closed stdout") || text.contains("stdin"),
            "{text}"
        );
        assert!(text.contains("fatal: boom"), "{text}");
    }

    #[test]
    fn lists_follow_next_cursor_and_name_skipped_tools() {
        let client = fixture_client(&["--paged"]);
        let names: Vec<&str> = client.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["echo", "second"]);
        assert_eq!(client.skipped, vec!["has space".to_string()]);
        // `second` came without an inputSchema; providers need an object.
        assert_eq!(client.tools[1].input_schema, default_input_schema());
        let uris: Vec<&str> = client.resources.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(uris, vec!["fixture://note", "fixture://second"]);
        assert_eq!(client.resources[1].mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn a_failed_list_fails_the_connection() {
        let err = Client::connect_with_timeout(
            "memory",
            stdio_config(&["--list-error"]),
            Path::new("."),
            Duration::from_secs(5),
        )
        .err()
        .expect("tools/list failure surfaces");
        let text = err.to_string();
        assert!(text.contains("tools/list"), "{text}");
        assert!(text.contains("list broke"), "{text}");
    }

    #[test]
    fn blob_resources_are_described_not_dropped() {
        let text = contents_text(&json!({
            "contents": [
                { "uri": "f://a", "text": "hello" },
                { "uri": "f://b", "mimeType": "image/png", "blob": "aGVsbG8=" },
                { "uri": "f://c" }
            ]
        }))
        .unwrap();
        assert_eq!(text, "hello\n[blob image/png, 5 bytes]");
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
        let mut client = Client::connect(
            "docs",
            TransportConfig::Http {
                url: format!("fixture:{}", path.display()),
                headers: BTreeMap::new(),
            },
            Path::new("."),
        )
        .expect("http fixture");
        assert_eq!(client.tools[0].name, "ping");
        assert_eq!(client.call_tool("ping", json!({})).unwrap().text(), "pong");
        assert!(client.stderr_tail().is_none());
    }

    #[test]
    fn a_server_without_the_resources_capability_is_not_asked_for_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        // No `resources/list` in the fixture: asking would fail the connect.
        std::fs::write(
            &path,
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"ping"}]}
            }"#,
        )
        .unwrap();
        let client = Client::connect(
            "docs",
            TransportConfig::Http {
                url: format!("fixture:{}", path.display()),
                headers: BTreeMap::new(),
            },
            Path::new("."),
        )
        .expect("http fixture");
        assert!(client.resources.is_empty());
        assert_eq!(client.tools[0].input_schema, default_input_schema());
    }
}
