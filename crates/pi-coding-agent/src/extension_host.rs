use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::extensions::{discover_extensions, ExtensionManifest};
use crate::js_host::{
    execute_command_tool, execute_js_tool, node_available, render_js_tool_call,
    render_js_tool_result, resolve_extension_module, run_js_extension, run_persistent_js_extension,
    stop_persistent_js_extension, JsAutocompleteProvider, JsExtensionResult, JsRegisteredCommand,
    JsRegisteredProvider, JsRegisteredTool,
};
use crate::native_extensions::{NativeExtensionHost, NATIVE_COMMANDS, NATIVE_TOOLS};
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
    SessionShutdown {
        #[serde(default)]
        reason: String,
    },
    #[serde(rename = "session_before_compact")]
    SessionBeforeCompact,
    #[serde(rename = "session_before_switch")]
    SessionBeforeSwitch,
    #[serde(rename = "session_before_fork")]
    SessionBeforeFork,
    #[serde(rename = "session_before_tree")]
    SessionBeforeTree,
    #[serde(rename = "before_provider_request")]
    BeforeProviderRequest { provider: String, model: String },
    #[serde(rename = "message_start")]
    MessageStart { text: String },
    #[serde(rename = "message_update")]
    MessageUpdate { text: String },
    #[serde(rename = "message_end")]
    MessageEnd { text: String },
    #[serde(rename = "tool_execution_start")]
    ToolExecutionStart {
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        #[serde(rename = "toolName")]
        tool_name: String,
    },
    #[serde(rename = "tool_execution_end")]
    ToolExecutionEnd {
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
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
    Input {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<pi_ai::MessageContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    #[serde(rename = "user_bash")]
    UserBash {
        command: String,
        #[serde(rename = "excludeFromContext")]
        exclude_from_context: bool,
        cwd: String,
    },
    #[serde(rename = "turn_start")]
    TurnStart,
    #[serde(rename = "turn_end")]
    TurnEnd,
    #[serde(rename = "project_trust")]
    ProjectTrust { path: String },
    #[serde(rename = "resources_discover")]
    ResourcesDiscover { cwd: String, reason: String },
    #[serde(rename = "session_info_changed")]
    SessionInfoChanged,
    #[serde(rename = "session_compact")]
    SessionCompact,
    #[serde(rename = "session_compact_failed")]
    SessionCompactFailed { error: String },
    #[serde(rename = "session_tree")]
    SessionTree,
    #[serde(rename = "context")]
    Context,
    #[serde(rename = "before_provider_headers")]
    BeforeProviderHeaders { provider: String, model: String },
    #[serde(rename = "after_provider_response")]
    AfterProviderResponse { provider: String, model: String },
    #[serde(rename = "ui_prompt_start")]
    UiPromptStart { kind: String },
    #[serde(rename = "ui_prompt_end")]
    UiPromptEnd { kind: String },
    #[serde(rename = "model_select")]
    ModelSelect { provider: String, model: String },
    #[serde(rename = "thinking_level_select")]
    ThinkingLevelSelect { level: String },
}

#[derive(Debug, Clone)]
pub struct LoadedJsExtension {
    pub path: String,
    pub handlers: Vec<String>,
    pub tools: Vec<String>,
    pub tool_defs: Vec<JsRegisteredTool>,
    pub commands: Vec<String>,
    pub command_details: Vec<JsRegisteredCommand>,
    pub autocomplete_providers: Vec<JsAutocompleteProvider>,
    pub message_renderers: Vec<String>,
    pub entry_renderers: Vec<String>,
    pub markdown_transformers: u32,
    pub shortcuts: Vec<String>,
    pub has_editor: bool,
    pub providers: Vec<JsRegisteredProvider>,
    pub flags: Vec<String>,
    pub terminal_input: bool,
}

#[derive(Debug, Clone)]
pub struct InputEventResult {
    pub action: String,
    pub text: String,
    pub images: Vec<pi_ai::MessageContent>,
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
    pub runtime_flag_values: Value,
    pub editor_text: String,
    pub runtime_system_prompt: String,
    pub unregistered_providers: Vec<String>,
    pub native: Arc<Mutex<NativeExtensionHost>>,
    before_agent_start_messages: Vec<Value>,
    before_agent_start_system_prompt: Option<String>,
}

impl ExtensionHost {
    pub fn load(agent_dir: &Path, names: &[String]) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        Self::load_with_cwd(agent_dir, names, &cwd)
    }

    pub fn load_with_cwd(agent_dir: &Path, names: &[String], cwd: &Path) -> Self {
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
            runtime_flag_values: Value::Object(serde_json::Map::new()),
            editor_text: String::new(),
            runtime_system_prompt: String::new(),
            unregistered_providers: Vec::new(),
            native: Arc::new(Mutex::new(NativeExtensionHost::new_with_agent_dir(
                "runtime",
                cwd,
                Some(agent_dir),
            ))),
            before_agent_start_messages: Vec::new(),
            before_agent_start_system_prompt: None,
        };
        if node_available() {
            for manifest in &host.manifests {
                let Some(dir) = manifest.path.as_ref().map(Path::new) else {
                    continue;
                };
                let Some(module) = resolve_extension_module(dir) else {
                    continue;
                };
                let themes: Vec<Value> = pi_tui::builtin_themes()
                    .into_iter()
                    .map(|theme| {
                        serde_json::json!({
                            "name": theme.name,
                            "path": serde_json::Value::Null,
                        })
                    })
                    .collect();
                if let Ok(loaded) = run_js_extension(
                    &module,
                    "load",
                    &serde_json::json!({
                        "themes": themes,
                        "theme": "dark",
                        "toolsExpanded": false,
                    }),
                ) {
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
                        host.unregistered_providers
                            .extend(loaded.unregistered_providers.clone());
                        let has_editor = loaded.has_editor
                            || loaded.handlers.iter().any(|name| name == "session_start");
                        if has_editor {
                            host.editor_modules.push(path.clone());
                        }
                        host.js.push(LoadedJsExtension {
                            path,
                            handlers: loaded.handlers,
                            tools: loaded.tools.iter().map(|tool| tool.name.clone()).collect(),
                            tool_defs: loaded.tools,
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
                            flags: loaded.flags,
                            terminal_input: loaded.terminal_input_handlers > 0
                                || loaded.ui_calls.iter().any(|call| {
                                    call.get("op").and_then(Value::as_str)
                                        == Some("onTerminalInput")
                                }),
                        });
                    }
                }
            }
        }
        host.drop_unregistered_providers();
        host
    }

    fn drop_unregistered_providers(&mut self) {
        let names = self.unregistered_providers.clone();
        for ext in &mut self.js {
            ext.providers
                .retain(|provider| !names.iter().any(|name| name == &provider.name));
        }
    }

    pub fn filter_models(&self, models: &mut Vec<pi_ai::Model>) {
        models.retain(|model| {
            !self
                .unregistered_providers
                .iter()
                .any(|name| name == &model.provider)
        });
    }

    pub fn dispatch_terminal_input(&mut self, data: &str) -> bool {
        for ext in &self.js {
            if !ext.terminal_input {
                continue;
            }
            let Ok(result) = run_persistent_js_extension(
                Path::new(&ext.path),
                "terminalInput",
                &serde_json::json!({ "data": data }),
            ) else {
                continue;
            };
            if result
                .result
                .as_ref()
                .and_then(|value| value.get("consumed"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                return true;
            }
        }
        false
    }

    pub fn emit(&mut self, event: ExtensionEvent) {
        self.dispatch_js(&event);
        {
            // Through the poison, like every other site: a panic that once
            // held this lock must not stop shutdown from aborting graph runs.
            let mut native = self.native.lock().unwrap_or_else(|err| err.into_inner());
            match event {
                ExtensionEvent::SessionStart => native.session_start(),
                ExtensionEvent::SessionBeforeCompact | ExtensionEvent::SessionCompact => {
                    native.session_compact()
                }
                ExtensionEvent::SessionShutdown { .. } => native.session_shutdown(),
                _ => {}
            }
        }
        self.events.push(event);
    }

    pub fn native_tool_names(&self) -> Vec<String> {
        self.native
            .lock()
            .map(|native| native.tool_names())
            .unwrap_or_default()
    }

    pub fn native_tool_specs(&self) -> Vec<pi_ai::ToolSpec> {
        NativeExtensionHost::tool_specs()
    }

    /// Graph workers inherit the session's model, thinking level, and trust
    /// decision unless the project pins a per-role model in `.pi/graph.json`.
    pub fn set_graph_session_context(
        &self,
        model: Option<String>,
        thinking: Option<String>,
        project_trusted: bool,
    ) {
        if let Ok(mut native) = self.native.lock() {
            native
                .graph
                .set_session_context(model, thinking, project_trusted);
        }
    }

    /// `state_hash` is computed lazily: only the governor's anti-loop ledger
    /// needs the repository state, and only for search tools.
    pub fn native_before_tool(
        &self,
        name: &str,
        args: &Value,
        state_hash: impl FnOnce() -> String,
    ) -> Option<String> {
        self.native
            .lock()
            .ok()
            .and_then(|mut native| native.before_tool(name, args, state_hash))
    }

    pub fn native_after_tool(
        &self,
        name: &str,
        args: &Value,
        result: pi_agent::ToolResult,
    ) -> pi_agent::ToolResult {
        match self.native.lock() {
            Ok(mut native) => native.after_tool(name, args, result),
            Err(_) => result,
        }
    }

    /// Retrieve a bounded, untrusted memory block for the active prompt.
    /// Retrieval is deliberately best-effort: unavailable local/remote
    /// indexes must never make a normal prompt fail.
    pub fn native_memory_inject(&self, query: &str) -> Option<String> {
        self.native
            .lock()
            .ok()
            .and_then(|native| native.memory_inject(query))
    }

    /// Index the current session snapshot after a settled turn. Errors are
    /// returned to the caller so the runtime can record diagnostics without
    /// surfacing them as provider/tool failures.
    pub fn native_index_messages(
        &self,
        messages: &[crate::native_extensions::MemoryMessage],
    ) -> Result<usize, pi_agent::ToolError> {
        self.native
            .lock()
            .map_err(|err| pi_agent::ToolError::Failed(err.to_string()))?
            .memory_index_messages(messages)
    }

    pub fn execute_native_tool(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Option<Result<pi_agent::ToolResult, pi_agent::ToolError>> {
        let is_worker_submit = name == crate::native_extensions::GRAPH_SUBMIT_TOOL
            && crate::native_extensions::graph_worker_context().is_some();
        if !is_worker_submit && !NATIVE_TOOLS.iter().any(|tool| *tool == name) {
            return None;
        }
        // `graph_run` blocks for the whole run. It must not do so while
        // holding the native host: every other tool call's pre-hook, the
        // status commands and session shutdown (`abort_all_runs`) take the
        // same lock, so the run would freeze the session and Ctrl-C could
        // never stop it. The controller keeps its live state process-wide,
        // so a clone runs the graph just as well.
        if name == "graph_run" {
            let graph = self
                .native
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .graph
                .clone();
            return Some(graph.execute_tool(name, args));
        }
        Some(
            self.native
                .lock()
                .map_err(|err| pi_agent::ToolError::Failed(err.to_string()))
                .and_then(|mut native| native.execute_tool(cwd, name, args)),
        )
    }

    pub fn execute_native_command(&self, name: &str, args: &str) -> Result<Option<Value>, String> {
        if !NATIVE_COMMANDS.iter().any(|command| *command == name) {
            return Ok(None);
        }
        self.native
            .lock()
            .map_err(|err| err.to_string())?
            .command(name, args)
    }

    pub fn emit_before_agent_start(&mut self, prompt: &str, images: &[pi_ai::MessageContent]) {
        self.dispatch_js_with_payload(
            &ExtensionEvent::BeforeAgentStart,
            Some(serde_json::json!({
                "prompt": prompt,
                "images": images,
            })),
        );
        self.events.push(ExtensionEvent::BeforeAgentStart);
    }

    /// TS `emitInput`: transforms chain across extensions; `handled` short-circuits.
    pub fn emit_input(
        &mut self,
        text: &str,
        images: &[pi_ai::MessageContent],
        source: &str,
    ) -> InputEventResult {
        let mut current_text = text.to_string();
        let mut current_images = images.to_vec();
        let mut transformed = false;
        for ext in &self.js {
            let payload = serde_json::json!({
                "type": "input",
                "text": current_text,
                "images": current_images,
                "source": source,
                "flagValues": self.runtime_flag_values,
                "activeTools": self.runtime_active_tools,
                "thinkingLevel": self.runtime_thinking_level,
                "editorText": self.editor_text,
                "systemPrompt": self.runtime_system_prompt,
            });
            if let Ok(result) = run_js_extension(Path::new(&ext.path), "emit", &payload) {
                if result.ok {
                    self.last_js_result = result.result.clone();
                    self.unregistered_providers
                        .extend(result.unregistered_providers.clone());
                    self.ui_calls.extend(result.ui_calls);
                    self.session_calls.extend(result.session_calls);
                    if let Some(value) = &result.result {
                        match value.get("action").and_then(Value::as_str) {
                            Some("handled") => {
                                self.events.push(ExtensionEvent::Input {
                                    text: current_text.clone(),
                                    images: Some(current_images.clone()),
                                    source: Some(source.to_string()),
                                });
                                return InputEventResult {
                                    action: "handled".into(),
                                    text: current_text,
                                    images: current_images,
                                };
                            }
                            Some("transform") => {
                                if let Some(next) = value.get("text").and_then(Value::as_str) {
                                    current_text = next.to_string();
                                    transformed = true;
                                }
                                if let Some(next_images) = value.get("images") {
                                    if let Ok(parsed) =
                                        serde_json::from_value::<Vec<pi_ai::MessageContent>>(
                                            next_images.clone(),
                                        )
                                    {
                                        current_images = parsed;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        self.events.push(ExtensionEvent::Input {
            text: current_text.clone(),
            images: Some(current_images.clone()),
            source: Some(source.to_string()),
        });
        InputEventResult {
            action: if transformed {
                "transform".into()
            } else {
                "continue".into()
            },
            text: current_text,
            images: current_images,
        }
    }

    fn dispatch_js(&mut self, event: &ExtensionEvent) {
        self.dispatch_js_with_payload(event, None);
    }

    fn dispatch_js_with_payload(&mut self, event: &ExtensionEvent, extra: Option<Value>) {
        self.last_js_result = None;
        let Ok(mut payload) = serde_json::to_value(event) else {
            return;
        };
        if let Some(object) = payload.as_object_mut() {
            object.insert("flagValues".into(), self.runtime_flag_values.clone());
            object.insert(
                "activeTools".into(),
                Value::Array(
                    self.runtime_active_tools
                        .iter()
                        .map(|name| Value::String(name.clone()))
                        .collect(),
                ),
            );
            object.insert(
                "thinkingLevel".into(),
                Value::String(self.runtime_thinking_level.clone()),
            );
            object.insert("editorText".into(), Value::String(self.editor_text.clone()));
            object.insert(
                "systemPrompt".into(),
                Value::String(self.runtime_system_prompt.clone()),
            );
            if let Some(extra) = extra.and_then(|value| value.as_object().cloned()) {
                object.extend(extra);
            }
        }
        let before_agent_start = matches!(event, ExtensionEvent::BeforeAgentStart);
        let user_bash = matches!(event, ExtensionEvent::UserBash { .. });
        if before_agent_start {
            self.before_agent_start_messages.clear();
            self.before_agent_start_system_prompt = None;
        }
        let mut current_system_prompt = self.runtime_system_prompt.clone();
        let mut emits = Vec::new();
        for ext in &self.js {
            if before_agent_start {
                if let Some(object) = payload.as_object_mut() {
                    object.insert(
                        "systemPrompt".into(),
                        Value::String(current_system_prompt.clone()),
                    );
                }
            }
            if let Ok(result) = run_js_extension(Path::new(&ext.path), "emit", &payload) {
                if result.ok {
                    let result_value = result.result.clone();
                    if before_agent_start {
                        if let Some(value) = result_value.as_ref() {
                            if let Some(messages) = value.get("messages").and_then(Value::as_array)
                            {
                                self.before_agent_start_messages
                                    .extend(messages.iter().cloned());
                            } else if let Some(message) = value.get("message") {
                                self.before_agent_start_messages.push(message.clone());
                            }
                            if let Some(system_prompt) =
                                value.get("systemPrompt").and_then(Value::as_str)
                            {
                                current_system_prompt = system_prompt.to_string();
                                self.before_agent_start_system_prompt =
                                    Some(current_system_prompt.clone());
                            }
                        }
                    }
                    self.last_js_result = result_value.clone();
                    self.unregistered_providers
                        .extend(result.unregistered_providers.clone());
                    self.ui_calls.extend(result.ui_calls);
                    self.session_calls.extend(result.session_calls);
                    emits.extend(result.event_emits);
                    if user_bash && result_value.as_ref().map(is_js_truthy).unwrap_or(false) {
                        break;
                    }
                }
            }
        }
        self.deliver_event_emits(&emits);
    }

    pub fn deliver_event_emits(&self, emits: &[Value]) {
        for emit in emits {
            let Some(channel) = emit.get("channel").and_then(Value::as_str) else {
                continue;
            };
            if channel.is_empty() {
                continue;
            }
            let data = emit.get("data").cloned().unwrap_or(Value::Null);
            for ext in &self.js {
                let _ = run_js_extension(
                    Path::new(&ext.path),
                    "event",
                    &serde_json::json!({ "channel": channel, "data": data }),
                );
            }
        }
    }

    pub fn js_stream_provider(&self, provider: &str) -> Option<(String, String)> {
        for ext in &self.js {
            for registered in &ext.providers {
                if registered.name == provider && registered.has_stream_simple {
                    return Some((ext.path.clone(), registered.name.clone()));
                }
            }
        }
        None
    }

    pub fn js_refresh_providers(&self) -> Vec<(String, String)> {
        self.js
            .iter()
            .flat_map(|ext| {
                ext.providers
                    .iter()
                    .filter(|registered| registered.has_refresh_models)
                    .map(|registered| (ext.path.clone(), registered.name.clone()))
            })
            .collect()
    }

    pub fn js_oauth_provider(&self, provider: &str) -> Option<(String, String)> {
        for ext in &self.js {
            for registered in &ext.providers {
                if registered.name == provider && registered.has_oauth {
                    return Some((ext.path.clone(), registered.name.clone()));
                }
            }
        }
        None
    }

    pub fn js_oauth_provider_names(&self) -> Vec<String> {
        self.js
            .iter()
            .flat_map(|ext| {
                ext.providers
                    .iter()
                    .filter(|registered| registered.has_oauth)
                    .map(|registered| registered.name.clone())
            })
            .collect()
    }

    pub fn tool_call_blocked(&self) -> bool {
        self.last_js_result
            .as_ref()
            .and_then(|value| value.get("block"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn last_result_cancelled(&self) -> bool {
        self.last_js_result
            .as_ref()
            .and_then(|value| value.get("cancel"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn last_user_bash_result(&self) -> Option<Value> {
        self.last_js_result
            .as_ref()
            .filter(|value| is_js_truthy(value))
            .cloned()
    }

    pub fn last_result_system_prompt(&self) -> Option<String> {
        self.before_agent_start_system_prompt.clone()
    }

    pub fn take_before_agent_start_messages(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.before_agent_start_messages)
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

    pub fn js_tool(&self, name: &str) -> Option<(&Path, &JsRegisteredTool)> {
        for ext in &self.js {
            if let Some(tool) = ext.tool_defs.iter().find(|tool| tool.name == name) {
                return Some((Path::new(&ext.path), tool));
            }
        }
        None
    }

    pub fn render_tool_call_lines(&self, name: &str, args: &Value, width: usize) -> Vec<String> {
        let Some((path, tool)) = self.js_tool(name) else {
            return Vec::new();
        };
        if !tool.has_render_call {
            return Vec::new();
        }
        render_js_tool_call(path, name, args, width)
    }

    pub fn render_tool_result_lines(
        &self,
        name: &str,
        result: &Value,
        width: usize,
    ) -> Vec<String> {
        let Some((path, tool)) = self.js_tool(name) else {
            return Vec::new();
        };
        if !tool.has_render_result {
            return Vec::new();
        }
        render_js_tool_result(path, name, result, width)
    }

    pub fn execute_named_tool(&self, name: &str, cwd: &Path) -> Option<Result<String, String>> {
        // Native tools are executed by Agent's custom-tool executor with the
        // model-supplied arguments. This helper is called from the event
        // notification path, where arguments are intentionally unavailable;
        // invoking a native tool here would run it a second time with an empty
        // object (and could duplicate stateful scans or mutations).
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
        if let Some(result) = self.execute_native_tool(cwd, name, args) {
            return result;
        }
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

    pub fn registered_flags(&self) -> Vec<(String, String)> {
        self.js
            .iter()
            .flat_map(|ext| {
                ext.flags
                    .iter()
                    .map(|name| (name.clone(), ext.path.clone()))
            })
            .collect()
    }

    pub fn registered_providers(&self) -> Vec<JsRegisteredProvider> {
        self.js
            .iter()
            .flat_map(|ext| ext.providers.clone())
            .filter(|provider| {
                !self
                    .unregistered_providers
                    .iter()
                    .any(|name| name == &provider.name)
            })
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
                ExtensionEvent::SessionShutdown { .. } => "session_shutdown",
                ExtensionEvent::SessionBeforeCompact => "session_before_compact",
                ExtensionEvent::SessionBeforeSwitch => "session_before_switch",
                ExtensionEvent::SessionBeforeFork => "session_before_fork",
                ExtensionEvent::SessionBeforeTree => "session_before_tree",
                ExtensionEvent::BeforeProviderRequest { .. } => "before_provider_request",
                ExtensionEvent::MessageStart { .. } => "message_start",
                ExtensionEvent::MessageUpdate { .. } => "message_update",
                ExtensionEvent::MessageEnd { .. } => "message_end",
                ExtensionEvent::ToolExecutionStart { .. } => "tool_execution_start",
                ExtensionEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
                ExtensionEvent::ToolExecutionEnd { .. } => "tool_execution_end",
                ExtensionEvent::ToolCall { .. } => "tool_call",
                ExtensionEvent::ToolResult { .. } => "tool_result",
                ExtensionEvent::Input { .. } => "input",
                ExtensionEvent::UserBash { .. } => "user_bash",
                ExtensionEvent::TurnStart => "turn_start",
                ExtensionEvent::TurnEnd => "turn_end",
                ExtensionEvent::ProjectTrust { .. } => "project_trust",
                ExtensionEvent::ResourcesDiscover { .. } => "resources_discover",
                ExtensionEvent::SessionInfoChanged => "session_info_changed",
                ExtensionEvent::SessionCompact => "session_compact",
                ExtensionEvent::SessionCompactFailed { .. } => "session_compact_failed",
                ExtensionEvent::SessionTree => "session_tree",
                ExtensionEvent::Context => "context",
                ExtensionEvent::BeforeProviderHeaders { .. } => "before_provider_headers",
                ExtensionEvent::AfterProviderResponse { .. } => "after_provider_response",
                ExtensionEvent::UiPromptStart { .. } => "ui_prompt_start",
                ExtensionEvent::UiPromptEnd { .. } => "ui_prompt_end",
                ExtensionEvent::ModelSelect { .. } => "model_select",
                ExtensionEvent::ThinkingLevelSelect { .. } => "thinking_level_select",
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
        self.unregistered_providers
            .extend(result.unregistered_providers.clone());
        self.ui_calls.extend(result.ui_calls.clone());
        self.session_calls.extend(result.session_calls.clone());
        self.drop_unregistered_providers();
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
                "flagValues": self.runtime_flag_values,
            }),
        )?;
        if !result.ok {
            stop_persistent_js_extension();
            return Err(result
                .error
                .unwrap_or_else(|| "Command handler error".into()));
        }
        self.unregistered_providers
            .extend(result.unregistered_providers.clone());
        self.ui_calls.extend(result.ui_calls);
        self.session_calls.extend(result.session_calls);
        self.drop_unregistered_providers();
        self.deliver_event_emits(&result.event_emits);
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
                    "{} handlers={} commands={} flags={} renderers={} entries={} md={} editor={}",
                    ext.path,
                    ext.handlers.join(","),
                    ext.commands.join(","),
                    ext.flags.join(","),
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

fn is_js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().map(|number| number != 0.0).unwrap_or(false),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
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
    use crate::js_host::JsRegisteredProvider;

    #[test]
    fn event_names_match_ts() {
        let mut host = ExtensionHost::default();
        host.emit(ExtensionEvent::BeforeAgentStart);
        host.emit(ExtensionEvent::ToolCall {
            tool_name: "read".into(),
            args: serde_json::json!({}),
        });
        host.emit(ExtensionEvent::SessionStart);
        host.emit(ExtensionEvent::UserBash {
            command: "echo hi".into(),
            exclude_from_context: false,
            cwd: "/tmp".into(),
        });
        host.last_js_result = Some(serde_json::json!({
            "content": "overridden",
            "exitCode": 0
        }));
        assert_eq!(
            host.last_user_bash_result()
                .and_then(|value| value.get("content").cloned()),
            Some(serde_json::json!("overridden"))
        );
        host.emit(ExtensionEvent::TurnStart);
        host.emit(ExtensionEvent::TurnEnd);
        host.emit(ExtensionEvent::ProjectTrust {
            path: "/tmp".into(),
        });
        host.emit(ExtensionEvent::BeforeProviderHeaders {
            provider: "google".into(),
            model: "gemini".into(),
        });
        assert_eq!(
            host.kinds(),
            [
                "before_agent_start",
                "tool_call",
                "session_start",
                "user_bash",
                "turn_start",
                "turn_end",
                "project_trust",
                "before_provider_headers"
            ]
        );
    }

    #[test]
    fn user_bash_without_an_override_does_not_reuse_a_previous_result() {
        let mut host = ExtensionHost {
            last_js_result: Some(serde_json::json!({ "content": "stale", "exitCode": 0 })),
            ..ExtensionHost::default()
        };

        host.emit(ExtensionEvent::UserBash {
            command: "echo current".into(),
            exclude_from_context: false,
            cwd: "/tmp".into(),
        });

        assert_eq!(host.last_user_bash_result(), None);
    }

    #[test]
    fn null_user_bash_result_is_not_an_override() {
        let host = ExtensionHost {
            last_js_result: Some(serde_json::Value::Null),
            ..ExtensionHost::default()
        };

        assert_eq!(host.last_user_bash_result(), None);
    }

    #[test]
    fn event_notification_helper_does_not_execute_native_tools_without_arguments() {
        let host = ExtensionHost::default();
        assert!(host
            .execute_named_tool("memory_search", Path::new("."))
            .is_none());
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

    #[test]
    fn unregister_provider_drops_models_from_catalog() {
        let mut host = ExtensionHost {
            unregistered_providers: vec!["anthropic".into()],
            ..ExtensionHost::default()
        };
        host.js.push(LoadedJsExtension {
            path: "/ext.js".into(),
            handlers: Vec::new(),
            tools: Vec::new(),
            tool_defs: Vec::new(),
            commands: Vec::new(),
            command_details: Vec::new(),
            autocomplete_providers: Vec::new(),
            message_renderers: Vec::new(),
            entry_renderers: Vec::new(),
            markdown_transformers: 0,
            shortcuts: Vec::new(),
            has_editor: false,
            providers: vec![JsRegisteredProvider {
                name: "anthropic".into(),
                ..JsRegisteredProvider::default()
            }],
            flags: Vec::new(),
            terminal_input: false,
        });
        host.drop_unregistered_providers();
        assert!(host.registered_providers().is_empty());
        let mut models = vec![
            pi_ai::Model {
                id: "sonnet".into(),
                name: "Sonnet".into(),
                api: "anthropic-messages".into(),
                provider: "anthropic".into(),
                base_url: None,
                reasoning: false,
                input: vec!["text".into()],
                cost: pi_ai::ModelCost {
                    input: 0.0,
                    output: 0.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                },
                context_window: 1000,
                max_tokens: 128,
                compat: serde_json::Value::Null,
                headers: Default::default(),
                thinking_level_map: Default::default(),
            },
            pi_ai::Model {
                id: "gpt".into(),
                name: "GPT".into(),
                api: "openai-responses".into(),
                provider: "openai".into(),
                base_url: None,
                reasoning: false,
                input: vec!["text".into()],
                cost: pi_ai::ModelCost {
                    input: 0.0,
                    output: 0.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                },
                context_window: 1000,
                max_tokens: 128,
                compat: serde_json::Value::Null,
                headers: Default::default(),
                thinking_level_map: Default::default(),
            },
        ];
        host.filter_models(&mut models);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider, "openai");
    }
}
