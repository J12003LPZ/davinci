use crate::jobs::supervisor::{ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand};
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
/// Server-to-client requests are answered only while a call waits, so they
/// queue between calls. A well-behaved server sends a few pings at most;
/// past either bound the transport fails instead of growing the host.
const MAX_QUEUED_SERVER_REQUESTS: usize = 64;
const MAX_QUEUED_SERVER_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct State {
    stdout: Vec<u8>,
    /// The id of the one call waiting for a reply. Calls are sequential, so
    /// any other id (a late reply to a timed-out call, or one never asked
    /// for) is dropped rather than retained.
    awaiting: Option<u64>,
    responses: BTreeMap<u64, Value>,
    server_requests: VecDeque<Value>,
    server_request_bytes: usize,
    failure: Option<String>,
    closed: bool,
    stderr: Vec<u8>,
}

impl State {
    fn queue_server_request(&mut self, value: Value, bytes: usize) {
        if self.server_requests.len() >= MAX_QUEUED_SERVER_REQUESTS
            || self.server_request_bytes.saturating_add(bytes) > MAX_QUEUED_SERVER_REQUEST_BYTES
        {
            self.server_requests.clear();
            self.server_request_bytes = 0;
            self.failure = Some(format!(
                "MCP server queued more than {MAX_QUEUED_SERVER_REQUESTS} unanswered requests \
                 or {MAX_QUEUED_SERVER_REQUEST_BYTES} bytes of them"
            ));
            return;
        }
        self.server_request_bytes += bytes;
        self.server_requests.push_back(value);
    }

    fn take_server_requests(&mut self) -> Vec<Value> {
        self.server_request_bytes = 0;
        self.server_requests.drain(..).collect()
    }

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
                    self.queue_server_request(value, line.len());
                    if self.failure.is_some() {
                        self.stdout = Vec::new();
                        return;
                    }
                }
                continue;
            }
            let Some(id) = object.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if self.awaiting == Some(id)
                && (object.contains_key("result") || object.contains_key("error"))
            {
                self.responses.entry(id).or_insert(value);
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

/// The server's configured `env`. Literal values pass as written; a
/// `${NAME}` reference resolves only for host variables the sandbox policy
/// allows, so a project-supplied server cannot pull ambient credentials into
/// the sandbox. Variables the policy injects (HOME, TMPDIR...) are not
/// overridable.
fn server_environment(
    server: &ServerConfig,
    sandbox: &SandboxSpec,
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    for (name, template) in &server.env {
        if sandbox.environment.inject.contains_key(name) {
            continue;
        }
        let valid = !name.is_empty()
            && !name.as_bytes()[0].is_ascii_digit()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !valid {
            return Err(Error::Protocol(format!(
                "sandboxed MCP env has an invalid variable name: {name}"
            )));
        }
        let denied = std::cell::RefCell::new(None::<String>);
        let value = davinci_mcp::expand_env(template, |reference| {
            if sandbox
                .environment
                .allow
                .iter()
                .any(|allowed| allowed == reference)
            {
                std::env::var(reference).ok()
            } else {
                denied
                    .borrow_mut()
                    .get_or_insert_with(|| reference.to_string());
                None
            }
        });
        if let Some(reference) = denied.into_inner() {
            return Err(Error::Protocol(format!(
                "sandboxed MCP env {name} references host variable {reference}, which \
                 sandbox.environment.allow does not include"
            )));
        }
        if value.contains('\0') {
            return Err(Error::Protocol(format!(
                "sandboxed MCP env {name} contains NUL"
            )));
        }
        environment.insert(name.clone(), value);
    }
    Ok(environment)
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
        environment.extend(server_environment(server, sandbox)?);

        // A service: the per-command output budget and lifetime would kill a
        // session-long server midway. Its framing limits above still apply.
        let config = ProcessConfig::new(executable, server.args.clone(), cwd, environment)
            .with_sandbox(sandbox.clone())
            .as_service();

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
        let mut bytes = serde_json::to_vec(value)
            .map_err(|error| Error::Protocol(format!("encode: {error}")))?;
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

    fn set_awaiting(&self, id: Option<u64>) {
        let (lock, _) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
        state.awaiting = id;
        state.responses.clear();
    }

    fn wait_response(&self, id: u64) -> Result<Value> {
        let deadline = Instant::now() + self.call_timeout;
        loop {
            let requests = {
                let (lock, _) = &*self.state;
                let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                state.take_server_requests()
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
                // A failed transport never recovers; stop the owned service
                // now rather than at drop.
                self.supervisor.stop();
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
        // Admit the reply before sending, so a fast server cannot beat it.
        self.set_awaiting(Some(id));
        let result = self
            .write_value(&json!({
                "jsonrpc":"2.0",
                "id":id,
                "method":method,
                "params":params
            }))
            .and_then(|()| self.wait_response(id));
        self.set_awaiting(None);
        result
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
    // Same envelope contract as the native stdio and HTTP transports.
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || response.get("method").is_some()
        || response.get("result").is_some() == response.get("error").is_some()
    {
        return Err(Error::Protocol(format!(
            "invalid MCP response envelope `{response}`"
        )));
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
    fn audit_supervised_mcp_rejects_response_without_jsonrpc_version() {
        let mut state = State {
            awaiting: Some(7),
            ..Default::default()
        };
        state.receive_stdout(b"{\"id\":7,\"result\":{\"ok\":true}}\n");
        let response = state.responses.remove(&7).unwrap();
        let decoded = decode_response(response, 7);
        assert!(
            matches!(decoded, Err(Error::Protocol(_))),
            "invalid envelope reached the consumer: {decoded:?}"
        );
    }

    #[test]
    fn supervised_mcp_rejects_every_malformed_envelope_but_keeps_null_results() {
        for bad in [
            json!({"jsonrpc":"1.0","id":7,"result":{}}),
            json!({"id":7,"result":{}}),
            json!({"jsonrpc":"2.0","id":7}),
            json!({"jsonrpc":"2.0","id":7,"result":{},"error":{"code":-1,"message":"x"}}),
            json!({"jsonrpc":"2.0","id":7,"method":"ping","result":{}}),
        ] {
            let decoded = decode_response(bad.clone(), 7);
            assert!(
                matches!(decoded, Err(Error::Protocol(_))),
                "{bad}: {decoded:?}"
            );
        }
        assert_eq!(
            decode_response(json!({"jsonrpc":"2.0","id":7,"result":null}), 7).unwrap(),
            Value::Null
        );
    }

    const FLOOD_FIXTURE_ENV: &str = "DAVINCI_MCP_IDLE_FLOOD_FIXTURE";

    /// Disposable MCP "server": floods pings and unsolicited replies while
    /// nobody calls it, then idles for a bounded time.
    #[test]
    fn idle_flood_fixture() {
        if std::env::var(FLOOD_FIXTURE_ENV).as_deref() != Ok("1") {
            return;
        }
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        for id in 1..=10_000u64 {
            let _ = writeln!(out, "{}", json!({"jsonrpc":"2.0","id":id,"method":"ping"}));
            let _ = writeln!(out, "{}", json!({"jsonrpc":"2.0","id":id,"result":{}}));
        }
        let _ = out.flush();
        drop(out);
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn supervised_idle_flood_fails_the_transport_and_stops_the_service() {
        let root = tempfile::tempdir().unwrap();
        let server = ServerConfig {
            command: Some(std::env::current_exe().unwrap().to_string_lossy().into()),
            args: vec![
                "--exact".into(),
                "mcp::sandboxed::tests::idle_flood_fixture".into(),
                "--nocapture".into(),
            ],
            env: BTreeMap::from([(FLOOD_FIXTURE_ENV.into(), "1".into())]),
            ..Default::default()
        };
        let mut spec = sandbox(&["PATH", "SYSTEMROOT", "TEMP", "TMP"]);
        spec.workspace = root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut transport = SupervisedMcpTransport::spawn(
            &crate::command_receipt::test_supervisor(),
            &server,
            root.path(),
            &spec,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = transport.state.0.lock().unwrap();
            assert!(state.responses.is_empty(), "unsolicited reply retained");
            assert!(state.server_requests.len() <= MAX_QUEUED_SERVER_REQUESTS);
            if state.failure.is_some() {
                break;
            }
            assert!(!state.closed, "fixture exited before flooding");
            drop(state);
            assert!(
                Instant::now() < deadline,
                "idle flood never tripped the bound"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let error = transport.call("tools/list", json!({})).unwrap_err();
        assert!(error.to_string().contains("unanswered requests"), "{error}");
        assert!(transport.supervisor.is_stopping());
        assert!(
            transport.supervisor.wait(Duration::from_secs(10)).is_some(),
            "owned service still running after the transport failed"
        );
    }

    #[test]
    fn idle_flood_of_replies_and_requests_stays_bounded() {
        let mut state = State::default();
        for id in 1..=10_000 {
            let frame = format!(
                "{}\n{}\n",
                json!({"jsonrpc": "2.0", "id": id, "result": {"data": "fixture"}}),
                json!({"jsonrpc": "2.0", "id": id, "method": "ping"})
            );
            state.receive_stdout(frame.as_bytes());
        }
        assert!(state.responses.is_empty(), "unsolicited replies retained");
        assert!(state.server_requests.len() <= MAX_QUEUED_SERVER_REQUESTS);
        assert!(state.stdout.is_empty());
        let failure = state.failure.as_deref().unwrap_or_default();
        assert!(failure.contains("unanswered requests"), "{failure:?}");
    }

    #[test]
    fn oversized_queued_requests_fail_by_bytes_before_count() {
        let mut state = State::default();
        let padding = "x".repeat(64 * 1024);
        for id in 1..=32 {
            let frame = format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": id, "method": "ping", "params": {"pad": padding}})
            );
            state.receive_stdout(frame.as_bytes());
        }
        assert!(state.server_request_bytes <= MAX_QUEUED_SERVER_REQUEST_BYTES);
        assert!(state.failure.is_some(), "1 MiB of queued requests accepted");
    }

    #[test]
    fn only_the_awaited_reply_is_admitted() {
        let mut state = State {
            awaiting: Some(7),
            ..Default::default()
        };
        for id in [3, 7, 7, 9] {
            let frame = format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": id, "result": {"n": id}})
            );
            state.receive_stdout(frame.as_bytes());
        }
        assert_eq!(state.responses.len(), 1);
        assert_eq!(state.responses[&7]["result"]["n"], 7);
        assert!(state.failure.is_none());
    }

    #[test]
    fn draining_requests_frees_their_budget() {
        let mut state = State::default();
        for round in 0..10 {
            for id in 0..MAX_QUEUED_SERVER_REQUESTS {
                let frame = format!(
                    "{}\n",
                    json!({"jsonrpc": "2.0", "id": round * 1000 + id, "method": "ping"})
                );
                state.receive_stdout(frame.as_bytes());
            }
            assert_eq!(
                state.take_server_requests().len(),
                MAX_QUEUED_SERVER_REQUESTS
            );
        }
        assert!(state.failure.is_none(), "{:?}", state.failure);
    }

    fn sandbox(allow: &[&str]) -> SandboxSpec {
        use davinci_protocol::{
            EnvironmentPolicy, FilesystemPolicy, NetworkPolicy, ProcessPolicy, ResourcePolicy,
            SandboxBackendKind, SandboxCapabilities, SandboxId, SandboxMode,
        };
        SandboxSpec {
            id: SandboxId("mcp-env".into()),
            mode: SandboxMode::FullAccess,
            backend: SandboxBackendKind::Host,
            container: None,
            workspace: "/workspace".into(),
            filesystem: FilesystemPolicy::default(),
            network: NetworkPolicy::Unrestricted,
            environment: EnvironmentPolicy {
                allow: allow.iter().map(|name| name.to_string()).collect(),
                inject: BTreeMap::from([("HOME".into(), "/tmp/davinci-home".into())]),
            },
            resources: ResourcePolicy::default(),
            process: ProcessPolicy::default(),
            required_capabilities: SandboxCapabilities::default(),
        }
    }

    fn server(env: &[(&str, &str)]) -> ServerConfig {
        ServerConfig {
            command: Some("server".into()),
            env: env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn configured_env_reaches_the_sandboxed_server_and_expands_allowed_references() {
        let path = std::env::var("PATH").unwrap();
        let env = server_environment(
            &server(&[("MODE", "fast"), ("SEARCH", "${PATH}"), ("HOME", "/root")]),
            &sandbox(&["PATH"]),
        )
        .unwrap();
        assert_eq!(env.get("MODE").map(String::as_str), Some("fast"));
        assert_eq!(env.get("SEARCH"), Some(&path));
        assert!(
            !env.contains_key("HOME"),
            "injected sandbox variables stay fixed"
        );
    }

    #[test]
    fn env_references_to_unallowed_host_variables_are_refused() {
        let error = server_environment(&server(&[("TOKEN", "${GITHUB_TOKEN}")]), &sandbox(&[]))
            .unwrap_err();
        assert!(error.to_string().contains("GITHUB_TOKEN"), "{error}");
    }

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
        let mut state = State {
            awaiting: Some(9),
            ..Default::default()
        };
        state.receive_stdout(
            b"log line\n{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"ok\":true}}\n",
        );
        assert_eq!(state.server_requests.len(), 1);
        assert_eq!(
            state
                .responses
                .get(&9)
                .and_then(|value| value.get("result")),
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

#[cfg(test)]
mod codemode_structured_tests {
    use super::*;

    #[test]
    fn supervised_mcp_preserves_same_result() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "content": [{"type": "text", "text": "supervised fixture"}],
                "structuredContent": {"items": [{"id": "A"}]},
                "isError": false
            }
        });
        let mut frame = serde_json::to_vec(&response).unwrap();
        frame.push(b'\n');
        let mut state = State {
            awaiting: Some(7),
            ..Default::default()
        };
        for chunk in frame.chunks(3) {
            state.receive_stdout(chunk);
        }
        assert!(state.failure.is_none());
        let decoded = decode_response(state.responses.remove(&7).unwrap(), 7).unwrap();
        let result: davinci_mcp::CallToolResult = serde_json::from_value(decoded).unwrap();
        assert_eq!(result.text(), "supervised fixture");
        assert_eq!(
            serde_json::to_value(result).unwrap()["structuredContent"],
            json!({"items": [{"id": "A"}]})
        );
    }
}
