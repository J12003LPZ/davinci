//! Conservative local module hints, never a package manager or filesystem resolver.
use super::scanner;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path};

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

pub(super) fn collect(root: &Path, paths: &[String]) -> Vec<ModuleAlias> {
    let mut aliases = Vec::new();
    for path in paths.iter().take(2000) {
        let Ok(body) = scanner::read_bounded(root, Path::new(path), 128 * 1024) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        let parent = Path::new(path).parent().unwrap_or(Path::new(""));
        if path.ends_with("package.json") {
            let Some(name) = value["name"].as_str() else {
                continue;
            };
            let entry = value["exports"]
                .as_str()
                .or_else(|| value["exports"]["."].as_str())
                .or_else(|| value["module"].as_str())
                .or_else(|| value["main"].as_str());
            if let Some(target) = entry.and_then(|entry| normalize(&parent.join(entry))) {
                aliases.push(ModuleAlias {
                    scope: String::new(),
                    pattern: name.into(),
                    targets: vec![target],
                });
            }
        } else if path.ends_with("tsconfig.json") || path.ends_with("jsconfig.json") {
            let options = &value["compilerOptions"];
            let Some(paths) = options["paths"].as_object() else {
                continue;
            };
            let base = parent.join(options["baseUrl"].as_str().unwrap_or("."));
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
    // Nearest project configuration and exact mappings win deterministically.
    aliases.sort_by_key(|alias| {
        (
            std::cmp::Reverse(alias.scope.len()),
            alias.pattern.contains('*'),
        )
    });
    aliases
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
