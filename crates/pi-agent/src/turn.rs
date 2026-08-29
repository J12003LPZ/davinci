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
    pub fn run_loop<F, T>(&mut self, complete: F) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<T, String>,
        T: Into<crate::CompleteOutput>,
    {
        self.run_loop_inner(true, complete)
    }

    /// Continue from an existing user or toolResult tail (TS `agentLoopContinue`).
    pub fn continue_loop<F, T>(&mut self, complete: F) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<T, String>,
        T: Into<crate::CompleteOutput>,
    {
        if self.messages.is_empty() {
            return Err("Cannot continue: no messages in context".into());
        }
        if self.messages.last().map(|m| m.role.as_str()) == Some("assistant") {
            return Err("Cannot continue from message role: assistant".into());
        }
        self.run_loop_inner(false, complete)
    }

    fn run_loop_inner<F, T>(
        &mut self,
        emit_prompt_messages: bool,
        mut complete: F,
    ) -> Result<Vec<AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<T, String>,
        T: Into<crate::CompleteOutput>,
    {
        self.is_streaming = true;
        self.retry_aborted = false;
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
                    will_retry: false,
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

            let (assistant, stream_events) =
                match self.complete_with_retry(&mut complete, &mut events) {
                    Ok(output) => output,
                    Err(err) => {
                        self.is_streaming = false;
                        return Err(err);
                    }
                };
            let chat = assistant_to_chat(&assistant);
            self.messages.push(chat.clone());
            self.persist_assistant(&assistant, &chat);
            new_messages.push(chat.clone());
            events.push(AgentEvent::MessageStart {
                message: chat.clone(),
            });
            let updates = stream_events.unwrap_or_else(|| pi_ai::events_from_complete(&assistant));
            for assistant_message_event in updates {
                events.push(AgentEvent::MessageUpdate {
                    message: chat.clone(),
                    assistant_message_event,
                });
            }
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
                    will_retry: false,
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
            will_retry: false,
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
            let message = self.prompt_with(&queued.text, &queued.images);
            new_messages.push(message.clone());
            events.push(AgentEvent::MessageStart {
                message: message.clone(),
            });
            events.push(AgentEvent::MessageEnd { message });
        }
    }

    fn complete_with_retry<F, T>(
        &mut self,
        complete: &mut F,
        events: &mut Vec<AgentEvent>,
    ) -> Result<(AssistantMessage, Option<Vec<pi_ai::AssistantMessageEvent>>), String>
    where
        F: FnMut(&Agent) -> Result<T, String>,
        T: Into<crate::CompleteOutput>,
    {
        let max_retries = if self.auto_retry {
            self.retry_attempts
        } else {
            0
        };
        let attempts = max_retries.max(1);
        let mut last_error = None;
        let mut scheduled_attempt = 0_u32;
        for attempt in 0..attempts {
            if self.aborted || self.retry_aborted {
                if scheduled_attempt > 0 {
                    events.push(AgentEvent::AutoRetryEnd {
                        success: false,
                        attempt: scheduled_attempt,
                        final_error: Some("Retry cancelled".into()),
                    });
                }
                return Ok((
                    AssistantMessage {
                        id: crate::new_message_id(),
                        role: "assistant".into(),
                        content: Vec::new(),
                        model: String::new(),
                        usage: None,
                        stop_reason: Some(StopReason::Aborted),
                        error_message: Some("aborted".into()),
                    },
                    None,
                ));
            }
            match complete(self) {
                Ok(output) => {
                    let output = output.into();
                    let message = output.message;
                    if message.stop_reason == Some(StopReason::Error)
                        && pi_ai::is_retryable_assistant_error(&message)
                        && attempt + 1 < attempts
                    {
                        scheduled_attempt = attempt + 1;
                        let delay = retry_delay_ms(self.retry_base_delay_ms, attempt);
                        events.push(AgentEvent::AutoRetryStart {
                            attempt: scheduled_attempt,
                            max_attempts: max_retries,
                            delay_ms: delay,
                            error_message: message
                                .error_message
                                .clone()
                                .unwrap_or_else(|| "Unknown error".into()),
                        });
                        sleep_retry_delay(delay);
                        continue;
                    }
                    if scheduled_attempt > 0 {
                        let success = message.stop_reason != Some(StopReason::Error);
                        events.push(AgentEvent::AutoRetryEnd {
                            success,
                            attempt: scheduled_attempt,
                            final_error: if success {
                                None
                            } else {
                                message.error_message.clone()
                            },
                        });
                    }
                    return Ok((message, output.stream_events));
                }
                Err(err) => {
                    last_error = Some(err.clone());
                    if attempt + 1 < attempts && pi_ai::is_retryable_error_text(&err) {
                        scheduled_attempt = attempt + 1;
                        let delay = retry_delay_ms(self.retry_base_delay_ms, attempt);
                        events.push(AgentEvent::AutoRetryStart {
                            attempt: scheduled_attempt,
                            max_attempts: max_retries,
                            delay_ms: delay,
                            error_message: err,
                        });
                        sleep_retry_delay(delay);
                    } else if attempt + 1 < attempts {
                        sleep_retry_delay(retry_delay_ms(self.retry_base_delay_ms, attempt));
                    }
                }
            }
        }
        if scheduled_attempt > 0 {
            events.push(AgentEvent::AutoRetryEnd {
                success: false,
                attempt: scheduled_attempt,
                final_error: last_error.clone(),
            });
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
        let result =
            if let Some(reason) = self.pre_tool.as_ref().and_then(|hook| (hook.0)(name, args)) {
                crate::ToolResult {
                    content: reason,
                    is_error: true,
                    details: None,
                }
            } else if !self.tools.iter().any(|tool| tool == name) {
                crate::ToolResult {
                    content: format!("Unknown tool: {name}"),
                    is_error: true,
                    details: None,
                }
            } else {
                match execute_tool(cwd, name, args) {
                    Ok(result) => result,
                    Err(crate::tools::ToolError::Unknown(_)) => {
                        if let Some(executor) = &self.custom_tool_executor {
                            match executor.execute(cwd, name, args) {
                                Ok(result) => result,
                                Err(err) => crate::ToolResult {
                                    content: err.to_string(),
                                    is_error: true,
                                    details: None,
                                },
                            }
                        } else {
                            crate::ToolResult {
                                content: format!("Unknown tool: {name}"),
                                is_error: true,
                                details: None,
                            }
                        }
                    }
                    Err(err) => crate::ToolResult {
                        content: err.to_string(),
                        is_error: true,
                        details: None,
                    },
                }
            };
        let mut details = result.details.clone();
        for partial in crate::ToolResult::take_updates(&mut details) {
            events.push(AgentEvent::ToolExecutionUpdate {
                tool_call_id: id.to_string(),
                tool_name: name.to_string(),
                args: args.clone(),
                partial_result: partial,
            });
        }
        events.push(AgentEvent::ToolExecutionUpdate {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: args.clone(),
            partial_result: Value::String(result.content.clone()),
        });
        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            result: Value::String(result.content.clone()),
            is_error: result.is_error,
        });
        tool_result_message(id, name, result, self.auto_resize_images)
    }

    fn persist_chat(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let content = serde_json::to_value(&message.content).unwrap_or(Value::Null);
            let _ = session.append_entry(pi_session::SessionEntry::message(&message.role, content));
        }
    }

    fn persist_assistant(&mut self, assistant: &AssistantMessage, chat: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let timestamp = pi_session::now_ms();
            let mut message = serde_json::json!({
                "role": "assistant",
                "content": chat.content,
                "model": assistant.model,
                "provider": self.provider,
                "timestamp": timestamp,
            });
            if let Some(usage) = &assistant.usage {
                if let Ok(value) = serde_json::to_value(usage) {
                    message["usage"] = value;
                }
            }
            if let Some(stop) = &assistant.stop_reason {
                if let Ok(value) = serde_json::to_value(stop) {
                    message["stopReason"] = value;
                }
            }
            let _ = session.append_entry(pi_session::SessionEntry {
                id: String::new(),
                entry_type: "message".into(),
                parent_id: None,
                seq: 0,
                timestamp,
                message: Some(message),
                custom_type: None,
                extra: serde_json::Map::new(),
            });
        }
    }
}

fn tool_result_message(
    id: &str,
    name: &str,
    result: crate::ToolResult,
    auto_resize_images: bool,
) -> ChatMessage {
    let mut content = vec![MessageContent::Text {
        text: result.content,
    }];
    if let Some(details) = &result.details {
        if let Some(image) = details.get("image") {
            if let (Some(data), Some(mime_type)) = (
                image.get("data").and_then(Value::as_str),
                image
                    .get("mimeType")
                    .or_else(|| image.get("mime_type"))
                    .and_then(Value::as_str),
            ) {
                content.push(MessageContent::Image {
                    data: data.to_string(),
                    mime_type: mime_type.to_string(),
                });
            }
        }
        if let Some(images) = details.get("images").and_then(Value::as_array) {
            content.extend(crate::parse_rpc_images(images));
        }
    }
    content = crate::normalize_tool_result_images(&content, auto_resize_images);
    ChatMessage {
        role: "toolResult".into(),
        content,
        tool_call_id: Some(id.to_string()),
        tool_name: Some(name.to_string()),
        is_error: Some(result.is_error),
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
