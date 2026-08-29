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
    pub providers: Vec<JsRegisteredProvider>,
    #[serde(default)]
    pub commands: Vec<JsRegisteredCommand>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub shortcuts: Vec<String>,
    #[serde(default, rename = "messageRenderers")]
    pub message_renderers: Vec<String>,
    #[serde(default, rename = "entryRenderers")]
    pub entry_renderers: Vec<String>,
    #[serde(default, rename = "markdownTransformers")]
    pub markdown_transformers: u32,
    #[serde(default, rename = "uiCalls")]
    pub ui_calls: Vec<Value>,
    #[serde(default, rename = "hasEditor")]
    pub has_editor: bool,
    #[serde(default, rename = "hasCustom")]
    pub has_custom: bool,
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
    #[serde(default)]
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredProvider {
    pub name: String,
    #[serde(default)]
    pub config: Value,
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

pub fn execute_js_tool(
    module: &Path,
    name: &str,
    args: &Value,
    cwd: &Path,
) -> Result<pi_agent::ToolResult, pi_agent::ToolError> {
    let result = run_js_extension(
        module,
        "tool",
        &serde_json::json!({
            "name": name,
            "args": args,
            "cwd": cwd,
            "toolCallId": "call",
        }),
    )
    .map_err(pi_agent::ToolError::Failed)?;
    if !result.ok {
        return Err(pi_agent::ToolError::Failed(
            result
                .error
                .unwrap_or_else(|| format!("JS tool {name} failed")),
        ));
    }
    let value = result.result.unwrap_or(Value::Null);
    Ok(pi_agent::ToolResult {
        content: value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_error: value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        details: value.get("details").cloned(),
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
    fn registers_and_invokes_shortcut_handler() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerShortcut("ctrl+k", {
    description: "ping",
    handler: () => ({ handled: true }),
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.shortcuts.iter().any(|key| key == "ctrl+k"));
        let invoked =
            run_js_extension(&module, "shortcut", &serde_json::json!({ "key": "ctrl+k" })).unwrap();
        assert_eq!(invoked.result.as_ref().unwrap()["handled"], true);
    }

    #[test]
    fn records_extension_ui_calls() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.ui.setWidget("banner", ["hello widget"], { placement: "aboveEditor" });
  pi.ui.setStatus("job", "running");
  pi.ui.notify("ready", "info");
  pi.registerCommand("ask", {
    description: "ask",
    handler: async (_args, ctx) => {
      const choice = await ctx.ui.select("Pick", ["a", "b"]);
      return { choice };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert!(loaded
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setWidget" && call["key"] == "banner"));
        std::env::set_var("PI_EXTENSION_UI_REPLY", "a");
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "ask", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        std::env::remove_var("PI_EXTENSION_UI_REPLY");
        assert_eq!(command.result.as_ref().unwrap()["choice"], "a");
        assert!(command.ui_calls.iter().any(|call| call["op"] == "select"));
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
    fn registers_and_renders_message_renderer() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerMessageRenderer("status-update", (message, options) => {
    const pad = " ".repeat(options.outputPad || 0);
    return { render: (width) => [`${pad}${message.customType}:${message.content}:${width}`] };
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.message_renderers, ["status-update"]);
        let rendered = run_js_extension(
            &module,
            "renderMessage",
            &serde_json::json!({
                "customType": "status-update",
                "message": {"role":"custom","customType":"status-update","content":"ok"},
                "options": {"expanded": false, "outputPad": 1},
                "width": 40
            }),
        )
        .unwrap();
        let lines = rendered.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert_eq!(lines[0], " status-update:ok:40");
        assert!(run_js_extension(
            &module,
            "renderMessage",
            &serde_json::json!({"customType":"missing"})
        )
        .unwrap()
        .result
        .is_none());
    }

    #[test]
    fn registers_entry_renderer_and_markdown_transformer() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerEntryRenderer("ticket", (entry) => [`entry:${entry.customType}`]);
  pi.registerMarkdownTransformer((markdown) => markdown.replace("TODO", "DONE"));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.entry_renderers, ["ticket"]);
        assert_eq!(loaded.markdown_transformers, 1);
        let entry = run_js_extension(
            &module,
            "renderEntry",
            &serde_json::json!({"customType":"ticket","entry":{"customType":"ticket"}}),
        )
        .unwrap();
        assert_eq!(entry.result.as_ref().unwrap()["lines"][0], "entry:ticket");
        let transformed = run_js_extension(
            &module,
            "transformMarkdown",
            &serde_json::json!({"markdown":"TODO now"}),
        )
        .unwrap();
        assert_eq!(transformed.result.as_ref().unwrap()["markdown"], "DONE now");
    }

    #[test]
    fn runs_manifest_command_tool() {
        let dir = tempdir().unwrap();
        let out = execute_command_tool("printf fixture-ok", dir.path()).unwrap();
        assert_eq!(out, "fixture-ok");
    }

    #[test]
    fn hosts_js_custom_editor_from_session_start() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Editor, matchesKey } = require("@earendil-works/pi-tui");
class ModalEditor extends Editor {
  constructor() {
    super();
    this.mode = "insert";
  }
  handleInput(data) {
    if (matchesKey(data, "escape")) {
      if (this.mode === "insert") { this.mode = "normal"; return; }
      super.handleInput(data);
      return;
    }
    if (this.mode === "normal" && data === "i") { this.mode = "insert"; return; }
    super.handleInput(data);
  }
  render(width) {
    const lines = super.render(width);
    lines.push(this.mode === "normal" ? " NORMAL " : " INSERT ");
    return lines;
  }
}
module.exports = (pi) => {
  pi.on("session_start", (_event, ctx) => {
    ctx.ui.setEditorComponent(() => new ModalEditor());
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert!(loaded.handlers.contains(&"session_start".into()));
        let rendered =
            run_js_extension(&module, "editorRender", &serde_json::json!({ "width": 40 })).unwrap();
        assert_eq!(rendered.result.as_ref().unwrap()["enabled"], true);
        let lines = rendered.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert!(
            lines.iter().any(|line| line.as_str() == Some(" INSERT ")),
            "{lines:?}"
        );
        let after_escape = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({ "data": "\u{1b}", "width": 40 }),
        )
        .unwrap();
        let escape_lines = after_escape.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert!(
            escape_lines
                .iter()
                .any(|line| line.as_str() == Some(" NORMAL ")),
            "{escape_lines:?}"
        );
        let typed = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({
                "data": "h",
                "snapshot": { "text": "", "extra": { "mode": "insert" } },
                "width": 40
            }),
        )
        .unwrap();
        assert_eq!(typed.result.as_ref().unwrap()["text"], "h");
        let submitted = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({
                "data": "\r",
                "snapshot": { "text": "hello", "extra": { "mode": "insert" } },
                "width": 40
            }),
        )
        .unwrap();
        assert_eq!(submitted.result.as_ref().unwrap()["action"], "submit");
        assert_eq!(submitted.result.as_ref().unwrap()["text"], "hello");
    }

    #[test]
    fn hosts_js_custom_overlay_until_done() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("board", {
    description: "board",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((_tui, _theme, _kb, done) => {
        return {
          label: "open",
          handleInput(data) {
            if (data === "q") done("closed");
            else this.label = data;
          },
          render() { return [this.label]; },
        };
      });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let opened = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "board", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(opened.ok, "{:?}", opened.error);
        assert_eq!(opened.result.as_ref().unwrap()["pending"], true);
        assert_eq!(opened.result.as_ref().unwrap()["lines"][0], "open");
        let typed = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "board",
                "data": "x",
                "snapshot": { "text": "", "extra": { "label": "open" } },
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(typed.result.as_ref().unwrap()["pending"], true);
        assert_eq!(typed.result.as_ref().unwrap()["lines"][0], "x");
        let closed = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "board",
                "data": "q",
                "snapshot": typed.result.as_ref().unwrap()["snapshot"],
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(closed.result.as_ref().unwrap(), "closed");
    }

    #[test]
    fn executes_registered_js_tool_and_records_provider() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerProvider("my-proxy", {
    baseUrl: "https://proxy.example.com",
    api: "anthropic-messages",
    models: [{ id: "demo", name: "Demo", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 128 }],
  });
  pi.registerTool({
    name: "ticket",
    description: "lookup",
    execute(_id, args) {
      return { content: [{ type: "text", text: "ticket:" + args.id }], isError: false };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert_eq!(loaded.providers[0].name, "my-proxy");
        let executed = execute_js_tool(
            &module,
            "ticket",
            &serde_json::json!({"id":"42"}),
            dir.path(),
        )
        .unwrap();
        assert_eq!(executed.content, "ticket:42");
        assert!(!executed.is_error);
    }

    #[test]
    fn mounts_js_virtual_tui_container_select_list_and_input() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Container, SelectList, Input, TUI, Text } = require("@earendil-works/pi-tui");
module.exports = (pi) => {
  pi.registerCommand("tools", {
    description: "tools",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((tui, _theme, _kb, done) => {
        const root = new Container();
        const title = new Text("tools");
        const list = new SelectList([
          { value: "read", label: "read", description: "files" },
          { value: "bash", label: "bash" },
        ], 8);
        const input = new Input();
        input.setValue("query");
        list.onSelect = (item) => done(item.value);
        root.addChild(title);
        root.addChild(list);
        root.addChild(input);
        tui.requestRender();
        return {
          extra: { selected: list.getSelectedItem() && list.getSelectedItem().value },
          handleInput(data) { list.handleInput(data); },
          render(width) { return root.render(width); },
        };
      });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let opened = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "tools", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(opened.ok, "{:?}", opened.error);
        let lines = opened.result.as_ref().unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>();
        assert!(lines.iter().any(|line| line.contains("tools")));
        assert!(lines.iter().any(|line| line.contains("read")));
        assert!(lines
            .iter()
            .any(|line| line.contains("> query") || line.contains("query")));
        let selected = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "tools",
                "data": "\r",
                "snapshot": opened.result.as_ref().unwrap()["snapshot"],
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(selected.result.as_ref().unwrap(), "read");
    }
}
