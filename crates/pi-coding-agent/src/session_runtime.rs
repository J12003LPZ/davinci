//! Shared AgentSession runtime used by print, JSON, RPC, and interactive.

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
        for tool in &self.registry.tools {
            self.tools.register(Box::new(ExtensionTool::from_meta(tool)));
        }
    }

    pub fn config(&self) -> AgentConfig {
        let (api_key, base_url, extra_headers, api) = self.provider_request();
        AgentConfig {
            cwd: self.cwd.clone(),
            system_prompt: self.system_prompt.clone(),
            model_provider: self.provider.clone(),
            model_id: self.model_id.clone(),
            api_key,
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
            base_url,
            extra_headers,
            api,
        }
    }

    fn provider_request(
        &self,
    ) -> (
        Option<String>,
        Option<String>,
        Vec<(String, String)>,
        Option<String>,
    ) {
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
                            extensions::resolve_config_value(raw).unwrap_or_else(|| raw.to_string()),
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
        (api_key, base_url, extra_headers, api)
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
        let invoked = extensions::invoke_extension_command(&path, name, args, &json!({}))?;
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
        self.messages.push(AgentMessage {
            role: "user".into(),
            content: text.to_string(),
            images,
        });
        self.append_message("user", text);
        self.run_turn()
    }

    pub fn run_turn(&mut self) -> Result<Vec<AgentEvent>, String> {
        self.bus.emit(
            "agent_start",
            json!({"provider": self.provider, "model": self.model_id}),
        );
        let _ = self.fire_extensions(
            "turn_start",
            &json!({"provider": self.provider, "model": self.model_id}),
        );
        self.is_streaming = true;
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
        self.bus.emit("agent_end", json!({"ok": true}));
        let _ = self.fire_extensions("turn_end", &json!({"ok": true}));
        Ok(events)
    }

    pub fn fire_extensions(&mut self, event: &str, data: &Value) -> Vec<Value> {
        let paths: Vec<_> = self.extensions.iter().map(|e| e.path.clone()).collect();
        let mut events = Vec::new();
        for path in paths {
            if let Ok(invoked) = extensions::invoke_extension_event(&path, event, data, &json!({}))
            {
                events.extend(self.ui.apply_calls(&invoked.ui_calls));
            }
        }
        events
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
        export::export_from_file(&path.to_string_lossy(), output).map_err(|e| e.to_string())
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
                max_tokens: obj.get("maxTokens").and_then(|v| v.as_u64()).unwrap_or(16_384),
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
        assert!(models.iter().any(|m| m["provider"] == "my-proxy" && m["id"] == "proxy-sm"));
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
}
