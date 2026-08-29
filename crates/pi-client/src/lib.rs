//! Pi protocol client with exclusive/shared session leases.

use std::collections::HashMap;

use pi_protocol::{
    encode_client_message, ClientMessage, Command, CommandResult, ServerMessage,
    ServerMessageDecoder, ServerSnapshot, SessionSnapshot, PROTOCOL_VERSION,
};
use pi_server::PiServer;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseMode {
    Exclusive,
    Shared,
}

#[derive(Debug, Clone)]
pub struct SessionLease {
    pub id: String,
    pub mode: LeaseMode,
    pub snapshot: SessionSnapshot,
    pub active: bool,
}

pub struct PiClient {
    server: PiServer,
    connection_id: String,
    pub snapshot: Option<ServerSnapshot>,
    leases: HashMap<String, u32>,
    request: u64,
}

impl PiClient {
    pub fn connect(server: PiServer) -> Result<Self, String> {
        let connection_id = Uuid::now_v7().to_string();
        let mut client = Self {
            server,
            connection_id,
            snapshot: None,
            leases: HashMap::new(),
            request: 0,
        };
        let hello = client.roundtrip(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        })?;
        if let ServerMessage::Hello { snapshot, .. } = hello {
            client.snapshot = Some(snapshot);
        } else if let ServerMessage::HelloError { error } = hello {
            return Err(error.message);
        }
        Ok(client)
    }

    fn roundtrip(&mut self, message: ClientMessage) -> Result<ServerMessage, String> {
        let bytes = encode_client_message(&message).map_err(|error| error.to_string())?;
        let reply = self
            .server
            .accept_bytes(&self.connection_id, &bytes)
            .map_err(|error| error.to_string())?;
        let messages = ServerMessageDecoder::new()
            .push(&reply)
            .map_err(|error| error.to_string())?;
        messages
            .into_iter()
            .next()
            .ok_or_else(|| "empty server reply".into())
    }

    fn request(&mut self, command: Command) -> Result<CommandResult, String> {
        self.request += 1;
        let id = self.request.to_string();
        match self.roundtrip(ClientMessage::Request {
            id: id.clone(),
            request: command,
        })? {
            ServerMessage::Response {
                id: response_id,
                ok: true,
                result: Some(result),
                ..
            } if response_id == id => Ok(result),
            ServerMessage::Response {
                ok: false,
                error: Some(error),
                ..
            } => Err(error.message),
            other => Err(format!("unexpected response {other:?}")),
        }
    }

    pub fn list_sessions(&mut self) -> Result<Vec<pi_protocol::SessionMetadata>, String> {
        match self.request(Command::List)? {
            CommandResult::List { sessions } => Ok(sessions),
            _ => Err("command mismatch".into()),
        }
    }

    pub fn create_session(
        &mut self,
        cwd: Option<String>,
        name: Option<String>,
    ) -> Result<SessionLease, String> {
        match self.request(Command::Create {
            cwd,
            name,
            model: None,
            thinking_level: None,
        })? {
            CommandResult::Create { session } => {
                self.leases.insert(session.id.clone(), 1);
                Ok(SessionLease {
                    id: session.id.clone(),
                    mode: LeaseMode::Exclusive,
                    snapshot: session,
                    active: true,
                })
            }
            _ => Err("command mismatch".into()),
        }
    }

    pub fn attach_session(&mut self, session_id: &str) -> Result<SessionLease, String> {
        match self.request(Command::Attach {
            session_id: session_id.to_string(),
        })? {
            CommandResult::Attach { session } => {
                *self.leases.entry(session.id.clone()).or_insert(0) += 1;
                Ok(SessionLease {
                    id: session.id.clone(),
                    mode: LeaseMode::Shared,
                    snapshot: session,
                    active: true,
                })
            }
            _ => Err("command mismatch".into()),
        }
    }

    pub fn prompt(
        &mut self,
        lease: &mut SessionLease,
        text: &str,
    ) -> Result<SessionSnapshot, String> {
        self.ensure_active(lease)?;
        match self.request(Command::Prompt {
            session_id: lease.id.clone(),
            text: text.to_string(),
        })? {
            CommandResult::Prompt { session } => {
                if session.revision >= lease.snapshot.revision {
                    lease.snapshot = session.clone();
                }
                Ok(session)
            }
            _ => Err("command mismatch".into()),
        }
    }

    pub fn detach(&mut self, lease: &mut SessionLease) -> Result<(), String> {
        self.ensure_active(lease)?;
        let remaining = self
            .leases
            .get(&lease.id)
            .copied()
            .unwrap_or(0)
            .saturating_sub(1);
        if remaining == 0 {
            self.request(Command::Detach {
                session_id: lease.id.clone(),
            })?;
            self.leases.remove(&lease.id);
        } else {
            self.leases.insert(lease.id.clone(), remaining);
        }
        lease.active = false;
        Ok(())
    }

    fn ensure_active(&self, lease: &SessionLease) -> Result<(), String> {
        if lease.active {
            Ok(())
        } else {
            Err("session lease is not active".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_create_prompt_detach() {
        let mut client = PiClient::connect(PiServer::default()).unwrap();
        let mut lease = client
            .create_session(Some("/tmp".into()), Some("demo".into()))
            .unwrap();
        let snapshot = client.prompt(&mut lease, "hello").unwrap();
        assert!(snapshot.transcript.len() >= 2);
        client.detach(&mut lease).unwrap();
        assert!(!lease.active);
        assert!(client
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| session.id == snapshot.id));
    }

    #[test]
    fn shared_leases_refcount_detach() {
        let mut client = PiClient::connect(PiServer::default()).unwrap();
        let created = client.create_session(None, None).unwrap();
        let mut shared = client.attach_session(&created.id).unwrap();
        assert_eq!(shared.mode, LeaseMode::Shared);
        client.detach(&mut shared).unwrap();
    }
}
