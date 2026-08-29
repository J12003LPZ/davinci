//! JSONL RPC mode matching `vendor/pi/packages/coding-agent/src/modes/rpc`.

use std::io::{BufRead, Write};

use pi_agent::{Agent, AgentContext, AgentEvent, AgentLoopConfig, QueueMode, ToolExecutionMode};
use pi_ai::{test_model, AssistantContent, Message, MockProvider};
use serde_json::{json, Value};

use crate::{create_coding_tools, with_cwd};

pub struct RpcSession {
    agent: Agent,
    session_id: String,
    session_name: Option<String>,
    thinking_level: String,
    steering_mode: QueueMode,
    follow_up_mode: QueueMode,
}

impl RpcSession {
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let context = AgentContext {
            system_prompt: Some("You are pi.".into()),
            messages: vec![],
            tools: create_coding_tools(cwd),
        };
        let config = AgentLoopConfig {
            model: test_model(),
            tool_execution: ToolExecutionMode::Sequential,
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        };
        Self {
            agent: Agent::new(config, MockProvider::default()).with_context(context),
            session_id: pi_core::next_id(),
            session_name: None,
            thinking_level: "off".into(),
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        }
    }

    pub fn handle_line(&mut self, line: &str) -> Vec<Value> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return vec![json!({
                    "type": "response",
                    "command": "unknown",
                    "success": false,
                    "error": error.to_string()
                })];
            }
        };
        self.handle_request(&request)
    }

    pub fn handle_request(&mut self, request: &Value) -> Vec<Value> {
        let id = request.get("id").cloned();
        let command = request
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let mut out = Vec::new();
        match command {
            "prompt" => {
                let message = request.get("message").and_then(Value::as_str).unwrap_or("");
                out.push(success(id.clone(), "prompt", None));
                out.extend(events_to_json(self.agent.prompt(message)));
            }
            "steer" => {
                let message = request.get("message").and_then(Value::as_str).unwrap_or("");
                self.agent.steer(message);
                out.push(success(id, "steer", None));
            }
            "follow_up" => {
                let message = request.get("message").and_then(Value::as_str).unwrap_or("");
                self.agent.follow_up(message);
                out.push(success(id, "follow_up", None));
            }
            "abort" => {
                self.agent.abort();
                out.push(success(id, "abort", None));
            }
            "clear_queue" => {
                out.push(success(
                    id,
                    "clear_queue",
                    Some(json!({"steering": [], "followUp": []})),
                ));
            }
            "new_session" => {
                *self = Self::new();
                out.push(success(
                    id,
                    "new_session",
                    Some(json!({"cancelled": false, "sessionId": self.session_id})),
                ));
            }
            "get_state" => out.push(success(id, "get_state", Some(self.state()))),
            "get_messages" => {
                out.push(success(
                    id,
                    "get_messages",
                    Some(json!({"messages": self.agent.messages()})),
                ));
            }
            "get_last_assistant_text" => {
                out.push(success(
                    id,
                    "get_last_assistant_text",
                    Some(json!({"text": last_assistant_text(self.agent.messages())})),
                ));
            }
            "get_session_stats" => {
                let count = self.agent.messages().len();
                out.push(success(
                    id,
                    "get_session_stats",
                    Some(json!({"messageCount": count})),
                ));
            }
            "set_session_name" => {
                self.session_name = request
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.push(success(id, "set_session_name", None));
            }
            "set_model" => {
                out.push(success(
                    id,
                    "set_model",
                    Some(serde_json::to_value(test_model()).unwrap_or(Value::Null)),
                ));
            }
            "get_available_models" => {
                out.push(success(
                    id,
                    "get_available_models",
                    Some(json!({"models": [test_model()]})),
                ));
            }
            "set_thinking_level" => {
                if let Some(level) = request.get("level").and_then(Value::as_str) {
                    self.thinking_level = level.to_string();
                }
                out.push(success(id, "set_thinking_level", None));
            }
            "get_available_thinking_levels" => {
                out.push(success(
                    id,
                    "get_available_thinking_levels",
                    Some(json!({"levels": ["off", "minimal", "low", "medium", "high"]})),
                ));
            }
            "set_steering_mode" => {
                self.steering_mode = parse_queue_mode(request.get("mode"));
                out.push(success(id, "set_steering_mode", None));
            }
            "set_follow_up_mode" => {
                self.follow_up_mode = parse_queue_mode(request.get("mode"));
                out.push(success(id, "set_follow_up_mode", None));
            }
            "bash" => {
                let command = request.get("command").and_then(Value::as_str).unwrap_or("");
                let result = crate::execute_bash(
                    &std::env::current_dir().unwrap_or_else(|_| ".".into()),
                    command,
                );
                match result {
                    Ok(text) => out.push(success(id, "bash", Some(json!({"output": text})))),
                    Err(error) => out.push(failure(id, "bash", &error)),
                }
            }
            "get_commands" => {
                out.push(success(
                    id,
                    "get_commands",
                    Some(json!({"commands": [
                        {"name": "exit", "description": "Leave the session"},
                        {"name": "help", "description": "Show slash commands"},
                        {"name": "clear", "description": "Clear the transcript"},
                        {"name": "sessions", "description": "List saved sessions"}
                    ]})),
                ));
            }
            "shutdown" => out.push(success(id, "shutdown", None)),
            other => out.push(failure(id, other, &format!("unknown command {other}"))),
        }
        out
    }

    fn state(&self) -> Value {
        json!({
            "model": test_model(),
            "thinkingLevel": self.thinking_level,
            "isStreaming": false,
            "isCompacting": false,
            "steeringMode": queue_mode_name(self.steering_mode),
            "followUpMode": queue_mode_name(self.follow_up_mode),
            "sessionId": self.session_id,
            "sessionName": self.session_name,
            "autoCompactionEnabled": false,
            "messageCount": self.agent.messages().len(),
            "pendingMessageCount": 0
        })
    }
}

impl Default for RpcSession {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_rpc(input: impl BufRead, mut output: impl Write) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    with_cwd(cwd, || {
        let mut session = RpcSession::new();
        for line in input.lines() {
            let line = line.map_err(|error| error.to_string())?;
            for value in session.handle_line(&line) {
                writeln!(output, "{}", serde_json::to_string(&value).unwrap())
                    .map_err(|error| error.to_string())?;
            }
            if line.contains("\"shutdown\"") {
                break;
            }
        }
        Ok(())
    })
}

pub fn run_rpc_lines(lines: &[&str]) -> Result<Vec<Value>, String> {
    let mut session = RpcSession::new();
    let mut out = Vec::new();
    for line in lines {
        out.extend(session.handle_line(line));
    }
    Ok(out)
}

fn success(id: Option<Value>, command: &str, data: Option<Value>) -> Value {
    let mut value = json!({
        "type": "response",
        "command": command,
        "success": true
    });
    if let Some(id) = id {
        value["id"] = id;
    }
    if let Some(data) = data {
        value["data"] = data;
    }
    value
}

fn failure(id: Option<Value>, command: &str, error: &str) -> Value {
    let mut value = json!({
        "type": "response",
        "command": command,
        "success": false,
        "error": error
    });
    if let Some(id) = id {
        value["id"] = id;
    }
    value
}

fn parse_queue_mode(value: Option<&Value>) -> QueueMode {
    match value.and_then(Value::as_str) {
        Some("one-at-a-time") => QueueMode::OneAtATime,
        _ => QueueMode::All,
    }
}

fn queue_mode_name(mode: QueueMode) -> &'static str {
    match mode {
        QueueMode::All => "all",
        QueueMode::OneAtATime => "one-at-a-time",
    }
}

fn last_assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::Assistant { content, .. } => content.iter().find_map(|block| match block {
                AssistantContent::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
}

fn events_to_json(events: Vec<AgentEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(|event| match event {
            AgentEvent::AgentStart => json!({"type": "agent_start"}),
            AgentEvent::TurnStart => json!({"type": "turn_start"}),
            AgentEvent::MessageStart { .. } => json!({"type": "message_start"}),
            AgentEvent::MessageUpdate { .. } => json!({"type": "message_update"}),
            AgentEvent::MessageEnd { .. } => json!({"type": "message_end"}),
            AgentEvent::ToolExecutionStart { name } => {
                json!({"type": "tool_execution_start", "name": name})
            }
            AgentEvent::ToolExecutionEnd { name, is_error } => {
                json!({"type": "tool_execution_end", "name": name, "isError": is_error})
            }
            AgentEvent::TurnEnd { .. } => json!({"type": "turn_end"}),
            AgentEvent::AgentEnd { .. } => json!({"type": "agent_end"}),
        })
        .collect()
}
