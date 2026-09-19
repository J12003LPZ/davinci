pub mod model;
pub mod resolver;
pub mod runners;
pub mod tools;

pub use model::*;
pub use resolver::BuildResolver;
pub use tools::{tool_spec, TOOL_NAMES};

use davinci_agent::{
    runtime::cache::{
        digest, CacheDependency, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
    },
    ToolError, ToolResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
struct BuildTelemetry {
    requests: u64,
    hits: u64,
    misses: u64,
    failures: u64,
}

#[derive(Debug, Clone)]
pub struct BuildIntelligence {
    root: PathBuf,
    cache: CacheRuntime,
    config: BuildIntelligenceConfig,
    telemetry: Arc<Mutex<BuildTelemetry>>,
}

impl Default for BuildIntelligence {
    fn default() -> Self {
        Self::with_root(Path::new("."), CacheRuntime::default())
    }
}

impl BuildIntelligence {
    pub fn new(root: &Path, cache: CacheRuntime, config: BuildIntelligenceConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            cache,
            config,
            telemetry: Arc::new(Mutex::new(BuildTelemetry::default())),
        }
    }

    pub fn with_root(root: &Path, cache: CacheRuntime) -> Self {
        Self::new(root, cache, BuildIntelligenceConfig::default())
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        if !self.config.enabled {
            return Ok(ToolResult {
                content: "build intelligence is disabled in settings".to_string(),
                details: Some(json!({ "error": "build intelligence is disabled in settings" })),
                is_error: true,
            });
        }

        self.record_request();

        let resolver = BuildResolver::new(&self.root);

        match name {
            "workspace_packages" => self.execute_workspace_packages(&resolver, args),
            "build_targets" => self.execute_build_targets(&resolver, args),
            "build_dependencies" => self.execute_build_dependencies(&resolver, args),
            "build_affected" => self.execute_build_affected(&resolver, args),
            "build_command" => self.execute_build_command(&resolver, args),
            _ => {
                self.record_failure();
                Err(ToolError::Unknown(format!(
                    "unknown build intelligence tool '{name}'"
                )))
            }
        }
    }

    fn execute_workspace_packages(
        &self,
        resolver: &BuildResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::WorkspacePackagesArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("workspace_packages: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("workspace_packages", args);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                self.record_hit();
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        match resolver.discover_packages(parsed.scope.as_deref()) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
                if let Some(key) = cache_key {
                    let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
                    let _ = self.cache.put(&req, val.clone(), || Ok(()));
                    self.record_miss();
                }
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                })
            }
            Err(err) => {
                self.record_failure();
                Ok(ToolResult {
                    content: format!("Error: {err}"),
                    details: Some(json!({ "error": err.to_string() })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_build_targets(
        &self,
        resolver: &BuildResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::BuildTargetsArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("build_targets: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("build_targets", args);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                self.record_hit();
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        match resolver.discover_targets(parsed.package.as_deref(), parsed.scope.as_deref()) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
                if let Some(key) = cache_key {
                    let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
                    let _ = self.cache.put(&req, val.clone(), || Ok(()));
                    self.record_miss();
                }
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                })
            }
            Err(err) => {
                self.record_failure();
                Ok(ToolResult {
                    content: format!("Error: {err}"),
                    details: Some(json!({ "error": err.to_string() })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_build_dependencies(
        &self,
        resolver: &BuildResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::BuildDependenciesArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("build_dependencies: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("build_dependencies", args);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                self.record_hit();
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        match resolver.build_dependencies(&parsed.package, parsed.target.as_deref()) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
                if let Some(key) = cache_key {
                    let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
                    let _ = self.cache.put(&req, val.clone(), || Ok(()));
                    self.record_miss();
                }
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                })
            }
            Err(err) => {
                self.record_failure();
                Ok(ToolResult {
                    content: format!("Error: {err}"),
                    details: Some(json!({ "error": err.to_string() })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_build_affected(
        &self,
        resolver: &BuildResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::BuildAffectedArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("build_affected: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("build_affected", args);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                self.record_hit();
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        match resolver.build_affected(
            parsed.files.as_deref(),
            parsed.packages.as_deref(),
            parsed.target.as_deref(),
        ) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
                if let Some(key) = cache_key {
                    let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
                    let _ = self.cache.put(&req, val.clone(), || Ok(()));
                    self.record_miss();
                }
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                })
            }
            Err(err) => {
                self.record_failure();
                Ok(ToolResult {
                    content: format!("Error: {err}"),
                    details: Some(json!({ "error": err.to_string() })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_build_command(
        &self,
        resolver: &BuildResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::BuildCommandArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("build_command: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("build_command", args);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                self.record_hit();
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        match resolver.build_command(
            parsed.packages.as_deref(),
            parsed.target.as_deref(),
            parsed.files.as_deref(),
        ) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
                if let Some(key) = cache_key {
                    let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
                    let _ = self.cache.put(&req, val.clone(), || Ok(()));
                    self.record_miss();
                }
                Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                })
            }
            Err(err) => {
                self.record_failure();
                Ok(ToolResult {
                    content: format!("Error: {err}"),
                    details: Some(json!({ "error": err.to_string() })),
                    is_error: true,
                })
            }
        }
    }

    pub fn status(&self) -> Value {
        let resolver = BuildResolver::new(&self.root);
        let pkgs = resolver.discover_packages(None).ok();
        let task_runner = pkgs.as_ref().and_then(|p| p.task_runner.clone());
        let pm = pkgs
            .as_ref()
            .map(|p| p.package_manager.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let pkg_count = pkgs.as_ref().map(|p| p.packages.len()).unwrap_or(0);

        let t = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());

        json!({
            "enabled": self.config.enabled,
            "workspace_root": self.root.to_string_lossy(),
            "package_manager": pm,
            "task_runner": task_runner,
            "packages_count": pkg_count,
            "telemetry": {
                "requests": t.requests,
                "hits": t.hits,
                "misses": t.misses,
                "failures": t.failures,
            }
        })
    }

    fn compute_cache_key(&self, tool: &str, args: &Value) -> Option<CacheKey> {
        let mut deps = Vec::new();
        let manifest = self.root.join("package.json");
        if let Ok(bytes) = fs::read(&manifest) {
            deps.push(CacheDependency::PackageManifestHash(digest(&bytes)));
        }
        for cfg in [
            "pnpm-lock.yaml",
            "package-lock.json",
            "yarn.lock",
            "bun.lockb",
        ] {
            let p = self.root.join(cfg);
            if let Ok(bytes) = fs::read(&p) {
                deps.push(CacheDependency::LockfileHash(digest(&bytes)));
                break;
            }
        }
        for cfg in [
            "turbo.json",
            "nx.json",
            "pnpm-workspace.yaml",
            "tsconfig.json",
        ] {
            let p = self.root.join(cfg);
            if let Ok(bytes) = fs::read(&p) {
                deps.push(CacheDependency::ConfigHash(digest(&bytes)));
            }
        }

        let args_str = serde_json::to_string(args).unwrap_or_default();
        Some(CacheKey::new(
            CacheNamespace::Build,
            format!("build:{tool}:{args_str}"),
            1,
            "sha256",
            deps,
        ))
    }

    fn record_request(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.requests += 1;
        }
    }

    fn record_hit(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.hits += 1;
        }
    }

    fn record_miss(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.misses += 1;
        }
    }

    fn record_failure(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.failures += 1;
        }
    }
}
