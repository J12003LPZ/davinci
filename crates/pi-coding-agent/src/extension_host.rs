use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::extensions::{discover_extensions, ExtensionManifest};
use crate::js_host::{
    execute_command_tool, execute_js_tool, node_available, resolve_extension_module,
    run_js_extension, run_persistent_js_extension, stop_persistent_js_extension,
    JsAutocompleteProvider, JsExtensionResult, JsRegisteredCommand, JsRegisteredProvider,
};
use pi_tui::Keybindings;

/// Extension event bus matching `vendor/pi/packages/coding-agent/src/core/extensions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExtensionEvent {
    #[serde(rename = "before_agent_start")]
    BeforeAgentStart,
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "agent_end")]
    AgentEnd,
    #[serde(rename = "agent_settled")]
    AgentSettled,
    #[serde(rename = "session_start")]
    SessionStart,
    #[serde(rename = "session_shutdown")]
    SessionShutdown,
    #[serde(rename = "session_before_compact")]
    SessionBeforeCompact,
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    #[serde(rename = "input")]
    Input { text: String },
}

#[derive(Debug, Clone)]
pub struct LoadedJsExtension {
    pub path: String,
    pub handlers: Vec<String>,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
    pub command_details: Vec<JsRegisteredCommand>,
    pub autocomplete_providers: Vec<JsAutocompleteProvider>,
    pub message_renderers: Vec<String>,
    pub entry_renderers: Vec<String>,
    pub markdown_transformers: u32,
    pub shortcuts: Vec<String>,
    pub has_editor: bool,
    pub providers: Vec<JsRegisteredProvider>,
}

#[derive(Debug, Default, Clone)]
pub struct ExtensionHost {
    pub events: Vec<ExtensionEvent>,
    pub manifests: Vec<ExtensionManifest>,
    pub js: Vec<LoadedJsExtension>,
    pub last_js_result: Option<Value>,
    pub message_renderers: std::collections::HashMap<String, String>,
    pub entry_renderers: std::collections::HashMap<String, String>,
    pub markdown_modules: Vec<String>,
    pub ui_calls: Vec<Value>,
    pub session_calls: Vec<Value>,
    pub editor_modules: Vec<String>,
    pub runtime_active_tools: Vec<String>,
    pub runtime_all_tools: Vec<String>,
    pub runtime_thinking_level: String,
    pub runtime_commands: Vec<Value>,
}

impl ExtensionHost {
    pub fn load(agent_dir: &Path, names: &[String]) -> Self {
        let manifests = discover_extensions(agent_dir, names);
        let mut host = Self {
            events: Vec::new(),
            manifests,
            js: Vec::new(),
            last_js_result: None,
            message_renderers: std::collections::HashMap::new(),
            entry_renderers: std::collections::HashMap::new(),
            markdown_modules: Vec::new(),
            ui_calls: Vec::new(),
            session_calls: Vec::new(),
            editor_modules: Vec::new(),
            runtime_active_tools: Vec::new(),
            runtime_all_tools: Vec::new(),
            runtime_thinking_level: "off".into(),
            runtime_commands: Vec::new(),
        };
        if node_available() {
            for manifest in &host.manifests {
                let Some(dir) = manifest.path.as_ref().map(Path::new) else {
                    continue;
                };
                let Some(module) = resolve_extension_module(dir) else {
                    continue;
                };
                if let Ok(loaded) = run_js_extension(&module, "load", &serde_json::json!({})) {
                    if loaded.ok {
                        let path = module.display().to_string();
                        for custom_type in &loaded.message_renderers {
                            host.message_renderers
                                .insert(custom_type.clone(), path.clone());
                        }
                        for custom_type in &loaded.entry_renderers {
                            host.entry_renderers
                                .insert(custom_type.clone(), path.clone());
                        }
                        if loaded.markdown_transformers > 0 {
                            host.markdown_modules.push(path.clone());
                        }
                        host.ui_calls.extend(loaded.ui_calls.clone());
                        host.session_calls.extend(loaded.session_calls.clone());
                        let has_editor = loaded.has_editor
                            || loaded.handlers.iter().any(|name| name == "session_start");
                        if has_editor {
                            host.editor_modules.push(path.clone());
                        }
                        host.js.push(LoadedJsExtension {
                            path,
                            handlers: loaded.handlers,
                            tools: loaded.tools.into_iter().map(|tool| tool.name).collect(),
                            commands: loaded
                                .commands
                                .iter()
                                .map(|command| command.name.clone())
                                .collect(),
                            command_details: loaded.commands,
                            autocomplete_providers: loaded.autocomplete_providers,
                            message_renderers: loaded.message_renderers,
                            entry_renderers: loaded.entry_renderers,
                            markdown_transformers: loaded.markdown_transformers,
                            shortcuts: loaded.shortcuts,
                            has_editor,
                            providers: loaded.providers,
                        });
                    }
                }
            }
        }
        host
    }

    pub fn emit(&mut self, event: ExtensionEvent) {
        self.dispatch_js(&event);
        self.events.push(event);
    }

    fn dispatch_js(&mut self, event: &ExtensionEvent) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        for ext in &self.js {
            if let Ok(result) = run_js_extension(Path::new(&ext.path), "emit", &payload) {
                if result.ok {
                    self.last_js_result = result.result;
                    self.ui_calls.extend(result.ui_calls);
                    self.session_calls.extend(result.session_calls);
                }
            }
        }
    }

    pub fn tool_call_blocked(&self) -> bool {
        self.last_js_result
            .as_ref()
            .and_then(|value| value.get("block"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn get_message_renderer(&self, custom_type: &str) -> Option<&str> {
        self.message_renderers.get(custom_type).map(String::as_str)
    }

    pub fn render_custom_message(
        &self,
        custom_type: &str,
        content: &str,
        expanded: bool,
        output_pad: usize,
        width: usize,
    ) -> Option<Vec<String>> {
        let module = self.message_renderers.get(custom_type)?;
        let rendered = run_js_extension(
            Path::new(module),
            "renderMessage",
            &serde_json::json!({
                "customType": custom_type,
                "content": content,
                "message": {
                    "role": "custom",
                    "customType": custom_type,
                    "content": content,
                    "display": true
                },
                "options": { "expanded": expanded, "outputPad": output_pad },
                "width": width
            }),
        )
        .ok()?;
        rendered.result?.get("lines")?.as_array().map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect()
        })
    }

    pub fn get_entry_renderer(&self, custom_type: &str) -> Option<&str> {
        self.entry_renderers.get(custom_type).map(String::as_str)
    }

    pub fn render_custom_entry(
        &self,
        custom_type: &str,
        data: &Value,
        expanded: bool,
        width: usize,
    ) -> Option<Vec<String>> {
        let module = self.entry_renderers.get(custom_type)?;
        let rendered = run_js_extension(
            Path::new(module),
            "renderEntry",
            &serde_json::json!({
                "customType": custom_type,
                "entry": {
                    "type": "custom",
                    "customType": custom_type,
                    "data": data
                },
                "options": { "expanded": expanded },
                "width": width
            }),
        )
        .ok()?;
        rendered.result?.get("lines")?.as_array().map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect()
        })
    }

    pub fn transform_markdown(
        &self,
        markdown: &str,
        message_type: &str,
        is_streaming: bool,
        width: usize,
    ) -> String {
        let mut text = markdown.to_string();
        for module in &self.markdown_modules {
            if let Ok(result) = run_js_extension(
                Path::new(module),
                "transformMarkdown",
                &serde_json::json!({
                    "markdown": text,
                    "context": {
                        "messageType": message_type,
                        "isStreaming": is_streaming,
                        "availableWidth": width
                    },
                    "width": width
                }),
            ) {
                if let Some(next) = result
                    .result
                    .as_ref()
                    .and_then(|value| value.get("markdown"))
                    .and_then(Value::as_str)
                {
                    text = next.to_string();
                }
            }
        }
        text
    }

    pub fn execute_named_tool(&self, name: &str, cwd: &Path) -> Option<Result<String, String>> {
        for ext in &self.js {
            if ext.tools.iter().any(|tool| tool == name) {
                return Some(
                    execute_js_tool(
                        Path::new(&ext.path),
                        name,
                        &Value::Object(Default::default()),
                        cwd,
                    )
                    .map(|result| result.content)
                    .map_err(|err| err.to_string()),
                );
            }
        }
        for manifest in &self.manifests {
            for tool in &manifest.tools {
                if tool.name == name {
                    if let Some(command) = &tool.command {
                        return Some(execute_command_tool(command, cwd));
                    }
                }
            }
        }
        None
    }

    pub fn execute_js_or_manifest_tool(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<pi_agent::ToolResult, pi_agent::ToolError> {
        for ext in &self.js {
            if ext.tools.iter().any(|tool| tool == name) {
                return execute_js_tool(Path::new(&ext.path), name, args, cwd);
            }
        }
        for manifest in &self.manifests {
            for tool in &manifest.tools {
                if tool.name == name {
                    if let Some(command) = &tool.command {
                        return execute_command_tool(command, cwd)
                            .map(|content| pi_agent::ToolResult {
                                content,
                                is_error: false,
                                details: None,
                            })
                            .map_err(pi_agent::ToolError::Failed);
                    }
                }
            }
        }
        Err(pi_agent::ToolError::Unknown(name.to_string()))
    }

    pub fn registered_providers(&self) -> Vec<JsRegisteredProvider> {
        self.js
            .iter()
            .flat_map(|ext| ext.providers.clone())
            .collect()
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.events
            .iter()
            .map(|event| match event {
                ExtensionEvent::BeforeAgentStart => "before_agent_start",
                ExtensionEvent::AgentStart => "agent_start",
                ExtensionEvent::AgentEnd => "agent_end",
                ExtensionEvent::AgentSettled => "agent_settled",
                ExtensionEvent::SessionStart => "session_start",
                ExtensionEvent::SessionShutdown => "session_shutdown",
                ExtensionEvent::SessionBeforeCompact => "session_before_compact",
                ExtensionEvent::ToolCall { .. } => "tool_call",
                ExtensionEvent::ToolResult { .. } => "tool_result",
                ExtensionEvent::Input { .. } => "input",
            })
            .collect()
    }

    pub fn js_summary(result: &JsExtensionResult) -> String {
        format!(
            "handlers={} tools={}",
            result.handlers.join(","),
            result
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn registered_shortcuts(&self) -> Vec<(String, Vec<String>)> {
        self.js
            .iter()
            .map(|ext| (ext.path.clone(), ext.shortcuts.clone()))
            .collect()
    }

    pub fn resolve_shortcuts(
        &self,
        keybindings: &Keybindings,
    ) -> (Vec<(String, String)>, Vec<String>) {
        resolve_extension_shortcuts(&self.registered_shortcuts(), keybindings)
    }

    pub fn invoke_shortcut(&mut self, path: &str, key: &str) -> Result<Option<Value>, String> {
        let result = run_js_extension(
            Path::new(path),
            "shortcut",
            &serde_json::json!({ "key": key }),
        )?;
        if !result.ok {
            return Err(result
                .error
                .unwrap_or_else(|| "Shortcut handler error".into()));
        }
        self.ui_calls.extend(result.ui_calls.clone());
        self.session_calls.extend(result.session_calls.clone());
        Ok(result.result)
    }

    pub fn invoke_command(&mut self, path: &str, name: &str) -> Result<Option<Value>, String> {
        self.invoke_command_with(path, name, "", None, 80)
    }

    pub fn invoke_command_with(
        &mut self,
        path: &str,
        name: &str,
        data: &str,
        snapshot: Option<&Value>,
        width: usize,
    ) -> Result<Option<Value>, String> {
        let result = run_persistent_js_extension(
            Path::new(path),
            if data.is_empty() {
                "command"
            } else {
                "customInput"
            },
            &serde_json::json!({
                "name": name,
                "data": data,
                "snapshot": snapshot,
                "width": width,
                "height": 24,
                "ctx": { "mode": "tui" },
                "activeTools": self.runtime_active_tools,
                "allTools": self.runtime_all_tools,
                "thinkingLevel": self.runtime_thinking_level,
                "commands": self.runtime_commands,
            }),
        )?;
        if !result.ok {
            stop_persistent_js_extension();
            return Err(result
                .error
                .unwrap_or_else(|| "Command handler error".into()));
        }
        self.ui_calls.extend(result.ui_calls);
        self.session_calls.extend(result.session_calls);
        if result
            .result
            .as_ref()
            .and_then(|value| value.get("pending"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            stop_persistent_js_extension();
        }
        Ok(result.result)
    }

    pub fn invoke_custom_tick(
        &mut self,
        path: &str,
        name: &str,
        snapshot: Option<&Value>,
        width: usize,
    ) -> Result<Option<Value>, String> {
        let result = run_persistent_js_extension(
            Path::new(path),
            "customTick",
            &serde_json::json!({
                "name": name,
                "snapshot": snapshot,
                "width": width,
                "height": 24,
                "ctx": { "mode": "tui" }
            }),
        )?;
        if !result.ok {
            crate::js_host::stop_persistent_js_extension();
            return Err(result
                .error
                .unwrap_or_else(|| "Command handler error".into()));
        }
        self.ui_calls.extend(result.ui_calls);
        self.session_calls.extend(result.session_calls);
        if result
            .result
            .as_ref()
            .and_then(|value| value.get("pending"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            crate::js_host::stop_persistent_js_extension();
        }
        Ok(result.result)
    }

    pub fn editor_input(
        &self,
        path: &str,
        data: &str,
        snapshot: Option<&Value>,
        width: usize,
    ) -> Result<Value, String> {
        let result = run_js_extension(
            Path::new(path),
            if data.is_empty() {
                "editorRender"
            } else {
                "editorInput"
            },
            &serde_json::json!({
                "data": data,
                "snapshot": snapshot,
                "width": width
            }),
        )?;
        if !result.ok {
            return Err(result.error.unwrap_or_else(|| "Editor host error".into()));
        }
        result
            .result
            .ok_or_else(|| "Editor host returned no result".into())
    }

    pub fn describe_js(&self) -> String {
        self.js
            .iter()
            .map(|ext| {
                format!(
                    "{} handlers={} commands={} renderers={} entries={} md={} editor={}",
                    ext.path,
                    ext.handlers.join(","),
                    ext.commands.join(","),
                    ext.message_renderers.join(","),
                    ext.entry_renderers.join(","),
                    ext.markdown_transformers,
                    ext.has_editor
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

const RESERVED_KEYBINDINGS_FOR_EXTENSION_CONFLICTS: &[&str] = &[
    "app.interrupt",
    "app.clear",
    "app.exit",
    "app.suspend",
    "app.thinking.cycle",
    "app.model.cycleForward",
    "app.model.cycleBackward",
    "app.model.select",
    "app.tools.expand",
    "app.thinking.toggle",
    "app.editor.external",
    "app.message.copy",
    "app.message.followUp",
    "tui.input.submit",
    "tui.select.confirm",
    "tui.select.cancel",
    "tui.input.copy",
    "tui.editor.deleteToLineEnd",
];

fn resolve_extension_shortcuts(
    registered: &[(String, Vec<String>)],
    keybindings: &Keybindings,
) -> (Vec<(String, String)>, Vec<String>) {
    let mut builtin: std::collections::HashMap<String, (String, bool)> =
        std::collections::HashMap::new();
    for (action, keys) in keybindings.bindings() {
        let restrict = RESERVED_KEYBINDINGS_FOR_EXTENSION_CONFLICTS.contains(&action);
        for key in keys {
            let normalized = key.to_ascii_lowercase();
            if let Some((_, existing_restrict)) = builtin.get(&normalized) {
                if *existing_restrict && !restrict {
                    continue;
                }
            }
            builtin.insert(normalized, (action.to_string(), restrict));
        }
    }
    let mut resolved: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut diagnostics = Vec::new();
    for (path, keys) in registered {
        for key in keys {
            let normalized = key.to_ascii_lowercase();
            if let Some((action, restrict)) = builtin.get(&normalized) {
                if *restrict {
                    diagnostics.push(format!(
                        "Extension shortcut '{key}' from {path} conflicts with built-in shortcut. Skipping."
                    ));
                    continue;
                }
                diagnostics.push(format!(
                    "Extension shortcut conflict: '{key}' is built-in shortcut for {action} and {path}. Using {path}."
                ));
            }
            if let Some(existing) = resolved.get(&normalized) {
                diagnostics.push(format!(
                    "Extension shortcut conflict: '{key}' registered by both {existing} and {path}. Using {path}."
                ));
            }
            resolved.insert(normalized, path.clone());
        }
    }
    let shortcuts = resolved.into_iter().collect();
    (shortcuts, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_ts() {
        let mut host = ExtensionHost::default();
        host.emit(ExtensionEvent::BeforeAgentStart);
        host.emit(ExtensionEvent::ToolCall {
            tool_name: "read".into(),
            args: serde_json::json!({}),
        });
        host.emit(ExtensionEvent::SessionStart);
        assert_eq!(
            host.kinds(),
            ["before_agent_start", "tool_call", "session_start"]
        );
    }

    #[test]
    fn extension_shortcuts_skip_reserved_and_last_wins() {
        let bindings = Keybindings::defaults();
        let (shortcuts, diagnostics) = resolve_extension_shortcuts(
            &[
                (
                    "/one.js".into(),
                    vec!["ctrl+c".into(), "ctrl+k".into(), "ctrl+shift+x".into()],
                ),
                (
                    "/two.js".into(),
                    vec!["ctrl+shift+x".into(), "ctrl+y".into()],
                ),
            ],
            &bindings,
        );
        assert!(diagnostics
            .iter()
            .any(|line| line.contains("conflicts with built-in shortcut")));
        assert!(diagnostics.iter().any(|line| {
            line.contains("ctrl+k") && line.contains("conflicts with built-in shortcut")
        }));
        assert!(diagnostics
            .iter()
            .any(|line| line.contains("registered by both")));
        assert!(!shortcuts.iter().any(|(key, _)| key == "ctrl+c"));
        assert!(!shortcuts.iter().any(|(key, _)| key == "ctrl+k"));
        assert_eq!(
            shortcuts
                .iter()
                .find(|(key, _)| key == "ctrl+shift+x")
                .map(|item| item.1.as_str()),
            Some("/two.js")
        );
        assert!(shortcuts
            .iter()
            .any(|(key, path)| key == "ctrl+y" && path == "/two.js"));
    }
}
