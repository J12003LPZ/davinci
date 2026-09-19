pub mod model;
pub mod resolver;
pub mod runner;
pub mod tools;

pub use model::*;
pub use resolver::GitResolver;
pub use tools::{tool_spec, TOOL_NAMES};

use davinci_agent::{
    runtime::cache::{
        CacheDependency, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
    },
    ToolError, ToolResult,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
struct GitTelemetry {
    requests: u64,
    hits: u64,
    misses: u64,
    failures: u64,
}

#[derive(Debug, Clone)]
pub struct GitIntelligence {
    root: PathBuf,
    cache: CacheRuntime,
    config: GitIntelligenceConfig,
    telemetry: Arc<Mutex<GitTelemetry>>,
}

impl Default for GitIntelligence {
    fn default() -> Self {
        Self::with_root(Path::new("."), CacheRuntime::default())
    }
}

impl GitIntelligence {
    pub fn new(root: &Path, cache: CacheRuntime, config: GitIntelligenceConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            cache,
            config,
            telemetry: Arc::new(Mutex::new(GitTelemetry::default())),
        }
    }

    pub fn with_root(root: &Path, cache: CacheRuntime) -> Self {
        Self::new(root, cache, GitIntelligenceConfig::default())
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        if !self.config.enabled {
            return Ok(ToolResult {
                content: "git intelligence is disabled in settings".to_string(),
                details: Some(json!({ "error": "git intelligence is disabled in settings" })),
                is_error: true,
            });
        }

        self.record_request();

        let resolver = match GitResolver::new(&self.root, self.config.clone()) {
            Ok(r) => r,
            Err(err) => {
                self.record_failure();
                return Ok(ToolResult {
                    content: format!("Git error: {err}"),
                    details: Some(json!({ "error": err })),
                    is_error: true,
                });
            }
        };

        match name {
            "git_symbol_history" => self.execute_symbol_history(&resolver, args),
            "git_related_commits" => self.execute_related_commits(&resolver, args),
            "git_changed_symbols" => self.execute_changed_symbols(&resolver, args),
            "git_branch_diff" => self.execute_branch_diff(&resolver, args),
            "git_blame_symbol" => self.execute_blame_symbol(&resolver, args),
            "git_commit_context" => self.execute_commit_context(&resolver, args),
            "git_conflict_explain" => self.execute_conflict_explain(&resolver, args),
            _ => {
                self.record_failure();
                Err(ToolError::Unknown(format!(
                    "unknown git intelligence tool '{name}'"
                )))
            }
        }
    }

    fn execute_symbol_history(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitSymbolHistoryArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_symbol_history: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("git_symbol_history", args);
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

        match resolver.symbol_history(&parsed.symbol, parsed.path.as_deref(), parsed.max_commits) {
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_related_commits(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitRelatedCommitsArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_related_commits: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("git_related_commits", args);
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

        match resolver.related_commits(
            parsed.query.as_deref(),
            parsed.path.as_deref(),
            parsed.symbol.as_deref(),
            parsed.limit,
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_changed_symbols(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitChangedSymbolsArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_changed_symbols: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("git_changed_symbols", args);
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

        match resolver.changed_symbols(
            parsed.base.as_deref(),
            parsed.head.as_deref(),
            parsed.path.as_deref(),
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_branch_diff(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitBranchDiffArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_branch_diff: invalid args: {e}")))?;

        match resolver.branch_diff(
            parsed.base.as_deref(),
            parsed.head.as_deref(),
            parsed.stat_only,
            parsed.max_files,
        ) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_blame_symbol(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitBlameSymbolArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_blame_symbol: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("git_blame_symbol", args);
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

        match resolver.blame_symbol(&parsed.symbol, &parsed.path, parsed.revision.as_deref()) {
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_commit_context(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitCommitContextArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_commit_context: invalid args: {e}")))?;

        let cache_key = self.compute_cache_key("git_commit_context", args);
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

        match resolver.commit_context(&parsed.commit) {
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    fn execute_conflict_explain(
        &self,
        resolver: &GitResolver,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let parsed: tools::GitConflictExplainArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("git_conflict_explain: invalid args: {e}")))?;

        match resolver.conflict_explain(parsed.path.as_deref()) {
            Ok(res) => {
                let val =
                    serde_json::to_value(&res).map_err(|e| ToolError::Failed(e.to_string()))?;
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
                    details: Some(json!({ "error": err })),
                    is_error: true,
                })
            }
        }
    }

    pub fn status(&self) -> Value {
        let is_git = runner::is_git_repo(&self.root);
        let head = if is_git {
            runner::current_head(&self.root).unwrap_or(None)
        } else {
            None
        };
        let branch = if is_git {
            runner::current_branch(&self.root).unwrap_or(None)
        } else {
            None
        };
        let t = self.telemetry.lock().unwrap().clone();

        json!({
            "enabled": self.config.enabled,
            "root": self.root.to_string_lossy(),
            "is_git_repo": is_git,
            "head": head,
            "branch": branch,
            "telemetry": {
                "requests": t.requests,
                "hits": t.hits,
                "misses": t.misses,
                "failures": t.failures,
            }
        })
    }

    fn compute_cache_key(&self, tool: &str, args: &Value) -> Option<CacheKey> {
        let head_commit = runner::current_head(&self.root)
            .ok()
            .flatten()
            .unwrap_or_default();
        let deps = vec![CacheDependency::GitObject(head_commit)];

        let args_str = serde_json::to_string(args).unwrap_or_default();
        Some(CacheKey::new(
            CacheNamespace::Git,
            format!("git:{tool}:{args_str}"),
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

    pub fn record_hit(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.hits += 1;
        }
    }

    pub fn record_miss(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.misses += 1;
        }
    }

    pub fn record_failure(&self) {
        if let Ok(mut t) = self.telemetry.lock() {
            t.failures += 1;
        }
    }
}
