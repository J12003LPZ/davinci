use std::path::Path;

use pi_ai::{
    assistant_to_chat, AssistantMessage, ChatMessage, ContentBlock, MessageContent, StopReason,
};
use serde_json::Value;

use crate::events::AgentEvent;
use crate::tools::execute_tool_with;
use crate::Agent;
use crate::ToolExecutionMode;

/// What stage one decided about a call.
pub(crate) enum Preparation {
    /// The call never runs; this is its result (a block, an unknown tool,
    /// a permission refusal).
    Immediate(crate::ToolResult),
    /// The call runs, in this lane.
    Ready { lane: crate::scheduler::ToolLane },
}

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
        let prompt_messages = if emit_prompt_messages {
            let pending = std::mem::take(&mut self.pending_prompt_messages);
            if pending.is_empty() {
                self.messages
                    .last()
                    .filter(|message| message.role == "user")
                    .cloned()
                    .into_iter()
                    .collect()
            } else {
                pending
            }
        } else {
            Vec::new()
        };
        let mut new_messages = prompt_messages.clone();
        self.push_event(&mut events, AgentEvent::AgentStart);
        self.push_event(&mut events, AgentEvent::TurnStart);

        for prompt in prompt_messages {
            self.push_event(
                &mut events,
                AgentEvent::MessageStart {
                    message: prompt.clone(),
                },
            );
            self.push_event(&mut events, AgentEvent::MessageEnd { message: prompt });
        }

        loop {
            if self.abort_requested() {
                self.push_event(
                    &mut events,
                    AgentEvent::AgentEnd {
                        messages: new_messages,
                        will_retry: false,
                    },
                );
                self.is_streaming = false;
                self.flush_pending_bash_messages();
                return Ok(events);
            }

            self.inject_queued(&mut events, &mut new_messages, true);
            self.inject_job_notices(&mut events, &mut new_messages);

            // Old tool output leaves the provider's view first; compaction
            // is the expensive fallback when that is not enough.
            self.prune_context();
            let tokens = self.estimated_context_tokens();
            self.stats.note_context(tokens);
            if self.auto_compaction {
                let mut settings = self.compaction;
                settings.enabled = true;
                if crate::should_compact(tokens, self.context_window, &settings)
                    && self.compact(None).compacted
                {
                    self.stats.compactions += 1;
                }
            }

            self.stats.model_turns += 1;
            let model_started = std::time::Instant::now();
            let (assistant, stream_events, streamed_live) =
                match self.complete_with_retry(&mut complete, &mut events) {
                    Ok(output) => output,
                    Err(err) => {
                        self.is_streaming = false;
                        self.flush_pending_bash_messages();
                        return Err(err);
                    }
                };
            self.stats.model_wall_ms += model_started.elapsed().as_millis() as u64;
            let chat = assistant_to_chat(&assistant);
            self.messages.push(chat.clone());
            self.persist_assistant(&assistant, &chat);
            new_messages.push(chat.clone());
            // A closure that streamed live has already shown the sink the
            // start and every update; they are recorded here, not resent.
            let start = AgentEvent::MessageStart {
                message: chat.clone(),
            };
            if streamed_live {
                events.push(start);
            } else {
                self.push_event(&mut events, start);
            }
            let updates = stream_events.unwrap_or_else(|| pi_ai::events_from_complete(&assistant));
            let shared_chat = std::sync::Arc::new(chat.clone());
            for assistant_message_event in updates {
                let update = AgentEvent::MessageUpdate {
                    message: std::sync::Arc::clone(&shared_chat),
                    assistant_message_event,
                };
                if streamed_live {
                    events.push(update);
                } else {
                    self.push_event(&mut events, update);
                }
            }
            self.push_event(
                &mut events,
                AgentEvent::MessageEnd {
                    message: chat.clone(),
                },
            );

            if matches!(
                assistant.stop_reason,
                Some(StopReason::Error) | Some(StopReason::Aborted)
            ) {
                self.push_event(
                    &mut events,
                    AgentEvent::TurnEnd {
                        message: chat,
                        tool_results: Vec::new(),
                    },
                );
                self.push_event(
                    &mut events,
                    AgentEvent::AgentEnd {
                        messages: new_messages,
                        will_retry: false,
                    },
                );
                self.is_streaming = false;
                self.flush_pending_bash_messages();
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
                        self.push_event(
                            &mut events,
                            AgentEvent::ToolExecutionStart {
                                tool_call_id: id.clone(),
                                tool_name: name.clone(),
                                args: args.clone(),
                            },
                        );
                        self.push_event(
                            &mut events,
                            AgentEvent::ToolExecutionEnd {
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
                                details: None,
                            },
                        );
                        self.messages.push(result.clone());
                        self.persist_chat(&result);
                        new_messages.push(result.clone());
                        self.push_event(
                            &mut events,
                            AgentEvent::MessageStart {
                                message: result.clone(),
                            },
                        );
                        self.push_event(
                            &mut events,
                            AgentEvent::MessageEnd {
                                message: result.clone(),
                            },
                        );
                        tool_results.push(result);
                    }
                } else {
                    let cwd = self.cwd.clone();
                    let messages = self.execute_tool_batch(&cwd, tool_calls, &mut events);
                    for result in messages {
                        let name = result.tool_name.clone().unwrap_or_default();
                        self.after_tool(&name, &result);
                        self.messages.push(result.clone());
                        self.persist_chat(&result);
                        new_messages.push(result.clone());
                        self.push_event(
                            &mut events,
                            AgentEvent::MessageStart {
                                message: result.clone(),
                            },
                        );
                        self.push_event(
                            &mut events,
                            AgentEvent::MessageEnd {
                                message: result.clone(),
                            },
                        );
                        tool_results.push(result);
                    }
                }
            }

            self.push_event(
                &mut events,
                AgentEvent::TurnEnd {
                    message: chat,
                    tool_results,
                },
            );

            if had_tools && !self.abort_requested() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                continue;
            }

            if !self.queues.steer.is_empty() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                continue;
            }

            if !self.queues.follow_up.is_empty() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                self.inject_queued(&mut events, &mut new_messages, false);
                continue;
            }

            break;
        }

        self.push_event(
            &mut events,
            AgentEvent::AgentEnd {
                messages: new_messages,
                will_retry: false,
            },
        );
        self.is_streaming = false;
        self.flush_pending_bash_messages();
        Ok(events)
    }

    /// Background jobs that finished since the last step are told to the
    /// model here, between one completion and the next — never inside a
    /// tool call, and never twice.
    fn inject_job_notices(
        &mut self,
        events: &mut Vec<AgentEvent>,
        new_messages: &mut Vec<ChatMessage>,
    ) {
        for notice in self.job_notice_messages() {
            self.messages.push(notice.clone());
            self.persist_chat(&notice);
            new_messages.push(notice.clone());
            self.push_event(
                events,
                AgentEvent::MessageStart {
                    message: notice.clone(),
                },
            );
            self.push_event(events, AgentEvent::MessageEnd { message: notice });
        }
    }

    /// What a finished tool owes the session beyond its result: the `todo`
    /// ledger is written after every change so a resume finds it.
    fn after_tool(&mut self, name: &str, result: &ChatMessage) {
        if name == "todo" && result.is_error != Some(true) {
            self.persist_todos();
        }
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
            let _ = self.pending_prompt_messages.pop();
            new_messages.push(message.clone());
            self.push_event(
                events,
                AgentEvent::MessageStart {
                    message: message.clone(),
                },
            );
            self.push_event(events, AgentEvent::MessageEnd { message });
        }
    }

    fn complete_with_retry<F, T>(
        &mut self,
        complete: &mut F,
        events: &mut Vec<AgentEvent>,
    ) -> Result<
        (
            AssistantMessage,
            Option<Vec<pi_ai::AssistantMessageEvent>>,
            bool,
        ),
        String,
    >
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
            if self.abort_requested() || self.retry_aborted {
                if scheduled_attempt > 0 {
                    self.push_event(
                        events,
                        AgentEvent::AutoRetryEnd {
                            success: false,
                            attempt: scheduled_attempt,
                            final_error: Some("Retry cancelled".into()),
                        },
                    );
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
                    false,
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
                        self.push_event(
                            events,
                            AgentEvent::AutoRetryStart {
                                attempt: scheduled_attempt,
                                max_attempts: max_retries,
                                delay_ms: delay,
                                error_message: message
                                    .error_message
                                    .clone()
                                    .unwrap_or_else(|| "Unknown error".into()),
                            },
                        );
                        sleep_retry_delay(delay);
                        continue;
                    }
                    if scheduled_attempt > 0 {
                        let success = message.stop_reason != Some(StopReason::Error);
                        self.push_event(
                            events,
                            AgentEvent::AutoRetryEnd {
                                success,
                                attempt: scheduled_attempt,
                                final_error: if success {
                                    None
                                } else {
                                    message.error_message.clone()
                                },
                            },
                        );
                    }
                    return Ok((message, output.stream_events, output.streamed_live));
                }
                Err(err) => {
                    last_error = Some(err.clone());
                    if attempt + 1 < attempts && pi_ai::is_retryable_error_text(&err) {
                        scheduled_attempt = attempt + 1;
                        let delay = retry_delay_ms(self.retry_base_delay_ms, attempt);
                        self.push_event(
                            events,
                            AgentEvent::AutoRetryStart {
                                attempt: scheduled_attempt,
                                max_attempts: max_retries,
                                delay_ms: delay,
                                error_message: err,
                            },
                        );
                        sleep_retry_delay(delay);
                    } else {
                        // A refused request (a 400, a bad key) comes back the
                        // same every time; TS fails at once and so does this.
                        break;
                    }
                }
            }
        }
        if scheduled_attempt > 0 {
            self.push_event(
                events,
                AgentEvent::AutoRetryEnd {
                    success: false,
                    attempt: scheduled_attempt,
                    final_error: last_error.clone(),
                },
            );
        }
        Err(last_error.unwrap_or_else(|| "Provider request failed".into()))
    }

    /// Run every tool call of one assistant message and return their
    /// result messages in source order.
    ///
    /// Three stages, as in TS `agent-loop.ts`: *prepare* (the extension
    /// hook, the unknown-tool check and the permission gate, on this thread
    /// and in order, so the approver is asked one question at a time),
    /// *run* (the scheduler overlaps what may overlap, see `scheduler.rs`),
    /// and *finalize* (the post hook and the events, emitted live as each
    /// call ends and recorded here in source order).
    fn execute_tool_batch(
        &mut self,
        cwd: &Path,
        tool_calls: Vec<(String, String, Value)>,
        events: &mut Vec<AgentEvent>,
    ) -> Vec<ChatMessage> {
        let width = tool_calls.len();
        self.stats.note_batch(width);
        let started_at = std::time::Instant::now();
        let sequential = self.tool_execution_mode == ToolExecutionMode::Sequential;
        let (outcomes, report) = {
            let agent: &Agent = self;
            let mut scheduled = Vec::with_capacity(width);
            for (id, name, args) in &tool_calls {
                if agent.abort_requested() {
                    break;
                }
                let preparation = agent.prepare_tool_call(cwd, id, name, args, 0);
                let lane = match &preparation {
                    Preparation::Ready { lane } => *lane,
                    Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
                };
                let (id, name, args) = (id.clone(), name.clone(), args.clone());
                scheduled.push(crate::scheduler::ScheduledCall {
                    lane,
                    run: Box::new(move || {
                        let result = match preparation {
                            Preparation::Immediate(result) => result,
                            Preparation::Ready { .. } => {
                                agent.run_prepared_call(cwd, &id, &name, &args, 0)
                            }
                        };
                        agent.finalize_tool_call(cwd, &id, &name, &args, result)
                    }),
                });
            }
            let abort = agent.abort_signal.clone();
            let mut starts: Vec<AgentEvent> = Vec::new();
            let (outcomes, report) = crate::scheduler::run_lanes(
                scheduled,
                sequential,
                crate::scheduler::MAX_TOOL_PARALLELISM,
                abort.as_deref(),
                |group| {
                    for index in group {
                        let (id, name, args) = &tool_calls[*index];
                        let event = AgentEvent::ToolExecutionStart {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            args: args.clone(),
                        };
                        agent.emit_live(event.clone());
                        starts.push(event);
                    }
                },
            );
            // Starts were shown live in group order; the record keeps them
            // ahead of the ends they belong to.
            events.extend(starts);
            (outcomes, report)
        };
        self.stats.parallel_groups += report.parallel_groups as u64;
        self.stats.tool_wall_ms += started_at.elapsed().as_millis() as u64;
        let mut messages = Vec::with_capacity(outcomes.len());
        for (message, local_events) in outcomes {
            events.extend(local_events);
            messages.push(message);
        }
        messages
    }

    /// Stage one of a tool call. `depth` is 0 for a call the model made and
    /// 1 for an operation inside a `batch`.
    pub(crate) fn prepare_tool_call(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        depth: usize,
    ) -> Preparation {
        let immediate = |content: String, denied: bool| {
            Preparation::Immediate(crate::ToolResult {
                content,
                is_error: true,
                details: denied.then(|| serde_json::json!({ "denied": true })),
            })
        };
        if depth > 0 && matches!(name, "batch" | "agent") {
            return immediate(
                format!("`{name}` cannot run inside a batch; call it directly."),
                false,
            );
        }
        if let Some(reason) = self.pre_tool.as_ref().and_then(|hook| (hook.0)(name, args)) {
            return immediate(reason, false);
        }
        if !self.tools.iter().any(|tool| tool == name) {
            return immediate(format!("Unknown tool: {name}"), false);
        }
        if let Some(reason) = self.permission_denial(cwd, id, name, args) {
            // `denied` marks a call that never ran, for the hosts' rows
            // and the post-tool hooks, without sniffing the text.
            return immediate(reason, true);
        }
        let class = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .class_of(name);
        let lane = if !crate::tools::BUILTIN_TOOLS.contains(&name) && !name.starts_with("mcp__") {
            // An extension tool has state the runtime cannot see.
            crate::scheduler::ToolLane::Serial
        } else {
            crate::scheduler::lane_for(name, class)
        };
        Preparation::Ready { lane }
    }

    /// Stage two: the call itself. Takes `&self` only, so it may run on a
    /// worker thread next to its siblings.
    pub(crate) fn run_prepared_call(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        depth: usize,
    ) -> crate::ToolResult {
        if name == "agent" {
            let workers = args
                .get("tasks")
                .and_then(Value::as_array)
                .map(Vec::len)
                .filter(|count| *count > 0)
                .unwrap_or(1);
            crate::stats::SharedCounters::add(&self.counters.subagents, workers as u64);
            let parent = crate::subagent::SubagentParent {
                provider: Some(self.provider.clone()),
                model_id: Some(self.model_id.clone()),
                abort: self.abort_signal.clone(),
            };
            return match crate::subagent::run_tool(
                args,
                &self.tools,
                self.subagent_runner.as_ref(),
                &parent,
            ) {
                Ok(result) => result,
                Err(err) => crate::ToolResult {
                    content: err.to_string(),
                    is_error: true,
                    details: None,
                },
            };
        }
        if name == "batch" && depth == 0 {
            return self.run_batch(cwd, id, args);
        }
        // The tool sees the turn's abort flag so a long shell command
        // or a `job_output` wait ends when the user interrupts.
        let mut context = self.tool_context.clone();
        context.abort = self.abort_signal.clone();
        match execute_tool_with(cwd, name, args, &context) {
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
    }

    /// Stage three: the post hook, the events (sent to the sink now, and
    /// returned so the caller records them in source order) and the message.
    fn finalize_tool_call(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        mut result: crate::ToolResult,
    ) -> (ChatMessage, Vec<AgentEvent>) {
        let mut events = Vec::new();
        if let Some(hook) = &self.post_tool {
            result = (hook.0)(id, cwd, name, args, result);
        }
        let mut details = result.details.clone();
        for partial in crate::ToolResult::take_updates(&mut details) {
            let event = AgentEvent::ToolExecutionUpdate {
                tool_call_id: id.to_string(),
                tool_name: name.to_string(),
                args: args.clone(),
                partial_result: partial,
            };
            self.emit_live(event.clone());
            events.push(event);
        }
        let update = AgentEvent::ToolExecutionUpdate {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: args.clone(),
            partial_result: Value::String(result.content.clone()),
        };
        self.emit_live(update.clone());
        events.push(update);
        let end = AgentEvent::ToolExecutionEnd {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            result: Value::String(result.content.clone()),
            is_error: result.is_error,
            details: event_details(details.as_ref()),
        };
        self.emit_live(end.clone());
        events.push(end);
        (
            tool_result_message(id, name, result, self.auto_resize_images),
            events,
        )
    }

    /// The permission gate: `None` lets the call run, `Some(reason)` is the
    /// error result the model gets instead. Sits after the extension hook (a
    /// block there wins) and after the unknown-tool check (nobody is asked
    /// about a tool that does not exist).
    fn permission_denial(&self, cwd: &Path, id: &str, name: &str, args: &Value) -> Option<String> {
        use crate::{PermissionVerdict, ToolApprovalDecision};
        let verdict = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .decide(id, name, args, cwd);
        let request = match verdict {
            PermissionVerdict::Allow => return None,
            PermissionVerdict::Deny { reason } => return Some(reason),
            PermissionVerdict::Ask(request) => request,
        };
        let Some(approver) = &self.approver else {
            return Some(format!(
                "Permission denied: `{}` needs approval in permission mode `{}`, and this run cannot ask. \
                 Start pi with --permission-mode auto, or add an allow rule such as `{}` to \
                 ~/.pi/agent/settings.json under permissions.allow.",
                request.summary,
                request.mode.as_str(),
                request.session_rule
            ));
        };
        match (approver.0)(&request) {
            ToolApprovalDecision::AllowOnce => None,
            ToolApprovalDecision::AllowForSession | ToolApprovalDecision::AllowAlways => {
                self.permissions
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .remember(&request.session_rule);
                None
            }
            ToolApprovalDecision::Deny => Some(format!(
                "Permission denied: the user declined `{}`.",
                request.summary
            )),
        }
    }

    fn persist_chat(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let content = serde_json::to_value(&message.content).unwrap_or(Value::Null);
            let _ = session.append_entry(crate::chat_entry(&message.role, content, &message.extra));
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

/// A tool's details as an event carries them: the same object without the
/// image payloads, which belong in the message and not in every sink.
fn event_details(details: Option<&Value>) -> Option<Value> {
    let Value::Object(map) = details? else {
        return None;
    };
    let kept: serde_json::Map<String, Value> = map
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "image" | "images" | "_piUpdates"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    (!kept.is_empty()).then_some(Value::Object(kept))
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
        ..ChatMessage::default()
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
