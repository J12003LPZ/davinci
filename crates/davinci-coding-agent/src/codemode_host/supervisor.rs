use super::{
    assets::{validate_node_version, HostAssets},
    protocol::{read_frame, write_frame},
};
use davinci_agent::codemode::*;
use serde_json::{json, Value};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

/// Constructed only after installation admission; never from script arguments.
pub struct NodeCodeModeHost {
    node: PathBuf,
    assets: HostAssets,
    manifest: super::assets::AssetManifest,
    node_fingerprint: [u8; 32],
}

/// The error for a script the sandbox reported as failed. The sandbox's own
/// deadline can fire before the host watchdog; that is still a timeout, not a
/// sandbox fault, so its kind decides the code.
fn sandbox_failure(result: &Value) -> CodeModeError {
    match result.pointer("/error/kind").and_then(Value::as_str) {
        Some("timeout") => CodeModeError::new("TIMEOUT", "script deadline expired"),
        Some("aborted") => CodeModeError::new("CANCELLED", "script was cancelled"),
        _ => CodeModeError::new("SANDBOX_FAILED", "script failed in the sandbox"),
    }
}

fn node_fingerprint(node: &Path) -> io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    if !node.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "absolute Node path required",
        ));
    }
    for ancestor in node.ancestors() {
        super::assets::reject_link(ancestor)?;
    }
    let metadata = std::fs::metadata(node)?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Node executable",
        ));
    }
    let mut file = std::fs::File::open(node)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 16384];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > metadata.len() {
            return Err(io::Error::other("Node executable changed during admission"));
        }
        hash.update(&buffer[..count]);
    }
    if bytes != metadata.len() || file.metadata()?.len() != metadata.len() {
        return Err(io::Error::other("Node executable changed during admission"));
    }
    Ok(hash.finalize().into())
}

fn command(node: &Path) -> Command {
    let mut command = Command::new(node);
    command.env_clear();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// Node's module resolver and permission model both reject Windows verbatim
/// (`\\?\`) paths, which `canonicalize` produces. Keep canonical identity
/// internally and pass Node the equivalent ordinary path.
fn node_path(path: &Path) -> PathBuf {
    url::Url::from_file_path(path)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .unwrap_or_else(|| path.to_path_buf())
}

fn allow_fs_read(root: &Path) -> std::ffi::OsString {
    let mut flag = std::ffi::OsString::from("--allow-fs-read=");
    flag.push(node_path(root));
    flag
}

/// Defense in depth behind QuickJS/WASM: the host process may read only its
/// admitted assets and start workers. No writes, child processes, addons,
/// WASI or inspector.
fn host_command(node: &Path, root: &Path, entry: &Path) -> Command {
    let mut host = command(node);
    host.arg("--permission")
        .arg("--allow-worker")
        .arg(allow_fs_read(root))
        .arg(node_path(entry));
    host
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct HostWatchdog {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    expired: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for HostWatchdog {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn watch_host(
    child: Arc<Mutex<OwnedChild>>,
    cancellation: davinci_agent::runtime::CancellationToken,
    deadline: Duration,
) -> HostWatchdog {
    let (stop, receiver) = mpsc::channel();
    let expired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let expiry = expired.clone();
    let thread = std::thread::spawn(move || {
        let started = Instant::now();
        loop {
            if receiver.recv_timeout(Duration::from_millis(10)).is_ok() {
                return;
            }
            if cancellation.is_cancelled() || started.elapsed() >= deadline {
                if !cancellation.is_cancelled() {
                    expiry.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                // Cancel before killing: the reader sees EOF as soon as the host
                // dies, and that must not be reported as a transport failure.
                cancellation.cancel();
                // The watchdog keeps running while Rust is awaiting a child tool.
                // Kill only this owned host; child adapters observe the run token.
                let _ = child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .0
                    .kill();
                return;
            }
        }
    });
    HostWatchdog {
        stop,
        thread: Some(thread),
        expired,
    }
}

const MAX_CALLBACK_WORKERS: usize = 16;
/// Callback threads still blocked in a child after their run ended. They are
/// detached, so this bounds the leak instead of letting them hold run slots.
const MAX_STRANDED_CALLBACK_WORKERS: usize = 64;
static ACTIVE_CALLBACK_WORKERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static STRANDED_CALLBACK_WORKERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq)]
enum SlotState {
    /// Counted in `ACTIVE_CALLBACK_WORKERS` for a running run.
    Active,
    /// The run ended while this worker was inside a child call.
    Stranded,
    Released,
}

/// One callback worker's share of the process-wide capacity. A run's slots
/// are returned when the run ends, not when its threads exit: a worker stuck
/// in a non-cooperative callback moves to the bounded stranded pool instead
/// of keeping later runs from starting.
struct CallbackWorkerSlot {
    state: Mutex<SlotState>,
    busy: std::sync::atomic::AtomicBool,
}

impl CallbackWorkerSlot {
    fn try_acquire() -> Result<Arc<Self>, CodeModeError> {
        use std::sync::atomic::Ordering;
        if STRANDED_CALLBACK_WORKERS.load(Ordering::SeqCst) >= MAX_STRANDED_CALLBACK_WORKERS {
            return Err(CodeModeError::new(
                "UNAVAILABLE",
                "Codemode callback workers are stuck in earlier runs",
            ));
        }
        ACTIVE_CALLBACK_WORKERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                (current < MAX_CALLBACK_WORKERS).then_some(current + 1)
            })
            .map_err(|_| {
                CodeModeError::new("UNAVAILABLE", "Codemode callback worker capacity exhausted")
            })?;
        Ok(Arc::new(Self {
            state: Mutex::new(SlotState::Active),
            busy: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// The run is over: return its capacity now.
    fn end_run(&self) {
        use std::sync::atomic::Ordering;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state == SlotState::Active {
            ACTIVE_CALLBACK_WORKERS.fetch_sub(1, Ordering::SeqCst);
            *state = if self.busy.load(Ordering::SeqCst) {
                STRANDED_CALLBACK_WORKERS.fetch_add(1, Ordering::SeqCst);
                SlotState::Stranded
            } else {
                SlotState::Released
            };
        }
    }

    /// The worker thread is exiting (or never started).
    fn exit(&self) {
        use std::sync::atomic::Ordering;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match *state {
            SlotState::Active => {
                ACTIVE_CALLBACK_WORKERS.fetch_sub(1, Ordering::SeqCst);
            }
            SlotState::Stranded => {
                STRANDED_CALLBACK_WORKERS.fetch_sub(1, Ordering::SeqCst);
            }
            SlotState::Released => {}
        }
        *state = SlotState::Released;
    }
}

struct WorkerExit(Arc<CallbackWorkerSlot>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.exit();
    }
}

struct BrokerTask {
    request_id: String,
    call: CodeModeCall,
}

struct BrokerReply {
    request_id: String,
    result: Result<CodeModeToolValue, CodeModeError>,
}

/// Closes a run's callback lane on every exit path, including early errors.
///
/// Workers are detached so a non-cooperative child cannot hold the run past its
/// deadline. Dropping the task sender alone does not discard buffered tasks, so
/// without this flag workers would keep dispatching queued children after the
/// parent outcome was already returned.
struct CallbackLaneGuard {
    closed: Arc<std::sync::atomic::AtomicBool>,
    cancellation: davinci_agent::runtime::CancellationToken,
    slots: Vec<Arc<CallbackWorkerSlot>>,
}

impl Drop for CallbackLaneGuard {
    fn drop(&mut self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        // An in-flight child observes the run token through the broker.
        self.cancellation.cancel();
        for slot in &self.slots {
            slot.end_run();
        }
    }
}

/// Task sender, reply receiver, the lane's shared `closed` flag and the run's
/// worker slots.
type CallbackLane = (
    mpsc::SyncSender<BrokerTask>,
    mpsc::Receiver<BrokerReply>,
    Arc<std::sync::atomic::AtomicBool>,
    Vec<Arc<CallbackWorkerSlot>>,
);

fn start_broker_workers(
    broker: Arc<dyn CodeModeBroker>,
    requested_workers: usize,
    pending_limit: usize,
) -> Result<CallbackLane, CodeModeError> {
    let worker_count = requested_workers.clamp(1, 4);
    let mut slots = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        match CallbackWorkerSlot::try_acquire() {
            Ok(slot) => slots.push(slot),
            Err(error) => {
                for slot in &slots {
                    slot.exit();
                }
                return Err(error);
            }
        }
    }

    let (task_sender, task_receiver) = mpsc::sync_channel::<BrokerTask>(pending_limit.clamp(1, 64));
    let task_receiver = Arc::new(Mutex::new(task_receiver));
    let (reply_sender, reply_receiver) = mpsc::channel();
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    for (index, slot) in slots.iter().enumerate() {
        let tasks = task_receiver.clone();
        let replies = reply_sender.clone();
        let broker = broker.clone();
        let worker_closed = closed.clone();
        let slot = slot.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("davinci-codemode-callback-{index}"))
            .spawn(move || {
                let _exit = WorkerExit(slot.clone());
                loop {
                    let task = {
                        let receiver = tasks.lock().unwrap_or_else(|error| error.into_inner());
                        match receiver.recv() {
                            Ok(task) => task,
                            Err(_) => break,
                        }
                    };
                    // Mark busy before checking `closed`: the lane guard sets
                    // `closed` before reading `busy`, so one of them sees the other.
                    slot.busy.store(true, std::sync::atomic::Ordering::SeqCst);
                    // Remaining queued tasks are discarded with the last receiver.
                    if worker_closed.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    // A panicking callback must still answer its request, or
                    // the parent waits for it until the host deadline.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        broker.call(task.call)
                    }))
                    .unwrap_or_else(|_| {
                        Err(CodeModeError::new("TOOL_FAILED", "child callback panicked"))
                    });
                    slot.busy.store(false, std::sync::atomic::Ordering::SeqCst);
                    if replies
                        .send(BrokerReply {
                            request_id: task.request_id,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        if spawned.is_err() {
            closed.store(true, std::sync::atomic::Ordering::SeqCst);
            // Started workers exit when the task sender drops; give back the
            // slots of the ones that never started.
            for slot in &slots[index..] {
                slot.exit();
            }
            return Err(CodeModeError::new(
                "UNAVAILABLE",
                "unable to start Codemode callback worker",
            ));
        }
    }
    drop(reply_sender);
    Ok((task_sender, reply_receiver, closed, slots))
}

fn write_broker_reply(
    input: &mut impl std::io::Write,
    run_id: &str,
    reply: BrokerReply,
    pending: &mut std::collections::BTreeSet<String>,
    limits: &CodeModeLimits,
    result_bytes: &mut usize,
) -> Result<(), CodeModeError> {
    if !pending.remove(&reply.request_id) {
        return Err(CodeModeError::new(
            "PROTOCOL_ERROR",
            "unexpected completed child request",
        ));
    }
    let ok = reply.result.is_ok();
    let value = match reply.result {
        Ok(value) => serde_json::to_value(value)
            .map_err(|_| CodeModeError::new("TOOL_FAILED", "result encoding failed"))?,
        Err(error) => json!({"error":error}),
    };
    let bytes =
        davinci_agent::codemode::projection::serialized_size(&value, limits.child_result_bytes)
            .map_err(|_| CodeModeError::new("LIMIT_EXCEEDED", "response byte budget"))?;
    *result_bytes = result_bytes.saturating_add(bytes);
    if *result_bytes > limits.total_result_bytes {
        return Err(CodeModeError::new("LIMIT_EXCEEDED", "child result budget"));
    }
    write_frame(
        input,
        &json!({
            "version":1,
            "type":"tool_result",
            "runId":run_id,
            "requestId":reply.request_id,
            "ok":ok,
            "value":value
        }),
    )
    .map_err(|_| CodeModeError::new("PROTOCOL_ERROR", "host write failed"))
}

impl NodeCodeModeHost {
    pub fn new(node: &Path, assets: HostAssets) -> io::Result<Self> {
        let fingerprint = node_fingerprint(node)?;
        if !node.is_absolute() || std::fs::symlink_metadata(node)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "explicit absolute Node executable required",
            ));
        }
        let node = node.canonicalize()?;
        let mut child = OwnedChild(command(&node).arg("--version").spawn()?);
        let started = Instant::now();
        loop {
            if let Some(status) = child.0.try_wait()? {
                if !status.success() {
                    return Err(io::Error::other("Node runtime probe failed"));
                }
                break;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Node runtime probe timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        use std::io::Read;
        let mut version = String::new();
        child
            .0
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing Node stdout"))?
            .take(128)
            .read_to_string(&mut version)?;
        validate_node_version(&version)?;
        let manifest = super::assets::trusted_manifest()?;
        HostAssets::validate(assets.root(), &manifest)?;
        if node_fingerprint(&node)? != fingerprint {
            return Err(io::Error::other("Node executable changed during admission"));
        }
        Ok(Self {
            node,
            assets,
            manifest,
            node_fingerprint: fingerprint,
        })
    }

    fn run(
        &self,
        request: &CodeModeRequest,
        context: &CodeModeRunContext,
        broker: Arc<dyn CodeModeBroker>,
    ) -> Result<CodeModeOutcome, CodeModeError> {
        if matches!(context.mode, CodeModeMode::Off) {
            return Err(CodeModeError::new("UNAVAILABLE", "Codemode is disabled"));
        }
        context.limits_for_request(request)?;
        if node_fingerprint(&self.node)
            .map_err(|_| CodeModeError::new("UNAVAILABLE", "Node executable binding unavailable"))?
            != self.node_fingerprint
        {
            return Err(CodeModeError::new(
                "UNAVAILABLE",
                "Node executable changed since admission",
            ));
        }
        HostAssets::validate(self.assets.root(), &self.manifest).map_err(|_| {
            CodeModeError::new("UNAVAILABLE", "host assets changed since admission")
        })?;
        if context.cancellation.is_cancelled() {
            return Err(CodeModeError::new(
                "CANCELLED",
                "run cancelled before launch",
            ));
        }
        // Privileged bootstrap: the guest's metadata budget stays its own.
        let tools: Vec<Value> = broker
            .catalog(1024)?
            .into_iter()
            .map(|name| json!({"name":name}))
            .collect();
        let private_cwd = tempfile::tempdir()
            .map_err(|_| CodeModeError::new("UNAVAILABLE", "private host directory unavailable"))?;
        let mut child = OwnedChild(
            host_command(&self.node, self.assets.root(), self.assets.entry())
                .current_dir(private_cwd.path())
                .spawn()
                .map_err(|_| CodeModeError::new("UNAVAILABLE", "host launch failed"))?,
        );
        let mut input = child
            .0
            .stdin
            .take()
            .ok_or_else(|| CodeModeError::new("SANDBOX_FAILED", "missing host input"))?;
        let mut output = child
            .0
            .stdout
            .take()
            .ok_or_else(|| CodeModeError::new("SANDBOX_FAILED", "missing host output"))?;
        let child = Arc::new(Mutex::new(child));
        let limits = context.limits_for_request(request)?;
        let (broker_tasks, broker_replies, closed, slots) =
            start_broker_workers(broker.clone(), limits.parallelism, limits.pending_calls)?;
        let lane = CallbackLaneGuard {
            closed,
            cancellation: context.cancellation.clone(),
            slots,
        };
        let watchdog = watch_host(
            child.clone(),
            context.cancellation.clone(),
            Duration::from_millis(limits.wall_ms),
        );
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || loop {
            let frame = read_frame(&mut output);
            let failed = frame.is_err();
            if sender.send(frame).is_err() || failed {
                break;
            }
        });
        let started = Instant::now();
        let run_id = &context.identity.invocation_id;
        let mut hello = false;
        let mut seen = std::collections::BTreeSet::new();
        let mut metadata_calls = 0u32;
        let mut metadata_bytes = 0usize;
        let mut tool_calls = 0u32;
        let mut result_bytes = 0usize;
        let mut pending_broker = std::collections::BTreeSet::new();
        let outcome = (|| loop {
            if context.cancellation.is_cancelled() {
                return Err(
                    if watchdog.expired.load(std::sync::atomic::Ordering::SeqCst) {
                        CodeModeError::new("TIMEOUT", "host deadline expired")
                    } else {
                        CodeModeError::new("CANCELLED", "run cancelled")
                    },
                );
            }
            if started.elapsed() > Duration::from_millis(limits.wall_ms + limits.cleanup_grace_ms) {
                return Err(CodeModeError::new("TIMEOUT", "host deadline expired"));
            }
            while let Ok(reply) = broker_replies.try_recv() {
                write_broker_reply(
                    &mut input,
                    run_id,
                    reply,
                    &mut pending_broker,
                    &limits,
                    &mut result_bytes,
                )?;
            }
            let frame = match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(frame)) => frame,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // A killed host closes its pipe; report why it was killed.
                _ if context.cancellation.is_cancelled() => continue,
                _ => {
                    return Err(CodeModeError::new(
                        "PROTOCOL_ERROR",
                        "host transport failed",
                    ))
                }
            };
            let kind = frame["type"].as_str().unwrap_or("");
            if !hello {
                if kind != "hello"
                    || frame["protocolVersion"] != 1
                    || frame["nodeVersion"] != "v24.21.0"
                {
                    return Err(CodeModeError::new(
                        "PROTOCOL_ERROR",
                        "host identity mismatch",
                    ));
                }
                hello = true;
                write_frame(&mut input, &json!({"version":1,"type":"execute","runId":run_id,
                        "code":request.code,"tools":tools,"timeoutMs":limits.wall_ms,"memoryBytes":limits.vm_heap_bytes}))
                        .map_err(|_| CodeModeError::new("PROTOCOL_ERROR", "host write failed"))?;
                continue;
            }
            if kind == "fatal" {
                // The host may have already cleared its run, so no runId is required.
                return Err(if frame["code"] == "LIMIT_EXCEEDED" {
                    CodeModeError::new("LIMIT_EXCEEDED", "host frame limit exceeded")
                } else {
                    CodeModeError::new("PROTOCOL_ERROR", "host reported a fatal protocol error")
                });
            }
            if frame["runId"] != *run_id {
                return Err(CodeModeError::new(
                    "PROTOCOL_ERROR",
                    "host run identity mismatch",
                ));
            }
            if kind == "finished" {
                if !pending_broker.is_empty() {
                    context.cancellation.cancel();
                    return Err(CodeModeError::new(
                        "CANCELLED",
                        "script finished with unresolved child calls",
                    ));
                }
                let result = &frame["result"];
                let ok = result["ok"]
                    .as_bool()
                    .ok_or_else(|| CodeModeError::new("PROTOCOL_ERROR", "invalid completion"))?;
                let mut text = String::new();
                if let Some(items) = result["output"].as_array() {
                    for item in items {
                        let value = item["text"].as_str().ok_or_else(|| {
                            CodeModeError::new("PROTOCOL_ERROR", "unsupported output")
                        })?;
                        if text.len() + value.len() > limits.collected_output_bytes {
                            return Err(CodeModeError::new(
                                "LIMIT_EXCEEDED",
                                "collected output limit",
                            ));
                        }
                        text.push_str(value);
                    }
                }
                if let Some(value) = result.get("value") {
                    let value = serde_json::to_string(value)
                        .map_err(|_| CodeModeError::new("PROTOCOL_ERROR", "invalid return"))?;
                    if text.len() + value.len() > limits.collected_output_bytes {
                        return Err(CodeModeError::new(
                            "LIMIT_EXCEEDED",
                            "collected return limit",
                        ));
                    }
                    text.push_str(&value);
                }
                let complete = text.len() <= limits.output_bytes;
                if !complete {
                    let mut end = limits.output_bytes;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                }
                return Ok(CodeModeOutcome {
                    status: if ok {
                        CodeModeStatus::Completed
                    } else {
                        CodeModeStatus::Failed
                    },
                    script_completed: ok,
                    output_text: text,
                    output_complete: complete,
                    output_artifact: None,
                    children: vec![],
                    host_notes: if complete {
                        vec![]
                    } else {
                        vec!["Script output was truncated.".into()]
                    },
                    operation_ref: context
                        .identity
                        .parent_operation_ref
                        .clone()
                        .unwrap_or_else(|| format!("ephemeral:{run_id}")),
                    error: (!ok).then(|| sandbox_failure(result)),
                });
            }
            let id = frame["requestId"]
                .as_str()
                .ok_or_else(|| CodeModeError::new("PROTOCOL_ERROR", "missing request identity"))?;
            if id.len() > 32 || !seen.insert(id.to_owned()) || seen.len() > 128 {
                return Err(CodeModeError::new(
                    "PROTOCOL_ERROR",
                    "duplicate or excessive request identity",
                ));
            }
            let name = frame["name"]
                .as_str()
                .ok_or_else(|| CodeModeError::new("PROTOCOL_ERROR", "missing capability name"))?;
            let result: Result<Value, CodeModeError> = match kind {
                "tool_call" => {
                    tool_calls += 1;
                    if tool_calls > limits.tool_calls {
                        return Err(CodeModeError::new("LIMIT_EXCEEDED", "tool call limit"));
                    }
                    if pending_broker.len() >= limits.pending_calls {
                        return Err(CodeModeError::new(
                            "LIMIT_EXCEEDED",
                            "pending child request limit",
                        ));
                    }
                    let parsed_id = id.parse().map_err(|_| {
                        CodeModeError::new("PROTOCOL_ERROR", "invalid request identity")
                    })?;
                    if !pending_broker.insert(id.to_owned()) {
                        return Err(CodeModeError::new(
                            "PROTOCOL_ERROR",
                            "duplicate pending child request",
                        ));
                    }
                    broker_tasks
                        .try_send(BrokerTask {
                            request_id: id.to_owned(),
                            call: CodeModeCall {
                                request_id: parsed_id,
                                tool: name.into(),
                                args: frame["arguments"].clone(),
                            },
                        })
                        .map_err(|_| {
                            CodeModeError::new("LIMIT_EXCEEDED", "child callback queue is full")
                        })?;
                    continue;
                }
                "metadata_query" => {
                    metadata_calls += 1;
                    if metadata_calls > limits.metadata_calls {
                        return Err(CodeModeError::new("LIMIT_EXCEEDED", "metadata call limit"));
                    }
                    match name {
                        "search" => serde_json::from_value(frame["arguments"].clone())
                            .map_err(|_| {
                                CodeModeError::new("INVALID_INPUT", "invalid metadata query")
                            })
                            .and_then(|query| broker.search(query))
                            .and_then(|page| {
                                serde_json::to_value(page).map_err(|_| {
                                    CodeModeError::new("TOOL_FAILED", "metadata encoding failed")
                                })
                            }),
                        "describe" => frame["arguments"]
                            .as_str()
                            .ok_or_else(|| {
                                CodeModeError::new("INVALID_INPUT", "expected canonical name")
                            })
                            .and_then(|name| broker.describe(name)),
                        _ => Err(CodeModeError::new(
                            "INVALID_INPUT",
                            "unknown metadata method",
                        )),
                    }
                }
                _ => {
                    return Err(CodeModeError::new(
                        "PROTOCOL_ERROR",
                        "unexpected host message",
                    ))
                }
            };
            let ok = result.is_ok();
            let value = match result {
                Ok(value) => value,
                Err(error) => json!({"error":error}),
            };
            let bytes = davinci_agent::codemode::projection::serialized_size(
                &value,
                if kind == "tool_call" {
                    limits.child_result_bytes
                } else {
                    limits.metadata_bytes
                },
            )
            .map_err(|_| CodeModeError::new("LIMIT_EXCEEDED", "response byte budget"))?;
            if kind == "tool_call" {
                result_bytes = result_bytes.saturating_add(bytes);
                if result_bytes > limits.total_result_bytes {
                    return Err(CodeModeError::new("LIMIT_EXCEEDED", "child result budget"));
                }
            } else {
                metadata_bytes = metadata_bytes.saturating_add(bytes);
                if metadata_bytes > limits.metadata_bytes {
                    return Err(CodeModeError::new("LIMIT_EXCEEDED", "metadata byte budget"));
                }
            }
            write_frame(&mut input, &json!({"version":1,"type":if kind == "tool_call" {"tool_result"} else {"metadata_result"},
                    "runId":run_id,"requestId":id,"ok":ok,"value":value}))
                    .map_err(|_| CodeModeError::new("PROTOCOL_ERROR", "host write failed"))?;
        })();
        drop(lane);
        drop(broker_tasks);
        drop(input);
        {
            let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
            let _ = child.0.kill();
            let _ = child.0.wait();
        }
        drop(receiver);
        let _ = reader.join();
        outcome
    }
}

impl CodeModeHost for NodeCodeModeHost {
    fn execute(
        &self,
        request: &CodeModeRequest,
        context: &CodeModeRunContext,
        broker: Arc<dyn CodeModeBroker>,
    ) -> CodeModeOutcome {
        self.run(request, context, broker)
            .unwrap_or_else(|error| CodeModeOutcome {
                status: match error.code.as_str() {
                    "CANCELLED" => CodeModeStatus::Cancelled,
                    "RECOVERY_REQUIRED" => CodeModeStatus::RecoveryRequired,
                    _ => CodeModeStatus::Failed,
                },
                script_completed: false,
                output_text: String::new(),
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: context
                    .identity
                    .parent_operation_ref
                    .clone()
                    .unwrap_or_else(|| format!("ephemeral:{}", context.identity.invocation_id)),
                error: Some(error),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_sandbox_deadline_is_reported_as_a_timeout_not_a_sandbox_fault() {
        // The sandbox deadline can beat the host watchdog when little root
        // budget is left; that must still read as TIMEOUT.
        let code = |result: Value| sandbox_failure(&result).code;
        assert_eq!(
            code(json!({"ok": false, "error": {"kind": "timeout"}})),
            "TIMEOUT"
        );
        assert_eq!(
            code(json!({"ok": false, "error": {"kind": "aborted"}})),
            "CANCELLED"
        );
        assert_eq!(
            code(json!({"ok": false, "error": {"kind": "script"}})),
            "SANDBOX_FAILED"
        );
        assert_eq!(code(json!({"ok": false})), "SANDBOX_FAILED");
    }

    struct CountingBroker(Arc<AtomicUsize>);
    impl CodeModeBroker for CountingBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            unreachable!()
        }
        fn describe(&self, _: &str) -> Result<Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, call: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
            Ok(CodeModeToolValue {
                text: String::new(),
                structured_content: None,
                complete: true,
                artifact: None,
                operation_ref: format!("child-{}", call.request_id),
            })
        }
    }

    #[test]
    fn host_launch_uses_the_node_permission_model() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let entry = root.join("host.mjs");
        let host = host_command(Path::new("node"), &root, &entry);
        let args: Vec<_> = host
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "--permission");
        assert_eq!(args[1], "--allow-worker");
        assert!(args[2].starts_with("--allow-fs-read="), "{args:?}");
        assert_eq!(args.len(), 4, "no other grants: {args:?}");
        // Node rejects Windows verbatim paths in both positions.
        assert!(!args[2].contains(r"\\?\"), "{args:?}");
        assert!(!args[3].contains(r"\\?\"), "{args:?}");
        assert_eq!(Path::new(&args[3]), node_path(&entry));
        assert_eq!(
            args[2].trim_start_matches("--allow-fs-read="),
            node_path(&root).to_string_lossy()
        );
    }

    #[test]
    fn closed_lane_never_dispatches_queued_children() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (tasks, replies, closed, slots) =
            start_broker_workers(Arc::new(CountingBroker(calls.clone())), 1, 8).unwrap();
        for request_id in 1..=6 {
            tasks
                .try_send(BrokerTask {
                    request_id: request_id.to_string(),
                    call: CodeModeCall {
                        request_id,
                        tool: "read".into(),
                        args: json!({}),
                    },
                })
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let cancellation = davinci_agent::runtime::CancellationToken::new();
        // The run exits (here: an early error) while five tasks are still queued.
        drop(CallbackLaneGuard {
            closed,
            cancellation: cancellation.clone(),
            slots,
        });
        drop(tasks);
        assert!(cancellation.is_cancelled());
        // The single worker finishes its in-flight child, then must stop.
        while replies.recv_timeout(Duration::from_secs(5)).is_ok() {}
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "queued children were dispatched after the run closed"
        );
    }

    fn task(request_id: u32) -> BrokerTask {
        BrokerTask {
            request_id: request_id.to_string(),
            call: CodeModeCall {
                request_id,
                tool: "read".into(),
                args: json!({}),
            },
        }
    }

    struct PanickingBroker;
    impl CodeModeBroker for PanickingBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            unreachable!()
        }
        fn describe(&self, _: &str) -> Result<Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            panic!("fixture callback panic")
        }
    }

    #[test]
    fn a_panicking_callback_still_answers_its_request() {
        let (tasks, replies, closed, slots) =
            start_broker_workers(Arc::new(PanickingBroker), 1, 8).unwrap();
        tasks.try_send(task(1)).unwrap();
        tasks.try_send(task(2)).unwrap();
        for expected in ["1", "2"] {
            let reply = replies.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(reply.request_id, expected);
            assert_eq!(reply.result.unwrap_err().code, "TOOL_FAILED");
        }
        drop(CallbackLaneGuard {
            closed,
            cancellation: Default::default(),
            slots,
        });
    }

    struct BlockedBroker(Mutex<mpsc::Receiver<()>>, mpsc::Sender<()>);
    impl CodeModeBroker for BlockedBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            unreachable!()
        }
        fn describe(&self, _: &str) -> Result<Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            // Ignores cancellation until the test releases it.
            self.1.send(()).unwrap();
            let _ = self.0.lock().unwrap().recv();
            Err(CodeModeError::new("TOOL_FAILED", "released"))
        }
    }

    #[test]
    fn a_stuck_callback_returns_its_run_slot_when_the_run_ends() {
        let (release, blocked) = mpsc::channel();
        let (entered_tx, entered) = mpsc::channel();
        let broker = Arc::new(BlockedBroker(Mutex::new(blocked), entered_tx));
        let (tasks, replies, closed, slots) = start_broker_workers(broker, 1, 8).unwrap();
        tasks.try_send(task(1)).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let slot = slots[0].clone();
        drop(CallbackLaneGuard {
            closed,
            cancellation: Default::default(),
            slots,
        });
        drop(tasks);
        // The run is over: its slot no longer counts against later runs.
        assert!(*slot.state.lock().unwrap() == SlotState::Stranded);
        release.send(()).unwrap();
        while replies.recv_timeout(Duration::from_secs(5)).is_ok() {}
        assert!(*slot.state.lock().unwrap() == SlotState::Released);
    }
}
