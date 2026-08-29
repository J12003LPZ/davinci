use std::path::PathBuf;

use crate::compaction::{compact_messages, should_auto_compact};
use crate::events::{AgentEvent, AgentMessage};
use crate::queues::{FollowUpQueue, SteerQueue};
use crate::tools::ToolRegistry;
use pi_ai::stream::{complete, StreamOptions};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub cwd: PathBuf,
    pub system_prompt: String,
    pub model_provider: String,
    pub model_id: String,
    pub auto_retry: bool,
    pub max_retries: u32,
    pub auto_compact: bool,
    pub context_window: usize,
    pub fixture: Option<pi_ai::stream::FixtureResponse>,
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
    loop {
        attempt += 1;
        events.push(AgentEvent::TurnStart { turn: attempt });
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
            fixture: config.fixture.clone(),
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
    let _ = tools;
    events.push(AgentEvent::AgentEnd);
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            auto_retry: false,
            max_retries: 0,
            auto_compact: false,
            context_window: 128_000,
            fixture: Some(fixture),
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
}
