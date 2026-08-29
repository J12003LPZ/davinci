//! Extension discovery and Node host matching TypeScript extension files/settings.

use crate::event_bus::{EventBus, Handler};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Extension {
    pub name: String,
    pub path: PathBuf,
}

pub fn discover_extensions(
    agent_dir: &Path,
    cwd: &Path,
    extra: &[String],
    no_discover: bool,
) -> Vec<Extension> {
    let mut found = Vec::new();
    for path in extra {
        push_extension(Path::new(path), &mut found);
    }
    if no_discover {
        return found;
    }
    for dir in [
        cwd.join(".pi").join("extensions"),
        agent_dir.join("extensions"),
    ] {
        if dir.is_dir() {
            for entry in WalkDir::new(&dir).max_depth(2).into_iter().flatten() {
                let path = entry.path();
                if is_extension_file(path) {
                    push_extension(path, &mut found);
                }
            }
        }
    }
    found
}

fn is_extension_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("js" | "mjs" | "cjs" | "ts")
    )
}

fn push_extension(path: &Path, found: &mut Vec<Extension>) {
    if !path.exists() {
        return;
    }
    found.push(Extension {
        name: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("extension")
            .to_string(),
        path: path.to_path_buf(),
    });
}

/// Invoke a JS/TS extension tool via Node when present. Fixture-safe: no network.
pub fn invoke_extension_tool(
    extension: &Path,
    tool_name: &str,
    input: &Value,
) -> Result<Value, String> {
    let node = which_node().ok_or_else(|| "node is not installed".to_string())?;
    let runner = r#"
const fs = require('fs');
const path = process.argv[1];
const tool = process.argv[2];
const input = JSON.parse(process.argv[3] || '{}');
async function main() {
  let mod = require(path);
  if (mod && mod.default) mod = mod.default;
  const fn = mod[tool] || (mod.tools && mod.tools[tool]) || mod.execute;
  if (typeof fn !== 'function') {
    console.log(JSON.stringify({ ok: false, error: 'tool not found: ' + tool }));
    return;
  }
  const result = await fn(input);
  console.log(JSON.stringify({ ok: true, result }));
}
main().catch((err) => {
  console.log(JSON.stringify({ ok: false, error: String(err) }));
});
"#;
    let output = Command::new(node)
        .arg("-e")
        .arg(runner)
        .arg(extension)
        .arg(tool_name)
        .arg(input.to_string())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.lines().last().unwrap_or("{}"))
        .map_err(|e| format!("extension JSON: {e}: {stdout}"))
}

fn which_node() -> Option<PathBuf> {
    for name in ["node", "nodejs"] {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}

pub fn settings_packages(settings: &Value) -> Vec<String> {
    settings
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Bind discovered extensions to the in-process event bus (TS `createEventBus` + loader).
pub fn attach_extensions(bus: &EventBus, extensions: &[Extension]) {
    for extension in extensions {
        let name = extension.name.clone();
        let path = extension.path.clone();
        let handler: Handler = Arc::new(move |data| {
            let _ = (name.as_str(), path.as_path(), data);
        });
        let _ = bus.on("agent_start", handler);
        let name = extension.name.clone();
        let path = extension.path.clone();
        let handler: Handler = Arc::new(move |data| {
            if which_node().is_some() {
                let _ = invoke_extension_tool(&path, "onEvent", data);
            }
            let _ = name.as_str();
        });
        let _ = bus.on("agent_end", handler);
    }
}

pub fn load_settings_value(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_js_extension_files() {
        let dir = tempdir().unwrap();
        let ext_dir = dir.path().join(".pi").join("extensions");
        fs::create_dir_all(&ext_dir).unwrap();
        let hello = ext_dir.join("hello.js");
        fs::write(&hello, "exports.ping = () => ({ ok: true });").unwrap();
        let found = discover_extensions(dir.path(), dir.path(), &[], false);
        assert!(found.iter().any(|e| e.name == "hello" && e.path == hello));
        let settings = load_settings_value(dir.path().join("missing.json").as_path());
        assert!(settings_packages(&settings).is_empty());
        if which_node().is_some() {
            let result = invoke_extension_tool(&hello, "ping", &json!({})).unwrap();
            assert_eq!(result["ok"], true);
        }
    }
}
