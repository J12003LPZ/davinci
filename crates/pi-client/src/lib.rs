use async_trait::async_trait;
use pi_core::{AgentEvent, Message, RpcRequest, RpcResponse, SessionMetadata};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("RPC error: {code} - {message}")]
    Rpc { code: i32, message: String },
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Transport error: {0}")]
    Transport(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[async_trait]
pub trait ClientTransport: Send + Sync {
    async fn send(&self, request: RpcRequest) -> Result<RpcResponse>;
    async fn subscribe_events(&self) -> Result<mpsc::Receiver<AgentEvent>>;
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateSessionParams {
    title: String,
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunPromptParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunPromptResult {
    response: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetMessagesParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

pub struct PiClient {
    transport: Arc<dyn ClientTransport>,
}

impl PiClient {
    pub fn new(transport: Arc<dyn ClientTransport>) -> Self {
        Self { transport }
    }

    pub async fn create_session(&self, title: &str, tags: &[String]) -> Result<SessionMetadata> {
        let req = RpcRequest {
            id: format!("req-{}", Uuid::new_v4()),
            method: "session.create".to_string(),
            params: serde_json::to_value(CreateSessionParams {
                title: title.to_string(),
                tags: tags.to_vec(),
            })?,
        };

        let res = self.transport.send(req).await?;
        if let Some(err) = res.error {
            return Err(ClientError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        let meta: SessionMetadata = serde_json::from_value(res.result.unwrap_or_default())?;
        Ok(meta)
    }

    pub async fn run_prompt(&self, session_id: &str, prompt: &str) -> Result<String> {
        let req = RpcRequest {
            id: format!("req-{}", Uuid::new_v4()),
            method: "agent.run".to_string(),
            params: serde_json::to_value(RunPromptParams {
                session_id: session_id.to_string(),
                prompt: prompt.to_string(),
            })?,
        };

        let res = self.transport.send(req).await?;
        if let Some(err) = res.error {
            return Err(ClientError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        let run_res: RunPromptResult = serde_json::from_value(res.result.unwrap_or_default())?;
        Ok(run_res.response)
    }

    pub async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let req = RpcRequest {
            id: format!("req-{}", Uuid::new_v4()),
            method: "session.getMessages".to_string(),
            params: serde_json::to_value(GetMessagesParams {
                session_id: session_id.to_string(),
            })?,
        };

        let res = self.transport.send(req).await?;
        if let Some(err) = res.error {
            return Err(ClientError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        let msgs: Vec<Message> = serde_json::from_value(res.result.unwrap_or_default())?;
        Ok(msgs)
    }
}
