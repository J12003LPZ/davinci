use pi_agent::Agent;
use pi_core::{AgentEvent, RpcError, RpcRequest, RpcResponse};
use pi_session_sqlite::{SessionStore, SqliteSessionStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct CreateSessionParams {
    title: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunPromptParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetMessagesParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

#[derive(Clone)]
pub struct PiServer {
    session_store: Arc<SqliteSessionStore>,
    agent: Arc<Agent>,
}

impl PiServer {
    pub fn new(session_store: Arc<SqliteSessionStore>, agent: Arc<Agent>) -> Self {
        Self {
            session_store,
            agent,
        }
    }

    pub async fn handle_rpc(
        &self,
        request: RpcRequest,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> RpcResponse {
        match request.method.as_str() {
            "session.create" => {
                match serde_json::from_value::<CreateSessionParams>(request.params) {
                    Ok(params) => {
                        let id = format!("sess-{}", Uuid::new_v4());
                        match self
                            .session_store
                            .create_session(&id, &params.title, &params.tags)
                            .await
                        {
                            Ok(session) => RpcResponse {
                                id: request.id,
                                result: Some(serde_json::to_value(session).unwrap()),
                                error: None,
                            },
                            Err(e) => RpcResponse {
                                id: request.id,
                                result: None,
                                error: Some(RpcError {
                                    code: -32000,
                                    message: e.to_string(),
                                    data: None,
                                }),
                            },
                        }
                    }
                    Err(e) => RpcResponse {
                        id: request.id,
                        result: None,
                        error: Some(RpcError {
                            code: -32602,
                            message: format!("Invalid params: {}", e),
                            data: None,
                        }),
                    },
                }
            }
            "session.list" => match self.session_store.list_sessions().await {
                Ok(sessions) => RpcResponse {
                    id: request.id,
                    result: Some(serde_json::to_value(sessions).unwrap()),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id: request.id,
                    result: None,
                    error: Some(RpcError {
                        code: -32000,
                        message: e.to_string(),
                        data: None,
                    }),
                },
            },
            "session.getMessages" => {
                match serde_json::from_value::<GetMessagesParams>(request.params) {
                    Ok(params) => match self.session_store.get_messages(&params.session_id).await {
                        Ok(msgs) => RpcResponse {
                            id: request.id,
                            result: Some(serde_json::to_value(msgs).unwrap()),
                            error: None,
                        },
                        Err(e) => RpcResponse {
                            id: request.id,
                            result: None,
                            error: Some(RpcError {
                                code: -32000,
                                message: e.to_string(),
                                data: None,
                            }),
                        },
                    },
                    Err(e) => RpcResponse {
                        id: request.id,
                        result: None,
                        error: Some(RpcError {
                            code: -32602,
                            message: format!("Invalid params: {}", e),
                            data: None,
                        }),
                    },
                }
            }
            "agent.run" => match serde_json::from_value::<RunPromptParams>(request.params) {
                Ok(params) => {
                    match self
                        .agent
                        .run(&params.session_id, &params.prompt, event_tx)
                        .await
                    {
                        Ok(response) => {
                            let res_val = serde_json::json!({ "response": response });
                            RpcResponse {
                                id: request.id,
                                result: Some(res_val),
                                error: None,
                            }
                        }
                        Err(e) => RpcResponse {
                            id: request.id,
                            result: None,
                            error: Some(RpcError {
                                code: -32000,
                                message: e.to_string(),
                                data: None,
                            }),
                        },
                    }
                }
                Err(e) => RpcResponse {
                    id: request.id,
                    result: None,
                    error: Some(RpcError {
                        code: -32602,
                        message: format!("Invalid params: {}", e),
                        data: None,
                    }),
                },
            },
            _ => RpcResponse {
                id: request.id,
                result: None,
                error: Some(RpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            },
        }
    }
}
