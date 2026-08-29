//! Shared AgentSession runtime used by print, JSON, RPC, and interactive.

use crate::args::FlagValue;
use crate::event_bus::EventBus;
use crate::export;
use crate::extension_ui::ExtensionUiHost;
use crate::extensions::{self, Extension, ExtensionRegistry, ExtensionTool};
use crate::slash;
use pi_agent::{
    compact_messages, run_agent, AgentConfig, AgentEvent, AgentMessage, AllowAllPermissionPolicy,
    FollowUpQueue, QueueMode, SteerQueue, ThinkingLevel, ToolRegistry,
};
use pi_ai::catalog::{resolve_model, Model, ModelCost};
use pi_ai::list_models;
use pi_session::{
    append_entry, append_session_name, clone_session, create_session, default_sessions_root,
    fork_from_entry, fork_messages, last_assistant_text, leaf_id, now_ms, read_entries,
    session_stats, session_tree, SessionInfo,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct SessionRuntime {
    pub cwd: PathBuf,
    pub provider: String,
    pub model_id: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub steer: SteerQueue,
    pub follow_up: FollowUpQueue,
    pub thinking: ThinkingLevel,
    pub tools: ToolRegistry,
    pub session_path: Option<PathBuf>,
    pub session_id: String,
    pub session_name: Option<String>,
    pub scoped_models: Vec<String>,
    pub auto_compact: bool,
    pub auto_retry: bool,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub aborted: bool,
    pub api_key: Option<String>,
    pub allow_network: bool,
    pub fixture: Option<pi_ai::stream::FixtureResponse>,
    pub bus: EventBus,
    pub max_turns: u32,
    pub context_window: usize,
    pub ui: ExtensionUiHost,
    pub extensions: Vec<Extension>,
    pub registry: ExtensionRegistry,
    pub theme: String,
    pub flag_values: BTreeMap<String, Value>,
    pub pending_custom_lines: Vec<(String, String)>,
    pub pending_next_turn: Vec<Value>,
    pub pending_custom_messages: Vec<Value>,
    pub pending_trigger_turn: bool,
    pub running_turn: bool,
    pub last_extension_turn_events: Vec<AgentEvent>,
}

impl SessionRuntime {
    pub fn new_session(&mut self, parent: Option<&str>) -> Result<Vec<Value>, String> {
        let dest = create_session(&default_sessions_root(), &self.cwd.to_string_lossy(), None)
            .map_err(|e| e.to_string())?;
        self.session_id = dest.id.clone();
        self.session_path = Some(dest.path);
        self.session_name = None;
        self.messages.clear();
        self.steer.clear();
        self.follow_up.clear();
        let _ = self.ui.reset();
        let events = self.fire_extensions(
            "session_start",
            &json!({"reason": if parent.is_some() { "parent" } else { "new" }}),
        );
        if let Some(parent) = parent {
            if let Some(path) = &self.session_path {
                let _ = append_entry(
                    path,
                    &json!({"type":"custom","parentSession": parent, "timestamp": now_ms()}),
                );
            }
        }
        Ok(events)
    }

    pub fn bind_extensions(&mut self) {
        self.registry = extensions::load_extensions(&self.extensions);
        self.registry
            .shortcuts
            .retain(|shortcut| !extensions::is_reserved_shortcut(&shortcut.shortcut));
        for tool in &self.registry.tools {
            self.tools
                .register(Box::new(ExtensionTool::from_meta(tool)));
        }
        for flag in &self.registry.flags {
            if !self.flag_values.contains_key(&flag.name) {
                if let Some(default) = &flag.default {
                    self.flag_values.insert(flag.name.clone(), default.clone());
                }
            }
        }
    }

    pub fn apply_cli_flags(
        &mut self,
        flags: &BTreeMap<String, FlagValue>,
    ) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let mut unknown = Vec::new();
        for (name, value) in flags {
            let Some(flag) = self.registry.flags.iter().find(|f| f.name == *name) else {
                unknown.push(name.clone());
                continue;
            };
            match flag.flag_type.as_str() {
                "string" => match value {
                    FlagValue::String(text) => {
                        self.flag_values.insert(name.clone(), json!(text));
                    }
                    FlagValue::Bool(_) => {
                        errors.push(format!("Extension flag \"--{name}\" requires a value"));
                    }
                },
                _ => {
                    self.flag_values.insert(name.clone(), json!(true));
                }
            }
        }
        if !unknown.is_empty() {
            errors.push(format!(
                "Unknown option{}: {}",
                if unknown.len() == 1 { "" } else { "s" },
                unknown
                    .iter()
                    .map(|name| format!("--{name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn flags_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (name, value) in &self.flag_values {
            map.insert(name.clone(), value.clone());
        }
        Value::Object(map)
    }

    pub fn invoke_shortcut(&mut self, shortcut: &str) -> Result<Vec<Value>, String> {
        let normalized = shortcut.to_ascii_lowercase();
        if extensions::is_reserved_shortcut(&normalized) {
            return Err(format!(
                "Extension shortcut '{normalized}' from built-in conflicts with built-in shortcut. Skipping."
            ));
        }
        let path = self
            .registry
            .shortcuts
            .iter()
            .find(|s| s.shortcut == normalized)
            .ok_or_else(|| format!("shortcut not found: {normalized}"))?
            .path
            .clone();
        let invoked = extensions::invoke_extension_shortcut(
            &path,
            &normalized,
            &self.flags_json(),
            &json!({}),
        )?;
        self.apply_extension_actions(&invoked.ui_calls);
        Ok(self.ui.apply_calls(&invoked.ui_calls))
    }

    pub fn config(&self) -> AgentConfig {
        let request = self.provider_request();
        AgentConfig {
            cwd: self.cwd.clone(),
            system_prompt: self.system_prompt.clone(),
            model_provider: self.provider.clone(),
            model_id: self.model_id.clone(),
            api_key: request.api_key,
            allow_network: self.allow_network,
            auto_retry: self.auto_retry,
            max_retries: 2,
            auto_compact: self.auto_compact,
            context_window: self.context_window,
            max_turns: self.max_turns,
            fixture: self.fixture.clone(),
            permission: Box::new(AllowAllPermissionPolicy),
            transport: if self.provider == "openai-codex" {
                Some(pi_ai::Transport::Auto)
            } else {
                None
            },
            session_id: Some(self.session_id.clone()),
            base_url: request.base_url,
            extra_headers: request.extra_headers,
            api: request.api,
        }
    }

    fn provider_request(&self) -> ProviderRequest {
        let mut api_key = self.api_key.clone();
        let mut base_url = None;
        let mut extra_headers = Vec::new();
        let mut api = None;
        if let Some(provider) = self.registry.provider(&self.provider) {
            if let Some(url) = provider
                .config
                .get("baseUrl")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                base_url = Some(url.to_string());
            }
            if let Some(kind) = provider.config.get("api").and_then(|v| v.as_str()) {
                api = Some(kind.to_string());
            }
            if let Some(key) = provider.config.get("apiKey").and_then(|v| v.as_str()) {
                if let Some(resolved) = extensions::resolve_config_value(key) {
                    api_key = Some(resolved);
                }
            }
            if let Some(headers) = provider.config.get("headers").and_then(|v| v.as_object()) {
                for (name, value) in headers {
                    if let Some(raw) = value.as_str() {
                        extra_headers.push((
                            name.clone(),
                            extensions::resolve_config_value(raw)
                                .unwrap_or_else(|| raw.to_string()),
                        ));
                    }
                }
            }
            if provider
                .config
                .get("authHeader")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if let Some(key) = &api_key {
                    if !extra_headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    {
                        extra_headers.push(("authorization".into(), format!("Bearer {key}")));
                    }
                }
            }
            if let Some(model) = models_from_provider(provider)
                .into_iter()
                .find(|m| m.id == self.model_id)
            {
                if model.base_url.is_some() {
                    base_url = model.base_url;
                }
                if !model.api.is_empty() {
                    api = Some(model.api);
                }
            }
        }
        ProviderRequest {
            api_key,
            base_url,
            extra_headers,
            api,
        }
    }

    pub fn invoke_registered_command(
        &mut self,
        name: &str,
        args: &str,
    ) -> Result<Vec<Value>, String> {
        let path = self
            .registry
            .command(name)
            .ok_or_else(|| format!("Unknown command: {name}"))?
            .path
            .clone();
        let invoked = if self.flag_values.is_empty() {
            extensions::invoke_extension_command(&path, name, args, &json!({}))?
        } else {
            extensions::invoke_extension_command_with_flags(
                &path,
                name,
                args,
                &self.flags_json(),
                &json!({}),
            )?
        };
        self.apply_extension_actions(&invoked.ui_calls);
        Ok(self.ui.apply_calls(&invoked.ui_calls))
    }

    pub fn prompt(&mut self, text: &str, images: Vec<Value>) -> Result<Vec<AgentEvent>, String> {
        if let Some((cmd, args)) = slash::parse_slash(text) {
            if self.registry.command(cmd).is_some() {
                let _ = self.invoke_registered_command(cmd, args)?;
                return Ok(vec![]);
            }
        }
        if self.aborted {
            self.aborted = false;
        }
        self.flush_pending_next_turn();
        self.messages.push(AgentMessage {
            role: "user".into(),
            content: text.to_string(),
            images,
        });
        self.append_message("user", text);
        self.run_turn()
    }

    pub fn run_turn(&mut self) -> Result<Vec<AgentEvent>, String> {
        self.running_turn = true;
        self.bus.emit(
            "agent_start",
            json!({"provider": self.provider, "model": self.model_id}),
        );
        self.is_streaming = true;
        let _ = self.fire_extensions(
            "turn_start",
            &json!({"provider": self.provider, "model": self.model_id}),
        );
        let config = self.config();
        let events = run_agent(
            &config,
            &self.messages,
            &self.tools,
            &mut self.steer,
            &mut self.follow_up,
        )
        .map_err(|e| e.to_string())?;
        for event in &events {
            if let AgentEvent::Message { message } = event {
                if message.role == "assistant" {
                    self.messages.push(message.clone());
                    self.append_message("assistant", &message.content);
                }
            }
        }
        self.is_streaming = false;
        self.flush_pending_custom_messages();
        self.bus.emit("agent_end", json!({"ok": true}));
        let _ = self.fire_extensions("turn_end", &json!({"ok": true}));
        self.running_turn = false;
        let mut events = events;
        if self.pending_trigger_turn {
            self.pending_trigger_turn = false;
            if let Ok(more) = self.run_turn() {
                events.extend(more);
            }
        }
        Ok(events)
    }

    pub fn fire_extensions(&mut self, event: &str, data: &Value) -> Vec<Value> {
        let paths: Vec<_> = self.extensions.iter().map(|e| e.path.clone()).collect();
        let mut payload = data.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("flags".into(), self.flags_json());
        }
        let mut events = Vec::new();
        for path in paths {
            if let Ok(invoked) =
                extensions::invoke_extension_event(&path, event, &payload, &json!({}))
            {
                self.apply_extension_actions(&invoked.ui_calls);
                events.extend(self.ui.apply_calls(&invoked.ui_calls));
            }
        }
        events
    }

    fn apply_extension_actions(&mut self, calls: &[Value]) {
        for call in calls {
            match call.get("method").and_then(|v| v.as_str()) {
                Some("appendEntry") => {
                    let custom_type = call
                        .get("customType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let data = call.get("data").cloned().unwrap_or(json!({}));
                    self.append_custom_entry(custom_type, data.clone());
                    let entry = json!({
                        "type": "custom",
                        "customType": custom_type,
                        "data": data,
                    });
                    if let Some(lines) = self.render_custom_entry(&entry, 80) {
                        for line in lines {
                            self.pending_custom_lines.push(("custom".into(), line));
                        }
                    }
                }
                Some("sendMessage") => {
                    if let Some(message) = call.get("message") {
                        self.send_custom_message(
                            message,
                            call.get("options").unwrap_or(&json!({})),
                        );
                    }
                }
                Some("sendUserMessage") => {
                    self.send_user_message_action(
                        call.get("content").unwrap_or(&json!("")),
                        call.get("options").unwrap_or(&json!({})),
                    );
                }
                _ => {}
            }
        }
    }

    pub fn append_custom_entry(&self, custom_type: &str, data: Value) {
        self.append_typed(
            "custom",
            json!({
                "customType": custom_type,
                "data": data,
                "id": uuid::Uuid::new_v4().to_string(),
            }),
        );
    }

    pub fn append_custom_message(&self, message: &Value) {
        let custom_type = message
            .get("customType")
            .and_then(|v| v.as_str())
            .unwrap_or("custom");
        let display = message
            .get("display")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        self.append_typed(
            "custom_message",
            json!({
                "customType": custom_type,
                "content": message.get("content").cloned().unwrap_or(json!("")),
                "display": display,
                "details": message.get("details").cloned().unwrap_or(json!({})),
                "id": uuid::Uuid::new_v4().to_string(),
            }),
        );
    }

    fn custom_text(message: &Value) -> String {
        match message.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }

    fn custom_agent_message(message: &Value) -> AgentMessage {
        AgentMessage {
            role: "custom".into(),
            content: Self::custom_text(message),
            images: vec![],
        }
    }

    fn commit_custom_message(&mut self, message: &Value) {
        self.append_custom_message(message);
        self.messages.push(Self::custom_agent_message(message));
        if message.get("display").and_then(|v| v.as_bool()) != Some(false) {
            for line in self.render_custom_message(message, 80) {
                self.pending_custom_lines.push(("custom".into(), line));
            }
        }
    }

    /// TypeScript `sendCustomMessage` triggerTurn / deliverAs.
    pub fn send_custom_message(&mut self, message: &Value, options: &Value) {
        let deliver = options.get("deliverAs").and_then(|v| v.as_str());
        let trigger = options.get("triggerTurn").and_then(|v| v.as_bool());
        if deliver == Some("nextTurn") {
            self.pending_next_turn.push(message.clone());
            return;
        }
        if self.is_streaming && trigger != Some(false) {
            let agent_msg = Self::custom_agent_message(message);
            if deliver == Some("followUp") {
                self.follow_up.enqueue(agent_msg);
            } else {
                self.steer.enqueue(agent_msg);
            }
            self.pending_custom_messages.push(message.clone());
            return;
        }
        if trigger == Some(true) {
            self.commit_custom_message(message);
            self.trigger_agent_turn();
            return;
        }
        if self.is_streaming {
            self.pending_custom_messages.push(message.clone());
            return;
        }
        self.commit_custom_message(message);
    }

    fn send_user_message_action(&mut self, content: &Value, options: &Value) {
        let text = match content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        let images = match content {
            Value::Array(parts) => parts
                .iter()
                .filter(|part| part.get("type").and_then(|v| v.as_str()) == Some("image"))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let deliver = options.get("deliverAs").and_then(|v| v.as_str());
        if self.is_streaming {
            let msg = AgentMessage {
                role: "user".into(),
                content: text,
                images,
            };
            if deliver == Some("followUp") {
                self.follow_up.enqueue(msg);
            } else {
                self.steer.enqueue(msg);
            }
            return;
        }
        if self.running_turn {
            self.messages.push(AgentMessage {
                role: "user".into(),
                content: text.clone(),
                images,
            });
            self.append_message("user", &text);
            self.pending_trigger_turn = true;
            return;
        }
        let _ = self.prompt(&text, images);
    }

    fn trigger_agent_turn(&mut self) {
        if self.running_turn || self.is_streaming {
            self.pending_trigger_turn = true;
            return;
        }
        if let Ok(events) = self.run_turn() {
            self.last_extension_turn_events.extend(events);
        }
    }

    fn flush_pending_next_turn(&mut self) {
        let pending = std::mem::take(&mut self.pending_next_turn);
        for message in pending {
            self.commit_custom_message(&message);
        }
    }

    fn flush_pending_custom_messages(&mut self) {
        let pending = std::mem::take(&mut self.pending_custom_messages);
        for message in pending {
            self.commit_custom_message(&message);
        }
    }

    pub fn take_extension_turn_events(&mut self) -> Vec<AgentEvent> {
        std::mem::take(&mut self.last_extension_turn_events)
    }

    pub fn transform_markdown(
        &self,
        markdown: &str,
        message_type: &str,
        is_streaming: bool,
        available_width: usize,
    ) -> String {
        let mut text = markdown.to_string();
        for path in &self.registry.markdown_transformers {
            if let Ok(next) = extensions::invoke_transform_markdown(
                path,
                &text,
                message_type,
                is_streaming,
                available_width,
            ) {
                text = next;
            }
        }
        text
    }

    pub fn render_custom_message(&self, message: &Value, width: usize) -> Vec<String> {
        let custom_type = message
            .get("customType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(renderer) = self
            .registry
            .message_renderers
            .iter()
            .find(|r| r.custom_type == custom_type)
        {
            if let Ok(Some(lines)) = extensions::invoke_message_renderer(
                &renderer.path,
                custom_type,
                message,
                false,
                width,
            ) {
                return lines;
            }
        }
        let content = match message.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        vec![format!("[{custom_type}]"), content]
    }

    pub fn take_custom_lines(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.pending_custom_lines)
    }

    pub fn render_custom_entry(&self, entry: &Value, width: usize) -> Option<Vec<String>> {
        let custom_type = entry
            .get("customType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let renderer = self
            .registry
            .entry_renderers
            .iter()
            .find(|r| r.custom_type == custom_type)?;
        extensions::invoke_entry_renderer(&renderer.path, custom_type, entry, false, width).ok()?
    }

    fn append_message(&self, role: &str, content: &str) {
        if let Some(path) = &self.session_path {
            let _ = append_entry(
                path,
                &json!({
                    "type": "message",
                    "id": uuid::Uuid::new_v4().to_string(),
                    "role": role,
                    "content": content,
                    "timestamp": now_ms(),
                }),
            );
        }
    }

    pub fn append_typed(&self, ty: &str, extra: Value) {
        if let Some(path) = &self.session_path {
            let mut entry = extra;
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("type".into(), json!(ty));
                obj.entry("timestamp").or_insert(json!(now_ms()));
            }
            let _ = append_entry(path, &entry);
        }
    }

    pub fn set_model(&mut self, provider: &str, model_id: &str) -> Value {
        self.provider = provider.to_string();
        self.model_id = model_id.to_string();
        self.append_typed(
            "model_change",
            json!({"provider": provider, "modelId": model_id}),
        );
        self.model_json()
    }

    pub fn cycle_model(&mut self) -> Option<Value> {
        let models = self.available_models();
        if models.is_empty() {
            return None;
        }
        let current = format!("{}/{}", self.provider, self.model_id);
        let idx = models
            .iter()
            .position(|m| {
                format!(
                    "{}/{}",
                    m["provider"].as_str().unwrap_or(""),
                    m["id"].as_str().unwrap_or("")
                ) == current
            })
            .unwrap_or(0);
        let next = &models[(idx + 1) % models.len()];
        let provider = next["provider"]
            .as_str()
            .unwrap_or(&self.provider)
            .to_string();
        let id = next["id"].as_str().unwrap_or(&self.model_id).to_string();
        Some(json!({
            "model": self.set_model(&provider, &id),
            "thinkingLevel": self.thinking.as_str(),
            "isScoped": !self.scoped_models.is_empty(),
        }))
    }

    pub fn available_models(&self) -> Vec<Value> {
        let mut all = list_models(None);
        for provider in &self.registry.providers {
            all.extend(models_from_provider(provider));
        }
        let filtered: Vec<_> = if self.scoped_models.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|m| {
                    self.scoped_models.iter().any(|pat| {
                        let spec = format!("{}/{}", m.provider, m.id);
                        spec.contains(pat) || m.id.contains(pat) || m.provider.contains(pat)
                    })
                })
                .collect()
        };
        filtered.into_iter().map(model_value).collect()
    }

    pub fn resolve_session_model(&self, pattern: &str) -> Option<Model> {
        if let Some((provider, id)) = pattern.split_once('/') {
            if let Some(found) = self
                .registry
                .provider(provider)
                .and_then(|p| models_from_provider(p).into_iter().find(|m| m.id == id))
            {
                return Some(found);
            }
        }
        for provider in &self.registry.providers {
            if let Some(found) = models_from_provider(provider)
                .into_iter()
                .find(|m| m.id == pattern || format!("{}/{}", m.provider, m.id) == pattern)
            {
                return Some(found);
            }
        }
        resolve_model(pattern)
    }

    pub fn model_json(&self) -> Value {
        self.resolve_session_model(&format!("{}/{}", self.provider, self.model_id))
            .map(model_value)
            .unwrap_or_else(|| json!({"provider": self.provider, "id": self.model_id}))
    }

    pub fn set_thinking(&mut self, level: ThinkingLevel) {
        self.thinking = level;
        self.append_typed("thinking_level_change", json!({"level": level.as_str()}));
    }

    pub fn compact(&mut self, instructions: Option<&str>) -> Value {
        self.is_compacting = true;
        let result = compact_messages(&self.messages, instructions, 4);
        self.messages = result.retained_tail.clone();
        self.append_typed("compaction", json!({"summary": result.summary}));
        self.is_compacting = false;
        json!({"summary": result.summary, "retained": self.messages.len()})
    }

    pub fn state(&self) -> Value {
        json!({
            "model": self.model_json(),
            "thinkingLevel": self.thinking.as_str(),
            "isStreaming": self.is_streaming,
            "isCompacting": self.is_compacting,
            "steeringMode": match self.steer.mode {
                QueueMode::All => "all",
                QueueMode::OneAtATime => "one-at-a-time",
            },
            "followUpMode": match self.follow_up.mode {
                QueueMode::All => "all",
                QueueMode::OneAtATime => "one-at-a-time",
            },
            "sessionFile": self.session_path.as_ref().map(|p| p.display().to_string()),
            "sessionId": self.session_id,
            "sessionName": self.session_name,
            "autoCompactionEnabled": self.auto_compact,
            "messageCount": self.messages.len(),
            "pendingMessageCount": self.steer.items.len() + self.follow_up.items.len(),
            "extensionProviders": self.registry.providers.iter().map(|provider| {
                json!({
                    "name": provider.name,
                    "native": provider.native,
                    "path": provider.path.display().to_string(),
                })
            }).collect::<Vec<_>>(),
            "extensionFlags": self.registry.flags.iter().map(|flag| {
                json!({
                    "name": flag.name,
                    "type": flag.flag_type,
                    "default": flag.default,
                    "description": flag.description,
                    "path": flag.path.display().to_string(),
                })
            }).collect::<Vec<_>>(),
        })
    }

    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session_path
            .as_ref()
            .and_then(|p| pi_session::read_session_info(p).ok())
    }

    pub fn switch_session(&mut self, path: &str) -> Result<bool, String> {
        let info = pi_session::read_session_info(Path::new(path)).map_err(|e| e.to_string())?;
        self.session_id = info.id;
        self.session_path = Some(info.path);
        self.session_name = info.name;
        Ok(false)
    }

    pub fn fork(&mut self, entry_id: &str) -> Result<Value, String> {
        let Some(info) = self.session_info() else {
            return Err("no session".into());
        };
        let dest = fork_from_entry(
            &default_sessions_root(),
            &info,
            &self.cwd.to_string_lossy(),
            entry_id,
        )
        .map_err(|e| e.to_string())?;
        self.session_id = dest.id.clone();
        self.session_path = Some(dest.path.clone());
        Ok(json!({"text": dest.id, "cancelled": false, "path": dest.path.display().to_string()}))
    }

    pub fn clone_current(&mut self) -> Result<Value, String> {
        let Some(info) = self.session_info() else {
            return Err("no session".into());
        };
        let dest = clone_session(&default_sessions_root(), &info, &self.cwd.to_string_lossy())
            .map_err(|e| e.to_string())?;
        self.session_id = dest.id;
        self.session_path = Some(dest.path);
        Ok(json!({"cancelled": false}))
    }

    pub fn entries_since(&self, since: Option<&str>) -> Value {
        let Some(path) = &self.session_path else {
            return json!({"entries": [], "leafId": null});
        };
        let mut entries = read_entries(path).unwrap_or_default();
        if let Some(since) = since {
            if let Some(idx) = entries
                .iter()
                .position(|e| e.get("id").and_then(|v| v.as_str()) == Some(since))
            {
                entries = entries.split_off(idx.saturating_add(1));
            }
        }
        json!({"entries": entries, "leafId": leaf_id(path).ok().flatten()})
    }

    pub fn tree(&self) -> Value {
        let Some(path) = &self.session_path else {
            return json!({"tree": [], "leafId": null});
        };
        json!({
            "tree": session_tree(path).unwrap_or_default(),
            "leafId": leaf_id(path).ok().flatten(),
        })
    }

    pub fn last_assistant(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .or_else(|| {
                self.session_path
                    .as_ref()
                    .and_then(|p| last_assistant_text(p).ok().flatten())
            })
    }

    pub fn export_html(&self, output: Option<&str>) -> Result<PathBuf, String> {
        let path = self
            .session_path
            .as_ref()
            .ok_or_else(|| "no session".to_string())?;
        let renderer = crate::tool_html::ToolHtmlRenderer::new(
            self.theme.clone(),
            self.cwd.clone(),
            self.registry.tools.clone(),
        );
        let state = export::HtmlExportState {
            system_prompt: Some(self.system_prompt.clone()),
            tools: self.tools.schemas(),
        };
        export::export_from_file_with_renderer(
            &path.to_string_lossy(),
            output,
            &self.theme,
            &renderer,
            Some(&state),
        )
        .map_err(|e| e.to_string())
    }

    pub fn fork_user_messages(&self) -> Vec<Value> {
        self.session_path
            .as_ref()
            .and_then(|p| fork_messages(p).ok())
            .unwrap_or_default()
    }

    pub fn stats(&self) -> Value {
        if let Some(path) = &self.session_path {
            if let Ok(stats) = session_stats(path) {
                return stats;
            }
        }
        json!({"messageCount": self.messages.len()})
    }

    pub fn set_name(&mut self, name: &str) {
        self.session_name = Some(name.to_string());
        if let Some(path) = &self.session_path {
            let _ = append_session_name(path, name);
        }
    }

    pub fn bash(&self, command: &str) -> Result<Value, String> {
        match self
            .tools
            .get("bash")
            .map(|t| t.execute(&json!({"command": command}), &self.cwd))
        {
            Some(Ok(result)) => Ok(json!({
                "output": result.output,
                "isError": result.is_error,
                "details": result.details,
            })),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("bash tool disabled".into()),
        }
    }
}

struct ProviderRequest {
    api_key: Option<String>,
    base_url: Option<String>,
    extra_headers: Vec<(String, String)>,
    api: Option<String>,
}

pub fn to_json_event(event: &AgentEvent) -> Value {
    serde_json::to_value(event).unwrap_or(json!({"type":"error","message":"serialize"}))
}

fn model_value(model: Model) -> Value {
    json!({
        "provider": model.provider,
        "id": model.id,
        "name": model.name,
        "api": model.api,
        "baseUrl": model.base_url,
        "reasoning": model.reasoning,
        "contextWindow": model.context_window,
        "maxTokens": model.max_tokens,
    })
}

fn models_from_provider(provider: &extensions::RegisteredProviderMeta) -> Vec<Model> {
    let default_api = provider
        .config
        .get("api")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let default_base = provider
        .config
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    provider
        .config
        .get("models")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let obj = raw.as_object()?;
            Some(Model {
                id: obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| obj.get("id").and_then(|v| v.as_str()).unwrap_or(""))
                    .to_string(),
                api: obj
                    .get("api")
                    .and_then(|v| v.as_str())
                    .unwrap_or(default_api)
                    .to_string(),
                provider: provider.name.clone(),
                base_url: obj
                    .get("baseUrl")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| default_base.clone()),
                reasoning: obj
                    .get("reasoning")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                input: obj
                    .get("input")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["text".into()]),
                cost: serde_json::from_value(obj.get("cost").cloned().unwrap_or(Value::Null))
                    .unwrap_or(ModelCost {
                        input: 0.0,
                        output: 0.0,
                        cache_read: 0.0,
                        cache_write: 0.0,
                    }),
                context_window: obj
                    .get("contextWindow")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(128_000),
                max_tokens: obj
                    .get("maxTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(16_384),
                extra: obj.clone(),
            })
        })
        .filter(|m| !m.id.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::RegisteredProviderMeta;
    use std::path::PathBuf;

    fn runtime_with_provider() -> SessionRuntime {
        let mut runtime = SessionRuntime {
            cwd: PathBuf::from("."),
            provider: "my-proxy".into(),
            model_id: "proxy-sm".into(),
            system_prompt: "test".into(),
            messages: vec![],
            steer: Default::default(),
            follow_up: Default::default(),
            thinking: ThinkingLevel::Off,
            tools: ToolRegistry::with_names(&[]),
            session_path: None,
            session_id: "sess".into(),
            session_name: None,
            scoped_models: vec![],
            auto_compact: true,
            auto_retry: true,
            is_streaming: false,
            is_compacting: false,
            aborted: false,
            api_key: None,
            allow_network: false,
            fixture: None,
            bus: EventBus::new(),
            max_turns: 2,
            context_window: 128_000,
            ui: crate::extension_ui::ExtensionUiHost::default(),
            extensions: vec![],
            registry: ExtensionRegistry::default(),
            theme: "dark".into(),
            flag_values: Default::default(),
            pending_custom_lines: Vec::new(),
            pending_next_turn: Vec::new(),
            pending_custom_messages: Vec::new(),
            pending_trigger_turn: false,
            running_turn: false,
            last_extension_turn_events: Vec::new(),
        };
        runtime.registry.providers.push(RegisteredProviderMeta {
            name: "my-proxy".into(),
            native: false,
            path: PathBuf::from("plugin.js"),
            config: json!({
                "name": "My Proxy",
                "baseUrl": "https://proxy.example",
                "apiKey": "literal-key",
                "api": "openai-completions",
                "authHeader": true,
                "headers": {"X-Proxy": "1"},
                "models": [{
                    "id": "proxy-sm",
                    "name": "Proxy SM",
                    "reasoning": false,
                    "input": ["text"],
                    "cost": {"input": 0, "output": 0},
                    "contextWindow": 8192,
                    "maxTokens": 1024
                }]
            }),
        });
        runtime
    }

    #[test]
    fn registered_provider_models_and_request() {
        let runtime = runtime_with_provider();
        let models = runtime.available_models();
        assert!(models
            .iter()
            .any(|m| m["provider"] == "my-proxy" && m["id"] == "proxy-sm"));
        let resolved = runtime.resolve_session_model("my-proxy/proxy-sm").unwrap();
        assert_eq!(resolved.base_url.as_deref(), Some("https://proxy.example"));
        let config = runtime.config();
        assert_eq!(config.base_url.as_deref(), Some("https://proxy.example"));
        assert_eq!(config.api.as_deref(), Some("openai-completions"));
        assert_eq!(config.api_key.as_deref(), Some("literal-key"));
        assert!(config
            .extra_headers
            .iter()
            .any(|(k, v)| k == "X-Proxy" && v == "1"));
        assert!(config
            .extra_headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer literal-key"));
    }

    #[test]
    fn apply_cli_flags_matches_typescript_errors() {
        let mut runtime = runtime_with_provider();
        runtime
            .registry
            .flags
            .push(crate::extensions::RegisteredFlagMeta {
                name: "region".into(),
                flag_type: "string".into(),
                default: Some(json!("us")),
                description: Some("Region".into()),
                path: PathBuf::from("plugin.js"),
            });
        runtime
            .registry
            .flags
            .push(crate::extensions::RegisteredFlagMeta {
                name: "verbose".into(),
                flag_type: "boolean".into(),
                default: Some(json!(false)),
                description: None,
                path: PathBuf::from("plugin.js"),
            });
        let mut flags = BTreeMap::new();
        flags.insert("verbose".into(), FlagValue::Bool(true));
        flags.insert("region".into(), FlagValue::String("eu".into()));
        runtime.apply_cli_flags(&flags).unwrap();
        assert_eq!(runtime.flag_values["verbose"], json!(true));
        assert_eq!(runtime.flag_values["region"], json!("eu"));
        let mut missing = BTreeMap::new();
        missing.insert("region".into(), FlagValue::Bool(true));
        let err = runtime.apply_cli_flags(&missing).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e == "Extension flag \"--region\" requires a value"));
        let mut unknown = BTreeMap::new();
        unknown.insert("nope".into(), FlagValue::Bool(true));
        unknown.insert("also".into(), FlagValue::String("x".into()));
        let err = runtime.apply_cli_flags(&unknown).unwrap_err();
        assert!(err.iter().any(|e| e == "Unknown options: --also, --nope"));
    }

    #[test]
    fn send_message_queues_and_writes_custom_message_entries() {
        let mut runtime = runtime_with_provider();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sess.jsonl");
        std::fs::write(&path, "").unwrap();
        runtime.session_path = Some(path.clone());

        runtime.send_custom_message(
            &json!({"customType":"note","content":"idle","display":true}),
            &json!({}),
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"type\":\"custom_message\""));
        assert!(raw.contains("\"customType\":\"note\""));
        assert_eq!(runtime.messages.last().unwrap().role, "custom");
        assert!(runtime.steer.items.is_empty());

        runtime.is_streaming = true;
        runtime.send_custom_message(
            &json!({"customType":"note","content":"steer-me","display":true}),
            &json!({}),
        );
        assert_eq!(runtime.steer.items.len(), 1);
        assert_eq!(runtime.steer.items[0].content, "steer-me");
        runtime.send_custom_message(
            &json!({"customType":"note","content":"follow","display":true}),
            &json!({"deliverAs":"followUp"}),
        );
        assert_eq!(runtime.follow_up.items.len(), 1);
        runtime.send_custom_message(
            &json!({"customType":"note","content":"later","display":true}),
            &json!({"deliverAs":"nextTurn"}),
        );
        assert_eq!(runtime.pending_next_turn.len(), 1);
        runtime.send_custom_message(
            &json!({"customType":"note","content":"defer","display":false}),
            &json!({"triggerTurn": false}),
        );
        assert_eq!(runtime.pending_custom_messages.len(), 3);

        runtime.is_streaming = false;
        runtime.flush_pending_custom_messages();
        runtime.flush_pending_next_turn();
        assert!(runtime.messages.iter().any(|m| m.content == "later"));
        assert!(runtime.messages.iter().any(|m| m.content == "defer"));

        runtime.is_streaming = true;
        runtime.send_user_message_action(&json!("queued-user"), &json!({"deliverAs":"followUp"}));
        assert!(runtime
            .follow_up
            .items
            .iter()
            .any(|m| m.content == "queued-user"));
    }
}
