use crate::error::Result;
use crate::events::AgentEvent;
use crate::permission::{AllowAllPermissionPolicy, PermissionDecision, PermissionPolicy};
use crate::tools::{AgentTool, AgentToolResult};
use futures::StreamExt;
use pi_ai::{
    now_ms, AssistantMessage, AssistantMessageEvent, ContentBlock, Context, Message, Model,
    SimpleStreamOptions, StopReason, ToolResultMessage,
};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct AgentConfig {
    pub model: Model,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub permission_policy: Arc<dyn PermissionPolicy>,
    pub max_turns: usize,
    pub stream_options: SimpleStreamOptions,
}

impl AgentConfig {
    pub fn new(model: Model) -> Self {
        Self {
            model,
            tools: Vec::new(),
            permission_policy: Arc::new(AllowAllPermissionPolicy),
            max_turns: 20,
            stream_options: SimpleStreamOptions::default(),
        }
    }

    pub fn with_tools(mut self, tools: Vec<Arc<dyn AgentTool>>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_permission_policy(mut self, policy: Arc<dyn PermissionPolicy>) -> Self {
        self.permission_policy = policy;
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }
}

pub struct AgentRuntime {
    config: AgentConfig,
}

impl AgentRuntime {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    pub async fn run(
        &self,
        initial_messages: Vec<Message>,
        event_sender: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<Vec<Message>> {
        let _ = event_sender.send(AgentEvent::AgentStart);

        let mut context = Context {
            system_prompt: None,
            messages: initial_messages,
            tools: Some(self.config.tools.iter().map(|t| t.to_tool_def()).collect()),
        };

        for prompt in &context.messages {
            let _ = event_sender.send(AgentEvent::MessageStart {
                message: prompt.clone(),
            });
            let _ = event_sender.send(AgentEvent::MessageEnd {
                message: prompt.clone(),
            });
        }

        let mut turn_count = 0;

        while turn_count < self.config.max_turns {
            turn_count += 1;
            let _ = event_sender.send(AgentEvent::TurnStart);

            let mut stream = pi_ai::providers::stream_simple(
                &self.config.model,
                &context,
                &self.config.stream_options,
            )
            .await?;

            let mut assistant_msg: Option<AssistantMessage> = None;

            while let Some(event) = stream.next().await {
                match event {
                    AssistantMessageEvent::Start { partial } => {
                        assistant_msg = Some(partial);
                    }
                    AssistantMessageEvent::TextDelta {
                        content_index,
                        delta,
                        partial,
                    } => {
                        assistant_msg = Some(partial);
                        let _ = event_sender.send(AgentEvent::TextDelta {
                            content_index,
                            delta,
                        });
                    }
                    AssistantMessageEvent::ThinkingDelta {
                        content_index,
                        delta,
                        partial,
                    } => {
                        assistant_msg = Some(partial);
                        let _ = event_sender.send(AgentEvent::ThinkingDelta {
                            content_index,
                            delta,
                        });
                    }
                    AssistantMessageEvent::ToolCallStart { content_index, .. } => {
                        let _ = event_sender.send(AgentEvent::ToolCallStart { content_index });
                    }
                    AssistantMessageEvent::ToolCallDelta {
                        content_index,
                        delta,
                        ..
                    } => {
                        let _ = event_sender.send(AgentEvent::ToolCallDelta {
                            content_index,
                            delta,
                        });
                    }
                    AssistantMessageEvent::ToolCallEnd {
                        content_index,
                        tool_call,
                        partial,
                    } => {
                        assistant_msg = Some(partial);
                        let _ = event_sender.send(AgentEvent::ToolCallEnd {
                            content_index,
                            tool_call,
                        });
                    }
                    AssistantMessageEvent::Done { message, .. } => {
                        assistant_msg = Some(message);
                    }
                    AssistantMessageEvent::Error { error, .. } => {
                        assistant_msg = Some(error);
                    }
                    _ => {}
                }
            }

            let final_assistant = assistant_msg.unwrap_or_else(|| AssistantMessage {
                role: "assistant".to_string(),
                content: vec![],
                api: self.config.model.api.clone(),
                provider: self.config.model.provider.clone(),
                model: self.config.model.id.clone(),
                response_model: None,
                response_id: None,
                usage: Default::default(),
                stop_reason: StopReason::Stop,
                deferred: None,
                error_message: None,
                raw_stop_reason: None,
                end_turn: Some(true),
                timestamp: now_ms(),
            });

            let _ = event_sender.send(AgentEvent::MessageEnd {
                message: Message::Assistant(final_assistant.clone()),
            });

            let _ = event_sender.send(AgentEvent::TurnEnd {
                stop_reason: final_assistant.stop_reason,
                usage: final_assistant.usage.clone(),
            });

            context
                .messages
                .push(Message::Assistant(final_assistant.clone()));

            // Find tool calls
            let mut tool_calls = Vec::new();
            for block in &final_assistant.content {
                if let ContentBlock::ToolCall(tc) = block {
                    tool_calls.push(tc.clone());
                }
            }

            if tool_calls.is_empty()
                || final_assistant.stop_reason == StopReason::Stop
                || final_assistant.stop_reason == StopReason::Error
                || final_assistant.stop_reason == StopReason::Aborted
            {
                break;
            }

            // Execute tool calls
            for tc in tool_calls {
                let _ = event_sender.send(AgentEvent::ToolExecutionStart {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });

                let tool_opt = self.config.tools.iter().find(|t| t.name() == tc.name);
                let tool_res = match tool_opt {
                    Some(tool) => {
                        let perm = if tool.requires_permission() {
                            self.config
                                .permission_policy
                                .check_permission(tool.as_ref(), &tc.id, &tc.arguments)
                                .await
                        } else {
                            PermissionDecision::Allow
                        };

                        match perm {
                            PermissionDecision::Allow | PermissionDecision::AllowSession => tool
                                .execute(&tc.id, &tc.arguments)
                                .await
                                .unwrap_or_else(|e| {
                                    AgentToolResult::error(format!("Execution failed: {}", e))
                                }),
                            PermissionDecision::Deny => {
                                AgentToolResult::error("Permission denied by user or policy")
                            }
                        }
                    }
                    None => AgentToolResult::error(format!("Tool '{}' not found", tc.name)),
                };

                let _ = event_sender.send(AgentEvent::ToolExecutionEnd {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    is_error: tool_res.is_error,
                });

                let tool_result_msg = ToolResultMessage {
                    role: "toolResult".to_string(),
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    content: tool_res.content,
                    details: tool_res.details,
                    usage: tool_res.usage,
                    added_tool_names: tool_res.added_tool_names,
                    is_error: tool_res.is_error,
                    timestamp: now_ms(),
                };

                let msg_enum = Message::ToolResult(tool_result_msg);
                let _ = event_sender.send(AgentEvent::MessageStart {
                    message: msg_enum.clone(),
                });
                let _ = event_sender.send(AgentEvent::MessageEnd {
                    message: msg_enum.clone(),
                });
                context.messages.push(msg_enum);
            }
        }

        let _ = event_sender.send(AgentEvent::AgentEnd {
            messages: context.messages.clone(),
        });

        Ok(context.messages)
    }
}
