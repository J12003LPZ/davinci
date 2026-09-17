use super::index::RepoIndex;
use super::symbols::digest;
use super::{parse_source, scanner, storage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct RepoIntelligenceConfig {
    pub enabled: bool,
    pub max_file_bytes: usize,
    pub max_results: usize,
    pub persist_index: bool,
    pub observe_changes: bool,
}
impl Default for RepoIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_bytes: 1_000_000,
            max_results: 25,
            persist_index: true,
            observe_changes: true,
        }
    }
}

#[derive(Debug, Default)]
struct SharedIndex {
    snapshot: RwLock<Option<Arc<RepoIndex>>>,
    refresh: Mutex<()>,
    observation: Mutex<super::observation::Observation>,
}

#[derive(Debug, Clone)]
pub struct RepoIntelligence {
    root: PathBuf,
    agent_dir: PathBuf,
    pub(super) config: RepoIntelligenceConfig,
    shared: Arc<SharedIndex>,
    pub(super) semantic: Option<Arc<dyn super::SemanticLanguageProvider>>,
}

impl Default for RepoIntelligence {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            agent_dir: PathBuf::new(),
            config: RepoIntelligenceConfig {
                enabled: false,
                ..Default::default()
            },
            shared: Arc::new(SharedIndex::default()),
            semantic: None,
        }
    }
}

type Registry = Mutex<BTreeMap<String, Weak<SharedIndex>>>;
static REGISTRY: OnceLock<Registry> = OnceLock::new();

impl RepoIntelligence {
    pub fn new(root: &Path, agent_dir: &Path, mut config: RepoIntelligenceConfig) -> Self {
        config.max_file_bytes = config.max_file_bytes.clamp(1, 1_000_000);
        config.max_results = config.max_results.clamp(1, 100);
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let agent_dir = agent_dir
            .canonicalize()
            .unwrap_or_else(|_| agent_dir.to_path_buf());
        let key = format!("{}:{}:{:?}", root.display(), agent_dir.display(), config);
        let mut registry = REGISTRY
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, v| v.strong_count() > 0);
        let shared = registry
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let value = Arc::new(SharedIndex::default());
                registry.insert(key, Arc::downgrade(&value));
                value
            });
        Self {
            root,
            agent_dir,
            config,
            shared,
            semantic: None,
        }
    }

    #[allow(dead_code)] // Public library adapter boundary; the CLI has no LSP host yet.
    pub fn with_semantic_provider(
        mut self,
        provider: Arc<dyn super::SemanticLanguageProvider>,
    ) -> Self {
        self.semantic = Some(provider);
        self
    }

    pub fn validate_path(&self, raw: &str) -> Result<PathBuf, String> {
        let relative = scanner::validate_relative(raw)?;
        let candidate = self
            .root
            .join(&relative)
            .canonicalize()
            .map_err(|_| "invalid_path")?;
        if !candidate.starts_with(&self.root) {
            return Err("outside_workspace".into());
        }
        let mut current = self.root.clone();
        for part in relative.components() {
            current.push(part);
            let meta = std::fs::symlink_metadata(&current).map_err(|_| "invalid_path")?;
            if scanner::linked(&meta) {
                return Err("outside_workspace: linked path".into());
            }
        }
        Ok(relative)
    }

    /// A deleted source is a valid impact input, but linked/outside ancestors are not.
    pub fn validate_changed_path(&self, raw: &str) -> Result<String, String> {
        if raw.len() > 4096 {
            return Err("invalid_path: path length".into());
        }
        let relative = scanner::validate_relative(raw)?;
        let mut current = self.root.clone();
        let mut parts = Vec::new();
        for part in relative.components() {
            if let std::path::Component::Normal(part) = part {
                current.push(part);
                parts.push(part.to_string_lossy().to_string());
                match std::fs::symlink_metadata(&current) {
                    Ok(meta) if scanner::linked(&meta) => {
                        return Err("outside_workspace: linked path".into())
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err("invalid_path: metadata unavailable".into()),
                }
            }
        }
        if parts.is_empty() {
            return Err("invalid_path: source or config required".into());
        }
        Ok(parts.join("/"))
    }

    /// Bounded, confined configuration read for deterministic index consumers.
    pub fn read_project_file(&self, path: &str, limit: usize) -> Result<String, String> {
        let relative = self.validate_path(path)?;
        scanner::read_bounded(&self.root, &relative, limit.min(1_000_000))
    }

    #[allow(dead_code)] // Public diagnostics/evaluation API, also compiled in the CLI.
    pub fn cache_path(&self) -> Result<PathBuf, String> {
        Ok(storage::directory(&self.agent_dir, &self.root)?.join("index.json"))
    }

    pub fn status(&self) -> Value {
        let snapshot = self
            .shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let index = snapshot.as_ref();
        json!({
            "root": self.root, "enabled": self.config.enabled, "initialized": index.is_some(),
            "languages": ["typescript", "javascript"], "schema_version": super::index::SCHEMA_VERSION,
            "indexed_files": index.map_or(0, |i| i.files.len()),
            "symbols": index.map_or(0, |i| i.files.values().map(|f| f.symbols.len()).sum::<usize>()),
            "edges": index.map_or(0, |i| i.files.values().map(|f| f.edges.len()).sum::<usize>()),
            "last_incremental_refresh_ms": index.map(|i| i.updated_ms),
            "cache_state": if self.config.persist_index { "persistent" } else { "memory_only" },
            "parse_failures": index.map_or(0, |i| i.files.values().filter(|f| f.parse_status != "ok").count()),
            "observation":self.shared.observation.lock().unwrap_or_else(|e|e.into_inner()).status(),
        })
    }

    pub fn refresh(&self) -> Result<Arc<RepoIndex>, String> {
        self.refresh_authorized(&|_| Ok(()))
    }

    /// Consumers may impose current read policy before any source/config read.
    pub fn refresh_authorized(
        &self,
        authorize: &impl Fn(&str) -> Result<(), String>,
    ) -> Result<Arc<RepoIndex>, String> {
        self.refresh_observed_authorized(&[], true, authorize)
    }

    /// Warm planning uses observed changes plus fresh inventory and input reads.
    /// Set `force` for full content reconciliation at a verification boundary.
    pub fn refresh_observed_authorized(
        &self,
        changed: &[String],
        force: bool,
        authorize: &impl Fn(&str) -> Result<(), String>,
    ) -> Result<Arc<RepoIndex>, String> {
        if !self.config.enabled {
            return Err("index_unavailable: disabled".into());
        }
        let _refresh = self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = self
            .root
            .canonicalize()
            .map_err(|_| "index_unavailable: root missing")?;
        if root != self.root {
            return Err("outside_workspace: root identity changed".into());
        }
        let mut observation = self
            .shared
            .observation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        observation.start(&root, self.config.observe_changes);
        let identity = digest(
            serde_json::to_string(&self.config)
                .unwrap_or_default()
                .as_bytes(),
        );
        let directory = if self.config.persist_index {
            storage::directory(&self.agent_dir, &root).ok()
        } else {
            None
        };
        // A busy lease must not cause duplicate indexing. Storage-unavailable mode
        // remains useful in-process, and reports that cross-process caching is unavailable.
        let _lease = directory.as_ref().map(|d| storage::lease(d)).transpose()?;
        let existing = self
            .shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let base = existing
            .as_deref()
            .cloned()
            .or_else(|| {
                directory
                    .as_ref()
                    .and_then(|d| storage::load(d, &root.to_string_lossy(), &identity))
            })
            .unwrap_or_else(|| RepoIndex::empty(root.to_string_lossy().into(), identity));
        let mut bytes_read = 0;
        let mut files_read = 0;
        let mut reparsed = 0;
        for attempt in 0..3 {
            let mut dirty = observation.drain(&root).paths;
            dirty.extend(changed.iter().cloned());
            let scan =
                match scanner::scan(&root, authorize, |path| observation.watch_directory(path)) {
                    Ok(scan) => scan,
                    Err(error) => {
                        observation.needs_full = true;
                        return Err(error);
                    }
                };
            dirty.extend(observation.drain(&root).paths);
            let full = force || attempt > 0 || observation.full_required();
            // Errors/cancellation cannot leave consumed events trusted on retry.
            observation.needs_full = true;
            let mut index = base.clone();
            index.warnings = scan.warnings;
            if self.config.persist_index && directory.is_none() {
                index
                    .warnings
                    .push("persistent cache unavailable; using memory".into());
            }
            let mut files = BTreeMap::new();
            let mut records = 0usize;
            let mut source_bytes = 0u64;
            for path in &scan.sources {
                authorize(path)?;
                let cached = base.files.get(path);
                let reusable = !full
                    && !dirty.contains(path)
                    && observation.stamps.get(path) == scan.stamps.get(path);
                let (file, file_bytes) = if let Some(file) = cached.filter(|_| reusable) {
                    (
                        file.clone(),
                        scan.stamps.get(path).map_or(0, |stamp| stamp.len),
                    )
                } else {
                    let source = match scanner::read_bounded(
                        &root,
                        Path::new(path),
                        self.config.max_file_bytes,
                    ) {
                        Ok(source) => source,
                        Err(reason) => {
                            if index.warnings.len() < 32 {
                                index.warnings.push(format!("{path}: {reason}"));
                            }
                            continue;
                        }
                    };
                    bytes_read += source.len();
                    files_read += 1;
                    let hash = digest(source.as_bytes());
                    let file = match cached.filter(|f| f.content_hash == hash) {
                        Some(file) => file.clone(),
                        None => {
                            reparsed += 1;
                            Arc::new(parse_source(path, &source)?)
                        }
                    };
                    (file, source.len() as u64)
                };
                source_bytes += file_bytes;
                records += file.symbols.len() + file.edges.len();
                if source_bytes > 64 * 1024 * 1024 || records > 250_000 {
                    index
                        .warnings
                        .push("repository source/structural record limit".into());
                    break;
                }
                files.insert(path.clone(), file);
            }
            index.files = files;
            index.metadata = scan.metadata;
            for path in &index.metadata {
                authorize(path)?;
            }
            index.aliases = super::modules::collect(&root, &index.metadata);
            index.text_files = scan.text_files;
            let pending = observation.drain(&root);
            if pending.rescan
                || pending.paths.iter().any(|p| {
                    super::LanguageAdapter::for_path(p).is_some()
                        || index.metadata.contains(p)
                        || !root.join(p).is_file()
                })
            {
                continue;
            }
            index.bytes_read = bytes_read;
            index.files_read = files_read;
            index.reparsed = reparsed;
            index.refresh_mode = if full { "full" } else { "observed" }.into();
            index.updated_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if let Some(directory) = &directory {
                if full || reparsed > 0 || index.files.len() != base.files.len() {
                    if let Err(reason) = storage::save(directory, &index) {
                        index.warnings.push(format!("cache save failed: {reason}"));
                    }
                }
            }
            index.warnings.truncate(32);
            observation.publish(&root, &scan.directories, scan.stamps, full);
            let index = Arc::new(index);
            *self
                .shared
                .snapshot
                .write()
                .unwrap_or_else(|e| e.into_inner()) = Some(index.clone());
            return Ok(index);
        }
        Err("index_unavailable: workspace kept changing during bounded reconciliation".into())
    }
}
