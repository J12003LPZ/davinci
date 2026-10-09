//! Newline JSON-RPC over a child process's stdin/stdout.
//!
//! Stdout is drained by a reader thread so a server that never answers
//! cannot block the agent past the call deadline; stderr is drained into a
//! bounded tail that the error row quotes when the server dies.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::jsonrpc::{Notification, Request, Response};
use crate::{Error, Result, RpcTransport, CALL_TIMEOUT_SECS};

/// How much of the child's stderr is kept for the error row.
pub const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// Variables a stdio server inherits, matching the MCP TypeScript SDK's
/// `getDefaultEnvironment`. Everything else must be passed explicitly in the
/// server's `env` configuration.
pub const INHERITED_ENV: &[&str] = if cfg!(windows) {
    &[
        "APPDATA",
        "HOMEDRIVE",
        "HOMEPATH",
        "LOCALAPPDATA",
        "PATH",
        "PATHEXT",
        "PROCESSOR_ARCHITECTURE",
        "SYSTEMDRIVE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "USERNAME",
        "USERPROFILE",
        "PROGRAMFILES",
        "COMSPEC",
    ]
} else {
    &[
        "HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER", "LANG", "TMPDIR",
    ]
};

/// How many trailing stderr lines a transport error quotes.
const STDERR_QUOTE_LINES: usize = 5;

/// How long a closed-stdout error waits for the stderr thread to finish
/// draining, so the child's last words make it into the message.
const STDERR_DRAIN_GRACE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct StderrTail {
    bytes: Vec<u8>,
    closed: bool,
}

pub struct StdioTransport {
    stdin: StdinWriter,
    lines: Receiver<std::io::Result<String>>,
    stderr: Arc<(Mutex<StderrTail>, Condvar)>,
    next_id: u64,
    call_timeout: Duration,
}

impl StdioTransport {
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<(Self, Child)> {
        let program = resolve_command(command, env);
        let mut cmd = Command::new(&program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.env_clear();
        for (key, value) in child_environment(std::env::vars_os(), env) {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|err| Error::Transport(format!("spawn `{command}`: {err}")))?;
        let stdin = StdinWriter::spawn(
            child
                .stdin
                .take()
                .ok_or_else(|| Error::Transport("stdio server has no stdin".into()))?,
        );
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Transport("stdio server has no stdout".into()))?;
        let stderr: Arc<(Mutex<StderrTail>, Condvar)> = Arc::default();
        match child.stderr.take() {
            Some(mut pipe) => {
                let shared = Arc::clone(&stderr);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let mut tail = shared.0.lock().unwrap_or_else(|err| err.into_inner());
                        tail.bytes.extend_from_slice(&buf[..n]);
                        if tail.bytes.len() > STDERR_TAIL_BYTES {
                            let excess = tail.bytes.len() - STDERR_TAIL_BYTES;
                            tail.bytes.drain(..excess);
                        }
                    }
                    shared
                        .0
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .closed = true;
                    shared.1.notify_all();
                });
            }
            None => {
                stderr
                    .0
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .closed = true
            }
        }
        let (sender, lines) = stdout_channel();
        let response_writer = stdin.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let line = match read_stdout_line(&mut reader, MAX_STDOUT_LINE_BYTES) {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                };
                let Ok(frame) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                // A line is one message or, per 2025-03-26, a JSON-RPC batch.
                let mut stop = false;
                for message in crate::http::flatten_batch(frame) {
                    let Value::Object(message) = message else {
                        continue;
                    };
                    if let Some(method) = message.get("method").and_then(Value::as_str) {
                        if let Some(id) = message.get("id").cloned() {
                            // Never wait here: a server that is not reading its
                            // stdin must not also stall this stdout drain.
                            response_writer.enqueue(&server_request_reply(id, method));
                        }
                        continue;
                    }
                    if message.get("id").is_none()
                        || (!message.contains_key("result") && !message.contains_key("error"))
                    {
                        continue;
                    }
                    let Ok(reply) = serde_json::to_string(&message) else {
                        continue;
                    };
                    if sender.send(Ok(reply)).is_err() {
                        stop = true;
                        break;
                    }
                }
                if stop {
                    break;
                }
            }
        });
        Ok((
            Self {
                stdin,
                lines,
                stderr,
                next_id: 1,
                call_timeout: Duration::from_secs(CALL_TIMEOUT_SECS),
            },
            child,
        ))
    }

    pub fn set_call_timeout(&mut self, timeout: Duration) {
        self.call_timeout = timeout;
    }

    /// The last [`STDERR_TAIL_BYTES`] of the child's stderr, lossily decoded.
    pub fn stderr_tail(&self) -> String {
        let tail = self.stderr.0.lock().unwrap_or_else(|err| err.into_inner());
        String::from_utf8_lossy(&tail.bytes).into_owned()
    }

    /// Give the stderr thread a moment to reach EOF once the child has
    /// gone, so the quoted tail includes its final lines.
    fn wait_for_stderr_close(&self) {
        let (lock, cvar) = &*self.stderr;
        let guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let _ = cvar.wait_timeout_while(guard, STDERR_DRAIN_GRACE, |tail| !tail.closed);
    }

    /// The last few non-empty stderr lines, ready to append to an error.
    fn stderr_excerpt(&self) -> String {
        let tail = self.stderr_tail();
        let lines: Vec<&str> = tail
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        let start = lines.len().saturating_sub(STDERR_QUOTE_LINES);
        format!(" (stderr: {})", lines[start..].join(" | "))
    }

    fn transport_error(&self, what: &str) -> Error {
        Error::Transport(format!("{what}{}", self.stderr_excerpt()))
    }

    /// A transport error for a server that has gone away.
    fn closed_error(&self, what: &str) -> Error {
        self.wait_for_stderr_close();
        self.transport_error(what)
    }

    /// Write one line, waiting for it to reach the pipe until `deadline`.
    fn write_line(&mut self, value: &Value, deadline: Instant) -> Result<()> {
        match self.stdin.write(value, deadline) {
            Ok(()) => Ok(()),
            Err(WriteError::TimedOut) => Err(self.transport_error(&format!(
                "mcp server stdin: write timed out after {}s; the server is not reading its input",
                self.call_timeout.as_secs_f32()
            ))),
            Err(WriteError::Backlog) => Err(self.transport_error(
                "mcp server stdin: not written; earlier lines are still unread by the server",
            )),
            Err(WriteError::Io(err)) => Err(self.closed_error(&format!("mcp server stdin: {err}"))),
        }
    }

    /// Best effort, never waits: the request already failed.
    fn cancel_request(&mut self, id: &Value) {
        self.stdin.enqueue(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": id, "reason": "timeout" }
        }));
    }

    /// Wait for the reply to `id`, answering server-to-client requests and
    /// skipping notifications and stray log lines on the way.
    fn read_response(&mut self, id: &Value, deadline: Instant) -> Result<Value> {
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.cancel_request(id);
                return Err(self.transport_error(&timeout_message(self.call_timeout)));
            }
            let line = match self.lines.recv_timeout(deadline - now) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(self.transport_error(&format!("mcp stdout: {error}")));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel_request(id);
                    return Err(self.transport_error(&timeout_message(self.call_timeout)));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.closed_error("mcp server closed stdout"));
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(Value::Object(message)) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if message.get("id") != Some(id) {
                continue;
            }
            let has_result = message.contains_key("result");
            let has_error = message.contains_key("error");
            if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
                || message.contains_key("method")
                || has_result == has_error
            {
                return Err(Error::Protocol(format!(
                    "invalid MCP response envelope `{trimmed}`"
                )));
            }
            let parsed: Response = serde_json::from_value(Value::Object(message))
                .map_err(|err| Error::Protocol(format!("decode `{trimmed}`: {err}")))?;
            if let Some(error) = parsed.error {
                return Err(Error::Rpc {
                    code: error.code,
                    message: error.message,
                });
            }
            return Ok(parsed.result.unwrap_or(Value::Null));
        }
    }
}

fn server_request_reply(id: Value, method: &str) -> Value {
    if method == "ping" {
        json!({ "jsonrpc": "2.0", "id": id, "result": {} })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "method not supported" }
        })
    }
}

/// Lines queued for the child's stdin beyond the one being written. A full
/// queue means the server has stopped reading, so further writes fail at
/// once instead of piling up behind the stuck one.
const MAX_QUEUED_STDIN_LINES: usize = 16;

struct WriteJob {
    line: Vec<u8>,
    done: Option<SyncSender<std::io::Result<()>>>,
    /// Set by a caller that stopped waiting. A job still queued when its
    /// caller gave up is skipped, so a request the caller already reported
    /// as failed does not reach the server later.
    abandoned: Arc<AtomicBool>,
}

enum WriteError {
    TimedOut,
    Backlog,
    Io(std::io::Error),
}

/// Owns the child's stdin on a dedicated thread (WOR-51). `write_all` on a
/// pipe the server never drains blocks forever, so callers wait for the
/// thread's acknowledgement with a deadline instead of writing themselves.
/// Lines are written whole and in order. A stuck write is released once every
/// holder of the pipe's read end exits. Killing the child is enough for a
/// direct child; a `.cmd` shim on Windows can leave a grandchild holding the
/// pipe, and then this thread stays blocked until that process exits. The
/// caller never waits on it either way.
#[derive(Clone)]
struct StdinWriter {
    jobs: SyncSender<WriteJob>,
}

impl StdinWriter {
    fn spawn(mut stdin: impl Write + Send + 'static) -> Self {
        let (jobs, queue) = mpsc::sync_channel::<WriteJob>(MAX_QUEUED_STDIN_LINES);
        std::thread::spawn(move || {
            let mut broken: Option<(std::io::ErrorKind, String)> = None;
            for job in queue {
                if job.abandoned.load(Ordering::SeqCst) {
                    continue;
                }
                let result = match &broken {
                    Some((kind, message)) => Err(std::io::Error::new(*kind, message.clone())),
                    None => stdin.write_all(&job.line).and_then(|()| stdin.flush()),
                };
                if let Err(error) = &result {
                    broken.get_or_insert_with(|| (error.kind(), error.to_string()));
                }
                if let Some(done) = job.done {
                    let _ = done.send(result);
                }
            }
        });
        Self { jobs }
    }

    fn encode(value: &Value) -> std::io::Result<Vec<u8>> {
        let mut line = serde_json::to_vec(value)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        line.push(b'\n');
        Ok(line)
    }

    fn write(&self, value: &Value, deadline: Instant) -> std::result::Result<(), WriteError> {
        let line = Self::encode(value).map_err(WriteError::Io)?;
        let (done, ack) = mpsc::sync_channel(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        match self.jobs.try_send(WriteJob {
            line,
            done: Some(done),
            abandoned: Arc::clone(&abandoned),
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(WriteError::Backlog),
            Err(TrySendError::Disconnected(_)) => {
                return Err(WriteError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "stdin writer stopped",
                )))
            }
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        match ack.recv_timeout(wait) {
            Ok(result) => result.map_err(WriteError::Io),
            Err(RecvTimeoutError::Timeout) => {
                abandoned.store(true, Ordering::SeqCst);
                Err(WriteError::TimedOut)
            }
            Err(RecvTimeoutError::Disconnected) => Err(WriteError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stdin writer stopped",
            ))),
        }
    }

    /// Queue a line without waiting for it; dropped if the queue is full.
    fn enqueue(&self, value: &Value) {
        if let Ok(line) = Self::encode(value) {
            let _ = self.jobs.try_send(WriteJob {
                line,
                done: None,
                abandoned: Arc::default(),
            });
        }
    }
}

const MAX_STDOUT_LINE_BYTES: usize = 16 * 1024 * 1024;

const MAX_QUEUED_STDOUT_LINES: usize = 4;
type StdoutLine = std::io::Result<String>;

fn stdout_channel() -> (mpsc::SyncSender<StdoutLine>, Receiver<StdoutLine>) {
    // Each entry is separately bounded. Slow or idle consumers exert backpressure
    // rather than accumulating unlimited notifications. Dropping the receiver
    // wakes a sender blocked on a full queue, so teardown needs no reader join.
    mpsc::sync_channel(MAX_QUEUED_STDOUT_LINES)
}

fn read_stdout_line(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    if reader
        .take(limit.saturating_add(1) as u64)
        .read_until(b'\n', &mut bytes)?
        == 0
    {
        return Ok(None);
    }
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("MCP stdout line exceeds {limit} bytes"),
        ));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn timeout_message(timeout: Duration) -> String {
    format!("mcp call timed out after {}s", timeout.as_secs())
}

impl RpcTransport for StdioTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request::new(id, method, params);
        let value = serde_json::to_value(&request)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        // One deadline covers both writing the request and reading the reply.
        let deadline = Instant::now() + self.call_timeout;
        let id = Value::from(id);
        if let Err(error) = self.write_line(&value, deadline) {
            // The request may already be partly in the pipe and reach the
            // server once it reads again; tell it the caller gave up.
            self.cancel_request(&id);
            return Err(error);
        }
        self.read_response(&id, deadline)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let note = Notification::new(method, params);
        let value =
            serde_json::to_value(&note).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        let deadline = Instant::now() + self.call_timeout;
        self.write_line(&value, deadline)
    }
}

fn child_environment(
    parent: impl Iterator<Item = (OsString, OsString)>,
    config: &BTreeMap<String, String>,
) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = parent
        .filter(|(key, _)| {
            key.to_str().is_some_and(|key| {
                INHERITED_ENV
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(key))
            })
        })
        .collect();
    env.extend(
        config
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    env
}

/// On Windows `CreateProcess` does not consult `PATHEXT`, so `npx` from
/// `mcp.json` would not find `npx.cmd`. Resolve it ourselves; the standard
/// library then runs a `.cmd`/`.bat` through `cmd.exe` with safe quoting.
/// Elsewhere the command is spawned as written.
#[cfg(windows)]
fn resolve_command(command: &str, env: &BTreeMap<String, String>) -> PathBuf {
    let path = env
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| std::ffi::OsString::from(value))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let exts: Vec<String> = std::env::var("PATHEXT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .map(|ext| ext.trim().to_ascii_lowercase())
        .filter(|ext| ext.starts_with('.'))
        .collect();
    resolve_command_in(command, &dirs, &exts).unwrap_or_else(|| PathBuf::from(command))
}

#[cfg(not(windows))]
fn resolve_command(command: &str, _env: &BTreeMap<String, String>) -> PathBuf {
    PathBuf::from(command)
}

/// Find `command` as `dir/command<ext>` for the first `dir` in `dirs` and
/// first `ext` in `exts` that exists. A command that already carries a
/// directory or an extension is returned as written; a command found
/// nowhere yields `None` so the caller can let the OS report the error.
pub fn resolve_command_in(command: &str, dirs: &[PathBuf], exts: &[String]) -> Option<PathBuf> {
    let as_path = Path::new(command);
    if command.contains(['/', '\\']) || as_path.extension().is_some() {
        return Some(as_path.to_path_buf());
    }
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for ext in exts {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writer whose first write blocks until released, recording every line.
    struct GatedPipe {
        gate: Option<Receiver<()>>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for GatedPipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if let Some(gate) = self.gate.take() {
                let _ = gate.recv();
            }
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn wor51_a_request_abandoned_while_queued_is_never_written() {
        let (release, gate) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer = StdinWriter::spawn(GatedPipe {
            gate: Some(gate),
            written: Arc::clone(&written),
        });
        let soon = || Instant::now() + Duration::from_millis(100);
        assert!(matches!(
            writer.write(&json!({"n": 1}), soon()),
            Err(WriteError::TimedOut)
        ));
        assert!(matches!(
            writer.write(&json!({"n": 2}), soon()),
            Err(WriteError::TimedOut)
        ));
        release.send(()).unwrap();
        writer
            .write(&json!({"n": 3}), soon() + Duration::from_secs(5))
            .ok()
            .unwrap();
        let text = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        // n=1 was already being written when its caller gave up, so it lands;
        // n=2 never started and is dropped; n=3 is written after n=1.
        assert_eq!(text, "{\"n\":1}\n{\"n\":3}\n");
    }

    #[test]
    fn child_environment_is_allowlisted_plus_config() {
        let parent = vec![
            (OsString::from("PATH"), OsString::from("/bin")),
            (OsString::from("OPENAI_API_KEY"), OsString::from("sk-x")),
            (OsString::from("HOME"), OsString::from("/home/u")),
        ];
        let mut config = BTreeMap::new();
        config.insert("FOO".to_string(), "bar".to_string());
        let env = child_environment(parent.into_iter(), &config);
        assert_eq!(
            env.get(std::ffi::OsStr::new("PATH"))
                .and_then(|value| value.to_str()),
            Some("/bin")
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new("FOO"))
                .and_then(|value| value.to_str()),
            Some("bar")
        );
        assert!(!env.contains_key(std::ffi::OsStr::new("OPENAI_API_KEY")));
    }

    #[cfg(unix)]
    #[test]
    fn child_environment_tolerates_non_unicode_values_and_keys() {
        use std::os::unix::ffi::OsStringExt;

        let invalid_value = OsString::from_vec(vec![0xff, b'x']);
        let invalid_key = OsString::from_vec(vec![0xfe, b'K']);
        let parent = vec![
            (OsString::from("PATH"), invalid_value.clone()),
            (invalid_key, OsString::from("ignored")),
        ];
        let env = child_environment(parent.into_iter(), &BTreeMap::new());
        assert_eq!(env.get(std::ffi::OsStr::new("PATH")), Some(&invalid_value));
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn security_stdout_line_is_bounded_before_buffering() {
        for terminated in [false, true] {
            let mut input = vec![b'x'; 1024];
            if terminated {
                input.push(b'\n');
            }
            let mut reader = std::io::Cursor::new(input);
            assert!(read_stdout_line(&mut reader, 64).is_err());
            assert!(reader.position() <= 65, "read beyond the line limit");
        }
        let mut reader = std::io::Cursor::new("é\r\nnext\nlast");
        assert_eq!(read_stdout_line(&mut reader, 4).unwrap(), Some("é".into()));
        assert_eq!(
            read_stdout_line(&mut reader, 5).unwrap(),
            Some("next".into())
        );
        assert_eq!(
            read_stdout_line(&mut reader, 4).unwrap(),
            Some("last".into())
        );
        assert_eq!(read_stdout_line(&mut reader, 4).unwrap(), None);
    }

    #[test]
    fn security_stdout_queue_is_bounded_and_drop_unblocks_sender() {
        let (sender, receiver) = stdout_channel();
        for _ in 0..MAX_QUEUED_STDOUT_LINES {
            sender.try_send(Ok("notification".repeat(4))).unwrap();
        }
        assert!(matches!(
            sender.try_send(Ok("overflow".into())),
            Err(mpsc::TrySendError::Full(_))
        ));
        let (done_sender, done_receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let failed = sender.send(Ok("pending".into())).is_err();
            let _ = done_sender.send(failed);
        });
        drop(receiver);
        assert!(done_receiver.recv_timeout(Duration::from_secs(5)).unwrap());
        handle.join().unwrap();
    }

    #[test]
    fn a_bare_command_resolves_through_pathext_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("npx.exe"), b"").unwrap();
        std::fs::write(second.join("npx.cmd"), b"").unwrap();
        std::fs::write(first.join("npx.cmd"), b"").unwrap();
        let exts = vec![".exe".to_string(), ".cmd".to_string()];
        // The first directory wins even though it only has the `.cmd`.
        assert_eq!(
            resolve_command_in("npx", &[first.clone(), second.clone()], &exts),
            Some(first.join("npx.cmd"))
        );
        // Within one directory the PATHEXT order wins.
        assert_eq!(
            resolve_command_in("npx", &[second.clone()], &exts),
            Some(second.join("npx.exe"))
        );
        assert_eq!(resolve_command_in("missing", &[first], &exts), None);
        // Explicit extensions and paths are left alone.
        assert_eq!(
            resolve_command_in("npx.cmd", &[second.clone()], &exts),
            Some(PathBuf::from("npx.cmd"))
        );
        assert_eq!(
            resolve_command_in("./npx", &[second], &exts),
            Some(PathBuf::from("./npx"))
        );
    }
}
