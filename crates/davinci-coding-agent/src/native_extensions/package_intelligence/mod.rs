pub mod locks;
pub mod model;
pub mod resolver;
pub mod symbols;
pub mod tools;

pub use model::*;
pub use tools::{
    tool_spec, PackageDependentsArgs, PackageExportsArgs, PackageInfoArgs, PackageSymbolArgs,
    PackageWhyArgs, TOOL_NAMES,
};

use davinci_agent::{
    runtime::cache::{
        digest, CacheDependency, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
    },
    ToolError, ToolResult,
};
use resolver::resolve_package;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageIntelligenceConfig {
    pub enabled: bool,
}

impl Default for PackageIntelligenceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Default, Serialize)]
struct PackageTelemetry {
    requests: u64,
    hits: u64,
    misses: u64,
    failures: u64,
}

#[derive(Debug, Clone)]
pub struct PackageIntelligence {
    root: PathBuf,
    cache: CacheRuntime,
    config: PackageIntelligenceConfig,
    telemetry: Arc<Mutex<PackageTelemetry>>,
}

impl Default for PackageIntelligence {
    fn default() -> Self {
        Self::new(
            Path::new("."),
            CacheRuntime::default(),
            PackageIntelligenceConfig::default(),
        )
    }
}

impl PackageIntelligence {
    pub fn new(root: &Path, cache: CacheRuntime, config: PackageIntelligenceConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            cache,
            config,
            telemetry: Arc::new(Mutex::new(PackageTelemetry::default())),
        }
    }

    #[allow(dead_code)]
    pub fn with_root(root: &Path, cache: CacheRuntime) -> Self {
        Self::new(root, cache, PackageIntelligenceConfig::default())
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        if !self.config.enabled {
            return Err(ToolError::Failed(
                "packageIntelligence is disabled in settings".into(),
            ));
        }

        let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
        tele.requests += 1;
        drop(tele);

        let result = match name {
            "package_info" => self.execute_package_info(args),
            "package_exports" => self.execute_package_exports(args),
            "package_symbol" => self.execute_package_symbol(args),
            "package_dependents" => self.execute_package_dependents(args),
            "package_why" => self.execute_package_why(args),
            _ => Err(ToolError::Unknown(name.to_string())),
        };

        if result.is_err() {
            let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
            tele.failures += 1;
        }

        result
    }

    pub fn status(&self) -> Value {
        let tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "enabled": self.config.enabled,
            "root": self.root.display().to_string(),
            "telemetry": {
                "requests": tele.requests,
                "hits": tele.hits,
                "misses": tele.misses,
                "failures": tele.failures,
            }
        })
    }

    fn execute_package_info(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: PackageInfoArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("package_info: invalid arguments: {e}")))?;
        let pkg = &parsed.package;
        let ws = parsed.workspace.as_deref();

        // Check CacheRuntime
        let cache_key = self.compute_cache_key("package_info", pkg, ws);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                tele.hits += 1;
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        let resolved = resolve_package(&self.root, ws, pkg)
            .map_err(|e| ToolError::Failed(format!("failed to resolve package: {e}")))?;

        let dependents = find_direct_dependents(&self.root, pkg);

        let info = PackageInfoResult {
            package_name: resolved.package_name,
            declared_version: resolved.declared_version,
            installed_version: resolved.installed_version,
            locked_version: resolved.locked_version,
            lock_kind: resolved.lock_kind.map(|k| k.as_str().to_string()),
            workspace_scope: resolved.workspace_scope,
            manifest_path: resolved
                .installed_manifest_path
                .unwrap_or(resolved.workspace_manifest_path),
            types_path: resolved.types_path,
            main_path: resolved.main_path,
            exports_summary: summarize_exports(&resolved.exports),
            dependencies: resolved.dependencies,
            dev_dependencies: resolved.dev_dependencies,
            peer_dependencies: resolved.peer_dependencies,
            optional_dependencies: resolved.optional_dependencies,
            dependents,
            warnings: resolved.warnings,
        };

        let val = serde_json::to_value(&info).map_err(|e| ToolError::Failed(e.to_string()))?;

        // Write to CacheRuntime
        if let Some(key) = cache_key {
            let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
            let _ = self.cache.put(&req, val.clone(), || Ok(()));
            let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
            tele.misses += 1;
        }

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap_or_default(),
            details: Some(val),
            is_error: false,
        })
    }

    fn execute_package_exports(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: PackageExportsArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("package_exports: invalid arguments: {e}")))?;
        let pkg = &parsed.package;
        let ws = parsed.workspace.as_deref();

        let cache_key = self.compute_cache_key("package_exports", pkg, ws);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                tele.hits += 1;
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        let resolved = resolve_package(&self.root, ws, pkg)
            .map_err(|e| ToolError::Failed(format!("failed to resolve package: {e}")))?;

        let exports_result = PackageExportsResult {
            package_name: resolved.package_name,
            installed_version: resolved.installed_version,
            manifest_path: resolved
                .installed_manifest_path
                .unwrap_or(resolved.workspace_manifest_path),
            exports: resolved.exports,
            warnings: resolved.warnings,
        };

        let val =
            serde_json::to_value(&exports_result).map_err(|e| ToolError::Failed(e.to_string()))?;

        if let Some(key) = cache_key {
            let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
            let _ = self.cache.put(&req, val.clone(), || Ok(()));
            let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
            tele.misses += 1;
        }

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap_or_default(),
            details: Some(val),
            is_error: false,
        })
    }

    fn execute_package_symbol(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: PackageSymbolArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("package_symbol: invalid arguments: {e}")))?;
        let pkg = &parsed.package;
        let symbol = &parsed.symbol;
        let ws = parsed.workspace.as_deref();

        let symbol_query = format!("{pkg}#{symbol}");
        let cache_key = self.compute_cache_key("package_symbol", &symbol_query, ws);
        if let Some(key) = &cache_key {
            let req = CacheRequest::new(key.clone(), CachePolicy::PersistentImmutable);
            if let Ok(Some(cached_val)) = self.cache.get::<Value>(&req, || Ok(())) {
                let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                tele.hits += 1;
                let val: Value = (*cached_val).clone();
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&val).unwrap_or_default(),
                    details: Some(val),
                    is_error: false,
                });
            }
        }

        let resolved = resolve_package(&self.root, ws, pkg)
            .map_err(|e| ToolError::Failed(format!("failed to resolve package: {e}")))?;

        let mut warnings = resolved.warnings;

        let result = if let Some(ref rel_types) = resolved.types_path {
            let abs_types = self.root.join(rel_types);
            match symbols::find_symbol_in_types(
                &abs_types,
                rel_types,
                &resolved.package_name,
                resolved.installed_version.as_deref(),
                symbol,
            ) {
                Ok(mut sym_res) => {
                    sym_res.warnings.append(&mut warnings);
                    sym_res
                }
                Err(err) => {
                    warnings.push(format!("error parsing types: {err}"));
                    PackageSymbolResult {
                        package_name: resolved.package_name,
                        installed_version: resolved.installed_version,
                        symbol: symbol.to_string(),
                        found: false,
                        file_path: Some(rel_types.clone()),
                        line: None,
                        declaration: None,
                        kind: None,
                        documentation: None,
                        warnings,
                    }
                }
            }
        } else {
            warnings.push("no type declaration file (.d.ts) available for package".to_string());
            PackageSymbolResult {
                package_name: resolved.package_name,
                installed_version: resolved.installed_version,
                symbol: symbol.to_string(),
                found: false,
                file_path: None,
                line: None,
                declaration: None,
                kind: None,
                documentation: None,
                warnings,
            }
        };

        let val = serde_json::to_value(&result).map_err(|e| ToolError::Failed(e.to_string()))?;

        if let Some(key) = cache_key {
            let req = CacheRequest::new(key, CachePolicy::PersistentImmutable);
            let _ = self.cache.put(&req, val.clone(), || Ok(()));
            let mut tele = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
            tele.misses += 1;
        }

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap_or_default(),
            details: Some(val),
            is_error: false,
        })
    }

    fn execute_package_dependents(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: PackageDependentsArgs = serde_json::from_value(args.clone()).map_err(|e| {
            ToolError::Failed(format!("package_dependents: invalid arguments: {e}"))
        })?;
        let pkg = &parsed.package;
        let ws = parsed.workspace.as_deref();

        let resolved = resolve_package(&self.root, ws, pkg)
            .map_err(|e| ToolError::Failed(format!("failed to resolve package: {e}")))?;

        let direct_dependents = find_direct_dependents(&self.root, pkg);
        let internal_files = find_internal_referencing_files(&self.root, pkg);

        let result = PackageDependentsResult {
            package_name: resolved.package_name,
            workspace_scope: resolved.workspace_scope,
            direct_dependents,
            internal_files_referencing: internal_files,
            warnings: resolved.warnings,
        };

        let val = serde_json::to_value(&result).map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap_or_default(),
            details: Some(val),
            is_error: false,
        })
    }

    fn execute_package_why(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let parsed: PackageWhyArgs = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::Failed(format!("package_why: invalid arguments: {e}")))?;
        let pkg = &parsed.package;
        let ws = parsed.workspace.as_deref();

        let resolved = resolve_package(&self.root, ws, pkg)
            .map_err(|e| ToolError::Failed(format!("failed to resolve package: {e}")))?;

        let mut reasons = Vec::new();
        let mut warnings = resolved.warnings;

        if let Some(ref dt) = resolved.dependency_type {
            reasons.push(DependencyReason {
                from_package: resolved.workspace_manifest_path.clone(),
                dependency_type: dt.clone(),
                required_range: resolved.declared_version.clone().unwrap_or_default(),
                resolved_version: resolved
                    .installed_version
                    .clone()
                    .or(resolved.locked_version.clone())
                    .unwrap_or_default(),
            });
        }

        // Search in lockfile for packages requiring this package
        if let Some(ref lock) = resolved.lock_data {
            for (parent_pkg, locked_info) in &lock.packages {
                if let Some(req_ver) = locked_info.dependencies.get(pkg) {
                    reasons.push(DependencyReason {
                        from_package: format!("{}@{}", parent_pkg, locked_info.version),
                        dependency_type: "lock_dependency".to_string(),
                        required_range: req_ver.clone(),
                        resolved_version: resolved
                            .installed_version
                            .clone()
                            .or(resolved.locked_version.clone())
                            .unwrap_or_default(),
                    });
                }
            }
        }

        if reasons.is_empty() {
            warnings.push(format!(
                "no direct or lock dependencies found explaining presence of {pkg}"
            ));
        }

        let why = PackageWhyResult {
            package_name: resolved.package_name,
            workspace_scope: resolved.workspace_scope,
            installed_version: resolved.installed_version,
            locked_version: resolved.locked_version,
            declared_range: resolved.declared_version,
            dependency_type: resolved.dependency_type,
            reasons,
            warnings,
        };

        let val = serde_json::to_value(&why).map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap_or_default(),
            details: Some(val),
            is_error: false,
        })
    }

    fn compute_cache_key(
        &self,
        tool: &str,
        pkg_or_symbol: &str,
        ws: Option<&str>,
    ) -> Option<CacheKey> {
        let ws_dir = self.root.join(ws.unwrap_or(""));
        let manifest = ws_dir.join("package.json");
        let manifest_bytes = fs::read(&manifest).ok()?;
        let manifest_hash = digest(&manifest_bytes);

        let mut deps = vec![CacheDependency::PackageManifestHash(manifest_hash)];

        for lock in &["pnpm-lock.yaml", "package-lock.json", "yarn.lock"] {
            let lock_path = ws_dir.join(lock);
            if let Ok(lock_bytes) = fs::read(&lock_path) {
                deps.push(CacheDependency::LockfileHash(digest(&lock_bytes)));
                break;
            }
        }

        Some(CacheKey::new(
            CacheNamespace::Package,
            format!("{tool}:{}:{pkg_or_symbol}", ws.unwrap_or(".")),
            1,
            "sha256",
            deps,
        ))
    }
}

fn summarize_exports(exports: &PackageExports) -> Option<String> {
    if exports.root_target.is_none() && exports.subpaths.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(ref root) = exports.root_target {
        let mut sub = Vec::new();
        if root.types.is_some() {
            sub.push("types");
        }
        if root.import.is_some() {
            sub.push("import");
        }
        if root.require.is_some() {
            sub.push("require");
        }
        if root.default.is_some() {
            sub.push("default");
        }
        parts.push(format!(". [{}]", sub.join(", ")));
    }
    for k in exports.subpaths.keys() {
        parts.push(k.clone());
    }
    Some(parts.join("; "))
}

fn find_direct_dependents(root: &Path, target_pkg: &str) -> Vec<String> {
    let mut dependents = Vec::new();
    // Scan all package.json files within root (up to 64 manifests)
    for entry in walkdir::WalkDir::new(root)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_name() == "package.json" {
            let path = entry.path();
            // Ignore node_modules manifests
            if path.to_string_lossy().contains("node_modules") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(val) = serde_json::from_str::<Value>(&content) {
                    let has_dep = [
                        "dependencies",
                        "devDependencies",
                        "peerDependencies",
                        "optionalDependencies",
                    ]
                    .iter()
                    .any(|field| {
                        val.get(*field)
                            .and_then(Value::as_object)
                            .map(|deps| deps.contains_key(target_pkg))
                            .unwrap_or(false)
                    });
                    if has_dep {
                        let name = val
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                entry
                                    .path()
                                    .strip_prefix(root)
                                    .ok()
                                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                                    .unwrap_or_else(|| "package.json".to_string())
                            });
                        dependents.push(name);
                    }
                }
            }
        }
    }
    dependents
}

fn find_internal_referencing_files(root: &Path, target_pkg: &str) -> Vec<String> {
    let mut referencing = Vec::new();
    // Bounded search for import/require of target_pkg in .ts, .tsx, .js, .jsx
    let import_pat = format!("from '{target_pkg}'");
    let import_double = format!("from \"{target_pkg}\"");
    let req_pat = format!("require('{target_pkg}')");
    let req_double = format!("require(\"{target_pkg}\")");

    for entry in walkdir::WalkDir::new(root)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.to_string_lossy().contains("node_modules") {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "ts" | "tsx" | "js" | "jsx") {
            if let Ok(content) = fs::read_to_string(path) {
                if content.contains(&import_pat)
                    || content.contains(&import_double)
                    || content.contains(&req_pat)
                    || content.contains(&req_double)
                {
                    if let Ok(rel) = path.strip_prefix(root) {
                        referencing.push(rel.to_string_lossy().replace('\\', "/"));
                        if referencing.len() >= 20 {
                            break;
                        }
                    }
                }
            }
        }
    }
    referencing
}
