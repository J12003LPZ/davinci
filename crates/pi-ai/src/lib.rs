//! Unified LLM types, stream protocol, and faux provider.
//! `stream` never throws: failures become terminal assistant events.

use async_trait::async_trait;
use pi_core::{AssistantMessageEvent, Message, Role, StopReason, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, AiError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompletionOptions {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone)]
pub enum FauxStep {
    Text(String),
    Tool {
        name: String,
        arguments: String,
    },
    Error(String),
    /// Simulate a truncated generation (`stopReason == length`).
    Length(String),
}

#[async_trait]
pub trait LanguageModel: Send + Sync {
    async fn generate(
        &self,
        messages: &[Message],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse>;

    async fn stream(
        &self,
        messages: &[Message],
        options: &CompletionOptions,
        events: mpsc::Sender<AssistantMessageEvent>,
    ) -> Result<CompletionResponse>;
}

/// Scripted provider used as the unit-test authority (mirrors TS `faux`).
pub struct FauxLanguageModel {
    steps: std::sync::Mutex<Vec<FauxStep>>,
}

impl FauxLanguageModel {
    pub fn new(steps: Vec<FauxStep>) -> Self {
        Self {
            steps: std::sync::Mutex::new(steps),
        }
    }

    pub fn echo(prefix: &str) -> Self {
        Self::new(vec![]).with_prefix(prefix)
    }

    fn with_prefix(self, prefix: &str) -> Self {
        Self {
            steps: std::sync::Mutex::new(vec![FauxStep::Text(prefix.to_string())]),
        }
    }
}

impl Default for FauxLanguageModel {
    fn default() -> Self {
        Self::echo("Echo: ")
    }
}

/// Prefix-echo model used by conformance and print-mode tests.
pub struct MockLanguageModel {
    pub prefix: String,
}

impl MockLanguageModel {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
        }
    }
}

#[async_trait]
impl LanguageModel for MockLanguageModel {
    async fn generate(
        &self,
        messages: &[Message],
        _options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        let last = messages.last().map(|m| m.content.as_str()).unwrap_or("");
        Ok(CompletionResponse {
            content: format!("{}{last}", self.prefix),
            tool_calls: None,
            stop_reason: StopReason::Stop,
        })
    }

    async fn stream(
        &self,
        messages: &[Message],
        _options: &CompletionOptions,
        events: mpsc::Sender<AssistantMessageEvent>,
    ) -> Result<CompletionResponse> {
        let message_id = format!("msg-{}", Uuid::now_v7());
        let last = messages.last().map(|m| m.content.as_str()).unwrap_or("");
        let content = format!("{}{last}", self.prefix);
        let _ = events
            .send(AssistantMessageEvent::Start {
                message_id: message_id.clone(),
            })
            .await;
        for ch in content.chars() {
            let _ = events
                .send(AssistantMessageEvent::TextDelta {
                    message_id: message_id.clone(),
                    delta: ch.to_string(),
                })
                .await;
        }
        let _ = events
            .send(AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            })
            .await;
        Ok(CompletionResponse {
            content,
            tool_calls: None,
            stop_reason: StopReason::Stop,
        })
    }
}

#[async_trait]
impl LanguageModel for FauxLanguageModel {
    async fn generate(
        &self,
        messages: &[Message],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        let (tx, _rx) = mpsc::channel(16);
        self.stream(messages, options, tx).await
    }

    async fn stream(
        &self,
        messages: &[Message],
        _options: &CompletionOptions,
        events: mpsc::Sender<AssistantMessageEvent>,
    ) -> Result<CompletionResponse> {
        let message_id = format!("msg-{}", Uuid::now_v7());
        let step = {
            let mut steps = self.steps.lock().unwrap();
            if steps.is_empty() {
                None
            } else {
                Some(steps.remove(0))
            }
        };
        let _ = events
            .send(AssistantMessageEvent::Start {
                message_id: message_id.clone(),
            })
            .await;

        let result = match step {
            Some(FauxStep::Text(text)) => {
                let _ = events
                    .send(AssistantMessageEvent::TextDelta {
                        message_id: message_id.clone(),
                        delta: text.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::Done {
                        stop_reason: StopReason::Stop,
                    })
                    .await;
                CompletionResponse {
                    content: text,
                    tool_calls: None,
                    stop_reason: StopReason::Stop,
                }
            }
            Some(FauxStep::Tool { name, arguments }) => {
                let id = format!("call-{}", Uuid::now_v7());
                let _ = events
                    .send(AssistantMessageEvent::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::ToolCallDelta {
                        id: id.clone(),
                        delta: arguments.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::ToolCallEnd {
                        id: id.clone(),
                        arguments: arguments.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    })
                    .await;
                CompletionResponse {
                    content: String::new(),
                    tool_calls: Some(vec![ToolCall {
                        id,
                        name,
                        arguments,
                    }]),
                    stop_reason: StopReason::ToolUse,
                }
            }
            Some(FauxStep::Length(text)) => {
                let _ = events
                    .send(AssistantMessageEvent::TextDelta {
                        message_id: message_id.clone(),
                        delta: text.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::Done {
                        stop_reason: StopReason::Length,
                    })
                    .await;
                CompletionResponse {
                    content: text,
                    tool_calls: Some(vec![ToolCall {
                        id: "truncated".into(),
                        name: "read".into(),
                        arguments: "{".into(),
                    }]),
                    stop_reason: StopReason::Length,
                }
            }
            Some(FauxStep::Error(message)) => {
                let _ = events
                    .send(AssistantMessageEvent::Error {
                        message: message.clone(),
                    })
                    .await;
                CompletionResponse {
                    content: String::new(),
                    tool_calls: None,
                    stop_reason: StopReason::Error,
                }
            }
            None => {
                let last = messages.last().map(|m| m.content.as_str()).unwrap_or("");
                let text = format!("Echo: {last}");
                let _ = events
                    .send(AssistantMessageEvent::TextDelta {
                        message_id: message_id.clone(),
                        delta: text.clone(),
                    })
                    .await;
                let _ = events
                    .send(AssistantMessageEvent::Done {
                        stop_reason: StopReason::Stop,
                    })
                    .await;
                CompletionResponse {
                    content: text,
                    tool_calls: None,
                    stop_reason: StopReason::Stop,
                }
            }
        };
        Ok(result)
    }
}

pub fn validate_tool_arguments(parameters: &Value, args: &Value) -> Result<()> {
    if !args.is_object() {
        return Err(AiError::Message("tool arguments must be an object".into()));
    }
    if let Some(required) = parameters.get("required").and_then(|v| v.as_array()) {
        for key in required {
            if let Some(name) = key.as_str() {
                if args.get(name).is_none() {
                    return Err(AiError::Message(format!(
                        "missing required argument {name}"
                    )));
                }
            }
        }
    }
    Ok(())
}

pub fn last_user_text(messages: &[Message]) -> &str {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_never_throws_on_scripted_error() {
        let model = FauxLanguageModel::new(vec![FauxStep::Error("boom".into())]);
        let (tx, mut rx) = mpsc::channel(8);
        let response = model
            .stream(&[], &CompletionOptions::default(), tx)
            .await
            .unwrap();
        assert_eq!(response.stop_reason, StopReason::Error);
        let mut saw_error = false;
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AssistantMessageEvent::Error { .. }) {
                saw_error = true;
            }
        }
        assert!(saw_error);
    }

    #[test]
    fn validates_required_tool_args() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"]
        });
        assert!(validate_tool_arguments(&schema, &serde_json::json!({"path":"a"})).is_ok());
        assert!(validate_tool_arguments(&schema, &serde_json::json!({})).is_err());
    }
}
