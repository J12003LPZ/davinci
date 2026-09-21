//! On-demand local LSP process adapter for semantic navigation.

use super::documents::DocumentTracker;
use super::manager::{
    build_client_capabilities, build_initialize_params, build_sanitized_command,
    parse_server_capabilities, LspServerConfig, ServerExecutionPolicy,
};
use super::tools::{is_path_in_root, path_to_uri, uri_to_path};
use super::transport::{is_unsolicited_effect, lsp_frame, LspFrameParser, RequestTable};
use super::{SemanticBackend, SemanticSessionKey};
use davinci_agent::runtime::cache::{
    CacheDependency, CacheError, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
    SingleFlight,
};
use davinci_agent::semantic::{
    Diagnostic, DiagnosticSeverity, Location, Position, Range, RenamePreview, SemanticCapabilities,
    SemanticResult, SymbolItem,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SERVER_ERROR_CHARS: usize = 512;
const MAX_LOCAL_SESSIONS: usize = 8;

struct LocalBackendSessions {
    max_sessions: usize,
    sessions: BTreeMap<SemanticSessionKey, Arc<LocalLspSession>>,
    insertion_order: Vec<SemanticSessionKey>,
}

impl LocalBackendSessions {
    fn new(max_sessions: usize) -> Self {
        Self {
            max_sessions: max_sessions.max(1),
            sessions: BTreeMap::new(),
            insertion_order: Vec::new(),
        }
    }

    fn insert(
        &mut self,
        key: SemanticSessionKey,
        session: Arc<LocalLspSession>,
    ) -> Vec<Arc<LocalLspSession>> {
        let mut retired: Vec<_> = self.sessions.remove(&key).into_iter().collect();
        self.insertion_order.retain(|entry| entry != &key);
        while self.sessions.len() >= self.max_sessions {
            let Some(oldest) = self.insertion_order.first().cloned() else {
                break;
            };
            self.insertion_order.remove(0);
            retired.extend(self.sessions.remove(&oldest));
        }
        self.insertion_order.push(key.clone());
        self.sessions.insert(key, session);
        retired
    }
}

/// A bounded, local language-server backend used only after an explicit launch
/// request has passed the trust and permission gate.
pub struct LocalSemanticBackend {
    sessions: Mutex<LocalBackendSessions>,
    launches: SingleFlight,
    cache: CacheRuntime,
}

impl fmt::Debug for LocalSemanticBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalSemanticBackend")
            .field(
                "session_count",
                &self.sessions.lock().map(|s| s.sessions.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl Default for LocalSemanticBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalSemanticBackend {
    pub fn shared(cache: CacheRuntime) -> Arc<Self> {
        type Registry = Mutex<BTreeMap<usize, std::sync::Weak<LocalSemanticBackend>>>;
        static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
        let mut registry = REGISTRY
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, value| value.strong_count() > 0);
        let key = cache.resource_scope();
        if let Some(backend) = registry.get(&key).and_then(std::sync::Weak::upgrade) {
            return backend;
        }
        let backend = Arc::new(Self::with_cache(cache));
        if registry.len() < 64 {
            registry.insert(key, Arc::downgrade(&backend));
        }
        backend
    }

    pub(super) fn discover(
        &self,
        language: &str,
        root: &Path,
    ) -> Option<super::LanguageServerSpec> {
        let root = root.canonicalize().ok()?;
        let environment = format!(
            "{:?}",
            (
                &root,
                std::env::var_os("PATH"),
                std::env::var_os("PATHEXT"),
                std::env::var_os("DAVINCI_RUST_ANALYZER"),
                std::env::var_os("DAVINCI_TYPESCRIPT_LANGUAGE_SERVER")
            )
        );
        let key = CacheKey::new(
            CacheNamespace::Lsp,
            format!("server-discovery:{}", super::normalize_language(language)),
            1,
            "local-server-discovery-v1",
            vec![CacheDependency::ConfigHash(
                davinci_agent::runtime::cache::digest(environment.as_bytes()),
            )],
        );
        let positive = CacheRequest::new(key.clone(), CachePolicy::TtlBound { ttl_ms: 250 });
        if let Ok(Some(spec)) = self
            .cache
            .get::<super::LanguageServerSpec>(&positive, || Ok(()))
        {
            if Path::new(&spec.program).is_file() {
                return Some((*spec).clone());
            }
        }
        let negative = CacheRequest::new(key, CachePolicy::Negative { ttl_ms: 250 });
        if matches!(self.cache.get::<bool>(&negative, || Ok(())), Ok(Some(_))) {
            return None;
        }
        let spec = super::resolve_language_server(language, &root);
        if let Some(spec) = &spec {
            let _ = self.cache.put(&positive, spec.clone(), || Ok(()));
        } else {
            let _ = self.cache.put(&negative, true, || Ok(()));
        }
        spec
    }

    pub fn new() -> Self {
        Self::with_cache(CacheRuntime::default())
    }

    pub fn with_cache(cache: CacheRuntime) -> Self {
        Self {
            sessions: Mutex::new(LocalBackendSessions::new(MAX_LOCAL_SESSIONS)),
            launches: SingleFlight::default(),
            cache,
        }
    }

    /// Launches and initializes one already-resolved local server.
    pub fn start_session(
        &self,
        key: SemanticSessionKey,
        spec: &super::LanguageServerSpec,
        root: &Path,
        request_timeout: Duration,
    ) -> Result<SemanticCapabilities, String> {
        let identity = format!("{key:?}:{spec:?}");
        let (session, leader) = self
            .launches
            .run(&identity, request_timeout, None, || {
                let existing = self
                    .sessions
                    .lock()
                    .map_err(|_| {
                        CacheError::Compute("Semantic session registry unavailable".into())
                    })?
                    .sessions
                    .get(&key)
                    .cloned();
                if let Some(session) = existing.filter(|session| {
                    session.program == spec.program
                        && session.args == spec.args
                        && session.is_alive()
                }) {
                    self.cache.record_resource(false, self.session_count());
                    return Ok(session);
                }
                // Process startup and initialization must not hold the registry lock.
                let session = Arc::new(
                    LocalLspSession::launch(spec, root, request_timeout, self.cache.clone())
                        .map_err(CacheError::Compute)?,
                );
                let mut sessions = self.sessions.lock().map_err(|_| {
                    CacheError::Compute("Semantic session registry unavailable".into())
                })?;
                let retired = sessions.insert(key, session.clone());
                self.cache.record_resource(true, sessions.sessions.len());
                drop(sessions);
                // Shutdown can perform I/O; never run it under the registry lock.
                drop(retired);
                Ok(session)
            })
            .map_err(|error| error.to_string())?;
        if !leader {
            self.cache.record_resource(false, self.session_count());
        }
        Ok(session.capabilities.clone())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().map(|s| s.sessions.len()).unwrap_or(0)
    }

    fn session_for_file(
        &self,
        cwd: &Path,
        file_path: &str,
        language: &str,
    ) -> Result<Arc<LocalLspSession>, String> {
        let target = resolve_target(cwd, file_path)?;
        let normalized_language = super::normalize_language(language);
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Semantic backend session registry is unavailable".to_string())?;
        sessions
            .sessions
            .iter()
            .filter(|(key, _)| {
                key.language == normalized_language && target.starts_with(&key.canonical_root)
            })
            .max_by_key(|(key, _)| key.canonical_root.as_os_str().len())
            .map(|(_, session)| session.clone())
            .ok_or_else(|| "No active local semantic session covers the requested file".into())
    }
}

impl SemanticBackend for LocalSemanticBackend {
    fn definition(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String> {
        let language = language_for_path(file_path);
        let session = self.session_for_file(cwd, file_path, &language)?;
        let result = session.request_for_document(
            "textDocument/definition",
            file_path,
            json!({
                "textDocument": {
                    "uri": path_to_uri(&normalize_verbatim_path(resolve_target(cwd, file_path)?))
                },
                "position": { "line": line, "character": character }
            }),
        )?;
        Ok(SemanticResult {
            request_id: "local-lsp-definition".into(),
            server_identity: Some(session.identity()),
            capability: "definition".into(),
            document_version: None,
            source_manifest: None,
            locations: parse_locations(&result, cwd, &session.root),
            diagnostics: Vec::new(),
            symbols: Vec::new(),
            calls: Vec::new(),
            partial: false,
            fallback_reason: None,
        })
    }

    fn references(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        let language = language_for_path(file_path);
        let session = self.session_for_file(cwd, file_path, &language)?;
        let result = session.request_for_document(
            "textDocument/references",
            file_path,
            json!({
                "textDocument": {
                    "uri": path_to_uri(&normalize_verbatim_path(resolve_target(cwd, file_path)?))
                },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": include_declaration }
            }),
        )?;
        Ok(SemanticResult {
            request_id: "local-lsp-references".into(),
            server_identity: Some(session.identity()),
            capability: "references".into(),
            document_version: None,
            source_manifest: None,
            locations: parse_locations(&result, cwd, &session.root),
            diagnostics: Vec::new(),
            symbols: Vec::new(),
            calls: Vec::new(),
            partial: false,
            fallback_reason: None,
        })
    }

    fn outline(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        let language = language_for_path(file_path);
        let session = self.session_for_file(cwd, file_path, &language)?;
        let result = session.request_for_document(
            "textDocument/documentSymbol",
            file_path,
            json!({
                "textDocument": {
                    "uri": path_to_uri(&normalize_verbatim_path(resolve_target(cwd, file_path)?))
                }
            }),
        )?;
        Ok(SemanticResult {
            request_id: "local-lsp-outline".into(),
            server_identity: Some(session.identity()),
            capability: "outline".into(),
            document_version: None,
            source_manifest: None,
            locations: Vec::new(),
            diagnostics: Vec::new(),
            symbols: parse_symbols(&result),
            calls: Vec::new(),
            partial: false,
            fallback_reason: None,
        })
    }

    fn diagnostics(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        let language = language_for_path(file_path);
        let session = self.session_for_file(cwd, file_path, &language)?;
        let result = session.request_for_document(
            "textDocument/diagnostic",
            file_path,
            json!({
                "textDocument": {
                    "uri": path_to_uri(&normalize_verbatim_path(resolve_target(cwd, file_path)?))
                }
            }),
        )?;
        Ok(SemanticResult {
            request_id: "local-lsp-diagnostics".into(),
            server_identity: Some(session.identity()),
            capability: "diagnostics".into(),
            document_version: None,
            source_manifest: None,
            locations: Vec::new(),
            diagnostics: parse_diagnostics(&result),
            symbols: Vec::new(),
            calls: Vec::new(),
            partial: false,
            fallback_reason: None,
        })
    }

    fn call_hierarchy(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _incoming: bool,
    ) -> Result<SemanticResult, String> {
        Err("Local LSP call hierarchy is not enabled by this adapter".into())
    }

    fn rename_preview(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _new_name: &str,
    ) -> Result<RenamePreview, String> {
        Err("Local LSP rename preview is not enabled by this adapter".into())
    }
}

struct LocalLspSession {
    root: PathBuf,
    program: String,
    args: Vec<String>,
    capabilities: SemanticCapabilities,
    request_timeout: Duration,
    connection: Mutex<LspConnection>,
    documents: Mutex<DocumentTracker>,
    cache: CacheRuntime,
    generation: String,
}

impl fmt::Debug for LocalLspSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalLspSession")
            .field("root", &self.root)
            .field("program", &self.program)
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

impl LocalLspSession {
    fn is_alive(&self) -> bool {
        self.connection
            .lock()
            .ok()
            .is_some_and(|mut connection| matches!(connection.child.try_wait(), Ok(None)))
    }

    fn launch(
        spec: &super::LanguageServerSpec,
        root: &Path,
        request_timeout: Duration,
        cache: CacheRuntime,
    ) -> Result<Self, String> {
        let request_timeout = request_timeout.max(Duration::from_millis(1));
        let config = LspServerConfig {
            executable: PathBuf::from(&spec.program),
            args: spec.args.clone(),
            languages: vec![spec.language.clone()],
            root_dir: root.to_path_buf(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: request_timeout,
            env_allowlist: Vec::new(),
            executable_hash: None,
        };
        let mut command = build_sanitized_command(&config);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start local language server: {error}"))?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Local language server stdin was not piped".into());
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Local language server stdout was not piped".into());
        };
        let stderr = child.stderr.take();
        drain_stderr(stderr);
        let mut connection = LspConnection::new(child, stdin, stdout);
        let initialize = connection.request(
            "initialize",
            build_initialize_params(root, build_client_capabilities()),
            request_timeout,
        )?;
        let (capabilities, _) = parse_server_capabilities(&initialize);
        connection.notify("initialized", json!({}))?;
        Ok(Self {
            root: root
                .canonicalize()
                .map_err(|error| format!("Could not canonicalize semantic root: {error}"))?,
            program: spec.program.clone(),
            args: spec.args.clone(),
            capabilities,
            request_timeout: request_timeout.max(Duration::from_millis(1)),
            connection: Mutex::new(connection),
            documents: Mutex::new(DocumentTracker::new()),
            cache,
            generation: uuid::Uuid::new_v4().to_string(),
        })
    }

    fn identity(&self) -> String {
        self.program.clone()
    }

    fn request_for_document(
        &self,
        method: &str,
        file_path: &str,
        params: Value,
    ) -> Result<Value, String> {
        let target = resolve_target(&self.root, file_path)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Local language-server connection is unavailable".to_string())?;
        self.sync_document(&target, &mut connection)?;
        if !matches!(connection.child.try_wait(), Ok(None)) {
            return Err("Local language server is no longer running".into());
        }
        // Document symbols are short-lived, document-local derived data. Broader
        // workspace queries and diagnostics deliberately bypass this cache.
        if method == "textDocument/documentSymbol" {
            let documents = self
                .documents
                .lock()
                .map_err(|_| "Semantic document tracker unavailable".to_string())?;
            let document = documents
                .get_document(&target)
                .ok_or_else(|| "Semantic document is not synchronized".to_string())?;
            let workspace = self.root.to_string_lossy().into_owned();
            let request = CacheRequest::new(
                CacheKey::new(
                    CacheNamespace::Lsp,
                    serde_json::to_string(&(method, &params)).map_err(|e| e.to_string())?,
                    1,
                    "document-symbols-v1",
                    vec![
                        CacheDependency::ServerGeneration {
                            workspace: workspace.clone(),
                            generation: self.generation.clone(),
                        },
                        CacheDependency::DocumentVersion {
                            workspace,
                            uri: path_to_uri(&target),
                            version: i64::from(document.version),
                        },
                        CacheDependency::ContentHash(document.content_hash.clone()),
                    ],
                ),
                CachePolicy::TtlBound { ttl_ms: 250 },
            );
            drop(documents);
            let result = self
                .cache
                .get_or_compute(
                    &request,
                    || Ok(()),
                    None,
                    || {
                        connection
                            .request(method, params, self.request_timeout)
                            .map_err(CacheError::Compute)
                    },
                )
                .map_err(|e| e.to_string())?;
            return Ok((*result).clone());
        }
        connection.request(method, params, self.request_timeout)
    }

    fn sync_document(&self, path: &Path, connection: &mut LspConnection) -> Result<(), String> {
        if !is_path_in_root(path, &self.root) {
            return Err(format!(
                "Language server document {} is outside workspace root {}",
                path.display(),
                self.root.display()
            ));
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| "Semantic document outside root".to_string())?;
        let snapshot = self
            .cache
            .read_current_file(&self.root, relative, MAX_DOCUMENT_BYTES as usize, || Ok(()))
            .map_err(|error| format!("Could not read semantic document: {error}"))?;
        let content = String::from_utf8(snapshot.bytes)
            .map_err(|error| format!("Semantic document is not UTF-8: {error}"))?;
        let uri = path_to_uri(&normalize_verbatim_path(path.to_path_buf()));
        let update = {
            let mut documents = self
                .documents
                .lock()
                .map_err(|_| "Semantic document tracker is unavailable".to_string())?;
            let existing = documents
                .get_document(path)
                .map(|document| (document.is_open, document.content_hash.clone()));
            match existing {
                None => Some((
                    "textDocument/didOpen",
                    documents.did_open(uri.clone(), path.to_path_buf(), content.clone()),
                )),
                Some((false, _)) => Some((
                    "textDocument/didOpen",
                    documents.did_open(uri.clone(), path.to_path_buf(), content.clone()),
                )),
                Some((true, hash)) if hash != snapshot.content_hash => Some((
                    "textDocument/didChange",
                    documents.did_change(path, content.clone())?,
                )),
                Some((true, _)) => None,
            }
        };
        let Some((method, version)) = update else {
            return Ok(());
        };
        let params = if method == "textDocument/didOpen" {
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id(path),
                    "version": version,
                    "text": content
                }
            })
        } else {
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": content }]
            })
        };
        connection.notify(method, params)
    }
}

struct LspConnection {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Result<Value, String>>,
    requests: RequestTable,
}

impl fmt::Debug for LspConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LspConnection")
            .field("pending_requests", &self.requests.pending_count())
            .finish()
    }
}

impl LspConnection {
    fn new(child: Child, stdin: ChildStdin, stdout: ChildStdout) -> Self {
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || read_frames(stdout, sender));
        Self {
            child,
            stdin,
            incoming,
            requests: RequestTable::new(),
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.requests.allocate_request(method, timeout)?;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        if let Err(error) = self.write_message(&message) {
            self.requests.cancel_request(id);
            return Err(error);
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.cancel(id);
                return Err(format!("Language server request {method} timed out"));
            }
            match self.incoming.recv_timeout(remaining) {
                Ok(Err(error)) => {
                    self.requests.cancel_request(id);
                    return Err(error);
                }
                Ok(Ok(message)) => {
                    if message.get("method").is_some() {
                        self.handle_unsolicited_message(&message)?;
                        continue;
                    }
                    if message.get("id").and_then(Value::as_u64) == Some(id) {
                        match self.requests.handle_response(id) {
                            Ok(Some(_)) => {}
                            Ok(None) => continue,
                            Err(error) => return Err(error),
                        }
                        if let Some(error) = message.get("error") {
                            return Err(format_server_error(error));
                        }
                        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                    }
                    self.handle_unsolicited_message(&message)?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.cancel(id);
                    return Err(format!("Language server request {method} timed out"));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.requests.cancel_request(id);
                    return Err("Language server transport closed unexpectedly".into());
                }
            }
        }
    }

    fn cancel(&mut self, id: u64) {
        self.requests.cancel_request(id);
        let _ = self.notify("$/cancelRequest", json!({ "id": id }));
    }

    fn handle_unsolicited_message(&mut self, message: &Value) -> Result<(), String> {
        match unsolicited_error_response(message) {
            Some(response) => self.write_message(&response),
            None => Ok(()),
        }
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let encoded = serde_json::to_string(message)
            .map_err(|error| format!("Could not encode language-server request: {error}"))?;
        if encoded.len() > super::transport::MAX_FRAME_SIZE {
            return Err("Language-server request exceeds the frame size limit".into());
        }
        self.stdin
            .write_all(&lsp_frame(&encoded))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("Could not write to language server: {error}"))
    }
}

impl Drop for LspConnection {
    fn drop(&mut self) {
        let _ = self.notify("shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_frames(mut stdout: ChildStdout, sender: mpsc::Sender<Result<Value, String>>) {
    let mut parser = LspFrameParser::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => {
                let _ = sender.send(Err("Language server stdout closed".into()));
                return;
            }
            Ok(count) => {
                parser.feed(&chunk[..count]);
                loop {
                    match parser.next_frame() {
                        Ok(Some(frame)) => match serde_json::from_str::<Value>(&frame) {
                            Ok(message) => {
                                if sender.send(Ok(message)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ = sender.send(Err(format!(
                                    "Language server returned invalid JSON: {error}"
                                )));
                                return;
                            }
                        },
                        Ok(None) => break,
                        Err(error) => {
                            let _ =
                                sender.send(Err(format!("Invalid language-server frame: {error}")));
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(Err(format!(
                    "Could not read language-server output: {error}"
                )));
                return;
            }
        }
    }
}

fn drain_stderr(stderr: Option<std::process::ChildStderr>) {
    let Some(stderr) = stderr else {
        return;
    };
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let _ = super::transport::sanitize_stderr_line(&line);
                }
            }
        }
    });
}

fn resolve_target(cwd: &Path, file_path: &str) -> Result<PathBuf, String> {
    let target = Path::new(file_path);
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    };
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("Could not resolve semantic document: {error}"))?;
    let root = cwd
        .canonicalize()
        .map_err(|error| format!("Could not resolve semantic workspace: {error}"))?;
    if !is_path_in_root(&canonical, &root) {
        return Err(format!(
            "Semantic document {} is outside workspace root {}",
            canonical.display(),
            cwd.display()
        ));
    }
    Ok(canonical)
}

fn normalize_verbatim_path(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}

fn language_for_path(path: &str) -> String {
    super::normalize_language(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default(),
    )
}

fn language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        _ => "plaintext",
    }
}

fn parse_locations(value: &Value, cwd: &Path, root: &Path) -> Vec<Location> {
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .filter_map(|value| parse_location(value, cwd, root))
            .take(256)
            .collect();
    }
    value
        .is_object()
        .then(|| parse_location(value, cwd, root))
        .flatten()
        .into_iter()
        .collect()
}

fn unsolicited_error_response(message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str()?;
    let id = message.get("id")?;
    if !id.is_number() && !id.is_string() {
        return None;
    }
    let error = if is_unsolicited_effect(method) {
        "Server-initiated workspace mutation is denied"
    } else {
        "Server-initiated requests are unsupported"
    };
    Some(json!({
        "jsonrpc": "2.0",
        "id": id.clone(),
        "error": { "code": -32601, "message": error }
    }))
}

fn parse_location(value: &Value, cwd: &Path, root: &Path) -> Option<Location> {
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))
        .and_then(Value::as_str)?;
    let range = value
        .get("range")
        .or_else(|| value.get("targetRange"))
        .and_then(parse_range)?;
    // Resolve both sides: temp directories and workspace aliases can differ
    // from their canonical spelling (including Windows short names/casing).
    let path = uri_to_path(uri).ok()?.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    let cwd = cwd.canonicalize().ok()?;
    if !is_path_in_root(&path, &root) {
        return None;
    }
    let display_path = path
        .strip_prefix(&cwd)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    Some(Location {
        path: display_path,
        range,
        snippet: None,
        is_textual_fallback: false,
    })
}

fn parse_symbols(value: &Value) -> Vec<SymbolItem> {
    let mut symbols = Vec::new();
    let values = value.as_array().cloned().unwrap_or_default();
    for value in &values {
        collect_symbol(value, &mut symbols);
        if symbols.len() >= 256 {
            break;
        }
    }
    symbols.truncate(256);
    symbols
}

fn collect_symbol(value: &Value, symbols: &mut Vec<SymbolItem>) {
    let Some(range) = value.get("range").and_then(parse_range) else {
        if let Some(location) = value.get("location") {
            if let Some(range) = location.get("range").and_then(parse_range) {
                symbols.push(SymbolItem {
                    name: value
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    kind: symbol_kind(value.get("kind")),
                    range,
                    selection_range: range,
                    detail: value
                        .get("containerName")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
        }
        return;
    };
    symbols.push(SymbolItem {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        kind: symbol_kind(value.get("kind")),
        range,
        selection_range: value
            .get("selectionRange")
            .and_then(parse_range)
            .unwrap_or(range),
        detail: value
            .get("detail")
            .and_then(Value::as_str)
            .map(str::to_string),
    });
    if let Some(children) = value.get("children").and_then(Value::as_array) {
        for child in children {
            collect_symbol(child, symbols);
            if symbols.len() >= 256 {
                break;
            }
        }
    }
}

fn parse_diagnostics(value: &Value) -> Vec<Diagnostic> {
    let values = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    values
        .iter()
        .filter_map(|value| {
            Some(Diagnostic {
                range: value.get("range").and_then(parse_range)?,
                severity: diagnostic_severity(value.get("severity")),
                code: value.get("code").and_then(value_string),
                source: value
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                message: value.get("message").and_then(Value::as_str)?.to_string(),
            })
        })
        .take(256)
        .collect()
}

fn parse_range(value: &Value) -> Option<Range> {
    Some(Range {
        start: parse_position(value.get("start")?)?,
        end: parse_position(value.get("end")?)?,
    })
}

fn parse_position(value: &Value) -> Option<Position> {
    Some(Position {
        line: value.get("line")?.as_u64()?.try_into().ok()?,
        character: value.get("character")?.as_u64()?.try_into().ok()?,
    })
}

fn symbol_kind(value: Option<&Value>) -> String {
    match value.and_then(Value::as_u64) {
        Some(5) => "class",
        Some(6) => "method",
        Some(8) => "field",
        Some(9) => "constructor",
        Some(10) => "enum",
        Some(11) => "interface",
        Some(12) => "function",
        Some(13) => "variable",
        Some(14) => "constant",
        Some(23) => "struct",
        Some(kind) => return format!("kind_{kind}"),
        None => "unknown",
    }
    .to_string()
}

fn diagnostic_severity(value: Option<&Value>) -> DiagnosticSeverity {
    match value.and_then(Value::as_u64) {
        Some(1) => DiagnosticSeverity::Error,
        Some(2) => DiagnosticSeverity::Warning,
        Some(4) => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Information,
    }
}

fn value_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
}

fn format_server_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Language server returned an error");
    message.chars().take(MAX_SERVER_ERROR_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    #[ignore = "subprocess fixture; invoked by cache_session_reuse_edit_and_restart"]
    fn cache_fixture_lsp() {
        if !Path::new("cache-lsp-fixture-enabled").is_file() {
            return;
        }
        let mut input = std::io::stdin().lock();
        let mut output = std::io::stdout().lock();
        // Separate the test harness preamble from the first framed header.
        output.write_all(b"\r\n").unwrap();
        output.flush().unwrap();
        let mut text = String::new();
        let mut calls = 0;
        for _ in 0..100 {
            let mut length = 0;
            loop {
                let mut line = String::new();
                if input.read_line(&mut line).unwrap() == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            input.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let result = match request["method"].as_str().unwrap_or("") {
                "initialize" => json!({"capabilities":{"documentSymbolProvider":true}}),
                "textDocument/didOpen" => {
                    text = request["params"]["textDocument"]["text"]
                        .as_str()
                        .unwrap()
                        .into();
                    continue;
                }
                "textDocument/didChange" => {
                    text = request["params"]["contentChanges"][0]["text"]
                        .as_str()
                        .unwrap()
                        .into();
                    continue;
                }
                "textDocument/documentSymbol" => {
                    calls += 1;
                    json!({"calls":calls,"text":text})
                }
                "shutdown" => Value::Null,
                "exit" => return,
                _ => continue,
            };
            output
                .write_all(&lsp_frame(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string(),
                ))
                .unwrap();
            output.flush().unwrap();
        }
    }

    #[test]
    fn cache_session_reuse_edit_and_restart() {
        let root = tempdir().unwrap();
        std::fs::write(root.path().join("cache-lsp-fixture-enabled"), "fixture").unwrap();
        std::fs::write(root.path().join("file.rs"), "before").unwrap();
        let spec = super::super::LanguageServerSpec {
            program: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec![
                "--exact".into(),
                "semantic::backend::tests::cache_fixture_lsp".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            language: "rust".into(),
        };
        let key = super::super::LazySemanticSessionRegistry::key(root.path(), "rust", "fixture");
        let cache = CacheRuntime::default();
        let backend = Arc::new(LocalSemanticBackend::with_cache(cache.clone()));
        let barrier = Arc::new(std::sync::Barrier::new(10));
        let workers: Vec<_> = (0..10)
            .map(|_| {
                let (backend, barrier, key, spec, root) = (
                    backend.clone(),
                    barrier.clone(),
                    key.clone(),
                    spec.clone(),
                    root.path().to_path_buf(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    backend
                        .start_session(key, &spec, &root, Duration::from_secs(5))
                        .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(cache.stats().resource_cold_starts, 1);
        assert_eq!(cache.stats().resource_reuses, 9);
        let session = backend.sessions.lock().unwrap().sessions[&key].clone();
        let query = || {
            session
                .request_for_document("textDocument/documentSymbol", "file.rs", json!({}))
                .unwrap()
        };
        assert_eq!(query()["calls"], 1);
        assert_eq!(query()["calls"], 1);
        std::fs::write(root.path().join("file.rs"), "edited").unwrap();
        assert_eq!(query()["text"], "edited");
        assert_eq!(query()["calls"], 2);
        session.connection.lock().unwrap().child.kill().unwrap();
        session.connection.lock().unwrap().child.wait().unwrap();
        assert!(session
            .request_for_document("textDocument/documentSymbol", "file.rs", json!({}))
            .is_err());
        backend
            .start_session(key.clone(), &spec, root.path(), Duration::from_secs(5))
            .unwrap();
        let restarted = backend.sessions.lock().unwrap().sessions[&key].clone();
        assert_ne!(session.generation, restarted.generation);
        assert_eq!(
            restarted
                .request_for_document("textDocument/documentSymbol", "file.rs", json!({}))
                .unwrap()["calls"],
            1
        );
        assert_eq!(cache.stats().resource_cold_starts, 2);
    }

    #[test]
    fn semantic_definition_converts_canned_lsp_response() {
        let root = tempdir().unwrap();
        let response = json!([
            {
                "uri": path_to_uri(&root.path().join("src/lib.rs")),
                "range": {
                    "start": {"line": 4, "character": 2},
                    "end": {"line": 4, "character": 8}
                }
            }
        ]);
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/lib.rs"), "fn target() {}\n").unwrap();
        let locations = parse_locations(&response, root.path(), root.path());
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].path, "src/lib.rs");
        assert!(!locations[0].is_textual_fallback);
        assert_eq!(locations[0].range.start.line, 4);
    }

    #[test]
    fn semantic_definition_accepts_single_location_response() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/lib.rs"), "fn target() {}\n").unwrap();
        let response = json!({
            "uri": path_to_uri(&root.path().join("src/lib.rs")),
            "range": {
                "start": {"line": 0, "character": 3},
                "end": {"line": 0, "character": 9}
            }
        });

        let locations = parse_locations(&response, root.path(), root.path());
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].path, "src/lib.rs");
    }

    #[test]
    fn semantic_definition_accepts_workspace_alias_without_exposing_absolute_path() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file.rs"), "fn target() {}\n").unwrap();
        #[cfg(windows)]
        let alias = dir.path().join("workspace");
        #[cfg(unix)]
        let alias = {
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(&root, &alias).unwrap();
            alias
        };
        let response = json!({
            "uri": path_to_uri(&root.join("file.rs")),
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 0, "character": 2}
            }
        });
        let locations = parse_locations(&response, &alias, &alias);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].path, "file.rs");
        assert_eq!(
            resolve_target(&alias, "file.rs").unwrap(),
            root.join("file.rs").canonicalize().unwrap()
        );
    }

    #[test]
    fn unsolicited_request_response_preserves_string_id() {
        let response = unsolicited_error_response(&json!({
            "jsonrpc": "2.0",
            "id": "server-request-1",
            "method": "workspace/applyEdit"
        }))
        .unwrap();
        assert_eq!(response["id"], "server-request-1");
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn semantic_response_discards_locations_outside_root() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "fn secret() {}\n").unwrap();
        let response = json!([
            {
                "uri": path_to_uri(&outside.path().join("secret.rs")),
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 6}
                }
            }
        ]);
        assert!(parse_locations(&response, root.path(), root.path()).is_empty());
    }

    #[test]
    fn semantic_outline_flattens_document_symbol_children() {
        let response = json!([
            {
                "name": "Parent",
                "kind": 5,
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 3, "character": 0}},
                "selectionRange": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 12}},
                "children": [{
                    "name": "child",
                    "kind": 12,
                    "range": {"start": {"line": 1, "character": 0}, "end": {"line": 2, "character": 0}},
                    "selectionRange": {"start": {"line": 1, "character": 3}, "end": {"line": 1, "character": 8}}
                }]
            }
        ]);
        let symbols = parse_symbols(&response);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[1].kind, "function");
    }

    #[test]
    fn local_backend_does_not_spawn_during_construction() {
        assert_eq!(LocalSemanticBackend::new().session_count(), 0);
    }
}
