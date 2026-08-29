//! RPC stdin/stdout protocol matching TypeScript `rpc-types.ts`.

use crate::session_runtime::{to_json_event, SessionRuntime};
use crate::slash::BUILTIN_SLASH_COMMANDS;
use pi_agent::{QueueMode, ThinkingLevel};
use serde_json::{json, Value};

pub fn success(id: Option<&Value>, command: &str, data: Option<Value>) -> Value {
    let mut out = json!({
        "type": "response",
        "command": command,
        "success": true,
    });
    if let Some(id) = id {
        out["id"] = id.clone();
    }
    if let Some(data) = data {
        out["data"] = data;
    }
    out
}

pub fn error(id: Option<&Value>, command: &str, message: impl Into<String>) -> Value {
    let mut out = json!({
        "type": "response",
        "command": command,
        "success": false,
        "error": message.into(),
    });
    if let Some(id) = id {
        out["id"] = id.clone();
    }
    out
}

pub fn extension_ui_request(method: &str, fields: Value) -> Value {
    crate::extension_ui::ExtensionUiHost::request(method, fields)
}

fn emit_ui(runtime: &mut SessionRuntime, method: &str, fields: &Value) -> Value {
    runtime
        .ui
        .dispatch(method, fields)
        .unwrap_or_else(|| extension_ui_request(method, fields.clone()))
}

/// Handle one RPC command. Prompt returns `(response, events)` so the caller can stream.
pub fn handle_rpc(command: &Value, runtime: &mut SessionRuntime) -> (Value, Vec<Value>) {
    let id = command.get("id");
    let ty = command.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if ty == "extension_ui_response" {
        let data = runtime.ui.apply_response(command);
        return (success(id, "extension_ui_response", Some(data)), vec![]);
    }
    match ty {
        "prompt" => {
            let text = command
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let images = command
                .get("images")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if runtime.is_streaming {
                let message = pi_agent::AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images,
                };
                match command
                    .get("streamingBehavior")
                    .and_then(|v| v.as_str())
                    .unwrap_or("steer")
                {
                    "followUp" => runtime.follow_up.enqueue(message),
                    _ => runtime.steer.enqueue(message),
                }
                return (success(id, "prompt", None), vec![]);
            }
            match runtime.prompt(text, images) {
                Ok(events) => (
                    success(id, "prompt", None),
                    events.iter().map(to_json_event).collect(),
                ),
                Err(err) => (error(id, "prompt", err), vec![]),
            }
        }
        "steer" => {
            if let Some(text) = command.get("message").and_then(|v| v.as_str()) {
                runtime.steer.enqueue(pi_agent::AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images: command
                        .get("images")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                });
            }
            (success(id, "steer", None), vec![])
        }
        "follow_up" => {
            if let Some(text) = command.get("message").and_then(|v| v.as_str()) {
                runtime.follow_up.enqueue(pi_agent::AgentMessage {
                    role: "user".into(),
                    content: text.to_string(),
                    images: vec![],
                });
            }
            (success(id, "follow_up", None), vec![])
        }
        "abort" => {
            runtime.aborted = true;
            (success(id, "abort", None), vec![])
        }
        "clear_queue" => {
            let steering: Vec<_> = runtime
                .steer
                .items
                .iter()
                .map(|m| m.content.clone())
                .collect();
            let follow: Vec<_> = runtime
                .follow_up
                .items
                .iter()
                .map(|m| m.content.clone())
                .collect();
            runtime.steer.clear();
            runtime.follow_up.clear();
            (
                success(
                    id,
                    "clear_queue",
                    Some(json!({"steering": steering, "followUp": follow})),
                ),
                vec![],
            )
        }
        "new_session" => {
            match runtime.new_session(command.get("parentSession").and_then(|v| v.as_str())) {
                Ok(mut events) => {
                    if events.is_empty() {
                        events.push(runtime.ui.set_title("pi"));
                    }
                    (
                        success(id, "new_session", Some(json!({"cancelled": false}))),
                        events,
                    )
                }
                Err(err) => (error(id, "new_session", err), vec![]),
            }
        }
        "extension_ui" => {
            let method = command.get("method").and_then(|v| v.as_str()).unwrap_or("");
            match runtime.ui.dispatch(method, command) {
                Some(request) => (success(id, "extension_ui", None), vec![request]),
                None => (
                    error(id, "extension_ui", format!("Unknown command: {method}")),
                    vec![emit_ui(runtime, method, command)],
                ),
            }
        }
        "get_state" => (success(id, "get_state", Some(runtime.state())), vec![]),
        "set_model" => {
            let provider = command
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or(&runtime.provider)
                .to_string();
            let model_id = command
                .get("modelId")
                .and_then(|v| v.as_str())
                .unwrap_or(&runtime.model_id)
                .to_string();
            let model = runtime.set_model(&provider, &model_id);
            (success(id, "set_model", Some(model)), vec![])
        }
        "cycle_model" => match runtime.cycle_model() {
            Some(data) => (success(id, "cycle_model", Some(data)), vec![]),
            None => (success(id, "cycle_model", Some(json!(null))), vec![]),
        },
        "get_available_models" => (
            success(
                id,
                "get_available_models",
                Some(json!({"models": runtime.available_models()})),
            ),
            vec![],
        ),
        "set_thinking_level" => {
            if let Some(level) = command
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(ThinkingLevel::parse)
            {
                runtime.set_thinking(level);
            }
            (success(id, "set_thinking_level", None), vec![])
        }
        "cycle_thinking_level" => {
            runtime.set_thinking(runtime.thinking.cycle());
            (
                success(
                    id,
                    "cycle_thinking_level",
                    Some(json!({"level": runtime.thinking.as_str()})),
                ),
                vec![],
            )
        }
        "get_available_thinking_levels" => (
            success(
                id,
                "get_available_thinking_levels",
                Some(json!({
                    "levels": ThinkingLevel::all().iter().map(|l| l.as_str()).collect::<Vec<_>>()
                })),
            ),
            vec![],
        ),
        "set_steering_mode" => {
            if let Some(mode) = command
                .get("mode")
                .and_then(|v| v.as_str())
                .and_then(QueueMode::parse)
            {
                runtime.steer.mode = mode;
            }
            (success(id, "set_steering_mode", None), vec![])
        }
        "set_follow_up_mode" => {
            if let Some(mode) = command
                .get("mode")
                .and_then(|v| v.as_str())
                .and_then(QueueMode::parse)
            {
                runtime.follow_up.mode = mode;
            }
            (success(id, "set_follow_up_mode", None), vec![])
        }
        "compact" => {
            let data = runtime.compact(command.get("customInstructions").and_then(|v| v.as_str()));
            (success(id, "compact", Some(data)), vec![])
        }
        "set_auto_compaction" => {
            runtime.auto_compact = command
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(runtime.auto_compact);
            (success(id, "set_auto_compaction", None), vec![])
        }
        "set_auto_retry" => {
            runtime.auto_retry = command
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(runtime.auto_retry);
            (success(id, "set_auto_retry", None), vec![])
        }
        "abort_retry" => (success(id, "abort_retry", None), vec![]),
        "bash" => match runtime.bash(
            command
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        ) {
            Ok(data) => (success(id, "bash", Some(data)), vec![]),
            Err(err) => (error(id, "bash", err), vec![]),
        },
        "abort_bash" => (success(id, "abort_bash", None), vec![]),
        "get_session_stats" => (
            success(id, "get_session_stats", Some(runtime.stats())),
            vec![],
        ),
        "export_html" => {
            match runtime.export_html(command.get("outputPath").and_then(|v| v.as_str())) {
                Ok(path) => (
                    success(
                        id,
                        "export_html",
                        Some(json!({"path": path.display().to_string()})),
                    ),
                    vec![],
                ),
                Err(err) => (error(id, "export_html", err), vec![]),
            }
        }
        "switch_session" => {
            match runtime.switch_session(
                command
                    .get("sessionPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            ) {
                Ok(cancelled) => (
                    success(id, "switch_session", Some(json!({"cancelled": cancelled}))),
                    vec![],
                ),
                Err(err) => (error(id, "switch_session", err), vec![]),
            }
        }
        "fork" => match runtime.fork(
            command
                .get("entryId")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        ) {
            Ok(data) => (success(id, "fork", Some(data)), vec![]),
            Err(err) => (error(id, "fork", err), vec![]),
        },
        "clone" => match runtime.clone_current() {
            Ok(data) => (success(id, "clone", Some(data)), vec![]),
            Err(err) => (error(id, "clone", err), vec![]),
        },
        "get_fork_messages" => (
            success(
                id,
                "get_fork_messages",
                Some(json!({"messages": runtime.fork_user_messages()})),
            ),
            vec![],
        ),
        "get_entries" => (
            success(
                id,
                "get_entries",
                Some(runtime.entries_since(command.get("since").and_then(|v| v.as_str()))),
            ),
            vec![],
        ),
        "get_tree" => (success(id, "get_tree", Some(runtime.tree())), vec![]),
        "get_last_assistant_text" => (
            success(
                id,
                "get_last_assistant_text",
                Some(json!({"text": runtime.last_assistant()})),
            ),
            vec![],
        ),
        "set_session_name" => {
            if let Some(name) = command.get("name").and_then(|v| v.as_str()) {
                runtime.set_name(name);
            }
            (success(id, "set_session_name", None), vec![])
        }
        "get_messages" => (
            success(
                id,
                "get_messages",
                Some(json!({"messages": runtime.messages})),
            ),
            vec![],
        ),
        "get_commands" => {
            let mut commands: Vec<_> = runtime
                .registry
                .commands
                .iter()
                .map(|command| {
                    json!({
                        "name": command.name,
                        "description": command.description,
                        "source": "extension",
                        "sourceInfo": {
                            "path": command.path.display().to_string(),
                            "source": command.path.display().to_string(),
                            "scope": "project",
                            "origin": "top-level"
                        }
                    })
                })
                .collect();
            commands.extend(BUILTIN_SLASH_COMMANDS.iter().map(|(name, description)| {
                json!({
                    "name": name,
                    "description": description,
                    "source": "prompt",
                    "sourceInfo": {
                        "path": *name,
                        "source": *name,
                        "scope": "temporary",
                        "origin": "top-level"
                    }
                })
            }));
            (
                success(id, "get_commands", Some(json!({"commands": commands}))),
                vec![],
            )
        }
        "extension_tool" => {
            let path = command.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = command.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let input = command.get("input").cloned().unwrap_or(json!({}));
            match crate::extensions::invoke_extension_tool(std::path::Path::new(path), name, &input)
            {
                Ok(result) => (success(id, "extension_tool", Some(result)), vec![]),
                Err(err) => (error(id, "extension_tool", err), vec![]),
            }
        }
        other => (
            error(id, other, format!("Unknown command: {other}")),
            vec![],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;
    use pi_agent::ToolRegistry;
    use std::path::PathBuf;

    fn runtime() -> SessionRuntime {
        SessionRuntime {
            cwd: PathBuf::from("."),
            provider: "google".into(),
            model_id: "gemini-2.5-flash".into(),
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
            registry: crate::extensions::ExtensionRegistry::default(),
            theme: "dark".into(),
            flag_values: Default::default(),
            pending_custom_lines: Vec::new(),
            pending_next_turn: Vec::new(),
            pending_custom_messages: Vec::new(),
            pending_trigger_turn: false,
            running_turn: false,
            last_extension_turn_events: Vec::new(),
        }
    }

    #[test]
    fn envelope_and_state() {
        let mut rt = runtime();
        let (reply, events) = handle_rpc(&json!({"type":"get_state","id":"1"}), &mut rt);
        assert_eq!(reply["type"], "response");
        assert_eq!(reply["command"], "get_state");
        assert_eq!(reply["success"], true);
        assert_eq!(reply["id"], "1");
        assert_eq!(reply["data"]["sessionId"], "sess");
        assert!(events.is_empty());
        let (reply, _) = handle_rpc(&json!({"type":"cycle_thinking_level"}), &mut rt);
        assert_eq!(reply["data"]["level"], "minimal");
        let (reply, _) = handle_rpc(&json!({"type":"get_commands"}), &mut rt);
        assert!(reply["data"]["commands"].as_array().unwrap().len() >= 20);
        let (reply, _) = handle_rpc(&json!({"type":"unknown_cmd"}), &mut rt);
        assert_eq!(reply["success"], false);
        assert_eq!(reply["error"], "Unknown command: unknown_cmd");
        rt.is_streaming = true;
        let (reply, events) = handle_rpc(
            &json!({"type":"prompt","message":"later","streamingBehavior":"followUp"}),
            &mut rt,
        );
        assert_eq!(reply["success"], true);
        assert!(events.is_empty());
        assert_eq!(rt.follow_up.items[0].content, "later");
    }

    #[test]
    fn extension_ui_request_and_response() {
        let mut rt = runtime();
        let select = rt
            .ui
            .select("Dangerous command", &["Allow".into(), "Block".into()], None);
        assert_eq!(select["type"], "extension_ui_request");
        assert_eq!(select["method"], "select");
        let id = select["id"].clone();
        let (reply, _) = handle_rpc(
            &json!({"type":"extension_ui_response","id": id, "value":"Allow"}),
            &mut rt,
        );
        assert_eq!(reply["success"], true);
        assert_eq!(reply["data"]["value"], "Allow");

        let confirm = rt
            .ui
            .confirm("Clear session?", "All messages will be lost.", None);
        let id = confirm["id"].clone();
        let (reply, _) = handle_rpc(
            &json!({"type":"extension_ui_response","id": id, "confirmed": true}),
            &mut rt,
        );
        assert_eq!(reply["data"]["confirmed"], true);

        let (reply, events) = handle_rpc(
            &json!({"type":"extension_ui","method":"notify","message":"hello","notifyType":"info"}),
            &mut rt,
        );
        assert_eq!(reply["success"], true);
        assert_eq!(events[0]["method"], "notify");
        assert_eq!(
            extension_ui_request("notify", json!({"message": "hello"}))["method"],
            "notify"
        );
        let status = rt.ui.set_status("rpc-demo", Some("Turns: 1"));
        assert_eq!(status["method"], "setStatus");
        let widget = rt
            .ui
            .set_widget("rpc-demo", Some(&["ready".into()]), Some("belowEditor"));
        assert_eq!(widget["method"], "setWidget");
        assert_eq!(rt.ui.set_editor_text("hi")["method"], "set_editor_text");
    }
}
