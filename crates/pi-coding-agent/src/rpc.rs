use std::path::{Path, PathBuf};

use pi_agent::{Agent, QueueMode};
use pi_ai::{find_model, load_builtin_models, Model};
use pi_protocol::ThinkingLevel;
use pi_session::JsonlSession;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::export;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            if let Some(message) = &command.message {
                runtime.agent.prompt(message);
            }
            ok(id, &kind, None)
        }
        "steer" => {
            if let Some(message) = &command.message {
                runtime.agent.queues.enqueue_steer(message);
            }
            ok(id, &kind, None)
        }
        "follow_up" => {
            if let Some(message) = &command.message {
                runtime.agent.queues.enqueue_follow_up(message);
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
                "isStreaming": false,
                "isCompacting": false,
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
            }
            ok(id, &kind, None)
        }
        "set_auto_retry" => {
            if let Some(enabled) = command.enabled {
                runtime.agent.auto_retry = enabled;
            }
            ok(id, &kind, None)
        }
        "abort_retry" => ok(id, &kind, None),
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
        "get_session_stats" => ok(
            id,
            &kind,
            Some(serde_json::json!({
                "messageCount": runtime.agent.messages.len(),
                "user": runtime.agent.messages.iter().filter(|m| m.role == "user").count(),
                "assistant": runtime.agent.messages.iter().filter(|m| m.role == "assistant").count(),
            })),
        ),
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
            Some(serde_json::json!({ "commands": crate::slash::rpc_commands() })),
        ),
        "bash" => {
            if runtime.bash_aborted {
                runtime.bash_aborted = false;
                return ok(id, &kind, Some(serde_json::json!({ "cancelled": true })));
            }
            match pi_agent::execute_tool(
                &runtime.cwd,
                "bash",
                &serde_json::json!({ "command": command.command.unwrap_or_default() }),
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
        "get_entries" => ok(
            id,
            &kind,
            Some(serde_json::json!({
                "entries": runtime.agent.entries_since(command.since.as_deref())
            })),
        ),
        "get_tree" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "nodes": runtime.agent.session_tree() })),
        ),
        "get_fork_messages" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "messages": runtime.agent.messages })),
        ),
        "abort_bash" => {
            runtime.bash_aborted = true;
            ok(id, &kind, None)
        }
        other => fail(id, other, format!("Unknown RPC command: {other}")),
    }
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
