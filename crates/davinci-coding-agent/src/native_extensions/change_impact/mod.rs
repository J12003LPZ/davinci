//! Native Change Impact Engine (P9).
//! Explains the likely edit blast radius across semantic references, structural AST,
//! tests, packages, builds, API/config, and potential browser flows.

pub mod adapter;
pub mod analyzer;
pub mod model;
pub mod tools;

pub use adapter::LanguageIntelligenceAdapter;
pub use analyzer::ChangeImpactAnalyzer;
pub use model::*;
pub use tools::{tool_spec, TOOL_NAMES};

use crate::native_extensions::{
    build_intelligence::BuildIntelligence, git_intelligence::GitIntelligence,
    language_intelligence::LanguageIntelligence, package_intelligence::PackageIntelligence,
    repo_intelligence::RepoIntelligence, test_impact::TestImpact,
};
use davinci_agent::{
    runtime::cache::{
        digest, CacheDependency, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
    },
    ToolError, ToolResult,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Debug, Default, Clone)]
struct ImpactTelemetry {
    requests: u64,
    hits: u64,
    misses: u64,
    failures: u64,
    last_latency_ms: f64,
}

#[derive(Debug, Clone)]
pub struct ChangeImpact {
    root: PathBuf,
    cache: CacheRuntime,
    config: ChangeImpactConfig,
    repo: RepoIntelligence,
    test_impact: TestImpact,
    package_intelligence: PackageIntelligence,
    build_intelligence: BuildIntelligence,
    git_intelligence: GitIntelligence,
    language: LanguageIntelligence,
    telemetry: Arc<Mutex<ImpactTelemetry>>,
}

impl Default for ChangeImpact {
    fn default() -> Self {
        Self::with_root(
            Path::new("."),
            CacheRuntime::default(),
            RepoIntelligence::default(),
            TestImpact::default(),
            PackageIntelligence::default(),
            BuildIntelligence::default(),
            GitIntelligence::default(),
            LanguageIntelligence::default(),
        )
    }
}

impl ChangeImpact {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: &Path,
        cache: CacheRuntime,
        config: ChangeImpactConfig,
        repo: RepoIntelligence,
        test_impact: TestImpact,
        package_intelligence: PackageIntelligence,
        build_intelligence: BuildIntelligence,
        git_intelligence: GitIntelligence,
        language: LanguageIntelligence,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            cache,
            config,
            repo,
            test_impact,
            package_intelligence,
            build_intelligence,
            git_intelligence,
            language,
            telemetry: Arc::new(Mutex::new(ImpactTelemetry::default())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_root(
        root: &Path,
        cache: CacheRuntime,
        repo: RepoIntelligence,
        test_impact: TestImpact,
        package_intelligence: PackageIntelligence,
        build_intelligence: BuildIntelligence,
        git_intelligence: GitIntelligence,
        language: LanguageIntelligence,
    ) -> Self {
        Self::new(
            root,
            cache,
            ChangeImpactConfig::default(),
            repo,
            test_impact,
            package_intelligence,
            build_intelligence,
            git_intelligence,
            language,
        )
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        if !self.config.enabled {
            return Ok(ToolResult {
                content: "Change impact analysis is disabled in settings".to_string(),
                is_error: false,
                details: Some(json!({"available": false, "enabled": false})),
            });
        }

        let start = Instant::now();
        {
            let mut telem = self.telemetry.lock().unwrap();
            telem.requests += 1;
        }

        let res = match name {
            "impact_analyze" => self.analyze_tool(args),
            _ => Err(ToolError::Unknown(format!("Tool '{name}' not found"))),
        };

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        {
            let mut telem = self.telemetry.lock().unwrap();
            telem.last_latency_ms = elapsed;
            if res.is_err() {
                telem.failures += 1;
            }
        }

        res
    }

    fn analyze_tool(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: tools::ImpactAnalyzeArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("Invalid arguments for impact_analyze: {e}")))?;

        // Check cache
        let args_str = serde_json::to_string(args).unwrap_or_default();
        let initial_key = CacheKey::new(
            CacheNamespace::Test,
            format!("impact_analyze:{args_str}"),
            1,
            "sha256",
            Vec::new(),
        );
        let req = CacheRequest::new(initial_key, CachePolicy::MemoryOnly);

        if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
            let mut telem = self.telemetry.lock().unwrap();
            telem.hits += 1;
            let val = (*cached_val).clone();
            return Ok(ToolResult {
                content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                is_error: false,
                details: Some(val),
            });
        }

        {
            let mut telem = self.telemetry.lock().unwrap();
            telem.misses += 1;
        }

        let analyzer = ChangeImpactAnalyzer::new(
            &self.root,
            &self.config,
            &self.repo,
            &self.test_impact,
            &self.package_intelligence,
            &self.build_intelligence,
            &self.git_intelligence,
            &self.language,
        );

        let report = analyzer
            .analyze(
                parsed.files,
                parsed.symbols,
                parsed.transaction_id,
                parsed.scope,
                parsed.limit,
            )
            .map_err(ToolError::Failed)?;

        let json_val = serde_json::to_value(&report)
            .map_err(|e| ToolError::Failed(format!("Failed to serialize report: {e}")))?;

        // Cache report with Source content dependencies
        let mut dependencies = Vec::new();
        for f in &report.files {
            dependencies.push(CacheDependency::ContentHash(digest(f.as_bytes())));
        }
        let dep_key = CacheKey::new(
            CacheNamespace::Test,
            format!("impact_analyze:{args_str}"),
            1,
            "sha256",
            dependencies,
        );
        let dep_req = CacheRequest::new(dep_key, CachePolicy::MemoryOnly);
        let _ = self.cache.put(&dep_req, json_val.clone(), || Ok(()));
        let _ = self.cache.put(&req, json_val.clone(), || Ok(()));

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&json_val).unwrap_or_default(),
            is_error: false,
            details: Some(json_val),
        })
    }

    pub fn status(&self) -> Value {
        let telem = self.telemetry.lock().unwrap().clone();
        json!({
            "enabled": self.config.enabled,
            "maxDepth": self.config.max_depth,
            "maxReferences": self.config.max_references,
            "maxFiles": self.config.max_files,
            "telemetry": {
                "requests": telem.requests,
                "hits": telem.hits,
                "misses": telem.misses,
                "failures": telem.failures,
                "lastLatencyMs": telem.last_latency_ms,
            }
        })
    }
}
