//! Connected MCP servers the agent can call.
//!
//! No TypeScript counterpart. Phase 4 spec:
//! `docs/superpowers/specs/2026-09-01-native-mcp-design.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::permission::ToolClass;
use crate::runtime::{CapabilitySource, RuntimeCapability, RuntimeCapabilityRegistry};
use crate::tools::{AgentTool, ToolError, ToolResult};

mod sandboxed;

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
type SharedClient = Arc<ClientSlot>;

/// A client plus whether the registry has since dropped it. Callers clone the
/// handle before taking the client lock, so one that waited behind a failing
/// call must see the flag when it gets the lock: sending on an evicted client
/// would run a new call on a server the registry already gave up on.
///
/// `evicted` is only ever written while the client mutex is held, and a request
/// checks it in the same lock hold that sends. A caller therefore either sends
/// before the eviction or is refused after it; there is no gap between.
struct ClientSlot {
    client: Mutex<davinci_mcp::Client>,
    evicted: AtomicBool,
}

impl ClientSlot {
    fn new(client: davinci_mcp::Client) -> SharedClient {
        Arc::new(Self {
            client: Mutex::new(client),
            evicted: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, davinci_mcp::Client>> {
        self.client.lock()
    }

    fn is_evicted(&self) -> bool {
        self.evicted.load(Ordering::SeqCst)
    }

    /// Flag the slot after the registry dropped it. Waits for any request in
    /// flight, so call it with no registry lock held.
    fn mark_evicted(&self) {
        let _client = self.lock().unwrap_or_else(|err| err.into_inner());
        self.evicted.store(true, Ordering::SeqCst);
    }

    /// Run one request, unless the slot was evicted while the caller waited.
    /// Nothing is sent in that case, so the refusal is a definite failure. A
    /// transport or protocol failure ends the connection, and the flag is set
    /// before the lock is released so the next waiter cannot slip in.
    fn send<T>(
        &self,
        server: &str,
        request: impl FnOnce(&mut davinci_mcp::Client) -> davinci_mcp::Result<T>,
    ) -> Result<davinci_mcp::Result<T>, ToolError> {
        let mut client = self.lock().unwrap_or_else(|err| err.into_inner());
        if self.is_evicted() {
            return Err(ToolError::Failed(format!(
                "MCP server `{server}` was disconnected before this call was sent"
            )));
        }
        let result = request(&mut client);
        if matches!(&result, Err(err) if !err.is_rpc()) {
            self.evicted.store(true, Ordering::SeqCst);
        }
        Ok(result)
    }
}

#[derive(Default)]
struct Inner {
    clients: BTreeMap<String, SharedClient>,
    routes: BTreeMap<String, (String, String)>,
    trusted_read_only_servers: BTreeSet<String>,
    rows: Vec<McpServerRow>,
    /// Servers the project's own `mcp.json` configured, noted when the
    /// connect starts (so ones still connecting are covered too).
    project: BTreeSet<String>,
    /// Project servers stopped on purpose; a background connect that finishes
    /// later must not bring them back.
    blocked: BTreeSet<String>,
}

/// Connected MCP servers, shared with the tool thread and the shell.
#[derive(Clone, Default)]
pub struct McpRegistry {
    inner: Arc<Mutex<Inner>>,
    progress: Arc<Progress>,
    /// Test seam: runs after a failing call released its client lock and
    /// before the registry entry is removed, the window a racing call needs.
    #[cfg(test)]
    failure_hook: Arc<Mutex<Option<FailureHook>>>,
    /// Test seam: runs after a retiring slot was flagged and before its
    /// registry entry is removed or replaced, the window a stale caller needs.
    #[cfg(test)]
    retire_hook: Arc<Mutex<Option<FailureHook>>>,
}

#[cfg(test)]
type FailureHook = Arc<dyn Fn() + Send + Sync>;

/// Where a background connect stands: still running, or finished and not yet
/// reported to the shell.
#[derive(Default)]
struct Progress {
    running: AtomicBool,
    unreported: AtomicBool,
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
        Self::connect_inner(config, cwd, None)
    }

    pub fn connect_with_executor(
        config: &davinci_mcp::ConfigFile,
        cwd: &Path,
        host: &crate::jobs::supervisor::SupervisorCommand,
        sandbox: &davinci_protocol::SandboxSpec,
    ) -> Self {
        Self::connect_inner(config, cwd, Some((host, sandbox)))
    }

    fn connect_inner(
        config: &davinci_mcp::ConfigFile,
        cwd: &Path,
        executor: Option<(
            &crate::jobs::supervisor::SupervisorCommand,
            &davinci_protocol::SandboxSpec,
        )>,
    ) -> Self {
        let registry = Self::default();
        registry.note_project_servers(config);
        registry.adopt(Self::connect_parts(config, cwd, executor));
        registry
    }

    /// Start every enabled server on a thread of its own and return at once
    /// with an empty registry. The shell learns of the finished connect from
    /// [`take_ready`](Self::take_ready); until then no MCP tool exists. A
    /// slow `npx` handshake no longer holds up the first frame.
    pub fn connect_in_background(
        config: &davinci_mcp::ConfigFile,
        cwd: &Path,
        executor: Option<(
            crate::jobs::supervisor::SupervisorCommand,
            davinci_protocol::SandboxSpec,
        )>,
    ) -> Self {
        let registry = Self::default();
        if config.mcp_servers.is_empty() {
            return registry;
        }
        registry.note_project_servers(config);
        registry.progress.running.store(true, Ordering::SeqCst);
        let worker = registry.clone();
        let config = config.clone();
        let cwd: PathBuf = cwd.to_path_buf();
        std::thread::spawn(move || {
            let parts = Self::connect_parts(
                &config,
                &cwd,
                executor.as_ref().map(|(host, sandbox)| (host, sandbox)),
            );
            worker.adopt(parts);
            worker.progress.unreported.store(true, Ordering::SeqCst);
            worker.progress.running.store(false, Ordering::SeqCst);
        });
        registry
    }

    /// Whether a background connect is still running.
    pub fn connecting(&self) -> bool {
        self.progress.running.load(Ordering::SeqCst)
    }

    /// `true` once, after a background connect finishes: the caller refreshes
    /// whatever it derived from the registry's tools.
    pub fn take_ready(&self) -> bool {
        self.progress.unreported.swap(false, Ordering::SeqCst)
    }

    /// Servers start at the same time: each handshake can take seconds
    /// (`npx` fetches a package), and one server's state never touches
    /// another's, so the parts are built apart and joined in config order.
    fn connect_parts(
        config: &davinci_mcp::ConfigFile,
        cwd: &Path,
        executor: Option<(
            &crate::jobs::supervisor::SupervisorCommand,
            &davinci_protocol::SandboxSpec,
        )>,
    ) -> Vec<Inner> {
        let connect = |(name, server): (&String, &davinci_mcp::ServerConfig)| {
            let mut part = Inner::default();
            part.connect_one(name, server, cwd, executor);
            part
        };
        if config.mcp_servers.len() < 2 {
            return config.mcp_servers.iter().map(connect).collect();
        }
        std::thread::scope(|scope| {
            let workers: Vec<_> = config
                .mcp_servers
                .iter()
                .map(|entry| scope.spawn(move || connect(entry)))
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        })
    }

    /// Publish connected parts. A slot being replaced is retired first (see
    /// [`ClientSlot::mark_evicted`]), so a caller still holding the old handle
    /// is refused before the new one is visible. Retiring waits for requests in
    /// flight, so it runs with no registry lock held; the loop repeats if
    /// another thread published a live slot in the meantime.
    fn adopt(&self, parts: Vec<Inner>) {
        loop {
            let mut inner = self.lock();
            let live: Vec<SharedClient> = parts
                .iter()
                .filter(|part| !inner.blocks(part))
                .flat_map(|part| part.clients.keys())
                .filter_map(|name| inner.clients.get(name))
                .filter(|slot| !slot.is_evicted())
                .cloned()
                .collect();
            if live.is_empty() {
                inner.publish(parts);
                return;
            }
            drop(inner);
            for slot in &live {
                slot.mark_evicted();
            }
            #[cfg(test)]
            self.run_retire_hook();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    #[cfg(test)]
    fn run_retire_hook(&self) {
        let hook = self
            .retire_hook
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    #[cfg(test)]
    fn run_failure_hook(&self) {
        let hook = self
            .failure_hook
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn note_project_servers(&self, config: &davinci_mcp::ConfigFile) {
        let mut inner = self.lock();
        for (name, server) in &config.mcp_servers {
            if server.project_origin {
                inner.project.insert(name.clone());
            }
        }
    }

    /// Stop and forget every server whose origin is the project: their
    /// process ends, their tools and rows go, and a connect still running will
    /// not re-add them. A user or plugin server is never touched, whatever its
    /// name. Used when trust in the project is withdrawn. Returns how many
    /// connected servers were stopped.
    pub fn disconnect_project(&self) -> usize {
        let (names, stopped): (Vec<String>, Vec<SharedClient>) = {
            let mut inner = self.lock();
            let names: Vec<String> = inner.project.iter().cloned().collect();
            inner.blocked.extend(names.iter().cloned());
            let stopped = names
                .iter()
                .filter_map(|name| inner.clients.get(name).cloned())
                .collect();
            (names, stopped)
        };
        // Flag first, with no registry lock held (this waits for any request
        // in flight), remove second: a caller that cloned the handle in
        // between is refused by the flag. The names are blocked, so nothing
        // replaces these entries meanwhile.
        for slot in &stopped {
            slot.mark_evicted();
        }
        #[cfg(test)]
        self.run_retire_hook();
        let mut inner = self.lock();
        for name in &names {
            inner.clients.remove(name);
            inner.routes.retain(|_, (server, _)| server != name);
            inner.trusted_read_only_servers.remove(name);
            inner.rows.retain(|row| &row.name != name);
        }
        stopped.len()
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
                    .filter(|tool| {
                        inner.trusted_read_only_servers.contains(&client.name) && tool.read_only()
                    })
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

    /// Host-only metadata retained from the current authenticated handshake.
    pub(crate) fn output_schema(&self, canonical_name: &str) -> Option<Value> {
        let inner = self.lock();
        let (server, tool) = inner.routes.get(canonical_name)?;
        let client = inner
            .clients
            .get(server)?
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        client
            .tools
            .iter()
            .find(|spec| &spec.name == tool)?
            .output_schema
            .clone()
    }

    /// Describe every connected MCP tool using the shared runtime capability contract.
    pub fn capabilities(&self) -> Vec<RuntimeCapability> {
        let read_only = self.read_only_names();
        self.specs()
            .into_iter()
            .map(|tool| {
                let is_read_only = read_only.contains(&tool.name);
                // Resolve the namespace from the handshake route, never from
                // a model-supplied name or a substring during discovery.
                let family = self.resolve_tool(&tool.name).map(|(server, name)| {
                    crate::runtime::capabilities::registered_discovery_family(&name)
                        .or(match server.as_str() {
                            "git" | "github" | "gitlab" => Some("git"),
                            "browser" | "playwright" | "puppeteer" => Some("browser"),
                            "lsp" | "language-server" => Some("lsp"),
                            "package" => Some("package"),
                            "build" => Some("build"),
                            "process" => Some("process"),
                            "graph" => Some("graph"),
                            "sec" | "security" => Some("sec"),
                            _ => None,
                        })
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("mcp:{server}"))
                });
                RuntimeCapability::new(
                    tool.name,
                    CapabilitySource::Mcp,
                    if is_read_only {
                        ToolClass::Read
                    } else {
                        ToolClass::Other
                    },
                    is_read_only,
                    &tool.parameters,
                    None,
                )
                .with_declared_effects(vec![if is_read_only {
                    crate::runtime::DeclaredEffect::McpRead
                } else {
                    crate::runtime::DeclaredEffect::McpMutation
                }])
                .with_description(tool.description)
                .with_family(family.unwrap_or_default())
            })
            .collect()
    }

    /// Register the current MCP tool set with the shared runtime registry.
    pub fn register_with(&self, registry: &RuntimeCapabilityRegistry) {
        registry.register_all(self.capabilities());
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

    /// Resolve one provider-visible MCP function name to the original server
    /// and tool names captured at handshake time.
    pub fn resolve_tool(&self, exposed: &str) -> Option<(String, String)> {
        self.lock().routes.get(exposed).cloned()
    }

    /// `invocation_id` is the tool-call id of this logical call; it always
    /// rides in the request's `_meta` so a server can tell a repeat from a new
    /// call.
    pub fn call_exposed(
        &self,
        exposed: &str,
        arguments: &Value,
        invocation_id: &str,
    ) -> Result<ToolResult, ToolError> {
        let Some((server, tool)) = self.resolve_tool(exposed) else {
            return Err(ToolError::Unknown(exposed.to_string()));
        };
        let result = self.call_full(&server, &tool, arguments, invocation_id)?;
        Ok(ToolResult {
            content: result.text(),
            is_error: result.is_error.unwrap_or(false),
            details: None,
        })
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
        invocation_id: &str,
    ) -> Result<ToolResult, ToolError> {
        let result = self.call_full(server, tool, arguments, invocation_id)?;
        Ok(ToolResult {
            content: result.text(),
            is_error: result.is_error.unwrap_or(false),
            details: None,
        })
    }

    /// Host-owned data seam for programmatic consumers. Callers still have to
    /// enter through the ordinary guarded dispatcher; this is not authority.
    /// Server metadata never becomes ToolResult.details or a host receipt.
    pub(crate) fn call_full(
        &self,
        server: &str,
        tool: &str,
        arguments: &Value,
        invocation_id: &str,
    ) -> Result<davinci_mcp::CallToolResult, ToolError> {
        let slot = self.client(server)?;
        self.call_on(server, &slot, tool, arguments, invocation_id)
    }

    /// The call itself, on a handle fetched earlier. Split out so the window
    /// between fetching a handle and locking it can be tested.
    fn call_on(
        &self,
        server: &str,
        slot: &SharedClient,
        tool: &str,
        arguments: &Value,
        invocation_id: &str,
    ) -> Result<davinci_mcp::CallToolResult, ToolError> {
        let result = slot.send(server, |client| {
            client.call_tool_with_invocation(tool, arguments.clone(), Some(invocation_id))
        })?;
        match result {
            Ok(result) => Ok(result),
            Err(err) => {
                // Preserve the direct path's distinction: an RPC error is a
                // failed tool answer; transport/protocol failure drops the
                // server and remains an error, not an empty structured value.
                #[cfg(test)]
                self.run_failure_hook();
                let result = self.lock().failed(server, slot, err)?;
                Ok(davinci_mcp::CallToolResult {
                    content: vec![davinci_mcp::ContentBlock {
                        kind: "text".into(),
                        text: Some(result.content),
                        mime_type: None,
                        data: None,
                        resource: None,
                    }],
                    structured_content: None,
                    is_error: Some(result.is_error),
                })
            }
        }
    }

    pub fn read(&self, server: &str, uri: &str) -> Result<ToolResult, ToolError> {
        let slot = self.client(server)?;
        self.read_on(server, &slot, uri)
    }

    /// The resource read itself, on a handle fetched earlier.
    fn read_on(
        &self,
        server: &str,
        slot: &SharedClient,
        uri: &str,
    ) -> Result<ToolResult, ToolError> {
        let result = slot.send(server, |client| client.read_resource(uri))?;
        match result {
            Ok(text) => Ok(ToolResult {
                content: text,
                is_error: false,
                details: None,
            }),
            Err(err) => self.lock().failed(server, slot, err),
        }
    }
}

impl Inner {
    /// A JSON-RPC error reply is the server's answer to this one call:
    /// `is_error` on the result, server kept. Anything else (closed pipe,
    /// timeout, decode) means the server is gone: it is dropped and its row
    /// turns into an error.
    fn failed(
        &mut self,
        server: &str,
        slot: &SharedClient,
        err: davinci_mcp::Error,
    ) -> Result<ToolResult, ToolError> {
        if err.is_rpc() {
            return Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
                details: None,
            });
        }
        // Only drop the server this call used: a newer connection under the
        // same name is not the one that failed. The slot is already flagged
        // (see `ClientSlot::send`), so its flag says nothing about this.
        if self
            .clients
            .get(server)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            self.drop_server(server, err.to_string());
        }
        if err.may_have_run() {
            return Err(ToolError::MayHaveRun(err.to_string()));
        }
        Err(ToolError::Failed(err.to_string()))
    }

    fn connect_one(
        &mut self,
        name: &str,
        server: &davinci_mcp::ServerConfig,
        cwd: &Path,
        executor: Option<(
            &crate::jobs::supervisor::SupervisorCommand,
            &davinci_protocol::SandboxSpec,
        )>,
    ) {
        self.trusted_read_only_servers.remove(name);
        let transport_label = if server.url.is_some() {
            "http"
        } else {
            "stdio"
        };
        if let Err(error) = davinci_mcp::validate_server_name(name) {
            self.rows.push(McpServerRow {
                name: name.to_string(),
                transport: transport_label.into(),
                status: "error".into(),
                tools: 0,
                error: Some(error),
                skipped: Vec::new(),
            });
            return;
        }
        // Route on the resolved policy, never the raw field: an omitted
        // `execution` on a local command means Sandboxed, and an invalid
        // transport/policy pair must be refused before anything is spawned.
        let policy = match server.execution_policy() {
            Ok(policy) => policy,
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
        if policy == davinci_mcp::McpExecutionPolicy::Disabled {
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
        if policy == davinci_mcp::McpExecutionPolicy::Sandboxed {
            let Some((host, sandbox)) = executor else {
                self.rows.push(McpServerRow {
                    name: name.to_string(),
                    transport: "sandboxed-stdio".into(),
                    status: "error".into(),
                    tools: 0,
                    error: Some(
                        "sandboxed local MCP requires the executor transport; refusing direct host spawn"
                            .into(),
                    ),
                    skipped: Vec::new(),
                });
                return;
            };
            let client = sandboxed::SupervisedMcpTransport::spawn(host, server, cwd, sandbox)
                .and_then(|transport| {
                    davinci_mcp::Client::connect_transport(name, Box::new(transport))
                });
            self.finish_connection(name, server, "sandboxed-stdio", client);
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
        let client = davinci_mcp::Client::connect(name, transport, cwd);
        self.finish_connection(name, server, transport_label, client);
    }

    fn finish_connection(
        &mut self,
        name: &str,
        server: &davinci_mcp::ServerConfig,
        transport_label: &str,
        client: davinci_mcp::Result<davinci_mcp::Client>,
    ) {
        match client {
            Ok(client) => {
                let tools = client.tools.len();
                let skipped = client.skipped.clone();
                for tool in &client.tools {
                    self.routes.insert(
                        client.agent_tool_name(&tool.name),
                        (name.to_string(), tool.name.clone()),
                    );
                }
                if server.trust_read_only_hints {
                    self.trusted_read_only_servers.insert(name.to_string());
                }
                self.clients
                    .insert(name.to_string(), ClientSlot::new(client));
                self.rows.push(McpServerRow {
                    name: name.to_string(),
                    transport: transport_label.into(),
                    status: "connected".into(),
                    tools,
                    error: None,
                    skipped,
                });
            }
            Err(err) => self.rows.push(McpServerRow {
                name: name.to_string(),
                transport: transport_label.into(),
                status: "error".into(),
                tools: 0,
                error: Some(err.to_string()),
                skipped: Vec::new(),
            }),
        }
    }

    /// Whether a stopped project server's name forbids publishing `part`.
    fn blocks(&self, part: &Inner) -> bool {
        part.clients.keys().any(|name| self.blocked.contains(name))
            || part.rows.iter().any(|row| self.blocked.contains(&row.name))
    }

    fn publish(&mut self, parts: Vec<Inner>) {
        for part in parts {
            if self.blocks(&part) {
                continue;
            }
            self.clients.extend(part.clients);
            self.routes.extend(part.routes);
            self.trusted_read_only_servers
                .extend(part.trusted_read_only_servers);
            self.rows.extend(part.rows);
        }
    }

    /// Only reached for a slot `ClientSlot::send` already flagged under the
    /// client lock, so removing the entry cannot expose an unflagged slot.
    fn drop_server(&mut self, name: &str, error: String) {
        self.clients.remove(name);
        self.routes.retain(|_, (server, _)| server != name);
        self.trusted_read_only_servers.remove(name);
        if let Some(row) = self.rows.iter_mut().find(|row| row.name == name) {
            row.status = "error".into();
            row.tools = 0;
            row.error = Some(error);
        }
    }
}

#[cfg(test)]
impl McpRegistry {
    /// A registry holding one connected server over `transport`, for tests.
    pub(crate) fn with_test_transport(
        name: &str,
        transport: Box<dyn davinci_mcp::RpcTransport>,
    ) -> Self {
        let client = davinci_mcp::Client::connect_transport(name, transport).unwrap();
        let registry = Self::default();
        {
            let mut inner = registry.lock();
            for tool in &client.tools {
                inner.routes.insert(
                    client.agent_tool_name(&tool.name),
                    (name.to_string(), tool.name.clone()),
                );
            }
            inner.rows.push(McpServerRow {
                name: name.to_string(),
                transport: "test".into(),
                status: "connected".into(),
                tools: client.tools.len(),
                error: None,
                skipped: Vec::new(),
            });
            inner
                .clients
                .insert(name.to_string(), ClientSlot::new(client));
        }
        registry
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    /// Serves one non-read-only tool, `effect`, and records the params of every
    /// `tools/call`. `lose_reply` makes each call fail after the (pretend)
    /// effect, the way a dropped connection does.
    pub(crate) struct RecordingTransport {
        pub(crate) calls: Arc<Mutex<Vec<Value>>>,
        pub(crate) lose_reply: bool,
    }

    impl davinci_mcp::RpcTransport for RecordingTransport {
        fn call(&mut self, method: &str, params: Value) -> davinci_mcp::Result<Value> {
            match method {
                "initialize" => Ok(json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}}
                })),
                "tools/list" => Ok(json!({
                    "tools": [{"name": "effect", "inputSchema": {"type": "object"}}]
                })),
                "tools/call" => {
                    self.calls.lock().unwrap().push(params);
                    if self.lose_reply {
                        Err(davinci_mcp::Error::Transport("connection reset".into()))
                    } else {
                        Ok(json!({"content": [{"type": "text", "text": "done"}]}))
                    }
                }
                other => Err(davinci_mcp::Error::Protocol(format!("unexpected {other}"))),
            }
        }

        fn notify(&mut self, _: &str, _: Value) -> davinci_mcp::Result<()> {
            Ok(())
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
    fn a_background_connect_reports_ready_once_and_then_has_its_tools() {
        let (dir, config) = http_fixture(
            r#"{
            "initialize":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"0"}},
            "tools/list":{"tools":[{"name":"ping","inputSchema":{"type":"object"}}]}
        }"#,
        );
        let registry = McpRegistry::connect_in_background(&config, dir.path(), None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !registry.take_ready() {
            assert!(std::time::Instant::now() < deadline, "never reported ready");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!registry.connecting());
        assert!(!registry.take_ready(), "ready is reported once");
        assert_eq!(registry.tool_names(), ["mcp__memory__ping"]);
        assert_eq!(registry.rows()[0].status, "connected");
        // No servers configured: nothing to wait for, nothing to report.
        let empty = McpRegistry::connect_in_background(
            &davinci_mcp::ConfigFile::default(),
            dir.path(),
            None,
        );
        assert!(!empty.connecting() && !empty.take_ready());
    }

    fn fixture_with_two_servers(project: &[&str]) -> (tempfile::TempDir, davinci_mcp::ConfigFile) {
        let (dir, mut config) = http_fixture(
            r#"{
            "initialize":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"0"}},
            "tools/list":{"tools":[{"name":"ping","inputSchema":{"type":"object"}}]}
        }"#,
        );
        let template = config.mcp_servers["memory"].clone();
        config.mcp_servers.insert("user".into(), template);
        for (name, server) in config.mcp_servers.iter_mut() {
            server.project_origin = project.contains(&name.as_str());
        }
        (dir, config)
    }

    #[test]
    fn disconnect_project_stops_only_project_origin_servers() {
        let (dir, config) = fixture_with_two_servers(&["memory"]);
        let registry = McpRegistry::connect(&config, dir.path());
        assert_eq!(
            registry.tool_names(),
            ["mcp__memory__ping", "mcp__user__ping"]
        );
        assert_eq!(registry.disconnect_project(), 1);
        assert_eq!(registry.tool_names(), ["mcp__user__ping"]);
        assert_eq!(registry.rows().len(), 1);
        assert!(registry.resolve_tool("mcp__memory__ping").is_none());
        // A connect still in flight lands afterwards: the project server is
        // dropped, the user one is not.
        let late = McpRegistry::connect_parts(&config, dir.path(), None);
        registry.adopt(late);
        assert_eq!(registry.tool_names(), ["mcp__user__ping"]);
    }

    #[test]
    fn a_user_server_sharing_a_name_with_an_unloaded_project_entry_is_never_stopped() {
        // Nothing is project-origin: the project's mcp.json was not loaded
        // (untrusted at startup), so every connected server is the user's.
        let (dir, config) = fixture_with_two_servers(&[]);
        let registry = McpRegistry::connect(&config, dir.path());
        assert_eq!(registry.disconnect_project(), 0);
        assert_eq!(
            registry.tool_names(),
            ["mcp__memory__ping", "mcp__user__ping"]
        );
    }

    #[test]
    fn disconnect_project_covers_servers_still_connecting() {
        let (dir, config) = fixture_with_two_servers(&["memory"]);
        let registry = McpRegistry::connect_in_background(&config, dir.path(), None);
        registry.disconnect_project();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !registry.take_ready() {
            assert!(std::time::Instant::now() < deadline, "never reported ready");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(registry.tool_names(), ["mcp__user__ping"]);
    }

    #[test]
    fn several_servers_connect_together_and_keep_config_order() {
        let dir = tempfile::tempdir().unwrap();
        let body = |tool: &str| {
            json!({
                "initialize": {"protocolVersion": "2025-03-26", "capabilities": {"tools": {}},
                    "serverInfo": {"name": "f", "version": "0"}},
                "tools/list": {"tools": [{"name": tool, "inputSchema": {"type": "object"}}]}
            })
            .to_string()
        };
        let mut servers = Vec::new();
        for (name, tool) in [("alpha", "one"), ("beta", "two"), ("gamma", "three")] {
            let path = dir.path().join(format!("{name}.json"));
            // A server whose script is missing must fail on its own row.
            if name != "beta" {
                std::fs::write(&path, body(tool)).unwrap();
            }
            let url = format!("fixture:{}", path.display());
            servers.push(format!(
                "{:?}:{{\"url\":{}}}",
                name,
                serde_json::to_string(&url).unwrap()
            ));
        }
        let config =
            davinci_mcp::parse_config(&format!(r#"{{"mcpServers":{{{}}}}}"#, servers.join(",")))
                .unwrap();
        let registry = McpRegistry::connect(&config, dir.path());
        let rows = registry.rows();
        let seen: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.status.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("alpha", "connected"),
                ("beta", "error"),
                ("gamma", "connected")
            ]
        );
        let mut tools = registry.tool_names();
        tools.sort();
        assert_eq!(tools, ["mcp__alpha__one", "mcp__gamma__three"]);
    }

    #[test]
    fn security_mcp_remote_hints_require_local_trust() {
        let (dir, config) = http_fixture(
            r#"{
            "initialize":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"0"}},
            "tools/list":{"tools":[{"name":"delete_records","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}
        }"#,
        );
        let registry = McpRegistry::connect(&config, dir.path());
        assert_eq!(registry.specs().len(), 1);
        assert!(
            registry.read_only_names().is_empty(),
            "remote hint became local authorization"
        );
        assert_eq!(registry.capabilities()[0].tool_class, ToolClass::Other);
        assert!(!registry.capabilities()[0].read_only);
        assert_eq!(
            registry.capabilities()[0].declared_effects,
            vec![crate::runtime::DeclaredEffect::McpMutation]
        );
        use crate::permission::{
            PermissionMode, PermissionPolicy, PermissionRule, PermissionVerdict,
        };
        let name = "mcp__memory__delete_records";
        for (mode, plan_mode) in [
            (PermissionMode::ReadOnly, false),
            (PermissionMode::Ask, false),
            (PermissionMode::Auto, true),
        ] {
            let mut policy = PermissionPolicy::new(mode);
            policy.mcp_read_only = registry.read_only_names();
            if plan_mode {
                policy.mode = PermissionMode::ReadOnly;
            }
            let decision = policy.decide("fixture", name, &json!({}), dir.path());
            if matches!(mode, PermissionMode::Ask) {
                assert!(matches!(decision, PermissionVerdict::Ask(_)));
            } else {
                assert!(matches!(decision, PermissionVerdict::Deny { .. }));
            }
        }
        let mut trusted_config = config.clone();
        trusted_config
            .mcp_servers
            .get_mut("memory")
            .unwrap()
            .trust_read_only_hints = true;
        let trusted = McpRegistry::connect(&trusted_config, dir.path());
        let mut policy = PermissionPolicy::new(PermissionMode::ReadOnly);
        policy.mcp_read_only = trusted.read_only_names();
        assert!(matches!(
            policy.decide("fixture", name, &json!({}), dir.path()),
            PermissionVerdict::Allow
        ));
        policy.deny = vec![PermissionRule::parse("mcp__memory__*").unwrap()];
        assert!(matches!(
            policy.decide("fixture", name, &json!({}), dir.path()),
            PermissionVerdict::Deny { .. }
        ));
    }

    #[test]
    fn a_fixture_server_becomes_one_agent_tool() {
        let (_dir, mut config) = http_fixture(
            r#"{
              "initialize": {"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"f","version":"0"}},
              "tools/list": {"tools":[{"name":"echo","description":"echo text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}},"annotations":{"readOnlyHint":true}}]},
              "tools/call": {"content":[{"type":"text","text":"hi"}]},
              "resources/list": {"resources":[{"uri":"fixture://note","name":"note"}]},
              "resources/read": {"contents":[{"uri":"fixture://note","text":"a note"}]}
            }"#,
        );
        config
            .mcp_servers
            .get_mut("memory")
            .unwrap()
            .trust_read_only_hints = true;
        let registry = McpRegistry::connect(&config, Path::new("."));
        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "mcp__memory__echo");
        assert!(specs[0].description.starts_with("mcp:memory."));
        assert!(registry.read_only_names().contains("mcp__memory__echo"));
        let capabilities = registry.capabilities();
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].name, "mcp__memory__echo");
        assert_eq!(capabilities[0].source, crate::CapabilitySource::Mcp);
        assert!(capabilities[0].read_only);
        assert_eq!(
            capabilities[0].declared_effects,
            vec![crate::runtime::DeclaredEffect::McpRead]
        );
        assert_eq!(capabilities[0].tool_class, crate::ToolClass::Read);
        let runtime_registry = crate::RuntimeCapabilityRegistry::new();
        registry.register_with(&runtime_registry);
        assert!(runtime_registry.is_read_only("mcp__memory__echo"));
        let result = registry
            .call("memory", "echo", &json!({"text": "hi"}), "call-1")
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
        agent.attach_mcp(registry.clone());
        assert!(agent.tools.iter().any(|name| name == "mcp__memory__echo"));
        let specs = agent.builtin_and_mcp_specs();
        assert!(specs.iter().any(|tool| tool.name == "mcp__memory__echo"));
        let mut context = agent.tool_context.clone();
        let refused = crate::execute_tool_with(
            Path::new("."),
            "mcp__memory__echo",
            &json!({"text": "hi"}),
            &context,
        )
        .unwrap_err();
        assert!(
            matches!(&refused, ToolError::Failed(m) if m.contains("needs a tool-call id")),
            "a call with no tool-call id must not be sent: {refused:?}"
        );
        context.tool_call_id = Some("call-1".into());
        let result = crate::execute_tool_with(
            Path::new("."),
            "mcp__memory__echo",
            &json!({"text": "hi"}),
            &context,
        )
        .unwrap();
        assert_eq!(result.content, "hi");
        assert_eq!(
            registry.resolve_tool("mcp__memory__echo"),
            Some(("memory".to_string(), "echo".to_string()))
        );
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
    fn shortened_tool_names_resolve_to_the_original_tool() {
        let long = "x".repeat(80);
        let (_dir, config) = http_fixture(&format!(
            r#"{{
              "initialize": {{"protocolVersion":"2025-03-26","capabilities":{{}},"serverInfo":{{"name":"f","version":"0"}}}},
              "tools/list": {{"tools":[{{"name":{},"inputSchema":{{"type":"object"}}}}]}},
              "tools/call": {{"content":[{{"type":"text","text":"ok"}}]}},
              "resources/list": {{"resources":[]}}
            }}"#,
            serde_json::to_string(&long).unwrap()
        ));
        let registry = McpRegistry::connect(&config, Path::new("."));
        let exposed = registry.tool_names().into_iter().next().unwrap();
        assert!(exposed.len() <= 64, "{exposed}");
        assert_eq!(
            registry.resolve_tool(&exposed),
            Some(("memory".to_string(), long.clone()))
        );
        assert_eq!(
            registry
                .call_exposed(&exposed, &json!({}), "call-1")
                .unwrap()
                .content,
            "ok"
        );
    }

    #[test]
    fn ambiguous_server_names_become_error_rows() {
        let config = davinci_mcp::parse_config(
            r#"{"mcpServers":{"my__srv":{"command":"does-not-run"},"a_":{"command":"does-not-run"}}}"#
        ).unwrap();
        let registry = McpRegistry::connect(&config, Path::new("."));
        let rows = registry.rows();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.status == "error"));
        assert!(rows.iter().all(|row| row
            .error
            .as_deref()
            .is_some_and(|e| e.contains("server name"))));
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
            .call("memory", "echo", &json!({}), "call-1")
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
    fn sandboxed_local_mcp_never_falls_back_to_direct_host_stdio() {
        let config = davinci_mcp::parse_config(
            r#"{"mcpServers":{"project":{"command":"does-not-run","execution":"sandboxed"}}}"#,
        )
        .unwrap();
        let registry = McpRegistry::connect(&config, Path::new("."));
        assert!(registry.specs().is_empty());
        let rows = registry.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].transport, "sandboxed-stdio");
        assert_eq!(rows[0].status, "error");
        assert!(rows[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("refusing direct host spawn")));
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

    struct LostReply;

    impl davinci_mcp::RpcTransport for LostReply {
        fn call(&mut self, method: &str, _: Value) -> davinci_mcp::Result<Value> {
            match method {
                "initialize" => Ok(json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}}
                })),
                "tools/list" => Ok(json!({
                    "tools": [{"name": "effect", "inputSchema": {"type": "object"}}]
                })),
                "tools/call" => Err(davinci_mcp::Error::Transport(
                    "the reply was lost after the effect".into(),
                )),
                other => Err(davinci_mcp::Error::Protocol(format!("unexpected {other}"))),
            }
        }

        fn notify(&mut self, _: &str, _: Value) -> davinci_mcp::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_lost_mcp_reply_becomes_may_have_run_not_failed() {
        let client = davinci_mcp::Client::connect_transport("lost", Box::new(LostReply)).unwrap();
        let registry = McpRegistry::default();
        {
            let mut inner = registry.lock();
            inner.clients.insert("lost".into(), ClientSlot::new(client));
            inner
                .routes
                .insert("mcp__lost__effect".into(), ("lost".into(), "effect".into()));
        }

        let error = registry
            .call_exposed("mcp__lost__effect", &json!({}), "call-1")
            .unwrap_err();
        match &error {
            ToolError::MayHaveRun(message) => {
                assert!(message.contains("reply was lost"), "{message}")
            }
            other => panic!("a lost reply must be an unknown outcome, got {other:?}"),
        }

        // The server and its route are still evicted: a later call cannot
        // even resolve, let alone silently re-run the effect.
        let later = registry
            .call_exposed("mcp__lost__effect", &json!({}), "call-2")
            .unwrap_err();
        assert!(matches!(later, ToolError::Unknown(_)), "{later:?}");
    }

    fn recording_registry(lose_reply: bool) -> (McpRegistry, Arc<Mutex<Vec<Value>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let registry = McpRegistry::with_test_transport(
            "rec",
            Box::new(test_support::RecordingTransport {
                calls: calls.clone(),
                lose_reply,
            }),
        );
        (registry, calls)
    }

    fn refused_before_sending(error: &ToolError) -> bool {
        matches!(error, ToolError::Failed(m) if m.contains("before this call was sent"))
    }

    #[test]
    fn g01_a_call_waiting_on_an_evicted_client_is_not_sent() {
        let (registry, calls) = recording_registry(true);
        // The second caller fetched its handle before the first call failed.
        let waiting = registry.client("rec").unwrap();

        let first = registry
            .call_exposed("mcp__rec__effect", &json!({}), "call-1")
            .unwrap_err();
        assert!(matches!(first, ToolError::MayHaveRun(_)), "{first:?}");
        assert_eq!(calls.lock().unwrap().len(), 1);

        let second = registry
            .call_on("rec", &waiting, "effect", &json!({}), "call-2")
            .unwrap_err();
        assert!(refused_before_sending(&second), "{second:?}");
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "the evicted client must not reach the transport again"
        );
    }

    #[test]
    fn g01_a_call_arriving_after_a_failure_but_before_the_registry_evicts_is_not_sent() {
        let (registry, calls) = recording_registry(true);
        let waiting = registry.client("rec").unwrap();
        // Park the failing call after it released the client lock and before it
        // takes the registry lock: the window the old code left open.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let go_rx = Mutex::new(go_rx);
        let parked = AtomicBool::new(false);
        *registry.failure_hook.lock().unwrap() = Some(Arc::new(move || {
            // Only the first failure parks, so a wrongly sent second call fails
            // the assertions below instead of hanging the test.
            if !parked.swap(true, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = go_rx.lock().unwrap().recv();
            }
        }));
        let first = {
            let registry = registry.clone();
            std::thread::spawn(move || {
                registry.call_exposed("mcp__rec__effect", &json!({}), "call-1")
            })
        };
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the first call never failed");
        assert!(
            registry.client("rec").is_ok(),
            "precondition: the registry still lists the failed client"
        );

        let second = registry
            .call_on("rec", &waiting, "effect", &json!({}), "call-2")
            .unwrap_err();
        assert!(refused_before_sending(&second), "{second:?}");
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "the waiting call must not reach the dead transport"
        );

        go_tx.send(()).unwrap();
        let first = first.join().unwrap().unwrap_err();
        assert!(matches!(first, ToolError::MayHaveRun(_)), "{first:?}");
        assert!(registry.client("rec").is_err(), "then the entry is removed");
    }

    /// A transport whose `tools/call` parks until released, so a test can hold
    /// a request in flight while other threads act.
    struct Gated {
        entered: Mutex<std::sync::mpsc::Sender<()>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        sent: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl davinci_mcp::RpcTransport for Gated {
        fn call(&mut self, method: &str, _: Value) -> davinci_mcp::Result<Value> {
            match method {
                "initialize" => Ok(json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}}
                })),
                "tools/list" => Ok(json!({
                    "tools": [{"name": "effect", "inputSchema": {"type": "object"}}]
                })),
                "tools/call" => {
                    self.sent.fetch_add(1, Ordering::SeqCst);
                    self.entered.lock().unwrap().send(()).unwrap();
                    let _ = self.release.lock().unwrap().recv();
                    Ok(json!({"content": [{"type": "text", "text": "done"}]}))
                }
                other => Err(davinci_mcp::Error::Protocol(format!("unexpected {other}"))),
            }
        }

        fn notify(&mut self, _: &str, _: Value) -> davinci_mcp::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn g01_external_eviction_waits_for_the_in_flight_call_and_never_holds_the_registry_lock() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let registry = McpRegistry::with_test_transport(
            "rec",
            Box::new(Gated {
                entered: Mutex::new(entered_tx),
                release: Mutex::new(release_rx),
                sent: sent.clone(),
            }),
        );
        registry.lock().project.insert("rec".into());
        let waiting = registry.client("rec").unwrap();

        let in_flight = {
            let registry = registry.clone();
            std::thread::spawn(move || {
                registry.call_exposed("mcp__rec__effect", &json!({}), "call-1")
            })
        };
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the first call never reached the transport");

        let disconnect = {
            let registry = registry.clone();
            std::thread::spawn(move || registry.disconnect_project())
        };
        // The flag is written first, under the client lock, so the disconnect
        // waits for the in-flight call and the entry stays listed meanwhile.
        // Reading the registry here proves its lock is not held while waiting.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(registry.rows().len(), 1);
        assert!(registry.client("rec").is_ok());
        assert!(!disconnect.is_finished());

        release_tx.send(()).unwrap();
        assert_eq!(in_flight.join().unwrap().unwrap().content, "done");
        assert_eq!(disconnect.join().unwrap(), 1);
        assert!(registry.client("rec").is_err());
        assert!(registry.rows().is_empty());

        let late = registry
            .call_on("rec", &waiting, "effect", &json!({}), "call-2")
            .unwrap_err();
        assert!(refused_before_sending(&late), "{late:?}");
        assert_eq!(sent.load(Ordering::SeqCst), 1, "the late call was not sent");
    }

    #[test]
    fn g01_adopting_a_replacement_flags_the_replaced_connection() {
        let (registry, calls) = recording_registry(false);
        let replaced = registry.client("rec").unwrap();
        let mut part = Inner::default();
        let (fresh, _) = recording_registry(false);
        part.clients
            .insert("rec".into(), fresh.client("rec").unwrap());
        registry.adopt(vec![part]);

        let late = registry
            .call_on("rec", &replaced, "effect", &json!({}), "call-1")
            .unwrap_err();
        assert!(refused_before_sending(&late), "{late:?}");
        assert!(calls.lock().unwrap().is_empty());
    }

    /// Run `retire` with the hook set to make a stale caller (holding `stale`)
    /// try a call in the window after the old slot was flagged and before the
    /// registry entry changes. Returns the stale call's outcome.
    fn stale_call_during_retire(
        registry: &McpRegistry,
        stale: &SharedClient,
        retire: impl FnOnce(),
    ) -> ToolError {
        let outcome = Arc::new(Mutex::new(None));
        let hook_outcome = outcome.clone();
        let hooked = registry.clone();
        let stale = stale.clone();
        *registry.retire_hook.lock().unwrap() = Some(Arc::new(move || {
            assert!(
                hooked.client("rec").is_ok(),
                "precondition: the registry entry is still in place"
            );
            let result = hooked.call_on("rec", &stale, "effect", &json!({}), "stale-call");
            *hook_outcome.lock().unwrap() = Some(result);
        }));
        retire();
        let outcome = outcome.lock().unwrap().take();
        outcome
            .expect("the retire hook never ran")
            .expect_err("the stale call must be refused")
    }

    #[test]
    fn g01_a_stale_caller_cannot_send_while_a_project_disconnect_is_retiring_the_slot() {
        let (registry, calls) = recording_registry(false);
        registry.lock().project.insert("rec".into());
        let stale = registry.client("rec").unwrap();

        let error = stale_call_during_retire(&registry, &stale, || {
            assert_eq!(registry.disconnect_project(), 1);
        });
        assert!(refused_before_sending(&error), "{error:?}");
        assert!(calls.lock().unwrap().is_empty());
        assert!(registry.client("rec").is_err());
    }

    #[test]
    fn g01_a_stale_caller_cannot_send_while_an_adopt_is_retiring_the_slot() {
        let (registry, calls) = recording_registry(false);
        let stale = registry.client("rec").unwrap();
        let (fresh, fresh_calls) = recording_registry(false);
        let mut part = Inner::default();
        part.clients
            .insert("rec".into(), fresh.client("rec").unwrap());

        let error = stale_call_during_retire(&registry, &stale, || registry.adopt(vec![part]));
        assert!(refused_before_sending(&error), "{error:?}");
        assert!(calls.lock().unwrap().is_empty());
        assert!(fresh_calls.lock().unwrap().is_empty());
        assert!(
            Arc::ptr_eq(
                &registry.client("rec").unwrap(),
                &fresh.client("rec").unwrap()
            ),
            "the replacement is published afterwards"
        );
    }

    #[test]
    fn g01_a_resource_read_on_an_evicted_client_is_not_sent() {
        let (registry, _calls) = recording_registry(false);
        let slot = registry.client("rec").unwrap();
        slot.mark_evicted();
        // The recording transport fails any other method with "unexpected ...",
        // so reaching it would show as that message instead of the refusal.
        let error = registry
            .read_on("rec", &slot, "fixture://note")
            .unwrap_err();
        assert!(refused_before_sending(&error), "{error:?}");
    }

    #[test]
    fn g01_a_stale_failure_does_not_drop_a_newer_connection() {
        let (registry, _calls) = recording_registry(true);
        let stale = registry.client("rec").unwrap();
        // A newer connection took the name; the stale slot is not yet flagged
        // (the registry replaced it before the failing call reported back).
        let (replacement, _) = recording_registry(false);
        let fresh = replacement.client("rec").unwrap();
        registry.lock().clients.insert("rec".into(), fresh.clone());

        // The stale call fails on the wire, which runs `Inner::failed` with a
        // slot that is no longer the registered one.
        let error = registry
            .call_on("rec", &stale, "effect", &json!({}), "call-1")
            .unwrap_err();
        assert!(matches!(error, ToolError::MayHaveRun(_)), "{error:?}");
        assert!(
            Arc::ptr_eq(&registry.client("rec").unwrap(), &fresh),
            "the newer connection must survive"
        );
        assert!(registry.resolve_tool("mcp__rec__effect").is_some());
        let rows = registry.rows();
        assert_eq!(rows[0].status, "connected", "{rows:?}");
        assert_eq!(rows[0].error, None);
    }

    #[test]
    fn g12_the_tool_call_id_reaches_the_server_as_meta() {
        let (registry, calls) = recording_registry(false);
        registry
            .call_exposed("mcp__rec__effect", &json!({"a": 1}), "call-77")
            .unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0]["_meta"][davinci_mcp::INVOCATION_ID_META_KEY],
            "call-77"
        );
        assert_eq!(calls[0]["arguments"], json!({"a": 1}));
    }
}

#[cfg(test)]
#[path = "../tests/common/codemode_mcp.rs"]
mod codemode_structured_tests;
