//! Parent-owned worker task command transport. No upstream TypeScript counterpart.
//! The task registry remains the sole commit coordinator and journal writer.

use super::{AgentId, RuntimeHandle};
use crate::tools::{ToolContext, ToolError, ToolResult};
use crate::{PermissionState, PermissionVerdict};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// A 100-row page repeats summaries in content/details. Worst-case JSON
// escaping of 512-byte titles and 4096-byte results exceeds 6 MiB including
// 64 UUID dependencies per row; retain a finite bound with room for metadata.
const MAX_FRAME: usize = 8 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(10);

/// Narrow parent-owned native capability carried by the existing worker channel.
pub trait CoordinatorToolHandler: Send + Sync {
    fn handles(&self, tool: &str) -> bool;
    fn execute(&self, tool: &str, args: &Value) -> Result<ToolResult, ToolError>;
    fn execute_with_context(
        &self,
        tool: &str,
        args: &Value,
        timeout: Duration,
        abort: Option<Arc<AtomicBool>>,
    ) -> Result<ToolResult, ToolError> {
        let _ = (timeout, abort);
        self.execute(tool, args)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    credential: String,
    tool: String,
    args: Value,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    result: Result<ToolResult, String>,
}

/// A per-attempt capability. Debug output never includes its credential.
#[derive(Clone)]
pub struct TaskCoordinatorClient {
    address: SocketAddr,
    credential: String,
}

impl std::fmt::Debug for TaskCoordinatorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskCoordinatorClient")
            .finish_non_exhaustive()
    }
}

impl TaskCoordinatorClient {
    pub fn new(address: SocketAddr, credential: String) -> Self {
        Self {
            address,
            credential,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn credential(&self) -> &str {
        &self.credential
    }

    pub fn from_env() -> Option<Self> {
        let addr_str = std::env::var("DAVINCI_TASK_COORDINATOR_ADDR").ok()?;
        let credential = std::env::var("DAVINCI_TASK_COORDINATOR_CREDENTIAL").ok()?;
        let address = addr_str.parse().ok()?;
        Some(Self {
            address,
            credential,
        })
    }

    /// A lost response is uncertain; callers retain their operation ID for reconciliation.
    /// No transport retry may silently turn one command into two operations.
    pub fn call(&self, tool: &str, args: &Value) -> Result<ToolResult, ToolError> {
        self.call_with_abort(tool, args, None)
    }

    pub fn call_with_abort(
        &self,
        tool: &str,
        args: &Value,
        abort: Option<&AtomicBool>,
    ) -> Result<ToolResult, ToolError> {
        self.call_with_timeout(tool, args, abort, IO_TIMEOUT)
    }

    pub fn call_with_timeout(
        &self,
        tool: &str,
        args: &Value,
        abort: Option<&AtomicBool>,
        timeout: Duration,
    ) -> Result<ToolResult, ToolError> {
        if abort.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(ToolError::Failed("task coordinator aborted".into()));
        }
        let request = Request {
            version: 1,
            credential: self.credential.clone(),
            tool: tool.into(),
            args: args.clone(),
            timeout_ms: Some(timeout.as_millis().min(120_000) as u64),
        };
        let default_stop = AtomicBool::new(false);
        let stop = abort.unwrap_or(&default_stop);
        let deadline = Instant::now() + timeout.min(Duration::from_secs(120));
        let mut stream =
            TcpStream::connect_timeout(&self.address, IO_TIMEOUT).map_err(|_| transport_error())?;
        configure(&stream)?;
        write_frame(&mut stream, &request, deadline, stop)?;
        let response: Response = read_frame(&mut stream, deadline, stop)?;
        response.result.map_err(ToolError::Failed)
    }
}

/// Owns one listener for one registered worker attempt. Only the host creates it.
/// Drop closes admission and waits for any admitted task command to finish; task
/// completion hooks retain their existing synchronous execution semantics.
pub struct TaskCoordinatorTransport {
    client: TaskCoordinatorClient,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TaskCoordinatorTransport {
    pub fn bind(
        parent: &RuntimeHandle,
        child: AgentId,
        permissions: Arc<PermissionState>,
        tools: Vec<String>,
        cwd: PathBuf,
        abort: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        Self::bind_with_handler(parent, child, permissions, tools, cwd, abort, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bind_with_handler(
        parent: &RuntimeHandle,
        child: AgentId,
        permissions: Arc<PermissionState>,
        tools: Vec<String>,
        cwd: PathBuf,
        abort: Arc<AtomicBool>,
        handler: Option<Arc<dyn CoordinatorToolHandler>>,
    ) -> Result<Self, String> {
        if tools
            .iter()
            .any(|tool| !is_task_tool(tool) && !handler.as_ref().is_some_and(|h| h.handles(tool)))
        {
            return Err("task coordinator allowlist contains a non-task tool".into());
        }
        let mut worker = parent.for_worker(child, None)?;
        worker.bus = parent.bus.clone();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "could not bind task coordinator")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "could not configure task coordinator")?;
        let client = TaskCoordinatorClient {
            address: listener
                .local_addr()
                .map_err(|_| "could not resolve task coordinator")?,
            credential: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        let credential = client.credential.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let context = ToolContext {
            runtime: Some(worker),
            abort: Some(abort.clone()),
            ..Default::default()
        };
        let thread = thread::Builder::new()
            .name("task-coordinator".into())
            .spawn(move || {
                while !stopping.load(Ordering::SeqCst) && !abort.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                            if configure(&stream).is_err() {
                                continue;
                            }
                            let deadline = Instant::now() + IO_TIMEOUT;
                            let Ok(request) =
                                read_frame::<Request>(&mut stream, deadline, &stopping)
                            else {
                                continue;
                            };
                            let received_at = Instant::now();
                            let result = dispatch(
                                request,
                                &credential,
                                &tools,
                                &context,
                                &permissions,
                                &cwd,
                                &stopping,
                                handler.as_deref(),
                                received_at,
                            );
                            let _ = write_frame(
                                &mut stream,
                                &Response { result },
                                Instant::now() + IO_TIMEOUT,
                                &stopping,
                            );
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(POLL)
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|_| "could not start task coordinator")?;
        Ok(Self {
            client,
            stop,
            thread: Some(thread),
        })
    }

    pub fn client(&self) -> TaskCoordinatorClient {
        self.client.clone()
    }
}

impl Drop for TaskCoordinatorTransport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn is_task_tool(tool: &str) -> bool {
    matches!(
        tool,
        "task_create" | "task_update" | "task_get" | "task_list"
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    request: Request,
    credential: &str,
    tools: &[String],
    context: &ToolContext,
    permissions: &PermissionState,
    cwd: &std::path::Path,
    stop: &AtomicBool,
    handler: Option<&dyn CoordinatorToolHandler>,
    received_at: Instant,
) -> Result<ToolResult, String> {
    // Fixed-size comparison does not reveal a matching credential prefix.
    let authenticated = request.credential.len() == credential.len()
        && request
            .credential
            .bytes()
            .zip(credential.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0;
    if !authenticated || request.version != 1 {
        return Err("invalid task coordinator capability".into());
    }
    if stop.load(Ordering::SeqCst) || context.is_aborted() {
        return Err("task coordinator stopped".into());
    }
    if let Some(runtime) = &context.runtime {
        if runtime
            .registry
            .get(&runtime.agent_id)
            .is_none_or(|record| {
                !matches!(
                    record.state,
                    super::AgentState::Starting
                        | super::AgentState::Running
                        | super::AgentState::Waiting
                        | super::AgentState::Idle
                )
            })
        {
            return Err("worker attempt is no longer active".into());
        }
    }
    if (!is_task_tool(&request.tool) && !handler.is_some_and(|h| h.handles(&request.tool)))
        || !tools.contains(&request.tool)
    {
        return Err("task tool is not authorized for this worker".into());
    }
    let (issued_policy, _issued_revision) = {
        let state = permissions
            .lock()
            .map_err(|_| "parent permission policy unavailable")?;
        (state.clone(), state.revision())
    };
    let verdict = issued_policy.decide("worker-task-command", &request.tool, &request.args, cwd);
    match verdict {
        PermissionVerdict::Allow => {}
        PermissionVerdict::Deny { reason } => {
            if let Some(runtime) = &context.runtime {
                runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                    call_id: "worker-task-command".into(),
                    reason: reason.clone(),
                });
            }
            return Err(reason);
        }
        PermissionVerdict::Ask(_) => {
            if let Some(runtime) = &context.runtime {
                if let Err(reason) =
                    runtime.emit_decision(crate::runtime::RuntimeEvent::PermissionRequested {
                        call_id: "worker-task-command".into(),
                        tool: request.tool.clone(),
                    })
                {
                    runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                        call_id: "worker-task-command".into(),
                        reason: reason.clone(),
                    });
                    return Err(reason);
                }
                runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                    call_id: "worker-task-command".into(),
                    reason: "worker task command requires parent approval".into(),
                });
            }
            return Err("worker task command requires parent approval".into());
        }
    }
    match request.tool.as_str() {
        "task_create" => super::task_create_tool(&request.args, context),
        "task_update" => super::task_update_tool(&request.args, context),
        "task_get" => super::task_get_tool(&request.args, context),
        "task_list" => super::task_list_tool(&request.args, context),
        _ => {
            let handler = handler
                .ok_or_else(|| "parent native handler unavailable".to_string())?;
            let requested = Duration::from_millis(
                request
                    .timeout_ms
                    .unwrap_or(IO_TIMEOUT.as_millis() as u64),
            )
            .min(Duration::from_secs(120));
            let remaining = requested
                .saturating_sub(received_at.elapsed())
                .max(Duration::from_millis(1));
            handler.execute_with_context(
                &request.tool,
                &request.args,
                remaining,
                context.abort.clone(),
            )
        },
    }
    .map_err(|error| error.to_string())
}

fn transport_error() -> ToolError {
    ToolError::Failed(
        "task coordinator transport unavailable; command outcome may require reconciliation".into(),
    )
}

fn configure(stream: &TcpStream) -> Result<(), ToolError> {
    stream.set_nonblocking(true).map_err(|_| transport_error())
}

fn transfer(
    deadline: Instant,
    stop: &AtomicBool,
    mut remaining: usize,
    mut operation: impl FnMut(usize) -> std::io::Result<usize>,
) -> Result<(), ToolError> {
    let mut offset = 0;
    while remaining > 0 {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err(transport_error());
        }
        match operation(offset) {
            Ok(0) => return Err(transport_error()),
            Ok(count) => {
                offset += count;
                remaining -= count;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(POLL)
            }
            Err(_) => return Err(transport_error()),
        }
    }
    Ok(())
}

fn read_frame<T: DeserializeOwned>(
    stream: &mut TcpStream,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<T, ToolError> {
    let mut header = [0u8; 4];
    transfer(deadline, stop, header.len(), |offset| {
        stream.read(&mut header[offset..])
    })?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(transport_error());
    }
    let mut bytes = vec![0; len];
    transfer(deadline, stop, len, |offset| {
        stream.read(&mut bytes[offset..])
    })?;
    serde_json::from_slice(&bytes).map_err(|_| transport_error())
}

fn write_frame<T: Serialize>(
    stream: &mut TcpStream,
    value: &T,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<(), ToolError> {
    let bytes = serde_json::to_vec(value).map_err(|_| transport_error())?;
    if bytes.len() > MAX_FRAME {
        return Err(transport_error());
    }
    let header = (bytes.len() as u32).to_be_bytes();
    transfer(deadline, stop, header.len(), |offset| {
        stream.write(&header[offset..])
    })?;
    transfer(deadline, stop, bytes.len(), |offset| {
        stream.write(&bytes[offset..])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{AgentId, AgentKind, AgentRecord, AgentState, RuntimeHandle};
    use crate::{PermissionMode, PermissionPolicy, PermissionState};
    use serde_json::json;
    use std::sync::{atomic::AtomicBool, Arc, Mutex};

    #[test]
    fn f03_transport_commits_to_parent_with_bound_worker_identity() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let path = session.path.clone();
        let mut agent = crate::Agent::new("fixture");
        agent.load_from_session(session).unwrap();
        let parent = agent.runtime_for_session().unwrap().clone();
        let child = register(&parent, dir.path());
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let server = TaskCoordinatorTransport::bind(
            &parent,
            child,
            permissions.clone(),
            vec!["task_create".into(), "task_list".into()],
            dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let client = server.client();
        let created = client
            .call(
                "task_create",
                &json!({"title":"remote", "operation_id":uuid::Uuid::new_v4()}),
            )
            .unwrap();
        assert!(!created.is_error);
        let tasks = parent.task_registry.list_tasks(Some(parent.run_id));
        assert_eq!(tasks.len(), 1);
        assert_eq!(created.details.unwrap()["task_id"], tasks[0].id.to_string());
        permissions.lock().unwrap().mode = PermissionMode::ReadOnly;
        assert!(client
            .call(
                "task_create",
                &json!({"title":"denied", "operation_id":uuid::Uuid::new_v4()})
            )
            .is_err());
        assert!(client.call("task_list", &json!({})).is_ok());
        assert!(client.call("task_update", &json!({})).is_err());
        parent
            .registry
            .transition(child, AgentState::Completed)
            .unwrap();
        assert!(client.call("task_list", &json!({})).is_err());
        drop(server);
        assert!(client.call("task_list", &json!({})).is_err());
        drop(parent);
        drop(agent);
        let mut resumed = crate::Agent::new("fixture");
        resumed
            .load_from_session(davinci_session::JsonlSession::open(&path).unwrap())
            .unwrap();
        assert_eq!(
            resumed
                .runtime_for_session()
                .unwrap()
                .task_registry
                .list_tasks(None),
            tasks
        );
    }

    #[test]
    fn native_handler_obeys_allowlist_credentials_policy_and_worker_lifetime() {
        struct Handler;
        impl CoordinatorToolHandler for Handler {
            fn handles(&self, tool: &str) -> bool {
                tool == "lsp_hover"
            }
            fn execute(&self, _tool: &str, _args: &Value) -> Result<ToolResult, ToolError> {
                Ok(ToolResult {
                    content: "parent-owned".into(),
                    is_error: false,
                    details: None,
                })
            }
        }
        let parent = RuntimeHandle::new(
            super::super::RunId::new(),
            AgentId::new(),
            super::super::RuntimeBus::new(),
        );
        let dir = tempfile::tempdir().unwrap();
        let child = register(&parent, dir.path());
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::ReadOnly,
        )));
        let server = TaskCoordinatorTransport::bind_with_handler(
            &parent,
            child,
            permissions.clone(),
            vec!["lsp_hover".into()],
            dir.path().into(),
            Arc::new(AtomicBool::new(false)),
            Some(Arc::new(Handler)),
        )
        .unwrap();
        let client = server.client();
        assert_eq!(
            client
                .call("lsp_hover", &json!({"path":"a.ts"}))
                .unwrap()
                .content,
            "parent-owned"
        );
        assert!(client
            .call("lsp_definition", &json!({"path":"a.ts"}))
            .is_err());
        let mut forged = client.clone();
        forged.credential = "0".repeat(64);
        assert!(forged.call("lsp_hover", &json!({"path":"a.ts"})).is_err());
        permissions
            .lock()
            .unwrap()
            .deny
            .push(crate::PermissionRule::parse("lsp_hover(*)").unwrap());
        assert!(client.call("lsp_hover", &json!({"path":"a.ts"})).is_err());
        permissions.lock().unwrap().deny.clear();
        parent
            .registry
            .transition(child, AgentState::Completed)
            .unwrap();
        assert!(client.call("lsp_hover", &json!({"path":"a.ts"})).is_err());
    }

    fn register(parent: &RuntimeHandle, cwd: &std::path::Path) -> AgentId {
        let child = AgentId::new();
        parent
            .registry
            .register_agent(AgentRecord {
                id: child,
                run_id: parent.run_id,
                parent: Some(parent.agent_id),
                kind: AgentKind::GraphWorker,
                name: "fixture".into(),
                provider: "fixture".into(),
                model_id: "fixture".into(),
                cwd: cwd.to_path_buf(),
                state: AgentState::Running,
                task_id: None,
                worktree: None,
                started_ms: 0,
                updated_ms: 0,
                failure_reason: None,
            })
            .unwrap();
        child
    }

    #[test]
    fn f03_transport_rejects_untrusted_frames_and_credentials() {
        let parent = RuntimeHandle::new(
            super::super::RunId::new(),
            AgentId::new(),
            super::super::RuntimeBus::new(),
        );
        let dir = tempfile::tempdir().unwrap();
        let child = register(&parent, dir.path());
        let abort = Arc::new(AtomicBool::new(false));
        let server = TaskCoordinatorTransport::bind(
            &parent,
            child,
            Arc::new(PermissionState::new(PermissionPolicy::new(
                PermissionMode::AlwaysApprove,
            ))),
            vec!["task_create".into(), "task_list".into()],
            dir.path().to_path_buf(),
            abort.clone(),
        )
        .unwrap();
        let client = server.client();
        let mut forged = client.clone();
        forged.credential = "0".repeat(64);
        assert!(forged
            .call(
                "task_create",
                &json!({"title":"forged","operation_id":uuid::Uuid::new_v4()})
            )
            .is_err());
        assert!(!format!("{client:?}").contains(&client.credential));
        let stop = AtomicBool::new(false);
        for request in [
            json!({"version":2,"credential":client.credential,"tool":"task_list","args":{}}),
            json!({"version":1,"credential":client.credential,"tool":"task_create","args":{},"actor":parent.agent_id}),
            json!({"version":1,"credential":client.credential,"tool":"write","args":{}}),
        ] {
            let mut stream = TcpStream::connect(client.address).unwrap();
            configure(&stream).unwrap();
            let deadline = Instant::now() + IO_TIMEOUT;
            write_frame(&mut stream, &request, deadline, &stop).unwrap();
            assert!(!matches!(
                read_frame::<Response>(&mut stream, deadline, &stop),
                Ok(Response { result: Ok(_) })
            ));
        }
        let mut oversized = TcpStream::connect(client.address).unwrap();
        oversized
            .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .unwrap();
        drop(oversized);
        assert!(client.call("task_list", &json!({})).is_ok());
        assert!(parent.task_registry.list_tasks(None).is_empty());
        abort.store(true, Ordering::SeqCst);
        assert!(client.call("task_list", &json!({})).is_err());
    }

    #[test]
    fn f03_transport_bounds_partial_frames_and_stops_admission() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut receiver, _) = listener.accept().unwrap();
        configure(&receiver).unwrap();
        let start = Instant::now();
        assert!(read_frame::<Request>(
            &mut receiver,
            start + Duration::from_millis(30),
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(read_frame::<Request>(
            &mut receiver,
            Instant::now() + IO_TIMEOUT,
            &AtomicBool::new(true)
        )
        .is_err());
    }

    #[test]
    fn f03_transport_delivers_maximum_task_summary_page() {
        let parent = RuntimeHandle::new(
            super::super::RunId::new(),
            AgentId::new(),
            super::super::RuntimeBus::new(),
        );
        let dir = tempfile::tempdir().unwrap();
        let child = register(&parent, dir.path());
        for _ in 0..100 {
            let mut task = super::super::TaskRecord::new(parent.run_id, "\u{1}".repeat(512));
            task.result = Some("\u{1}".repeat(4096));
            parent.task_registry.create_task(task).unwrap();
        }
        let server = TaskCoordinatorTransport::bind(
            &parent,
            child,
            Arc::new(PermissionState::new(PermissionPolicy::new(
                PermissionMode::ReadOnly,
            ))),
            vec!["task_list".into()],
            dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let result = server
            .client()
            .call("task_list", &json!({"limit":100}))
            .unwrap();
        let details = result.details.unwrap();
        assert_eq!(details["tasks"].as_array().unwrap().len(), 100);
        assert_eq!(details["tasks"][0]["result"].as_str().unwrap().len(), 4096);
    }

    #[test]
    fn f03_transport_competing_workers_claim_once() {
        let parent = RuntimeHandle::new(
            super::super::RunId::new(),
            AgentId::new(),
            super::super::RuntimeBus::new(),
        );
        let dir = tempfile::tempdir().unwrap();
        let task = super::super::TaskRecord::new(parent.run_id, "shared");
        let id = parent.task_registry.create_task(task).unwrap();
        let revision = parent.task_registry.get_task(&id).unwrap().revision;
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut servers = Vec::new();
        let mut threads = Vec::new();
        for _ in 0..2 {
            let child = register(&parent, dir.path());
            let server = TaskCoordinatorTransport::bind(
                &parent,
                child,
                permissions.clone(),
                vec!["task_update".into()],
                dir.path().to_path_buf(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            let client = server.client();
            let barrier = barrier.clone();
            threads.push(thread::spawn(move || {
                let input = json!({"task_id":id,"assigned_to":child,"expected_revision":revision,"operation_id":uuid::Uuid::new_v4()});
                barrier.wait();
                let result = client.call("task_update", &input);
                if result.is_ok() {
                    // The same operation may be reconciled without a second claim.
                    assert!(client.call("task_update", &input).is_ok());
                }
                (child, result)
            }));
            servers.push(server);
        }
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|(_, result)| result.is_ok()).count(),
            1,
            "{results:?}"
        );
        let winner = results.iter().find(|(_, result)| result.is_ok()).unwrap().0;
        let task = parent.task_registry.get_task(&id).unwrap();
        assert_eq!(task.assigned_to, Some(winner));
        assert_eq!(task.revision, revision + 1);
    }

    #[test]
    fn f03_transport_tool_context_delegation_and_abort() {
        let parent = RuntimeHandle::new(
            super::super::RunId::new(),
            AgentId::new(),
            super::super::RuntimeBus::new(),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        struct EventRecorder(Arc<Mutex<Vec<crate::runtime::RuntimeEvent>>>);
        impl crate::runtime::RuntimeSubscriber for EventRecorder {
            fn on_event(
                &self,
                env: &crate::runtime::RuntimeEventEnvelope,
            ) -> crate::runtime::RuntimeDecision {
                self.0.lock().unwrap().push(env.payload.clone());
                crate::runtime::RuntimeDecision::Continue
            }
        }
        parent
            .bus
            .subscribe(Arc::new(EventRecorder(events.clone())));
        let dir = tempfile::tempdir().unwrap();
        let child = register(&parent, dir.path());
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let server = TaskCoordinatorTransport::bind(
            &parent,
            child,
            permissions.clone(),
            vec!["task_create".into(), "task_list".into()],
            dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        let client = server.client();
        let tool_context = ToolContext {
            task_coordinator: Some(client.clone()),
            ..Default::default()
        };

        // Delegation through task_create_tool with no local runtime
        let res = super::super::task_create_tool(
            &json!({"title": "via_context", "operation_id": uuid::Uuid::new_v4()}),
            &tool_context,
        )
        .unwrap();
        assert!(!res.is_error);
        let tasks = parent.task_registry.list_tasks(Some(parent.run_id));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "via_context");

        // Abort signal stops call immediately
        let abort_flag = AtomicBool::new(true);
        let aborted_err = client
            .call_with_abort("task_list", &json!({}), Some(&abort_flag))
            .unwrap_err();
        assert!(aborted_err.to_string().contains("aborted"));

        // When parent permission mode requires approval (e.g. Ask)
        permissions.lock().unwrap().mode = PermissionMode::Ask;
        let ask_err = super::super::task_create_tool(
            &json!({"title": "needs_approval", "operation_id": uuid::Uuid::new_v4()}),
            &tool_context,
        )
        .unwrap_err();
        assert!(ask_err.to_string().contains("requires parent approval"));

        // Verify runtime events were emitted
        let recorded = events.lock().unwrap();
        assert!(recorded
            .iter()
            .any(|e| matches!(e, crate::runtime::RuntimeEvent::PermissionRequested { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, crate::runtime::RuntimeEvent::PermissionDenied { .. })));
    }
}
