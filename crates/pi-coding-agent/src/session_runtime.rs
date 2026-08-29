//! Shared AgentSession runtime used by print, JSON, RPC, and interactive.

use crate::event_bus::EventBus;
use crate::export;
use crate::extension_ui::ExtensionUiHost;
use crate::extensions::{self, Extension};
use pi_agent::{
    compact_messages, run_agent, AgentConfig, AgentEvent, AgentMessage, AllowAllPermissionPolicy,
    FollowUpQueue, QueueMode, SteerQueue, ThinkingLevel, ToolRegistry,
};
use pi_ai::catalog::resolve_model;
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

    pub fn config(&self) -> AgentConfig {
        AgentConfig {
            cwd: self.cwd.clone(),
            system_prompt: self.system_prompt.clone(),
            model_provider: self.provider.clone(),
            model_id: self.model_id.clone(),
            api_key: self.api_key.clone(),
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
        }
    }

    pub fn prompt(&mut self, text: &str, images: Vec<Value>) -> Result<Vec<AgentEvent>, String> {
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
        let all = list_models(None);
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
        filtered
            .into_iter()
            .map(|m| {
                json!({
                    "provider": m.provider,
                    "id": m.id,
                    "name": m.name,
                    "api": m.api,
                })
            })
            .collect()
    }

    pub fn model_json(&self) -> Value {
        resolve_model(&format!("{}/{}", self.provider, self.model_id))
            .map(|m| {
                json!({
                    "provider": m.provider,
                    "id": m.id,
                    "name": m.name,
                    "api": m.api,
                })
            })
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
