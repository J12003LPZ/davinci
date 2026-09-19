use super::{LockedPackage, LockfileData, LockfileKind};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn parse(content: &str) -> Result<LockfileData, String> {
    let root: Value =
        serde_json::from_str(content).map_err(|e| format!("invalid package-lock.json: {e}"))?;
    let lockfile_version_num = root
        .get("lockfileVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let mut data = LockfileData {
        kind: LockfileKind::Npm,
        lockfile_version: Some(lockfile_version_num.to_string()),
        packages: BTreeMap::new(),
        workspace_packages: BTreeMap::new(),
    };

    // If lockfileVersion >= 2, packages map is preferred
    if let Some(packages) = root.get("packages").and_then(Value::as_object) {
        for (key, val) in packages {
            if key.is_empty() {
                continue;
            }
            let version = val
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if version.is_empty() {
                continue;
            }
            let resolved = val
                .get("resolved")
                .and_then(Value::as_str)
                .map(str::to_string);
            let integrity = val
                .get("integrity")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut dependencies = BTreeMap::new();
            if let Some(deps) = val.get("dependencies").and_then(Value::as_object) {
                for (dep_name, dep_ver) in deps {
                    if let Some(v) = dep_ver.as_str() {
                        dependencies.insert(dep_name.clone(), v.to_string());
                    }
                }
            }

            // Extract package name and workspace subpath
            if let Some((ws_prefix, pkg_name)) = key.rsplit_once("/node_modules/") {
                let locked = LockedPackage {
                    name: pkg_name.to_string(),
                    version: version.clone(),
                    resolved: resolved.clone(),
                    integrity: integrity.clone(),
                    dependencies: dependencies.clone(),
                };
                data.workspace_packages
                    .entry(ws_prefix.to_string())
                    .or_default()
                    .insert(pkg_name.to_string(), locked);
                // Also ensure available in global packages if missing
                data.packages
                    .entry(pkg_name.to_string())
                    .or_insert_with(|| LockedPackage {
                        name: pkg_name.to_string(),
                        version: version.clone(),
                        resolved: resolved.clone(),
                        integrity: integrity.clone(),
                        dependencies: dependencies.clone(),
                    });
            } else if let Some(pkg_name) = key.strip_prefix("node_modules/") {
                let locked = LockedPackage {
                    name: pkg_name.to_string(),
                    version,
                    resolved,
                    integrity,
                    dependencies,
                };
                data.packages.insert(pkg_name.to_string(), locked);
            }
        }
    }

    // Fallback or v1 dependencies
    if data.packages.is_empty() {
        if let Some(deps) = root.get("dependencies").and_then(Value::as_object) {
            collect_v1_dependencies(deps, &mut data.packages);
        }
    }

    Ok(data)
}

fn collect_v1_dependencies(
    deps: &serde_json::Map<String, Value>,
    out: &mut BTreeMap<String, LockedPackage>,
) {
    for (name, val) in deps {
        let version = val
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if version.is_empty() {
            continue;
        }
        let resolved = val
            .get("resolved")
            .and_then(Value::as_str)
            .map(str::to_string);
        let integrity = val
            .get("integrity")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut dependencies = BTreeMap::new();
        if let Some(requires) = val.get("requires").and_then(Value::as_object) {
            for (req_name, req_ver) in requires {
                if let Some(v) = req_ver.as_str() {
                    dependencies.insert(req_name.clone(), v.to_string());
                }
            }
        }
        out.entry(name.clone()).or_insert_with(|| LockedPackage {
            name: name.clone(),
            version,
            resolved,
            integrity,
            dependencies,
        });
        if let Some(nested) = val.get("dependencies").and_then(Value::as_object) {
            collect_v1_dependencies(nested, out);
        }
    }
}
