//! Newline JSON-RPC over a child process's stdin/stdout.
//!
//! Stdout is drained by a reader thread so a server that never answers
//! cannot block the agent past the call deadline; stderr is drained into a
//! bounded tail that the error row quotes when the server dies.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::jsonrpc::{Notification, Request, Response};
use crate::{Error, Result, Rpc, CALL_TIMEOUT_SECS};

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
    stdin: ChildStdin,
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
        for (key, value) in child_environment(std::env::vars(), env) {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|err| Error::Transport(format!("spawn `{command}`: {err}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Transport("stdio server has no stdin".into()))?;
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
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let line = match read_stdout_line(&mut reader, MAX_STDOUT_LINE_BYTES) {
                    Ok(Some(line)) => Ok(line),
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                };
                if sender.send(line).is_err() {
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

    fn write_line(&mut self, value: &Value) -> Result<()> {
        let mut line =
            serde_json::to_vec(value).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        line.push(b'\n');
        if let Err(err) = self.stdin.write_all(&line).and_then(|_| self.stdin.flush()) {
            return Err(self.closed_error(&format!("mcp server stdin: {err}")));
        }
        Ok(())
    }

    /// Wait for the reply to `id`, answering server-to-client requests and
    /// skipping notifications and stray log lines on the way.
    fn read_response(&mut self, id: &Value) -> Result<Value> {
        let deadline = Instant::now() + self.call_timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(self.transport_error(&timeout_message(self.call_timeout)));
            }
            let line = match self.lines.recv_timeout(deadline - now) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(self.transport_error(&format!("mcp stdout: {error}")));
                }
                Err(RecvTimeoutError::Timeout) => {
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
            // Servers that log to stdout do not fail the call.
            let Ok(Value::Object(message)) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if message.contains_key("method") {
                // A server-to-client request or notification, never our reply
                // — even if its id collides with ours.
                if let Some(request_id) = message.get("id") {
                    self.refuse_request(request_id.clone())?;
                }
                continue;
            }
            let parsed: Response = serde_json::from_value(Value::Object(message))
                .map_err(|err| Error::Protocol(format!("decode `{trimmed}`: {err}")))?;
            if parsed.id != *id {
                continue;
            }
            if let Some(error) = parsed.error {
                return Err(Error::Rpc {
                    code: error.code,
                    message: error.message,
                });
            }
            return Ok(parsed.result.unwrap_or(Value::Null));
        }
    }

    /// The host is not a nested model: every server request (sampling,
    /// elicitation, roots) is refused with `-32601`.
    fn refuse_request(&mut self, id: Value) -> Result<()> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "method not supported" }
        }))
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

impl Rpc for StdioTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request::new(id, method, params);
        let value = serde_json::to_value(&request)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        self.write_line(&value)?;
        self.read_response(&Value::from(id))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let note = Notification::new(method, params);
        let value =
            serde_json::to_value(&note).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        self.write_line(&value)
    }
}

fn child_environment(
    parent: impl Iterator<Item = (String, String)>,
    config: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = parent
        .filter(|(key, _)| {
            INHERITED_ENV
                .iter()
                .any(|name| name.eq_ignore_ascii_case(key))
        })
        .collect();
    env.extend(
        config
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
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

    #[test]
    fn child_environment_is_allowlisted_plus_config() {
        let parent = vec![
            ("PATH".to_string(), "/bin".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-x".to_string()),
            ("HOME".to_string(), "/home/u".to_string()),
        ];
        let mut config = BTreeMap::new();
        config.insert("FOO".to_string(), "bar".to_string());
        let env = child_environment(parent.into_iter(), &config);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(env.get("FOO").map(String::as_str), Some("bar"));
        assert!(!env.contains_key("OPENAI_API_KEY"));
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
