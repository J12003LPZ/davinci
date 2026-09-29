use crate::jobs::supervisor::{
    ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand,
};
use davinci_mcp::{Error, Result, RpcTransport, ServerConfig};
use davinci_protocol::SandboxSpec;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const WRITE_CHUNK: usize = 16 * 1024;
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct State {
    stdout: Vec<u8>,
    responses: BTreeMap<u64, Value>,
    server_requests: VecDeque<Value>,
    failure: Option<String>,
    closed: bool,
    stderr: Vec<u8>,
}

impl State {
    fn receive_stdout(&mut self, bytes: &[u8]) {
        if self.failure.is_some() {
            return;
        }
        self.stdout.extend_from_slice(bytes);
        if self.stdout.len() > MAX_LINE_BYTES && !self.stdout.contains(&b'\n') {
            self.failure = Some(format!("MCP stdout line exceeds {MAX_LINE_BYTES} bytes"));
            return;
        }
        loop {
            let Some(end) = self.stdout.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            let mut line = self.stdout.drain(..=end).collect::<Vec<_>>();
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_BYTES {
                self.failure = Some(format!("MCP stdout line exceeds {MAX_LINE_BYTES} bytes"));
                return;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            let Some(object) = value.as_object() else {
                continue;
            };
            if object.get("method").and_then(Value::as_str).is_some() {
                if object.get("id").is_some() {
                    self.server_requests.push_back(value);
                }
                continue;
            }
            let Some(id) = object.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if object.contains_key("result") || object.contains_key("error") {
                self.responses.insert(id, value);
            }
        }
    }

    fn receive_stderr(&mut self, bytes: &[u8]) {
        if bytes.len() >= MAX_STDERR_BYTES {
            self.stderr.clear();
            self.stderr
                .extend_from_slice(&bytes[bytes.len() - MAX_STDERR_BYTES..]);
            return;
        }
        let required = self.stderr.len().saturating_add(bytes.len());
        if required > MAX_STDERR_BYTES {
            self.stderr.drain(..required - MAX_STDERR_BYTES);
        }
        self.stderr.extend_from_slice(bytes);
    }
}

pub struct SupervisedMcpTransport {
    supervisor: Arc<Supervisor>,
    state: Arc<(Mutex<State>, Condvar)>,
    next_id: u64,
    call_timeout: Duration,
}

impl SupervisedMcpTransport {
    pub fn spawn(
        host: &SupervisorCommand,
        server: &ServerConfig,
        cwd: &Path,
        sandbox: &SandboxSpec,
    ) -> Result<Self> {
        let cwd = cwd
            .canonicalize()
            .map_err(|error| Error::Transport(format!("sandboxed MCP cwd: {error}")))?;
        let command = server
            .command
            .as_deref()
            .ok_or_else(|| Error::Protocol("sandboxed MCP requires a local command".into()))?;
        let executable = resolve_program(command, &cwd)
            .map_err(|error| Error::Transport(format!("sandboxed MCP executable: {error}")))?;

        let mut environment = crate::sandbox::sanitize_current_environment(&sandbox.environment)
            .map_err(|error| Error::Transport(format!("sandboxed MCP environment: {error}")))?;
        for name in &sandbox.environment.allow {
            if sandbox.environment.inject.contains_key(name) {
                continue;
            }
            if let Some(value) = server.env.get(name) {
                environment.insert(name.clone(), value.clone());
            }
        }

        let config = ProcessConfig::new(executable, server.args.clone(), cwd, environment)
            .with_sandbox(sandbox.clone())
            .as_background();

        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let stdout_state = Arc::clone(&state);
        let stderr_state = Arc::clone(&state);
        let supervisor = Arc::new(
            Supervisor::spawn_with_stderr(
                host,
                config,
                Arc::new(move |event| {
                    let (lock, changed) = &*stdout_state;
                    let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                    match event {
                        ProcessEvent::Output(bytes) => state.receive_stdout(&bytes),
                        ProcessEvent::Finished(exit) => {
                            state.closed = true;
                            if exit.code != Some(0) || exit.error.is_some() || exit.stopped {
                                state.failure = Some(exit.error.unwrap_or_else(|| {
                                    format!("sandboxed MCP exited with {:?}", exit.code)
                                }));
                            }
                        }
                    }
                    changed.notify_all();
                }),
                Arc::new(move |bytes| {
                    let (lock, changed) = &*stderr_state;
                    let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                    state.receive_stderr(&bytes);
                    changed.notify_all();
                }),
            )
            .map_err(|error| Error::Transport(format!("sandboxed MCP spawn: {error}")))?,
        );

        Ok(Self {
            supervisor,
            state,
            next_id: 1,
            call_timeout: DEFAULT_CALL_TIMEOUT,
        })
    }

    fn write_value(&self, value: &Value) -> Result<()> {
        let mut bytes =
            serde_json::to_vec(value).map_err(|error| Error::Protocol(format!("encode: {error}")))?;
        bytes.push(b'\n');
        for chunk in bytes.chunks(WRITE_CHUNK) {
            self.supervisor
                .write(chunk)
                .map_err(|error| Error::Transport(format!("sandboxed MCP stdin: {error}")))?;
        }
        Ok(())
    }

    fn answer_server_request(&self, request: Value) -> Result<()> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let reply = match method {
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "roots/list" => json!({"jsonrpc":"2.0","id":id,"result":{"roots":[]}}),
            _ => json!({
                "jsonrpc":"2.0",
                "id":id,
                "error":{"code":-32601,"message":"Method not found"}
            }),
        };
        self.write_value(&reply)
    }

    fn wait_response(&self, id: u64) -> Result<Value> {
        let deadline = Instant::now() + self.call_timeout;
        loop {
            let requests = {
                let (lock, _) = &*self.state;
                let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                state.server_requests.drain(..).collect::<Vec<_>>()
            };
            for request in requests {
                self.answer_server_request(request)?;
            }

            let (lock, changed) = &*self.state;
            let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(response) = state.responses.remove(&id) {
                return decode_response(response, id);
            }
            if let Some(error) = state.failure.clone() {
                return Err(Error::Transport(format!(
                    "{error}{}",
                    stderr_excerpt(&state.stderr)
                )));
            }
            if state.closed {
                return Err(Error::Transport(format!(
                    "sandboxed MCP closed before response{}",
                    stderr_excerpt(&state.stderr)
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Transport(format!(
                    "mcp call timed out after {}s{}",
                    self.call_timeout.as_secs(),
                    stderr_excerpt(&state.stderr)
                )));
            }
            let wait = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(50));
            let (next, _) = changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|error| error.into_inner());
            drop(next);
        }
    }
}

impl RpcTransport for SupervisedMcpTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::Transport("MCP request id exhausted".into()))?;
        self.write_value(&json!({
            "jsonrpc":"2.0",
            "id":id,
            "method":method,
            "params":params
        }))?;
        self.wait_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_value(&json!({
            "jsonrpc":"2.0",
            "method":method,
            "params":params
        }))
    }

    fn set_call_timeout(&mut self, timeout: Duration) {
        self.call_timeout = timeout;
    }

    fn stderr_tail(&self) -> Option<String> {
        let state = self
            .state
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Some(String::from_utf8_lossy(&state.stderr).into_owned())
    }
}

fn decode_response(response: Value, expected_id: u64) -> Result<Value> {
    if response.get("id").and_then(Value::as_u64) != Some(expected_id) {
        return Err(Error::Protocol("MCP response id mismatch".into()));
    }
    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("MCP server error")
            .to_string();
        return Err(Error::Rpc { code, message });
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Error::Protocol("MCP response has no result".into()))
}

fn stderr_excerpt(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        let start = lines.len().saturating_sub(3);
        format!(" (stderr: {})", lines[start..].join(" | "))
    }
}

fn resolve_program(command: &str, cwd: &Path) -> std::result::Result<PathBuf, String> {
    if command.is_empty() || command.contains('\0') {
        return Err("invalid command".into());
    }
    let command_path = Path::new(command);
    if command_path.is_absolute() || command.contains(['/', '\\']) {
        let path = if command_path.is_absolute() {
            command_path.to_path_buf()
        } else {
            cwd.join(command_path)
        };
        return path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(|path| {
                if path.is_file() {
                    Ok(path)
                } else {
                    Err(format!("{} is not a file", path.display()))
                }
            });
    }

    let path = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
    #[cfg(windows)]
    let extensions = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    #[cfg(not(windows))]
    let extensions = vec![String::new()];

    for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
        for extension in &extensions {
            let candidate = if extension.is_empty() {
                directory.join(command)
            } else {
                directory.join(format!("{command}{extension}"))
            };
            if let Ok(canonical) = candidate.canonicalize() {
                if canonical.is_file() {
                    return Ok(canonical);
                }
            }
        }
    }
    Err(format!("command '{command}' was not found on PATH"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_response_preserves_result_and_rpc_error_class() {
        assert_eq!(
            decode_response(json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}}), 7).unwrap(),
            json!({"ok":true})
        );
        let error = decode_response(
            json!({"jsonrpc":"2.0","id":7,"error":{"code":-32602,"message":"bad"}}),
            7,
        )
        .unwrap_err();
        assert!(matches!(error, Error::Rpc { code: -32602, .. }));
    }

    #[test]
    fn stdout_parser_ignores_logs_and_queues_server_requests() {
        let mut state = State::default();
        state.receive_stdout(
            b"log line\n{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"ok\":true}}\n",
        );
        assert_eq!(state.server_requests.len(), 1);
        assert_eq!(
            state.responses.get(&9).and_then(|value| value.get("result")),
            Some(&json!({"ok":true}))
        );
        assert!(state.failure.is_none());
    }

    #[test]
    fn oversized_unterminated_stdout_fails_boundedly() {
        let mut state = State::default();
        state.receive_stdout(&vec![b'x'; MAX_LINE_BYTES + 1]);
        assert!(state
            .failure
            .as_deref()
            .is_some_and(|message| message.contains("exceeds")));
    }
}
