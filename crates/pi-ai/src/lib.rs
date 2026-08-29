use async_trait::async_trait;
use pi_core::{Message, ToolCall};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("API error: {0}")]
    Api(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Provider error: {0}")]
    Provider(String),
}

pub type Result<T> = std::result::Result<T, AiError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionOptions {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub finish_reason: FinishReason,
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
        chunk_tx: mpsc::Sender<String>,
    ) -> Result<CompletionResponse>;
}

pub struct MockLanguageModel {
    pub prefix: String,
}

impl Default for MockLanguageModel {
    fn default() -> Self {
        Self {
            prefix: "Echo: ".to_string(),
        }
    }
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
        let last_content = messages.last().map(|m| m.content.as_str()).unwrap_or("");
        let content = format!("{}{}", self.prefix, last_content);
        Ok(CompletionResponse {
            content,
            tool_calls: None,
            finish_reason: FinishReason::Stop,
        })
    }

    async fn stream(
        &self,
        messages: &[Message],
        _options: &CompletionOptions,
        chunk_tx: mpsc::Sender<String>,
    ) -> Result<CompletionResponse> {
        let last_content = messages.last().map(|m| m.content.as_str()).unwrap_or("");
        let content = format!("{}{}", self.prefix, last_content);

        for ch in content.chars() {
            let _ = chunk_tx.send(ch.to_string()).await;
        }

        Ok(CompletionResponse {
            content,
            tool_calls: None,
            finish_reason: FinishReason::Stop,
        })
    }
}

pub type DynLanguageModel = Arc<dyn LanguageModel>;
