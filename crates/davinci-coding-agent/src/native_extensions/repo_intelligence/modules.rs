//! Conservative local module hints, never a package manager or filesystem resolver.
use super::scanner;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ModuleAlias {
    scope: String,
    pattern: String,
    targets: Vec<String>,
}

pub(super) fn normalize(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn read_config(root: &Path, path: &str) -> Option<Value> {
    let body = scanner::read_bounded(root, Path::new(path), 128 * 1024).ok()?;
    // tsconfig/jsconfig are JSONC (comments, trailing commas); manifests are strict.
    if path.ends_with("package.json") {
        serde_json::from_str::<Value>(&body).ok()
    } else {
        crate::native_extensions::build_intelligence::runners::tsconfig::parse_jsonc::<Value>(&body)
            .ok()
    }
}

pub(super) fn collect(root: &Path, paths: &[String]) -> Vec<ModuleAlias> {
    let mut aliases = Vec::new();
    for path in paths.iter().take(2000) {
        let Some(value) = read_config(root, path) else {
            continue;
        };
        let parent = Path::new(path).parent().unwrap_or(Path::new(""));
        if path.ends_with("package.json") {
            let Some(name) = value["name"].as_str() else {
                continue;
            };
            package_aliases(name, parent, &value, &mut aliases);
        } else if path.ends_with("tsconfig.json") || path.ends_with("jsconfig.json") {
            let Some((paths, base)) = compiler_paths(root, path, &value) else {
                continue;
            };
            for (pattern, targets) in paths.iter().take(256) {
                let Some(targets) = targets.as_array() else {
                    continue;
                };
                aliases.push(ModuleAlias {
                    scope: parent.to_string_lossy().replace('\\', "/"),
                    pattern: pattern.clone(),
                    targets: targets
                        .iter()
                        .take(16)
                        .filter_map(|target| normalize(&base.join(target.as_str()?)))
                        .collect(),
                });
            }
        }
        if aliases.len() >= 4096 {
            aliases.truncate(4096);
            break;
        }
    }
    // Nearest project configuration, exact mappings, then the longest
    // pattern win deterministically.
    aliases.sort_by_key(|alias| {
        (
            std::cmp::Reverse(alias.scope.len()),
            alias.pattern.contains('*'),
            std::cmp::Reverse(alias.pattern.len()),
        )
    });
    aliases
}

/// Aliases for a workspace package: its `exports` map (subpaths and
/// conditions), else `module`/`main` for the bare name.
fn package_aliases(name: &str, parent: &Path, value: &Value, aliases: &mut Vec<ModuleAlias>) {
    let mut push = |pattern: String, targets: Vec<String>| {
        let targets: Vec<String> = targets
            .iter()
            .filter_map(|target| normalize(&parent.join(target)))
            .collect();
        if !targets.is_empty() {
            aliases.push(ModuleAlias {
                scope: String::new(),
                pattern,
                targets,
            });
        }
    };
    let exports = &value["exports"];
    let subpaths = exports
        .as_object()
        .filter(|map| map.keys().any(|key| key.starts_with('.')));
    if let Some(map) = subpaths {
        for (subpath, target) in map.iter().take(256) {
            let pattern = match subpath.as_str() {
                "." => name.to_string(),
                other => match other.strip_prefix("./") {
                    Some(rest) => format!("{name}/{rest}"),
                    None => continue,
                },
            };
            push(pattern, export_targets(target, 0));
        }
    } else if !exports.is_null() {
        // A string, array or conditions object describes `.` only.
        push(name.into(), export_targets(exports, 0));
    } else if let Some(entry) = value["module"].as_str().or_else(|| value["main"].as_str()) {
        push(name.into(), vec![entry.into()]);
    }
}

/// Relative targets of one `exports` entry, best condition first. Source
/// conditions come before `types`, which usually names a declaration file.
fn export_targets(value: &Value, depth: usize) -> Vec<String> {
    const ORDER: [&str; 7] = [
        "source", "import", "module", "default", "require", "node", "browser",
    ];
    if depth > 8 {
        return Vec::new();
    }
    match value {
        Value::String(target) if target.starts_with("./") => vec![target.clone()],
        Value::Array(items) => items
            .iter()
            .take(16)
            .flat_map(|item| export_targets(item, depth + 1))
            .collect(),
        Value::Object(map) => {
            let rank = |key: &str| {
                ORDER
                    .iter()
                    .position(|known| *known == key)
                    .unwrap_or(if key == "types" {
                        ORDER.len() + 1
                    } else {
                        ORDER.len()
                    })
            };
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|key| rank(key));
            keys.into_iter()
                .take(16)
                .flat_map(|key| export_targets(&map[key], depth + 1))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// `compilerOptions.paths` and the directory its targets resolve from,
/// following relative `extends` inside the repository. As in TypeScript,
/// targets resolve from the nearest `baseUrl`, else from the config that
/// declared `paths`.
fn compiler_paths(
    root: &Path,
    path: &str,
    value: &Value,
) -> Option<(serde_json::Map<String, Value>, PathBuf)> {
    let mut paths: Option<(serde_json::Map<String, Value>, PathBuf)> = None;
    let mut base_url: Option<PathBuf> = None;
    let mut seen = BTreeSet::new();
    let mut pending = vec![(path.to_string(), value.clone())];
    while let Some((config_path, config)) = pending.pop() {
        if seen.len() >= 16 || !seen.insert(config_path.clone()) {
            continue;
        }
        let dir = Path::new(&config_path)
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let options = &config["compilerOptions"];
        if paths.is_none() {
            if let Some(map) = options["paths"].as_object() {
                paths = Some((map.clone(), dir.clone()));
            }
        }
        if base_url.is_none() {
            if let Some(url) = options["baseUrl"].as_str() {
                base_url = Some(dir.join(url));
            }
        }
        if paths.is_some() && base_url.is_some() {
            break;
        }
        let extends: Vec<&str> = match &config["extends"] {
            Value::String(item) => vec![item.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        // Later `extends` entries override earlier ones; the stack pops them first.
        for item in extends {
            if !item.starts_with("./") && !item.starts_with("../") {
                continue; // Package configs live outside the repository index.
            }
            let item = if item.ends_with(".json") {
                item.to_string()
            } else {
                format!("{item}.json")
            };
            // `normalize` refuses paths that climb above the repository root.
            let Some(next) = normalize(&dir.join(item)) else {
                continue;
            };
            if let Some(parsed) = read_config(root, &next) {
                pending.push((next, parsed));
            }
        }
    }
    let (paths, declared_in) = paths?;
    Some((paths, base_url.unwrap_or(declared_in)))
}

pub(super) fn candidates(aliases: &[ModuleAlias], from: &str, specifier: &str) -> Vec<String> {
    for alias in aliases {
        if !alias.scope.is_empty() && !from.starts_with(&format!("{}/", alias.scope)) {
            continue;
        }
        let wildcard = if let Some((prefix, suffix)) = alias.pattern.split_once('*') {
            specifier
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(suffix))
        } else if alias.pattern == specifier {
            Some("")
        } else {
            None
        };
        if let Some(wildcard) = wildcard {
            return alias
                .targets
                .iter()
                .filter_map(|target| normalize(Path::new(&target.replace('*', wildcard))))
                .collect();
        }
    }
    Vec::new()
}
