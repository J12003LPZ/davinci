use std::path::{Path, PathBuf};

use davinci_agent::{Agent, QueueMode};
#[cfg(test)]
use davinci_ai::load_builtin_models;
use davinci_ai::{available_thinking_levels, cycle_thinking_level, find_model, Model};
use davinci_protocol::ThinkingLevel;
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::export;
use crate::model_resolver::{
    resolve_model_scope_from_models, thinking_level_for_model_switch, ScopedModelRef,
};

pub const COMPACTION_PROMPT_ERROR: &str =
    "Cannot submit a prompt while compaction is in progress. Wait for compaction to finish and retry.";
pub const STREAMING_PROMPT_ERROR: &str =
    "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message.";

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
    #[serde(rename = "agentId", default)]
    pub agent_id: Option<String>,
    #[serde(rename = "action", default)]
    pub action: Option<String>,
    #[serde(rename = "operationId", default)]
    pub operation_id: Option<String>,
    #[serde(rename = "expectedRevision", default)]
    pub expected_revision: Option<u64>,
    #[serde(rename = "expectedGeneration", default)]
    pub expected_generation: Option<u64>,
    #[serde(rename = "decisionId", default)]
    pub decision_id: Option<String>,
    #[serde(rename = "choiceId", default)]
    pub choice_id: Option<String>,
    #[serde(rename = "runId", default)]
    pub run_id: Option<String>,
    #[serde(rename = "nodeId", default)]
    pub node_id: Option<String>,
    #[serde(rename = "expectedAttempt", default)]
    pub expected_attempt: Option<u32>,
    #[serde(rename = "control", default)]
    pub control: Option<crate::native_extensions::graph::control::GraphControl>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcSessionEvent {
    QueueUpdate {
        steering: Vec<String>,
        #[serde(rename = "followUp")]
        follow_up: Vec<String>,
    },
    CompactionStart {
        reason: String,
    },
    CompactionEnd {
        reason: String,
        result: Option<Value>,
        aborted: bool,
        #[serde(rename = "willRetry")]
        will_retry: bool,
        #[serde(rename = "errorMessage", skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    SessionInfoChanged {
        name: Option<String>,
    },
    ThinkingLevelChanged {
        level: ThinkingLevel,
    },
    AgentSettled,
    EntryAppended {
        entry: Value,
    },
}

pub struct RpcRuntime {
    pub agent: Agent,
    pub session_dir: PathBuf,
    pub cwd: PathBuf,
    pub models: Vec<Model>,
    pub scoped_models: Vec<Model>,
    pub scoped_thinking: std::collections::BTreeMap<String, ThinkingLevel>,
    pub model_thinking_levels: std::collections::BTreeMap<String, ThinkingLevel>,
    pub default_thinking_level: Option<ThinkingLevel>,
    pub bash_aborted: bool,
    pub invocable_commands: Vec<Value>,
    pub pending_ui: std::collections::HashMap<String, RpcCommand>,
    pub pending_events: Vec<RpcSessionEvent>,
    pub prompt_needs_turn: bool,
    pub control_operations: std::collections::HashMap<String, Value>,
}

impl RpcRuntime {
    #[cfg(test)]
    pub fn new(agent: Agent, session_dir: PathBuf, cwd: PathBuf) -> Self {
        Self::with_models(agent, session_dir, cwd, load_builtin_models())
    }

    pub fn with_models(
        agent: Agent,
        session_dir: PathBuf,
        cwd: PathBuf,
        models: Vec<Model>,
    ) -> Self {
        Self {
            agent,
            session_dir,
            cwd,
            models,
            scoped_models: Vec::new(),
            scoped_thinking: std::collections::BTreeMap::new(),
            model_thinking_levels: std::collections::BTreeMap::new(),
            default_thinking_level: None,
            bash_aborted: false,
            invocable_commands: Vec::new(),
            pending_ui: std::collections::HashMap::new(),
            pending_events: Vec::new(),
            prompt_needs_turn: false,
            control_operations: std::collections::HashMap::new(),
        }
    }

    pub fn take_events(&mut self) -> Vec<RpcSessionEvent> {
        std::mem::take(&mut self.pending_events)
    }

    fn emit(&mut self, event: RpcSessionEvent) {
        self.pending_events.push(event);
    }

    fn emit_queue_update(&mut self) {
        let steering = self
            .agent
            .queues
            .steer
            .iter()
            .map(|message| message.text.clone())
            .collect();
        let follow_up = self
            .agent
            .queues
            .follow_up
            .iter()
            .map(|message| message.text.clone())
            .collect();
        self.emit(RpcSessionEvent::QueueUpdate {
            steering,
            follow_up,
        });
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

    pub fn set_scoped_models(&mut self, patterns: &[String]) {
        let resolved = resolve_model_scope_from_models(patterns, &self.models);
        self.scoped_thinking = resolved
            .scoped_models
            .iter()
            .filter_map(|item| {
                item.thinking_level
                    .map(|level| (format!("{}/{}", item.model.provider, item.model.id), level))
            })
            .collect();
        self.scoped_models = resolve_scoped_models(&self.models, patterns);
    }

    fn apply_thinking_for_switch(&mut self, provider: &str, model_id: &str) {
        let key = format!("{provider}/{model_id}");
        self.agent.thinking_level = thinking_level_for_model_switch(
            self.scoped_thinking.get(&key).copied(),
            self.model_thinking_levels.get(&key).copied(),
            self.default_thinking_level,
            self.agent.thinking_level,
        );
    }
}

pub fn resolve_scoped_models(models: &[Model], patterns: &[String]) -> Vec<Model> {
    resolve_model_scope_from_models(patterns, models)
        .scoped_models
        .into_iter()
        .map(|item: ScopedModelRef| item.model)
        .collect()
}

pub fn handle_rpc(runtime: &mut RpcRuntime, command: RpcCommand) -> RpcResponse {
    let id = command.id.clone();
    let kind = command.kind.clone();
    match kind.as_str() {
        "prompt" => {
            runtime.prompt_needs_turn = false;
            let images = command
                .images
                .as_deref()
                .map(davinci_agent::parse_rpc_images)
                .unwrap_or_default();
            if runtime.agent.is_compacting {
                return fail(id, &kind, COMPACTION_PROMPT_ERROR.to_string());
            }
            if runtime.agent.is_streaming {
                let Some(message) = command.message.as_deref() else {
                    return fail(id, &kind, STREAMING_PROMPT_ERROR.to_string());
                };
                let text = davinci_agent::expand_user_text(
                    message,
                    &runtime.agent.skills,
                    &runtime.agent.templates,
                );
                match command.streaming_behavior.as_deref() {
                    Some("followUp") => {
                        runtime.agent.queues.follow_up_mode = QueueMode::All;
                        runtime.agent.queues.enqueue_follow_up_with(&text, images);
                        runtime.emit_queue_update();
                        return ok(id, &kind, None);
                    }
                    Some("steer") => {
                        runtime.agent.queues.steer_mode = QueueMode::All;
                        runtime.agent.queues.enqueue_steer_with(&text, images);
                        runtime.emit_queue_update();
                        return ok(id, &kind, None);
                    }
                    _ => return fail(id, &kind, STREAMING_PROMPT_ERROR.to_string()),
                }
            }
            if let Some(message) = &command.message {
                let text = davinci_agent::expand_user_text(
                    message,
                    &runtime.agent.skills,
                    &runtime.agent.templates,
                );
                runtime.agent.prompt_user_with(&text, &images);
                runtime.prompt_needs_turn = true;
            }
            ok(id, &kind, None)
        }
        "steer" => {
            let images = command
                .images
                .as_deref()
                .map(davinci_agent::parse_rpc_images)
                .unwrap_or_default();
            if let Some(message) = &command.message {
                let text = davinci_agent::expand_user_text(
                    message,
                    &runtime.agent.skills,
                    &runtime.agent.templates,
                );
                runtime.agent.queues.enqueue_steer_with(&text, images);
            }
            runtime.emit_queue_update();
            ok(id, &kind, None)
        }
        "follow_up" => {
            let images = command
                .images
                .as_deref()
                .map(davinci_agent::parse_rpc_images)
                .unwrap_or_default();
            if let Some(message) = &command.message {
                let text = davinci_agent::expand_user_text(
                    message,
                    &runtime.agent.skills,
                    &runtime.agent.templates,
                );
                runtime.agent.queues.enqueue_follow_up_with(&text, images);
            }
            runtime.emit_queue_update();
            ok(id, &kind, None)
        }
        "abort" => {
            runtime.agent.abort();
            ok(id, &kind, None)
        }
        "clear_queue" => {
            let (steering, follow_up) = runtime.agent.queues.clear();
            runtime.emit_queue_update();
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
                runtime.emit(RpcSessionEvent::ThinkingLevelChanged { level });
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
            runtime.emit(RpcSessionEvent::CompactionStart {
                reason: "manual".into(),
            });
            let result = runtime
                .agent
                .compact(command.custom_instructions.as_deref());
            runtime.emit(RpcSessionEvent::CompactionEnd {
                reason: "manual".into(),
                result: Some(serde_json::to_value(&result).unwrap_or(Value::Null)),
                aborted: false,
                will_retry: false,
                error_message: None,
            });
            if let Some(entry) = runtime
                .agent
                .session
                .as_ref()
                .and_then(|session| session.entries.last())
            {
                runtime.emit(RpcSessionEvent::EntryAppended {
                    entry: serde_json::to_value(entry).unwrap_or(Value::Null),
                });
            }
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
            let changed = if let (Some(session), Some(name)) =
                (runtime.agent.session.as_mut(), command.name.as_deref())
            {
                let _ = session.set_name(name);
                session.display_name()
            } else {
                None
            };
            if changed.is_some() {
                runtime.emit(RpcSessionEvent::SessionInfoChanged { name: changed });
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
            let bash_command = command.command.unwrap_or_default();
            match davinci_agent::execute_tool(
                &runtime.cwd,
                "bash",
                &serde_json::json!({
                    "command": bash_command,
                    "excludeFromContext": exclude
                }),
            ) {
                Ok(result) => {
                    let bash_result = serde_json::json!({
                        "output": result.content,
                        "exitCode": result
                            .details
                            .as_ref()
                            .and_then(|details| details.get("exitCode"))
                            .cloned()
                            .unwrap_or(Value::Null),
                        "cancelled": false,
                        "truncated": false,
                    });
                    runtime
                        .agent
                        .record_bash_result(&bash_command, &bash_result, exclude);
                    ok(id, &kind, Some(bash_result))
                }
                Err(err) => fail(id, &kind, err.to_string()),
            }
        }
        "get_available_thinking_levels" => {
            let model = runtime.current_model();
            ok(
                id,
                &kind,
                Some(serde_json::json!({
                    "levels": available_thinking_levels(model)
                })),
            )
        }
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
                fail(id, &kind, format!("Model not found: {provider}/{model_id}"))
            }
        }
        "cycle_model" => {
            let scoped = !runtime.scoped_models.is_empty();
            let pool: Vec<Model> = if scoped {
                runtime.scoped_models.clone()
            } else {
                runtime.models.clone()
            };
            if pool.len() <= 1 {
                return ok(id, &kind, Some(Value::Null));
            }
            let current = pool
                .iter()
                .position(|model| {
                    model.provider == runtime.agent.provider && model.id == runtime.agent.model_id
                })
                .unwrap_or(0);
            let next = pool[(current + 1) % pool.len()].clone();
            runtime.apply_thinking_for_switch(&next.provider, &next.id);
            runtime.agent.provider = next.provider.clone();
            runtime.agent.model_id = next.id.clone();
            ok(
                id,
                &kind,
                Some(serde_json::json!({
                    "model": RpcRuntime::model_json(&next),
                    "thinkingLevel": runtime.agent.thinking_level,
                    "isScoped": scoped,
                })),
            )
        }
        "get_available_models" => ok(
            id,
            &kind,
            Some(serde_json::json!({ "models": runtime.models })),
        ),
        "cycle_thinking_level" => {
            match cycle_thinking_level(runtime.current_model(), runtime.agent.thinking_level) {
                Some(next) => {
                    runtime.agent.thinking_level = next;
                    ok(id, &kind, Some(serde_json::json!({ "level": next })))
                }
                None => ok(id, &kind, Some(Value::Null)),
            }
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
            match davinci_session::entries_since(entries, command.since.as_deref()) {
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
                    "tree": davinci_session::build_session_tree(entries),
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
                    "messages": davinci_session::fork_user_messages(entries)
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
        "decision_response" => {
            if let Some(response_id) = id.clone() {
                runtime.pending_ui.insert(response_id, command);
            }
            ok(id, &kind, None)
        }
        "agent_control" => {
            let Some(agent_id_str) = command.agent_id.as_deref() else {
                return fail(id, &kind, "Missing agentId parameter".into());
            };
            let Ok(agent_id) = agent_id_str.parse::<davinci_agent::runtime::AgentId>() else {
                return fail(id, &kind, format!("Invalid agentId: {agent_id_str}"));
            };
            let action_name = command.action.as_deref().unwrap_or("inspect");
            let Some(runtime_handle) = &runtime.agent.runtime else {
                return fail(id, &kind, "No active runtime handle available".into());
            };
            let controller =
                davinci_agent::runtime::WorkerController::new(runtime_handle.registry.clone());

            let op_id = command
                .operation_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            match action_name {
                "inspect" => {
                    let snapshot = controller.build_snapshot(&agent_id);
                    ok(
                        id,
                        &kind,
                        Some(serde_json::json!({
                            "operationId": op_id,
                            "snapshot": snapshot,
                            "completed": true,
                        })),
                    )
                }
                "diff" => {
                    let baseline_res =
                        crate::native_extensions::graph::mutation::capture_baseline(&runtime.cwd);
                    match baseline_res {
                        Ok(baseline) => {
                            match crate::native_extensions::graph::mutation::compute_owned_diff(
                                &runtime.cwd,
                                &baseline,
                                &[],
                            ) {
                                Ok(report) => ok(
                                    id,
                                    &kind,
                                    Some(serde_json::json!({
                                        "operationId": op_id,
                                        "diff": report,
                                        "completed": true,
                                    })),
                                ),
                                Err(err) => {
                                    fail(id, &kind, format!("Failed to compute diff: {err}"))
                                }
                            }
                        }
                        Err(err) => fail(id, &kind, format!("Failed to capture baseline: {err}")),
                    }
                }
                "stop" | "retry" | "steer" => {
                    let action = match action_name {
                        "stop" => davinci_agent::runtime::WorkerControlAction::Stop {
                            reason: command.message.clone(),
                        },
                        "retry" => davinci_agent::runtime::WorkerControlAction::Retry {
                            reason: command.message.clone(),
                        },
                        _ => davinci_agent::runtime::WorkerControlAction::Steer {
                            message: command.message.clone().unwrap_or_default(),
                            redirect: true,
                        },
                    };
                    let generation = command
                        .expected_generation
                        .unwrap_or_else(|| runtime_handle.registry.get_generation(&agent_id));
                    let expected_revision = command
                        .expected_revision
                        .unwrap_or_else(|| runtime_handle.registry.get_revision(&agent_id));
                    let cmd = davinci_agent::runtime::WorkerControlCommand {
                        id: uuid::Uuid::new_v4(),
                        root_run_id: runtime_handle.run_id,
                        agent_id,
                        generation,
                        task_id: None,
                        expected_revision,
                        action,
                    };
                    let receipt = controller.execute_command(cmd, true);
                    let is_completed = receipt.status
                        == davinci_agent::runtime::ControlStatus::Stopped
                        || receipt.status == davinci_agent::runtime::ControlStatus::Rejected;
                    let result_val = serde_json::json!({
                        "operationId": op_id,
                        "agentId": agent_id.to_string(),
                        "status": receipt.status.as_str(),
                        "action": receipt.action,
                        "completed": is_completed,
                        "reason": receipt.reason,
                    });
                    runtime.control_operations.insert(op_id, result_val.clone());
                    ok(id, &kind, Some(result_val))
                }
                other => fail(id, &kind, format!("Unknown control action: {other}")),
            }
        }
        "get_worker_snapshots" | "agent_snapshots" => {
            let snapshots = if let Some(runtime_handle) = &runtime.agent.runtime {
                let controller =
                    davinci_agent::runtime::WorkerController::new(runtime_handle.registry.clone());
                controller.build_all_snapshots()
            } else {
                Vec::new()
            };
            ok(
                id,
                &kind,
                Some(serde_json::json!({ "snapshots": snapshots })),
            )
        }
        "poll_control_operation" => {
            let Some(op_id) = command.operation_id.as_deref() else {
                return fail(id, &kind, "Missing operationId parameter".into());
            };
            if let Some(data) = runtime.control_operations.get(op_id) {
                ok(id, &kind, Some(data.clone()))
            } else {
                fail(id, &kind, format!("Operation not found: {op_id}"))
            }
        }
        "graph_control" => {
            let ctrl = if let Some(ctrl) = command.control {
                ctrl
            } else {
                let Some(run_id) = command.run_id.clone() else {
                    return fail(
                        id,
                        &kind,
                        "Missing runId parameter for graph_control".into(),
                    );
                };
                let action = match command.action.as_deref().unwrap_or("pause") {
                    "pause" => crate::native_extensions::graph::control::GraphControlAction::Pause,
                    "resume" => {
                        crate::native_extensions::graph::control::GraphControlAction::Resume
                    }
                    "stop_node" => {
                        crate::native_extensions::graph::control::GraphControlAction::StopNode
                    }
                    "stop_graph" | "stop" => {
                        crate::native_extensions::graph::control::GraphControlAction::StopGraph
                    }
                    "retry_node" | "retry" => {
                        crate::native_extensions::graph::control::GraphControlAction::RetryNode
                    }
                    other => {
                        return fail(id, &kind, format!("Unknown graph control action: {other}"))
                    }
                };
                crate::native_extensions::graph::control::GraphControl {
                    operation_id: command
                        .operation_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    run_id,
                    expected_run_revision: command.expected_revision.unwrap_or(0),
                    node_id: command.node_id.clone(),
                    expected_attempt: command.expected_attempt,
                    action,
                }
            };

            let ctrl_json = match serde_json::to_string(&ctrl) {
                Ok(j) => j,
                Err(e) => {
                    return fail(
                        id,
                        &kind,
                        format!("Failed to serialize control command: {e}"),
                    )
                }
            };

            let controller =
                crate::native_extensions::graph::GraphController::new(PathBuf::from(&runtime.cwd));
            match controller.command("graph-control", &ctrl_json) {
                Ok(Some(receipt_val)) => ok(id, &kind, Some(receipt_val)),
                Ok(None) => fail(id, &kind, "No receipt returned for graph control".into()),
                Err(err) => fail(id, &kind, err),
            }
        }

        other => fail(id, other, format!("Unknown RPC command: {other}")),
    }
}

fn with_optional_timeout(mut extra: Value, call: &Value) -> Value {
    if let Some(timeout) = call.get("timeout") {
        if let Some(object) = extra.as_object_mut() {
            object.insert("timeout".into(), timeout.clone());
        }
    }
    extra
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
                    with_optional_timeout(
                        serde_json::json!({
                            "title": call.get("title"),
                            "options": call.get("options"),
                        }),
                        call,
                    ),
                ),
                "confirm" => extension_ui_request(
                    &id,
                    "confirm",
                    with_optional_timeout(
                        serde_json::json!({
                            "title": call.get("title"),
                            "message": call.get("message"),
                        }),
                        call,
                    ),
                ),
                "input" => extension_ui_request(
                    &id,
                    "input",
                    with_optional_timeout(
                        serde_json::json!({
                            "title": call.get("title"),
                            "placeholder": call.get("placeholder"),
                        }),
                        call,
                    ),
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
    next.permissions = runtime.agent.permissions.clone();
    next.approver = runtime.agent.approver.clone();
    next.approval_responder = runtime.agent.approval_responder.clone();
    next.load_from_session(session)?;
    runtime.agent.reset_session_approvals();
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
    runtime.agent.load_from_session(cloned)?;
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
    runtime.agent.load_from_session(forked)?;
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
    runtime.agent.load_from_session(session)?;
    Ok(false)
}

pub fn session_stats_json(runtime: &RpcRuntime) -> Value {
    session_stats_for_agent(&runtime.agent, runtime.current_model())
}

pub fn session_stats_for_agent(agent: &Agent, model: Option<&Model>) -> Value {
    let session = agent.session.as_ref();
    let entries = session.map(|item| item.entries.as_slice()).unwrap_or(&[]);
    let stats = davinci_session::session_usage_stats(entries);
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
        "runtime": agent.run_stats(),
    })
}

fn context_usage_json(
    entries: &[davinci_session::SessionEntry],
    leaf_id: Option<&str>,
    model: Option<&Model>,
) -> Option<Value> {
    let model = model?;
    let context_window = model.context_window;
    if context_window == 0 {
        return None;
    }
    let branch = davinci_session::branch_entries(entries, leaf_id);
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
    let tokens = davinci_session::session_usage_stats(
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

pub fn ok_response(id: Option<String>, command: &str, data: Option<Value>) -> RpcResponse {
    ok(id, command, data)
}

pub fn fail_response(id: Option<String>, command: &str, error: String) -> RpcResponse {
    fail(id, command, error)
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

#[allow(dead_code)]
pub fn approval_host_result(
    interactive: bool,
    valid_reply: bool,
    disconnected: bool,
) -> &'static str {
    if disconnected {
        "denied"
    } else if valid_reply {
        "resolved"
    } else if interactive {
        "pending"
    } else {
        "approval_required"
    }
}

pub fn rpc_resolve_decision(
    request: &davinci_agent::DecisionHostRequest,
    mut ask: impl FnMut(&serde_json::Value) -> serde_json::Value,
) -> davinci_agent::DecisionHostResponse {
    let issued_request_id = format!("dec-req-{}", davinci_session::now_ms());
    let question = &request.question;

    let payload = serde_json::json!({
        "op": "decision",
        "id": issued_request_id,
        "question": {
            "id": question.id,
            "kind": question.kind,
            "title": question.title,
            "question": question.question,
            "materiality": question.materiality,
            "evidence_refs": question.evidence_refs,
            "options": question.options,
            "allow_custom": question.allow_custom,
            "custom_only": question.custom_only,
            "plan_revision": question.plan_revision,
        }
    });

    let answer = ask(&payload);
    if answer.is_null() {
        // Older clients may use sequential select/input but must preserve question/revision/request identity
        let mut options: Vec<String> = question.options.iter().map(|o| o.label.clone()).collect();
        if question.allow_custom {
            options.push("Custom response".into());
        }
        options.push("Defer decision".into());
        options.push("Cancel".into());

        let select_call = serde_json::json!({
            "op": "select",
            "id": issued_request_id,
            "title": format!("{}: {}", question.title, question.question),
            "options": options,
            "decision_id": question.id,
            "plan_revision": question.plan_revision,
        });
        let sel_ans = ask(&select_call);
        if let Some(choice_str) = sel_ans.as_str() {
            if choice_str == "Cancel" {
                return davinci_agent::DecisionHostResponse::Cancelled;
            }
            if choice_str == "Defer decision" {
                return davinci_agent::DecisionHostResponse::Reply(
                    davinci_agent::DecisionHostReply {
                        action: davinci_agent::decisions::HostDecisionAction::Defer,
                        host_event_id: format!("rpc-evt-{}", davinci_session::now_ms()),
                        answered_at_ms: davinci_session::now_ms(),
                    },
                );
            }
            if choice_str == "Custom response" && question.allow_custom {
                let input_call = serde_json::json!({
                    "op": "input",
                    "id": issued_request_id,
                    "title": "Enter custom response",
                    "placeholder": "Custom answer...",
                    "decision_id": question.id,
                    "plan_revision": question.plan_revision,
                });
                let input_ans = ask(&input_call);
                if let Some(custom_text) = input_ans.as_str() {
                    let trimmed = custom_text.trim();
                    if !trimmed.is_empty() {
                        return davinci_agent::DecisionHostResponse::Reply(
                            davinci_agent::DecisionHostReply {
                                action: davinci_agent::decisions::HostDecisionAction::AnswerCustom(
                                    trimmed.to_string(),
                                ),
                                host_event_id: format!("rpc-evt-{}", davinci_session::now_ms()),
                                answered_at_ms: davinci_session::now_ms(),
                            },
                        );
                    }
                }
                return davinci_agent::DecisionHostResponse::Cancelled;
            }
            if let Some(opt) = question
                .options
                .iter()
                .find(|o| o.label == choice_str || o.id == choice_str)
            {
                return davinci_agent::DecisionHostResponse::Reply(
                    davinci_agent::DecisionHostReply {
                        action: davinci_agent::decisions::HostDecisionAction::AnswerChoice(
                            opt.id.clone(),
                        ),
                        host_event_id: format!("rpc-evt-{}", davinci_session::now_ms()),
                        answered_at_ms: davinci_session::now_ms(),
                    },
                );
            }
        }
        return davinci_agent::DecisionHostResponse::Unavailable;
    }

    if let Some(action_str) = answer.get("action").and_then(|a| a.as_str()) {
        let host_event_id = answer
            .get("host_event_id")
            .and_then(|h| h.as_str())
            .unwrap_or(&issued_request_id)
            .to_string();
        let answered_at_ms = answer
            .get("answered_at_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or_else(davinci_session::now_ms);
        match action_str {
            "answer_choice" => {
                if let Some(choice) = answer.get("choice_id").and_then(|c| c.as_str()) {
                    return davinci_agent::DecisionHostResponse::Reply(
                        davinci_agent::DecisionHostReply {
                            action: davinci_agent::decisions::HostDecisionAction::AnswerChoice(
                                choice.to_string(),
                            ),
                            host_event_id,
                            answered_at_ms,
                        },
                    );
                }
            }
            "answer_custom" => {
                if let Some(custom) = answer.get("custom_text").and_then(|c| c.as_str()) {
                    return davinci_agent::DecisionHostResponse::Reply(
                        davinci_agent::DecisionHostReply {
                            action: davinci_agent::decisions::HostDecisionAction::AnswerCustom(
                                custom.to_string(),
                            ),
                            host_event_id,
                            answered_at_ms,
                        },
                    );
                }
            }
            "defer" => {
                return davinci_agent::DecisionHostResponse::Reply(
                    davinci_agent::DecisionHostReply {
                        action: davinci_agent::decisions::HostDecisionAction::Defer,
                        host_event_id,
                        answered_at_ms,
                    },
                );
            }
            "cancel" => {
                return davinci_agent::DecisionHostResponse::Cancelled;
            }
            _ => {}
        }
    }
    davinci_agent::DecisionHostResponse::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::PromptProfile;
    use davinci_session::JsonlSession;
    use tempfile::tempdir;

    #[test]
    fn f01_noninteractive_fail_closed_contract() {
        assert_eq!(
            approval_host_result(false, false, false),
            "approval_required"
        );
        assert_eq!(approval_host_result(true, false, true), "denied");
        assert_eq!(approval_host_result(true, true, false), "resolved");
    }

    #[test]
    fn f03_rpc_session_switch_reports_recovery_without_switching() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let first_path = first.path.clone();
        agent.load_from_session(first).unwrap();
        let next = JsonlSession::create(dir.path(), "fixture", None).unwrap();
        std::fs::write(
            davinci_session::runtime_log_path(&next.path),
            b"{bad}\n{bad}\n",
        )
        .unwrap();
        let mut runtime = RpcRuntime::new(agent, dir.path().into(), dir.path().into());
        let response = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "switch_session".into(),
                session_path: Some(next.path.to_string_lossy().into_owned()),
                ..RpcCommand::default()
            },
        );
        assert!(!response.success);
        assert!(response
            .error
            .unwrap()
            .starts_with("Runtime recovery required:"));
        assert_eq!(runtime.agent.session.as_ref().unwrap().path, first_path);
    }

    #[test]
    fn f01_rpc_new_session_preserves_policy_and_host_but_clears_consent() {
        use davinci_agent::{PermissionMode, PermissionRule, ToolApprovalDecision, ToolApprover};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new_builtin(PromptProfile::Stable);
        agent.set_permission_mode(PermissionMode::Ask);
        {
            let mut policy = agent.permissions.lock().unwrap();
            policy.project_trusted = true;
            policy
                .deny
                .push(PermissionRule::subject("write", "protected.txt"));
            policy
                .session_allow
                .push(PermissionRule::subject("write", "ordinary.txt"));
        }
        agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::Deny)));
        agent.approval_responder = Some(davinci_agent::approval::ApprovalResponder(Arc::new(
            |_, challenge| {
                davinci_agent::approval::ApprovalReply::from_legacy(
                    challenge,
                    ToolApprovalDecision::Deny,
                )
            },
        )));
        let permissions = agent.permissions.clone();
        let mut runtime = RpcRuntime::new(agent, dir.path().into(), dir.path().into());
        assert!(!create_session(&mut runtime, None).unwrap());
        assert!(runtime.agent.runtime_for_session().is_some());
        assert!(runtime
            .agent
            .session
            .as_ref()
            .unwrap()
            .path
            .with_extension("tasks.jsonl")
            .is_file());
        assert!(Arc::ptr_eq(&permissions, &runtime.agent.permissions));
        let policy = runtime.agent.permissions.lock().unwrap();
        assert_eq!(policy.mode, PermissionMode::Ask);
        assert!(policy.project_trusted);
        assert_eq!(policy.deny.len(), 1);
        assert!(policy.session_allow.is_empty());
        assert!(runtime.agent.approver.is_some());
        assert!(runtime.agent.approval_responder.is_some());
    }

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
        let timed = extension_ui_requests_from_calls(&[serde_json::json!({
            "op": "select",
            "title": "Pick",
            "options": ["a"],
            "timeout": 1500
        })]);
        assert_eq!(timed[0]["timeout"], 1500);
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
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
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
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
            .any(|block| matches!(block, davinci_ai::MessageContent::Image { mime_type, .. } if mime_type == "image/png")));
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

    #[test]
    fn prompt_expands_skill_and_template() {
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        runtime.agent.templates.push(davinci_agent::PromptTemplate {
            name: "review".into(),
            path: PathBuf::from("/virtual/review.md"),
            body: "Review this code: $1".into(),
            description: "Review template".into(),
            argument_hint: None,
        });
        let response = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("/review src/lib.rs".into()),
                ..RpcCommand::default()
            },
        );
        assert!(response.success);
        let user = runtime.agent.messages.last().expect("prompt");
        assert_eq!(
            davinci_ai::content_text(&user.content),
            "Review this code: src/lib.rs"
        );
    }

    #[test]
    fn cycle_model_reports_is_scoped_from_scoped_pool() {
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        runtime.models.truncate(3);
        assert!(runtime.models.len() >= 2);
        runtime.agent.provider = runtime.models[0].provider.clone();
        runtime.agent.model_id = runtime.models[0].id.clone();
        let unscoped = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_model".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(unscoped.data.as_ref().unwrap()["isScoped"], false);
        runtime.scoped_models = vec![runtime.models[0].clone(), runtime.models[1].clone()];
        runtime.agent.provider = runtime.models[0].provider.clone();
        runtime.agent.model_id = runtime.models[0].id.clone();
        let scoped = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_model".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(scoped.data.as_ref().unwrap()["isScoped"], true);
        assert_eq!(
            scoped.data.as_ref().unwrap()["model"]["id"],
            runtime.models[1].id
        );
        runtime.scoped_models = vec![runtime.models[0].clone()];
        let single = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_model".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(single.data, Some(Value::Null));
    }

    #[test]
    fn cycle_model_applies_scoped_thinking() {
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        runtime.models.truncate(3);
        runtime.agent.provider = runtime.models[0].provider.clone();
        runtime.agent.model_id = runtime.models[0].id.clone();
        runtime.agent.thinking_level = ThinkingLevel::Off;
        let scoped_ids =
            resolve_scoped_models(&runtime.models, &[format!("{}:high", runtime.models[1].id)]);
        assert!(scoped_ids
            .iter()
            .any(|model| model.id == runtime.models[1].id));
        runtime.scoped_models = vec![runtime.models[0].clone(), runtime.models[1].clone()];
        runtime.scoped_thinking.insert(
            format!("{}/{}", runtime.models[1].provider, runtime.models[1].id),
            ThinkingLevel::High,
        );
        let scoped = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_model".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(scoped.data.as_ref().unwrap()["thinkingLevel"], "high");
        assert_eq!(runtime.agent.thinking_level, ThinkingLevel::High);
    }

    #[test]
    fn thinking_levels_are_model_scoped() {
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        let mute = runtime
            .models
            .iter()
            .find(|model| !model.reasoning)
            .cloned()
            .expect("non-reasoning model");
        runtime.agent.provider = mute.provider.clone();
        runtime.agent.model_id = mute.id.clone();
        let levels = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "get_available_thinking_levels".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(
            levels.data.as_ref().unwrap()["levels"],
            serde_json::json!(["off"])
        );
        let cycled = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_thinking_level".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(cycled.data, Some(Value::Null));

        let fable = runtime
            .models
            .iter()
            .find(|model| model.provider == "anthropic" && model.id == "claude-fable-5")
            .cloned()
            .expect("claude-fable-5");
        runtime.agent.provider = fable.provider.clone();
        runtime.agent.model_id = fable.id.clone();
        runtime.agent.thinking_level = ThinkingLevel::High;
        let available = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "get_available_thinking_levels".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(
            available.data.as_ref().unwrap()["levels"],
            serde_json::json!(available_thinking_levels(Some(&fable)))
        );
        let next = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "cycle_thinking_level".into(),
                ..RpcCommand::default()
            },
        );
        assert_eq!(next.data.as_ref().unwrap()["level"], "xhigh");
    }

    #[test]
    fn session_events_match_ts_agent_session_extras() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/rpc", Some("named")).unwrap();
        session
            .append_entry(davinci_session::SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"hi"}]),
            ))
            .unwrap();
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            dir.path().to_path_buf(),
            PathBuf::from("/tmp"),
        );
        runtime.agent.session = Some(session);
        let steer = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "steer".into(),
                message: Some("nudge".into()),
                ..RpcCommand::default()
            },
        );
        assert!(steer.success);
        let follow = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "follow_up".into(),
                message: Some("later".into()),
                ..RpcCommand::default()
            },
        );
        assert!(follow.success);
        let thinking = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "set_thinking_level".into(),
                level: Some("high".into()),
                ..RpcCommand::default()
            },
        );
        assert!(thinking.success);
        let compact = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "compact".into(),
                ..RpcCommand::default()
            },
        );
        assert!(compact.success);
        let named = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "set_session_name".into(),
                name: Some("renamed".into()),
                ..RpcCommand::default()
            },
        );
        assert!(named.success);
        let events = runtime.take_events();
        let kinds: Vec<_> = events
            .iter()
            .map(|event| match event {
                RpcSessionEvent::QueueUpdate { .. } => "queue_update",
                RpcSessionEvent::CompactionStart { .. } => "compaction_start",
                RpcSessionEvent::CompactionEnd { .. } => "compaction_end",
                RpcSessionEvent::SessionInfoChanged { .. } => "session_info_changed",
                RpcSessionEvent::ThinkingLevelChanged { .. } => "thinking_level_changed",
                RpcSessionEvent::AgentSettled => "agent_settled",
                RpcSessionEvent::EntryAppended { .. } => "entry_appended",
            })
            .collect();
        assert!(kinds.contains(&"queue_update"));
        assert!(kinds.contains(&"compaction_start"));
        assert!(kinds.contains(&"compaction_end"));
        assert!(kinds.contains(&"thinking_level_changed"));
        assert!(kinds.contains(&"session_info_changed"));
        let appended = serde_json::to_value(RpcSessionEvent::EntryAppended {
            entry: serde_json::json!({"id": "e1", "type": "message"}),
        })
        .unwrap();
        assert_eq!(appended["type"], "entry_appended");
        let queue = events
            .iter()
            .rev()
            .find_map(|event| match event {
                RpcSessionEvent::QueueUpdate {
                    steering,
                    follow_up,
                } if !follow_up.is_empty() => Some((steering.clone(), follow_up.clone())),
                _ => None,
            })
            .expect("queue_update");
        assert_eq!(queue.0, vec!["nudge".to_string()]);
        assert_eq!(queue.1, vec!["later".to_string()]);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert!(json.get("type").is_some());
    }

    #[test]
    fn available_models_and_set_model_use_runtime_snapshot() {
        let only = vec![load_builtin_models().into_iter().next().expect("model")];
        let mut runtime = RpcRuntime::with_models(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
            only.clone(),
        );
        let listed = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "get_available_models".into(),
                ..RpcCommand::default()
            },
        );
        assert!(listed.success);
        let models = listed.data.as_ref().unwrap()["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["id"], only[0].id);
        let missing = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "set_model".into(),
                provider: Some("nope".into()),
                model_id: Some("missing".into()),
                ..RpcCommand::default()
            },
        );
        assert!(!missing.success);
        assert_eq!(
            missing.error.as_deref(),
            Some("Model not found: nope/missing")
        );
    }

    #[test]
    fn prompt_preflight_rejects_compaction_and_streaming_without_behavior() {
        let mut runtime = RpcRuntime::new(
            davinci_agent::Agent::new_builtin(PromptProfile::Stable),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        runtime.agent.is_compacting = true;
        let compacting = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("hi".into()),
                ..RpcCommand::default()
            },
        );
        assert!(!compacting.success);
        assert_eq!(compacting.error.as_deref(), Some(COMPACTION_PROMPT_ERROR));
        runtime.agent.is_compacting = false;
        runtime.agent.is_streaming = true;
        let streaming = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("hi".into()),
                ..RpcCommand::default()
            },
        );
        assert!(!streaming.success);
        assert_eq!(streaming.error.as_deref(), Some(STREAMING_PROMPT_ERROR));
        let queued = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("later".into()),
                streaming_behavior: Some("followUp".into()),
                ..RpcCommand::default()
            },
        );
        assert!(queued.success);
        assert!(!runtime.prompt_needs_turn);
        assert_eq!(runtime.agent.queues.follow_up.len(), 1);
    }

    #[test]
    fn test_rpc_agent_control_and_poll() {
        let bus = davinci_agent::RuntimeBus::new();
        let run_id = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();
        let handle = davinci_agent::RuntimeHandle::new(run_id, agent_id, bus);
        let mut agent = davinci_agent::Agent::new_builtin(PromptProfile::Stable);
        agent.runtime = Some(handle);

        let mut runtime = RpcRuntime::new(agent, PathBuf::from("/tmp"), PathBuf::from("/tmp"));

        let worker_id = davinci_agent::AgentId::new();
        let record = davinci_agent::runtime::AgentRecord {
            id: worker_id,
            run_id,
            parent: Some(agent_id),
            kind: davinci_agent::runtime::AgentKind::GraphWorker,
            name: "test-worker".into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: PathBuf::from("/tmp"),
            state: davinci_agent::runtime::AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: 1000,
            updated_ms: 1000,
            failure_reason: None,
        };
        runtime
            .agent
            .runtime
            .as_ref()
            .unwrap()
            .registry
            .register_agent(record)
            .unwrap();

        // 1. Get snapshots
        let snap_res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "get_worker_snapshots".into(),
                ..RpcCommand::default()
            },
        );
        assert!(snap_res.success);
        let sn = snap_res.data.as_ref().unwrap()["snapshots"]
            .as_array()
            .unwrap();
        assert_eq!(sn.len(), 1);
        assert_eq!(sn[0]["name"], "test-worker");

        // 2. Control inspect
        let inspect_res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "agent_control".into(),
                agent_id: Some(worker_id.to_string()),
                action: Some("inspect".into()),
                ..RpcCommand::default()
            },
        );
        assert!(inspect_res.success);
        assert!(inspect_res.data.as_ref().unwrap()["completed"]
            .as_bool()
            .unwrap());

        // 3. Control stop (completed action)
        let stop_res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "agent_control".into(),
                agent_id: Some(worker_id.to_string()),
                action: Some("stop".into()),
                operation_id: Some("op-123".into()),
                message: Some("graceful stop".into()),
                ..RpcCommand::default()
            },
        );
        assert!(stop_res.success);
        let stop_data = stop_res.data.as_ref().unwrap();
        assert_eq!(stop_data["operationId"], "op-123");
        assert_eq!(stop_data["status"], "stopped");
        assert!(stop_data["completed"].as_bool().unwrap());

        // 4. Poll operation by ID
        let poll_res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "poll_control_operation".into(),
                operation_id: Some("op-123".into()),
                ..RpcCommand::default()
            },
        );
        assert!(poll_res.success);
        let polled_data = poll_res.data.as_ref().unwrap();
        assert_eq!(polled_data["operationId"], "op-123");
        assert_eq!(polled_data["status"], "stopped");

        // 5. Control retry on active worker (accepted command, not yet terminal/completed)
        let worker_id2 = davinci_agent::AgentId::new();
        let record2 = davinci_agent::runtime::AgentRecord {
            id: worker_id2,
            run_id,
            parent: Some(agent_id),
            kind: davinci_agent::runtime::AgentKind::GraphWorker,
            name: "test-worker-2".into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: PathBuf::from("/tmp"),
            state: davinci_agent::runtime::AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: 1000,
            updated_ms: 1000,
            failure_reason: None,
        };
        runtime
            .agent
            .runtime
            .as_ref()
            .unwrap()
            .registry
            .register_agent(record2)
            .unwrap();

        let retry_res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "agent_control".into(),
                agent_id: Some(worker_id2.to_string()),
                action: Some("retry".into()),
                operation_id: Some("op-retry".into()),
                ..RpcCommand::default()
            },
        );
        assert!(retry_res.success);
        let retry_data = retry_res.data.as_ref().unwrap();
        assert_eq!(retry_data["status"], "accepted");
        assert!(!retry_data["completed"].as_bool().unwrap());
    }

    #[test]
    fn f02_rpc_decision_typed_reply() {
        let question = davinci_agent::decisions::DecisionQuestion {
            id: "db-choice".into(),
            kind: davinci_agent::decisions::DecisionKind::Persistence,
            title: "Database".into(),
            question: "Choose db".into(),
            materiality: "Changes storage".into(),
            evidence_refs: vec![],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![davinci_agent::decisions::DecisionOptionInput {
                id: "sqlite".into(),
                label: "SQLite".into(),
                explanation: "Local file".into(),
                recommended: true,
            }],
            allow_custom: false,
            custom_only: false,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
            plan_revision: 1,
        };
        let request = davinci_agent::DecisionHostRequest { question };
        let response = rpc_resolve_decision(&request, |call| {
            assert_eq!(call["op"], "decision");
            assert_eq!(call["question"]["id"], "db-choice");
            serde_json::json!({
                "action": "answer_choice",
                "choice_id": "sqlite",
                "host_event_id": "evt-123",
                "answered_at_ms": 1000
            })
        });
        match response {
            davinci_agent::DecisionHostResponse::Reply(reply) => {
                assert_eq!(
                    reply.action,
                    davinci_agent::decisions::HostDecisionAction::AnswerChoice("sqlite".into())
                );
                assert_eq!(reply.host_event_id, "evt-123");
            }
            other => panic!("expected Reply, got {:?}", other),
        }
    }

    #[test]
    fn f02_rpc_decision_sequential_select_and_custom() {
        let question = davinci_agent::decisions::DecisionQuestion {
            id: "db-choice".into(),
            kind: davinci_agent::decisions::DecisionKind::Persistence,
            title: "Database".into(),
            question: "Choose db".into(),
            materiality: "Changes storage".into(),
            evidence_refs: vec![],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![davinci_agent::decisions::DecisionOptionInput {
                id: "sqlite".into(),
                label: "SQLite".into(),
                explanation: "Local file".into(),
                recommended: false,
            }],
            allow_custom: true,
            custom_only: false,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
            plan_revision: 2,
        };
        let request = davinci_agent::DecisionHostRequest { question };
        let mut call_count = 0;
        let response = rpc_resolve_decision(&request, |call| {
            call_count += 1;
            if call["op"] == "decision" {
                // Older client returns null (unknown op)
                serde_json::Value::Null
            } else if call["op"] == "select" {
                // Preserves question and revision identity
                assert_eq!(call["decision_id"], "db-choice");
                assert_eq!(call["plan_revision"], 2);
                serde_json::json!("Custom response")
            } else if call["op"] == "input" {
                assert_eq!(call["decision_id"], "db-choice");
                assert_eq!(call["plan_revision"], 2);
                serde_json::json!("postgres://localhost:5432")
            } else {
                panic!("unexpected op {}", call["op"]);
            }
        });
        assert_eq!(call_count, 3);
        match response {
            davinci_agent::DecisionHostResponse::Reply(reply) => {
                assert_eq!(
                    reply.action,
                    davinci_agent::decisions::HostDecisionAction::AnswerCustom(
                        "postgres://localhost:5432".into()
                    )
                );
            }
            other => panic!("expected Reply with custom answer, got {:?}", other),
        }
    }

    #[test]
    fn f02_rpc_decision_timeout_and_disconnect_cannot_synthesize_answer() {
        let question = davinci_agent::decisions::DecisionQuestion {
            id: "db-choice".into(),
            kind: davinci_agent::decisions::DecisionKind::Persistence,
            title: "Database".into(),
            question: "Choose db".into(),
            materiality: "Changes storage".into(),
            evidence_refs: vec![],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![],
            allow_custom: false,
            custom_only: false,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
            plan_revision: 1,
        };
        let request = davinci_agent::DecisionHostRequest { question };
        let response = rpc_resolve_decision(&request, |_| serde_json::Value::Null);
        assert_eq!(response, davinci_agent::DecisionHostResponse::Unavailable);
    }

    #[test]
    fn f13_rpc_graph_control_and_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let agent = Agent::new("fixture");
        let mut runtime = RpcRuntime::new(agent, dir.path().into(), dir.path().into());
        let run_id = crate::native_extensions::graph::store::new_run_id();
        crate::native_extensions::graph::store::create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = crate::native_extensions::graph::types::GraphRun {
            version: 1,
            run_id: run_id.clone(),
            goal: "rpc test goal".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            phase: crate::native_extensions::graph::types::Phase::Implement,
            forced: None,
            dry_run: true,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: crate::native_extensions::graph::types::GraphBudgets::default(),
            counters: crate::native_extensions::graph::types::GraphCounters {
                workers_spawned: 1,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: crate::native_extensions::graph::store::now_ms(),
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: Some(crate::native_extensions::graph::types::GraphLifecycle::Running),
            revision: 1,
        };
        crate::native_extensions::graph::store::save_run(&mut run).unwrap();

        // 1. Send graph_control command to pause
        let res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "graph_control".into(),
                run_id: Some(run_id.clone()),
                action: Some("pause".into()),
                operation_id: Some("op-pause-1".into()),
                expected_revision: Some(1),
                ..RpcCommand::default()
            },
        );
        assert!(res.success);
        let receipt = res.data.unwrap();
        assert_eq!(receipt["operationId"], "op-pause-1");
        assert_eq!(receipt["state"], "applied");

        // Verify state is paused on disk
        let loaded = crate::native_extensions::graph::store::load_run(dir.path(), &run_id).unwrap();
        assert_eq!(
            loaded.current_lifecycle(),
            crate::native_extensions::graph::types::GraphLifecycle::Paused
        );

        // 2. Send graph_control to missing run (simulates disconnect / missing target)
        let res_missing = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "graph_control".into(),
                run_id: Some("non-existent-run".into()),
                action: Some("pause".into()),
                ..RpcCommand::default()
            },
        );
        assert!(!res_missing.success);
        assert!(res_missing.error.unwrap().contains("not found"));
    }

    #[test]
    fn rpc_queued_steer_and_follow_up_prepare_each_user_turn_before_provider() {
        fn assistant(id: &str) -> davinci_ai::AssistantMessage {
            davinci_ai::AssistantMessage {
                id: id.into(),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            }
        }

        let dir = tempdir().unwrap();
        let mut steer_agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
        steer_agent.prompt_session.append("RPC APPEND SENTINEL");
        steer_agent.queues.steer_mode = davinci_agent::QueueMode::OneAtATime;
        let mut steer_runtime = RpcRuntime::new(
            steer_agent,
            dir.path().join("sessions-steer"),
            dir.path().to_path_buf(),
        );
        for message in [
            "Redesign this dashboard so it feels premium and intentional.",
            "What is 2 + 2?",
        ] {
            let response = handle_rpc(
                &mut steer_runtime,
                RpcCommand {
                    kind: "steer".into(),
                    message: Some(message.into()),
                    ..RpcCommand::default()
                },
            );
            assert!(response.success);
        }
        let mut steer_provider_prompts = Vec::new();
        steer_runtime
            .agent
            .run_loop(|agent| {
                steer_provider_prompts.push(agent.system_prompt.clone());
                Ok(assistant("rpc-steer"))
            })
            .unwrap();
        assert_eq!(steer_provider_prompts.len(), 2);
        let active = &steer_provider_prompts[0];
        assert!(active.contains("frontend_design_policy"));
        assert!(
            active.find("frontend_design_policy").unwrap()
                < active.find("RPC APPEND SENTINEL").unwrap(),
            "dynamic capability suffix must precede user append"
        );
        assert!(!steer_provider_prompts[1].contains("frontend_design_policy"));

        let mut follow_agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
        follow_agent.prompt_session.append("RPC FOLLOW APPEND");
        follow_agent.queues.follow_up_mode = davinci_agent::QueueMode::OneAtATime;
        let mut follow_runtime = RpcRuntime::new(
            follow_agent,
            dir.path().join("sessions-follow"),
            dir.path().to_path_buf(),
        );
        let prompt_response = handle_rpc(
            &mut follow_runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some("Start with a plain factual answer.".into()),
                ..RpcCommand::default()
            },
        );
        assert!(prompt_response.success);
        for message in [
            "Redesign this dashboard so it feels premium and intentional.",
            "What is 2 + 2?",
        ] {
            let response = handle_rpc(
                &mut follow_runtime,
                RpcCommand {
                    kind: "follow_up".into(),
                    message: Some(message.into()),
                    ..RpcCommand::default()
                },
            );
            assert!(response.success);
        }
        let mut follow_provider_prompts = Vec::new();
        follow_runtime
            .agent
            .run_loop(|agent| {
                follow_provider_prompts.push(agent.system_prompt.clone());
                Ok(assistant("rpc-follow"))
            })
            .unwrap();
        assert_eq!(follow_provider_prompts.len(), 3);
        assert!(!follow_provider_prompts[0].contains("frontend_design_policy"));
        let active = &follow_provider_prompts[1];
        assert!(active.contains("frontend_design_policy"));
        assert!(
            active.find("frontend_design_policy").unwrap()
                < active.find("RPC FOLLOW APPEND").unwrap(),
            "dynamic capability suffix must precede user append"
        );
        assert!(!follow_provider_prompts[2].contains("frontend_design_policy"));
    }

    #[test]
    fn rpc_default_all_queued_turns_union_capabilities_for_one_provider_request() {
        fn assistant(id: &str) -> davinci_ai::AssistantMessage {
            davinci_ai::AssistantMessage {
                id: id.into(),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            }
        }

        let dir = tempdir().unwrap();
        let mut agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
        agent.prompt_session.append("RPC ALL APPEND");
        assert_eq!(agent.queues.steer_mode, davinci_agent::QueueMode::All);
        let mut runtime = RpcRuntime::new(
            agent,
            dir.path().join("sessions-all"),
            dir.path().to_path_buf(),
        );
        for message in [
            "Redesign this dashboard so it feels premium and intentional.",
            "What is 2 + 2?",
        ] {
            let response = handle_rpc(
                &mut runtime,
                RpcCommand {
                    kind: "steer".into(),
                    message: Some(message.into()),
                    ..RpcCommand::default()
                },
            );
            assert!(response.success);
        }
        let mut provider_prompts = Vec::new();
        runtime
            .agent
            .run_loop(|agent| {
                provider_prompts.push(agent.system_prompt.clone());
                Ok(assistant("rpc-all"))
            })
            .unwrap();
        assert_eq!(provider_prompts.len(), 1);
        assert!(provider_prompts[0].contains("frontend_design_policy"));
        assert!(
            provider_prompts[0].find("frontend_design_policy").unwrap()
                < provider_prompts[0].find("RPC ALL APPEND").unwrap()
        );
    }

    #[test]
    fn rpc_prompt_activates_capabilities_on_user_turn() {
        let dir = tempdir().unwrap();
        let agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
        let mut runtime =
            RpcRuntime::new(agent, dir.path().join("sessions"), dir.path().to_path_buf());

        assert!(!runtime
            .agent
            .system_prompt
            .contains("frontend_design_policy"));

        let res = handle_rpc(
            &mut runtime,
            RpcCommand {
                kind: "prompt".into(),
                message: Some(
                    "Redesign this dashboard so it feels premium and intentional.".into(),
                ),
                ..RpcCommand::default()
            },
        );
        assert!(res.success);
        assert!(runtime.prompt_needs_turn);
        assert!(
            runtime
                .agent
                .system_prompt
                .contains("frontend_design_policy"),
            "RPC user prompt must activate capability"
        );
        assert!(runtime
            .agent
            .prompt_manifest
            .as_ref()
            .unwrap()
            .modules
            .iter()
            .any(|m| m.id == "capability.frontend-design"));
    }
}
