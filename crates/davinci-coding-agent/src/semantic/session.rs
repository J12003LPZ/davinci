//! Lazy, permission-aware language-server discovery and session identity.

use super::manager::server_launch_allowed;
use davinci_agent::semantic::SemanticCapabilities;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

const DEFAULT_MAX_SESSIONS: usize = 8;

/// Stable identity for one language-server workspace session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SemanticSessionKey {
    pub canonical_root: PathBuf,
    pub language: String,
    pub config_digest: String,
}

/// Lifecycle state tracked before a process handle is attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticSessionState {
    Starting,
    Ready,
    Unavailable { reason: String },
}

/// A locally discovered language-server command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageServerSpec {
    pub program: String,
    pub args: Vec<String>,
    pub language: String,
}

/// Bounded session registry. It records lifecycle intent but never starts a
/// process during construction or discovery.
#[derive(Debug, Clone)]
pub struct LazySemanticSessionRegistry {
    max_sessions: usize,
    sessions: BTreeMap<SemanticSessionKey, SemanticSessionState>,
    specs: BTreeMap<SemanticSessionKey, LanguageServerSpec>,
    capabilities: BTreeMap<SemanticSessionKey, SemanticCapabilities>,
    insertion_order: Vec<SemanticSessionKey>,
}

impl Default for LazySemanticSessionRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SESSIONS)
    }
}

impl LazySemanticSessionRegistry {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            max_sessions: max_sessions.max(1),
            sessions: BTreeMap::new(),
            specs: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            insertion_order: Vec::new(),
        }
    }

    pub fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn key(root: &Path, language: &str, config_digest: &str) -> SemanticSessionKey {
        SemanticSessionKey {
            canonical_root: canonical_root(root),
            language: normalize_language(language),
            config_digest: normalize_config_digest(config_digest),
        }
    }

    pub fn state(&self, key: &SemanticSessionKey) -> Option<&SemanticSessionState> {
        self.sessions.get(key)
    }

    pub fn spec(&self, key: &SemanticSessionKey) -> Option<&LanguageServerSpec> {
        self.specs.get(key)
    }

    pub fn capabilities(&self, key: &SemanticSessionKey) -> Option<&SemanticCapabilities> {
        self.capabilities.get(key)
    }

    pub fn ready_for(&self, root: &Path, language: &str) -> bool {
        let root = canonical_root(root);
        let language = normalize_language(language);
        self.sessions.iter().any(|(key, state)| {
            key.canonical_root == root
                && key.language == language
                && *state == SemanticSessionState::Ready
        })
    }

    /// Returns the most specific ready session containing a source path.
    pub fn ready_key_for_path(&self, path: &Path, language: &str) -> Option<SemanticSessionKey> {
        let path = canonical_root(path);
        let language = normalize_language(language);
        self.sessions
            .iter()
            .filter(|(key, state)| {
                key.language == language
                    && **state == SemanticSessionState::Ready
                    && path.starts_with(&key.canonical_root)
            })
            .max_by_key(|(key, _)| key.canonical_root.as_os_str().len())
            .map(|(key, _)| key.clone())
    }

    /// Resolve and register a session without spawning its process.
    pub fn ensure_session(
        &mut self,
        root: &Path,
        language: &str,
        config_digest: &str,
        project_trusted: bool,
        execution_allowed: bool,
        effects_contained: bool,
    ) -> Result<LanguageServerSpec, String> {
        if !server_launch_allowed(project_trusted, execution_allowed, effects_contained) {
            let key = Self::key(root, language, config_digest);
            let reason = "Language-server launch denied by trust or permission policy".to_string();
            self.insert(
                key,
                SemanticSessionState::Unavailable {
                    reason: reason.clone(),
                },
                None,
            );
            return Err(reason);
        }

        let Some(spec) = resolve_language_server(language, root) else {
            let key = Self::key(root, language, config_digest);
            let reason = format!(
                "No locally installed language server is available for {}",
                normalize_language(language)
            );
            self.insert(
                key,
                SemanticSessionState::Unavailable {
                    reason: reason.clone(),
                },
                None,
            );
            return Err(reason);
        };

        self.ensure_resolved_session(
            root,
            language,
            config_digest,
            spec,
            project_trusted,
            execution_allowed,
            effects_contained,
        )
    }

    /// Register the exact executable specification that passed the caller's
    /// permission check. This prevents a second environment lookup from
    /// selecting a different program between authorization and launch.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_resolved_session(
        &mut self,
        root: &Path,
        language: &str,
        config_digest: &str,
        spec: LanguageServerSpec,
        project_trusted: bool,
        execution_allowed: bool,
        effects_contained: bool,
    ) -> Result<LanguageServerSpec, String> {
        let key = Self::key(root, language, config_digest);

        if let Some(state) = self.sessions.get(&key) {
            if let SemanticSessionState::Unavailable { reason } = state {
                return Err(reason.clone());
            }
            if let Some(existing) = self.specs.get(&key) {
                return Ok(existing.clone());
            }
        }

        if !server_launch_allowed(project_trusted, execution_allowed, effects_contained) {
            return Err("Language-server launch denied by trust or permission policy".into());
        }

        if normalize_language(&spec.language) != normalize_language(language) {
            return Err("Resolved language-server specification does not match the request".into());
        }

        self.insert(key, SemanticSessionState::Starting, Some(spec.clone()));
        Ok(spec)
    }

    /// Marks a previously prepared session ready after an external launch and
    /// initialize handshake have succeeded.
    pub fn mark_ready(&mut self, key: &SemanticSessionKey) -> Result<(), String> {
        self.mark_ready_with_capabilities(key, SemanticCapabilities::default())
    }

    pub fn mark_ready_with_capabilities(
        &mut self,
        key: &SemanticSessionKey,
        capabilities: SemanticCapabilities,
    ) -> Result<(), String> {
        if !self.sessions.contains_key(key) {
            return Err("Cannot mark an unknown semantic session ready".into());
        }
        self.sessions
            .insert(key.clone(), SemanticSessionState::Ready);
        self.capabilities.insert(key.clone(), capabilities);
        Ok(())
    }

    /// Marks a session unavailable so repeated requests do not retry a broken
    /// launch on every tool call.
    pub fn mark_unavailable(
        &mut self,
        key: &SemanticSessionKey,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        if !self.sessions.contains_key(key) {
            return Err("Cannot mark an unknown semantic session unavailable".into());
        }
        self.sessions.insert(
            key.clone(),
            SemanticSessionState::Unavailable {
                reason: reason.into(),
            },
        );
        self.capabilities.remove(key);
        Ok(())
    }

    pub fn remove(&mut self, key: &SemanticSessionKey) -> bool {
        let removed = self.sessions.remove(key).is_some();
        self.specs.remove(key);
        self.capabilities.remove(key);
        self.insertion_order.retain(|candidate| candidate != key);
        removed
    }

    fn insert(
        &mut self,
        key: SemanticSessionKey,
        state: SemanticSessionState,
        spec: Option<LanguageServerSpec>,
    ) {
        let is_new = !self.sessions.contains_key(&key);
        if is_new {
            while self.sessions.len() >= self.max_sessions {
                let Some(oldest) = self.insertion_order.first().cloned() else {
                    break;
                };
                self.remove(&oldest);
            }
            self.insertion_order.push(key.clone());
        }
        self.sessions.insert(key.clone(), state);
        if !matches!(self.sessions.get(&key), Some(SemanticSessionState::Ready)) {
            self.capabilities.remove(&key);
        }
        if let Some(spec) = spec {
            self.specs.insert(key, spec);
        }
    }
}

/// Resolves only already-installed local language-server executables.
///
/// The resolver checks an explicit environment override, project-adjacent
/// executable locations, and the current process PATH. It never downloads,
/// installs, or executes a server.
pub fn resolve_language_server(language: &str, root: &Path) -> Option<LanguageServerSpec> {
    let language = normalize_language(language);
    let (env_key, program_name, args) = match language.as_str() {
        "rust" => (
            "DAVINCI_RUST_ANALYZER",
            "rust-analyzer",
            Vec::<String>::new(),
        ),
        "typescript" | "javascript" => (
            "DAVINCI_TYPESCRIPT_LANGUAGE_SERVER",
            "typescript-language-server",
            vec!["--stdio".to_string()],
        ),
        _ => return None,
    };

    let program = env::var_os(env_key)
        .and_then(|value| resolve_program(Path::new(&value), root))
        .or_else(|| resolve_project_program(root, program_name))
        .or_else(|| find_on_path(program_name))?;

    Some(LanguageServerSpec {
        program: program.to_string_lossy().into_owned(),
        args,
        language,
    })
}

pub fn normalize_language(language: &str) -> String {
    let normalized = language.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "rs" => "rust".into(),
        "ts" | "tsx" => "typescript".into(),
        "js" | "jsx" => "javascript".into(),
        _ => normalized,
    }
}

fn normalize_config_digest(config_digest: &str) -> String {
    let normalized = config_digest.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "default".into()
    } else {
        normalized
    }
}

fn canonical_root(root: &Path) -> PathBuf {
    if let Ok(canonical) = root.canonicalize() {
        return canonical;
    }
    if root.is_absolute() {
        root.to_path_buf()
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(root))
            .unwrap_or_else(|_| root.to_path_buf())
    }
}

fn resolve_project_program(root: &Path, program_name: &str) -> Option<PathBuf> {
    [
        root.join(".davinci").join("language-servers"),
        root.join("node_modules").join(".bin"),
    ]
    .into_iter()
    .flat_map(|directory| executable_candidates(&directory.join(program_name)))
    .find(|candidate| candidate.is_file())
}

fn resolve_program(value: &Path, root: &Path) -> Option<PathBuf> {
    if value.components().count() > 1 || value.is_absolute() {
        return executable_candidates(value)
            .into_iter()
            .find(|candidate| candidate.is_file());
    }
    resolve_project_program(root, &value.to_string_lossy())
        .or_else(|| find_on_path(value.to_str()?))
}

fn find_on_path(program_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .flat_map(|directory| executable_candidates(&directory.join(program_name)))
        .find(|candidate| candidate.is_file())
}

fn executable_candidates(path: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![path.to_path_buf()];
    if cfg!(windows) && path.extension().is_none() {
        for extension in ["exe", "cmd", "bat"] {
            let mut candidate = path.to_path_buf();
            candidate.set_extension(extension);
            candidates.push(candidate);
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn fixture_server(root: &Path, name: &str) -> PathBuf {
        let directory = root.join(".davinci").join("language-servers");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, b"fixture executable").unwrap();
        path
    }

    #[test]
    fn semantic_service_is_lazy_at_construction() {
        let registry = LazySemanticSessionRegistry::default();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn semantic_session_key_normalizes_root_language_and_config() {
        let dir = tempdir().unwrap();
        let key = LazySemanticSessionRegistry::key(&dir.path().join("."), " RS ", " ABCDEF ");
        assert_eq!(key.canonical_root, dir.path().canonicalize().unwrap());
        assert_eq!(key.language, "rust");
        assert_eq!(key.config_digest, "abcdef");
    }

    #[test]
    fn semantic_server_resolution_is_local_only() {
        let dir = tempdir().unwrap();
        let expected = fixture_server(dir.path(), "rust-analyzer");
        let spec = resolve_language_server("rust", dir.path()).unwrap();
        assert_eq!(PathBuf::from(spec.program), expected);
        assert!(resolve_language_server("brainfuck", dir.path()).is_none());
    }

    #[test]
    fn semantic_launch_respects_permission_gate() {
        let dir = tempdir().unwrap();
        fixture_server(dir.path(), "rust-analyzer");
        let mut registry = LazySemanticSessionRegistry::default();
        let key = LazySemanticSessionRegistry::key(dir.path(), "rust", "v1");
        let result = registry.ensure_session(dir.path(), "rust", "v1", false, true, true);
        assert!(result.is_err());
        assert!(matches!(
            registry.state(&key),
            Some(SemanticSessionState::Unavailable { .. })
        ));
    }

    #[test]
    fn semantic_sessions_evict_in_insertion_order() {
        let dir = tempdir().unwrap();
        let mut registry = LazySemanticSessionRegistry::new(1);
        fixture_server(dir.path(), "rust-analyzer");
        registry
            .ensure_session(dir.path(), "rust", "one", true, true, true)
            .unwrap();
        let first = LazySemanticSessionRegistry::key(dir.path(), "rust", "one");
        registry
            .ensure_session(dir.path(), "rust", "two", true, true, true)
            .unwrap();
        let second = LazySemanticSessionRegistry::key(dir.path(), "rust", "two");
        assert!(registry.state(&first).is_none());
        assert!(registry.state(&second).is_some());
    }
}
