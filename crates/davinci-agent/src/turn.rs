use std::path::Path;

use davinci_ai::{
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
    /// A duplicate in-flight call waiting for the leader's result.
    Wait {
        call_id: String,
        lane: crate::scheduler::ToolLane,
    },
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
        let mut capability_completion_reminders = 0_u32;
        let mut verification_reminded_generation = None;
        self.push_event(&mut events, AgentEvent::AgentStart);
        self.push_event(&mut events, AgentEvent::TurnStart);
        if let Some(runtime) = &self.runtime {
            runtime.emit_observe(crate::runtime::RuntimeEvent::TurnStarted);
        }

        if !prompt_messages.is_empty() {
            if let Some(runtime) = &self.runtime {
                let _ = runtime.emit_decision(crate::runtime::RuntimeEvent::UserPromptSubmitted);
            }
        }

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
                self.emit_behavior_telemetry();
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

            if self
                .last_prepared_manifest
                .as_ref()
                .is_some_and(|m| m.is_mandatory_violated())
            {
                self.is_streaming = false;
                self.flush_pending_bash_messages();
                return Err(
                    "Request blocked: mandatory context policy is violated or unavailable".into(),
                );
            }

            self.stats.model_turns += 1;
            let model_started = std::time::Instant::now();
            let completion = self.complete_with_retry(&mut complete, &mut events);
            self.stats.model_wall_ms += model_started.elapsed().as_millis() as u64;
            let (assistant, stream_events, streamed_live) = match completion {
                Ok(output) => output,
                Err(err) => {
                    self.is_streaming = false;
                    self.flush_pending_bash_messages();
                    return Err(err);
                }
            };
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
            let updates =
                stream_events.unwrap_or_else(|| davinci_ai::events_from_complete(&assistant));
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

            if tool_calls.is_empty() {
                let capability_state = self.capability_run_state();
                match crate::prompt::evaluate_completion(
                    &capability_state,
                    capability_completion_reminders,
                ) {
                    crate::prompt::CapabilityGateOutcome::AllowCompletion => {
                        if capability_completion_reminders
                            >= crate::prompt::MAX_CAPABILITY_COMPLETION_REMINDERS
                            && crate::prompt::incomplete_evidence_reason(&capability_state)
                                .is_some()
                        {
                            self.stats.capability_incomplete_evidence =
                                self.stats.capability_incomplete_evidence.saturating_add(1);
                        }
                    }
                    crate::prompt::CapabilityGateOutcome::ContinueWithReminder {
                        message,
                        reason_code,
                    } => {
                        capability_completion_reminders =
                            capability_completion_reminders.saturating_add(1);
                        self.queue_capability_reminder(
                            &message,
                            &reason_code,
                            &mut events,
                            &mut new_messages,
                        );
                        continue;
                    }
                }

                let completion_evidence = self.completion_evidence();
                let mutation_generation = self.mutation_verification_state().mutation_generation;
                if matches!(
                    completion_evidence,
                    crate::CompletionEvidence::Unverified
                        | crate::CompletionEvidence::VerificationFailed
                ) && verification_reminded_generation != Some(mutation_generation)
                {
                    verification_reminded_generation = Some(mutation_generation);
                    let message = match completion_evidence {
                        crate::CompletionEvidence::VerificationFailed => {
                            "The latest verification command failed after a file change. Investigate the failure or report it explicitly before finalizing."
                        }
                        _ => {
                            "You changed files but have not completed a verification command. Run the narrowest appropriate test, check, or lint command before finalizing."
                        }
                    };
                    self.queue_capability_reminder(
                        message,
                        "verification_required",
                        &mut events,
                        &mut new_messages,
                    );
                    continue;
                }
            }

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
                    for mut result in messages {
                        let name = result.tool_name.clone().unwrap_or_default();
                        self.after_tool(&name, &mut result);
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
            if let Some(runtime) = &self.runtime {
                runtime.emit_observe(crate::runtime::RuntimeEvent::TurnEnded {
                    success: !self.abort_requested(),
                });
            }

            if had_tools && !self.abort_requested() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                if let Some(runtime) = &self.runtime {
                    runtime.emit_observe(crate::runtime::RuntimeEvent::TurnStarted);
                }
                continue;
            }

            if !self.queues.steer.is_empty() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                if let Some(runtime) = &self.runtime {
                    runtime.emit_observe(crate::runtime::RuntimeEvent::TurnStarted);
                }
                continue;
            }

            if !self.queues.follow_up.is_empty() {
                self.push_event(&mut events, AgentEvent::TurnStart);
                if let Some(runtime) = &self.runtime {
                    runtime.emit_observe(crate::runtime::RuntimeEvent::TurnStarted);
                }
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
        self.emit_behavior_telemetry();
        Ok(events)
    }

    fn queue_capability_reminder(
        &mut self,
        message_text: &str,
        reason_code: &str,
        events: &mut Vec<AgentEvent>,
        new_messages: &mut Vec<ChatMessage>,
    ) {
        let mut message = ChatMessage::text("user", message_text);
        message.extra.insert(
            "davinciCapabilityReminder".into(),
            Value::String(reason_code.to_string()),
        );
        self.messages.push(message.clone());
        self.persist_full_message(&message);
        new_messages.push(message.clone());
        self.push_event(
            events,
            AgentEvent::MessageStart {
                message: message.clone(),
            },
        );
        self.push_event(events, AgentEvent::MessageEnd { message });
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
    fn after_tool(&mut self, name: &str, result: &mut ChatMessage) {
        if matches!(name, "todo" | "update_plan") && result.is_error != Some(true) {
            self.persist_todos();
        }
        // Structured plans were already persisted transactionally in
        // execute_plan_batch, before the first success event was emitted.
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
        if steer {
            self.stats.user_steers += drained.len() as u64;
        }
        let batch_prepared = if drained.len() > 1 {
            let texts: Vec<&str> = drained.iter().map(|queued| queued.text.as_str()).collect();
            self.prepare_builtin_prompt_for_user_turn_batch(&texts)
                .is_ok()
        } else {
            false
        };
        for queued in drained {
            let message = if batch_prepared {
                self.prompt_user_with_prepared(&queued.text, &queued.images)
            } else {
                self.prompt_user_with(&queued.text, &queued.images)
            };
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

        let maybe_runtime = self.tool_context.runtime.clone();
        if let Some(runtime) = maybe_runtime {
            let limit = 10;
            let agent_msgs = runtime.mailbox.drain(runtime.agent_id, limit);
            for msg in agent_msgs {
                let gen = runtime.registry.get_generation(&runtime.agent_id);
                if runtime
                    .mailbox
                    .mark_applied(&runtime.agent_id, gen, &msg.id)
                {
                    let message = self.prompt_with(&msg.content, &[]);
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
        }
    }

    fn complete_with_retry<F, T>(
        &mut self,
        complete: &mut F,
        events: &mut Vec<AgentEvent>,
    ) -> Result<
        (
            AssistantMessage,
            Option<Vec<davinci_ai::AssistantMessageEvent>>,
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
            let permit = crate::runtime::capacity::REQUEST_CAPACITY
                .acquire(crate::runtime::capacity::RequestClass::Foreground, || {
                    self.abort_requested() || self.retry_aborted
                });
            if permit.is_none() {
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
            if attempt > 0 {
                self.stats.provider_retries += 1;
                if let Some(runtime) = &self.runtime {
                    if let Some(ledger) = &runtime.budget_ledger {
                        let _ = ledger.record_retry();
                    }
                }
            }
            let result = complete(self);
            drop(permit);
            match result {
                Ok(output) => {
                    let output = output.into();
                    let message = output.message;
                    if let Some(runtime) = &self.runtime {
                        if let Some(ledger) = &runtime.budget_ledger {
                            let total_tokens =
                                message.usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                            let cache_read =
                                message.usage.as_ref().map(|u| u.cache_read).unwrap_or(0);
                            let cache_write =
                                message.usage.as_ref().map(|u| u.cache_write).unwrap_or(0);
                            let cost = if let Some(usage) = &message.usage {
                                if usage.cost.total > 0.0 {
                                    crate::runtime::CostAmount::Known(
                                        (usage.cost.total * 10_000.0) as u64,
                                    )
                                } else {
                                    crate::runtime::CostAmount::Unknown
                                }
                            } else {
                                crate::runtime::CostAmount::Unknown
                            };
                            let receipt = crate::runtime::UsageReceipt {
                                attempt_id: format!("provider_attempt_{}_{}", message.id, attempt),
                                provider_usage: total_tokens,
                                estimated_usage: self.estimated_context_tokens(),
                                cache_read_tokens: cache_read,
                                cache_write_tokens: cache_write,
                                cost,
                                finished_at: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                            };
                            let _ = ledger.settle_receipt(None, receipt);
                            self.stats.apply_budget_snapshot(&ledger.snapshot());
                        }
                    }
                    if message.stop_reason == Some(StopReason::Error)
                        && davinci_ai::is_retryable_assistant_error(&message)
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
                        sleep_retry_delay(delay, || self.abort_requested() || self.retry_aborted);
                        continue;
                    }
                    if scheduled_attempt > 0 {
                        let success = !matches!(
                            message.stop_reason,
                            Some(StopReason::Error | StopReason::Aborted)
                        );
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
                    if attempt + 1 < attempts && davinci_ai::is_retryable_error_text(&err) {
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
                        sleep_retry_delay(delay, || self.abort_requested() || self.retry_aborted);
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
        if tool_calls
            .iter()
            .any(|(_, name, _)| matches!(name.as_str(), "propose_plan" | "ask_user_question"))
        {
            return self.execute_plan_batch(cwd, tool_calls, events);
        }
        let width = tool_calls.len();
        self.stats.note_batch(width);
        let started_at = std::time::Instant::now();
        let sequential = self.tool_execution_mode == ToolExecutionMode::Sequential;
        let mut immediate_results = Vec::with_capacity(width);
        let mut owned_reservations = Vec::with_capacity(width);
        let (mut outcomes, report) = {
            let agent: &Agent = self;
            let mut scheduled = Vec::with_capacity(width);
            for (id, name, args) in &tool_calls {
                if agent.abort_requested() {
                    break;
                }
                let preparation = agent.prepare_tool_call(cwd, id, name, args, 0);
                owned_reservations.push(matches!(&preparation, Preparation::Ready { .. }));
                immediate_results.push(match &preparation {
                    Preparation::Immediate(result) => Some(result.clone()),
                    _ => None,
                });
                let lane = match &preparation {
                    Preparation::Ready { lane } => *lane,
                    Preparation::Wait { lane, .. } => *lane,
                    Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
                };
                let (id, name, args) = (id.clone(), name.clone(), args.clone());
                scheduled.push(crate::scheduler::ScheduledCall {
                    lane,
                    run: Box::new(move || {
                        let result = match preparation {
                            Preparation::Immediate(result) => result,
                            Preparation::Wait { call_id, .. } => agent.wait_for_tool_call(&call_id),
                            Preparation::Ready { .. } => {
                                agent.run_prepared_call(cwd, &id, &name, &args, 0)
                            }
                        };
                        agent.finalize_tool_call(cwd, &id, &name, &args, result)
                    }),
                });
            }
            let mut starts: Vec<AgentEvent> = Vec::new();
            let (outcomes, report) = crate::scheduler::run_lanes_with_cancel(
                scheduled,
                sequential,
                crate::scheduler::MAX_TOOL_PARALLELISM,
                || agent.abort_requested(),
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
        // The scheduler returns only the started prefix. Every remaining call
        // still needs a terminal result, including a denial that cancelled the
        // turn during preparation. Never run its executor to obtain that result.
        for (index, (id, name, args)) in tool_calls.iter().enumerate().skip(outcomes.len()) {
            let result = immediate_results
                .get_mut(index)
                .and_then(Option::take)
                .unwrap_or_else(|| crate::ToolResult {
                    content: "Tool execution cancelled before dispatch.".into(),
                    is_error: true,
                    details: Some(serde_json::json!({"denied": true, "cancelled": true})),
                });
            if owned_reservations.get(index) == Some(&true) {
                if let Ok(mut ledger) = self.tool_ledger.lock() {
                    ledger.cancel_reservation(id);
                }
            }
            outcomes.push(self.finalize_tool_call(cwd, id, name, args, result));
        }
        self.stats.parallel_groups += report.parallel_groups as u64;
        self.stats.tool_wall_ms += started_at.elapsed().as_millis() as u64;
        let mut messages = Vec::with_capacity(outcomes.len());
        for (message, local_events) in outcomes {
            events.extend(local_events);
            messages.push(message);
        }
        if let Some(runtime) = &self.runtime {
            let failures = messages
                .iter()
                .filter(|m| m.is_error.unwrap_or(false))
                .count();
            runtime.emit_observe(crate::runtime::RuntimeEvent::PostToolBatch {
                calls: width,
                failures,
            });
        }
        messages
    }

    /// Plan updates are session transactions; this mixed batch is serialized
    /// so persistence finishes before live success. Other batches stay parallel.
    fn execute_plan_batch(
        &mut self,
        cwd: &Path,
        calls: Vec<(String, String, Value)>,
        events: &mut Vec<AgentEvent>,
    ) -> Vec<ChatMessage> {
        let width = calls.len();
        self.stats.note_batch(width);
        let started = std::time::Instant::now();
        let mut messages = Vec::with_capacity(width);
        for (id, name, args) in calls {
            if self.abort_requested() {
                let result = crate::ToolResult {
                    content: "Tool execution cancelled before dispatch.".into(),
                    is_error: true,
                    details: Some(serde_json::json!({"denied": true, "cancelled": true})),
                };
                let (message, local_events) =
                    self.finalize_tool_call(cwd, &id, &name, &args, result);
                events.extend(local_events);
                messages.push(message);
                continue;
            }
            let preparation = self.prepare_tool_call(cwd, &id, &name, &args, 0);
            self.push_event(
                events,
                AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    args: args.clone(),
                },
            );
            let before = matches!(name.as_str(), "propose_plan" | "ask_user_question").then(|| {
                self.tool_context
                    .living_plan
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            });
            let mut result = match preparation {
                Preparation::Immediate(result) => result,
                Preparation::Wait { call_id, .. } => self.wait_for_tool_call(&call_id),
                Preparation::Ready { .. } => self.run_prepared_call(cwd, &id, &name, &args, 0),
            };
            let replayed = result
                .details
                .as_ref()
                .and_then(|d| d.get("replayed_from_ledger"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let plan_call = before.is_some();
            if let Some(mut before) = before {
                let executed = !result.is_error && !replayed;
                let execution_failed = result.is_error;
                // Validation/output middleware must run before the session
                // commit, exactly once, and cannot revive a denied execution.
                if let Some(hook) = &self.post_tool {
                    result = (hook.0)(&id, cwd, &name, &args, result);
                }
                result.is_error |= execution_failed;
                if executed && result.is_error {
                    before.approved_revision = None;
                    *self
                        .tool_context
                        .living_plan
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = before.clone();
                    self.sync_plan_todos();
                    self.set_permission_mode(crate::PermissionMode::ReadOnly);
                }
                if executed && !result.is_error {
                    match self.persist_plan() {
                        Ok(()) => {
                            let revision = self
                                .tool_context
                                .living_plan
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .revision;
                            if before.revision != revision {
                                self.previous_plan_revision = Some(before);
                            }
                            self.sync_plan_todos();
                        }
                        Err(error) => {
                            before.approved_revision = None;
                            *self
                                .tool_context
                                .living_plan
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = before;
                            self.sync_plan_todos();
                            self.set_permission_mode(crate::PermissionMode::ReadOnly);
                            result = crate::ToolResult {
                                content: error,
                                is_error: true,
                                details: Some(serde_json::json!({"plan_storage_error":true})),
                            };
                        }
                    }
                }
                // Only amend a real execution's transaction failure. A
                // rejected duplicate must not overwrite the original record.
                if executed && result.is_error {
                    self.tool_ledger
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record_failure(&id, &result.content);
                }
            }
            let (message, local_events) = if plan_call {
                self.emit_tool_result(&id, &name, &args, result)
            } else {
                self.finalize_tool_call(cwd, &id, &name, &args, result)
            };
            events.extend(local_events);
            messages.push(message);
        }
        self.stats.tool_wall_ms += started.elapsed().as_millis() as u64;
        if let Some(runtime) = &self.runtime {
            runtime.emit_observe(crate::runtime::RuntimeEvent::PostToolBatch {
                calls: width,
                failures: messages
                    .iter()
                    .filter(|m| m.is_error.unwrap_or(false))
                    .count(),
            });
        }
        messages
    }

    pub(crate) fn wait_for_tool_call(&self, call_id: &str) -> crate::ToolResult {
        match crate::tool_ledger::ToolCallLedger::wait_for_terminal(
            &self.tool_ledger,
            call_id,
            self.abort_signal.as_deref(),
        ) {
            Ok((cached, is_error)) => crate::ToolResult {
                content: cached,
                is_error,
                details: Some(serde_json::json!({ "replayed_from_ledger": true })),
            },
            Err(err) => crate::ToolResult {
                content: err,
                is_error: true,
                details: None,
            },
        }
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
        if depth > 0
            && matches!(
                name,
                "batch" | "agent" | "propose_plan" | "ask_user_question"
            )
        {
            return immediate(
                format!("`{name}` cannot run inside a batch; call it directly."),
                false,
            );
        }
        if let Ok(mut ledger) = self.tool_ledger.lock() {
            match ledger.reserve_call(id, name, args) {
                Err(collision_err) => {
                    return Preparation::Immediate(crate::ToolResult {
                        content: collision_err,
                        is_error: true,
                        details: Some(serde_json::json!({ "collision": true })),
                    });
                }
                Ok(crate::tool_ledger::ReservationOutcome::Replay { output, is_error }) => {
                    return Preparation::Immediate(crate::ToolResult {
                        content: output,
                        is_error,
                        details: Some(serde_json::json!({ "replayed_from_ledger": true })),
                    });
                }
                Ok(crate::tool_ledger::ReservationOutcome::WaitForInFlight) => {
                    let class = self
                        .permissions
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .class_of(name);
                    let lane = crate::scheduler::lane_for(name, class);
                    return Preparation::Wait {
                        call_id: id.to_string(),
                        lane,
                    };
                }
                Ok(crate::tool_ledger::ReservationOutcome::Reserved) => {
                    // Identity reserved as Pending; proceed to check pre_tool / permissions
                }
            }
        }
        if let Some(runtime) = &self.runtime {
            let event = crate::runtime::RuntimeEvent::PreToolUse {
                call_id: id.to_string(),
                tool: name.to_string(),
                args: args.clone(),
            };
            if let Err(reason) = runtime.emit_decision(event) {
                if let Ok(mut ledger) = self.tool_ledger.lock() {
                    ledger.cancel_reservation(id);
                }
                return immediate(reason, false);
            }
        }
        if let Some(reason) = self.pre_tool.as_ref().and_then(|hook| (hook.0)(name, args)) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.cancel_reservation(id);
            }
            return immediate(reason, false);
        }
        if !self.tools.iter().any(|tool| tool == name) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.cancel_reservation(id);
            }
            return immediate(format!("Unknown tool: {name}"), false);
        }
        if let Err(violation) = self.check_contract_gate(cwd, id, name, args) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.record_blocked(id, &violation.to_string());
            }
            return Preparation::Immediate(crate::ToolResult {
                content: violation.to_string(),
                is_error: true,
                details: Some(serde_json::json!({
                    "denied": true,
                    "scope_violation": violation,
                })),
            });
        }
        if let Some(reason) = self.capability_effect_denial(cwd, name, args) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.record_blocked(id, &reason);
            }
            return immediate(reason, true);
        }
        if let Some(reason) = self.permission_denial(cwd, id, name, args) {
            // `denied` marks a call that never ran, for the hosts' rows
            // and the post-tool hooks, without sniffing the text.
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.record_blocked(id, &reason);
            }
            return immediate(reason, true);
        }
        let class = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .class_of(name);
        // Optional task reads are built-in implementations, but are excluded
        // from BUILTIN_TOOLS because that list also enables default tools.
        let lane = if !crate::tools::BUILTIN_TOOLS.contains(&name)
            && !matches!(name, "task_get" | "task_list")
            && !name.starts_with("mcp__")
        {
            // An extension tool has state the runtime cannot see.
            crate::scheduler::ToolLane::Serial
        } else {
            crate::scheduler::lane_for(name, class)
        };
        Preparation::Ready { lane }
    }

    /// Stage two: the call itself. Takes `&self` only, so it may run on a
    /// worker thread when the turn executes calls in parallel.
    pub(crate) fn run_prepared_call(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        depth: usize,
    ) -> crate::ToolResult {
        if let Err(violation) = self.check_contract_gate(cwd, id, name, args) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.cancel_reservation(id);
            }
            return crate::ToolResult {
                content: violation.to_string(),
                is_error: true,
                details: Some(serde_json::json!({
                    "denied": true,
                    "scope_violation": violation,
                })),
            };
        }
        if let Some(reason) = self.capability_effect_denial(cwd, name, args) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.record_blocked(id, &reason);
            }
            return crate::ToolResult {
                content: reason,
                is_error: true,
                details: Some(serde_json::json!({ "denied": true })),
            };
        }
        if let Err(violation) = self.check_contract_dispatch_boundary(cwd, name) {
            if let Ok(mut ledger) = self.tool_ledger.lock() {
                ledger.cancel_reservation(id);
            }
            return crate::ToolResult {
                content: violation.to_string(),
                is_error: true,
                details: Some(serde_json::json!({
                    "denied": true,
                    "scope_violation": violation,
                })),
            };
        }

        if let Ok(mut ledger) = self.tool_ledger.lock() {
            match ledger.begin_execution(id, name, args) {
                crate::tool_ledger::BeginOutcome::Collision(collision_err) => {
                    return crate::ToolResult {
                        content: collision_err,
                        is_error: true,
                        details: Some(serde_json::json!({ "collision": true })),
                    };
                }
                crate::tool_ledger::BeginOutcome::Replay { output, is_error } => {
                    return crate::ToolResult {
                        content: output,
                        is_error,
                        details: Some(serde_json::json!({ "replayed_from_ledger": true })),
                    };
                }
                crate::tool_ledger::BeginOutcome::WaitForInFlight => {
                    drop(ledger);
                    return self.wait_for_tool_call(id);
                }
                crate::tool_ledger::BeginOutcome::Execute => {
                    // Ready to execute tool as leader
                }
            }
        }

        let outcome = if name == "agent" {
            let workers = args
                .get("tasks")
                .and_then(Value::as_array)
                .map(Vec::len)
                .filter(|count| *count > 0)
                .unwrap_or(1);
            crate::stats::SharedCounters::add(&self.counters.subagents, workers as u64);
            let token = self
                .runtime
                .as_ref()
                .map(|rt| rt.cancellation_token.clone());
            let abort = token
                .as_ref()
                .map(|t| t.as_atomic_bool())
                .or_else(|| self.abort_signal.clone());
            let permission_mode = Some(self.permission_mode());
            let parent_agent_id = self.runtime.as_ref().map(|rt| rt.agent_id);
            let worktree_manager = self
                .runtime
                .as_ref()
                .and_then(|rt| rt.worktree_manager.clone());
            let active_contract = self.active_contract();
            let contract_digest = active_contract.as_ref().map(|c| c.digest.clone());
            let parent = crate::subagent::SubagentParent {
                provider: Some(self.provider.clone()),
                model_id: Some(self.model_id.clone()),
                abort,
                cancellation_token: token,
                runtime: self.runtime.clone(),
                permission_mode,
                agent_id: parent_agent_id,
                worktree_manager,
                contract_digest,
                active_contract,
            };
            match crate::subagent::run_tool(
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
            }
        } else if name == "batch" && depth == 0 {
            self.run_batch(cwd, id, args)
        } else {
            let mutating = crate::runtime::workflow::validate::is_mutating_tool(name);
            let targets = if mutating {
                crate::runtime::contracts::extract_tool_targets(name, args)
            } else {
                Vec::new()
            };
            let task_id = self
                .active_contract()
                .map(|c| c.task_id)
                .unwrap_or_default();

            let preimages: Vec<crate::runtime::checkpoints::FileCapture> = if mutating {
                if let Some(runtime) = &self.runtime {
                    targets
                        .iter()
                        .filter_map(|target| {
                            runtime.blob_store.capture_file(task_id, cwd, target).ok()
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // The tool sees the turn's abort flag so a long shell command
            // or a `job_output` wait ends when the user interrupts.
            let mut context = self.tool_context.clone();
            context.abort = self
                .runtime
                .as_ref()
                .map(|rt| rt.cancellation_token.as_atomic_bool())
                .or_else(|| self.abort_signal.clone());
            let executed = match execute_tool_with(cwd, name, args, &context) {
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
            };

            if mutating && !executed.is_error {
                if let Some(runtime) = &self.runtime {
                    for pre in preimages {
                        if let Ok(post) = runtime.blob_store.capture_file(task_id, cwd, &pre.path) {
                            let kind = if !pre.exists && post.exists {
                                crate::runtime::effects::FileEffectKind::Created
                            } else if pre.exists && !post.exists {
                                crate::runtime::effects::FileEffectKind::Deleted
                            } else if pre.mode != post.mode {
                                crate::runtime::effects::FileEffectKind::ModeChanged
                            } else {
                                crate::runtime::effects::FileEffectKind::Modified
                            };
                            let mut effect = crate::runtime::effects::OwnedFileEffect::new(
                                id,
                                runtime.agent_id,
                                1,
                                pre.path,
                                kind,
                                task_id,
                            );
                            effect.before_blob = pre.blob_hash;
                            effect.after_blob = post.blob_hash;
                            effect.before_mode = pre.mode;
                            effect.after_mode = post.mode;
                            if let Some(report_path) = std::env::var_os("PI_GRAPH_EFFECT_REPORT") {
                                let before_bytes = effect
                                    .before_blob
                                    .as_deref()
                                    .and_then(|hash| runtime.blob_store.get_blob(hash));
                                let after_bytes = effect
                                    .after_blob
                                    .as_deref()
                                    .and_then(|hash| runtime.blob_store.get_blob(hash));
                                // The in-process ledger remains authoritative for the
                                // parent; this best-effort sidecar is only the
                                // cross-process handoff used by graph rewind.
                                let _ = crate::runtime::effects::append_effect_report(
                                    std::path::Path::new(&report_path),
                                    &effect,
                                    before_bytes.as_deref(),
                                    after_bytes.as_deref(),
                                );
                            }
                            if let Ok(mut effects) = runtime.effect_ledger.write() {
                                effects.push(effect);
                            }
                        }
                    }
                }
            }

            executed
        };
        if let Ok(mut ledger) = self.tool_ledger.lock() {
            if outcome.is_error {
                ledger.record_failure(id, &outcome.content);
            } else {
                ledger.record_completion(id, &outcome.content, false);
            }
        }
        if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit") && !outcome.is_error {
            crate::stats::SharedCounters::add(&self.counters.files_changed_count, 1);
        }
        if matches!(name, "bash" | "powershell" | "exec_command") {
            let cmd = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if is_verification_command(cmd) {
                crate::stats::SharedCounters::add(&self.counters.verification_commands_run, 1);
                if outcome.is_error {
                    crate::stats::SharedCounters::add(&self.counters.verification_failures, 1);
                }
            }
        }
        outcome
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
        let terminal_markers = ["denied", "cancelled"].map(|key| {
            (
                key,
                result
                    .details
                    .as_ref()
                    .and_then(|d| d.get(key))
                    .and_then(Value::as_bool)
                    == Some(true),
            )
        });
        let storage_failure = result
            .details
            .as_ref()
            .and_then(|d| d.get("plan_storage_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let replayed = result
            .details
            .as_ref()
            .and_then(|details| details.get("replayed_from_ledger"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let pre_hook_error = result.is_error;
        let pre_hook_result = result.clone();
        if !storage_failure {
            if let Some(hook) = &self.post_tool {
                result = (hook.0)(id, cwd, name, args, result);
            }
        }
        // Output decoration cannot turn an undispatched operation into success.
        for (key, protected) in terminal_markers {
            if protected {
                result.is_error = true;
                let details = result.details.get_or_insert_with(|| serde_json::json!({}));
                if !details.is_object() {
                    *details = serde_json::json!({});
                }
                details[key] = Value::Bool(true);
            }
        }
        if !replayed {
            if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit")
                && !pre_hook_error
                && !result.is_error
            {
                self.record_successful_mutation();
            }
            if matches!(name, "bash" | "powershell" | "exec_command") {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if is_verification_command(cmd) {
                    self.record_verification_result(!pre_hook_error && !result.is_error);
                }
            }
        }
        let hook_vetoed = !pre_hook_error && result.is_error;
        self.record_receipt(cwd, id, name, args, &pre_hook_result, &result, hook_vetoed);
        self.emit_tool_result(id, name, args, result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_receipt(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        pre_hook: &crate::ToolResult,
        _post_hook: &crate::ToolResult,
        hook_vetoed: bool,
    ) {
        let (started, permission_denied, cancelled) = if let Some(details) = &pre_hook.details {
            let denied = details
                .get("denied")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let cancelled = details
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let not_dispatched = details
                .get("not_dispatched")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (!denied && !not_dispatched, denied, cancelled)
        } else {
            (true, false, false)
        };

        let fallback_exit = if !pre_hook.is_error && started {
            Some(0)
        } else {
            None
        };

        let exit_code = pre_hook
            .details
            .as_ref()
            .and_then(|d| d.get("exitCode").or_else(|| d.get("exit_code")))
            .and_then(Value::as_i64)
            .map(|c| c as i32)
            .or(fallback_exit);

        let timed_out = pre_hook
            .details
            .as_ref()
            .and_then(|d| d.get("timed_out").or_else(|| d.get("timeout")))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let killed = pre_hook
            .details
            .as_ref()
            .and_then(|d| d.get("killed"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let simulated = name == "dry_run_verify_exec"
            || args
                .get("simulated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || pre_hook
                .details
                .as_ref()
                .and_then(|d| d.get("simulated"))
                .and_then(Value::as_bool)
                .unwrap_or(false);

        let argv = if let Some(cmd) = args.get("command").and_then(Value::as_str) {
            vec![cmd.to_string()]
        } else {
            Vec::new()
        };

        let _receipt = crate::runtime::evidence_store::ExecutionReceipt {
            receipt_id: crate::runtime::ids::EvidenceId::new(),
            operation_id: id.to_string(),
            task_id: None,
            tool_name: name.to_string(),
            argv,
            cwd: cwd.to_string_lossy().to_string(),
            started,
            exit_code,
            timed_out,
            cancelled,
            killed,
            permission_denied,
            simulated,
            hook_vetoed,
            ..Default::default()
        };

        if let Some(runtime) = &self.runtime {
            let input_fp = if let Some(cmd) = args.get("command").and_then(Value::as_str) {
                cmd.to_string()
            } else if let Some(path) = args.get("path").and_then(Value::as_str) {
                path.to_string()
            } else {
                args.to_string()
            };

            let target_path = args
                .get("path")
                .or_else(|| args.get("file_path"))
                .and_then(Value::as_str)
                .map(|p| p.to_string());

            let edit_content_hash = if name == "edit" || name == "write" {
                args.get("content")
                    .or_else(|| args.get("new_str"))
                    .or_else(|| args.get("replacement"))
                    .and_then(Value::as_str)
                    .map(|c| {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        c.hash(&mut hasher);
                        format!("{:x}", hasher.finish())
                    })
            } else {
                None
            };

            let normalized_output =
                crate::runtime::progress_watchdog::normalize_output_noise(&pre_hook.content);
            let output_digest = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                normalized_output.hash(&mut hasher);
                format!("{:x}", hasher.finish())
            };

            let test_failure_signature =
                if pre_hook.is_error && (name == "bash" || name == "powershell") {
                    if normalized_output.contains("FAILED")
                        || normalized_output.contains("assertion failed")
                        || normalized_output.contains("error:")
                    {
                        Some(normalized_output.chars().take(200).collect())
                    } else {
                        None
                    }
                } else {
                    None
                };

            let is_pruning_recovery = pre_hook
                .details
                .as_ref()
                .and_then(|d| d.get("is_pruning_recovery"))
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let is_job_poll = name == "job_output";

            let observation = crate::runtime::progress_watchdog::ProgressObservation {
                tool_name: name.to_string(),
                input_fingerprint: input_fp,
                output_digest,
                hypothesis_id: None,
                hypothesis_evidence_refs: Vec::new(),
                target_path,
                edit_content_hash,
                test_failure_signature,
                is_pruning_recovery,
                is_job_poll,
                has_new_evidence: false,
            };

            if let Ok(mut wd) = runtime.progress_watchdog.lock() {
                let _ = wd.observe(observation);
            }
        }
    }

    /// Emit a final result after hooks and any transactional persistence.
    fn emit_tool_result(
        &self,
        id: &str,
        name: &str,
        args: &Value,
        result: crate::ToolResult,
    ) -> (ChatMessage, Vec<AgentEvent>) {
        let mut events = Vec::new();
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
        if let Some(runtime) = &self.runtime {
            runtime.emit_observe(crate::runtime::RuntimeEvent::PostToolUse {
                call_id: id.to_string(),
                tool: name.to_string(),
                is_error: result.is_error,
            });
        }
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
        // Asking for user intent is a host interaction, not a repository or
        // global-permission grant. The operations selected later still pass
        // the normal permission and task-contract gates.
        if name == "ask_user_question" {
            return None;
        }
        use crate::PermissionVerdict;
        let (issued_policy, issued_revision) = {
            let state = self
                .permissions
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            (state.clone(), state.revision())
        };
        let verdict = issued_policy.decide(id, name, args, cwd);
        let request = match verdict {
            PermissionVerdict::Allow => return None,
            PermissionVerdict::Deny { reason } => {
                crate::stats::SharedCounters::add(&self.counters.permission_denials, 1);
                if let Some(runtime) = &self.runtime {
                    runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                        call_id: id.to_string(),
                        reason: reason.clone(),
                    });
                }
                return Some(reason);
            }
            PermissionVerdict::Ask(request) => {
                crate::stats::SharedCounters::add(&self.counters.permission_prompts, 1);
                if let Some(runtime) = &self.runtime {
                    if let Err(reason) =
                        runtime.emit_decision(crate::runtime::RuntimeEvent::PermissionRequested {
                            call_id: id.to_string(),
                            tool: name.to_string(),
                        })
                    {
                        runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                            call_id: id.to_string(),
                            reason: reason.clone(),
                        });
                        return Some(reason);
                    }
                }
                request
            }
        };
        if self.approver.is_none() && self.approval_responder.is_none() {
            crate::stats::SharedCounters::add(&self.counters.permission_denials, 1);
            let guidance = if request.session_rule.is_empty() {
                "This action requires one-shot approval from an interactive host.".to_string()
            } else {
                format!("A supported interactive host can ask to allow this call or the scoped rule `{}`.", request.session_rule)
            };
            let reason = format!(
                "Permission denied: `{}` needs approval in permission mode `{}`, and this run cannot ask. \
                 {}",
                request.summary,
                request.mode.as_str(),
                guidance
            );
            if let Some(runtime) = &self.runtime {
                runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                    call_id: id.to_string(),
                    reason: reason.clone(),
                });
            }
            return Some(reason);
        }
        let denied = |reason: String| {
            crate::stats::SharedCounters::add(&self.counters.permission_denials, 1);
            if let Some(runtime) = &self.runtime {
                runtime.emit_observe(crate::runtime::RuntimeEvent::PermissionDenied {
                    call_id: id.to_string(),
                    reason: reason.clone(),
                });
            }
            Some(reason)
        };
        let Some(revision) = issued_revision else {
            return denied("Permission denied: policy revision is exhausted.".into());
        };
        let digest = self
            .approval_registry
            .digest(&request, cwd, self.runtime.as_ref());
        let pending = match self.approval_registry.issue(
            &request,
            digest.clone(),
            revision,
            davinci_session::now_ms(),
        ) {
            Ok(pending) => pending,
            Err(reason) => return denied(format!("Permission denied: {reason}.")),
        };
        // Host callbacks run without the policy lock and may fail or change
        // policy while the question is open. Neither can authorize this call.
        let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(responder) = &self.approval_responder {
                (responder.0)(&request, &pending.challenge)
            } else {
                let decision = (self.approver.as_ref().expect("host checked above").0)(&request);
                crate::approval::ApprovalReply::from_legacy(&pending.challenge, decision)
            }
        })) {
            Ok(reply) => reply,
            Err(_) => return denied("Permission denied: approval callback failed.".into()),
        };
        if self
            .abort_signal
            .as_ref()
            .is_some_and(|signal| signal.load(std::sync::atomic::Ordering::SeqCst))
            || self
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.cancellation_token.is_cancelled())
        {
            return denied("Permission denied: approval was cancelled.".into());
        }
        let stale_reason = {
            let mut policy = self
                .permissions
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if issued_revision.is_none()
                || policy.revision() != issued_revision
                || *policy != issued_policy
            {
                Some("Permission denied: policy changed while approval was pending; request approval again.".into())
            } else {
                match policy.decide(id, name, args, cwd) {
                    PermissionVerdict::Ask(current) if current == request => {
                        let current_digest = self.approval_registry.digest(&current, cwd, self.runtime.as_ref());
                        match pending.resolve(&reply, &current_digest, policy.revision(), davinci_session::now_ms()) {
                            Ok(crate::approval::GrantScope::Once) => None,
                            Ok(crate::approval::GrantScope::Session | crate::approval::GrantScope::Project) => {
                                policy.remember(&request.session_rule);
                                None
                            }
                            Ok(crate::approval::GrantScope::Deny | crate::approval::GrantScope::DenyWithInstructions) => {
                                let guidance = reply.instructions.as_deref().map(crate::approval::instruction_text).unwrap_or_default();
                                let mut reason = format!("Permission denied: the user declined `{}`.", request.summary);
                                if !guidance.is_empty() {
                                    reason.push(' ');
                                    reason.push_str(&guidance);
                                }
                                Some(reason)
                            }
                            Err(reason) => Some(format!("Permission denied: {reason}.")),
                        }
                    }
                    PermissionVerdict::Deny { reason } => Some(reason),
                    _ => Some("Permission denied: action changed while approval was pending; request approval again.".into()),
                }
            }
        };
        // Observers may read policy themselves, so emit after releasing it.
        match stale_reason {
            Some(reason) => denied(reason),
            None => None,
        }
    }

    fn capability_effect_denial(&self, cwd: &Path, name: &str, args: &Value) -> Option<String> {
        let state = self.capability_run_state();
        let decision = crate::prompt::capabilities::CapabilityDecision {
            capabilities: state.active,
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let request = self.last_real_user_request.as_deref().unwrap_or_default();
        let ceiling = crate::prompt::capabilities::capability_effect_ceiling(&decision, request);
        if ceiling != crate::prompt::capabilities::CapabilityEffectCeiling::ReadOnly {
            return None;
        }

        let command = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if name == "ask_user_question"
            || crate::read_only_capability_allows(name, args, command, cwd)
        {
            return None;
        }

        Some(format!(
            "review-only capability effect ceiling denied `{name}`; only reads and recognized local checks are allowed."
        ))
    }

    /// Checks whether an action conforms to the active task contract and execution gate invariants.
    pub fn check_contract_gate(
        &self,
        cwd: &Path,
        _id: &str,
        name: &str,
        args: &Value,
    ) -> Result<(), crate::runtime::ScopeViolation> {
        let contract = match self.active_contract() {
            Some(c) => c,
            None => return Ok(()),
        };

        let mut owner_valid = true;
        if let Some(runtime) = &self.runtime {
            if let Some(task) = runtime.task_registry.get_task(&contract.task_id) {
                if let Some(assigned) = task.assigned_to {
                    if assigned != runtime.agent_id {
                        owner_valid = false;
                    }
                }
                if let Some(digest) = &task.contract_digest {
                    if digest != &contract.digest {
                        owner_valid = false;
                    }
                }
                if task.state.is_terminal() {
                    owner_valid = false;
                }
            }
        }

        let hard_limits_allow = !self.abort_requested();

        // A task contract is a hard execution boundary, not merely a path allowlist. Native
        // file tools are constrained by `check_call`, but process/network-capable tools must
        // also have a backend that can actually enforce their declared effects. The ordinary
        // host process has no such sandbox today, so fail closed before permission approval or
        // dispatch rather than pretending a command allowlist contains project-controlled code.
        let class = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .class_of(name);
        if name == "mcp_read" || name.starts_with("mcp__") {
            return Err(crate::runtime::ScopeViolation {
                requested_target: name.to_string(),
                tool: name.to_string(),
                writable_scope: contract.writable_paths.clone(),
                protected_scope: contract.protected_paths.clone(),
                reason: "execution_contract_unenforceable: advisory MCP metadata cannot prove or contain remote effects under a hard contract".into(),
            });
        }
        if matches!(name, "bash" | "powershell" | "exec_command") {
            let Some(command) = args.get("command").and_then(Value::as_str) else {
                return Err(crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: "execution_contract_unenforceable: contracted shell command is missing a canonical command string".into(),
                });
            };
            let executor = crate::runtime::ContractExecutor::new(
                "unconfined-host",
                crate::runtime::ExecutorCapabilities::default(),
                cwd,
            )
            .with_contract(contract.clone());
            if let Err(error) = executor.execute_shell(command, false) {
                return Err(crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: error.to_string(),
                });
            }
        } else {
            let effects = crate::runtime::default_declared_effects(name, class);
            if !crate::tools::BUILTIN_TOOLS.contains(&name)
                && effects
                    .iter()
                    .any(|effect| matches!(effect, crate::runtime::DeclaredEffect::Other(_)))
            {
                return Err(crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: "execution_contract_unenforceable: custom/native tool has no host-verified effect profile under a hard contract".into(),
                });
            }
            if !effects.iter().any(|effect| {
                matches!(
                    effect,
                    crate::runtime::DeclaredEffect::ProcessExecution
                        | crate::runtime::DeclaredEffect::NetworkAccess
                )
            }) {
                // Native file/read operations are enforced by `check_call` below.
            } else {
                let action = crate::runtime::PreparedAction::new(name, Vec::new(), effects.clone())
                    .with_contract(contract.digest.clone(), None);
                let executor = crate::runtime::ContractExecutor::new(
                    "unconfined-host",
                    crate::runtime::ExecutorCapabilities::default(),
                    cwd,
                )
                .with_contract(contract.clone());
                if let Err(error) = executor.execute(&action) {
                    return Err(crate::runtime::ScopeViolation {
                        requested_target: name.to_string(),
                        tool: name.to_string(),
                        writable_scope: contract.writable_paths.clone(),
                        protected_scope: contract.protected_paths.clone(),
                        reason: error.to_string(),
                    });
                }
            }
        }

        let call_check = contract.check_call(cwd, name, args);
        let in_scope = call_check.is_ok();

        let mode = self.permission_mode();
        let allowed =
            crate::runtime::contract_gate(mode.as_str(), in_scope, owner_valid, hard_limits_allow);

        if !allowed {
            if !owner_valid {
                return Err(crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: "Task ownership is invalid, handed off, or contract digest mismatch"
                        .into(),
                });
            }
            if !hard_limits_allow {
                return Err(crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: "Execution aborted or hard security boundary disallows execution"
                        .into(),
                });
            }
            return Err(call_check
                .err()
                .unwrap_or_else(|| crate::runtime::ScopeViolation {
                    requested_target: name.to_string(),
                    tool: name.to_string(),
                    writable_scope: contract.writable_paths.clone(),
                    protected_scope: contract.protected_paths.clone(),
                    reason: "Contract gate disallowed execution".into(),
                }));
        }

        Ok(())
    }

    fn check_contract_dispatch_boundary(
        &self,
        cwd: &Path,
        name: &str,
    ) -> Result<(), crate::runtime::ScopeViolation> {
        let Some(contract) = self.active_contract() else {
            return Ok(());
        };
        let class = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .class_of(name);
        let effects = crate::runtime::default_declared_effects(name, class);
        if !effects
            .iter()
            .any(|effect| matches!(effect, crate::runtime::DeclaredEffect::FileSystemWrite))
        {
            return Ok(());
        }

        let action = crate::runtime::PreparedAction::new(name, Vec::new(), effects)
            .with_contract(contract.digest.clone(), None);
        let executor = crate::runtime::ContractExecutor::new(
            "unconfined-host",
            crate::runtime::ExecutorCapabilities::default(),
            cwd,
        )
        .with_contract(contract.clone());
        executor
            .execute(&action)
            .map_err(|error| crate::runtime::ScopeViolation {
                requested_target: name.to_string(),
                tool: name.to_string(),
                writable_scope: contract.writable_paths.clone(),
                protected_scope: contract.protected_paths.clone(),
                reason: error.to_string(),
            })
    }

    fn persist_chat(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let content = serde_json::to_value(&message.content).unwrap_or(Value::Null);
            let _ = session.append_entry(crate::chat_entry(&message.role, content, &message.extra));
        }
    }

    fn persist_assistant(&mut self, assistant: &AssistantMessage, chat: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let timestamp = davinci_session::now_ms();
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
            let _ = session.append_entry(davinci_session::SessionEntry {
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

    fn emit_behavior_telemetry(&self) {
        let final_stats = self.run_stats();
        let profile = self
            .prompt_manifest
            .as_ref()
            .map(|m| m.profile.clone())
            .unwrap_or_else(|| "stable".into());
        let version = self
            .prompt_manifest
            .as_ref()
            .map(|m| m.profile_version)
            .unwrap_or(2);
        let hash_prefix = self
            .prompt_manifest
            .as_ref()
            .map(|m| {
                if m.stable_sha256.len() >= 8 {
                    m.stable_sha256[..8].to_string()
                } else {
                    m.stable_sha256.clone()
                }
            })
            .unwrap_or_default();
        let family = crate::prompt::provider::prompt_model_family(&self.provider, &self.model_id)
            .name()
            .to_string();
        let (model_policy, model_policy_version) = self
            .prompt_manifest
            .as_ref()
            .map(|manifest| (manifest.model_policy.clone(), manifest.model_policy_version))
            .unwrap_or_else(|| ("default".to_string(), 0));

        davinci_telemetry::record_behavior_telemetry(davinci_telemetry::BehaviorTelemetry {
            prompt_profile: profile,
            prompt_version: version,
            prompt_stable_hash_prefix: hash_prefix,
            model_family: family,
            model_policy,
            model_policy_version,
            model_turns: final_stats.model_turns,
            tool_calls: final_stats.tool_calls,
            permission_prompts: final_stats.permission_prompts,
            permission_denials: final_stats.permission_denials,
            files_changed_count: final_stats.files_changed_count,
            verification_commands_run: final_stats.verification_commands_run,
            verification_failures: final_stats.verification_failures,
            capability_incomplete_evidence: final_stats.capability_incomplete_evidence,
            aborted: self.abort_requested(),
            user_steers: final_stats.user_steers,
        });
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

fn sleep_retry_delay(delay_ms: u64, cancelled: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    let delay = std::time::Duration::from_millis(delay_ms);
    while !cancelled() {
        let remaining = delay.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(25)));
    }
}

pub(crate) fn is_verification_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("cargo test")
        || lower.contains("cargo check")
        || lower.contains("cargo clippy")
        || lower.contains("pytest")
        || lower.contains("npm test")
        || lower.contains("pnpm test")
        || lower.contains("yarn test")
        || lower.contains("go test")
        || lower.contains("make test")
        || lower.contains("make check")
        || lower.contains("ctest")
        || lower.contains("mvn test")
        || lower.contains("gradle test")
}

#[cfg(test)]
mod tests {
    #[test]
    fn f01_cancelled_approval_finishes_every_tool_without_dispatch() {
        for with_runtime in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("input.txt"), "must not be read").unwrap();
            let mut agent = Agent::new("offline cancellation");
            agent.cwd = dir.path().to_path_buf();
            agent.tools = vec!["read".into(), "write".into()];
            agent.permissions = std::sync::Arc::new(crate::PermissionState::new(
                crate::PermissionPolicy::new(crate::PermissionMode::Ask),
            ));
            if with_runtime {
                agent.set_runtime(crate::RuntimeHandle::new(
                    crate::RunId::new(),
                    crate::AgentId::new(),
                    crate::RuntimeBus::new(),
                ));
            }
            let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            agent.abort_signal = Some(abort.clone());
            agent.approver = Some(crate::ToolApprover(std::sync::Arc::new(move |_| {
                abort.store(true, std::sync::atomic::Ordering::SeqCst);
                crate::ToolApprovalDecision::Deny
            })));
            let calls = vec![
                (
                    "before".into(),
                    "read".into(),
                    serde_json::json!({"path": "input.txt"}),
                ),
                (
                    "denied".into(),
                    "write".into(),
                    serde_json::json!({"path": "blocked.txt", "content": "blocked"}),
                ),
                (
                    "after".into(),
                    "write".into(),
                    serde_json::json!({"path": "after.txt", "content": "blocked"}),
                ),
            ];
            let mut events = Vec::new();
            let messages = agent.execute_tool_batch(dir.path(), calls, &mut events);
            assert_eq!(
                messages
                    .iter()
                    .map(|m| m.tool_call_id.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                ["before", "denied", "after"]
            );
            assert!(messages.iter().all(|m| m.is_error == Some(true)));
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
                    .count(),
                3
            );
            assert!(!dir.path().join("blocked.txt").exists());
            assert!(!dir.path().join("after.txt").exists());
            let ledger = serde_json::to_value(&*agent.tool_ledger.lock().unwrap()).unwrap();
            assert!(ledger["records"]
                .as_object()
                .unwrap()
                .values()
                .all(|record| record["status"] != "pending" && record["status"] != "executing"));
        }
    }

    #[test]
    fn f01_post_hook_cannot_clear_denial_or_cancellation() {
        let mut agent = Agent::new("offline post hook protection");
        agent.post_tool = Some(crate::PostToolHook(Arc::new(|_, _, _, _, _| {
            crate::ToolResult {
                content: "hook decoration".into(),
                is_error: false,
                details: None,
            }
        })));
        for marker in ["denied", "cancelled"] {
            let result = crate::ToolResult {
                content: "not executed".into(),
                is_error: true,
                details: Some(json!({marker: true})),
            };
            let (message, events) =
                agent.finalize_tool_call(Path::new("."), "skipped", "write", &json!({}), result);
            assert_eq!(message.is_error, Some(true));
            assert!(events.iter().any(|event| matches!(event,
                AgentEvent::ToolExecutionEnd { is_error: true, details: Some(details), .. } if details[marker] == true
            )));
        }
    }

    #[test]
    fn f01_cancelled_mixed_plan_batch_finishes_every_call() {
        for pre_cancelled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut agent = Agent::new("offline mixed plan cancellation");
            agent.tools = vec!["write".into(), "propose_plan".into()];
            agent.permissions = Arc::new(crate::PermissionState::new(
                crate::PermissionPolicy::new(crate::PermissionMode::Ask),
            ));
            let abort = Arc::new(std::sync::atomic::AtomicBool::new(pre_cancelled));
            agent.abort_signal = Some(abort.clone());
            agent.approver = Some(crate::ToolApprover(Arc::new(move |_| {
                abort.store(true, std::sync::atomic::Ordering::SeqCst);
                crate::ToolApprovalDecision::Deny
            })));
            let before = agent.tool_context.living_plan.lock().unwrap().clone();
            agent.post_tool = Some(crate::PostToolHook(Arc::new(|_, _, _, _, mut result| {
                if result.details.as_ref().and_then(|d| d.get("cancelled"))
                    == Some(&Value::Bool(true))
                {
                    result.is_error = false;
                    result.details = None;
                }
                result
            })));
            let calls = vec![
                (
                    "denied".into(),
                    "write".into(),
                    json!({"path":"blocked.txt","content":"blocked"}),
                ),
                (
                    "plan".into(),
                    "propose_plan".into(),
                    json!({"title":"must not run","steps":[]}),
                ),
                (
                    "after".into(),
                    "write".into(),
                    json!({"path":"after.txt","content":"blocked"}),
                ),
            ];
            let mut events = Vec::new();
            let messages = agent.execute_tool_batch(dir.path(), calls, &mut events);
            assert_eq!(
                messages
                    .iter()
                    .map(|m| m.tool_call_id.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                ["denied", "plan", "after"]
            );
            assert!(messages.iter().all(|m| m.is_error == Some(true)));
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
                    .count(),
                3
            );
            assert_eq!(
                serde_json::to_value(&*agent.tool_context.living_plan.lock().unwrap()).unwrap(),
                serde_json::to_value(before).unwrap()
            );
            let starts = events
                .iter()
                .filter_map(|event| match event {
                    AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                        Some(tool_call_id.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                starts,
                if pre_cancelled {
                    vec![]
                } else {
                    vec!["denied"]
                }
            );
            assert!(!dir.path().join("blocked.txt").exists());
            assert!(!dir.path().join("after.txt").exists());
        }
    }

    #[test]
    fn f01_denial_instructions_reach_tool_result_without_execution() {
        let dir = tempdir().unwrap();
        let mut agent = Agent::new("offline denial instructions fixture");
        agent.tools = vec!["write".into()];
        agent.permissions = Arc::new(crate::PermissionState::new(crate::PermissionPolicy::new(
            crate::PermissionMode::Ask,
        )));
        let instructions = format!("{}read the docs first", "x".repeat(2048));
        let reply_instructions = instructions.clone();
        agent.approval_responder = Some(crate::approval::ApprovalResponder(Arc::new(
            move |_, challenge| crate::approval::ApprovalReply {
                challenge_id: challenge.id,
                choice_id: "deny_with_instructions".into(),
                instructions: Some(reply_instructions.clone()),
            },
        )));
        let args = json!({"path":"denied.txt", "content":"must not be written"});
        let Preparation::Immediate(result) =
            agent.prepare_tool_call(dir.path(), "denied", "write", &args, 0)
        else {
            panic!("denial must not dispatch the executor");
        };
        assert!(result.is_error);
        assert!(serde_json::to_string(&result.content)
            .unwrap()
            .contains(&instructions));
        assert!(!dir.path().join("denied.txt").exists());
        assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
    }

    #[test]
    fn f01_typed_reply_must_match_the_issued_challenge() {
        for forged in [false, true] {
            let dir = tempdir().unwrap();
            let mut agent = Agent::new("offline typed approval fixture");
            agent.tools = vec!["write".into()];
            agent.permissions = Arc::new(crate::PermissionState::new(
                crate::PermissionPolicy::new(crate::PermissionMode::Ask),
            ));
            agent.approval_responder = Some(crate::approval::ApprovalResponder(Arc::new(
                move |_, challenge| crate::approval::ApprovalReply {
                    challenge_id: if forged {
                        uuid::Uuid::new_v4()
                    } else {
                        challenge.id
                    },
                    choice_id: "once".into(),
                    instructions: None,
                },
            )));
            let args = json!({"path":"typed.txt", "content":"fixture"});
            let result = agent.prepare_tool_call(dir.path(), "typed", "write", &args, 0);
            assert_eq!(matches!(result, Preparation::Ready { .. }), !forged);
            assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
            assert!(!dir.path().join("typed.txt").exists());
            if !forged {
                assert!(
                    !agent
                        .run_prepared_call(dir.path(), "typed", "write", &args, 0)
                        .is_error
                );
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("typed.txt")).unwrap(),
                    "fixture"
                );
            }
        }
    }

    #[test]
    fn f01_unoffered_persistent_scope_blocks_actual_preparation() {
        let dir = tempdir().unwrap();
        let mut agent = Agent::new("offline legal scope fixture");
        agent.tools = vec!["write".into()];
        agent.permissions = Arc::new(crate::PermissionState::new(crate::PermissionPolicy::new(
            crate::PermissionMode::Ask,
        )));
        agent.approver = Some(crate::ToolApprover(Arc::new(|request| {
            assert!(!request.allows(crate::ToolApprovalDecision::AllowForSession));
            crate::ToolApprovalDecision::AllowForSession
        })));
        let args = json!({"path":".env", "content":"fixture"});
        let Preparation::Immediate(result) =
            agent.prepare_tool_call(dir.path(), "risky", "write", &args, 0)
        else {
            panic!("forged scope must deny")
        };
        assert!(result.is_error);
        assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
        assert!(!dir.path().join(".env").exists());
    }

    #[test]
    fn f01_callback_changes_and_panics_cannot_grant_execution() {
        for case in [
            "deny",
            "mode",
            "mode_restored",
            "cancel",
            "panic",
            "unchanged_once",
            "unchanged_session",
        ] {
            let dir = tempdir().unwrap();
            let mut agent = Agent::new("offline approval fixture");
            agent.tools = vec!["write".into()];
            agent.permissions = Arc::new(crate::PermissionState::new(
                crate::PermissionPolicy::new(crate::PermissionMode::Ask),
            ));
            let policy = agent.permissions.clone();
            let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
            let cancellation = runtime.cancellation_token.clone();
            agent.set_runtime(runtime);
            agent.approver = Some(crate::ToolApprover(Arc::new(move |_| {
                match case {
                    "deny" => policy
                        .lock()
                        .unwrap()
                        .deny
                        .push(crate::PermissionRule::bare("write")),
                    "mode" => policy.lock().unwrap().mode = crate::PermissionMode::AlwaysApprove,
                    "mode_restored" => {
                        let mut state = policy.lock().unwrap();
                        state.mode = crate::PermissionMode::AlwaysApprove;
                        state.mode = crate::PermissionMode::Ask;
                    }
                    "cancel" => cancellation.cancel(),
                    "panic" => panic!("fixture approval callback panic"),
                    _ => {}
                }
                if case == "unchanged_once" {
                    crate::ToolApprovalDecision::AllowOnce
                } else {
                    crate::ToolApprovalDecision::AllowForSession
                }
            })));
            let args = json!({"path":"approval.txt", "content":"fixture"});
            let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                agent.prepare_tool_call(dir.path(), "approval-call", "write", &args, 0)
            }))
            .expect("callback panic must become a denied tool result");
            let expected_allow = case.starts_with("unchanged");
            match prepared {
                Preparation::Ready { .. } if expected_allow => {
                    assert!(
                        !agent
                            .run_prepared_call(dir.path(), "approval-call", "write", &args, 0)
                            .is_error
                    );
                }
                Preparation::Immediate(result) if !expected_allow => {
                    assert!(result.is_error);
                    assert_eq!(result.details.unwrap()["denied"], true);
                }
                _ => panic!("unexpected approval outcome for {case}"),
            }
            assert_eq!(dir.path().join("approval.txt").exists(), expected_allow);
            assert_eq!(
                !agent.permissions.lock().unwrap().session_allow.is_empty(),
                case == "unchanged_session"
            );
        }
    }

    #[test]
    fn f01_permission_subscriber_denial_prevents_callback_and_execution() {
        struct Refuse(Arc<AtomicUsize>);
        impl RuntimeSubscriber for Refuse {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                match &event.payload {
                    RuntimeEvent::PermissionRequested { .. } => RuntimeDecision::Deny {
                        reason: "fixture policy refusal".into(),
                    },
                    RuntimeEvent::PermissionDenied { .. } => {
                        self.0.fetch_add(1, Ordering::SeqCst);
                        RuntimeDecision::Continue
                    }
                    _ => RuntimeDecision::Continue,
                }
            }
        }
        let dir = tempdir().unwrap();
        let mut agent = Agent::new("offline approval fixture");
        agent.cwd = dir.path().to_path_buf();
        agent.tools = vec!["write".into()];
        agent.permissions = Arc::new(crate::PermissionState::new(crate::PermissionPolicy::new(
            crate::PermissionMode::Ask,
        )));
        let callbacks = Arc::new(AtomicUsize::new(0));
        let called = callbacks.clone();
        agent.approver = Some(crate::ToolApprover(Arc::new(move |_| {
            called.fetch_add(1, Ordering::SeqCst);
            crate::ToolApprovalDecision::AllowForSession
        })));
        let denials = Arc::new(AtomicUsize::new(0));
        let bus = RuntimeBus::new();
        bus.subscribe(Arc::new(Refuse(denials.clone())));
        agent.set_runtime(RuntimeHandle::new(RunId::new(), AgentId::new(), bus));
        let turns = AtomicUsize::new(0);
        let events = agent
            .run_loop(|_| {
                let first = turns.fetch_add(1, Ordering::SeqCst) == 0;
                Ok(AssistantMessage {
                    id: "fixture".into(),
                    role: "assistant".into(),
                    content: if first {
                        vec![ContentBlock::ToolCall {
                            id: "denied-write".into(),
                            name: "write".into(),
                            arguments: json!({"path":"must-not-exist.txt", "content":"blocked"}),
                        }]
                    } else {
                        vec![ContentBlock::Text {
                            text: "done".into(),
                        }]
                    },
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(if first {
                        StopReason::ToolUse
                    } else {
                        StopReason::Stop
                    }),
                    error_message: None,
                })
            })
            .unwrap();
        assert_eq!(callbacks.load(Ordering::SeqCst), 0);
        assert_eq!(denials.load(Ordering::SeqCst), 1);
        assert!(!dir.path().join("must-not-exist.txt").exists());
        assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. })));
    }

    use super::*;
    use crate::events::AgentEvent;
    use crate::runtime::{
        AgentId, RunId, RuntimeBus, RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope,
        RuntimeHandle, RuntimeSubscriber,
    };
    use davinci_ai::{content_text, AssistantMessage, ContentBlock, StopReason};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[test]
    fn existing_serialized_agent_event_json_is_unchanged() {
        let event = AgentEvent::ToolExecutionStart {
            tool_call_id: "call_1".into(),
            tool_name: "read".into(),
            args: json!({"path": "src/main.rs"}),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "tool_execution_start");
        assert_eq!(value["toolCallId"], "call_1");
        assert_eq!(value["toolName"], "read");
        assert_eq!(value["args"]["path"], "src/main.rs");
    }

    #[test]
    fn f03_task_reads_prepare_in_parallel_lane() {
        let dir = tempdir().unwrap();
        let mut agent = Agent::new("Test task reads");
        agent.tools = vec!["task_get".into(), "task_list".into()];
        agent.set_permission_mode(crate::PermissionMode::ReadOnly);
        for name in ["task_get", "task_list"] {
            assert!(matches!(
                agent.prepare_tool_call(dir.path(), name, name, &json!({}), 0),
                Preparation::Ready {
                    lane: crate::scheduler::ToolLane::Parallel
                }
            ));
        }
    }

    #[derive(Default)]
    struct SequenceRecorder {
        events: Mutex<Vec<String>>,
    }

    impl RuntimeSubscriber for SequenceRecorder {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            let desc = match &event.payload {
                RuntimeEvent::PreToolUse { tool, .. } => format!("runtime:PreToolUse:{tool}"),
                RuntimeEvent::PostToolUse { tool, .. } => format!("runtime:PostToolUse:{tool}"),
                RuntimeEvent::TurnStarted => "runtime:TurnStarted".into(),
                RuntimeEvent::TurnEnded { .. } => "runtime:TurnEnded".into(),
                _ => "runtime:other".into(),
            };
            self.events.lock().unwrap().push(desc);
            RuntimeDecision::Continue
        }
    }

    #[test]
    fn tool_call_emits_pre_tool_use_then_tool_start_then_post_tool_use() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.txt");
        std::fs::write(&test_file, "hello runtime").unwrap();

        let mut agent = Agent::new("You are a test agent");
        agent.cwd = dir.path().to_path_buf();
        agent.tools = vec!["read".into()];

        let recorder = Arc::new(SequenceRecorder::default());
        let bus = RuntimeBus::new();
        bus.subscribe(recorder.clone());

        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let handle = RuntimeHandle::new(run_id, agent_id, bus);
        agent.runtime = Some(handle);

        let live_events = Arc::new(Mutex::new(Vec::new()));
        let live_clone = live_events.clone();
        agent.event_sink = Some(crate::EventSink(Arc::new(move |ev| {
            if let AgentEvent::ToolExecutionStart { tool_name, .. } = ev {
                live_clone
                    .lock()
                    .unwrap()
                    .push(format!("agent:ToolExecutionStart:{tool_name}"));
            }
        })));

        let called = Arc::new(AtomicUsize::new(0));
        let called_clone = called.clone();

        let events = agent
            .run_loop(|_ag| {
                let count = called_clone.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    Ok(AssistantMessage {
                        id: "msg_tool".into(),
                        role: "assistant".into(),
                        content: vec![ContentBlock::ToolCall {
                            id: "call_read_1".into(),
                            name: "read".into(),
                            arguments: json!({"path": "test.txt"}),
                        }],
                        model: "test-model".into(),
                        usage: None,
                        stop_reason: Some(StopReason::ToolUse),
                        error_message: None,
                    })
                } else {
                    Ok(AssistantMessage {
                        id: "msg_end".into(),
                        role: "assistant".into(),
                        content: vec![ContentBlock::Text {
                            text: "Done reading".into(),
                        }],
                        model: "test-model".into(),
                        usage: None,
                        stop_reason: Some(StopReason::Stop),
                        error_message: None,
                    })
                }
            })
            .unwrap();

        assert!(!events.is_empty());

        let recorded_runtime = recorder.events.lock().unwrap().clone();
        assert!(recorded_runtime.contains(&"runtime:TurnStarted".to_string()));
        assert!(recorded_runtime.contains(&"runtime:PreToolUse:read".to_string()));
        assert!(recorded_runtime.contains(&"runtime:PostToolUse:read".to_string()));
        assert!(recorded_runtime.contains(&"runtime:TurnEnded".to_string()));

        let pre_idx = recorded_runtime
            .iter()
            .position(|e| e == "runtime:PreToolUse:read")
            .unwrap();
        let post_idx = recorded_runtime
            .iter()
            .position(|e| e == "runtime:PostToolUse:read")
            .unwrap();
        assert!(pre_idx < post_idx);

        let live = live_events.lock().unwrap().clone();
        assert!(live.contains(&"agent:ToolExecutionStart:read".to_string()));
    }

    #[test]
    fn agent_run_loop_without_runtime_has_zero_behavioral_difference() {
        let mut agent = Agent::new("Test prompt");
        assert!(agent.runtime.is_none());

        let events = agent
            .run_loop(|_ag| {
                Ok(AssistantMessage {
                    id: "msg_plain".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "Direct text response".into(),
                    }],
                    model: "test-model".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();

        assert_eq!(events[0], AgentEvent::AgentStart);
        assert_eq!(events[1], AgentEvent::TurnStart);
        assert!(matches!(events.last(), Some(AgentEvent::AgentEnd { .. })));
    }

    #[test]
    fn run_loop_records_behavioral_telemetry() {
        davinci_telemetry::clear_behavior_telemetry();
        let mut agent = Agent::new("Test prompt");
        let _ = agent.run_loop(|_ag| {
            Ok(AssistantMessage {
                id: "msg_1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "Hello".into(),
                }],
                model: "claude-3-5-sonnet".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        });

        let telemetry = davinci_telemetry::get_behavior_telemetry();
        assert!(!telemetry.is_empty());
        let entry = telemetry.last().unwrap();
        assert_eq!(entry.model_turns, 1);
        assert!(!entry.aborted);
        assert_eq!(entry.prompt_version, 2);
    }

    #[test]
    fn capability_gate_continues_before_turn_finalization() {
        davinci_telemetry::clear_behavior_telemetry();
        let mut agent = Agent::new_builtin(crate::PromptProfile::Stable);
        agent.prompt_user_with("Diagnose the failing command and find the root cause", &[]);
        {
            let mut state = agent.capability_run_state.lock().unwrap();
            state.debugging = Some(crate::DebuggingState {
                reproducer: Some(crate::ReproducerEvidence {
                    failed_before_edit: true,
                    ..crate::ReproducerEvidence::default()
                }),
                causal_edit_seen: true,
                ..crate::DebuggingState::default()
            });
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let provider_views = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let calls_for_provider = Arc::clone(&calls);
        let views_for_provider = Arc::clone(&provider_views);
        let events = agent
            .run_loop(move |current| {
                views_for_provider.lock().unwrap().push(
                    current
                        .messages_for_provider()
                        .iter()
                        .map(|message| content_text(&message.content))
                        .collect(),
                );
                let call = calls_for_provider.fetch_add(1, Ordering::SeqCst);
                Ok(AssistantMessage {
                    id: format!("msg_{call}"),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "I am ready to finish".into(),
                    }],
                    model: "test-model".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let views = provider_views.lock().unwrap();
        assert!(views[1]
            .iter()
            .any(|text| text
                .contains("Before completing, rerun the original failing command unchanged")));
        assert!(views[2]
            .iter()
            .any(|text| text
                .contains("Before completing, rerun the original failing command unchanged")));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::TurnEnd { .. }))
                .count(),
            1
        );
        assert_eq!(agent.run_stats().capability_incomplete_evidence, 1);
        assert_eq!(
            davinci_telemetry::get_behavior_telemetry()
                .last()
                .unwrap()
                .capability_incomplete_evidence,
            1
        );
    }

    #[test]
    fn f05_hard_contract_refuses_unconfined_process_before_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("turn contract process boundary test");
        agent.tools = vec!["bash".into()];
        agent.permissions = std::sync::Arc::new(crate::PermissionState::new(
            crate::PermissionPolicy::new(crate::PermissionMode::AlwaysApprove),
        ));
        let contract = crate::runtime::TaskContract::new(
            "contract-process-boundary",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec!["target/".into()],
        )
        .unwrap();
        agent.set_active_contract(contract);

        let prep = agent.prepare_tool_call(
            temp.path(),
            "call_process",
            "bash",
            &serde_json::json!({"command": "cargo test"}),
            0,
        );
        match prep {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("execution_contract_unenforceable"));
            }
            _ => panic!("hard contract must refuse an unconfined process before dispatch"),
        }
    }

    #[test]
    fn f05_turn_prepare_and_run_contract_gate_enforcement() {
        let temp = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("turn contract gate test");
        agent.tools = vec!["write".into(), "edit".into()];
        agent.permissions = std::sync::Arc::new(crate::PermissionState::new(
            crate::PermissionPolicy::new(crate::PermissionMode::AlwaysApprove),
        ));

        let contract = crate::runtime::TaskContract::new(
            "contract-turn-gate",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["secret.env".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        agent.set_active_contract(contract);

        // 1. Prepare in-scope call -> Ready
        let prep_ok = agent.prepare_tool_call(
            temp.path(),
            "call_1",
            "write",
            &serde_json::json!({"path": "src/ok.rs", "content": "pub fn ok() {}"}),
            0,
        );
        assert!(matches!(prep_ok, Preparation::Ready { .. }));

        // 2. Prepare out-of-scope call -> Immediate denial with structured violation
        let prep_bad = agent.prepare_tool_call(
            temp.path(),
            "call_2",
            "write",
            &serde_json::json!({"path": "secret.env", "content": "SECRET=leak"}),
            0,
        );
        match prep_bad {
            Preparation::Immediate(res) => {
                assert!(res.is_error);
                assert!(res.content.contains("Scope violation"));
                assert!(res.details.unwrap()["scope_violation"].is_object());
            }
            _ => panic!("Expected Immediate rejection"),
        }

        // 3. Transformed args at dispatch time: call_1 was prepared for src/ok.rs,
        // but args mutate to secret.env before run_prepared_call:
        let dispatch_bad = agent.run_prepared_call(
            temp.path(),
            "call_1",
            "write",
            &serde_json::json!({"path": "secret.env", "content": "SECRET=leak"}),
            0,
        );
        assert!(dispatch_bad.is_error);
        assert!(dispatch_bad.content.contains("Scope violation"));
        assert!(dispatch_bad.details.unwrap()["scope_violation"].is_object());
    }

    #[test]
    fn f05_hard_contract_refuses_native_write_without_race_safe_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("native write containment fixture");
        agent.tools = vec!["write".into()];
        agent.permissions = std::sync::Arc::new(crate::PermissionState::new(
            crate::PermissionPolicy::new(crate::PermissionMode::AlwaysApprove),
        ));
        let contract = crate::runtime::TaskContract::new(
            "contract-native-write-boundary",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        agent.set_active_contract(contract);
        let args = serde_json::json!({"path": "src/ok.rs", "content": "must not land"});

        let result = agent.run_prepared_call(temp.path(), "native_write", "write", &args, 0);
        assert!(result.is_error);
        assert!(result.content.contains("execution_contract_unenforceable"));
        assert!(!temp.path().join("src/ok.rs").exists());
    }

    #[test]
    fn f05_trusted_mcp_read_only_hint_cannot_grant_contract_authority() {
        let temp = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("lying MCP hint fixture");
        let tool = "mcp__fixture__mutate";
        agent.tools = vec![tool.into()];
        let mut policy = crate::PermissionPolicy::new(crate::PermissionMode::AlwaysApprove);
        policy.mcp_read_only.insert(tool.into());
        agent.permissions = std::sync::Arc::new(crate::PermissionState::new(policy));
        let contract = crate::runtime::TaskContract::new(
            "contract-mcp-hint-boundary",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        agent.set_active_contract(contract);

        let preparation =
            agent.prepare_tool_call(temp.path(), "mcp_lie", tool, &serde_json::json!({}), 0);
        match preparation {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("execution_contract_unenforceable"));
            }
            _ => panic!("hard contract must reject advisory MCP read-only hints before dispatch"),
        }
    }

    #[test]
    fn f05_unknown_custom_tool_cannot_bypass_hard_contract() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = tempfile::tempdir().unwrap();
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let mut agent = Agent::new("unknown custom mutation fixture");
        agent.tools = vec!["custom_mutating".into()];
        agent.custom_tool_executor =
            Some(crate::CustomToolExecutor::new(move |_cwd, _name, _args| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(crate::ToolResult {
                    content: "mutated".into(),
                    is_error: false,
                    details: None,
                })
            }));
        agent.permissions = std::sync::Arc::new(crate::PermissionState::new(
            crate::PermissionPolicy::new(crate::PermissionMode::AlwaysApprove),
        ));
        let contract = crate::runtime::TaskContract::new(
            "contract-custom-tool-boundary",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        agent.set_active_contract(contract);

        let preparation = agent.prepare_tool_call(
            temp.path(),
            "custom_mutation",
            "custom_mutating",
            &serde_json::json!({}),
            0,
        );
        match preparation {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("execution_contract_unenforceable"));
            }
            _ => panic!("unknown custom effects must fail closed under a hard contract"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    fn f02_turn_question_fixture() -> (tempfile::TempDir, Agent, serde_json::Value) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
        let mut agent = Agent::new("structured clarification fixture");
        agent.cwd = dir.path().to_path_buf();
        agent.tools = vec!["ask_user_question".into()];
        agent.set_permission_mode(crate::PermissionMode::ReadOnly);
        agent
            .tool_context
            .living_plan
            .lock()
            .unwrap()
            .update(
                &serde_json::json!({
                    "expected_revision":0,
                    "goal":"Choose persistence",
                    "evidence":[{"path":"src.rs","finding":"Current entry point"}]
                }),
                dir.path(),
            )
            .unwrap();
        let question = serde_json::json!({
            "id":"cache-scope",
            "kind":"persistence",
            "title":"Cache scope",
            "question":"Which cache should be used?",
            "materiality":"Changes persistence semantics",
            "evidence_refs":["src.rs"],
            "options":[
                {"id":"memory","label":"Memory","explanation":"Process local","recommended":true},
                {"id":"sqlite","label":"SQLite","explanation":"Persistent"}
            ],
            "allow_custom":true,
            "custom_only":false
        });
        (dir, agent, question)
    }

    #[test]
    fn f02_plan_mode_question_is_serial_and_needs_no_permission_grant() {
        let (dir, agent, question) = f02_turn_question_fixture();
        match agent.prepare_tool_call(dir.path(), "ask-1", "ask_user_question", &question, 0) {
            Preparation::Ready { lane } => {
                assert_eq!(lane, crate::scheduler::ToolLane::Serial);
            }
            _ => panic!("host question should be legal in read-only plan mode"),
        }
        assert_eq!(agent.permission_mode(), crate::PermissionMode::ReadOnly);
    }

    #[test]
    fn f02_question_answer_rolls_back_when_session_persistence_fails() {
        let (dir, mut agent, question) = f02_turn_question_fixture();
        agent.tool_context.decision_responder = Some(crate::tools::DecisionResponder::new(|_| {
            crate::tools::DecisionHostResponse::Reply(crate::tools::DecisionHostReply {
                action: crate::decisions::HostDecisionAction::AnswerChoice("sqlite".into()),
                host_event_id: "ui-event-1".into(),
                answered_at_ms: 42,
            })
        }));
        let mut session =
            davinci_session::JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None)
                .unwrap();
        session.path = dir.path().join("missing-parent/session.jsonl");
        agent.session = Some(session);
        let before = agent.tool_context.living_plan.lock().unwrap().clone();
        let mut events = Vec::new();
        let messages = agent.execute_tool_batch(
            dir.path(),
            vec![("ask-1".into(), "ask_user_question".into(), question)],
            &mut events,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].is_error, Some(true));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolExecutionEnd {
                is_error: true,
                details: Some(details),
                ..
            } if details.get("plan_storage_error") == Some(&serde_json::Value::Bool(true))
        )));
        assert_eq!(*agent.tool_context.living_plan.lock().unwrap(), before);
        assert_eq!(agent.permission_mode(), crate::PermissionMode::ReadOnly);
    }

    #[test]
    fn f04_mutating_tool_records_owned_effect() {
        let dir = tempfile::tempdir().unwrap();
        let bus = crate::runtime::bus::RuntimeBus::new();
        let run_id = crate::runtime::ids::RunId::new();
        let agent_id = crate::runtime::ids::AgentId::new();
        let runtime = crate::runtime::RuntimeHandle::new(run_id, agent_id, bus);

        let mut agent = Agent::new("system");
        agent.cwd = dir.path().to_path_buf();
        agent.set_runtime(runtime.clone());
        agent.set_permission_mode(crate::PermissionMode::Auto);

        // 1. Initial write creates a file
        let args = serde_json::json!({
            "path": "test.txt",
            "content": "first line\n"
        });
        let res = agent.run_prepared_call(dir.path(), "call-1", "write", &args, 0);
        assert!(!res.is_error);

        let effects = runtime.effect_ledger.read().unwrap().clone();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].path, "test.txt");
        assert_eq!(
            effects[0].kind,
            crate::runtime::effects::FileEffectKind::Created
        );
        assert_eq!(effects[0].before_blob, None);
        assert!(effects[0].after_blob.is_some());
        let first_blob = effects[0].after_blob.clone();

        // 2. Second write modifies the file
        let args2 = serde_json::json!({
            "path": "test.txt",
            "content": "first line\nsecond line\n"
        });
        let res2 = agent.run_prepared_call(dir.path(), "call-2", "write", &args2, 0);
        assert!(!res2.is_error);

        let effects2 = runtime.effect_ledger.read().unwrap().clone();
        assert_eq!(effects2.len(), 2);
        assert_eq!(effects2[1].path, "test.txt");
        assert_eq!(
            effects2[1].kind,
            crate::runtime::effects::FileEffectKind::Modified
        );
        assert_eq!(effects2[1].before_blob, first_blob);
        assert_ne!(effects2[1].after_blob, first_blob);
    }

    #[test]
    fn review_only_ceiling_denies_mutations_and_nonlocal_shells() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new_builtin(crate::PromptProfile::Stable);
        agent.cwd = dir.path().to_path_buf();
        agent.tools = vec!["read".into(), "write".into(), "bash".into()];
        agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
        agent.prompt_user_with("Review this PR", &[]);

        let write = agent.prepare_tool_call(
            dir.path(),
            "review-write",
            "write",
            &serde_json::json!({"path":"reviewed.rs","content":"should not land"}),
            0,
        );
        match write {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("review-only"));
            }
            _ => panic!("review-only capability must deny writes before permission approval"),
        }

        let local_check = agent.prepare_tool_call(
            dir.path(),
            "review-test",
            "bash",
            &serde_json::json!({"command":"cargo test --offline"}),
            0,
        );
        assert!(matches!(local_check, Preparation::Ready { .. }));

        let network_shell = agent.prepare_tool_call(
            dir.path(),
            "review-network",
            "bash",
            &serde_json::json!({"command":"curl https://example.com"}),
            0,
        );
        match network_shell {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("review-only"));
            }
            _ => panic!("review-only capability must deny nonlocal shell commands"),
        }

        let dispatch = agent.run_prepared_call(
            dir.path(),
            "review-dispatch-write",
            "write",
            &serde_json::json!({"path":"dispatch.rs","content":"should not land"}),
            0,
        );
        assert!(dispatch.is_error);
        assert!(dispatch.content.contains("review-only"));
        assert!(!dir.path().join("dispatch.rs").exists());
    }
}
