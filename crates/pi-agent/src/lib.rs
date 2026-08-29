//! Agent loop: thought-action-observation with official length-stop fail-all.

use async_trait::async_trait;
use pi_ai::{validate_tool_arguments, CompletionOptions, LanguageModel, ToolDefinition};
use pi_core::{AgentEvent, Message, Role, StopReason, ToolCall};
use pi_session_sqlite::SqliteSessionRepository;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Session(#[from] pi_session_sqlite::StoreError),
    #[error(transparent)]
    Ai(#[from] pi_ai::AiError),
    #[error("writer lease was not available for session {0}")]
    LeaseFailed(String),
    #[error("tool: {0}")]
    Tool(String),
}

pub type Result<T> = std::result::Result<T, AgentError>;

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: serde_json::Value) -> Result<String>;
}

pub struct Agent {
    session_store: Arc<SqliteSessionRepository>,
    model: Arc<dyn LanguageModel>,
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

impl Agent {
    pub fn new(session_store: Arc<SqliteSessionRepository>, model: Arc<dyn LanguageModel>) -> Self {
        Self {
            session_store,
            model,
            tools: HashMap::new(),
        }
    }

    pub fn register_tool(&mut self, tool: Arc<dyn AgentTool>) {
        self.tools.insert(tool.definition().name, tool);
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_millis() as i64
    }

    pub async fn run(
        &self,
        session_id: &str,
        prompt: &str,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String> {
        let now = Self::now_ms();
        if let Some(tx) = &event_tx {
            let _ = tx
                .send(AgentEvent::Started {
                    session_id: session_id.to_string(),
                    timestamp: now,
                })
                .await;
        }

        let user_id = format!("msg-{}", Uuid::now_v7());
        self.session_store
            .append_message(session_id, "main", &user_id, Role::User, prompt)?;

        let mut history = self.load_history(session_id)?;
        let last_text;

        loop {
            let options = CompletionOptions {
                model: "default".into(),
                temperature: Some(0.0),
                max_tokens: Some(2048),
                tools: if self.tools.is_empty() {
                    None
                } else {
                    Some(self.tools.values().map(|t| t.definition()).collect())
                },
            };

            let (tx, mut rx) = mpsc::channel(64);
            let sid = session_id.to_string();
            let event_clone = event_tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    if let Some(tx) = &event_clone {
                        match ev {
                            pi_core::AssistantMessageEvent::TextDelta { message_id, delta } => {
                                let _ = tx
                                    .send(AgentEvent::MessageUpdate {
                                        session_id: sid.clone(),
                                        message_id,
                                        chunk: delta,
                                    })
                                    .await;
                            }
                            pi_core::AssistantMessageEvent::Start { message_id } => {
                                let _ = tx
                                    .send(AgentEvent::MessageStart {
                                        session_id: sid.clone(),
                                        message_id,
                                    })
                                    .await;
                            }
                            _ => {}
                        }
                    }
                }
            });

            let response = self.model.stream(&history, &options, tx).await?;
            let _ = forward.await;

            if response.stop_reason == StopReason::Length {
                if let Some(calls) = &response.tool_calls {
                    for call in calls {
                        if let Some(tx) = &event_tx {
                            let _ = tx
                                .send(AgentEvent::ToolExecutionEnd {
                                    session_id: session_id.to_string(),
                                    tool_call_id: call.id.clone(),
                                    result: "tool call aborted: stopReason=length".into(),
                                })
                                .await;
                        }
                    }
                }
                last_text = response.content;
                self.persist_assistant(session_id, &last_text, None)?;
                break;
            }

            if response.stop_reason == StopReason::Error
                || response.stop_reason == StopReason::Aborted
            {
                last_text = response.content;
                self.persist_assistant(session_id, &last_text, None)?;
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(AgentEvent::Error {
                            session_id: session_id.to_string(),
                            error: "stream terminated".into(),
                        })
                        .await;
                }
                break;
            }

            if let Some(calls) = response.tool_calls.clone() {
                self.persist_assistant(session_id, &response.content, Some(calls.clone()))?;
                let mut results = Vec::new();
                for call in calls {
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(AgentEvent::ToolCallStart {
                                session_id: session_id.to_string(),
                                tool_call: call.clone(),
                            })
                            .await;
                    }
                    let result = self.execute_tool(&call).await;
                    let text = match result {
                        Ok(text) => text,
                        Err(err) => err.to_string(),
                    };
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(AgentEvent::ToolExecutionEnd {
                                session_id: session_id.to_string(),
                                tool_call_id: call.id.clone(),
                                result: text.clone(),
                            })
                            .await;
                    }
                    let tool_msg = Message {
                        id: format!("msg-{}", Uuid::now_v7()),
                        session_id: session_id.to_string(),
                        role: Role::Tool,
                        content: text.clone(),
                        tool_calls: None,
                        tool_call_id: Some(call.id.clone()),
                        timestamp: Self::now_ms(),
                    };
                    self.session_store.append_message(
                        session_id,
                        "main",
                        &tool_msg.id,
                        Role::Tool,
                        &text,
                    )?;
                    results.push(tool_msg);
                }
                history = self.load_history(session_id)?;
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(AgentEvent::TurnEnd {
                            session_id: session_id.to_string(),
                        })
                        .await;
                }
                continue;
            }

            last_text = response.content;
            self.persist_assistant(session_id, &last_text, None)?;
            break;
        }

        if let Some(tx) = &event_tx {
            let _ = tx
                .send(AgentEvent::Completed {
                    session_id: session_id.to_string(),
                    timestamp: Self::now_ms(),
                })
                .await;
        }
        Ok(last_text)
    }

    fn persist_assistant(
        &self,
        session_id: &str,
        content: &str,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Result<()> {
        let id = format!("msg-{}", Uuid::now_v7());
        self.session_store
            .append_message(session_id, "main", &id, Role::Assistant, content)?;
        let _ = tool_calls;
        Ok(())
    }

    fn load_history(&self, session_id: &str) -> Result<Vec<Message>> {
        let entries = self.session_store.entries(session_id)?;
        Ok(entries
            .into_iter()
            .filter_map(|e| match e {
                pi_core::Entry::Message {
                    id,
                    timestamp,
                    message,
                    ..
                } => Some(Message {
                    id,
                    session_id: session_id.to_string(),
                    role: message.role,
                    content: message.content,
                    tool_calls: message.tool_calls,
                    tool_call_id: message.tool_call_id,
                    timestamp,
                }),
                _ => None,
            })
            .collect())
    }

    async fn execute_tool(&self, call: &ToolCall) -> Result<String> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| AgentError::Tool(format!("unknown tool {}", call.name)))?;
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or_else(|_| serde_json::json!({}));
        validate_tool_arguments(&tool.definition().parameters, &args)?;
        tool.execute(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{FauxLanguageModel, FauxStep, MockLanguageModel};
    use pi_core::WriterLeaseOptions;
    use pi_session_sqlite::SqliteSessionRepository;

    struct EchoTool;

    #[async_trait]
    impl AgentTool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".into(),
                description: "echo".into(),
                parameters: serde_json::json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}}}),
            }
        }

        async fn execute(&self, args: serde_json::Value) -> Result<String> {
            Ok(args["text"].as_str().unwrap_or("").to_string())
        }
    }

    #[tokio::test]
    async fn runs_prompt_and_persists_turn() {
        let store = Arc::new(
            SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap(),
        );
        store.create(Some("s"), "/tmp", None, None).unwrap();
        let agent = Agent::new(store.clone(), Arc::new(MockLanguageModel::new("Pi: ")));
        let reply = agent.run("s", "hello", None).await.unwrap();
        assert_eq!(reply, "Pi: hello");
        assert_eq!(store.entries("s").unwrap().len(), 2);
    }

    #[tokio::test]
    async fn length_stop_fails_all_tools() {
        let store = Arc::new(
            SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap(),
        );
        store.create(Some("s"), "/tmp", None, None).unwrap();
        let model = FauxLanguageModel::new(vec![FauxStep::Length("cut".into())]);
        let mut agent = Agent::new(store, Arc::new(model));
        agent.register_tool(Arc::new(EchoTool));
        let (tx, mut rx) = mpsc::channel(16);
        let _ = agent.run("s", "go", Some(tx)).await.unwrap();
        let mut failed = false;
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::ToolExecutionEnd { result, .. } = ev {
                if result.contains("stopReason=length") {
                    failed = true;
                }
            }
        }
        assert!(failed);
    }
}
