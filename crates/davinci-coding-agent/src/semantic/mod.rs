pub mod backend;
pub mod documents;
pub mod manager;
pub mod rename;
pub mod session;
pub mod shared;
pub mod tools;
pub mod transport;

use davinci_agent::semantic::{
    text_fallback_definition, text_fallback_diagnostics, text_fallback_outline,
    text_fallback_references, RenamePreview, SemanticCapabilities, SemanticResult, SemanticService,
};
use davinci_agent::{PermissionState, PermissionVerdict};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use backend::LocalSemanticBackend;
pub use shared::{SemanticClient, SemanticServiceFacade};
pub use session::{
    normalize_language, resolve_language_server, LanguageServerSpec, LazySemanticSessionRegistry,
    SemanticSessionKey, SemanticSessionState,
};

/// Backend used after a separately authorized language-server session is ready.
///
/// Keeping the backend behind this narrow synchronous contract lets the host
/// connect the existing LSP manager or an offline fixture without making
/// session discovery spawn a process or weakening the fallback path.
pub trait SemanticBackend: std::fmt::Debug + Send + Sync {
    fn definition(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
    ) -> Result<SemanticResult, String> {
        Err("Semantic backend does not support definition".into())
    }

    fn references(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        Err("Semantic backend does not support references".into())
    }

    fn outline(&self, _cwd: &Path, _file_path: &str) -> Result<SemanticResult, String> {
        Err("Semantic backend does not support outline".into())
    }

    fn diagnostics(&self, _cwd: &Path, _file_path: &str) -> Result<SemanticResult, String> {
        Err("Semantic backend does not support diagnostics".into())
    }

    fn call_hierarchy(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _incoming: bool,
    ) -> Result<SemanticResult, String> {
        Err("Semantic backend does not support call hierarchy".into())
    }

    fn rename_preview(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _new_name: &str,
    ) -> Result<RenamePreview, String> {
        Err("Semantic backend does not support rename preview".into())
    }
}

type ActiveSemanticBackend = (SemanticSessionKey, Arc<dyn SemanticBackend>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticLaunchAuthority {
    pub project_trusted: bool,
    pub execution_allowed: bool,
    pub effects_contained: bool,
}

/// Native implementation of SemanticService connecting to the lazy session
/// registry and an authorized semantic backend.
#[derive(Debug)]
pub struct NativeSemanticService {
    sessions: Arc<Mutex<LazySemanticSessionRegistry>>,
    backend: Arc<Mutex<Option<Arc<dyn SemanticBackend>>>>,
    local_backend: Arc<LocalSemanticBackend>,
    launch_permissions: Option<Arc<PermissionState>>,
}

impl NativeSemanticService {
    pub fn new() -> Self {
        let local_backend = Arc::new(LocalSemanticBackend::new());
        Self {
            sessions: Arc::new(Mutex::new(LazySemanticSessionRegistry::default())),
            backend: Arc::new(Mutex::new(Some(local_backend.clone()))),
            local_backend,
            launch_permissions: None,
        }
    }

    /// Creates a service with a host-provided, already-local backend. The
    /// backend is still ignored until a session is explicitly marked ready.
    pub fn with_backend(backend: Arc<dyn SemanticBackend>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(LazySemanticSessionRegistry::default())),
            backend: Arc::new(Mutex::new(Some(backend))),
            local_backend: Arc::new(LocalSemanticBackend::new()),
            launch_permissions: None,
        }
    }

    /// Creates the product service. Local language servers are discovered and
    /// started only when a semantic tool is called and the exact command is
    /// already allowed by the current permission policy.
    pub fn with_permissions(permissions: Arc<PermissionState>) -> Self {
        Self::with_permissions_and_cache(permissions, Default::default())
    }

    pub fn with_permissions_and_cache(
        permissions: Arc<PermissionState>,
        cache: davinci_agent::runtime::cache::CacheRuntime,
    ) -> Self {
        let local_backend = LocalSemanticBackend::shared(cache);
        let mut service = Self::new();
        service.backend = Arc::new(Mutex::new(Some(local_backend.clone())));
        service.local_backend = local_backend;
        service.launch_permissions = Some(permissions);
        service
    }

    /// Resolves, launches, and initializes a local server after the caller has
    /// explicitly requested semantic capability and supplied all authority
    /// gates. Construction and ordinary fallback calls never spawn a process.
    pub fn request_semantic_session(
        &self,
        root: &Path,
        language: &str,
        config_digest: &str,
        authority: SemanticLaunchAuthority,
        request_timeout: Duration,
    ) -> Result<SemanticCapabilities, String> {
        let spec = self.local_backend.discover(language, root).ok_or_else(|| {
            format!(
                "No locally installed language server is available for {}",
                normalize_language(language)
            )
        })?;
        self.request_resolved_semantic_session(
            root,
            language,
            config_digest,
            spec,
            authority,
            request_timeout,
        )
    }

    fn request_resolved_semantic_session(
        &self,
        root: &Path,
        language: &str,
        config_digest: &str,
        spec: LanguageServerSpec,
        authority: SemanticLaunchAuthority,
        request_timeout: Duration,
    ) -> Result<SemanticCapabilities, String> {
        let spec = self
            .sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?
            .ensure_resolved_session(
                root,
                language,
                config_digest,
                spec,
                authority.project_trusted,
                authority.execution_allowed,
                authority.effects_contained,
            )?;
        let key = LazySemanticSessionRegistry::key(root, language, config_digest);
        let backend = self.local_backend.clone();
        let capabilities = match backend.start_session(key, &spec, root, request_timeout) {
            Ok(capabilities) => capabilities,
            Err(error) => {
                let _ = self.mark_session_unavailable(root, language, config_digest, &error);
                return Err(error);
            }
        };
        self.set_backend(Some(backend))?;
        self.mark_session_ready(root, language, config_digest, capabilities.clone())?;
        Ok(capabilities)
    }

    fn try_launch_authorized_session(&self, root: &Path, language: &str) -> bool {
        let Some(permissions) = &self.launch_permissions else {
            return false;
        };
        let Some(spec) = self.local_backend.discover(language, root) else {
            return false;
        };
        let command = render_command_for_policy(&spec);
        // Keep the policy guard until the process has started so a concurrent
        // policy update cannot revoke the exact command between decision and
        // execution.
        let policy = match permissions.lock() {
            Ok(policy) => policy,
            Err(_) => return false,
        };
        let authority = SemanticLaunchAuthority {
            project_trusted: policy.project_trusted,
            execution_allowed: matches!(
                policy.decide(
                    "semantic-language-server",
                    "bash",
                    &json!({ "command": command }),
                    root,
                ),
                PermissionVerdict::Allow
            ),
            effects_contained: true,
        };
        if !authority.project_trusted || !authority.execution_allowed {
            return false;
        }
        self.request_resolved_semantic_session(
            root,
            language,
            "default",
            spec,
            authority,
            Duration::from_secs(5),
        )
        .is_ok()
    }

    pub fn set_backend(&self, backend: Option<Arc<dyn SemanticBackend>>) -> Result<(), String> {
        self.backend
            .lock()
            .map_err(|_| "Semantic backend registry is unavailable".to_string())?
            .clone_from(&backend);
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions
            .lock()
            .map(|registry| registry.len())
            .unwrap_or(0)
    }

    pub fn prepare_session(
        &self,
        root: &Path,
        language: &str,
        config_digest: &str,
        project_trusted: bool,
        execution_allowed: bool,
        effects_contained: bool,
    ) -> Result<LanguageServerSpec, String> {
        self.sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?
            .ensure_session(
                root,
                language,
                config_digest,
                project_trusted,
                execution_allowed,
                effects_contained,
            )
    }

    /// Completes the host-controlled launch and initialize handshake.
    pub fn mark_session_ready(
        &self,
        root: &Path,
        language: &str,
        config_digest: &str,
        capabilities: SemanticCapabilities,
    ) -> Result<(), String> {
        let key = LazySemanticSessionRegistry::key(root, language, config_digest);
        self.sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?
            .mark_ready_with_capabilities(&key, capabilities)
    }

    pub fn mark_session_unavailable(
        &self,
        root: &Path,
        language: &str,
        config_digest: &str,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        let key = LazySemanticSessionRegistry::key(root, language, config_digest);
        self.sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?
            .mark_unavailable(&key, reason)
    }

    fn target_path(&self, cwd: &Path, file_path: &str) -> Result<PathBuf, String> {
        let target = Path::new(file_path);
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            cwd.join(target)
        };
        if !tools::is_path_in_root(&target, cwd) {
            return Err(format!(
                "Semantic target {} is outside workspace root {}",
                target.display(),
                cwd.display()
            ));
        }
        Ok(target)
    }

    fn backend(&self) -> Option<Arc<dyn SemanticBackend>> {
        self.backend.lock().ok().and_then(|backend| backend.clone())
    }

    fn active_backend(
        &self,
        cwd: &Path,
        file_path: &str,
        language: &str,
        supported: impl FnOnce(&SemanticCapabilities) -> bool,
    ) -> Result<Option<ActiveSemanticBackend>, String> {
        let target = self.target_path(cwd, file_path)?;
        let Some(backend) = self.backend() else {
            return Ok(None);
        };
        let ready = self
            .sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?
            .ready_key_for_path(&target, language)
            .is_some();
        // Recheck current authority and child health even when the registry is ready.
        if self.launch_permissions.is_some() {
            if !self.try_launch_authorized_session(cwd, language) {
                return Ok(None);
            }
        } else if !ready {
            return Ok(None);
        }
        let registry = self
            .sessions
            .lock()
            .map_err(|_| "Semantic session registry is unavailable".to_string())?;
        let Some(key) = registry.ready_key_for_path(&target, language) else {
            return Ok(None);
        };
        let capabilities = registry.capabilities(&key).cloned().unwrap_or_default();
        if !supported(&capabilities) {
            return Ok(None);
        }
        Ok(Some((key, backend)))
    }

    fn mark_backend_unavailable(&self, key: &SemanticSessionKey, reason: &str) {
        if let Ok(mut registry) = self.sessions.lock() {
            let _ = registry.mark_unavailable(key, bounded_error(reason));
        }
    }

    fn symbol_at_position(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<String, String> {
        let path = self.target_path(cwd, file_path)?;
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Could not read semantic target {file_path}: {e}"))?;
        let source_line = content
            .lines()
            .nth(line as usize)
            .ok_or_else(|| format!("Semantic line {line} is outside {file_path}"))?;
        let cursor = utf16_offset_to_byte(source_line, character as usize);
        let mut selected = None;
        for (start, value) in source_line.char_indices() {
            let end = start + value.len_utf8();
            if !is_symbol_char(value) {
                continue;
            }
            if start <= cursor && cursor < end {
                selected = Some((start, end));
                break;
            }
            if end <= cursor {
                selected = Some((start, end));
            }
        }
        let Some((mut start, mut end)) = selected else {
            return Err(format!(
                "No identifier at line {line}, character {character} in {file_path}"
            ));
        };
        while start > 0 {
            let Some((previous, value)) = source_line[..start].char_indices().next_back() else {
                break;
            };
            if !is_symbol_char(value) {
                break;
            }
            start = previous;
        }
        while end < source_line.len() {
            let Some(value) = source_line[end..].chars().next() else {
                break;
            };
            if !is_symbol_char(value) {
                break;
            }
            end += value.len_utf8();
        }
        Ok(source_line[start..end].to_string())
    }

    fn fallback_error<T>(backend_error: &str, fallback: Result<T, String>) -> Result<T, String> {
        fallback.map_err(|fallback_error| {
            format!(
                "semantic backend failed: {}; deterministic fallback failed: {}",
                bounded_error(backend_error),
                bounded_error(&fallback_error)
            )
        })
    }
}

pub fn render_command_for_policy(spec: &LanguageServerSpec) -> String {
    std::iter::once(spec.program.as_str())
        .chain(spec.args.iter().map(String::as_str))
        .map(quote_command_argument)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_command_argument(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./:\\".contains(character))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

impl Default for NativeSemanticService {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticService for NativeSemanticService {
    fn is_server_available(&self, language: &str, path: &Path) -> bool {
        let Some(backend) = self.backend() else {
            return false;
        };
        let _ = backend;
        self.sessions
            .lock()
            .ok()
            .and_then(|registry| registry.ready_key_for_path(path, language))
            .is_some()
    }

    fn capabilities(&self, language: &str, path: &Path) -> SemanticCapabilities {
        if self.backend().is_none() {
            return SemanticCapabilities::default();
        }
        self.sessions
            .lock()
            .ok()
            .and_then(|registry| registry.ready_key_for_path(path, language))
            .and_then(|key| self.sessions.lock().ok()?.capabilities(&key).cloned())
            .unwrap_or_default()
    }

    fn definition(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.definition)?;
        if let Some((key, backend)) = active {
            match backend.definition(cwd, file_path, line, character) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    self.mark_backend_unavailable(&key, &error);
                    let fallback = self
                        .symbol_at_position(cwd, file_path, line, character)
                        .and_then(|symbol| {
                            text_fallback_definition(cwd, &symbol, Some(Path::new(file_path)), None)
                        });
                    return Self::fallback_error(&error, fallback);
                }
            }
        }
        let symbol = self.symbol_at_position(cwd, file_path, line, character)?;
        text_fallback_definition(cwd, &symbol, Some(Path::new(file_path)), None)
    }

    fn references(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.references)?;
        if let Some((key, backend)) = active {
            match backend.references(cwd, file_path, line, character, include_declaration) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    self.mark_backend_unavailable(&key, &error);
                    let fallback = self
                        .symbol_at_position(cwd, file_path, line, character)
                        .and_then(|symbol| {
                            text_fallback_references(cwd, &symbol, Some(Path::new(file_path)), None)
                        });
                    return Self::fallback_error(&error, fallback);
                }
            }
        }
        let symbol = self.symbol_at_position(cwd, file_path, line, character)?;
        text_fallback_references(cwd, &symbol, Some(Path::new(file_path)), None)
    }

    fn outline(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.outline)?;
        if let Some((key, backend)) = active {
            match backend.outline(cwd, file_path) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    self.mark_backend_unavailable(&key, &error);
                    return Self::fallback_error(&error, text_fallback_outline(cwd, file_path));
                }
            }
        }
        text_fallback_outline(cwd, file_path)
    }

    fn diagnostics(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.diagnostics)?;
        if let Some((key, backend)) = active {
            match backend.diagnostics(cwd, file_path) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    self.mark_backend_unavailable(&key, &error);
                    return Self::fallback_error(&error, text_fallback_diagnostics(cwd, file_path));
                }
            }
        }
        text_fallback_diagnostics(cwd, file_path)
    }

    fn call_hierarchy(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        incoming: bool,
    ) -> Result<SemanticResult, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.call_hierarchy)?;
        let Some((key, backend)) = active else {
            return Err("Call hierarchy is unavailable without an active semantic backend".into());
        };
        backend
            .call_hierarchy(cwd, file_path, line, character, incoming)
            .map_err(|error| {
                self.mark_backend_unavailable(&key, &error);
                bounded_error(&error)
            })
    }

    fn rename_preview(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<RenamePreview, String> {
        let language = Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let active = self.active_backend(cwd, file_path, language, |caps| caps.rename_preview)?;
        let Some((key, backend)) = active else {
            return Err("Rename preview is unavailable without an active semantic backend; plain text search cannot safely guarantee semantic rename".into());
        };
        backend
            .rename_preview(cwd, file_path, line, character, new_name)
            .map_err(|error| {
                self.mark_backend_unavailable(&key, &error);
                bounded_error(&error)
            })
    }
}

fn bounded_error(error: &str) -> String {
    error.chars().take(512).collect()
}

fn is_symbol_char(value: char) -> bool {
    value.is_alphanumeric() || value == '_'
}

fn utf16_offset_to_byte(line: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, value) in line.char_indices() {
        if units >= offset {
            return byte;
        }
        units += value.len_utf16();
        if units > offset {
            return byte;
        }
    }
    line.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[derive(Debug)]
    struct FixtureBackend {
        definition_calls: Arc<AtomicUsize>,
        fail_definition: bool,
    }

    impl SemanticBackend for FixtureBackend {
        fn definition(
            &self,
            _cwd: &Path,
            _file_path: &str,
            _line: u32,
            _character: u32,
        ) -> Result<SemanticResult, String> {
            self.definition_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_definition {
                return Err("fixture timeout".into());
            }
            Ok(SemanticResult {
                request_id: "fixture-definition".into(),
                server_identity: Some("fixture-lsp".into()),
                capability: "definition".into(),
                document_version: None,
                source_manifest: None,
                locations: Vec::new(),
                diagnostics: Vec::new(),
                symbols: Vec::new(),
                calls: Vec::new(),
                partial: false,
                fallback_reason: None,
            })
        }
    }

    fn fixture_service(
        fail_definition: bool,
    ) -> (tempfile::TempDir, NativeSemanticService, Arc<AtomicUsize>) {
        let dir = tempdir().unwrap();
        let server_dir = dir.path().join(".davinci").join("language-servers");
        fs::create_dir_all(&server_dir).unwrap();
        fs::write(server_dir.join("rust-analyzer"), b"fixture").unwrap();
        fs::write(dir.path().join("code.rs"), "fn target() {}\n").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = NativeSemanticService::with_backend(Arc::new(FixtureBackend {
            definition_calls: calls.clone(),
            fail_definition,
        }));
        service
            .prepare_session(dir.path(), "rust", "fixture", true, true, true)
            .unwrap();
        service
            .mark_session_ready(
                dir.path(),
                "rust",
                "fixture",
                SemanticCapabilities {
                    definition: true,
                    references: true,
                    outline: true,
                    diagnostics: true,
                    ..SemanticCapabilities::default()
                },
            )
            .unwrap();
        (dir, service, calls)
    }

    #[test]
    fn test_native_semantic_service_defaults_to_unavailable() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let svc = NativeSemanticService::new();
        assert!(!svc.is_server_available("rust", &dir.path().join("main.rs")));
        assert_eq!(
            svc.capabilities("rust", &dir.path().join("main.rs")),
            SemanticCapabilities::default()
        );
        assert_eq!(
            svc.definition(dir.path(), "main.rs", 0, 3)
                .unwrap()
                .capability,
            "text_fallback_definition"
        );
        assert!(svc.diagnostics(dir.path(), "main.rs").unwrap().partial);
        assert!(svc
            .call_hierarchy(Path::new("."), "src/main.rs", 1, 1, true)
            .is_err());
        assert!(svc
            .rename_preview(Path::new("."), "src/main.rs", 1, 1, "foo")
            .is_err());
    }

    #[test]
    fn semantic_definition_uses_lazy_server() {
        let (dir, service, calls) = fixture_service(false);

        assert!(service.is_server_available("rs", &dir.path().join("code.rs")));
        let result = service.definition(dir.path(), "code.rs", 0, 3).unwrap();

        assert_eq!(result.request_id, "fixture-definition");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn semantic_timeout_falls_back_deterministically() {
        let (dir, service, calls) = fixture_service(true);

        let first = service.definition(dir.path(), "code.rs", 0, 3).unwrap();
        assert_eq!(first.capability, "text_fallback_definition");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!service.is_server_available("rust", &dir.path().join("code.rs")));

        let second = service.definition(dir.path(), "code.rs", 0, 3).unwrap();
        assert_eq!(second.capability, "text_fallback_definition");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn semantic_diagnostic_fallback_is_explicitly_partial() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("code.ts"), "const answer = 42;\n").unwrap();
        let service = NativeSemanticService::new();

        let result = service.diagnostics(dir.path(), "code.ts").unwrap();

        assert!(result.partial);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn semantic_launch_permission_uses_exact_shell_command() {
        let spec = LanguageServerSpec {
            program: r#"C:\Program Files\semantic server.exe"#.into(),
            args: vec!["--stdio".into(), "quoted value".into()],
            language: "rust".into(),
        };

        assert_eq!(
            render_command_for_policy(&spec),
            r#""C:\Program Files\semantic server.exe" --stdio "quoted value""#
        );
    }
}
