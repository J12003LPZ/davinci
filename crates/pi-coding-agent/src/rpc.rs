use pi_agent::{Agent, QueueMode};
use pi_ai::content_text;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

pub fn handle_rpc(agent: &mut Agent, command: RpcCommand) -> RpcResponse {
    let id = command.id.clone();
    let kind = command.kind.clone();
    let result = match kind.as_str() {
        "prompt" => {
            if let Some(message) = &command.message {
                agent.prompt(message);
            }
            ok(id, &kind, None)
        }
        "steer" => {
            if let Some(message) = &command.message {
                agent.queues.enqueue_steer(message);
            }
            ok(id, &kind, None)
        }
        "follow_up" => {
            if let Some(message) = &command.message {
                agent.queues.enqueue_follow_up(message);
            }
            ok(id, &kind, None)
        }
        "abort" => ok(id, &kind, None),
        "clear_queue" => {
            let (steering, follow_up) = agent.queues.clear();
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
                "thinkingLevel": agent.thinking_level,
                "isStreaming": false,
                "isCompacting": false,
                "steeringMode": agent.queues.steer_mode,
                "followUpMode": agent.queues.follow_up_mode,
                "sessionFile": agent.session.as_ref().map(|s| s.path.display().to_string()),
                "sessionId": agent.session.as_ref().map(|s| s.header.id.clone()).unwrap_or_default(),
                "sessionName": agent.session.as_ref().and_then(|s| s.display_name()),
                "autoCompactionEnabled": agent.auto_compaction,
                "messageCount": agent.messages.len(),
                "pendingMessageCount": agent.queues.steer.len() + agent.queues.follow_up.len(),
            })),
        ),
        "set_thinking_level" => {
            if let Some(level) = command
                .level
                .as_deref()
                .and_then(pi_protocol::ThinkingLevel::parse)
            {
                agent.thinking_level = level;
            }
            ok(id, &kind, None)
        }
        "set_steering_mode" => {
            agent.queues.steer_mode = parse_queue_mode(command.mode.as_deref());
            ok(id, &kind, None)
        }
        "set_follow_up_mode" => {
            agent.queues.follow_up_mode = parse_queue_mode(command.mode.as_deref());
            ok(id, &kind, None)
        }
        "compact" => {
            let result = agent.compact(command.custom_instructions.as_deref());
            ok(
                id,
                &kind,
                Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            )
        }
        "set_auto_compaction" => {
            if let Some(enabled) = command.enabled {
                agent.auto_compaction = enabled;
            }
            ok(id, &kind, None)
        }
        "set_auto_retry" => {
            if let Some(enabled) = command.enabled {
                agent.auto_retry = enabled;
            }
            ok(id, &kind, None)
        }
        "abort_retry" => ok(id, &kind, None),
        "get_messages" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "messages": agent.messages })),
        ),
        "get_last_assistant_text" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "text": agent.last_assistant_text() })),
        ),
        "get_session_stats" => ok(
            id,
            &kind,
            Some(serde_json::json!({
                "messageCount": agent.messages.len(),
                "user": agent.messages.iter().filter(|m| m.role == "user").count(),
                "assistant": agent.messages.iter().filter(|m| m.role == "assistant").count(),
            })),
        ),
        "set_session_name" => {
            if let (Some(session), Some(name)) = (agent.session.as_mut(), command.name.as_deref()) {
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
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match pi_agent::execute_tool(
                &cwd,
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
            Some(serde_json::json!({ "levels": pi_protocol::ThinkingLevel::all() })),
        ),
        "new_session"
        | "clone"
        | "fork"
        | "switch_session"
        | "export_html"
        | "get_entries"
        | "get_tree"
        | "get_fork_messages"
        | "set_model"
        | "cycle_model"
        | "get_available_models"
        | "cycle_thinking_level"
        | "abort_bash" => ok(id, &kind, Some(serde_json::json!({ "cancelled": false }))),
        other => fail(id, other, format!("Unknown RPC command: {other}")),
    };
    let _ = content_text;
    result
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
