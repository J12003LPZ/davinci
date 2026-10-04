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
                // The watchdog keeps running while Rust is awaiting a child tool.
                // Kill only this owned host; child adapters observe the run token.
                let _ = child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .0
                    .kill();
                cancellation.cancel();
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
static ACTIVE_CALLBACK_WORKERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

struct CallbackWorkerPermit;

impl CallbackWorkerPermit {
    fn try_acquire() -> Option<Self> {
        use std::sync::atomic::Ordering;
        let mut current = ACTIVE_CALLBACK_WORKERS.load(Ordering::SeqCst);
        loop {
            if current >= MAX_CALLBACK_WORKERS {
                return None;
            }
            match ACTIVE_CALLBACK_WORKERS.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(Self),
                Err(actual) => current = actual,
            }
        }
    }
}

impl Drop for CallbackWorkerPermit {
    fn drop(&mut self) {
        ACTIVE_CALLBACK_WORKERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
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

fn start_broker_workers(
    broker: Arc<dyn CodeModeBroker>,
    requested_workers: usize,
    pending_limit: usize,
) -> Result<
    (
        mpsc::SyncSender<BrokerTask>,
        mpsc::Receiver<BrokerReply>,
        Vec<std::thread::JoinHandle<()>>,
    ),
    CodeModeError,
> {
    let worker_count = requested_workers.clamp(1, 4);
    let mut permits = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        permits.push(CallbackWorkerPermit::try_acquire().ok_or_else(|| {
            CodeModeError::new("UNAVAILABLE", "Codemode callback worker capacity exhausted")
        })?);
    }

    let (task_sender, task_receiver) = mpsc::sync_channel(pending_limit.clamp(1, 64));
    let task_receiver = Arc::new(Mutex::new(task_receiver));
    let (reply_sender, reply_receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(worker_count);
    for (index, permit) in permits.into_iter().enumerate() {
        let tasks = task_receiver.clone();
        let replies = reply_sender.clone();
        let broker = broker.clone();
        let worker = std::thread::Builder::new()
            .name(format!("davinci-codemode-callback-{index}"))
            .spawn(move || {
                let _permit = permit;
                loop {
                    let task = {
                        let receiver = tasks.lock().unwrap_or_else(|error| error.into_inner());
                        match receiver.recv() {
                            Ok(task) => task,
                            Err(_) => break,
                        }
                    };
                    let result = broker.call(task.call);
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
            })
            .map_err(|_| {
                CodeModeError::new("UNAVAILABLE", "unable to start Codemode callback worker")
            })?;
        workers.push(worker);
    }
    drop(reply_sender);
    Ok((task_sender, reply_receiver, workers))
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
    let bytes = davinci_agent::codemode::projection::serialized_size(
        &value,
        limits.child_result_bytes,
    )
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
        let mut tools = Vec::new();
        let mut cursor = None;
        loop {
            let page = broker.search(ToolQuery {
                query: String::new(),
                limit: 20,
                cursor: cursor.clone(),
            })?;
            if tools.len() + page.tools.len() > 1024 {
                return Err(CodeModeError::new(
                    "LIMIT_EXCEEDED",
                    "capability catalog limit",
                ));
            }
            tools.extend(
                page.tools
                    .into_iter()
                    .map(|tool| json!({"name":tool.canonical_name})),
            );
            match page.cursor {
                Some(next) if Some(&next) != cursor.as_ref() && tools.len() < 1024 => {
                    cursor = Some(next)
                }
                None => break,
                _ => {
                    return Err(CodeModeError::new(
                        "PROTOCOL_ERROR",
                        "invalid catalog cursor",
                    ))
                }
            }
        }
        let private_cwd = tempfile::tempdir()
            .map_err(|_| CodeModeError::new("UNAVAILABLE", "private host directory unavailable"))?;
        let mut child = OwnedChild(
            command(&self.node)
                .arg(self.assets.entry())
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
        let (broker_tasks, broker_replies, _broker_workers) = start_broker_workers(
            broker.clone(),
            limits.parallelism,
            limits.pending_calls,
        )?;
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
                    error: (!ok).then(|| {
                        CodeModeError::new("SANDBOX_FAILED", "script failed in the sandbox")
                    }),
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
                            CodeModeError::new(
                                "LIMIT_EXCEEDED",
                                "child callback queue is full",
                            )
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
