use std::path::Path;

use pi_ai::{
    assistant_to_chat, AssistantMessage, ChatMessage, ContentBlock, MessageContent, StopReason,
};
use serde_json::Value;

use crate::events::AgentEvent;
use crate::tools::execute_tool;
use crate::Agent;
use crate::ToolExecutionMode;

impl Agent {
    /// Start a loop after user prompts have already been appended.
    pub fn run_loop<F>(&mut self, complete: F) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<AssistantMessage, String>,
    {
        self.run_loop_inner(true, complete)
    }

    /// Continue from an existing user or toolResult tail (TS `agentLoopContinue`).
    pub fn continue_loop<F>(&mut self, complete: F) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<AssistantMessage, String>,
    {
        if self.messages.is_empty() {
            return Err("Cannot continue: no messages in context".into());
        }
        if self.messages.last().map(|m| m.role.as_str()) == Some("assistant") {
            return Err("Cannot continue from message role: assistant".into());
        }
        self.run_loop_inner(false, complete)
    }

    fn run_loop_inner<F>(
        &mut self,
        emit_prompt_messages: bool,
        mut complete: F,
    ) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<AssistantMessage, String>,
    {
        self.is_streaming = true;
        let mut events = Vec::new();
        let mut new_messages: Vec<ChatMessage> = Vec::new();
        events.push(AgentEvent::AgentStart);
        events.push(AgentEvent::TurnStart);

        if emit_prompt_messages {
            if let Some(prompt) = self.messages.last().cloned() {
                if prompt.role == "user" {
                    events.push(AgentEvent::MessageStart {
                        message: prompt.clone(),
                    });
                    events.push(AgentEvent::MessageEnd { message: prompt });
                }
            }
        }

        loop {
            if self.aborted {
                events.push(AgentEvent::AgentEnd {
                    messages: new_messages,
                });
                self.is_streaming = false;
                return Ok(events);
            }

            self.inject_queued(&mut events, &mut new_messages, true);

            if self.auto_compaction {
                let tokens = crate::estimate_context_tokens(&self.messages);
                let mut settings = self.compaction;
                settings.enabled = true;
                if crate::should_compact(tokens, self.context_window, &settings) {
                    let _ = self.compact(None);
                }
            }

            let assistant = match self.complete_with_retry(&mut complete) {
                Ok(message) => message,
                Err(err) => {
                    self.is_streaming = false;
                    return Err(err);
                }
            };
            let chat = assistant_to_chat(&assistant);
            self.messages.push(chat.clone());
            self.persist_chat(&chat);
            new_messages.push(chat.clone());
            events.push(AgentEvent::MessageStart {
                message: chat.clone(),
            });
            events.push(AgentEvent::MessageEnd {
                message: chat.clone(),
            });

            if matches!(
                assistant.stop_reason,
                Some(StopReason::Error) | Some(StopReason::Aborted)
            ) {
                events.push(AgentEvent::TurnEnd {
                    message: chat,
                    tool_results: Vec::new(),
                });
                events.push(AgentEvent::AgentEnd {
                    messages: new_messages,
                });
                self.is_streaming = false;
                return Ok(events);
            }

            let tool_calls = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some((id.clone(), name.clone(), arguments.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>();

            let had_tools = !tool_calls.is_empty();
            let mut tool_results = Vec::new();
            if had_tools {
                if assistant.stop_reason == Some(StopReason::Length) {
                    for (id, name, args) in &tool_calls {
                        let result = ChatMessage::tool_result(
                            id,
                            name,
                            "Tool call arguments were truncated by the output token limit",
                            true,
                        );
                        events.push(AgentEvent::ToolExecutionStart {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            args: args.clone(),
                        });
                        events.push(AgentEvent::ToolExecutionEnd {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            result: Value::String(
                                result
                                    .content
                                    .first()
                                    .and_then(|c| match c {
                                        MessageContent::Text { text } => Some(text.clone()),
                                        _ => None,
                                    })
                                    .unwrap_or_default(),
                            ),
                            is_error: true,
                        });
                        self.messages.push(result.clone());
                        self.persist_chat(&result);
                        new_messages.push(result.clone());
                        events.push(AgentEvent::MessageStart {
                            message: result.clone(),
                        });
                        events.push(AgentEvent::MessageEnd {
                            message: result.clone(),
                        });
                        tool_results.push(result);
                    }
                } else {
                    let cwd = self.cwd.clone();
                    match self.tool_execution_mode {
                        ToolExecutionMode::Sequential => {
                            for (id, name, args) in tool_calls {
                                if self.aborted {
                                    break;
                                }
                                let result = self.execute_one(&cwd, &id, &name, &args, &mut events);
                                self.messages.push(result.clone());
                                self.persist_chat(&result);
                                new_messages.push(result.clone());
                                events.push(AgentEvent::MessageStart {
                                    message: result.clone(),
                                });
                                events.push(AgentEvent::MessageEnd {
                                    message: result.clone(),
                                });
                                tool_results.push(result);
                            }
                        }
                        ToolExecutionMode::Parallel => {
                            let prepared: Vec<_> = tool_calls;
                            for (id, name, args) in prepared {
                                if self.aborted {
                                    break;
                                }
                                let result = self.execute_one(&cwd, &id, &name, &args, &mut events);
                                self.messages.push(result.clone());
                                self.persist_chat(&result);
                                new_messages.push(result.clone());
                                events.push(AgentEvent::MessageStart {
                                    message: result.clone(),
                                });
                                events.push(AgentEvent::MessageEnd {
                                    message: result.clone(),
                                });
                                tool_results.push(result);
                            }
                        }
                    }
                }
            }

            events.push(AgentEvent::TurnEnd {
                message: chat,
                tool_results,
            });

            if had_tools && !self.aborted {
                events.push(AgentEvent::TurnStart);
                continue;
            }

            if !self.queues.steer.is_empty() {
                events.push(AgentEvent::TurnStart);
                continue;
            }

            if !self.queues.follow_up.is_empty() {
                events.push(AgentEvent::TurnStart);
                self.inject_queued(&mut events, &mut new_messages, false);
                continue;
            }

            break;
        }

        events.push(AgentEvent::AgentEnd {
            messages: new_messages,
        });
        self.is_streaming = false;
        Ok(events)
    }

    fn inject_queued(
        &mut self,
        events: &mut Vec<AgentEvent>,
        new_messages: &mut Vec<ChatMessage>,
        steer: bool,
    ) {
        let drained = if steer {
            let mode = self.queues.steer_mode;
            self.queues.drain_steer(mode)
        } else {
            let mode = self.queues.follow_up_mode;
            self.queues.drain_follow_up(mode)
        };
        for queued in drained {
            let message = self.prompt(&queued.text);
            new_messages.push(message.clone());
            events.push(AgentEvent::MessageStart {
                message: message.clone(),
            });
            events.push(AgentEvent::MessageEnd { message });
        }
    }

    fn complete_with_retry<F>(&mut self, complete: &mut F) -> Result<AssistantMessage, String>
    where
        F: FnMut(&Agent) -> Result<AssistantMessage, String>,
    {
        let attempts = if self.auto_retry {
            self.retry_attempts.max(1)
        } else {
            1
        };
        let mut last_error = None;
        for attempt in 0..attempts {
            if self.aborted {
                return Ok(AssistantMessage {
                    id: crate::new_message_id(),
                    role: "assistant".into(),
                    content: Vec::new(),
                    model: String::new(),
                    usage: None,
                    stop_reason: Some(StopReason::Aborted),
                    error_message: Some("aborted".into()),
                });
            }
            match complete(self) {
                Ok(message) => return Ok(message),
                Err(err) => {
                    last_error = Some(err);
                    if attempt + 1 < attempts {
                        sleep_retry_delay(retry_delay_ms(self.retry_base_delay_ms, attempt));
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| "Provider request failed".into()))
    }

    fn execute_one(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        events: &mut Vec<AgentEvent>,
    ) -> ChatMessage {
        events.push(AgentEvent::ToolExecutionStart {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: args.clone(),
        });
        let (content, is_error) = if !self.tools.iter().any(|tool| tool == name) {
            (format!("Unknown tool: {name}"), true)
        } else {
            match execute_tool(cwd, name, args) {
                Ok(result) => (result.content, result.is_error),
                Err(crate::tools::ToolError::Unknown(_)) => {
                    if let Some(executor) = &self.custom_tool_executor {
                        match executor.execute(cwd, name, args) {
                            Ok(result) => (result.content, result.is_error),
                            Err(err) => (err.to_string(), true),
                        }
                    } else {
                        (format!("Unknown tool: {name}"), true)
                    }
                }
                Err(err) => (err.to_string(), true),
            }
        };
        events.push(AgentEvent::ToolExecutionUpdate {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: args.clone(),
            partial_result: Value::String(content.clone()),
        });
        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            result: Value::String(content.clone()),
            is_error,
        });
        ChatMessage::tool_result(id, name, content, is_error)
    }

    fn persist_chat(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let content = serde_json::to_value(&message.content).unwrap_or(Value::Null);
            let _ = session.append_entry(pi_session::SessionEntry::message(&message.role, content));
        }
    }
}

/// TS `baseDelayMs * 2 ** (attempt - 1)` with attempt starting at 1.
pub fn retry_delay_ms(base_delay_ms: u64, zero_based_attempt: u32) -> u64 {
    let shift = zero_based_attempt.min(20);
    base_delay_ms.saturating_mul(1_u64 << shift)
}

fn sleep_retry_delay(delay_ms: u64) {
    if delay_ms == 0 || cfg!(test) {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
}
