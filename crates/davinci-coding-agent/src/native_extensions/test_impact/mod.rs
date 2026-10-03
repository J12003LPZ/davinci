//! Deterministic test planning over the existing AST and shared cache runtime.
mod analysis;
mod commands;
mod tools;
pub use tools::{tool_spec, TOOL_NAMES};

use super::{
    engineering_snapshot::{EngineeringSnapshot, EngineeringSnapshots},
    repo_intelligence::RepoIntelligence,
};
use analysis::TestMapping;
use davinci_agent::{
    runtime::cache::{
        CacheDependency, CacheError, CacheKey, CacheNamespace, CachePolicy, CacheRequest,
        CacheRuntime,
    },
    PermissionMode, PermissionPolicy, PermissionState, PermissionVerdict, ToolError, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Instant,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TestImpactConfig {
    pub enabled: bool,
}
impl Default for TestImpactConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    symbol_ids: Vec<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    refresh: bool,
}
fn default_limit() -> usize {
    25
}

#[derive(Debug, Clone)]
pub struct TestImpact {
    root: PathBuf,
    repo: RepoIntelligence,
    snapshots: Option<EngineeringSnapshots>,
    cache: CacheRuntime,
    config: TestImpactConfig,
    permissions: Arc<RwLock<Arc<PermissionState>>>,
    cancellation: Arc<RwLock<Option<Arc<AtomicBool>>>>,
    telemetry: Arc<Mutex<Telemetry>>,
}

#[derive(Debug, Default, Serialize)]
struct Telemetry {
    requests: u64,
    failures: u64,
    mapping_hits: u64,
    last_latency_ms: f64,
    last_failure: Option<&'static str>,
}
impl Default for TestImpact {
    fn default() -> Self {
        Self::new(
            Path::new(""),
            RepoIntelligence::default(),
            CacheRuntime::default(),
            TestImpactConfig { enabled: false },
        )
    }
}
impl TestImpact {
    pub fn new(
        root: &Path,
        repo: RepoIntelligence,
        cache: CacheRuntime,
        config: TestImpactConfig,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            repo,
            snapshots: None,
            cache,
            config,
            permissions: Arc::new(RwLock::new(Arc::new(PermissionState::new(
                PermissionPolicy::new(PermissionMode::Ask),
            )))),
            cancellation: Default::default(),
            telemetry: Default::default(),
        }
    }
    pub fn with_snapshots(mut self, snapshots: EngineeringSnapshots) -> Self {
        self.snapshots = Some(snapshots);
        self
    }

    pub(crate) fn workspace_facts(
        &self,
        changed: &[String],
        force: bool,
    ) -> Result<Arc<EngineeringSnapshot>, String> {
        self.workspace_facts_with_usage(changed, force)
            .map(|(snapshot, _)| snapshot)
    }

    fn workspace_facts_with_usage(
        &self,
        changed: &[String],
        force: bool,
    ) -> Result<(Arc<EngineeringSnapshot>, bool), String> {
        let temporary = EngineeringSnapshots::default();
        let snapshots = self.snapshots.as_ref().unwrap_or(&temporary);
        snapshots.get_with_usage(&self.root, &self.repo, changed, force, &|path| {
            self.authorize("read", &json!({"path":path}), &[])
                .map_err(|e| e.to_string())
        })
    }

    pub fn set_permissions(&self, permissions: Arc<PermissionState>) {
        *self.permissions.write().unwrap_or_else(|e| e.into_inner()) = permissions;
    }
    pub fn set_cancellation(&self, signal: Option<Arc<AtomicBool>>) {
        *self.cancellation.write().unwrap_or_else(|e| e.into_inner()) = signal;
    }
    pub fn status(&self) -> Value {
        json!({"enabled":self.config.enabled,"languages":["typescript","javascript"],"cache_namespace":"test",
            "repository":self.repo.status(),"engineering_snapshot":self.snapshots.as_ref().map(EngineeringSnapshots::status),"telemetry":*self.telemetry.lock().unwrap_or_else(|e| e.into_inner())})
    }
    fn authorize(&self, name: &str, args: &Value, paths: &[String]) -> Result<(), CacheError> {
        if self
            .cancellation
            .read()
            .map_err(|_| CacheError::Cancelled)?
            .as_ref()
            .is_some_and(|signal| signal.load(Ordering::Acquire))
        {
            return Err(CacheError::Cancelled);
        }
        let state = self
            .permissions
            .read()
            .map_err(|_| CacheError::Denied)?
            .clone();
        let policy = state.lock().map_err(|_| CacheError::Denied)?;
        if !matches!(
            policy.decide("test-impact", name, args, &self.root),
            PermissionVerdict::Allow
        ) {
            return Err(CacheError::Denied);
        }
        for path in paths {
            if !matches!(
                policy.decide("test-impact", "read", &json!({"path":path}), &self.root),
                PermissionVerdict::Allow
            ) {
                return Err(CacheError::Denied);
            }
        }
        Ok(())
    }
    pub fn execute(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        self.query(name, args)
            .map(|value| ToolResult {
                content: value.to_string(),
                details: Some(value),
                is_error: false,
            })
            .map_err(ToolError::Failed)
    }
    pub fn query(&self, name: &str, args: &Value) -> Result<Value, String> {
        let started = Instant::now();
        let mut result = self.query_inner(name, args);
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        let mut telemetry = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
        telemetry.requests = telemetry.requests.saturating_add(1);
        telemetry.last_latency_ms = latency;
        telemetry.last_failure = None;
        match &mut result {
            Ok(value) => {
                value["telemetry"]["latency_ms"] = json!(latency);
                if value["telemetry"]["mapping_cache_hit"] == true {
                    telemetry.mapping_hits = telemetry.mapping_hits.saturating_add(1);
                }
            }
            Err(reason) => {
                telemetry.failures = telemetry.failures.saturating_add(1);
                // Status carries a category, never arbitrary input or error text.
                telemetry.last_failure = Some(if reason.contains("cancelled") {
                    "cancelled"
                } else if reason.contains("denied") {
                    "denied"
                } else if reason.starts_with("query_invalid") || reason.contains("invalid_path") {
                    "invalid_input"
                } else {
                    "unavailable"
                });
            }
        }
        result
    }
    fn query_inner(&self, name: &str, args: &Value) -> Result<Value, String> {
        if !TOOL_NAMES.contains(&name) {
            return Err("unknown test capability".into());
        }
        if !self.config.enabled {
            return Err("test_impact_unavailable: disabled".into());
        }
        let mut request: Request =
            serde_json::from_value(args.clone()).map_err(|e| format!("query_invalid: {e}"))?;
        if let Some(path) = request.path.take() {
            request.paths.push(path);
        }
        if request.paths.len() + request.symbol_ids.len() > 64
            || !(1..=100).contains(&request.limit)
        {
            return Err("query_invalid: maximum 64 inputs and limit 1..100".into());
        }
        if request.paths.is_empty() && request.symbol_ids.is_empty() {
            return Err("query_invalid: paths or symbolIds required".into());
        }
        if request
            .symbol_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 4096)
        {
            return Err("query_invalid: symbolId length".into());
        }
        let mut changed: BTreeSet<String> = request
            .paths
            .iter()
            .map(|p| self.repo.validate_changed_path(p))
            .collect::<Result<_, _>>()?;
        self.authorize(name, args, &changed.iter().cloned().collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        let (snapshot, snapshot_hit) = self.workspace_facts_with_usage(
            &changed.iter().cloned().collect::<Vec<_>>(),
            request.refresh,
        )?;
        let index = &snapshot.index;
        let metadata = &snapshot.metadata;
        for id in request.symbol_ids {
            let symbol = index
                .files
                .values()
                .flat_map(|f| &f.symbols)
                .find(|s| s.id == id)
                .ok_or("query_invalid: unknown symbolId")?;
            changed.insert(symbol.file.clone());
        }
        let changed: Vec<String> = changed.into_iter().collect();
        let scope: Vec<String> = index
            .files
            .keys()
            .chain(index.metadata.iter())
            .cloned()
            .collect();
        let identity = snapshot.identity.clone();
        let cache_request = CacheRequest::new(
            CacheKey::new(
                CacheNamespace::Test,
                "reverse-test-map",
                1,
                "ts-js-imports-3",
                vec![CacheDependency::ContentHash(identity.clone())],
            ),
            CachePolicy::MemoryOnly,
        );
        let cached = self
            .cache
            .get::<TestMapping>(&cache_request, || self.authorize(name, args, &[]))
            .map_err(|e| e.to_string())?;
        let cache_hit = cached.is_some();
        let mapping = match cached {
            Some(mapping) => mapping,
            None => self
                .cache
                .get_or_compute(
                    &cache_request,
                    || self.authorize(name, args, &[]),
                    None,
                    || Ok(TestMapping::from_index(index)),
                )
                .map_err(|e| e.to_string())?,
        };
        let (mut rows, mut warnings) = mapping.select(&changed, metadata);
        for path in &changed {
            if !index.files.contains_key(path) && !index.metadata.contains(path) {
                warnings.push(format!("changed source absent from current index: {path}; broader verification required"));
            }
        }
        let (first, broader, command_warnings) = commands::plans(
            &rows,
            metadata,
            &changed,
            !warnings.is_empty() || !metadata.warnings.is_empty(),
        );
        warnings.extend(metadata.warnings.iter().cloned());
        warnings.extend(command_warnings);
        let total = rows.len();
        rows.truncate(request.limit);
        let mut row_bytes = 0;
        let retained = rows
            .iter()
            .take_while(|row| {
                row_bytes += serde_json::to_vec(row).map_or(usize::MAX / 2, |bytes| bytes.len());
                row_bytes <= 64 * 1024
            })
            .count();
        if retained < rows.len() {
            rows.truncate(retained);
            warnings
                .push("result byte limit; use narrower changed inputs for further evidence".into());
        }
        let remaining = total - rows.len();
        warnings.sort();
        warnings.dedup();
        warnings.truncate(32);
        self.authorize(name, args, &scope)
            .map_err(|e| e.to_string())?;
        Ok(json!({
            "summary":format!("{total} related tests; targeted checks precede required broader verification"),
            "results":rows,"total":total,"remaining":remaining,
            "truncated":remaining>0,"partial":!warnings.is_empty(),"warnings":warnings,
            "changed":changed,"first_tier":first,"broader_verification":broader,
            "source_identity":identity,"history":"not available",
            "freshness":{"mode":if snapshot_hit { "shared_snapshot" } else { &index.refresh_mode },"full_refresh_required_before_completion":true},
            "telemetry":{"mapping_cache_hit":cache_hit,"snapshot_hit":snapshot_hit,"source_bytes_read":if snapshot_hit { 0 } else { index.bytes_read },"source_files_read":if snapshot_hit { 0 } else { index.files_read },"metadata_bytes_read":if snapshot_hit { 0 } else { metadata.bytes_read },"reparsed":if snapshot_hit { 0 } else { index.reparsed },"refresh_mode":if snapshot_hit { "shared_snapshot" } else { &index.refresh_mode },
                "tests_selected":total,"indexed_tests_not_selected":mapping.tests.len().saturating_sub(total)}
        }))
    }
}
