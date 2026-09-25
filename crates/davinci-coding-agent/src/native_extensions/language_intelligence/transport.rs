//! Bounded Content-Length framing and supervised stdio JSON-RPC.

use super::client_requests::ClientRequestState;
use super::diagnostics;
use super::protocol::{IntelligenceError, RequestBudget, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_HEADER: usize = 8192;
pub(super) const MAX_FRAME: usize = 8 * 1024 * 1024;

const MAX_PENDING: usize = 16;
const MAX_STDERR: usize = 8192;
pub(super) const MAX_DOCUMENTS: usize = 64;

// vscode-uri and url serialize drive letters/escapes differently. Diagnostic
// identity is a decoded file path, never the server's choice of URI spelling.
fn diagnostic_key(uri: &str) -> String {
    let path = url::Url::parse(uri)
        .ok()
        .and_then(|uri| uri.to_file_path().ok());
    let key = path
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| uri.into());
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DiagnosticSnapshot {
    pub version: Option<i64>,
    pub sequence: u64,
    pub items: Vec<Value>,
    pub omitted: usize,
}

#[derive(Debug, Default)]
struct State {
    next_id: u64,
    pending: HashMap<u64, mpsc::SyncSender<Result<Value>>>,
    failure: Option<IntelligenceError>,
    diagnostics: HashMap<String, DiagnosticSnapshot>,
    stderr: Vec<u8>,
    rejected_edits: u64,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn fail(&self, error: IntelligenceError) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.failure.is_none() {
            state.failure = Some(error.clone());
        }
        for (_, pending) in state.pending.drain() {
            let _ = pending.try_send(Err(error.clone()));
        }
        self.changed.notify_all();
    }
}

pub(super) struct Transport {
    shared: Arc<Shared>,
    client: ClientRequestState,
    writer: Option<mpsc::SyncSender<Value>>,
    child: Mutex<Child>,
    #[cfg(windows)]
    job: process_job::Job,
    threads: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport")
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

impl Transport {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let client = ClientRequestState::new(workspace, Value::Null)?;
        Self::spawn_with_client(command, client)
    }

    pub fn spawn_with_client(command: &mut Command, client: ClientRequestState) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| {
            IntelligenceError::new(
                "server_start_failed",
                "Could not launch the discovered language server",
            )
        })?;
        #[cfg(windows)]
        let job = process_job::Job::attach(&child).map_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
            IntelligenceError::new(
                "server_start_failed",
                "Could not supervise the language-server process tree",
            )
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let shared = Arc::new(Shared::default());
        let (sender, receiver) = mpsc::sync_channel::<Value>(32);
        let mut transport = Self {
            shared: shared.clone(),
            client: client.clone(),
            writer: Some(sender.clone()),
            child: Mutex::new(child),
            #[cfg(windows)]
            job,
            threads: Vec::new(),
        };
        let write_shared = shared.clone();
        transport.start_thread("lsp-write", move || {
            let mut stdin = stdin;
            while let Ok(value) = receiver.recv() {
                if let Err(error) = write_frame(&mut stdin, &value) {
                    write_shared.fail(error);
                    break;
                }
            }
        })?;
        let read_shared = shared.clone();
        let read_client = client.clone();
        transport.start_thread("lsp-read", move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let message = match read_frame(&mut reader) {
                    Ok(value) => value,
                    Err(error) => {
                        read_shared.fail(error);
                        break;
                    }
                };
                if let Err(error) = dispatch_message(&read_shared, &read_client, &sender, message) {
                    read_shared.fail(error);
                    break;
                }
            }
        })?;
        transport.start_thread("lsp-stderr", move || {
            let mut reader = stderr;
            let mut buffer = [0u8; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                state.stderr.extend_from_slice(&buffer[..count]);
                let overflow = state.stderr.len().saturating_sub(MAX_STDERR);
                state.stderr.drain(..overflow);
            }
        })?;
        Ok(transport)
    }

    fn start_thread(&mut self, name: &str, f: impl FnOnce() + Send + 'static) -> Result<()> {
        let handle = thread::Builder::new()
            .name(name.into())
            .spawn(f)
            .map_err(|_| {
                IntelligenceError::new(
                    "server_start_failed",
                    "Could not start language-server I/O thread",
                )
            })?;
        self.threads.push(handle);
        Ok(())
    }

    fn send(&self, value: Value) -> Result<()> {
        if serde_json::to_vec(&value)
            .map_err(|_| protocol_error())?
            .len()
            > MAX_FRAME
        {
            return Err(protocol_error());
        }
        self.writer
            .as_ref()
            .ok_or_else(exited)?
            .try_send(value)
            .map_err(|_| {
                IntelligenceError::new(
                    "server_busy",
                    "Language-server write queue is full or closed",
                )
            })
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0", "method":method, "params":params}))
    }

    pub fn notify_with_budget(&self, method: &str, params: Value, budget: &RequestBudget) -> Result<()> {
        budget.check()?;
        self.send_until(json!({"jsonrpc":"2.0", "method":method, "params":params}), budget.deadline)
    }

    fn send_until(&self, mut value: Value, deadline: Instant) -> Result<()> {
        if serde_json::to_vec(&value).map_err(|_| protocol_error())?.len() > MAX_FRAME {
            return Err(protocol_error());
        }
        let sender = self.writer.as_ref().ok_or_else(exited)?;
        loop {
            match sender.try_send(value) {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Full(returned)) => {
                    value = returned;
                    if Instant::now() >= deadline {
                        return Err(IntelligenceError::new("request_timeout", "Language-server write queue stayed full until the deadline"));
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return Err(exited()),
            }
        }
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let budget = RequestBudget::from_timeout(timeout);
        self.request_with_budget(method, params, &budget)
    }

    pub fn request_with_budget(&self, method: &str, params: Value, budget: &RequestBudget) -> Result<Value> {
        budget.check()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let id = {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            if state.pending.len() >= MAX_PENDING {
                return Err(IntelligenceError::new(
                    "server_busy",
                    "Too many pending language-server requests",
                ));
            }
            state.next_id = state.next_id.checked_add(1).ok_or_else(protocol_error)?;
            let id = state.next_id;
            state.pending.insert(id, sender);
            id
        };
        if let Err(error) =
            self.send_until(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}), budget.deadline)
        {
            self.shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&id);
            return Err(error);
        }
        loop {
            if let Err(error) = budget.check() {
                self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).pending.remove(&id);
                let _ = self.notify("$/cancelRequest", json!({"id":id}));
                return Err(error);
            }
            let wait = budget.remaining()?.min(Duration::from_millis(25));
            match receiver.recv_timeout(wait) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(exited()),
            }
        }
    }

    pub fn has_dynamic_diagnostics(&self) -> bool {
        self.client.has_document_diagnostics()
    }

    pub fn client_status(&self) -> Value {
        self.client.status()
    }

    pub fn is_alive(&self) -> bool {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failure
            .is_none()
            && matches!(
                self.child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .try_wait(),
                Ok(None)
            )
    }

    pub fn pid(&self) -> u32 {
        self.child.lock().unwrap_or_else(|e| e.into_inner()).id()
    }

    pub fn watch_document(&self, uri: &str) -> Result<()> {
        let uri = diagnostic_key(uri);
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.diagnostics.len() >= MAX_DOCUMENTS && !state.diagnostics.contains_key(&uri) {
            return Err(IntelligenceError::new(
                "document_limit",
                "Session document limit reached",
            ));
        }
        state.diagnostics.entry(uri).or_default();
        Ok(())
    }

    pub fn diagnostics(&self, uri: &str) -> Option<DiagnosticSnapshot> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .get(&diagnostic_key(uri))
            .cloned()
    }

    pub fn unwatch_document(&self, uri: &str) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .remove(&diagnostic_key(uri));
    }

    pub fn wait_diagnostics(
        &self,
        uri: &str,
        after: u64,
        timeout: Duration,
    ) -> Result<DiagnosticSnapshot> {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.failure.is_none()
                    && state
                        .diagnostics
                        .get(&diagnostic_key(uri))
                        .is_none_or(|d| d.sequence <= after)
            })
            .unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        state
            .diagnostics
            .get(&diagnostic_key(uri))
            .filter(|d| d.sequence > after)
            .cloned()
            .ok_or_else(|| {
                IntelligenceError::new(
                    "diagnostics_pending",
                    "No fresh diagnostic publication arrived before the deadline",
                )
            })
    }

    fn terminate(&self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        {
            let group = -(child.id() as i32);
            unsafe { libc::kill(group, libc::SIGTERM); }
            let grace = Instant::now() + Duration::from_millis(200);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < grace {
                thread::sleep(Duration::from_millis(10));
            }
            unsafe { libc::kill(group, libc::SIGKILL); }
        }
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(windows)]
mod process_job {
    use std::ffi::c_void;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Child;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        fn SetInformationJobObject(job: *mut c_void, info_class: i32, info: *const c_void, info_len: u32) -> i32;
    }

    pub(super) struct Job(OwnedHandle);

    impl Job {
        pub fn attach(child: &Child) -> std::io::Result<Self> {
            // Null arguments create a private, unnamed job with default security.
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            // CreateJobObjectW transferred ownership of a valid handle.
            let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
            #[repr(C)]
            struct BasicLimit {
                per_process_user_time_limit: i64,
                per_job_user_time_limit: i64,
                limit_flags: u32,
                minimum_working_set_size: usize,
                maximum_working_set_size: usize,
                active_process_limit: u32,
                affinity: usize,
                priority_class: u32,
                scheduling_class: u32,
            }
            #[repr(C)]
            struct IoCounters {
                read_operation_count: u64,
                write_operation_count: u64,
                other_operation_count: u64,
                read_transfer_count: u64,
                write_transfer_count: u64,
                other_transfer_count: u64,
            }
            #[repr(C)]
            struct ExtendedLimit {
                basic_limit_information: BasicLimit,
                io_info: IoCounters,
                process_memory_limit: usize,
                job_memory_limit: usize,
                peak_process_memory_used: usize,
                peak_job_memory_used: usize,
            }
            const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x00002000;
            const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
            let info = ExtendedLimit {
                basic_limit_information: BasicLimit {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    minimum_working_set_size: 0,
                    maximum_working_set_size: 0,
                    active_process_limit: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io_info: IoCounters {
                    read_operation_count: 0, write_operation_count: 0, other_operation_count: 0,
                    read_transfer_count: 0, write_transfer_count: 0, other_transfer_count: 0,
                },
                process_memory_limit: 0, job_memory_limit: 0,
                peak_process_memory_used: 0, peak_job_memory_used: 0,
            };
            if unsafe {
                SetInformationJobObject(
                    job.0.as_raw_handle(),
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<ExtendedLimit>() as u32,
                )
            } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }

        pub fn terminate(&self) {
            // The owned handle stays valid; descendants inherit job membership.
            unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            self.terminate();
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if self.is_alive() {
            let _ = self.request("shutdown", Value::Null, Duration::from_millis(200));
            let _ = self.notify("exit", Value::Null);
        }
        self.shared.fail(exited());
        self.writer.take();
        self.terminate();
        let deadline = Instant::now() + Duration::from_millis(250);
        // A defective descendant must not make host shutdown wait forever.
        for handle in self.threads.drain(..) {
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

fn dispatch_message(
    shared: &Shared,
    client: &ClientRequestState,
    writer: &mpsc::SyncSender<Value>,
    message: Value,
) -> Result<()> {
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(protocol_error());
    }
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        if let Some(id) = message.get("id") {
            if method == "workspace/applyEdit" {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                state.rejected_edits = state.rejected_edits.saturating_add(1);
            }
            let response = match client.handle_request(method, &message["params"]) {
                Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
                Err((code, text)) => json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":text}}),
            };
            writer.try_send(response).map_err(|_| protocol_error())?;
        } else if method == "textDocument/publishDiagnostics" {
            let params = &message["params"];
            let uri = params["uri"].as_str().ok_or_else(protocol_error)?;
            let items = params["diagnostics"]
                .as_array()
                .ok_or_else(protocol_error)?;
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(snapshot) = state.diagnostics.get_mut(&diagnostic_key(uri)) {
                let incoming = params.get("version").and_then(Value::as_i64);
                if diagnostics::accepts_version(snapshot.version, incoming) {
                    // Bound retained diagnostics separately from wire frame limits.
                    // Reject stale/unversioned replacement before mutating the current set.
                    let mut bytes = 0;
                    let next_items: Vec<Value> = items
                        .iter()
                        .take(1000)
                        .take_while(|item| {
                            bytes += item.to_string().len();
                            bytes <= 256 * 1024
                        })
                        .cloned()
                        .collect();
                    snapshot.omitted = items.len().saturating_sub(next_items.len());
                    snapshot.items = next_items;
                    snapshot.version = incoming;
                    snapshot.sequence = snapshot.sequence.saturating_add(1);
                    shared.changed.notify_all();
                }
            }
        } else {
            client.observe_notification(method, &message["params"]);
        }
        return Ok(());
    }
    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(protocol_error)?;
    let result = match (message.get("result"), message.get("error")) {
        (Some(value), None) => Ok(value.clone()),
        (None, Some(error)) if error["code"].is_i64() => Err(IntelligenceError::new(
            if error["code"] == -32601 {
                "unsupported_method"
            } else {
                "server_error"
            },
            "Language server rejected the semantic request",
        )),
        _ => return Err(protocol_error()),
    };
    if let Some(sender) = shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending
        .remove(&id)
    {
        let _ = sender.try_send(result);
    }
    Ok(())
}

fn protocol_error() -> IntelligenceError {
    IntelligenceError::new(
        "protocol_error",
        "Invalid or oversized language-server frame",
    )
}

fn exited() -> IntelligenceError {
    IntelligenceError::new("server_exited", "Language-server stream closed")
}

fn read_frame(reader: &mut impl BufRead) -> Result<Value> {
    let mut consumed = 0;
    let mut length = None;
    loop {
        let mut line = Vec::new();
        // Take bounds allocation even for a malicious header with no newline.
        let count = reader
            .take((MAX_HEADER - consumed + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| exited())?;
        if count == 0 {
            return Err(exited());
        }
        consumed += count;
        if consumed > MAX_HEADER || !line.ends_with(b"\r\n") {
            return Err(protocol_error());
        }
        if line == b"\r\n" {
            break;
        }
        let line = std::str::from_utf8(&line[..line.len() - 2]).map_err(|_| protocol_error())?;
        let (name, value) = line.split_once(':').ok_or_else(protocol_error)?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() || !value.trim().bytes().all(|c| c.is_ascii_digit()) {
                return Err(protocol_error());
            }
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| protocol_error())?;
            if parsed == 0 || parsed > MAX_FRAME {
                return Err(protocol_error());
            }
            length = Some(parsed);
        }
    }
    let mut bytes = vec![0; length.ok_or_else(protocol_error)?];
    reader.read_exact(&mut bytes).map_err(|_| exited())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| protocol_error())?;
    if !value.is_object() {
        return Err(protocol_error());
    }
    Ok(value)
}

fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| protocol_error())?;
    if bytes.len() > MAX_FRAME {
        return Err(protocol_error());
    }
    write!(writer, "Content-Length: {}\r\n\r\n", bytes.len()).map_err(|_| exited())?;
    writer.write_all(&bytes).map_err(|_| exited())?;
    writer.flush().map_err(|_| exited())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_uri_encodings_share_identity() {
        #[cfg(not(windows))]
        assert_eq!(
            diagnostic_key("file:///tmp/a%20b.ts"),
            diagnostic_key("file:///tmp/a b.ts")
        );
        #[cfg(windows)]
        assert_eq!(
            diagnostic_key("file:///C:/a.ts"),
            diagnostic_key("file:///c%3A/a.ts")
        );
    }

    #[test]
    fn descendants_are_cleaned_even_when_server_parent_exited() {
        let transport = fixture("normal");
        let pid = transport
            .request("fixture/orphan", json!({}), Duration::from_secs(3))
            .unwrap()
            .as_u64()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while transport
            .child
            .lock()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < deadline, "fixture parent did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        drop(transport);
        let result = Command::new("node").args(["-e", "try { process.kill(Number(process.argv[1]), 0); process.exit(1); } catch { process.exit(0); }", &pid.to_string()]).status().unwrap();
        // Always clean up the fixture even on the RED run.
        if !result.success() {
            davinci_agent::jobs::kill_tree(pid as u32);
        }
        assert!(
            result.success(),
            "server descendant survived transport shutdown"
        );
    }
    use serde_json::json;
    use std::io::{BufReader, Cursor, Read};

    struct Chunks(Cursor<Vec<u8>>);
    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let len = buf.len().min(2);
            self.0.read(&mut buf[..len])
        }
    }

    #[test]
    fn content_length_split_and_coalesced_frames_use_byte_lengths() {
        let first = json!({"jsonrpc":"2.0","id":1,"result":"λ🦀"});
        let second = json!({"jsonrpc":"2.0","method":"notification","params":{}});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &first).unwrap();
        write_frame(&mut bytes, &second).unwrap();
        for capacity in [1, 2, 4096] {
            let mut reader = BufReader::with_capacity(capacity, Chunks(Cursor::new(bytes.clone())));
            assert_eq!(read_frame(&mut reader).unwrap(), first);
            assert_eq!(read_frame(&mut reader).unwrap(), second);
            assert_eq!(read_frame(&mut reader).unwrap_err().code, "server_exited");
        }
    }

    #[test]
    fn malformed_headers_json_and_bounds_are_rejected() {
        let cases = [
            "Content-Length: -1\r\n\r\n{}".to_owned(),
            "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".into(),
            "Content-Type: application/json\r\n\r\n{}".into(),
            "Content-Length: 2\n\n{}".into(),
            "Content-Length: 2\r\n\r\nxx".into(),
            "Content-Length: 4\r\n\r\nnull".into(),
            format!("Content-Length: {}\r\n\r\n", MAX_FRAME + 1),
            format!("X-Header: {}", "x".repeat(MAX_HEADER)),
        ];
        for case in cases {
            assert_eq!(
                read_frame(&mut Cursor::new(case)).unwrap_err().code,
                "protocol_error"
            );
        }
    }

    #[test]
    fn valid_additional_headers_and_truncated_body() {
        let mut input = Cursor::new(b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}");
        assert_eq!(read_frame(&mut input).unwrap(), json!({}));
        let mut input = Cursor::new(b"Content-Length: 20\r\n\r\n{}");
        assert_eq!(read_frame(&mut input).unwrap_err().code, "server_exited");
    }

    fn fixture(mode: &str) -> Transport {
        let mut command = std::process::Command::new("node");
        command
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/language-server.cjs"
            ))
            .arg(mode);
        Transport::spawn(&mut command).unwrap()
    }

    #[test]
    fn concurrent_response_ids_and_server_messages_do_not_cross() {
        let transport = std::sync::Arc::new(fixture("normal"));
        transport.watch_document("file:///fixture.ts").unwrap();
        let handles: Vec<_> = (0..8)
            .map(|n| {
                let transport = transport.clone();
                std::thread::spawn(move || {
                    let params = json!({"n": n, "delay": (8-n)*5, "uri":"file:///fixture.ts"});
                    assert_eq!(
                        transport
                            .request(
                                "fixture/echo",
                                params.clone(),
                                std::time::Duration::from_secs(3)
                            )
                            .unwrap(),
                        params
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let state = transport.shared.state.lock().unwrap();
        assert_eq!(state.rejected_edits, 8);
        assert!(state.stderr.len() <= MAX_STDERR);
        assert_eq!(
            state.diagnostics[&diagnostic_key("file:///fixture.ts")].version,
            Some(1)
        );
    }

    #[test]
    fn a_request_timeout_leaves_the_server_running() {
        let transport = fixture("timeout");
        let error = transport
            .request(
                "fixture/echo",
                json!({}),
                std::time::Duration::from_millis(200),
            )
            .unwrap_err();
        assert_eq!(error.code, "request_timeout");
        assert!(transport.is_alive());
    }

    #[test]
    fn timeout_exit_and_protocol_failure_are_structured() {
        for (mode, code) in [
            ("timeout", "request_timeout"),
            ("exit", "server_exited"),
            ("malformed", "protocol_error"),
        ] {
            let transport = fixture(mode);
            let started = std::time::Instant::now();
            let error = transport
                .request(
                    "fixture/echo",
                    json!({}),
                    std::time::Duration::from_millis(500),
                )
                .unwrap_err();
            assert_eq!(error.code, code);
            assert!(started.elapsed() < std::time::Duration::from_secs(3));
        }
    }
}
