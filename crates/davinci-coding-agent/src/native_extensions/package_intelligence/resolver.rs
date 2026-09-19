use super::locks::{parse_lockfile, LockfileData, LockfileKind};
use super::model::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ResolvedPackage {
    pub package_name: String,
    pub workspace_scope: String,
    pub workspace_manifest_path: String,
    pub declared_version: Option<String>,
    pub dependency_type: Option<String>,
    pub locked_version: Option<String>,
    pub lock_kind: Option<LockfileKind>,
    pub installed_manifest_path: Option<String>,
    pub installed_version: Option<String>,
    #[allow(dead_code)]
    pub installed_dir: Option<PathBuf>,
    pub types_path: Option<String>,
    pub main_path: Option<String>,
    pub exports: PackageExports,
    pub dependencies: Vec<String>,
    pub dev_dependencies: Vec<String>,
    pub peer_dependencies: Vec<String>,
    pub optional_dependencies: Vec<String>,
    pub warnings: Vec<String>,
    pub lock_data: Option<LockfileData>,
}

pub fn resolve_package(
    root: &Path,
    workspace: Option<&str>,
    target_pkg: &str,
) -> Result<ResolvedPackage, String> {
    if target_pkg.is_empty()
        || target_pkg.contains("..")
        || target_pkg.contains('\\')
        || target_pkg.starts_with('/')
        || (target_pkg.contains('/') && !target_pkg.starts_with('@'))
        || target_pkg.chars().filter(|c| *c == '/').count() > 1
    {
        return Err(format!("invalid or malicious package name: {target_pkg}"));
    }

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ws_rel = workspace.unwrap_or(".").trim_matches(['/', '\\']);
    if ws_rel.contains("..") {
        return Err(format!("workspace escapes repository root: {ws_rel}"));
    }
    let ws_dir = if ws_rel == "." || ws_rel.is_empty() {
        canonical_root.clone()
    } else {
        let candidate = canonical_root.join(ws_rel);
        if !candidate.starts_with(&canonical_root) {
            return Err(format!("workspace escapes repository root: {ws_rel}"));
        }
        candidate.canonicalize().unwrap_or(candidate)
    };

    let mut warnings = Vec::new();

    // 1. Read workspace package.json
    let ws_manifest_path = ws_dir.join("package.json");
    let mut declared_version = None;
    let mut dependency_type = None;
    let mut ws_manifest_rel = ws_rel.to_string();
    if ws_manifest_rel.is_empty() || ws_manifest_rel == "." {
        ws_manifest_rel = "package.json".to_string();
    } else {
        ws_manifest_rel = format!("{ws_rel}/package.json");
    }

    if let Ok(manifest_content) = fs::read_to_string(&ws_manifest_path) {
        if let Ok(val) = serde_json::from_str::<Value>(&manifest_content) {
            if let Some(deps) = val.get("dependencies").and_then(Value::as_object) {
                if let Some(v) = deps.get(target_pkg).and_then(Value::as_str) {
                    declared_version = Some(v.to_string());
                    dependency_type = Some("dependencies".to_string());
                }
            }
            if declared_version.is_none() {
                if let Some(deps) = val.get("devDependencies").and_then(Value::as_object) {
                    if let Some(v) = deps.get(target_pkg).and_then(Value::as_str) {
                        declared_version = Some(v.to_string());
                        dependency_type = Some("devDependencies".to_string());
                    }
                }
            }
            if declared_version.is_none() {
                if let Some(deps) = val.get("peerDependencies").and_then(Value::as_object) {
                    if let Some(v) = deps.get(target_pkg).and_then(Value::as_str) {
                        declared_version = Some(v.to_string());
                        dependency_type = Some("peerDependencies".to_string());
                    }
                }
            }
            if declared_version.is_none() {
                if let Some(deps) = val.get("optionalDependencies").and_then(Value::as_object) {
                    if let Some(v) = deps.get(target_pkg).and_then(Value::as_str) {
                        declared_version = Some(v.to_string());
                        dependency_type = Some("optionalDependencies".to_string());
                    }
                }
            }
        }
    } else {
        warnings.push(format!("workspace manifest not found: {ws_manifest_rel}"));
    }

    // 2. Discover Lockfile
    let (lock_data, lock_kind) = find_and_parse_lockfile(&ws_dir, &canonical_root, &mut warnings);

    let mut locked_version = None;
    if let Some(ref lock) = lock_data {
        // First check workspace importers if workspace != "."
        if ws_rel != "." && !ws_rel.is_empty() {
            if let Some(importer_pkgs) = lock.workspace_packages.get(ws_rel) {
                if let Some(pkg) = importer_pkgs.get(target_pkg) {
                    locked_version = Some(pkg.version.clone());
                }
            }
        }
        if locked_version.is_none() {
            if let Some(pkg) = lock.packages.get(target_pkg) {
                locked_version = Some(pkg.version.clone());
            }
        }
    } else {
        warnings.push("no lockfile found in workspace or root".to_string());
    }

    // 3. Find installed package in node_modules
    let (installed_dir, installed_pkg_json) =
        find_installed_package(&ws_dir, &canonical_root, target_pkg, &mut warnings);

    let mut installed_version = None;
    let mut types_path = None;
    let mut main_path = None;
    let mut exports = PackageExports::default();
    let mut dependencies = Vec::new();
    let mut dev_dependencies = Vec::new();
    let mut peer_dependencies = Vec::new();
    let mut optional_dependencies = Vec::new();
    let mut installed_manifest_rel = None;

    if let Some(ref pkg_dir) = installed_dir {
        if let Some(ref content) = installed_pkg_json {
            if let Ok(val) = serde_json::from_str::<Value>(content) {
                installed_version = val
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                main_path = val.get("main").and_then(Value::as_str).map(str::to_string);

                // Parse exports
                if let Some(exp_val) = val.get("exports") {
                    exports = parse_package_exports(exp_val);
                }

                // Types resolution
                let types_entry = val
                    .get("types")
                    .or_else(|| val.get("typings"))
                    .and_then(Value::as_str);

                if let Some(t) = types_entry {
                    let rel_clean = t.trim_start_matches("./");
                    let target_types = pkg_dir.join(rel_clean);
                    if target_types.exists() && target_types.starts_with(&canonical_root) {
                        types_path = path_to_repo_rel(&target_types, &canonical_root);
                    } else if !target_types.starts_with(&canonical_root) {
                        warnings.push(format!("types path escapes repository root: {t}"));
                    }
                } else if let Some(root_t) =
                    exports.root_target.as_ref().and_then(|t| t.types.as_ref())
                {
                    let rel_clean = root_t.trim_start_matches("./");
                    let target_types = pkg_dir.join(rel_clean);
                    if target_types.exists() && target_types.starts_with(&canonical_root) {
                        types_path = path_to_repo_rel(&target_types, &canonical_root);
                    }
                } else {
                    let index_dts = pkg_dir.join("index.d.ts");
                    if index_dts.exists() && index_dts.starts_with(&canonical_root) {
                        types_path = path_to_repo_rel(&index_dts, &canonical_root);
                    }
                }

                // If types not in package, check @types/<pkg>
                if types_path.is_none() {
                    let at_types_name = if let Some(stripped) = target_pkg.strip_prefix('@') {
                        stripped.replace('/', "__")
                    } else {
                        target_pkg.to_string()
                    };
                    let at_types_dir = canonical_root
                        .join("node_modules")
                        .join("@types")
                        .join(&at_types_name);
                    let at_types_index = at_types_dir.join("index.d.ts");
                    if at_types_index.exists() && at_types_index.starts_with(&canonical_root) {
                        types_path = path_to_repo_rel(&at_types_index, &canonical_root);
                    }
                }

                // Dependencies
                if let Some(deps) = val.get("dependencies").and_then(Value::as_object) {
                    dependencies = deps.keys().cloned().collect();
                }
                if let Some(deps) = val.get("devDependencies").and_then(Value::as_object) {
                    dev_dependencies = deps.keys().cloned().collect();
                }
                if let Some(deps) = val.get("peerDependencies").and_then(Value::as_object) {
                    peer_dependencies = deps.keys().cloned().collect();
                }
                if let Some(deps) = val.get("optionalDependencies").and_then(Value::as_object) {
                    optional_dependencies = deps.keys().cloned().collect();
                }

                let manifest_file = pkg_dir.join("package.json");
                installed_manifest_rel = path_to_repo_rel(&manifest_file, &canonical_root);
            }
        }
    } else {
        warnings.push(format!(
            "package not installed in node_modules: {target_pkg}"
        ));
    }

    // Check version discrepancies
    if let (Some(ref locked), Some(ref inst)) = (&locked_version, &installed_version) {
        if locked != inst {
            warnings.push(format!(
                "version mismatch: installed {inst} differs from lockfile {locked}"
            ));
        }
    }

    Ok(ResolvedPackage {
        package_name: target_pkg.to_string(),
        workspace_scope: ws_rel.to_string(),
        workspace_manifest_path: ws_manifest_rel,
        declared_version,
        dependency_type,
        locked_version,
        lock_kind,
        installed_manifest_path: installed_manifest_rel,
        installed_version,
        installed_dir,
        types_path,
        main_path,
        exports,
        dependencies,
        dev_dependencies,
        peer_dependencies,
        optional_dependencies,
        warnings,
        lock_data,
    })
}

fn find_and_parse_lockfile(
    ws_dir: &Path,
    root: &Path,
    warnings: &mut Vec<String>,
) -> (Option<LockfileData>, Option<LockfileKind>) {
    let mut candidate_dirs = vec![ws_dir];
    if ws_dir != root {
        candidate_dirs.push(root);
    }

    for dir in candidate_dirs {
        for lock_name in &[
            "pnpm-lock.yaml",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
        ] {
            let lock_path = dir.join(lock_name);
            if lock_path.is_file() {
                if let Ok(content) = fs::read_to_string(&lock_path) {
                    match parse_lockfile(lock_name, &content) {
                        Ok(data) => {
                            let kind = data.kind;
                            return (Some(data), Some(kind));
                        }
                        Err(err) => {
                            warnings.push(format!("failed to parse {lock_name}: {err}"));
                        }
                    }
                }
            }
        }
    }

    (None, None)
}

fn find_installed_package(
    ws_dir: &Path,
    root: &Path,
    target_pkg: &str,
    warnings: &mut Vec<String>,
) -> (Option<PathBuf>, Option<String>) {
    let mut candidate_nm_dirs = vec![ws_dir.join("node_modules")];
    if ws_dir != root {
        candidate_nm_dirs.push(root.join("node_modules"));
    }

    for nm in candidate_nm_dirs {
        let pkg_candidate = nm.join(target_pkg);
        if pkg_candidate.exists() {
            // Check if it's a symlink (e.g. pnpm or workspace link)
            let resolved_dir = match pkg_candidate.canonicalize() {
                Ok(canon) => {
                    if !canon.starts_with(root) {
                        warnings.push(format!(
                            "security refusal: package link escapes repository root for {target_pkg}"
                        ));
                        return (None, None);
                    }
                    canon
                }
                Err(_) => pkg_candidate.clone(),
            };

            let manifest_path = resolved_dir.join("package.json");
            if manifest_path.is_file() {
                if let Ok(content) = fs::read_to_string(&manifest_path) {
                    return (Some(resolved_dir), Some(content));
                }
            }
        }
    }

    // Check .pnpm virtual store if present
    let pnpm_store = root.join("node_modules").join(".pnpm");
    if pnpm_store.is_dir() {
        if let Ok(entries) = fs::read_dir(&pnpm_store) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let clean_name = name.strip_prefix('/').unwrap_or(&name);
                if clean_name.starts_with(target_pkg) {
                    let candidate = entry.path().join("node_modules").join(target_pkg);
                    if candidate.is_dir() {
                        let manifest = candidate.join("package.json");
                        if manifest.is_file() {
                            if let Ok(content) = fs::read_to_string(&manifest) {
                                return (Some(candidate), Some(content));
                            }
                        }
                    }
                }
            }
        }
    }

    (None, None)
}

fn parse_package_exports(val: &Value) -> PackageExports {
    let mut exports = PackageExports::default();
    match val {
        Value::String(s) => {
            exports.root_target = Some(ExportTarget {
                default: Some(s.clone()),
                ..Default::default()
            });
        }
        Value::Object(map) => {
            let has_subpaths = map.keys().any(|k| k.starts_with('.'));
            if !has_subpaths {
                exports.root_target = Some(parse_export_target(val));
            } else {
                for (key, target_val) in map {
                    let target = parse_export_target(target_val);
                    if key == "." {
                        exports.root_target = Some(target);
                    } else {
                        exports.subpaths.insert(key.clone(), target);
                    }
                }
            }
        }
        _ => {}
    }
    exports
}

fn parse_export_target(val: &Value) -> ExportTarget {
    let mut target = ExportTarget::default();
    match val {
        Value::String(s) => {
            target.default = Some(s.clone());
        }
        Value::Object(map) => {
            target.types = map.get("types").and_then(Value::as_str).map(str::to_string);
            target.import = map
                .get("import")
                .and_then(Value::as_str)
                .map(str::to_string);
            target.require = map
                .get("require")
                .and_then(Value::as_str)
                .map(str::to_string);
            target.default = map
                .get("default")
                .and_then(Value::as_str)
                .map(str::to_string);
            target.browser = map
                .get("browser")
                .and_then(Value::as_str)
                .map(str::to_string);
            target.node = map.get("node").and_then(Value::as_str).map(str::to_string);
        }
        _ => {}
    }
    target
}

pub fn path_to_repo_rel(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}
