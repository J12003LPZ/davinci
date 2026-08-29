//! Pi protocol client with exclusive/shared session leases.

pub mod unix;

use std::collections::HashMap;
use std::path::Path;

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

enum ClientIo {
    Memory {
        server: PiServer,
        connection_id: String,
    },
    Unix(unix::UnixTransport),
}

pub struct PiClient {
    io: ClientIo,
    pub snapshot: Option<ServerSnapshot>,
    leases: HashMap<String, u32>,
    request: u64,
}

impl PiClient {
    pub fn connect(server: PiServer) -> Result<Self, String> {
        let connection_id = Uuid::now_v7().to_string();
        let mut client = Self {
            io: ClientIo::Memory {
                server,
                connection_id,
            },
            snapshot: None,
            leases: HashMap::new(),
            request: 0,
        };
        client.finish_hello()
    }

    pub fn connect_unix(path: impl AsRef<Path>) -> Result<Self, String> {
        let transport = unix::UnixTransport::connect(path)?;
        let mut client = Self {
            io: ClientIo::Unix(transport),
            snapshot: None,
            leases: HashMap::new(),
            request: 0,
        };
        client.finish_hello()
    }

    fn finish_hello(mut self) -> Result<Self, String> {
        let hello = self.roundtrip(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        })?;
        if let ServerMessage::Hello { snapshot, .. } = hello {
            self.snapshot = Some(snapshot);
        } else if let ServerMessage::HelloError { error } = hello {
            return Err(error.message);
        }
        Ok(self)
    }

    fn roundtrip(&mut self, message: ClientMessage) -> Result<ServerMessage, String> {
        match &mut self.io {
            ClientIo::Memory {
                server,
                connection_id,
            } => {
                let bytes = encode_client_message(&message).map_err(|error| error.to_string())?;
                let reply = server
                    .accept_bytes(connection_id, &bytes)
                    .map_err(|error| error.to_string())?;
                let messages = ServerMessageDecoder::new()
                    .push(&reply)
                    .map_err(|error| error.to_string())?;
                messages
                    .into_iter()
                    .next()
                    .ok_or_else(|| "empty server reply".into())
            }
            ClientIo::Unix(transport) => {
                transport.send(&message)?;
                transport.recv()
            }
        }
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

    fn temp_socket_path(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "pi-client-{}-{}-{}.sock",
            label,
            std::process::id(),
            Uuid::now_v7()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn unix_loopback_hello_create_prompt_detach() {
        let path = temp_socket_path("loopback");
        let _listener = pi_server::listen_unix(
            PiServer::default(),
            &path,
            pi_server::UnixListenerOptions::default(),
        )
        .unwrap();
        let mut client = PiClient::connect_unix(&path).unwrap();
        assert!(client.snapshot.is_some());
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
    fn unix_stale_socket_rebind() {
        use std::os::unix::net::UnixListener;

        let path = temp_socket_path("stale");
        {
            let _stale = UnixListener::bind(&path).unwrap();
        }
        assert!(path.exists());
        let first = pi_server::listen_unix(
            PiServer::default(),
            &path,
            pi_server::UnixListenerOptions::default(),
        )
        .unwrap();
        let mut client = PiClient::connect_unix(first.path()).unwrap();
        assert!(client.snapshot.is_some());

        let error = pi_server::listen_unix(
            PiServer::default(),
            &path,
            pi_server::UnixListenerOptions::default(),
        )
        .unwrap_err();
        assert!(
            error.contains("already running"),
            "expected live-listener error, got {error}"
        );
    }

    #[test]
    fn unix_handshake_timeout() {
        use pi_protocol::{ProtocolErrorCode, ServerMessageDecoder};
        use std::io::Read;
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let path = temp_socket_path("handshake");
        let _listener = pi_server::listen_unix(
            PiServer::default(),
            &path,
            pi_server::UnixListenerOptions {
                handshake_timeout_ms: 50,
            },
        )
        .unwrap();

        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut decoder = ServerMessageDecoder::new();
        let mut buf = [0u8; 8192];
        let start = std::time::Instant::now();
        let message = loop {
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "timed out waiting for handshake error"
            );
            match stream.read(&mut buf) {
                Ok(0) => panic!("closed without hello_error"),
                Ok(n) => {
                    if let Some(message) = decoder.push(&buf[..n]).unwrap().into_iter().next() {
                        break message;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => panic!("{error}"),
            }
        };
        match message {
            ServerMessage::HelloError { error } => {
                assert_eq!(error.code, ProtocolErrorCode::InvalidRequest);
                assert_eq!(error.message, "Handshake timeout");
            }
            other => panic!("expected handshake timeout, got {other:?}"),
        }
    }
}
