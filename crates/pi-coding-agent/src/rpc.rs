use pi_agent::{
    compact_messages, AgentMessage, FollowUpQueue, SteerQueue, ThinkingLevel, ToolRegistry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEnvelope {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub rest: Value,
}

pub fn handle_rpc(
    command: &Value,
    messages: &mut Vec<AgentMessage>,
    steer: &mut SteerQueue,
    follow_up: &mut FollowUpQueue,
    thinking: &mut ThinkingLevel,
    tools: &ToolRegistry,
) -> Value {
    let id = command.get("id").cloned();
    let ty = command.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let result = match ty {
        "prompt" => {
            if let Some(text) = command.get("message").and_then(|v| v.as_str()) {
                messages.push(AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images: command
                        .get("images")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                });
            }
            json!({"ok": true})
        }
        "steer" => {
            if let Some(text) = command.get("message").and_then(|v| v.as_str()) {
                steer.enqueue(AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images: vec![],
                });
            }
            json!({"ok": true, "queued": steer.items.len()})
        }
        "follow_up" => {
            if let Some(text) = command.get("message").and_then(|v| v.as_str()) {
                follow_up.enqueue(AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images: vec![],
                });
            }
            json!({"ok": true, "queued": follow_up.items.len()})
        }
        "abort" => json!({"ok": true, "aborted": true}),
        "clear_queue" => {
            steer.clear();
            follow_up.clear();
            json!({"ok": true})
        }
        "get_state" => json!({
            "ok": true,
            "thinkingLevel": thinking.as_str(),
            "isStreaming": false,
            "queuedSteer": steer.items.len(),
            "queuedFollowUp": follow_up.items.len(),
            "messages": messages.len()
        }),
        "set_thinking_level" => {
            if let Some(level) = command
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(ThinkingLevel::parse)
            {
                *thinking = level;
            }
            json!({"ok": true, "level": thinking.as_str()})
        }
        "get_available_thinking_levels" => json!({
            "ok": true,
            "levels": ["off","minimal","low","medium","high","xhigh","max"]
        }),
        "get_available_models" => {
            let models: Vec<_> = pi_ai::list_models(None)
                .into_iter()
                .take(50)
                .map(|m| json!({"provider": m.provider, "id": m.id, "name": m.name}))
                .collect();
            json!({"ok": true, "models": models})
        }
        "compact" => {
            let result = compact_messages(
                messages,
                command.get("customInstructions").and_then(|v| v.as_str()),
                4,
            );
            *messages = result.retained_tail;
            json!({"ok": true, "summary": result.summary})
        }
        "get_messages" => json!({"ok": true, "messages": messages}),
        "get_commands" => {
            let commands: Vec<_> = crate::slash::BUILTIN_SLASH_COMMANDS
                .iter()
                .map(|(name, description)| json!({"name": name, "description": description, "source": "extension"}))
                .collect();
            json!({"ok": true, "commands": commands})
        }
        "set_session_name" => json!({"ok": true, "name": command.get("name")}),
        "get_session_stats" => json!({"ok": true, "messages": messages.len()}),
        "bash" => {
            let command_text = command
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match tools.get("bash").map(|t| {
                t.execute(
                    &json!({"command": command_text}),
                    &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                )
            }) {
                Some(Ok(result)) => json!({"ok": !result.is_error, "output": result.output}),
                Some(Err(e)) => json!({"ok": false, "error": e.to_string()}),
                None => json!({"ok": false, "error": "bash tool disabled"}),
            }
        }
        other => json!({"ok": false, "error": format!("Unknown RPC command {other}")}),
    };
    let mut out = result;
    if let Some(id) = id {
        out.as_object_mut().unwrap().insert("id".into(), id);
    }
    out
}

use std::path::PathBuf;
