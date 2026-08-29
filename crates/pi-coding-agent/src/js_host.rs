//! JavaScript extension subprocess runner matching
//! `vendor/pi/packages/coding-agent/src/core/extensions/loader.ts` when Node is present.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RUNNER_JS: &str = include_str!("extension_runner.js");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsExtensionResult {
    pub ok: bool,
    #[serde(default)]
    pub handlers: Vec<String>,
    #[serde(default)]
    pub tools: Vec<JsRegisteredTool>,
    #[serde(default)]
    pub commands: Vec<JsRegisteredCommand>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub shortcuts: Vec<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

pub fn find_node() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("PI_NODE") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Some(path);
        }
    }
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

pub fn node_available() -> bool {
    find_node().is_some()
}

fn runner_path() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("pi-extension-runner");
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let path = dir.join("extension_runner.js");
    std::fs::write(&path, RUNNER_JS).map_err(|err| err.to_string())?;
    Ok(path)
}

pub fn resolve_extension_module(dir: &Path) -> Option<PathBuf> {
    // TS `resolveExtensionEntries`: index.ts before index.js, then package.json main.
    for name in [
        "index.ts",
        "index.tsx",
        "index.mts",
        "index.cts",
        "extension.ts",
        "index.js",
        "index.mjs",
        "index.cjs",
        "extension.js",
    ] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let package = dir.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(package) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            if let Some(main) = value.get("main").and_then(Value::as_str) {
                let candidate = dir.join(main);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn run_js_extension(
    module: &Path,
    op: &str,
    payload: &Value,
) -> Result<JsExtensionResult, String> {
    let node =
        find_node().ok_or_else(|| "Node.js is not available for JS extensions".to_string())?;
    let runner = runner_path()?;
    let mut child = Command::new(node)
        .arg(&runner)
        .arg(module)
        .arg(op)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.to_string().as_bytes())
            .map_err(|err| err.to_string())?;
    }
    let output = child.wait_with_output().map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|err| {
        format!(
            "extension runner: {err}: {} {}",
            stdout.trim(),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn execute_command_tool(command: &str, cwd: &Path) -> Result<String, String> {
    let output = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", command])
            .current_dir(cwd)
            .output()
    } else {
        Command::new("sh")
            .args(["-c", command])
            .current_dir(cwd)
            .output()
    }
    .map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_js_factory_when_node_is_present() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.on("tool_call", (event) => ({ block: event.toolName === "bash", reason: "blocked by fixture" }));
  pi.registerTool({ name: "ticket", description: "lookup" });
  pi.registerCommand("tickets", { description: "list tickets" });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert!(loaded.handlers.contains(&"tool_call".into()));
        let emitted = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({"type":"tool_call","toolName":"bash"}),
        )
        .unwrap();
        assert_eq!(emitted.result.as_ref().unwrap()["block"], true);
        assert_eq!(
            emitted.result.as_ref().unwrap()["reason"],
            "blocked by fixture"
        );
    }

    #[test]
    fn loads_typescript_factory_with_virtual_packages() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.ts"),
            r#"
import ai from "@earendil-works/pi-ai";
import { version } from "@mariozechner/pi-agent-core";

export default (pi) => {
  pi.registerTool({ name: "ticket", description: String(ai.version || version || "ok") });
  pi.on("tool_call", (event) => ({ block: event.toolName === "bash" }));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        assert!(module.extension() == Some("ts".as_ref()));
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert!(loaded.tools[0].description.contains("0.84.4"));
        assert!(loaded.handlers.contains(&"tool_call".into()));
    }

    #[test]
    fn runs_manifest_command_tool() {
        let dir = tempdir().unwrap();
        let out = execute_command_tool("printf fixture-ok", dir.path()).unwrap();
        assert_eq!(out, "fixture-ok");
    }
}
