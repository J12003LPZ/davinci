use async_trait::async_trait;
use pi_ai::{CompletionOptions, DynLanguageModel, ToolDefinition};
use pi_core::{AgentEvent, Message, Role};
use pi_session_sqlite::{SessionStore, SqliteSessionStore};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("Session storage error: {0}")]
    Session(#[from] pi_session_sqlite::SessionStoreError),
    #[error("AI provider error: {0}")]
    Ai(#[from] pi_ai::AiError),
    #[error("Lease acquisition failed for session: {0}")]
    LeaseFailed(String),
    #[error("Tool execution error: {0}")]
    Tool(String),
}

pub type Result<T> = std::result::Result<T, AgentError>;

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: serde_json::Value) -> Result<String>;
}

pub struct Agent {
    session_store: Arc<SqliteSessionStore>,
    model: DynLanguageModel,
    agent_id: String,
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

impl Agent {
    pub fn new(
        session_store: Arc<SqliteSessionStore>,
        model: DynLanguageModel,
        agent_id: Option<String>,
    ) -> Self {
        Self {
            session_store,
            model,
            agent_id: agent_id.unwrap_or_else(|| format!("agent-{}", Uuid::new_v4())),
            tools: HashMap::new(),
        }
    }

    pub fn register_tool(&mut self, tool: Arc<dyn AgentTool>) {
        let name = tool.definition().name;
        self.tools.insert(name, tool);
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    pub async fn run(
        &self,
        session_id: &str,
        prompt: &str,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String> {
        let lease_ok = self
            .session_store
            .acquire_writer_lease(session_id, &self.agent_id, 30_000)
            .await?;

        if !lease_ok {
            return Err(AgentError::LeaseFailed(session_id.to_string()));
        }

        let run_result = self.execute_loop(session_id, prompt, event_tx).await;

        let _ = self
            .session_store
            .release_writer_lease(session_id, &self.agent_id)
            .await;

        run_result
    }

    async fn execute_loop(
        &self,
        session_id: &str,
        prompt: &str,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String> {
        let now = Self::now_ms();
        if let Some(ref tx) = event_tx {
            let _ = tx
                .send(AgentEvent::Started {
                    session_id: session_id.to_string(),
                    timestamp: now,
                })
                .await;
        }

        let user_msg = Message {
            id: format!("msg-{}", Uuid::new_v4()),
            session_id: session_id.to_string(),
            role: Role::User,
            content: prompt.to_string(),
            tool_calls: None,
            tool_call_id: None,
            timestamp: now,
        };
        self.session_store.append_message(&user_msg).await?;

        let history = self.session_store.get_messages(session_id).await?;

        let tool_defs: Vec<ToolDefinition> = self.tools.values().map(|t| t.definition()).collect();

        let options = CompletionOptions {
            model: "default".to_string(),
            temperature: Some(0.7),
            max_tokens: Some(2048),
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs)
            },
        };

        let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(100);
        let event_tx_clone = event_tx.clone();
        let sid = session_id.to_string();

        let stream_forwarder = tokio::spawn(async move {
            let mut full = String::new();
            while let Some(chunk) = chunk_rx.recv().await {
                full.push_str(&chunk);
                if let Some(ref tx) = event_tx_clone {
                    let _ = tx
                        .send(AgentEvent::MessageChunk {
                            session_id: sid.clone(),
                            chunk,
                        })
                        .await;
                }
            }
            full
        });

        let response = self.model.stream(&history, &options, chunk_tx).await?;
        let _streamed_text = stream_forwarder.await.unwrap_or_default();

        let assistant_msg = Message {
            id: format!("msg-{}", Uuid::new_v4()),
            session_id: session_id.to_string(),
            role: Role::Assistant,
            content: response.content.clone(),
            tool_calls: response.tool_calls.clone(),
            tool_call_id: None,
            timestamp: Self::now_ms(),
        };
        self.session_store.append_message(&assistant_msg).await?;

        if let Some(ref tx) = event_tx {
            let _ = tx
                .send(AgentEvent::Completed {
                    session_id: session_id.to_string(),
                    timestamp: Self::now_ms(),
                })
                .await;
        }

        Ok(response.content)
    }
}
