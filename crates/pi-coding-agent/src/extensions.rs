//! Extension discovery and Node host matching TypeScript extension files/settings.

use crate::event_bus::{EventBus, Handler};
use pi_agent::{BuiltinTool, ToolError, ToolResult};
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
        path.extension().and_then(|s| s.to_str()),
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
    pub registry: ExtensionRegistry,
}

#[derive(Debug, Clone)]
pub struct RegisteredToolMeta {
    pub name: String,
    pub label: Option<String>,
    pub description: String,
    pub parameters: Value,
    pub path: PathBuf,
    pub has_render_call: bool,
    pub has_render_result: bool,
}

#[derive(Debug, Clone)]
pub struct RegisteredCommandMeta {
    pub name: String,
    pub description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RegisteredProviderMeta {
    pub name: String,
    pub config: Value,
    pub native: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RegisteredShortcutMeta {
    pub shortcut: String,
    pub description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RegisteredFlagMeta {
    pub name: String,
    pub flag_type: String,
    pub default: Option<Value>,
    pub description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    pub tools: Vec<RegisteredToolMeta>,
    pub commands: Vec<RegisteredCommandMeta>,
    pub providers: Vec<RegisteredProviderMeta>,
    pub shortcuts: Vec<RegisteredShortcutMeta>,
    pub flags: Vec<RegisteredFlagMeta>,
}

impl ExtensionRegistry {
    pub fn merge(&mut self, other: ExtensionRegistry) {
        for tool in other.tools {
            self.tools.retain(|t| t.name != tool.name);
            self.tools.push(tool);
        }
        for command in other.commands {
            self.commands.retain(|c| c.name != command.name);
            self.commands.push(command);
        }
        for provider in other.providers {
            self.providers.retain(|p| p.name != provider.name);
            self.providers.push(provider);
        }
        for shortcut in other.shortcuts {
            self.shortcuts.retain(|s| s.shortcut != shortcut.shortcut);
            self.shortcuts.push(shortcut);
        }
        for flag in other.flags {
            self.flags.retain(|f| f.name != flag.name);
            self.flags.push(flag);
        }
    }

    pub fn command(&self, name: &str) -> Option<&RegisteredCommandMeta> {
        self.commands.iter().find(|c| c.name == name)
    }

    pub fn provider(&self, name: &str) -> Option<&RegisteredProviderMeta> {
        self.providers.iter().find(|p| p.name == name)
    }
}

/// Tool registered via TypeScript `pi.registerTool`.
pub struct ExtensionTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub path: PathBuf,
}

impl ExtensionTool {
    pub fn from_meta(meta: &RegisteredToolMeta) -> Self {
        Self {
            name: meta.name.clone(),
            description: if meta.description.is_empty() {
                meta.label.clone().unwrap_or_default()
            } else {
                meta.description.clone()
            },
            parameters: meta.parameters.clone(),
            path: meta.path.clone(),
        }
    }
}

impl BuiltinTool for ExtensionTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    fn execute(&self, input: &Value, _cwd: &Path) -> Result<ToolResult, ToolError> {
        match invoke_registered_tool(&self.path, &self.name, input) {
            Ok(result) => Ok(tool_result_from_value(&result)),
            Err(err) => Ok(ToolResult {
                output: err,
                is_error: true,
                details: json!({}),
            }),
        }
    }
}

fn tool_result_from_value(result: &Value) -> ToolResult {
    if result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return ToolResult {
            output: result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("extension tool failed")
                .to_string(),
            is_error: true,
            details: result.clone(),
        };
    }
    let body = result.get("result").unwrap_or(result);
    let is_error = body
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output = if let Some(text) = body.as_str() {
        text.to_string()
    } else if let Some(text) = body.get("output").and_then(|v| v.as_str()) {
        text.to_string()
    } else if let Some(text) = body.get("content").and_then(|v| v.as_str()) {
        text.to_string()
    } else if let Some(parts) = body.get("content").and_then(|v| v.as_array()) {
        parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("")
    } else {
        body.to_string()
    };
    ToolResult {
        output,
        is_error,
        details: body.get("details").cloned().unwrap_or_else(|| body.clone()),
    }
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
function jsonSafe(value) {
  try { return JSON.parse(JSON.stringify(value)); } catch (err) { return {}; }
}
const registrations = { tools: [], commands: [], providers: [], shortcuts: [], flags: [] };
const liveTools = {};
const liveCommands = {};
const liveShortcuts = {};
const handlers = {};
const ctx = {
  ui,
  hasUI: true,
  waitForIdle: async () => {},
  newSession: async () => ({ cancelled: false }),
  fork: async () => ({ cancelled: false }),
  navigateTree: async () => ({ cancelled: false }),
  switchSession: async () => ({ cancelled: false }),
  reload: async () => {},
  getSystemPromptOptions: () => ({}),
  modelRegistry: {
    refresh: async () => ({ aborted: false, errors: new Map() }),
    getProviderAuth: async () => null,
  },
};
async function load() {
  let mod = require(path);
  if (mod && mod.default) mod = mod.default;
  return mod;
}
function buildPi() {
  return {
    on(event, handler) { handlers[event] = handler; },
    registerTool(tool) {
      if (!tool || !tool.name) throw new Error('Tool name must not be empty.');
      registrations.tools.push({
        name: tool.name,
        label: tool.label,
        description: tool.description || '',
        parameters: jsonSafe(tool.parameters),
        promptSnippet: tool.promptSnippet,
        promptGuidelines: tool.promptGuidelines,
        hasRenderCall: typeof tool.renderCall === 'function',
        hasRenderResult: typeof tool.renderResult === 'function',
      });
      liveTools[tool.name] = tool;
    },
    registerCommand(cmdName, options) {
      registrations.commands.push({
        name: cmdName,
        description: options && options.description,
      });
      liveCommands[cmdName] = options || {};
    },
    registerProvider(providerOrName, config) {
      if (typeof providerOrName === 'string') {
        if (!providerOrName.trim()) throw new Error('Provider id must not be empty.');
        if (!config) throw new Error('Provider config is required when registering by name');
        registrations.providers = registrations.providers.filter((p) => p.name !== providerOrName);
        registrations.providers.push({ name: providerOrName, config: jsonSafe(config), native: false });
        return;
      }
      const id = (providerOrName && (providerOrName.id || providerOrName.name)) || '';
      if (!String(id).trim()) throw new Error('Provider id must not be empty.');
      registrations.providers = registrations.providers.filter((p) => p.name !== id);
      registrations.providers.push({ name: id, config: jsonSafe(providerOrName), native: true });
    },
    unregisterProvider(providerName) {
      registrations.providers = registrations.providers.filter((p) => p.name !== providerName);
    },
    registerShortcut(shortcut, options) {
      const key = String(shortcut || '').toLowerCase();
      registrations.shortcuts.push({
        shortcut: key,
        description: options && options.description,
      });
      liveShortcuts[key] = options || {};
    },
    registerFlag(flagName, options) {
      if (options && options.default !== undefined && typeof options.default !== options.type) {
        throw new Error('Invalid default for flag "' + flagName + '": expected ' + options.type + ', got ' + typeof options.default);
      }
      registrations.flags.push({
        name: flagName,
        type: options && options.type,
        default: options && options.default,
        description: options && options.description,
      });
    },
    getFlag(flagName) {
      const flag = registrations.flags.find((f) => f.name === flagName);
      if (!flag) return undefined;
      if (payload.flags && payload.flags[flagName] !== undefined) return payload.flags[flagName];
      return flag.default;
    },
    registerMessageRenderer() {},
    registerMarkdownTransformer() {},
    registerEntryRenderer() {},
    sendMessage() {},
    sendUserMessage() {},
    appendEntry() {},
    setSessionName() {},
    getSessionName() { return payload.sessionName; },
    setLabel() {},
    exec() { return { stdout: '', stderr: '', code: 0 }; },
    getActiveTools() { return payload.activeTools || []; },
    getAllTools() { return payload.allTools || []; },
    setActiveTools() {},
    getCommands() { return registrations.commands.slice(); },
    setModel() { return Promise.resolve(); },
    getThinkingLevel() { return payload.thinkingLevel || 'off'; },
    setThinkingLevel() {},
    events: { emit() {}, on() { return function unsubscribe() {}; } },
  };
}
async function main() {
  const mod = await load();
  const pi = buildPi();
  if (typeof mod === 'function') {
    const maybe = mod(pi);
    if (maybe && typeof maybe.then === 'function') await maybe;
  } else if (kind === 'tool' && !liveTools[name]) {
    const fn = mod[name] || (mod.tools && mod.tools[name]) || mod.execute;
    if (typeof fn !== 'function') {
      console.log(JSON.stringify({ ok: false, error: 'tool not found: ' + name, uiCalls, registrations }));
      return;
    }
    const result = await fn(payload, ctx);
    console.log(JSON.stringify({ ok: true, result, uiCalls, registrations }));
    return;
  }
  if (kind === 'load') {
    console.log(JSON.stringify({ ok: true, result: null, uiCalls, registrations }));
    return;
  }
  if (kind === 'tool' || kind === 'registered-tool') {
    const tool = liveTools[name];
    if (tool && typeof tool.execute === 'function') {
      const result = await tool.execute(payload.toolCallId || 'call', payload.params || payload, undefined, undefined, ctx);
      console.log(JSON.stringify({ ok: true, result, uiCalls, registrations }));
      return;
    }
    console.log(JSON.stringify({ ok: false, error: 'tool not found: ' + name, uiCalls, registrations }));
    return;
  }
  if (kind === 'render_call' || kind === 'render_result') {
    const tool = liveTools[name];
    if (!tool) {
      console.log(JSON.stringify({ ok: false, error: 'tool not found: ' + name, uiCalls, registrations }));
      return;
    }
    const colors = payload.themeColors || {};
    const ESC = String.fromCharCode(27);
    const hexToRgb = (hex) => {
      const h = String(hex || '#d4d4d4').replace('#', '');
      return {
        r: parseInt(h.slice(0, 2), 16) || 0,
        g: parseInt(h.slice(2, 4), 16) || 0,
        b: parseInt(h.slice(4, 6), 16) || 0,
      };
    };
    const theme = {
      fg(color, text) {
        const { r, g, b } = hexToRgb(colors[color]);
        return ESC + '[38;2;' + r + ';' + g + ';' + b + 'm' + text + ESC + '[39m';
      },
      bg(_color, text) { return text; },
      bold(text) { return ESC + '[1m' + text + ESC + '[22m'; },
      italic(text) { return ESC + '[3m' + text + ESC + '[23m'; },
      underline(text) { return ESC + '[4m' + text + ESC + '[24m'; },
    };
    const context = {
      args: payload.args,
      toolCallId: payload.toolCallId || name,
      invalidate() {},
      lastComponent: undefined,
      state: {},
      cwd: payload.cwd || '.',
      executionStarted: true,
      argsComplete: true,
      isPartial: kind === 'render_call',
      expanded: !!(payload.options && payload.options.expanded),
      showImages: false,
      isError: !!payload.isError,
    };
    let component;
    if (kind === 'render_call') {
      if (typeof tool.renderCall !== 'function') {
        console.log(JSON.stringify({ ok: true, result: { lines: null }, uiCalls, registrations }));
        return;
      }
      component = tool.renderCall(payload.args, theme, context);
    } else {
      if (typeof tool.renderResult !== 'function') {
        console.log(JSON.stringify({ ok: true, result: { lines: null }, uiCalls, registrations }));
        return;
      }
      component = tool.renderResult(payload.result, payload.options || { expanded: false, isPartial: false }, theme, context);
    }
    const lines = component && typeof component.render === 'function'
      ? component.render(payload.width || 100)
      : null;
    console.log(JSON.stringify({ ok: true, result: { lines }, uiCalls, registrations }));
    return;
  }
  if (kind === 'shortcut') {
    const shortcut = liveShortcuts[String(name || '').toLowerCase()];
    if (!shortcut || typeof shortcut.handler !== 'function') {
      console.log(JSON.stringify({ ok: false, error: 'shortcut not found: ' + name, uiCalls, registrations }));
      return;
    }
    const result = await shortcut.handler(ctx);
    console.log(JSON.stringify({ ok: true, result, uiCalls, registrations }));
    return;
  }
  if (kind === 'command') {
    const command = liveCommands[name];
    if (!command || typeof command.handler !== 'function') {
      console.log(JSON.stringify({ ok: false, error: 'command not found: ' + name, uiCalls, registrations }));
      return;
    }
    const result = await command.handler(typeof payload.args === 'string' ? payload.args : '', ctx);
    console.log(JSON.stringify({ ok: true, result, uiCalls, registrations }));
    return;
  }
  const handler = handlers[name];
  if (typeof handler !== 'function') {
    console.log(JSON.stringify({ ok: true, result: null, uiCalls, registrations }));
    return;
  }
  const result = await handler(payload, ctx);
  console.log(JSON.stringify({ ok: true, result, uiCalls, registrations }));
}
main().catch((err) => {
  console.log(JSON.stringify({ ok: false, error: String(err), uiCalls, registrations }));
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

pub fn invoke_registered_tool(
    extension: &Path,
    tool_name: &str,
    input: &Value,
) -> Result<Value, String> {
    Ok(invoke_extension(extension, "registered-tool", tool_name, input, &json!({}))?.result)
}

pub fn invoke_extension_event(
    extension: &Path,
    event: &str,
    data: &Value,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    invoke_extension(extension, "event", event, data, replies)
}

pub fn invoke_extension_command(
    extension: &Path,
    command: &str,
    args: &str,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    invoke_extension_command_with_flags(extension, command, args, &json!({}), replies)
}

pub fn invoke_extension_command_with_flags(
    extension: &Path,
    command: &str,
    args: &str,
    flags: &Value,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    invoke_extension(
        extension,
        "command",
        command,
        &json!({"args": args, "flags": flags}),
        replies,
    )
}

/// TypeScript reserved editor-global shortcuts that extensions may not override.
pub const RESERVED_SHORTCUTS: &[&str] = &[
    "ctrl+c", "ctrl+p", "ctrl+t", "escape", "enter", "ctrl+d", "ctrl+z",
];

pub fn is_reserved_shortcut(shortcut: &str) -> bool {
    RESERVED_SHORTCUTS.contains(&shortcut.to_ascii_lowercase().as_str())
}

pub fn invoke_extension_shortcut(
    extension: &Path,
    shortcut: &str,
    flags: &Value,
    replies: &Value,
) -> Result<ExtensionInvoke, String> {
    invoke_extension(
        extension,
        "shortcut",
        shortcut,
        &json!({"flags": flags}),
        replies,
    )
}

/// Invoke TypeScript `renderCall` / `renderResult` and return TUI lines.
pub fn invoke_extension_render(
    extension: &Path,
    kind: &str,
    tool_name: &str,
    payload: &Value,
) -> Result<Option<Vec<String>>, String> {
    let invoked = invoke_extension(extension, kind, tool_name, payload, &json!({}))?;
    if invoked.result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Ok(None);
    }
    let lines = invoked
        .result
        .get("result")
        .and_then(|v| v.get("lines"))
        .or_else(|| invoked.result.get("lines"));
    match lines {
        Some(Value::Array(items)) => Ok(Some(
            items
                .iter()
                .map(|item| item.as_str().unwrap_or("").to_string())
                .collect(),
        )),
        _ => Ok(None),
    }
}

pub fn load_extension(path: &Path) -> Result<ExtensionRegistry, String> {
    let invoked = invoke_extension(path, "load", "", &json!({}), &json!({}))?;
    if invoked.result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Err(invoked
            .result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("extension failed to load")
            .to_string());
    }
    Ok(invoked.registry)
}

pub fn load_extensions(extensions: &[Extension]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::default();
    for extension in extensions {
        if let Ok(loaded) = load_extension(&extension.path) {
            registry.merge(loaded);
        }
    }
    registry
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
        registry: parse_registry(&value, extension),
        result: value,
        ui_calls,
    })
}

fn parse_registry(value: &Value, path: &Path) -> ExtensionRegistry {
    let raw = value.get("registrations").cloned().unwrap_or(json!({}));
    let tools = raw
        .get("tools")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            Some(RegisteredToolMeta {
                name: tool.get("name")?.as_str()?.to_string(),
                label: tool
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                description: tool
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parameters: tool.get("parameters").cloned().unwrap_or(json!({})),
                path: path.to_path_buf(),
                has_render_call: tool
                    .get("hasRenderCall")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                has_render_result: tool
                    .get("hasRenderResult")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect();
    let commands = raw
        .get("commands")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|command| {
            Some(RegisteredCommandMeta {
                name: command.get("name")?.as_str()?.to_string(),
                description: command
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                path: path.to_path_buf(),
            })
        })
        .collect();
    let providers = raw
        .get("providers")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|provider| {
            Some(RegisteredProviderMeta {
                name: provider.get("name")?.as_str()?.to_string(),
                config: provider.get("config").cloned().unwrap_or(json!({})),
                native: provider
                    .get("native")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                path: path.to_path_buf(),
            })
        })
        .collect();
    let shortcuts = raw
        .get("shortcuts")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .map(|shortcut| RegisteredShortcutMeta {
            shortcut: shortcut
                .get("shortcut")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: shortcut
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            path: path.to_path_buf(),
        })
        .collect();
    let flags = raw
        .get("flags")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|flag| {
            Some(RegisteredFlagMeta {
                name: flag.get("name")?.as_str()?.to_string(),
                flag_type: flag
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("boolean")
                    .to_string(),
                default: flag.get("default").cloned(),
                description: flag
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                path: path.to_path_buf(),
            })
        })
        .collect();
    ExtensionRegistry {
        tools,
        commands,
        providers,
        shortcuts,
        flags,
    }
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

/// TypeScript `resolveConfigValue`: `$ENV` / `${ENV}` / `$$` / `$!` / leading `!command`.
pub fn resolve_config_value(config: &str) -> Option<String> {
    if let Some(command) = config.strip_prefix('!') {
        return run_config_command(command);
    }
    resolve_template(config)
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn resolve_template(config: &str) -> Option<String> {
    let chars: Vec<char> = config.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '$' {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let next = chars.get(index + 1).copied();
        if next == Some('$') || next == Some('!') {
            out.push(next.unwrap_or('$'));
            index += 2;
            continue;
        }
        if next == Some('{') {
            if let Some(rel_end) = chars[index + 2..].iter().position(|&c| c == '}') {
                let name: String = chars[index + 2..index + 2 + rel_end].iter().collect();
                if is_env_name(&name) {
                    out.push_str(&std::env::var(&name).ok()?);
                    index += 3 + rel_end;
                    continue;
                }
                out.push_str(
                    &chars[index..=index + 2 + rel_end]
                        .iter()
                        .collect::<String>(),
                );
                index += 3 + rel_end;
                continue;
            }
            out.push('$');
            index += 1;
            continue;
        }
        let rest: String = chars[index + 1..].iter().collect();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && is_env_name(&name) {
            out.push_str(&std::env::var(&name).ok()?);
            index += 1 + name.chars().count();
            continue;
        }
        out.push('$');
        index += 1;
    }
    Some(out)
}

fn run_config_command(command: &str) -> Option<String> {
    let output = Command::new("bash").arg("-lc").arg(command).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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

    #[test]
    fn loads_register_provider_tool_and_command() {
        if which_node().is_none() {
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("plugin.js");
        fs::write(
            &path,
            r#"
module.exports = function (pi) {
  pi.registerProvider("my-proxy", {
    name: "My Proxy",
    baseUrl: "https://proxy.example",
    apiKey: "$PROXY_API_KEY",
    api: "openai-completions",
    authHeader: true,
    models: [{
      id: "proxy-sm",
      name: "Proxy SM",
      reasoning: false,
      input: ["text"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 8192,
      maxTokens: 1024
    }]
  });
  pi.registerProvider({ id: "native-ai", name: "Native", models: [] });
  pi.unregisterProvider("native-ai");
  pi.registerTool({
    name: "echo_tool",
    label: "Echo",
    description: "Echo arguments",
    parameters: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
    execute: async (_id, params) => ({ content: [{ type: "text", text: "echo:" + params.text }] }),
    renderCall(args) {
      return { render: () => ["echo " + (args && args.text ? args.text : "")] };
    },
    renderResult(result) {
      const text = result && result.content && result.content[0] ? result.content[0].text : "";
      return { render: () => ["", text, ""] };
    }
  });
  pi.registerCommand("echo-cmd", {
    description: "Echo a slash argument",
    handler: async (args, ctx) => {
      ctx.ui.setTitle("cmd:" + args);
      ctx.ui.notify("ran " + args, "info");
    }
  });
  pi.registerShortcut("ctrl+l", {
    description: "Llama overlay",
    handler: async (ctx) => { ctx.ui.setTitle("short:" + String(pi.getFlag("verbose"))); }
  });
  pi.registerFlag("verbose", { type: "boolean", default: false, description: "Verbose" });
};
"#,
        )
        .unwrap();
        let registry = load_extension(&path).unwrap();
        assert!(registry.provider("my-proxy").is_some());
        assert!(registry.provider("native-ai").is_none());
        assert_eq!(
            registry.provider("my-proxy").unwrap().config["baseUrl"],
            "https://proxy.example"
        );
        assert_eq!(registry.tools[0].name, "echo_tool");
        assert!(registry.tools[0].has_render_call);
        assert!(registry.tools[0].has_render_result);
        let call_lines = invoke_extension_render(
            &path,
            "render_call",
            "echo_tool",
            &json!({"args": {"text": "hi"}, "width": 100}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(call_lines, vec!["echo hi".to_string()]);
        let result_lines = invoke_extension_render(
            &path,
            "render_result",
            "echo_tool",
            &json!({
                "result": {"content": [{"type":"text","text":"echo:hi"}]},
                "options": {"expanded": true, "isPartial": false},
                "width": 100
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            result_lines,
            vec!["".to_string(), "echo:hi".into(), "".into()]
        );
        assert_eq!(registry.commands[0].name, "echo-cmd");
        assert_eq!(registry.shortcuts[0].shortcut, "ctrl+l");
        assert_eq!(registry.flags[0].name, "verbose");

        let tool = invoke_registered_tool(&path, "echo_tool", &json!({"text": "hi"})).unwrap();
        assert_eq!(tool["ok"], true);
        assert_eq!(tool["result"]["content"][0]["text"], "echo:hi");
        let converted = tool_result_from_value(&tool);
        assert_eq!(converted.output, "echo:hi");
        assert!(!converted.is_error);

        let invoked = invoke_extension_command(&path, "echo-cmd", "hello", &json!({})).unwrap();
        assert!(invoked
            .ui_calls
            .iter()
            .any(|c| c["method"] == "setTitle" && c["title"] == "cmd:hello"));
        let short =
            invoke_extension_shortcut(&path, "ctrl+l", &json!({"verbose": true}), &json!({}))
                .unwrap();
        assert!(short
            .ui_calls
            .iter()
            .any(|c| c["method"] == "setTitle" && c["title"] == "short:true"));
        assert!(is_reserved_shortcut("ctrl+c"));
        assert!(!is_reserved_shortcut("ctrl+l"));
    }

    #[test]
    fn register_provider_by_name_requires_config() {
        if which_node().is_none() {
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.js");
        fs::write(
            &path,
            r#"
module.exports = function (pi) {
  pi.registerProvider("broken");
};
"#,
        )
        .unwrap();
        let err = load_extension(&path).unwrap_err();
        assert!(err.contains("Provider config is required when registering by name"));
    }

    #[test]
    fn register_flag_rejects_wrong_default_type() {
        if which_node().is_none() {
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("flag.js");
        fs::write(
            &path,
            r#"
module.exports = function (pi) {
  pi.registerFlag("count", { type: "string", default: 1 });
};
"#,
        )
        .unwrap();
        let err = load_extension(&path).unwrap_err();
        assert!(err.contains("Invalid default for flag \"count\": expected string, got number"));
    }

    #[test]
    fn resolve_config_value_matches_typescript() {
        std::env::set_var("PI_TEST_PROXY_KEY", "secret-key");
        assert_eq!(
            resolve_config_value("$PI_TEST_PROXY_KEY").as_deref(),
            Some("secret-key")
        );
        assert_eq!(
            resolve_config_value("pre-${PI_TEST_PROXY_KEY}-post").as_deref(),
            Some("pre-secret-key-post")
        );
        assert_eq!(
            resolve_config_value("$$literal").as_deref(),
            Some("$literal")
        );
        assert_eq!(resolve_config_value("$!bang").as_deref(), Some("!bang"));
        assert_eq!(resolve_config_value("$MISSING_PI_ENV_VAR_XYZ"), None);
        assert_eq!(
            resolve_config_value("!printf cmd-secret").as_deref(),
            Some("cmd-secret")
        );
        std::env::remove_var("PI_TEST_PROXY_KEY");
    }
}
