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

#[derive(Debug, Clone, Default)]
pub struct ExtensionInvoke {
    pub result: Value,
    pub ui_calls: Vec<Value>,
}

const EXTENSION_HOST: &str = r#"
const path = process.argv[1];
const kind = process.argv[2];
const name = process.argv[3];
const payload = JSON.parse(process.argv[4] || '{}');
const replies = JSON.parse(process.argv[5] || '{}');
const uiCalls = [];
const ui = {
  select: async (title, options, opts) => {
    uiCalls.push({ method: 'select', title, options, timeout: opts && opts.timeout });
    return replies.select;
  },
  confirm: async (title, message, opts) => {
    uiCalls.push({ method: 'confirm', title, message, timeout: opts && opts.timeout });
    return replies.confirm === true;
  },
  input: async (title, placeholder, opts) => {
    uiCalls.push({ method: 'input', title, placeholder, timeout: opts && opts.timeout });
    return replies.input;
  },
  editor: async (title, prefill) => {
    uiCalls.push({ method: 'editor', title, prefill });
    return replies.editor;
  },
  notify: (message, notifyType) => { uiCalls.push({ method: 'notify', message, notifyType }); },
  setStatus: (statusKey, statusText) => { uiCalls.push({ method: 'setStatus', statusKey, statusText }); },
  setWidget: (widgetKey, widgetLines, options) => {
    uiCalls.push({ method: 'setWidget', widgetKey, widgetLines, widgetPlacement: options && options.placement });
  },
  setTitle: (title) => { uiCalls.push({ method: 'setTitle', title }); },
  setEditorText: (text) => { uiCalls.push({ method: 'set_editor_text', text }); },
  pasteToEditor: (text) => { uiCalls.push({ method: 'set_editor_text', text }); },
};
const ctx = { ui, hasUI: true };
async function load() {
  let mod = require(path);
  if (mod && mod.default) mod = mod.default;
  return mod;
}
async function main() {
  const mod = await load();
  if (kind === 'tool') {
    const fn = (typeof mod === 'function' ? null : (mod[name] || (mod.tools && mod.tools[name]) || mod.execute));
    if (typeof fn !== 'function') {
      console.log(JSON.stringify({ ok: false, error: 'tool not found: ' + name, uiCalls }));
      return;
    }
    const result = await fn(payload, ctx);
    console.log(JSON.stringify({ ok: true, result, uiCalls }));
    return;
  }
  const handlers = {};
  const pi = {
    on(event, handler) { handlers[event] = handler; },
  };
  if (typeof mod === 'function') {
    const maybe = mod(pi);
    if (maybe && typeof maybe.then === 'function') await maybe;
  }
  const handler = handlers[name];
  if (typeof handler !== 'function') {
    console.log(JSON.stringify({ ok: true, result: null, uiCalls }));
    return;
  }
  const result = await handler(payload, ctx);
  console.log(JSON.stringify({ ok: true, result, uiCalls }));
}
main().catch((err) => {
  console.log(JSON.stringify({ ok: false, error: String(err), uiCalls }));
});
"#;

/// Invoke a JS/TS extension tool via Node when present. Fixture-safe: no network.
pub fn invoke_extension_tool(
    extension: &Path,
    tool_name: &str,
    input: &Value,
) -> Result<Value, String> {
    Ok(invoke_extension(extension, "tool", tool_name, input, &json!({}))?.result)
}

pub fn invoke_extension_event(
    extension: &Path,
    event: &str,
    data: &Value,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    invoke_extension(extension, "event", event, data, replies)
}

fn invoke_extension(
    extension: &Path,
    kind: &str,
    name: &str,
    payload: &Value,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    let node = which_node().ok_or_else(|| "node is not installed".to_string())?;
    let output = Command::new(node)
        .arg("-e")
        .arg(EXTENSION_HOST)
        .arg(extension)
        .arg(kind)
        .arg(name)
        .arg(payload.to_string())
        .arg(replies.to_string())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: Value = serde_json::from_str(stdout.lines().last().unwrap_or("{}"))
        .map_err(|e| format!("extension JSON: {e}: {stdout}"))?;
    let ui_calls = value
        .get("uiCalls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(ExtensionInvoke {
        result: value,
        ui_calls,
    })
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
            let ui_ext = ext_dir.join("ui.js");
            fs::write(
                &ui_ext,
                r#"
module.exports = function (pi) {
  pi.on("session_start", async (event, ctx) => {
    ctx.ui.setTitle(event.reason === "new" ? "pi RPC Demo (new session)" : "pi RPC Demo");
    ctx.ui.setWidget("rpc-demo", ["ready"]);
    ctx.ui.setStatus("rpc-demo", "Turns: 0");
  });
};
"#,
            )
            .unwrap();
            let invoked = invoke_extension_event(
                &ui_ext,
                "session_start",
                &json!({"reason": "new"}),
                &json!({}),
            )
            .unwrap();
            assert!(invoked
                .ui_calls
                .iter()
                .any(|c| c["method"] == "setTitle" && c["title"] == "pi RPC Demo (new session)"));
            assert!(invoked
                .ui_calls
                .iter()
                .any(|c| c["method"] == "setStatus" && c["statusKey"] == "rpc-demo"));
        }
    }
}
