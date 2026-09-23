//! Host-owned, lazy, bounded sessions. Clones share ownership across callers.
use super::protocol::{IntelligenceError, Result};
use super::servers::{Backend, TypeScriptAdapter};
use super::session::Session;
use super::{documents, normalize, servers, session, tools};
use davinci_agent::{PermissionState, ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LanguageIntelligenceConfig {
    pub enabled: bool,
    pub typescript: TypeScriptConfig,
    #[serde(skip)]
    pub configuration_error: Option<String>,
}
impl Default for LanguageIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            typescript: TypeScriptConfig::default(),
            configuration_error: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptConfig {
    pub enabled: bool,
    pub backend: Backend,
    pub request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
}
impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Backend::Auto,
            request_timeout_ms: 5000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
        }
    }
}

#[derive(Debug, Default)]
struct Slot {
    session: Option<Session>,
    starts: usize,
    last_error: Option<IntelligenceError>,
}
#[derive(Debug)]
struct Manager {
    workspace: PathBuf,
    config: LanguageIntelligenceConfig,
    slots: Arc<Mutex<BTreeMap<PathBuf, Arc<Mutex<Slot>>>>>,
    closed: Arc<AtomicBool>,
    permissions: Mutex<Option<Arc<PermissionState>>>,
    governor: Mutex<Option<crate::native_extensions::TokenGovernor>>,
}
#[derive(Debug, Clone)]
pub struct LanguageIntelligence {
    inner: Arc<Manager>,
}
impl Default for LanguageIntelligence {
    fn default() -> Self {
        Self::new(Path::new("."), LanguageIntelligenceConfig::default())
    }
}

impl LanguageIntelligence {
    pub fn new(workspace: &Path, config: LanguageIntelligenceConfig) -> Self {
        Self {
            inner: Arc::new(Manager {
                workspace: workspace.into(),
                config,
                slots: Arc::new(Mutex::new(BTreeMap::new())),
                closed: Arc::new(AtomicBool::new(false)),
                permissions: Mutex::new(None),
                governor: Mutex::new(None),
            }),
        }
    }
    pub fn set_permissions(&self, permissions: Option<Arc<PermissionState>>) {
        *self
            .inner
            .permissions
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = permissions;
    }
    /// The controller, never model arguments, chooses a worker's checkout.
    /// Different worktrees share supervision but cannot analyze each other's files.
    pub fn for_workspace(&self, workspace: &Path) -> Self {
        Self {
            inner: Arc::new(Manager {
                workspace: workspace.into(),
                config: self.inner.config.clone(),
                slots: self.inner.slots.clone(),
                closed: self.inner.closed.clone(),
                permissions: Mutex::new(
                    self.inner
                        .permissions
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone(),
                ),
                governor: Mutex::new(
                    self.inner
                        .governor
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone(),
                ),
            }),
        }
    }
    pub fn set_governor(&self, governor: crate::native_extensions::TokenGovernor) {
        *self
            .inner
            .governor
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(governor);
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(
            self.inner
                .config
                .typescript
                .request_timeout_ms
                .clamp(100, 30000),
        )
    }
    pub fn shutdown(&self) {
        let slots = {
            let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            self.inner.closed.store(true, Ordering::Release);
            std::mem::take(&mut *slots)
        };
        // In-flight requests already have finite deadlines. Drain their sessions
        // too; dropping only the map would leave their Arc-owned children alive.
        for slot in slots.into_values() {
            slot.lock()
                .unwrap_or_else(|e| e.into_inner())
                .session
                .take();
        }
    }
    pub fn status(&self) -> Value {
        let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        let sessions: Vec<_> = slots
            .iter()
            .map(|(root, slot)| {
                let Ok(slot) = slot.try_lock() else {
                    return json!({"workspace":root,"session":"busy"});
                };
                let mut status = slot
                    .session
                    .as_ref()
                    .map(Session::status)
                    .unwrap_or_else(|| json!({"workspace":root,"session":"unavailable"}));
                status["starts"] = json!(slot.starts);
                status["lastError"] = json!(slot.last_error);
                status
            })
            .collect();
        drop(slots);
        let discovery = if sessions.is_empty()
            && !self.inner.closed.load(Ordering::Acquire)
            && self.inner.config.enabled
            && self.inner.config.typescript.enabled
        {
            self.inner.workspace.canonicalize().ok().map(|root| {
                match servers::discover(&root, &root, self.inner.config.typescript.backend, &std::env::var_os("PATH").unwrap_or_default()) {
                    Ok(commands) => json!({"workspace":root,"available":true,"candidates":commands.iter().map(|command| json!({
                        "backend":command.kind,"serverVersion":command.version,"projectTypeScript":command.typescript_version
                    })).collect::<Vec<_>>()}),
                    Err(error) => json!({"workspace":root,"available":false,"error":error}),
                }
            })
        } else {
            None
        };
        json!({"enabled":self.inner.config.enabled && self.inner.config.typescript.enabled,
            "closed":self.inner.closed.load(Ordering::Acquire),
            "configurationError":self.inner.config.configuration_error,
            "discovery":discovery,
            "languages":["TypeScript","JavaScript"],"backend":self.inner.config.typescript.backend,
            "sessions":sessions,"startup":"lazy; status never launches a server","verification":"Diagnostics are advisory; run repository compiler, lint and tests."})
    }

    fn request(&self, name: &str, args: &Value) -> Result<Value> {
        self.ensure_open()?;
        let config = &self.inner.config;
        if let Some(message) = &config.configuration_error {
            return Err(IntelligenceError::new("invalid_settings", message));
        }
        if !config.enabled || !config.typescript.enabled {
            return Err(IntelligenceError::new(
                "disabled",
                "Language intelligence is disabled in settings",
            ));
        }
        if !(100..=30000).contains(&config.typescript.request_timeout_ms)
            || [
                config.typescript.max_references,
                config.typescript.max_workspace_symbols,
                config.typescript.max_diagnostics,
            ]
            .iter()
            .any(|n| !(1..=200).contains(n))
        {
            return Err(IntelligenceError::new(
                "invalid_settings",
                "requestTimeoutMs must be 100..30000; result caps must be 1..200",
            ));
        }
        let parsed = tools::Arguments::parse(name, args)?;
        let deadline = Instant::now() + self.timeout();
        let workspace = self.inner.workspace.canonicalize().map_err(|_| {
            IntelligenceError::new("invalid_source_path", "Workspace is unavailable")
        })?;
        let source = parsed
            .path
            .as_deref()
            .map(|path| documents::source_path(&workspace, path))
            .transpose()?;
        let project = source
            .as_deref()
            .map(|path| servers::project_root(&workspace, path, &TypeScriptAdapter))
            .transpose()?
            .unwrap_or_else(|| workspace.clone());
        let permissions = lock_until(&self.inner.permissions, deadline)?
            .clone()
            .ok_or_else(launch_denied)?;
        {
            let policy = permissions.lock().map_err(|_| launch_denied())?;
            if !policy.project_trusted {
                return Err(launch_denied());
            }
            if !matches!(
                policy.decide("language-intelligence", name, args, &workspace),
                davinci_agent::PermissionVerdict::Allow
            ) {
                return Err(IntelligenceError::new(
                    "permission_denied",
                    "Semantic request is not allowed by current permissions",
                ));
            }
        }
        let slot = {
            let mut slots = lock_until(&self.inner.slots, deadline)?;
            self.ensure_open()?;
            if slots.len() >= 8 && !slots.contains_key(&project) {
                return Err(IntelligenceError::new("session_limit","Eight language-server projects are already open; restart the session to release them"));
            }
            slots.entry(project.clone()).or_default().clone()
        };
        let mut slot = lock_until(&slot, deadline)?;
        self.ensure_open()?;
        if slot.session.as_ref().is_some_and(|s| !s.is_alive()) {
            slot.session = None;
        }
        if slot.session.is_none() {
            if slot.starts >= 2 {
                return Err(slot.last_error.clone().unwrap_or_else(|| {
                    IntelligenceError::new(
                        "server_exited",
                        "Language-server restart budget exhausted; restart the DaVinci session",
                    )
                }));
            }
            let commands = servers::discover(
                &workspace,
                &project,
                config.typescript.backend,
                &std::env::var_os("PATH").unwrap_or_default(),
            )?;
            for command in commands.into_iter().take(2 - slot.starts) {
                // Reuse the existing semantic process policy and sanitized environment.
                // Read-only tool classification does not grant permission to execute project code.
                let rendered = crate::semantic::render_command_for_policy(
                    &crate::semantic::LanguageServerSpec {
                        program: command.program.to_string_lossy().into_owned(),
                        args: command.args.clone(),
                        language: "typescript".into(),
                    },
                );
                let policy = permissions.lock().map_err(|_| launch_denied())?;
                if !policy.project_trusted
                    || !matches!(
                        policy.decide(
                            "language-server-launch",
                            "bash",
                            &json!({"command":rendered}),
                            &project
                        ),
                        davinci_agent::PermissionVerdict::Allow
                    )
                {
                    return Err(IntelligenceError::new("server_launch_denied", &format!("Project trust and an existing launch permission are required for: {rendered}")));
                }
                slot.starts += 1;
                match Session::start(command, deadline) {
                    Ok(session) => {
                        slot.session = Some(session);
                        break;
                    }
                    Err(error) => {
                        slot.last_error = Some(error);
                    }
                }
            }
        }
        let (method, capability) = tools::operation(name).expect("validated tool");
        let Some(session) = slot.session.as_mut() else {
            return Err(slot.last_error.clone().unwrap_or_else(launch_denied));
        };
        let response = session.execute(
            method,
            capability,
            source.as_deref(),
            parsed.params(name),
            &TypeScriptAdapter,
            deadline,
        );
        let mut raw = match response {
            Ok(raw) => raw,
            Err(error) => {
                slot.last_error = Some(error.clone());
                if !slot.session.as_ref().is_some_and(Session::is_alive) {
                    slot.session = None;
                }
                return Err(error);
            }
        };
        let mut freshness = None;
        let mut omitted = 0;
        if name == "lsp_diagnostics" && raw.is_object() {
            freshness = raw.get("freshness").cloned();
            omitted = raw["omitted"].as_u64().unwrap_or(0);
            raw = raw["items"].take();
        }
        if name == "lsp_diagnostics" {
            if let Some(severity) = parsed.severity.as_deref().filter(|v| *v != "all") {
                let number = match severity {
                    "error" => 1,
                    "warning" => 2,
                    "information" => 3,
                    _ => 4,
                };
                if let Some(items) = raw.as_array_mut() {
                    items.retain(|item| item["severity"] == number);
                }
            }
        }
        let cap = match name {
            "lsp_workspace_symbols" => config.typescript.max_workspace_symbols,
            "lsp_diagnostics" => config.typescript.max_diagnostics,
            _ => config.typescript.max_references,
        };
        let uri = source.as_deref().map(documents::file_uri).transpose()?;
        let mut normalized = normalize::normalize_retained(
            name,
            raw,
            &workspace,
            uri.as_deref(),
            parsed.limit.unwrap_or(cap).min(cap),
            |full| {
                self.inner
                    .governor
                    .lock()
                    .ok()?
                    .as_mut()?
                    .retain_native_output(name, args, full)
                    .ok()
            },
        )?;
        normalized["available"] = json!(true);
        normalized["backend"] = json!(session.command.kind);
        if name == "lsp_diagnostics" {
            normalized["freshness"] = freshness.unwrap_or(json!("pull-response"));
            normalized["omittedByTransport"] = json!(omitted);
            normalized["advisory"] = json!(true);
        }
        Ok(normalized)
    }
    fn ensure_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            Err(IntelligenceError::new(
                "session_closed",
                "Language intelligence has shut down",
            ))
        } else {
            Ok(())
        }
    }

    pub fn execute(&self, name: &str, args: &Value) -> std::result::Result<ToolResult, ToolError> {
        let (value, is_error) = match self.request(name, args) {
            Ok(value) => (value, false),
            Err(error) => (
                json!({"available":false,"error":error,"fallback":"Use read/search and the repository compiler, lint and tests."}),
                true,
            ),
        };
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&value).expect("JSON value"),
            is_error,
            details: Some(value),
        })
    }
}

fn launch_denied() -> IntelligenceError {
    IntelligenceError::new("server_launch_denied","Language servers require project trust and an existing permission grant for the discovered server command")
}

impl davinci_agent::runtime::task_transport::CoordinatorToolHandler for LanguageIntelligence {
    fn handles(&self, tool: &str) -> bool {
        tools::TOOL_NAMES.contains(&tool) || tool == "retrieve_output"
    }
    fn execute(&self, tool: &str, args: &Value) -> std::result::Result<ToolResult, ToolError> {
        if tool == "retrieve_output" {
            return self
                .inner
                .governor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .ok_or_else(|| ToolError::Failed("Parent output store unavailable".into()))?
                .retrieve(args);
        }
        LanguageIntelligence::execute(self, tool, args)
    }
}

fn lock_until<T>(mutex: &Mutex<T>, deadline: Instant) -> Result<MutexGuard<'_, T>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(IntelligenceError::new(
                    "session_unavailable",
                    "Language-intelligence state unavailable",
                ))
            }
            Err(TryLockError::WouldBlock) => {
                session::remaining(deadline)?;
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_worktrees_resolve_against_their_own_checkout() {
        let (_parent_dir, manager) = fixture();
        let (worker_dir, _) = fixture();
        let worker = manager.for_workspace(worker_dir.path());
        std::fs::write(worker_dir.path().join("only-in-worker.ts"), "worker").unwrap();
        let args = json!({"path":"only-in-worker.ts","line":1,"column":1});
        assert!(manager.execute("lsp_hover", &args).unwrap().is_error);
        assert!(!worker.execute("lsp_hover", &args).unwrap().is_error);
        let source = worker_dir.path().canonicalize().unwrap();
        assert_eq!(manager.status()["sessions"][0]["workspace"], json!(source));
        assert_eq!(manager.status()["sessions"][0]["starts"], 1);
    }

    #[test]
    fn capped_results_use_existing_governor_retrieval() {
        use crate::native_extensions::{OutputStore, TokenGovernor, TokenGovernorConfig};
        use davinci_agent::runtime::task_transport::CoordinatorToolHandler;
        let (dir, manager) = fixture();
        let mut governor = TokenGovernor::with_store(
            "semantic-fixture",
            TokenGovernorConfig::default(),
            OutputStore::new(dir.path().join("outputs")),
        );
        manager.set_governor(governor.clone());
        let result = manager
            .execute(
                "lsp_references",
                &json!({"path":"a.ts","line":1,"column":1,"limit":1}),
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        let details = result.details.unwrap();
        assert_eq!(
            details["remaining"], 2,
            "unexpected semantic result: {details:#}"
        );
        assert_eq!(details["items"].as_array().unwrap().len(), 1);
        let id = details["fullResult"]["id"]
            .as_str()
            .expect("existing output retrieval reference");
        let args = json!({"id":id});
        let local = governor.retrieve(&args).unwrap();
        let remote = CoordinatorToolHandler::execute(&manager, "retrieve_output", &args).unwrap();
        assert_eq!(local.content, remote.content);
        assert!(remote.content.contains("\"total\": 3"));
    }

    #[test]
    fn monorepo_projects_are_independent_and_dead_servers_restart_once() {
        let (dir, manager) = fixture();
        for project in ["app", "api"] {
            std::fs::create_dir(dir.path().join(project)).unwrap();
            std::fs::write(dir.path().join(project).join("tsconfig.json"), "{}").unwrap();
            std::fs::write(dir.path().join(project).join("a.ts"), "x").unwrap();
            let response = manager
                .execute(
                    "lsp_hover",
                    &json!({"path":format!("{project}/a.ts"),"line":1,"column":1}),
                )
                .unwrap();
            assert!(!response.is_error, "{}", response.content);
        }
        assert_eq!(manager.status()["sessions"].as_array().unwrap().len(), 2);
        let root = dir.path().join("app").canonicalize().unwrap();
        for expected in [2, 2] {
            let slot = manager.inner.slots.lock().unwrap()[&root].clone();
            let pid = slot.lock().unwrap().session.as_ref().unwrap().status()["pid"]
                .as_u64()
                .unwrap();
            davinci_agent::jobs::kill_tree(pid as u32);
            let deadline = Instant::now() + Duration::from_secs(2);
            while slot.lock().unwrap().session.as_ref().unwrap().is_alive() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            let response = manager
                .execute("lsp_hover", &json!({"path":"app/a.ts","line":1,"column":1}))
                .unwrap();
            assert_eq!(slot.lock().unwrap().starts, expected);
            if response.is_error {
                assert_eq!(response.details.unwrap()["error"]["code"], "server_exited");
                break;
            }
        }
        assert_eq!(
            manager.inner.slots.lock().unwrap()[&root]
                .lock()
                .unwrap()
                .starts,
            2
        );
    }

    #[test]
    fn graph_worker_processes_share_parent_language_session() {
        use davinci_agent::runtime::task_transport::TaskCoordinatorTransport;
        use davinci_agent::runtime::{
            AgentId, AgentKind, AgentRecord, AgentState, RunId, RuntimeBus, RuntimeHandle,
        };
        use std::sync::atomic::AtomicBool;
        let (dir, manager) = fixture();
        manager.set_governor(crate::native_extensions::TokenGovernor::with_store(
            "graph-semantic",
            Default::default(),
            crate::native_extensions::OutputStore::new(dir.path().join("outputs")),
        ));
        let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        let permissions = manager.inner.permissions.lock().unwrap().clone().unwrap();
        let mut transports = Vec::new();
        let mut workers = Vec::new();
        for _ in 0..4 {
            let id = AgentId::new();
            runtime
                .registry
                .register_agent(AgentRecord {
                    id,
                    run_id: runtime.run_id,
                    parent: Some(runtime.agent_id),
                    kind: AgentKind::GraphWorker,
                    name: "semantic fixture".into(),
                    provider: "fixture".into(),
                    model_id: "fixture".into(),
                    cwd: dir.path().into(),
                    state: AgentState::Running,
                    task_id: None,
                    worktree: None,
                    started_ms: 0,
                    updated_ms: 0,
                    failure_reason: None,
                })
                .unwrap();
            let transport = TaskCoordinatorTransport::bind_with_handler(
                &runtime,
                id,
                permissions.clone(),
                vec![
                    "lsp_hover".into(),
                    "lsp_references".into(),
                    "retrieve_output".into(),
                ],
                dir.path().into(),
                Arc::new(AtomicBool::new(false)),
                Some(Arc::new(manager.clone())),
            )
            .unwrap();
            let client = transport.client();
            let mut command = std::process::Command::new("node");
            command
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/graph-language-worker.cjs"
                ))
                .env(
                    "DAVINCI_TASK_COORDINATOR_ADDR",
                    client.address().to_string(),
                )
                .env("DAVINCI_TASK_COORDINATOR_CREDENTIAL", client.credential())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            workers.push(command.spawn().unwrap());
            transports.push(transport);
        }
        let mut rss = 0;
        for worker in workers {
            let output = worker.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "worker failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["success"], true);
            rss += report["rss"].as_u64().unwrap();
        }
        assert_eq!(manager.status()["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(manager.status()["sessions"][0]["starts"], 1);
        println!("graph semantic fixture: worker processes=4, shared LSP processes=1, summed worker RSS={rss} bytes");
        drop(transports);
        manager.shutdown();
    }
    fn fixture() -> (tempfile::TempDir, LanguageIntelligence) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let package = root.join("node_modules/typescript-language-server");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"typescript-language-server","version":"fixture","bin":"server.cjs"}"#,
        )
        .unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/language-server.cjs"
            ),
            package.join("server.cjs"),
        )
        .unwrap();
        std::fs::write(root.join("a.ts"), "hello").unwrap();
        let manager = LanguageIntelligence::new(&root, LanguageIntelligenceConfig::default());
        let mut policy =
            davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
        policy.project_trusted = true;
        manager.set_permissions(Some(Arc::new(PermissionState::new(policy))));
        (dir, manager)
    }
    #[test]
    fn lazy_concurrent_calls_share_one_session_and_shutdown_cleans_it() {
        let (_dir, manager) = fixture();
        assert_eq!(manager.status()["sessions"], json!([]));
        let workers: Vec<_> = (0..6)
            .map(|_| {
                let manager = manager.clone();
                std::thread::spawn(move || {
                    let result = manager
                        .execute("lsp_hover", &json!({"path":"a.ts","line":1,"column":1}))
                        .unwrap();
                    assert!(!result.is_error, "{}", result.content);
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let status = manager.status();
        assert_eq!(status["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(status["sessions"][0]["starts"], 1);
        assert_eq!(status["sessions"][0]["documents"], 1);
        manager.shutdown();
        assert_eq!(manager.status()["sessions"], json!([]));
        let stopped = manager.for_workspace(&_dir.path().canonicalize().unwrap());
        assert_eq!(
            stopped
                .request("lsp_hover", &json!({"path":"a.ts","line":1,"column":1}))
                .unwrap_err()
                .code,
            "session_closed"
        );
    }
    #[test]
    fn disabled_invalid_and_untrusted_requests_never_launch() {
        let (dir, manager) = fixture();
        manager.set_permissions(None);
        assert_eq!(
            manager
                .request("lsp_hover", &json!({"path":"a.ts","line":1,"column":1}))
                .unwrap_err()
                .code,
            "server_launch_denied"
        );
        let config = LanguageIntelligenceConfig {
            enabled: false,
            ..Default::default()
        };
        let manager = LanguageIntelligence::new(dir.path(), config);
        assert_eq!(
            manager
                .request("lsp_workspace_symbols", &json!({"query":"x"}))
                .unwrap_err()
                .code,
            "disabled"
        );
        assert_eq!(manager.status()["sessions"], json!([]));
    }
}
