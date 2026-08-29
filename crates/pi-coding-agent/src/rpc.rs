use std::path::{Path, PathBuf};

use pi_agent::{Agent, QueueMode};
use pi_ai::{find_model, load_builtin_models, Model};
use pi_protocol::ThinkingLevel;
use pi_session::JsonlSession;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::export;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RpcCommand {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(rename = "modelId", default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(rename = "customInstructions", default)]
    pub custom_instructions: Option<String>,
    #[serde(rename = "sessionPath", default)]
    pub session_path: Option<String>,
    #[serde(rename = "entryId", default)]
    pub entry_id: Option<String>,
    #[serde(rename = "outputPath", default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(rename = "parentSession", default)]
    pub parent_session: Option<String>,
    #[serde(default)]
    pub images: Option<Vec<Value>>,
    #[serde(rename = "streamingBehavior", default)]
    pub streaming_behavior: Option<String>,
    #[serde(rename = "excludeFromContext", default)]
    pub exclude_from_context: Option<bool>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub confirmed: Option<bool>,
    #[serde(default)]
    pub cancelled: Option<bool>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(rename = "notifyType", default)]
    pub notify_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct RpcRuntime {
    pub agent: Agent,
    pub session_dir: PathBuf,
    pub cwd: PathBuf,
    pub models: Vec<Model>,
    pub bash_aborted: bool,
    pub invocable_commands: Vec<Value>,
    pub pending_ui: std::collections::HashMap<String, RpcCommand>,
}

impl RpcRuntime {
    pub fn new(agent: Agent, session_dir: PathBuf, cwd: PathBuf) -> Self {
        let models = load_builtin_models();
        Self {
            agent,
            session_dir,
            cwd,
            models,
            bash_aborted: false,
            invocable_commands: Vec::new(),
            pending_ui: std::collections::HashMap::new(),
        }
    }

    fn current_model(&self) -> Option<&Model> {
        find_model(&self.models, &self.agent.provider, &self.agent.model_id).or_else(|| {
            self.models
                .iter()
                .find(|model| model.provider == self.agent.provider)
                .or_else(|| self.models.first())
        })
    }

    fn model_json(model: &Model) -> Value {
        serde_json::to_value(model).unwrap_or(Value::Null)
    }
}

pub fn handle_rpc(runtime: &mut RpcRuntime, command: RpcCommand) -> RpcResponse {
    let id = command.id.clone();
    let kind = command.kind.clone();
    match kind.as_str() {
        "prompt" => {
            let images = command
                .images
                .as_deref()
                .map(pi_agent::parse_rpc_images)
                .unwrap_or_default();
            if let Some(message) = &command.message {
                runtime.agent.prompt_with(message, &images);
            }
            if command.streaming_behavior.as_deref() == Some("followUp") {
                runtime.agent.queues.follow_up_mode = QueueMode::All;
            }
            if command.streaming_behavior.as_deref() == Some("steer") {
                runtime.agent.queues.steer_mode = QueueMode::All;
            }
            ok(id, &kind, None)
        }
        "steer" => {
            let images = command
                .images
                .as_deref()
                .map(pi_agent::parse_rpc_images)
                .unwrap_or_default();
            if let Some(message) = &command.message {
                runtime.agent.queues.enqueue_steer_with(message, images);
            }
            ok(id, &kind, None)
        }
        "follow_up" => {
            let images = command
                .images
                .as_deref()
                .map(pi_agent::parse_rpc_images)
                .unwrap_or_default();
            if let Some(message) = &command.message {
                runtime.agent.queues.enqueue_follow_up_with(message, images);
            }
            ok(id, &kind, None)
        }
        "abort" => {
            runtime.agent.abort();
            ok(id, &kind, None)
        }
        "clear_queue" => {
            let (steering, follow_up) = runtime.agent.queues.clear();
            ok(
                id,
                &kind,
                Some(serde_json::json!({ "steering": steering, "followUp": follow_up })),
            )
        }
        "get_state" => ok(
            id,
            &kind,
            Some(serde_json::json!({
                "model": runtime.current_model().map(RpcRuntime::model_json),
                "thinkingLevel": runtime.agent.thinking_level,
                "isStreaming": runtime.agent.is_streaming,
                "isCompacting": runtime.agent.is_compacting,
                "steeringMode": runtime.agent.queues.steer_mode,
                "followUpMode": runtime.agent.queues.follow_up_mode,
                "sessionFile": runtime.agent.session.as_ref().map(|s| s.path.display().to_string()),
                "sessionId": runtime.agent.session.as_ref().map(|s| s.header.id.clone()).unwrap_or_default(),
                "sessionName": runtime.agent.session.as_ref().and_then(|s| s.display_name()),
                "autoCompactionEnabled": runtime.agent.auto_compaction,
                "messageCount": runtime.agent.messages.len(),
                "pendingMessageCount": runtime.agent.queues.steer.len() + runtime.agent.queues.follow_up.len(),
            })),
        ),
        "set_thinking_level" => {
            if let Some(level) = command.level.as_deref().and_then(ThinkingLevel::parse) {
                runtime.agent.thinking_level = level;
            }
            ok(id, &kind, None)
        }
        "set_steering_mode" => {
            runtime.agent.queues.steer_mode = parse_queue_mode(command.mode.as_deref());
            ok(id, &kind, None)
        }
        "set_follow_up_mode" => {
            runtime.agent.queues.follow_up_mode = parse_queue_mode(command.mode.as_deref());
            ok(id, &kind, None)
        }
        "compact" => {
            let result = runtime
                .agent
                .compact(command.custom_instructions.as_deref());
            ok(
                id,
                &kind,
                Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            )
        }
        "set_auto_compaction" => {
            if let Some(enabled) = command.enabled {
                runtime.agent.auto_compaction = enabled;
                runtime.agent.compaction.enabled = enabled;
            }
            ok(id, &kind, None)
        }
        "set_auto_retry" => {
            if let Some(enabled) = command.enabled {
                runtime.agent.auto_retry = enabled;
            }
            ok(id, &kind, None)
        }
        "abort_retry" => {
            runtime.agent.abort_retry();
            ok(id, &kind, None)
        }
        "get_messages" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "messages": runtime.agent.messages })),
        ),
        "get_last_assistant_text" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "text": runtime.agent.last_assistant_text() })),
        ),
        "get_session_stats" => ok(id, &kind, Some(session_stats_json(runtime))),
        "set_session_name" => {
            if let (Some(session), Some(name)) =
                (runtime.agent.session.as_mut(), command.name.as_deref())
            {
                let _ = session.set_name(name);
            }
            ok(id, &kind, None)
        }
        "get_commands" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "commands": runtime.invocable_commands })),
        ),
        "bash" => {
            if runtime.bash_aborted {
                runtime.bash_aborted = false;
                return ok(id, &kind, Some(serde_json::json!({ "cancelled": true })));
            }
            let exclude = command.exclude_from_context.unwrap_or(false);
            match pi_agent::execute_tool(
                &runtime.cwd,
                "bash",
                &serde_json::json!({
                    "command": command.command.unwrap_or_default(),
                    "excludeFromContext": exclude
                }),
            ) {
                Ok(result) => ok(
                    id,
                    &kind,
                    Some(serde_json::to_value(result).unwrap_or(Value::Null)),
                ),
                Err(err) => fail(id, &kind, err.to_string()),
            }
        }
        "get_available_thinking_levels" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "levels": ThinkingLevel::all() })),
        ),
        "set_model" => {
            let provider = command
                .provider
                .unwrap_or_else(|| runtime.agent.provider.clone());
            let model_id = command.model_id.unwrap_or_default();
            if let Some(model) = find_model(&runtime.models, &provider, &model_id).cloned() {
                runtime.agent.provider = provider;
                runtime.agent.model_id = model_id;
                ok(id, &kind, Some(RpcRuntime::model_json(&model)))
            } else {
                fail(id, &kind, format!("Unknown model {provider}/{model_id}"))
            }
        }
        "cycle_model" => {
            if runtime.models.is_empty() {
                return ok(id, &kind, Some(Value::Null));
            }
            let current = runtime
                .models
                .iter()
                .position(|model| {
                    model.provider == runtime.agent.provider && model.id == runtime.agent.model_id
                })
                .unwrap_or(0);
            let next = &runtime.models[(current + 1) % runtime.models.len()];
            runtime.agent.provider = next.provider.clone();
            runtime.agent.model_id = next.id.clone();
            ok(
                id,
                &kind,
                Some(serde_json::json!({
                    "model": RpcRuntime::model_json(next),
                    "thinkingLevel": runtime.agent.thinking_level,
                    "isScoped": false,
                })),
            )
        }
        "get_available_models" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "models": runtime.models })),
        ),
        "cycle_thinking_level" => {
            let levels = ThinkingLevel::all();
            let current = levels
                .iter()
                .position(|level| *level == runtime.agent.thinking_level)
                .unwrap_or(0);
            let next = levels[(current + 1) % levels.len()];
            runtime.agent.thinking_level = next;
            ok(id, &kind, Some(serde_json::json!({ "level": next })))
        }
        "new_session" => match create_session(runtime, command.parent_session.as_deref()) {
            Ok(cancelled) => ok(
                id,
                &kind,
                Some(serde_json::json!({ "cancelled": cancelled })),
            ),
            Err(err) => fail(id, &kind, err),
        },
        "clone" => match clone_session(runtime) {
            Ok(cancelled) => ok(
                id,
                &kind,
                Some(serde_json::json!({ "cancelled": cancelled })),
            ),
            Err(err) => fail(id, &kind, err),
        },
        "fork" => match fork_session(runtime, command.entry_id.as_deref()) {
            Ok(data) => ok(id, &kind, Some(data)),
            Err(err) => fail(id, &kind, err),
        },
        "switch_session" => match switch_session(runtime, command.session_path.as_deref()) {
            Ok(cancelled) => ok(
                id,
                &kind,
                Some(serde_json::json!({ "cancelled": cancelled })),
            ),
            Err(err) => fail(id, &kind, err),
        },
        "export_html" => {
            let Some(session) = &runtime.agent.session else {
                return fail(id, &kind, "No session".into());
            };
            let output = command
                .output_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("session.html"));
            match export::export_html(session, &output) {
                Ok(path) => ok(id, &kind, Some(serde_json::json!({ "path": path }))),
                Err(err) => fail(id, &kind, err),
            }
        }
        "get_entries" => {
            let session = runtime.agent.session.as_ref();
            let entries = session.map(|item| item.entries.as_slice()).unwrap_or(&[]);
            match pi_session::entries_since(entries, command.since.as_deref()) {
                Ok(entries) => ok(
                    id,
                    &kind,
                    Some(serde_json::json!({
                        "entries": entries,
                        "leafId": session.and_then(|item| item.leaf_id.clone())
                    })),
                ),
                Err(err) => fail(id, &kind, err),
            }
        }
        "get_tree" => {
            let session = runtime.agent.session.as_ref();
            let entries = session.map(|item| item.entries.as_slice()).unwrap_or(&[]);
            ok(
                id,
                &kind,
                Some(serde_json::json!({
                    "tree": pi_session::build_session_tree(entries),
                    "leafId": session.and_then(|item| item.leaf_id.clone())
                })),
            )
        }
        "get_fork_messages" => {
            let entries = runtime
                .agent
                .session
                .as_ref()
                .map(|item| item.entries.as_slice())
                .unwrap_or(&[]);
            ok(
                id,
                &kind,
                Some(serde_json::json!({
                    "messages": pi_session::fork_user_messages(entries)
                })),
            )
        }
        "abort_bash" => {
            runtime.bash_aborted = true;
            ok(id, &kind, None)
        }
        "extension_ui_response" => {
            if let Some(response_id) = id.clone() {
                runtime.pending_ui.insert(response_id, command);
            }
            ok(id, &kind, None)
        }
        other => fail(id, other, format!("Unknown RPC command: {other}")),
    }
}

pub fn extension_ui_request(id: &str, method: &str, extra: Value) -> Value {
    let mut object = extra.as_object().cloned().unwrap_or_default();
    object.insert("type".into(), Value::String("extension_ui_request".into()));
    object.insert("id".into(), Value::String(id.to_string()));
    object.insert("method".into(), Value::String(method.to_string()));
    Value::Object(object)
}

pub fn extension_ui_requests_from_calls(calls: &[Value]) -> Vec<Value> {
    calls
        .iter()
        .filter_map(|call| {
            let op = call.get("op").and_then(Value::as_str)?;
            let id = uuid::Uuid::new_v4().to_string();
            Some(match op {
                "select" => extension_ui_request(
                    &id,
                    "select",
                    serde_json::json!({
                        "title": call.get("title"),
                        "options": call.get("options"),
                    }),
                ),
                "confirm" => extension_ui_request(
                    &id,
                    "confirm",
                    serde_json::json!({
                        "title": call.get("title"),
                        "message": call.get("message"),
                    }),
                ),
                "input" => extension_ui_request(
                    &id,
                    "input",
                    serde_json::json!({
                        "title": call.get("title"),
                        "placeholder": call.get("placeholder"),
                    }),
                ),
                "editor" => extension_ui_request(
                    &id,
                    "editor",
                    serde_json::json!({
                        "title": call.get("title"),
                        "prefill": call.get("prefill"),
                    }),
                ),
                "notify" => extension_ui_request(
                    &id,
                    "notify",
                    serde_json::json!({
                        "message": call.get("message"),
                        "notifyType": call.get("type"),
                    }),
                ),
                "setStatus" => extension_ui_request(
                    &id,
                    "setStatus",
                    serde_json::json!({
                        "statusKey": call.get("key"),
                        "statusText": call.get("text"),
                    }),
                ),
                "setWidget" => extension_ui_request(
                    &id,
                    "setWidget",
                    serde_json::json!({
                        "widgetKey": call.get("key"),
                        "widgetLines": call.get("lines"),
                        "widgetPlacement": call.get("placement"),
                    }),
                ),
                "setTitle" => extension_ui_request(
                    &id,
                    "setTitle",
                    serde_json::json!({ "title": call.get("title") }),
                ),
                "setEditorText" | "pasteToEditor" => extension_ui_request(
                    &id,
                    "set_editor_text",
                    serde_json::json!({ "text": call.get("text") }),
                ),
                _ => return None,
            })
        })
        .collect()
}

fn create_session(runtime: &mut RpcRuntime, parent: Option<&str>) -> Result<bool, String> {
    let mut session =
        JsonlSession::create(&runtime.session_dir, &runtime.cwd.to_string_lossy(), None)
            .map_err(|err| err.to_string())?;
    if let Some(parent) = parent {
        session.header.parent_session_id = Some(parent.to_string());
    }
    let prompt = runtime.agent.system_prompt.clone();
    let mut next = Agent::new(prompt);
    next.cwd = runtime.cwd.clone();
    next.provider = runtime.agent.provider.clone();
    next.model_id = runtime.agent.model_id.clone();
    next.tools = runtime.agent.tools.clone();
    next.session = Some(session);
    runtime.agent = next;
    Ok(false)
}

fn clone_session(runtime: &mut RpcRuntime) -> Result<bool, String> {
    let Some(session) = &runtime.agent.session else {
        return Ok(true);
    };
    let cloned = session
        .clone_session(&runtime.session_dir)
        .map_err(|err| err.to_string())?;
    runtime.agent.load_from_session(cloned);
    Ok(false)
}

fn fork_session(runtime: &mut RpcRuntime, entry_id: Option<&str>) -> Result<Value, String> {
    let Some(session) = &runtime.agent.session else {
        return Ok(serde_json::json!({ "text": "", "cancelled": true }));
    };
    let entry_id = entry_id
        .map(str::to_string)
        .or_else(|| session.leaf_id.clone())
        .ok_or_else(|| "No entry to fork".to_string())?;
    let forked = session
        .fork(&entry_id, &runtime.session_dir)
        .map_err(|err| err.to_string())?;
    runtime.agent.load_from_session(forked);
    Ok(serde_json::json!({
        "text": runtime.agent.last_assistant_text().unwrap_or_default(),
        "cancelled": false,
    }))
}

fn switch_session(runtime: &mut RpcRuntime, session_path: Option<&str>) -> Result<bool, String> {
    let Some(session_path) = session_path else {
        return Ok(true);
    };
    let path = Path::new(session_path);
    let session = JsonlSession::open(path).map_err(|err| err.to_string())?;
    runtime.agent.load_from_session(session);
    Ok(false)
}

pub fn session_stats_json(runtime: &RpcRuntime) -> Value {
    session_stats_for_agent(&runtime.agent, runtime.current_model())
}

pub fn session_stats_for_agent(agent: &Agent, model: Option<&Model>) -> Value {
    let session = agent.session.as_ref();
    let entries = session.map(|item| item.entries.as_slice()).unwrap_or(&[]);
    let stats = pi_session::session_usage_stats(entries);
    let context_usage = context_usage_json(
        entries,
        session.and_then(|item| item.leaf_id.as_deref()),
        model,
    );
    serde_json::json!({
        "sessionFile": session.map(|item| item.path.display().to_string()),
        "sessionId": session.map(|item| item.header.id.clone()).unwrap_or_default(),
        "userMessages": stats.user_messages,
        "assistantMessages": stats.assistant_messages,
        "toolCalls": stats.tool_calls,
        "toolResults": stats.tool_results,
        "totalMessages": stats.total_messages,
        "tokens": {
            "input": stats.input,
            "output": stats.output,
            "cacheRead": stats.cache_read,
            "cacheWrite": stats.cache_write,
            "total": stats.token_total(),
        },
        "cost": stats.cost,
        "contextUsage": context_usage,
    })
}

fn context_usage_json(
    entries: &[pi_session::SessionEntry],
    leaf_id: Option<&str>,
    model: Option<&Model>,
) -> Option<Value> {
    let model = model?;
    let context_window = model.context_window;
    if context_window == 0 {
        return None;
    }
    let branch = pi_session::branch_entries(entries, leaf_id);
    let compaction = branch
        .iter()
        .rposition(|entry| entry.entry_type == "compaction");
    if let Some(index) = compaction {
        let has_post = branch[index + 1..].iter().any(|entry| {
            entry.entry_type == "message"
                && entry
                    .message
                    .as_ref()
                    .and_then(|message| message.get("role"))
                    .and_then(Value::as_str)
                    == Some("assistant")
                && entry
                    .message
                    .as_ref()
                    .and_then(|message| message.get("usage"))
                    .is_some()
        });
        if !has_post {
            return Some(serde_json::json!({
                "tokens": Value::Null,
                "contextWindow": context_window,
                "percent": Value::Null
            }));
        }
    }
    let tokens = pi_session::session_usage_stats(
        &branch
            .iter()
            .map(|entry| (*entry).clone())
            .collect::<Vec<_>>(),
    )
    .token_total();
    Some(serde_json::json!({
        "tokens": tokens,
        "contextWindow": context_window,
        "percent": (tokens as f64 / context_window as f64) * 100.0
    }))
}

fn parse_queue_mode(value: Option<&str>) -> QueueMode {
    match value {
        Some("one-at-a-time") => QueueMode::OneAtATime,
        _ => QueueMode::All,
    }
}

fn ok(id: Option<String>, command: &str, data: Option<Value>) -> RpcResponse {
    RpcResponse {
        id,
        kind: "response".into(),
        command: command.to_string(),
        success: true,
        data,
        error: None,
    }
}

fn fail(id: Option<String>, command: &str, error: String) -> RpcResponse {
    RpcResponse {
        id,
        kind: "response".into(),
        command: command.to_string(),
        success: false,
        data: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_agent::default_system_prompt;

    #[test]
    fn extension_ui_protocol_matches_ts_shapes() {
        let request = extension_ui_request(
            "ui-1",
            "select",
            serde_json::json!({ "title": "Pick", "options": ["a", "b"] }),
        );
        assert_eq!(request["type"], "extension_ui_request");
        assert_eq!(request["id"], "ui-1");
        assert_eq!(request["method"], "select");
        assert_eq!(request["options"][0], "a");
        let calls = vec![serde_json::json!({"op":"notify","message":"ready","type":"info"})];
        let emitted = extension_ui_requests_from_calls(&calls);
        assert_eq!(emitted[0]["method"], "notify");
        assert_eq!(emitted[0]["notifyType"], "info");
        let mut runtime = RpcRuntime::new(
            pi_agent::Agent::new(default_system_prompt()),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        let response = handle_rpc(
            &mut runtime,
            RpcCommand {
                id: Some("ui-1".into()),
                kind: "extension_ui_response".into(),
                value: Some("a".into()),
                ..RpcCommand::default()
            },
        );
        assert!(response.success);
        assert_eq!(
            runtime
                .pending_ui
                .get("ui-1")
                .and_then(|item| item.value.clone()),
            Some("a".into())
        );
    }

    #[test]
    fn prompt_images_and_abort_retry_match_ts() {
        let mut runtime = RpcRuntime::new(
            pi_agent::Agent::new(default_system_prompt()),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        let response = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("see this".into()),
                images: Some(vec![serde_json::json!({
                    "type": "image",
                    "data": "abc",
                    "mimeType": "image/png"
                })]),
                ..RpcCommand::default()
            },
        );
        assert!(response.success);
        let user = runtime.agent.messages.last().expect("prompt");
        assert_eq!(user.role, "user");
        assert!(user
            .content
            .iter()
            .any(|block| matches!(block, pi_ai::MessageContent::Image { mime_type, .. } if mime_type == "image/png")));
        let abort = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "abort_retry".into(),
                ..RpcCommand::default()
            },
        );
        assert!(abort.success);
        assert!(runtime.agent.retry_aborted);
    }
}
