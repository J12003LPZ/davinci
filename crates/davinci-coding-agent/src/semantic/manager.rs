//! Language server process lifecycle, configuration, and lease management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::transport::RequestTable;
use davinci_agent::semantic::SemanticCapabilities;

/// Server execution policy controlling build-script and proc-macro execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerExecutionPolicy {
    NavigationOnly,
    FullExecution,
}

/// Explicit configuration for a language server process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub languages: Vec<String>,
    pub root_dir: PathBuf,
    pub policy: ServerExecutionPolicy,
    pub idle_timeout: Duration,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    #[serde(default)]
    pub executable_hash: Option<String>,
}

/// Lifecycle state of an active LSP server session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLifecycleState {
    Uninitialized,
    Initializing,
    Ready,
    ShuttingDown,
    Terminated,
}

/// Active server session holding capabilities and transport state.
pub struct LspServerSession {
    pub config: LspServerConfig,
    pub state: ServerLifecycleState,
    pub capabilities: SemanticCapabilities,
    pub position_encoding: String,
    pub request_table: RequestTable,
    pub child: Option<Child>,
    pub last_active: Instant,
    pub stderr_logs: Vec<String>,
}

impl std::fmt::Debug for LspServerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspServerSession")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("capabilities", &self.capabilities)
            .field("position_encoding", &self.position_encoding)
            .finish()
    }
}

/// Builds the client capabilities payload for `initialize`.
pub fn build_client_capabilities() -> Value {
    serde_json::json!({
        "textDocument": {
            "definition": { "dynamicRegistration": false, "linkSupport": true },
            "references": { "dynamicRegistration": false },
            "documentSymbol": {
                "dynamicRegistration": false,
                "hierarchicalDocumentSymbolSupport": true
            },
            "publishDiagnostics": { "relatedInformation": true },
            "callHierarchy": { "dynamicRegistration": false },
            "rename": { "dynamicRegistration": false, "prepareSupport": true }
        },
        "general": {
            "positionEncodings": ["utf-16", "utf-8"]
        }
    })
}

/// Builds the `initialize` request params.
pub fn build_initialize_params(root_dir: &Path, client_capabilities: Value) -> Value {
    serde_json::json!({
        "processId": std::process::id(),
        "rootUri": format!("file:///{}", root_dir.to_string_lossy().replace('\\', "/")),
        "capabilities": client_capabilities,
        "initializationOptions": {},
        "trace": "off"
    })
}

/// Parses server capabilities from the `initialize` response.
pub fn parse_server_capabilities(result: &Value) -> (SemanticCapabilities, String) {
    let caps = result.get("capabilities");
    let mut capabilities = SemanticCapabilities::default();
    let mut position_encoding = "utf-16".to_string();

    if let Some(c) = caps {
        capabilities.definition = c
            .get("definitionProvider")
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => true,
            })
            .unwrap_or(false);

        capabilities.references = c
            .get("referencesProvider")
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => true,
            })
            .unwrap_or(false);

        capabilities.outline = c
            .get("documentSymbolProvider")
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => true,
            })
            .unwrap_or(false);

        capabilities.call_hierarchy = c
            .get("callHierarchyProvider")
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => true,
            })
            .unwrap_or(false);

        capabilities.rename_preview = c
            .get("renameProvider")
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => true,
            })
            .unwrap_or(false);

        capabilities.diagnostics = true; // LSP server handles diagnostics via publishDiagnostics

        if let Some(enc) = c.get("positionEncoding").and_then(Value::as_str) {
            position_encoding = enc.to_string();
        }
    }

    (capabilities, position_encoding)
}

/// Determines if launching a language server is authorized.
pub fn server_launch_allowed(
    project_trusted: bool,
    execution_allowed: bool,
    effects_contained: bool,
) -> bool {
    project_trusted && execution_allowed && effects_contained
}

/// Verifies that the executable exists and, if an expected hash is provided, matches it.
pub fn verify_executable(path: &Path, expected_hash: Option<&str>) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Executable not found: {}", path.display()));
    }
    if let Some(expected) = expected_hash {
        let content = std::fs::read(path)
            .map_err(|e| format!("Failed to read executable {}: {}", path.display(), e))?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "Executable hash mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            ));
        }
    }
    Ok(())
}

/// Builds an isolated Command for the server, sanitizing environment to prevent secret leakage.
pub fn build_sanitized_command(config: &LspServerConfig) -> std::process::Command {
    let mut cmd = std::process::Command::new(&config.executable);
    cmd.args(&config.args);
    cmd.current_dir(&config.root_dir);

    // Defense against inherited secrets: clear ambient environment
    cmd.env_clear();

    // Standard baseline safe environment variables
    const SAFE_ENV_VARS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "RUSTUP_HOME",
        "CARGO_HOME",
    ];
    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // Explicitly allowlisted environment variables (filtering out sensitive keys)
    for var in &config.env_allowlist {
        let upper = var.to_ascii_uppercase();
        if upper.contains("KEY")
            || upper.contains("SECRET")
            || upper.contains("TOKEN")
            || upper.contains("PASS")
            || upper.contains("CRED")
            || upper.contains("AUTH")
        {
            continue; // Deny sensitive keys from being passed to LSP server
        }
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // For Rust servers, if NavigationOnly, enforce flags to prevent build scripts/proc macros from running arbitrary code
    if config.policy == ServerExecutionPolicy::NavigationOnly {
        cmd.env("RA_PROC_MACRO_ENABLE", "0");
        cmd.env("RA_BUILD_SCRIPTS_ENABLE", "0");
    }

    cmd
}

/// Tracks launch crashes and enforces bounded exponential backoff.
#[derive(Debug, Clone)]
pub struct LaunchCrashTracker {
    pub crash_count: u32,
    pub last_crash: Option<Instant>,
    pub max_consecutive_crashes: u32,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for LaunchCrashTracker {
    fn default() -> Self {
        Self {
            crash_count: 0,
            last_crash: None,
            max_consecutive_crashes: 5,
            base_backoff_ms: 500,
            max_backoff_ms: 30_000,
        }
    }
}

impl LaunchCrashTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_crash(&mut self) {
        self.crash_count = self.crash_count.saturating_add(1);
        self.last_crash = Some(Instant::now());
    }

    pub fn reset(&mut self) {
        self.crash_count = 0;
        self.last_crash = None;
    }

    pub fn current_backoff(&self) -> Duration {
        if self.crash_count == 0 {
            return Duration::ZERO;
        }
        let exp = 2u64.saturating_pow(self.crash_count.saturating_sub(1));
        let ms = (self.base_backoff_ms.saturating_mul(exp)).min(self.max_backoff_ms);
        Duration::from_millis(ms)
    }

    pub fn can_launch(&self) -> Result<(), String> {
        if self.crash_count >= self.max_consecutive_crashes {
            return Err(format!(
                "Server crash backoff limit reached ({} consecutive crashes); launch denied",
                self.crash_count
            ));
        }
        if let Some(last) = self.last_crash {
            let backoff = self.current_backoff();
            let elapsed = last.elapsed();
            if elapsed < backoff {
                return Err(format!(
                    "Server crash backoff in effect: {:?} remaining",
                    backoff - elapsed
                ));
            }
        }
        Ok(())
    }
}

/// High-level manager coordinating language server instances across workspace roots.
#[derive(Debug, Default)]
pub struct LspServerManager {
    sessions: Arc<Mutex<HashMap<PathBuf, LspServerSession>>>,
    crash_trackers: Arc<Mutex<HashMap<PathBuf, LaunchCrashTracker>>>,
}

impl LspServerManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            crash_trackers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Registers or updates server configuration for a workspace root.
    pub fn register_server(&self, config: LspServerConfig) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        let root = config.root_dir.clone();
        let session = LspServerSession {
            config,
            state: ServerLifecycleState::Uninitialized,
            capabilities: SemanticCapabilities::default(),
            position_encoding: "utf-16".into(),
            request_table: RequestTable::new(),
            child: None,
            last_active: Instant::now(),
            stderr_logs: Vec::new(),
        };
        sessions.insert(root, session);
        Ok(())
    }

    /// Finds the registered server whose root_dir contains the given path.
    pub fn find_root_for_path(&self, path: &Path) -> Option<PathBuf> {
        let sessions = self.sessions.lock().unwrap();
        let mut matching = None;
        let mut max_len = 0;
        for root in sessions.keys() {
            if path.starts_with(root) {
                let len = root.as_os_str().len();
                if len > max_len {
                    max_len = len;
                    matching = Some(root.clone());
                }
            }
        }
        matching
    }

    /// Verifies if a given path belongs to the specified root.
    pub fn is_path_isolated_to_root(&self, root_dir: &Path, query_path: &Path) -> bool {
        query_path.starts_with(root_dir)
    }

    /// Records a process crash for the workspace root to track backoff.
    pub fn record_crash(&self, root_dir: &Path) {
        let mut trackers = self.crash_trackers.lock().unwrap();
        trackers
            .entry(root_dir.to_path_buf())
            .or_default()
            .record_crash();
    }

    /// Checks if a server can be launched under crash backoff limits.
    pub fn can_launch(&self, root_dir: &Path) -> Result<(), String> {
        let trackers = self.crash_trackers.lock().unwrap();
        if let Some(tracker) = trackers.get(root_dir) {
            tracker.can_launch()
        } else {
            Ok(())
        }
    }

    /// Checks whether a query is allowed to proceed on the given root, validating state and deadline.
    pub fn check_query_allowed(
        &self,
        root_dir: &Path,
        deadline: Option<Instant>,
    ) -> Result<(), String> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(root_dir)
            .ok_or_else(|| format!("No server registered for root {}", root_dir.display()))?;

        if session.state == ServerLifecycleState::ShuttingDown
            || session.state == ServerLifecycleState::Terminated
        {
            return Err("Server is shutting down or terminated; query aborted".to_string());
        }

        if session.state != ServerLifecycleState::Ready {
            return Err(format!(
                "Server is not ready (current state: {:?})",
                session.state
            ));
        }

        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                return Err("Query budget deadline exceeded".to_string());
            }
        }

        Ok(())
    }

    /// Checks if an active server is ready for the specified workspace.
    pub fn is_ready(&self, root_dir: &Path) -> bool {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(root_dir)
            .is_some_and(|s| s.state == ServerLifecycleState::Ready)
    }

    /// Retrieve capabilities for a workspace root if ready.
    pub fn capabilities(&self, root_dir: &Path) -> Option<SemanticCapabilities> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(root_dir).map(|s| s.capabilities.clone())
    }

    /// Simulates or executes the initialize handshake on a registered server session.
    pub fn complete_initialize(
        &self,
        root_dir: &Path,
        server_result: &Value,
    ) -> Result<SemanticCapabilities, String> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(root_dir)
            .ok_or_else(|| format!("No server registered for root {}", root_dir.display()))?;

        let (caps, encoding) = parse_server_capabilities(server_result);
        session.capabilities = caps.clone();
        session.position_encoding = encoding;
        session.state = ServerLifecycleState::Ready;
        session.last_active = Instant::now();
        Ok(caps)
    }

    /// Perform a clean shutdown and exit sequence for a registered server session.
    pub fn shutdown(&self, root_dir: &Path) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(root_dir)
            .ok_or_else(|| format!("No server registered for root {}", root_dir.display()))?;

        session.state = ServerLifecycleState::ShuttingDown;
        if let Some(mut child) = session.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        session.state = ServerLifecycleState::Terminated;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_payload_builder() {
        let root = Path::new("/workspace/project");
        let caps = build_client_capabilities();
        let params = build_initialize_params(root, caps);

        assert!(params.get("capabilities").is_some());
        assert!(params["rootUri"].as_str().unwrap().contains("project"));
    }

    #[test]
    fn test_server_capabilities_parsing() {
        let sample_init = serde_json::json!({
            "capabilities": {
                "definitionProvider": true,
                "referencesProvider": true,
                "documentSymbolProvider": true,
                "callHierarchyProvider": true,
                "renameProvider": { "prepareSupport": true },
                "positionEncoding": "utf-8"
            }
        });

        let (caps, encoding) = parse_server_capabilities(&sample_init);
        assert!(caps.definition);
        assert!(caps.references);
        assert!(caps.outline);
        assert!(caps.call_hierarchy);
        assert!(caps.rename_preview);
        assert!(caps.diagnostics);
        assert_eq!(encoding, "utf-8");
    }

    #[test]
    fn test_server_lifecycle_initialize_and_shutdown() {
        let manager = LspServerManager::new();
        let root = PathBuf::from("/test/proj");
        let config = LspServerConfig {
            executable: PathBuf::from("dummy-lsp"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: root.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };

        manager.register_server(config).unwrap();
        assert!(!manager.is_ready(&root));

        let server_reply = serde_json::json!({
            "capabilities": {
                "definitionProvider": true,
                "referencesProvider": false
            }
        });

        let caps = manager.complete_initialize(&root, &server_reply).unwrap();
        assert!(caps.definition);
        assert!(!caps.references);
        assert!(manager.is_ready(&root));

        manager.shutdown(&root).unwrap();
        assert!(!manager.is_ready(&root));
    }

    #[test]
    fn f10_server_launch_authority() {
        assert!(!server_launch_allowed(false, true, true));
        assert!(!server_launch_allowed(true, false, true));
        assert!(!server_launch_allowed(true, true, false));
        assert!(server_launch_allowed(true, true, true));
    }

    #[test]
    fn test_project_file_supplies_malicious_executable() {
        assert!(!server_launch_allowed(false, true, true));

        let untrusted_script = Path::new("./scripts/malicious.bat");
        assert!(verify_executable(untrusted_script, None).is_err());
    }

    #[test]
    fn test_path_changes_after_config_approval() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_path = tmp.path().join("server.exe");
        std::fs::write(&bin_path, b"original-binary-v1").unwrap();

        let mut hasher = Sha256::new();
        hasher.update(b"original-binary-v1");
        let hash_v1 = format!("{:x}", hasher.finalize());

        assert!(verify_executable(&bin_path, Some(&hash_v1)).is_ok());

        std::fs::write(&bin_path, b"malicious-tampered-binary").unwrap();
        assert!(verify_executable(&bin_path, Some(&hash_v1)).is_err());
    }

    #[test]
    fn test_server_spawns_build_script() {
        let config = LspServerConfig {
            executable: PathBuf::from("rust-analyzer"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: PathBuf::from("/workspace/rust-proj"),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };
        let cmd = build_sanitized_command(&config);
        let envs: HashMap<_, _> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|x| x.to_os_string())))
            .collect();
        assert_eq!(
            envs.get(std::ffi::OsStr::new("RA_BUILD_SCRIPTS_ENABLE"))
                .and_then(|v| v.as_ref()),
            Some(&std::ffi::OsString::from("0"))
        );
        assert_eq!(
            envs.get(std::ffi::OsStr::new("RA_PROC_MACRO_ENABLE"))
                .and_then(|v| v.as_ref()),
            Some(&std::ffi::OsString::from("0"))
        );

        assert!(!server_launch_allowed(true, true, false));
    }

    #[test]
    fn test_inherited_secrets() {
        std::env::set_var("DAVINCI_SECRET_API_KEY", "super-secret-token");
        let config = LspServerConfig {
            executable: PathBuf::from("lsp"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: PathBuf::from("/workspace"),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec!["DAVINCI_SECRET_API_KEY".into(), "SAFE_VAR".into()],
            executable_hash: None,
        };
        let cmd = build_sanitized_command(&config);
        let envs: HashMap<_, _> = cmd
            .get_envs()
            .map(|(k, _)| k.to_string_lossy().to_uppercase())
            .map(|k| (k, ()))
            .collect();
        assert!(!envs.contains_key("DAVINCI_SECRET_API_KEY"));
    }

    #[test]
    fn test_two_roots() {
        let manager = LspServerManager::new();
        let root_a = PathBuf::from("/workspace/repo-a");
        let root_b = PathBuf::from("/workspace/repo-b");

        let config_a = LspServerConfig {
            executable: PathBuf::from("lsp-a"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: root_a.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };
        let config_b = LspServerConfig {
            executable: PathBuf::from("lsp-b"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: root_b.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };

        manager.register_server(config_a).unwrap();
        manager.register_server(config_b).unwrap();

        let file_a = Path::new("/workspace/repo-a/src/main.rs");
        let file_b = Path::new("/workspace/repo-b/src/lib.rs");

        assert_eq!(manager.find_root_for_path(file_a), Some(root_a.clone()));
        assert_eq!(manager.find_root_for_path(file_b), Some(root_b.clone()));

        assert!(manager.is_path_isolated_to_root(&root_a, file_a));
        assert!(!manager.is_path_isolated_to_root(&root_a, file_b));
        assert!(manager.is_path_isolated_to_root(&root_b, file_b));
        assert!(!manager.is_path_isolated_to_root(&root_b, file_a));
    }

    #[test]
    fn test_missing_binary() {
        let non_existent = Path::new("/non/existent/path/to/lsp-server.exe");
        let res = verify_executable(non_existent, None);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Executable not found"));
    }

    #[test]
    fn test_stop_during_query() {
        let manager = LspServerManager::new();
        let root = PathBuf::from("/workspace/stopping-proj");
        let config = LspServerConfig {
            executable: PathBuf::from("dummy-lsp"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: root.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };
        manager.register_server(config).unwrap();
        let reply = serde_json::json!({ "capabilities": {} });
        manager.complete_initialize(&root, &reply).unwrap();

        assert!(manager.check_query_allowed(&root, None).is_ok());

        manager.shutdown(&root).unwrap();

        let res = manager.check_query_allowed(&root, None);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("shutting down or terminated"));
    }

    #[test]
    fn test_budget_deadline() {
        let manager = LspServerManager::new();
        let root = PathBuf::from("/workspace/deadline-proj");
        let config = LspServerConfig {
            executable: PathBuf::from("dummy-lsp"),
            args: vec![],
            languages: vec!["rust".into()],
            root_dir: root.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: Duration::from_secs(300),
            env_allowlist: vec![],
            executable_hash: None,
        };
        manager.register_server(config).unwrap();
        let reply = serde_json::json!({ "capabilities": {} });
        manager.complete_initialize(&root, &reply).unwrap();

        let expired = Instant::now().checked_sub(Duration::from_millis(100));
        let res = manager.check_query_allowed(&root, expired);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("budget deadline exceeded"));

        let future = Instant::now().checked_add(Duration::from_secs(10));
        assert!(manager.check_query_allowed(&root, future).is_ok());
    }

    #[test]
    fn test_repeated_launch_crash_backoff_bounded() {
        let mut tracker = LaunchCrashTracker::new();
        tracker.base_backoff_ms = 100;
        tracker.max_backoff_ms = 1000;
        tracker.max_consecutive_crashes = 3;

        assert!(tracker.can_launch().is_ok());

        tracker.record_crash();
        assert_eq!(tracker.crash_count, 1);
        assert_eq!(tracker.current_backoff(), Duration::from_millis(100));
        assert!(tracker.can_launch().is_err());

        tracker.record_crash();
        assert_eq!(tracker.crash_count, 2);
        assert_eq!(tracker.current_backoff(), Duration::from_millis(200));

        tracker.record_crash();
        assert_eq!(tracker.crash_count, 3);
        assert!(tracker.can_launch().is_err());
        assert!(tracker
            .can_launch()
            .unwrap_err()
            .contains("backoff limit reached"));

        tracker.reset();
        assert_eq!(tracker.crash_count, 0);
        assert!(tracker.can_launch().is_ok());
    }
}
