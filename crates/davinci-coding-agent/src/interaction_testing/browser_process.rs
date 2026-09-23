//! Host-owned JSONL browser transport on the existing OS process supervisor.
//! This is not an authorization boundary: native dispatch must supply authority.

use davinci_agent::jobs::supervisor::{ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    time::{Duration, Instant},
};

use super::artifacts::{compute_sha256, ArtifactBudgetTracker};

const MAX_RESPONSE: usize = 64 * 1024;
const MAX_REQUEST: usize = 16 * 1024;
const FILES: [(&str, &str); 4] = [
    ("browser_host.js", include_str!("browser_host.js")),
    ("browser_backend.js", include_str!("browser_backend.js")),
    ("browser_network.js", include_str!("browser_network.js")),
    ("browser_transport.js", include_str!("browser_transport.js")),
];

#[derive(Default)]
struct State {
    frame: Vec<u8>,
    sequence: u64,
    pending: HashMap<u64, Option<Result<Value, String>>>,
    failure: Option<String>,
    closed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    id: u64,
    ok: bool,
    result: Option<Value>,
    error: Option<String>,
}

impl State {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.failure.is_some() {
            return Err("browser transport failed".into());
        }
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.frame.len().saturating_add(part.len()) > MAX_RESPONSE {
                return Err("browser response exceeds limit".into());
            }
            self.frame.extend_from_slice(part);
            if part.last() != Some(&b'\n') {
                continue;
            }
            let response: Response =
                serde_json::from_slice(&self.frame).map_err(|_| "invalid browser response")?;
            self.frame.clear();
            let slot = self
                .pending
                .get_mut(&response.id)
                .filter(|slot| slot.is_none())
                .ok_or("unknown or repeated browser response")?;
            *slot = Some(match (response.ok, response.result, response.error) {
                (true, Some(mut result), None) => {
                    // Bind provenance to the correlated request, never to a field
                    // supplied by the backend or the page being inspected.
                    result
                        .as_object_mut()
                        .ok_or("invalid browser result")?
                        .insert("action_sequence".into(), json!(response.id));
                    Ok(result)
                }
                (false, None, Some(_)) => Err("browser request failed".into()),
                _ => return Err("inconsistent browser response".into()),
            });
        }
        Ok(())
    }
}

/// Pinned dependency paths are trusted host settings, never model arguments.
pub struct BrowserProcessConfig<'a> {
    pub node: &'a Path,
    pub package: &'a Path,
    pub version: &'a str,
    pub workspace: &'a Path,
    pub environment: BTreeMap<String, String>,
}

pub struct BrowserProcess {
    supervisor: Arc<Supervisor>,
    state: Arc<(Mutex<State>, Condvar)>,
    writer: Mutex<()>,
    directory: PathBuf,
}

impl BrowserProcess {
    /// A failed or exited transport cannot own reusable live page state.
    pub fn is_healthy(&self) -> bool {
        let state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        state.failure.is_none() && !state.closed
    }

    /// Launches only after the caller's current browser request is authorized.
    pub fn start(
        host: &SupervisorCommand,
        config: BrowserProcessConfig<'_>,
    ) -> Result<Self, String> {
        let workspace = config
            .workspace
            .canonicalize()
            .map_err(|_| "browser workspace unavailable")?;
        let node = config
            .node
            .canonicalize()
            .map_err(|_| "browser Node executable unavailable")?;
        let package = config
            .package
            .canonicalize()
            .map_err(|_| "browser dependency unavailable")?;
        if node.starts_with(&workspace)
            || package.starts_with(&workspace)
            || config.version.len() > 64
        {
            return Err("browser dependencies must be pinned outside the workspace".into());
        }
        let directory =
            std::env::temp_dir().join(format!("davinci-browser-{}", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(|_| "browser private directory unavailable")?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&directory).map_err(|_| "browser private directory unavailable")?;
        let launch = (|| {
            let diagnostics = std::env::var_os("DAVINCI_BROWSER_DIAGNOSTICS").is_some();
            if directory
                .canonicalize()
                .map_err(|_| "browser private directory unavailable")?
                .starts_with(&workspace)
            {
                return Err("browser host directory must be outside the workspace".into());
            }
            if config.environment.keys().any(|name| {
                !matches!(
                    name.as_str(),
                    "PATH"
                        | "SystemRoot"
                        | "WINDIR"
                        | "TEMP"
                        | "TMP"
                        | "TMPDIR"
                        | "PLAYWRIGHT_BROWSERS_PATH"
                        | "DAVINCI_BROWSER_DIAGNOSTICS"
                )
            }) {
                return Err("unsupported browser host environment variable".into());
            }
            if let Some(cache) = config.environment.get("PLAYWRIGHT_BROWSERS_PATH") {
                let cache = Path::new(cache);
                if !cache.is_absolute()
                    || cache
                        .canonicalize()
                        .map_err(|_| "browser cache unavailable")?
                        .starts_with(&workspace)
                {
                    return Err("browser cache must be outside the workspace".into());
                }
            }
            let mut environment = config.environment;
            if diagnostics {
                environment.insert("DAVINCI_BROWSER_DIAGNOSTICS".into(), "1".into());
            }
            // Windows Node crypto initialization needs SystemRoot. Preserve the
            // supervisor's platform-path allowlist, never loader/credential env.
            for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
                if let Ok(value) = std::env::var(name) {
                    environment.entry(name.into()).or_insert(value);
                }
            }
            for (name, bytes) in FILES {
                fs::write(directory.join(name), bytes)
                    .map_err(|_| "cannot materialize browser host")?;
            }
            fs::write(
                directory.join("config.json"),
                serde_json::to_vec(&json!({
                    "packagePath": package, "version": config.version, "workspace": workspace
                }))
                .map_err(|_| "invalid browser host configuration")?,
            )
            .map_err(|_| "cannot configure browser host")?;
            let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
            let event_state = state.clone();
            let owner: Arc<Mutex<Option<Weak<Supervisor>>>> = Arc::new(Mutex::new(None));
            let event_owner = owner.clone();
            let supervisor = Arc::new(Supervisor::spawn_with_stderr(
                host,
                ProcessConfig {
                    executable: node,
                    argv: vec![
                        directory
                            .join("browser_host.js")
                            .to_string_lossy()
                            .into_owned(),
                        directory.join("config.json").to_string_lossy().into_owned(),
                    ],
                    cwd: directory.clone(),
                    environment,
                    operation: None,
                },
                Arc::new(move |event| {
                    let mut state = event_state.0.lock().unwrap_or_else(|e| e.into_inner());
                    let failure = match event {
                        ProcessEvent::Output(bytes) => state.receive(&bytes).err(),
                        ProcessEvent::Finished(exit) => {
                            state.closed = true;
                            if exit.code != Some(0)
                                || exit.error.is_some()
                                || !exit.output_complete
                                || !state.frame.is_empty()
                            {
                                Some("browser host exited without complete output".into())
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(failure) = failure {
                        state.failure = Some(failure);
                    }
                    let failed = state.failure.is_some();
                    event_state.1.notify_all();
                    drop(state);
                    if failed {
                        if let Some(owner) = event_owner
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .as_ref()
                            .and_then(Weak::upgrade)
                        {
                            owner.stop();
                        }
                    }
                }),
                Arc::new(move |bytes| {
                    if diagnostics {
                        let detail = String::from_utf8_lossy(&bytes);
                        eprint!(
                            "browser host stderr: {}",
                            detail.chars().take(1024).collect::<String>()
                        );
                    }
                }),
            )
            .map_err(|error| error.to_string())?);
            *owner.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(&supervisor));
            if state
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .failure
                .is_some()
            {
                supervisor.stop();
                return Err("browser host startup failed".into());
            }
            Ok(Self {
                supervisor,
                state,
                writer: Mutex::new(()),
                directory: directory.clone(),
            })
        })();
        if launch.is_err() {
            clean_directory(&directory);
        }
        launch
    }

    /// Correlates bounded responses, including out-of-order context operations.
    /// An unknown delivery outcome invalidates the transport; it is never retried.
    pub fn request(&self, request: Value, timeout: Duration) -> Result<Value, String> {
        self.request_with_abort(request, timeout, None)
    }

    pub fn request_with_abort(
        &self,
        mut request: Value,
        timeout: Duration,
        abort: Option<&AtomicBool>,
    ) -> Result<Value, String> {
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err("invalid browser request deadline".into());
        }
        if abort.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err("browser request cancelled".into());
        }
        let started = Instant::now();
        let control = request["op"] == "cancel";
        let writer = loop {
            match self.writer.try_lock() {
                Ok(writer) => break writer,
                Err(std::sync::TryLockError::WouldBlock)
                    if control && started.elapsed() < timeout.min(Duration::from_secs(1)) =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => return Err("browser input is busy".into()),
            }
        };
        let mut state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(failure) = &state.failure {
            return Err(failure.clone());
        }
        if state.closed {
            return Err("browser host is closed".into());
        }
        if state.pending.len() >= if control { 32 } else { 16 } {
            return Err("browser pending request limit".into());
        }
        let id = state
            .sequence
            .checked_add(1)
            .ok_or("browser request sequence exhausted")?;
        let object = request.as_object_mut().ok_or("invalid browser request")?;
        if object.contains_key("id") {
            return Err("browser correlation is host-owned".into());
        }
        object.insert("id".into(), json!(id));
        let mut bytes = serde_json::to_vec(&request).map_err(|_| "invalid browser request")?;
        if bytes.len() > MAX_REQUEST {
            return Err("browser request exceeds limit".into());
        }
        bytes.push(b'\n');
        state.sequence = id;
        state.pending.insert(id, None);
        drop(state);
        for chunk in bytes.chunks(MAX_REQUEST) {
            if self.supervisor.write(chunk).is_err() {
                self.invalidate("browser input delivery failed");
                return Err("browser input delivery failed".into());
            }
        }
        drop(writer);
        let mut state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(failure) = &state.failure {
                return Err(failure.clone());
            }
            if abort.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                drop(state);
                // The control response follows target completion on the same
                // ordered stream, so correlation can be retired without a
                // tombstone or retry of an unknown action outcome.
                let cancelled =
                    self.request(json!({"op":"cancel","target":id}), Duration::from_secs(5));
                if !cancelled
                    .as_ref()
                    .is_ok_and(|value| value["cancelled"].is_boolean())
                {
                    self.invalidate("browser cancellation unconfirmed");
                    return Err("browser cancellation unconfirmed".into());
                }
                self.state
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pending
                    .remove(&id);
                return Err("browser request cancelled".into());
            }
            // A complete response may precede the process's orderly shutdown.
            if let Some(Some(_)) = state.pending.get(&id) {
                return state.pending.remove(&id).unwrap().unwrap();
            }
            if state.closed {
                return Err("browser host exited before responding".into());
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                drop(state);
                self.invalidate("browser response deadline exceeded");
                return Err("browser response deadline exceeded".into());
            }
            state = self
                .state
                .1
                .wait_timeout(
                    state,
                    if abort.is_some() {
                        remaining.min(Duration::from_millis(10))
                    } else {
                        remaining
                    },
                )
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn invalidate(&self, reason: &str) {
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failure = Some(reason.into());
        self.state.1.notify_all();
        self.supervisor.stop();
    }

    /// Imports screenshot bytes into the existing tracker before exposing a ref.
    pub fn retain_screenshot(
        &self,
        result: &Value,
        tracker: &mut ArtifactBudgetTracker,
    ) -> Result<Value, String> {
        let sequence = result["action_sequence"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or("missing screenshot action sequence")?;
        let reference = result["artifact"]
            .as_str()
            .ok_or("missing screenshot transfer")?;
        let (transfer, hash) = reference
            .strip_prefix("staging:")
            .and_then(|value| value.split_once(':'))
            .filter(|(transfer, hash)| valid_hash(transfer) && valid_hash(hash))
            .ok_or("invalid screenshot transfer")?;
        let size = result["size"]
            .as_u64()
            .filter(|size| *size <= 4 * 1024 * 1024)
            .ok_or("invalid screenshot size")?;
        if result["mediaType"] != "image/png" {
            return Err("invalid screenshot media type".into());
        }
        let file = self.directory.join(format!("{transfer}.png"));
        let metadata =
            fs::symlink_metadata(&file).map_err(|_| "screenshot transfer unavailable")?;
        if !metadata.is_file() || metadata.len() != size {
            return Err("screenshot transfer changed".into());
        }
        let mut bytes = Vec::new();
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let opened = options
            .open(&file)
            .map_err(|_| "screenshot transfer unavailable")?;
        if !opened
            .metadata()
            .map_err(|_| "screenshot transfer unavailable")?
            .is_file()
        {
            return Err("invalid screenshot transfer file".into());
        }
        opened
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "screenshot transfer unavailable")?;
        if bytes.len() as u64 != size || compute_sha256(&bytes) != hash {
            return Err("screenshot transfer changed".into());
        }
        let label = format!("browser/{}.png", uuid::Uuid::new_v4());
        if !tracker.store_artifact(&label, bytes) {
            return Err("screenshot retention budget exceeded".into());
        }
        fs::remove_file(file).map_err(|_| "screenshot transfer cleanup failed")?;
        Ok(
            json!({"artifact":label,"sha256":hash,"size":size,"mediaType":"image/png",
            "action_sequence":sequence}),
        )
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        self.supervisor.stop();
        if self.supervisor.wait(Duration::from_secs(2)).is_some() {
            clean_directory(&self.directory);
        } else {
            let supervisor = self.supervisor.clone();
            let directory = self.directory.clone();
            std::thread::spawn(move || {
                if supervisor.wait(Duration::from_secs(30)).is_some() {
                    clean_directory(&directory);
                }
            });
        }
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn clean_directory(directory: &Path) {
    for (name, _) in FILES {
        let _ = fs::remove_file(directory.join(name));
    }
    let _ = fs::remove_file(directory.join("config.json"));
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name
                .to_str()
                .and_then(|name| name.strip_suffix(".png"))
                .is_some_and(valid_hash)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let _ = fs::remove_dir(directory);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_entry() {
        if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
            davinci_agent::jobs::supervisor::run();
        }
    }

    #[test]
    #[ignore = "requires explicitly configured trusted Node and Playwright installation"]
    fn supervised_chromium_actions_retention_and_cleanup() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Server(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
                self.1.take().unwrap().join().unwrap();
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let _server = Server(
            stop,
            Some(std::thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    if let Ok((mut stream, _)) = listener.accept() {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0; 4096];
                        let _ = stream.read(&mut request);
                        let body = "<html><body><button onclick=\"this.textContent='Done'\">Start</button></body></html>";
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    } else {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            })),
        );
        let workspace = tempfile::tempdir().unwrap();
        let node = PathBuf::from(std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH").unwrap());
        let package = PathBuf::from(std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH").unwrap());
        let bridge = BrowserProcess::start(
            &SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "interaction_testing::browser_process::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            },
            BrowserProcessConfig {
                node: &node,
                package: &package,
                version: "1.62.0",
                workspace: workspace.path(),
                environment: BTreeMap::new(),
            },
        )
        .unwrap();
        let directory = bridge.directory.clone();
        let request = |value| bridge.request(value, Duration::from_secs(30)).unwrap();
        let opened = request(json!({"op":"open","options":{"origins":[origin]}}));
        assert!(opened["browserVersion"].as_str().unwrap().contains('.'));
        let resource = opened["resource"].as_u64().unwrap();
        request(
            json!({"op":"execute","resource":resource,"command":{"action":"navigate","url":origin}}),
        );
        request(
            json!({"op":"execute","resource":resource,"command":{"action":"click","selector":{"kind":"role","role":"button","name":"Start"}}}),
        );
        let snapshot =
            request(json!({"op":"execute","resource":resource,"command":{"action":"snapshot"}}));
        assert!(snapshot["html"]
            .as_str()
            .unwrap()
            .contains(">Done</button>"));
        let screenshot =
            request(json!({"op":"execute","resource":resource,"command":{"action":"screenshot"}}));
        assert!(!screenshot.to_string().contains("bytes"));
        let mut tracker = ArtifactBudgetTracker::new(50 * 1024 * 1024);
        let mut wrong_hash = screenshot.clone();
        let transfer = screenshot["artifact"]
            .as_str()
            .unwrap()
            .split(':')
            .nth(1)
            .unwrap();
        wrong_hash["artifact"] = json!(format!("staging:{transfer}:{}", "0".repeat(64)));
        assert!(bridge.retain_screenshot(&wrong_hash, &mut tracker).is_err());
        assert_eq!(tracker.total_bytes, 0);
        let mut traversal = screenshot.clone();
        traversal["artifact"] = json!("staging:../../outside:invalid");
        assert!(bridge.retain_screenshot(&traversal, &mut tracker).is_err());
        let retained = bridge.retain_screenshot(&screenshot, &mut tracker).unwrap();
        let bytes = &tracker.items[retained["artifact"].as_str().unwrap()];
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(compute_sha256(bytes), retained["sha256"]);
        assert_eq!(retained["action_sequence"], screenshot["action_sequence"]);
        assert!(bridge.retain_screenshot(&screenshot, &mut tracker).is_err());
        let other = request(json!({"op":"open","options":{"origins":[origin]}}))["resource"]
            .as_u64()
            .unwrap();
        let abort = AtomicBool::new(true);
        let sequence = bridge.state.0.lock().unwrap().sequence;
        assert_eq!(
            bridge.request_with_abort(
                json!({"op":"execute","resource":resource,"command":{"action":"snapshot"}}),
                Duration::from_secs(10),
                Some(&abort)
            ),
            Err("browser request cancelled".into())
        );
        assert_eq!(bridge.state.0.lock().unwrap().sequence, sequence);
        abort.store(false, Ordering::SeqCst);
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let deadline = Instant::now() + Duration::from_secs(2);
                while bridge.state.0.lock().unwrap().pending.is_empty() {
                    assert!(Instant::now() < deadline, "action was not admitted");
                    std::thread::sleep(Duration::from_millis(5));
                }
                // Give Chromium time to begin waiting for the absent selector.
                std::thread::sleep(Duration::from_millis(100));
                abort.store(true, Ordering::SeqCst);
            });
            assert_eq!(bridge.request_with_abort(json!({"op":"execute","resource":resource,"command":{"action":"click","selector":{"kind":"role","role":"button","name":"Missing"}}}), Duration::from_secs(10), Some(&abort)), Err("browser request cancelled".into()));
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        {
            let state = bridge.state.0.lock().unwrap();
            assert!(state.pending.is_empty());
            assert!(state.failure.is_none());
        }
        request(
            json!({"op":"execute","resource":other,"command":{"action":"navigate","url":origin}}),
        );
        let surviving =
            request(json!({"op":"execute","resource":other,"command":{"action":"snapshot"}}));
        assert!(surviving["html"]
            .as_str()
            .unwrap()
            .contains(">Start</button>"));
        request(json!({"op":"close","resource":other}));
        request(json!({"op":"shutdown"}));
        assert_eq!(
            bridge.supervisor.wait(Duration::from_secs(5)).unwrap().code,
            Some(0)
        );
        drop(bridge);
        assert!(!directory.exists());
    }

    #[test]
    fn response_frames_are_bounded_correlated_and_replay_safe() {
        let mut state = State::default();
        state.pending.insert(1, None);
        state.pending.insert(2, None);
        state.receive(b"{\"id\":2,\"ok\":true,\"result\":").unwrap();
        state
            .receive(b"{}}\n{\"id\":1,\"ok\":false,\"error\":\"secret\"}\n")
            .unwrap();
        assert_eq!(state.pending[&2], Some(Ok(json!({"action_sequence":2}))));
        assert_eq!(
            state.pending[&1],
            Some(Err("browser request failed".into()))
        );
        assert!(state
            .receive(b"{\"id\":2,\"ok\":true,\"result\":{}}\n")
            .is_err());
        let mut state = State::default();
        assert!(state.receive(&vec![b'x'; MAX_RESPONSE + 1]).is_err());
        assert!(state.frame.len() <= MAX_RESPONSE);
    }

    #[test]
    fn response_sequence_is_host_owned_and_tracks_out_of_order_completion() {
        let mut state = State::default();
        state.pending.insert(1, None);
        state.pending.insert(2, None);
        state.receive(b"{\"id\":2,\"ok\":true,\"result\":{\"action_sequence\":999}}\n{\"id\":1,\"ok\":true,\"result\":{}}\n").unwrap();
        assert_eq!(state.pending[&2], Some(Ok(json!({"action_sequence":2}))));
        assert_eq!(state.pending[&1], Some(Ok(json!({"action_sequence":1}))));
    }

    #[test]
    fn malformed_response_cannot_complete_a_request() {
        for bytes in [
            b"{\"id\":1,\"ok\":true,\"result\":null}\n".as_slice(),
            b"{\"id\":1,\"ok\":true,\"result\":{},\"extra\":1}\n".as_slice(),
            b"{\"id\":1,\"ok\":true,\"result\":{},\"error\":\"wrong\"}\n",
            b"{\"id\":1,\"ok\":false}\n",
            b"{\"id\":99,\"ok\":true,\"result\":{}}\n",
            b"\xff\n",
        ] {
            let mut state = State::default();
            state.pending.insert(1, None);
            assert!(state.receive(bytes).is_err());
            assert!(state.pending[&1].is_none());
        }
    }
}
