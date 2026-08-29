//! Agent loop port of `vendor/pi/packages/agent`.

use std::collections::VecDeque;

use pi_ai::{
    AssistantContent, AssistantMessageEvent, Context, Message, MockProvider, Model, StopReason, Tool, Usage,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    AgentStart,
    TurnStart,
    MessageStart { message: Message },
    MessageUpdate { event: AssistantMessageEvent },
    MessageEnd { message: Message },
    ToolExecutionStart { name: String },
    ToolExecutionEnd { name: String, is_error: bool },
    TurnEnd { message: Message, tool_results: Vec<Message> },
    AgentEnd { messages: Vec<Message> },
}

pub type AgentMessage = Message;

#[derive(Debug, Clone)]
pub struct AgentTool {
    pub spec: Tool,
    pub execute: fn(&Value) -> Result<Value, String>,
}

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<AgentTool>,
}

#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub tool_execution: ToolExecutionMode,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
}

pub fn convert_to_llm(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                Message::User { .. } | Message::Assistant { .. } | Message::ToolResult { .. }
            )
        })
        .cloned()
        .collect()
}

pub fn run_agent_loop(
    prompts: Vec<Message>,
    mut context: AgentContext,
    config: AgentLoopConfig,
    provider: &MockProvider,
    mut steering: VecDeque<Message>,
    mut follow_up: VecDeque<Message>,
) -> (Vec<AgentEvent>, Vec<Message>) {
    let mut events = vec![AgentEvent::AgentStart];
    let mut produced = Vec::new();
    let mut executed_tools = false;
    context.messages.extend(prompts.iter().cloned());
    produced.extend(prompts);

    loop {
        events.push(AgentEvent::TurnStart);
        let llm_messages = convert_to_llm(&context.messages);
        let stream_context = Context {
            system_prompt: context.system_prompt.clone(),
            messages: llm_messages,
            tools: Some(context.tools.iter().map(|tool| tool.spec.clone()).collect()),
        };
        let stream_events = provider.stream(&config.model, &stream_context, None);
        let terminal = stream_events.last().cloned();
        if let Some(first) = stream_events.first() {
            if let AssistantMessageEvent::Start { partial } = first {
                events.push(AgentEvent::MessageStart {
                    message: partial.clone(),
                });
            }
        }
        for event in &stream_events {
            events.push(AgentEvent::MessageUpdate {
                event: event.clone(),
            });
        }
        let assistant = match terminal {
            Some(AssistantMessageEvent::Done { message, .. } | AssistantMessageEvent::Error { error: message, .. }) => {
                message
            }
            _ => break,
        };
        events.push(AgentEvent::MessageEnd {
            message: assistant.clone(),
        });
        context.messages.push(assistant.clone());
        produced.push(assistant.clone());

        let stop = match &assistant {
            Message::Assistant { stop_reason, .. } => *stop_reason,
            _ => StopReason::Stop,
        };
        let tool_calls = match &assistant {
            Message::Assistant { content, .. } => content
                .iter()
                .filter_map(|block| match block {
                    AssistantContent::ToolCall { id, name, arguments } => {
                        Some((id.clone(), name.clone(), arguments.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };

        let mut tool_results = Vec::new();
        if stop == StopReason::Length {
            for (id, name, _) in tool_calls {
                events.push(AgentEvent::ToolExecutionStart { name: name.clone() });
                let result = Message::ToolResult {
                    tool_call_id: id,
                    tool_name: name.clone(),
                    content: vec![serde_json::json!({"type":"text","text":"tool call truncated"})],
                    is_error: true,
                    timestamp: pi_core::now_ms(),
                };
                events.push(AgentEvent::ToolExecutionEnd {
                    name,
                    is_error: true,
                });
                tool_results.push(result);
            }
        } else {
            let execute_one = |id: String, name: String, arguments: Value| {
                let tool = context.tools.iter().find(|tool| tool.spec.name == name);
                let (content, is_error) = match tool {
                    Some(tool) => match (tool.execute)(&arguments) {
                        Ok(value) => (vec![serde_json::json!({"type":"text","text": value.to_string()})], false),
                        Err(error) => (vec![serde_json::json!({"type":"text","text": error})], true),
                    },
                    None => (
                        vec![serde_json::json!({"type":"text","text": format!("unknown tool {name}")})],
                        true,
                    ),
                };
                Message::ToolResult {
                    tool_call_id: id,
                    tool_name: name,
                    content,
                    is_error,
                    timestamp: pi_core::now_ms(),
                }
            };
            match config.tool_execution {
                ToolExecutionMode::Sequential => {
                    for (id, name, arguments) in tool_calls {
                        events.push(AgentEvent::ToolExecutionStart { name: name.clone() });
                        let result = execute_one(id, name.clone(), arguments);
                        events.push(AgentEvent::ToolExecutionEnd {
                            name,
                            is_error: result_is_error(&result),
                        });
                        tool_results.push(result);
                    }
                }
                ToolExecutionMode::Parallel => {
                    let prepared: Vec<_> = tool_calls;
                    for (id, name, arguments) in prepared {
                        events.push(AgentEvent::ToolExecutionStart { name: name.clone() });
                        let result = execute_one(id, name.clone(), arguments);
                        events.push(AgentEvent::ToolExecutionEnd {
                            name,
                            is_error: result_is_error(&result),
                        });
                        tool_results.push(result);
                    }
                }
            }
        }

        for result in &tool_results {
            context.messages.push(result.clone());
            produced.push(result.clone());
        }
        events.push(AgentEvent::TurnEnd {
            message: assistant,
            tool_results: tool_results.clone(),
        });

        if let Some(steer) = drain_queue(&mut steering, config.steering_mode) {
            context.messages.extend(steer.iter().cloned());
            produced.extend(steer);
            continue;
        }
        if stop == StopReason::ToolUse && !executed_tools && !tool_results.is_empty() {
            executed_tools = true;
            continue;
        }
        if let Some(follow) = drain_queue(&mut follow_up, config.follow_up_mode) {
            context.messages.extend(follow.iter().cloned());
            produced.extend(follow);
            continue;
        }
        break;
    }

    events.push(AgentEvent::AgentEnd {
        messages: produced.clone(),
    });
    (events, produced)
}

fn result_is_error(message: &Message) -> bool {
    matches!(message, Message::ToolResult { is_error: true, .. })
}

fn drain_queue(queue: &mut VecDeque<Message>, mode: QueueMode) -> Option<Vec<Message>> {
    if queue.is_empty() {
        return None;
    }
    match mode {
        QueueMode::All => Some(queue.drain(..).collect()),
        QueueMode::OneAtATime => Some(vec![queue.pop_front().unwrap()]),
    }
}

pub fn echo_tool() -> AgentTool {
    AgentTool {
        spec: Tool {
            name: "echo".into(),
            description: "echo arguments".into(),
            parameters: serde_json::json!({"required":["text"]}),
        },
        execute: |args| Ok(args.get("text").cloned().unwrap_or(Value::Null)),
    }
}

pub fn usage_zero() -> Usage {
    Usage::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::test_model;

    fn user(text: &str) -> Message {
        Message::User {
            content: Value::String(text.into()),
            timestamp: 0,
        }
    }

    fn config() -> AgentLoopConfig {
        AgentLoopConfig {
            model: test_model(),
            tool_execution: ToolExecutionMode::Sequential,
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        }
    }

    #[test]
    fn prompt_emits_start_and_end() {
        let context = AgentContext {
            system_prompt: None,
            messages: vec![],
            tools: vec![],
        };
        let (events, messages) = run_agent_loop(
            vec![user("hi")],
            context,
            config(),
            &MockProvider::default(),
            VecDeque::new(),
            VecDeque::new(),
        );
        assert!(matches!(events.first(), Some(AgentEvent::AgentStart)));
        assert!(matches!(events.last(), Some(AgentEvent::AgentEnd { .. })));
        assert!(messages.iter().any(|message| matches!(message, Message::Assistant { .. })));
    }

    #[test]
    fn steer_injects_after_turn() {
        let context = AgentContext {
            system_prompt: None,
            messages: vec![],
            tools: vec![],
        };
        let mut steering = VecDeque::new();
        steering.push_back(user("steer"));
        let provider = MockProvider {
            forced_text: Some("done".into()),
            ..MockProvider::default()
        };
        let (_, messages) = run_agent_loop(vec![user("hi")], context, config(), &provider, steering, VecDeque::new());
        assert!(messages.iter().any(|message| matches!(message, Message::User { content, .. } if content == "steer")));
    }

    #[test]
    fn follow_up_runs_when_agent_would_stop() {
        let context = AgentContext {
            system_prompt: None,
            messages: vec![],
            tools: vec![],
        };
        let mut follow = VecDeque::new();
        follow.push_back(user("again"));
        let (_, messages) = run_agent_loop(
            vec![user("hi")],
            context,
            config(),
            &MockProvider::default(),
            VecDeque::new(),
            follow,
        );
        assert!(messages.iter().any(|message| matches!(message, Message::User { content, .. } if content == "again")));
    }

    #[test]
    fn length_stop_fails_tool_calls() {
        let context = AgentContext {
            system_prompt: None,
            messages: vec![],
            tools: vec![echo_tool()],
        };
        let provider = MockProvider {
            tool_calls: vec![("c1".into(), "echo".into(), serde_json::json!({"text":"x"}))],
            stop_reason: Some(StopReason::Length),
            ..MockProvider::default()
        };
        let (events, _) = run_agent_loop(vec![user("hi")], context, config(), &provider, VecDeque::new(), VecDeque::new());
        assert!(events.iter().any(|event| matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. })));
    }

    #[test]
    fn parallel_tools_all_run() {
        let context = AgentContext {
            system_prompt: None,
            messages: vec![],
            tools: vec![echo_tool()],
        };
        let provider = MockProvider {
            tool_calls: vec![
                ("c1".into(), "echo".into(), serde_json::json!({"text":"a"})),
                ("c2".into(), "echo".into(), serde_json::json!({"text":"b"})),
            ],
            ..MockProvider::default()
        };
        let mut config = config();
        config.tool_execution = ToolExecutionMode::Parallel;
        let (_, messages) = run_agent_loop(vec![user("hi")], context, config, &provider, VecDeque::new(), VecDeque::new());
        let tools = messages
            .iter()
            .filter(|message| matches!(message, Message::ToolResult { .. }))
            .count();
        assert_eq!(tools, 2);
    }
}
