//! Pi server: handshake, command dispatch, in-memory session runtimes.

use pi_agent::Agent;
use pi_core::{
    is_supported_protocol_version, ClientMessage, Command, CommandResult, ModelMetadata, ModelRef,
    ProtocolError, ServerMessage, ServerSnapshot, SessionMetadata, SessionPhase, SessionSnapshot,
    ThinkingLevel, PROTOCOL_VERSION,
};
use pi_session_sqlite::SqliteSessionRepository;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
struct LiveSession {
    snapshot: SessionSnapshot,
    attached: HashSet<String>,
}

#[derive(Clone)]
pub struct PiServer {
    session_store: Arc<SqliteSessionRepository>,
    agent: Arc<Agent>,
    server_id: String,
    revision: Arc<Mutex<i64>>,
    live: Arc<Mutex<HashMap<String, LiveSession>>>,
    models: Vec<ModelMetadata>,
}

impl PiServer {
    pub fn new(session_store: Arc<SqliteSessionRepository>, agent: Arc<Agent>) -> Self {
        Self {
            session_store,
            agent,
            server_id: Uuid::now_v7().to_string(),
            revision: Arc::new(Mutex::new(0)),
            live: Arc::new(Mutex::new(HashMap::new())),
            models: vec![ModelMetadata {
                provider: "faux".into(),
                id: "echo".into(),
                name: "Faux Echo".into(),
                authenticated: true,
            }],
        }
    }

    pub async fn handle_message(
        &self,
        connection_id: &str,
        message: ClientMessage,
    ) -> ServerMessage {
        match message {
            ClientMessage::Hello { version } => self.hello(connection_id, version).await,
            ClientMessage::Request { id, request } => {
                let (ok, result, error) = match self.dispatch(connection_id, request).await {
                    Ok(result) => (true, Some(result), None),
                    Err(error) => (false, None, Some(error)),
                };
                ServerMessage::Response {
                    id,
                    ok,
                    result,
                    error,
                }
            }
        }
    }

    async fn hello(&self, connection_id: &str, version: i64) -> ServerMessage {
        if !is_supported_protocol_version(version) {
            return ServerMessage::HelloError {
                error: ProtocolError::version(),
            };
        }
        match self.snapshot().await {
            Ok(snapshot) => ServerMessage::Hello {
                version: PROTOCOL_VERSION,
                connection_id: connection_id.to_string(),
                snapshot,
            },
            Err(_) => ServerMessage::HelloError {
                error: ProtocolError::internal(),
            },
        }
    }

    async fn snapshot(&self) -> Result<ServerSnapshot, ProtocolError> {
        let sessions = self
            .session_store
            .list(None)
            .map_err(|_| ProtocolError::internal())?;
        let revision = *self.revision.lock().await;
        Ok(ServerSnapshot {
            server_id: self.server_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            revision,
            sessions,
            models: self.models.clone(),
        })
    }

    async fn dispatch(
        &self,
        connection_id: &str,
        command: Command,
    ) -> Result<CommandResult, ProtocolError> {
        match command {
            Command::List => {
                let sessions = self
                    .session_store
                    .list(None)
                    .map_err(|_| ProtocolError::internal())?;
                Ok(CommandResult::List { sessions })
            }
            Command::Create {
                cwd,
                name,
                model,
                thinking_level,
            } => {
                let cwd = cwd.unwrap_or_else(|| ".".into());
                let created = self
                    .session_store
                    .create(None, &cwd, None, None)
                    .map_err(|_| ProtocolError::internal())?;
                if let Some(name) = name {
                    let _ = self
                        .session_store
                        .set_name(&created.metadata.id, Some(&name));
                }
                let snap = self
                    .materialize(
                        &created.metadata,
                        model.unwrap_or(ModelRef {
                            provider: "faux".into(),
                            id: "echo".into(),
                        }),
                        thinking_level.unwrap_or(ThinkingLevel::Off),
                    )
                    .await;
                self.attach_runtime(connection_id, snap.clone()).await;
                self.bump().await;
                Ok(CommandResult::Create { session: snap })
            }
            Command::Attach { session_id } => {
                let meta = self
                    .session_store
                    .list(None)
                    .map_err(|_| ProtocolError::internal())?
                    .into_iter()
                    .find(|s| s.id == session_id)
                    .ok_or_else(|| {
                        ProtocolError::not_found(format!("Session not found: {session_id}"))
                    })?;
                let snap = self
                    .materialize(
                        &meta,
                        ModelRef {
                            provider: "faux".into(),
                            id: "echo".into(),
                        },
                        ThinkingLevel::Off,
                    )
                    .await;
                self.attach_runtime(connection_id, snap.clone()).await;
                Ok(CommandResult::Attach { session: snap })
            }
            Command::Detach { session_id } => {
                let mut live = self.live.lock().await;
                if let Some(session) = live.get_mut(&session_id) {
                    session.attached.remove(connection_id);
                    if session.attached.is_empty() && session.snapshot.phase == SessionPhase::Idle {
                        live.remove(&session_id);
                    }
                }
                Ok(CommandResult::Detach { session_id })
            }
            Command::Prompt { session_id, text } => {
                self.require_attached(connection_id, &session_id).await?;
                self.set_phase(&session_id, SessionPhase::Turn).await;
                let reply = self
                    .agent
                    .run(&session_id, &text, None)
                    .await
                    .map_err(|_| ProtocolError::internal())?;
                let mut snap = self.require_snapshot(&session_id).await?;
                snap.transcript
                    .push(serde_json::json!({"role":"user","content": text}));
                snap.transcript
                    .push(serde_json::json!({"role":"assistant","content": reply}));
                snap.phase = SessionPhase::Idle;
                snap.updated_at = now_ms();
                snap.revision += 1;
                self.store_snapshot(snap.clone()).await;
                Ok(CommandResult::Prompt { session: snap })
            }
            Command::Steer { session_id, text } => {
                self.require_attached(connection_id, &session_id).await?;
                let mut snap = self.require_snapshot(&session_id).await?;
                if snap.phase != SessionPhase::Turn {
                    return Err(ProtocolError::busy("steer requires an active turn"));
                }
                snap.queued_steer.push(text);
                snap.queued_steer_count = snap.queued_steer.len() as i64;
                self.store_snapshot(snap.clone()).await;
                Ok(CommandResult::Steer { session: snap })
            }
            Command::Abort { session_id } => {
                self.require_attached(connection_id, &session_id).await?;
                let mut snap = self.require_snapshot(&session_id).await?;
                snap.phase = SessionPhase::Idle;
                snap.updated_at = now_ms();
                self.store_snapshot(snap.clone()).await;
                Ok(CommandResult::Abort { session: snap })
            }
            Command::SetModel { session_id, model } => {
                self.require_attached(connection_id, &session_id).await?;
                let mut snap = self.require_snapshot(&session_id).await?;
                if snap.phase != SessionPhase::Idle {
                    return Err(ProtocolError::busy("session is busy"));
                }
                snap.model = model;
                snap.updated_at = now_ms();
                snap.revision += 1;
                self.store_snapshot(snap.clone()).await;
                Ok(CommandResult::SetModel { session: snap })
            }
            Command::SetThinking {
                session_id,
                thinking_level,
            } => {
                self.require_attached(connection_id, &session_id).await?;
                let mut snap = self.require_snapshot(&session_id).await?;
                if snap.phase != SessionPhase::Idle {
                    return Err(ProtocolError::busy("session is busy"));
                }
                snap.thinking_level = thinking_level;
                snap.updated_at = now_ms();
                snap.revision += 1;
                self.store_snapshot(snap.clone()).await;
                Ok(CommandResult::SetThinking { session: snap })
            }
        }
    }

    async fn attach_runtime(&self, connection_id: &str, snapshot: SessionSnapshot) {
        let mut live = self.live.lock().await;
        let entry = live.entry(snapshot.id.clone()).or_insert(LiveSession {
            snapshot: snapshot.clone(),
            attached: HashSet::new(),
        });
        entry.snapshot = snapshot;
        entry.attached.insert(connection_id.to_string());
    }

    async fn require_attached(
        &self,
        connection_id: &str,
        session_id: &str,
    ) -> Result<(), ProtocolError> {
        let live = self.live.lock().await;
        match live.get(session_id) {
            Some(session) if session.attached.contains(connection_id) => Ok(()),
            Some(_) => Err(ProtocolError::invalid_request("session is not attached")),
            None => Err(ProtocolError::not_found(format!(
                "Session not found: {session_id}"
            ))),
        }
    }

    async fn require_snapshot(&self, session_id: &str) -> Result<SessionSnapshot, ProtocolError> {
        let live = self.live.lock().await;
        live.get(session_id)
            .map(|s| s.snapshot.clone())
            .ok_or_else(|| ProtocolError::not_found(format!("Session not found: {session_id}")))
    }

    async fn store_snapshot(&self, snapshot: SessionSnapshot) {
        let mut live = self.live.lock().await;
        if let Some(session) = live.get_mut(&snapshot.id) {
            session.snapshot = snapshot;
        }
    }

    async fn set_phase(&self, session_id: &str, phase: SessionPhase) {
        let mut live = self.live.lock().await;
        if let Some(session) = live.get_mut(session_id) {
            session.snapshot.phase = phase;
        }
    }

    async fn materialize(
        &self,
        meta: &SessionMetadata,
        model: ModelRef,
        thinking_level: ThinkingLevel,
    ) -> SessionSnapshot {
        SessionSnapshot {
            id: meta.id.clone(),
            name: meta.session_name.clone(),
            cwd: meta.cwd.clone().unwrap_or_else(|| ".".into()),
            created_at: meta.created_at,
            updated_at: meta.updated_at.unwrap_or(meta.created_at),
            phase: SessionPhase::Idle,
            model,
            thinking_level,
            attached: true,
            locked: false,
            revision: 1,
            transcript: Vec::new(),
            queued_steer: Vec::new(),
            queued_steer_count: 0,
        }
    }

    async fn bump(&self) {
        *self.revision.lock().await += 1;
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::MockLanguageModel;
    use pi_core::WriterLeaseOptions;

    #[tokio::test]
    async fn hello_rejects_unsupported_version() {
        let store = Arc::new(
            SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap(),
        );
        let agent = Arc::new(Agent::new(
            store.clone(),
            Arc::new(MockLanguageModel::new("Pi: ")),
        ));
        let server = PiServer::new(store, agent);
        let reply = server
            .handle_message("c1", ClientMessage::Hello { version: 99 })
            .await;
        match reply {
            ServerMessage::HelloError { error } => {
                assert_eq!(error.code, pi_core::ProtocolErrorCode::Version);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn create_prompt_roundtrip() {
        let store = Arc::new(
            SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap(),
        );
        let agent = Arc::new(Agent::new(
            store.clone(),
            Arc::new(MockLanguageModel::new("Pi: ")),
        ));
        let server = PiServer::new(store, agent);
        let hello = server
            .handle_message("c1", ClientMessage::Hello { version: 1 })
            .await;
        assert!(matches!(hello, ServerMessage::Hello { .. }));
        let created = server
            .handle_message(
                "c1",
                ClientMessage::Request {
                    id: "1".into(),
                    request: Command::Create {
                        cwd: Some("/tmp".into()),
                        name: Some("demo".into()),
                        model: None,
                        thinking_level: None,
                    },
                },
            )
            .await;
        let session_id = match created {
            ServerMessage::Response {
                result: Some(CommandResult::Create { session }),
                ..
            } => session.id,
            other => panic!("{other:?}"),
        };
        let prompted = server
            .handle_message(
                "c1",
                ClientMessage::Request {
                    id: "2".into(),
                    request: Command::Prompt {
                        session_id,
                        text: "hello".into(),
                    },
                },
            )
            .await;
        match prompted {
            ServerMessage::Response {
                result: Some(CommandResult::Prompt { session }),
                ..
            } => {
                assert_eq!(session.phase, SessionPhase::Idle);
                assert_eq!(session.transcript.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }
}
