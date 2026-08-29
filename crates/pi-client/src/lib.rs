//! Pi client: framed-protocol commands and exclusive/shared session leases.

use async_trait::async_trait;
use pi_core::{
    is_supported_protocol_version, ClientMessage, Command, CommandResult, ModelRef, ProtocolError,
    ServerMessage, ServerSnapshot, SessionLeaseMode, SessionSnapshot, ThinkingLevel,
    PROTOCOL_VERSION,
};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0}")]
    Protocol(String),
    #[error(transparent)]
    Server(#[from] ProtocolErrorAdapter),
    #[error("disconnected: {0}")]
    Disconnected(String),
    #[error("session {0} is detached")]
    Detached(String),
    #[error("session {0} ownership conflict")]
    Ownership(String),
    #[error("client is disposed")]
    Disposed,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ProtocolErrorAdapter(pub ProtocolError);

impl From<ProtocolError> for ClientError {
    fn from(value: ProtocolError) -> Self {
        Self::Server(ProtocolErrorAdapter(value))
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[async_trait]
pub trait ClientTransport: Send + Sync {
    async fn send(&self, message: ClientMessage) -> Result<ServerMessage>;
}

#[derive(Clone)]
struct LeaseState {
    mode: SessionLeaseMode,
    holders: usize,
    _generation: u64,
}

pub struct PiClient {
    transport: Arc<dyn ClientTransport>,
    snapshot: Mutex<Option<ServerSnapshot>>,
    leases: Mutex<HashMap<String, LeaseState>>,
    disposed: Mutex<bool>,
    connected: Mutex<bool>,
}

impl PiClient {
    pub fn new(transport: Arc<dyn ClientTransport>) -> Self {
        Self {
            transport,
            snapshot: Mutex::new(None),
            leases: Mutex::new(HashMap::new()),
            disposed: Mutex::new(false),
            connected: Mutex::new(false),
        }
    }

    pub async fn connect(&self) -> Result<ServerSnapshot> {
        self.ensure_open().await?;
        let reply = self
            .transport
            .send(ClientMessage::Hello {
                version: PROTOCOL_VERSION,
            })
            .await?;
        match reply {
            ServerMessage::Hello {
                version, snapshot, ..
            } => {
                if !is_supported_protocol_version(version) {
                    return Err(ClientError::from(ProtocolError::version()));
                }
                *self.snapshot.lock().await = Some(snapshot.clone());
                *self.connected.lock().await = true;
                Ok(snapshot)
            }
            ServerMessage::HelloError { error } => Err(error.into()),
            other => Err(ClientError::Protocol(format!(
                "unexpected handshake reply: {other:?}"
            ))),
        }
    }

    pub async fn snapshot(&self) -> Option<ServerSnapshot> {
        self.snapshot.lock().await.clone()
    }

    pub async fn list_sessions(&self) -> Result<Vec<pi_core::SessionMetadata>> {
        match self.request(Command::List).await? {
            CommandResult::List { sessions } => Ok(sessions),
            other => Err(ClientError::Protocol(format!(
                "unexpected list result {other:?}"
            ))),
        }
    }

    pub async fn create_session(
        &self,
        cwd: Option<&str>,
        name: Option<&str>,
    ) -> Result<SessionLease> {
        let result = self
            .request(Command::Create {
                cwd: cwd.map(ToOwned::to_owned),
                name: name.map(ToOwned::to_owned),
                model: None,
                thinking_level: None,
            })
            .await?;
        let snapshot = match result {
            CommandResult::Create { session } => session,
            other => {
                return Err(ClientError::Protocol(format!(
                    "unexpected create {other:?}"
                )))
            }
        };
        self.acquire_local(&snapshot.id, SessionLeaseMode::Exclusive)
            .await?;
        Ok(SessionLease {
            client: self,
            session_id: snapshot.id.clone(),
            snapshot,
            mode: SessionLeaseMode::Exclusive,
        })
    }

    pub async fn attach_session(&self, session_id: &str) -> Result<SessionLease> {
        let result = self
            .request(Command::Attach {
                session_id: session_id.to_string(),
            })
            .await?;
        let snapshot = match result {
            CommandResult::Attach { session } => session,
            other => {
                return Err(ClientError::Protocol(format!(
                    "unexpected attach {other:?}"
                )))
            }
        };
        self.acquire_local(&snapshot.id, SessionLeaseMode::Shared)
            .await?;
        Ok(SessionLease {
            client: self,
            session_id: snapshot.id.clone(),
            snapshot,
            mode: SessionLeaseMode::Shared,
        })
    }

    async fn request(&self, command: Command) -> Result<CommandResult> {
        self.ensure_open().await?;
        if !*self.connected.lock().await {
            return Err(ClientError::Disconnected("not connected".into()));
        }
        let id = format!("req-{}", Uuid::now_v7());
        let reply = self
            .transport
            .send(ClientMessage::Request {
                id: id.clone(),
                request: command,
            })
            .await?;
        match reply {
            ServerMessage::Response {
                id: rid,
                ok,
                result,
                error,
            } => {
                if rid != id {
                    return Err(ClientError::Protocol("response id mismatch".into()));
                }
                if !ok {
                    return Err(error.unwrap_or_else(ProtocolError::internal).into());
                }
                result.ok_or_else(|| ClientError::Protocol("missing result".into()))
            }
            ServerMessage::HelloError { error } => Err(error.into()),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    async fn acquire_local(&self, session_id: &str, mode: SessionLeaseMode) -> Result<()> {
        let mut leases = self.leases.lock().await;
        if let Some(existing) = leases.get_mut(session_id) {
            if mode == SessionLeaseMode::Exclusive || existing.mode == SessionLeaseMode::Exclusive {
                return Err(ClientError::Ownership(session_id.to_string()));
            }
            existing.holders += 1;
            return Ok(());
        }
        leases.insert(
            session_id.to_string(),
            LeaseState {
                mode,
                holders: 1,
                _generation: 1,
            },
        );
        Ok(())
    }

    async fn release_local(&self, session_id: &str) -> Result<bool> {
        let mut leases = self.leases.lock().await;
        let Some(existing) = leases.get_mut(session_id) else {
            return Ok(false);
        };
        existing.holders = existing.holders.saturating_sub(1);
        if existing.holders == 0 {
            leases.remove(session_id);
            return Ok(true);
        }
        Ok(false)
    }

    async fn ensure_open(&self) -> Result<()> {
        if *self.disposed.lock().await {
            return Err(ClientError::Disposed);
        }
        Ok(())
    }

    pub async fn dispose(&self) {
        *self.disposed.lock().await = true;
        *self.connected.lock().await = false;
        self.leases.lock().await.clear();
    }
}

pub struct SessionLease<'a> {
    client: &'a PiClient,
    session_id: String,
    snapshot: SessionSnapshot,
    mode: SessionLeaseMode,
}

impl<'a> SessionLease<'a> {
    pub fn id(&self) -> &str {
        &self.session_id
    }

    pub fn snapshot(&self) -> &SessionSnapshot {
        &self.snapshot
    }

    pub fn mode(&self) -> SessionLeaseMode {
        self.mode
    }

    pub async fn prompt(&mut self, text: &str) -> Result<SessionSnapshot> {
        let result = self
            .client
            .request(Command::Prompt {
                session_id: self.session_id.clone(),
                text: text.to_string(),
            })
            .await?;
        self.snapshot = match result {
            CommandResult::Prompt { session } => session,
            other => {
                return Err(ClientError::Protocol(format!(
                    "unexpected prompt {other:?}"
                )))
            }
        };
        Ok(self.snapshot.clone())
    }

    pub async fn abort(&mut self) -> Result<SessionSnapshot> {
        let result = self
            .client
            .request(Command::Abort {
                session_id: self.session_id.clone(),
            })
            .await?;
        self.snapshot = match result {
            CommandResult::Abort { session } => session,
            other => return Err(ClientError::Protocol(format!("unexpected abort {other:?}"))),
        };
        Ok(self.snapshot.clone())
    }

    pub async fn set_thinking(&mut self, level: ThinkingLevel) -> Result<SessionSnapshot> {
        let result = self
            .client
            .request(Command::SetThinking {
                session_id: self.session_id.clone(),
                thinking_level: level,
            })
            .await?;
        self.snapshot = match result {
            CommandResult::SetThinking { session } => session,
            other => {
                return Err(ClientError::Protocol(format!(
                    "unexpected set_thinking {other:?}"
                )))
            }
        };
        Ok(self.snapshot.clone())
    }

    pub async fn set_model(&mut self, model: ModelRef) -> Result<SessionSnapshot> {
        let result = self
            .client
            .request(Command::SetModel {
                session_id: self.session_id.clone(),
                model,
            })
            .await?;
        self.snapshot = match result {
            CommandResult::SetModel { session } => session,
            other => {
                return Err(ClientError::Protocol(format!(
                    "unexpected set_model {other:?}"
                )))
            }
        };
        Ok(self.snapshot.clone())
    }

    pub async fn detach(self) -> Result<()> {
        let last = self.client.release_local(&self.session_id).await?;
        if last {
            let _ = self
                .client
                .request(Command::Detach {
                    session_id: self.session_id.clone(),
                })
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{ProtocolErrorCode, SessionMetadata, SessionPhase};

    struct RejectHello;

    #[async_trait]
    impl ClientTransport for RejectHello {
        async fn send(&self, message: ClientMessage) -> Result<ServerMessage> {
            match message {
                ClientMessage::Hello { version } if version != 1 => Ok(ServerMessage::HelloError {
                    error: ProtocolError::version(),
                }),
                ClientMessage::Hello { .. } => Ok(ServerMessage::Hello {
                    version: 1,
                    connection_id: "c1".into(),
                    snapshot: ServerSnapshot {
                        server_id: "s".into(),
                        protocol_version: 1,
                        revision: 0,
                        sessions: Vec::new(),
                        models: Vec::new(),
                    },
                }),
                _ => Ok(ServerMessage::HelloError {
                    error: ProtocolError::invalid_request("request before hello"),
                }),
            }
        }
    }

    #[tokio::test]
    async fn handshake_accepts_version_one() {
        let client = PiClient::new(Arc::new(RejectHello));
        let snap = client.connect().await.unwrap();
        assert_eq!(snap.protocol_version, 1);
    }

    #[test]
    fn session_metadata_list_shape() {
        let meta = SessionMetadata {
            id: "s".into(),
            created_at: 1,
            updated_at: None,
            parent_session_id: None,
            session_name: None,
            cwd: Some("/tmp".into()),
            path: None,
            metadata: None,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["createdAt"], 1);
        assert!(json.get("phase").is_none());
        let _ = SessionPhase::Idle;
        let _ = ProtocolErrorCode::Version;
    }
}
