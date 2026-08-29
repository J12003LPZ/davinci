use std::path::PathBuf;

use crate::compaction::{compact_messages, should_auto_compact};
use crate::events::{AgentEvent, AgentMessage};
use crate::permission::{AllowAllPermissionPolicy, PermissionDecision, PermissionPolicy};
use crate::queues::{FollowUpQueue, SteerQueue};
use crate::tools::ToolRegistry;
use pi_ai::stream::{complete, StreamOptions};
use pi_ai::types::{ContentBlock, Message, Role};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Message(String),
}

pub struct AgentConfig {
    pub cwd: PathBuf,
    pub system_prompt: String,
    pub model_provider: String,
    pub model_id: String,
    pub api_key: Option<String>,
    pub allow_network: bool,
    pub auto_retry: bool,
    pub max_retries: u32,
    pub auto_compact: bool,
    pub context_window: usize,
    pub max_turns: u32,
    pub fixture: Option<pi_ai::stream::FixtureResponse>,
    pub permission: Box<dyn PermissionPolicy>,
    pub transport: Option<pi_ai::Transport>,
    pub session_id: Option<String>,
    pub base_url: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub api: Option<String>,
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("cwd", &self.cwd)
            .field("model_provider", &self.model_provider)
            .field("model_id", &self.model_id)
            .field("auto_retry", &self.auto_retry)
            .field("max_turns", &self.max_turns)
            .finish()
    }
}

impl Clone for AgentConfig {
    fn clone(&self) -> Self {
        Self {
            cwd: self.cwd.clone(),
            system_prompt: self.system_prompt.clone(),
            model_provider: self.model_provider.clone(),
            model_id: self.model_id.clone(),
            api_key: self.api_key.clone(),
            allow_network: self.allow_network,
            auto_retry: self.auto_retry,
            max_retries: self.max_retries,
            auto_compact: self.auto_compact,
            context_window: self.context_window,
            max_turns: self.max_turns,
            fixture: self.fixture.clone(),
            permission: Box::new(AllowAllPermissionPolicy),
            transport: self.transport,
            session_id: self.session_id.clone(),
            base_url: self.base_url.clone(),
            extra_headers: self.extra_headers.clone(),
            api: self.api.clone(),
        }
    }
}

fn to_ai_messages(messages: &[AgentMessage]) -> Vec<Message> {
    messages
        .iter()
        .map(|m| Message {
            role: match m.role.as_str() {
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                "system" => Role::System,
                _ => Role::User,
            },
            content: vec![ContentBlock::Text {
                text: m.content.clone(),
            }],
            timestamp: None,
        })
        .collect()
}

pub fn run_agent(
    config: &AgentConfig,
    messages: &[AgentMessage],
    tools: &ToolRegistry,
    steer: &mut SteerQueue,
    follow_up: &mut FollowUpQueue,
) -> Result<Vec<AgentEvent>, AgentError> {
    let mut events = vec![AgentEvent::AgentStart];
    let mut transcript = messages.to_vec();
    transcript.extend(steer.drain());
    let mut attempt = 0;
    let mut turn = 0;
    loop {
        turn += 1;
        if turn > config.max_turns {
            events.push(AgentEvent::Error {
                message: "max turns exceeded".into(),
            });
            break;
        }
        attempt += 1;
        events.push(AgentEvent::TurnStart { turn });
        if config.auto_compact
            && should_auto_compact(
                transcript.iter().map(|m| m.content.len()).sum(),
                config.context_window,
            )
        {
            let compacted = compact_messages(&transcript, None, 4);
            events.push(AgentEvent::Compaction {
                summary: compacted.summary.clone(),
            });
            transcript = compacted.retained_tail;
            transcript.insert(
                0,
                AgentMessage {
                    role: "user".into(),
                    content: compacted.summary,
                    images: vec![],
                },
            );
        }

        match complete(&StreamOptions {
            provider: config.model_provider.clone(),
            model: config.model_id.clone(),
            api_key: config.api_key.clone(),
            allow_network: config.allow_network,
            system: Some(config.system_prompt.clone()),
            messages: to_ai_messages(&transcript),
            tools: tools.schemas(),
            fixture: config.fixture.clone(),
            transport: config.transport,
            session_id: config.session_id.clone(),
            base_url: config.base_url.clone(),
            extra_headers: config.extra_headers.clone(),
            api: config.api.clone(),
            ..StreamOptions::default()
        }) {
            Ok(assistant) => {
                let text = assistant
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        pi_ai::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let message = AgentMessage {
                    role: "assistant".into(),
                    content: text,
                    images: vec![],
                };
                events.push(AgentEvent::Message {
                    message: message.clone(),
                });
                if let Some(usage) = assistant.usage {
                    events.push(AgentEvent::Usage { usage });
                }
                transcript.push(message);

                let mut ran_tools = false;
                for block in &assistant.content {
                    if let pi_ai::ContentBlock::ToolCall {
                        tool_call_id,
                        tool_name,
                        input,
                    } = block
                    {
                        ran_tools = true;
                        events.push(AgentEvent::ToolStart {
                            name: tool_name.clone(),
                            id: tool_call_id.clone(),
                        });
                        let decision = config.permission.decide(tool_name);
                        let result =
                            match decision {
                                PermissionDecision::Deny => crate::tools::ToolResult {
                                    output: format!("Error: permission denied for {tool_name}"),
                                    is_error: true,
                                    details: serde_json::json!({"denied": true}),
                                },
                                PermissionDecision::Ask | PermissionDecision::Allow => {
                                    match tools.get(tool_name) {
                                        Some(tool) => tool
                                            .execute(input, &config.cwd)
                                            .unwrap_or_else(|e| crate::tools::ToolResult {
                                                output: e.to_string(),
                                                is_error: true,
                                                details: serde_json::json!({}),
                                            }),
                                        None => crate::tools::ToolResult {
                                            output: format!("Error: unknown tool {tool_name}"),
                                            is_error: true,
                                            details: serde_json::json!({}),
                                        },
                                    }
                                }
                            };
                        events.push(AgentEvent::ToolEnd {
                            name: tool_name.clone(),
                            id: tool_call_id.clone(),
                            is_error: result.is_error,
                            output: result.output.clone(),
                        });
                        transcript.push(AgentMessage {
                            role: "tool".into(),
                            content: result.output,
                            images: vec![],
                        });
                    }
                }
                if ran_tools {
                    continue;
                }
                break;
            }
            Err(error) if config.auto_retry && attempt < config.max_retries => {
                events.push(AgentEvent::Retry {
                    attempt,
                    message: error.to_string(),
                });
                continue;
            }
            Err(error) => {
                events.push(AgentEvent::Error {
                    message: error.to_string(),
                });
                break;
            }
        }
    }
    transcript.extend(follow_up.drain());
    events.push(AgentEvent::AgentEnd);
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::AllowAllPermissionPolicy;
    use pi_ai::events::{AssistantMessage, AssistantMessageEvent, StopReason};

    #[test]
    fn fixture_turn_emits_agent_events() {
        let fixture = pi_ai::stream::FixtureResponse {
            events: vec![
                AssistantMessageEvent::Start {
                    partial: AssistantMessage {
                        id: "1".into(),
                        role: "assistant".into(),
                        content: vec![],
                        model: None,
                        stop_reason: None,
                        usage: None,
                        error_message: None,
                        timestamp: 1,
                    },
                },
                AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: AssistantMessage {
                        id: "1".into(),
                        role: "assistant".into(),
                        content: vec![pi_ai::ContentBlock::Text { text: "ok".into() }],
                        model: None,
                        stop_reason: Some(StopReason::Stop),
                        usage: Some(pi_ai::Usage {
                            input: 1,
                            output: 1,
                            total_tokens: 2,
                            ..pi_ai::Usage::default()
                        }),
                        error_message: None,
                        timestamp: 1,
                    },
                },
            ],
            sse: None,
        };
        let config = AgentConfig {
            cwd: std::env::current_dir().unwrap(),
            system_prompt: "test".into(),
            model_provider: "faux".into(),
            model_id: "fixture".into(),
            api_key: None,
            allow_network: false,
            auto_retry: false,
            max_retries: 0,
            auto_compact: false,
            context_window: 128_000,
            max_turns: 4,
            fixture: Some(fixture),
            permission: Box::new(AllowAllPermissionPolicy),
            transport: None,
            session_id: None,
            base_url: None,
            extra_headers: vec![],
            api: None,
        };
        let mut steer = SteerQueue::default();
        let mut follow = FollowUpQueue::default();
        let events = run_agent(
            &config,
            &[AgentMessage {
                role: "user".into(),
                content: "hi".into(),
                images: vec![],
            }],
            &ToolRegistry::builtins(),
            &mut steer,
            &mut follow,
        )
        .unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, AgentEvent::Message { message } if message.content == "ok")));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Usage { .. })));
    }

    #[test]
    fn executes_tool_calls_from_fixture() {
        let fixture = pi_ai::stream::FixtureResponse {
            events: vec![AssistantMessageEvent::Done {
                reason: StopReason::ToolUse,
                message: AssistantMessage {
                    id: "1".into(),
                    role: "assistant".into(),
                    content: vec![pi_ai::ContentBlock::ToolCall {
                        tool_call_id: "c1".into(),
                        tool_name: "bash".into(),
                        input: serde_json::json!({"command":"printf tool-ok"}),
                    }],
                    model: None,
                    stop_reason: Some(StopReason::ToolUse),
                    usage: None,
                    error_message: None,
                    timestamp: 1,
                },
            }],
            sse: None,
        };
        let config = AgentConfig {
            cwd: std::env::current_dir().unwrap(),
            system_prompt: "test".into(),
            model_provider: "faux".into(),
            model_id: "fixture".into(),
            api_key: None,
            allow_network: false,
            auto_retry: false,
            max_retries: 0,
            auto_compact: false,
            context_window: 128_000,
            max_turns: 1,
            fixture: Some(fixture),
            permission: Box::new(AllowAllPermissionPolicy),
            transport: None,
            session_id: None,
            base_url: None,
            extra_headers: vec![],
            api: None,
        };
        let events = run_agent(
            &config,
            &[AgentMessage {
                role: "user".into(),
                content: "run".into(),
                images: vec![],
            }],
            &ToolRegistry::builtins(),
            &mut SteerQueue::default(),
            &mut FollowUpQueue::default(),
        )
        .unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolEnd { output, .. } if output.contains("tool-ok")
        )));
    }
}
