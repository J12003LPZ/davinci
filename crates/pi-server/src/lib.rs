//! In-memory Pi protocol server.

mod unix;

pub use unix::{listen_unix, UnixListenerOptions, UnixServer, DEFAULT_HANDSHAKE_TIMEOUT_MS};

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use pi_core::now_ms;
use pi_protocol::{
    encode_server_message, ClientMessage, ClientMessageDecoder, Command, CommandResult, ModelRef,
    ProtocolError, ProtocolErrorCode, ServerEvent, ServerMessage, ServerSnapshot, SessionMetadata,
    SessionPhase, SessionSnapshot, PROTOCOL_VERSION,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct LiveSession {
    pub snapshot: SessionSnapshot,
    pub attached: HashSet<String>,
}

#[derive(Clone)]
pub struct MemoryService {
    inner: Arc<Mutex<MemoryServiceInner>>,
}

struct MemoryServiceInner {
    server_id: String,
    revision: u64,
    sessions: HashMap<String, LiveSession>,
    models: Vec<serde_json::Value>,
}

impl Default for MemoryService {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemoryServiceInner {
                server_id: Uuid::now_v7().to_string(),
                revision: 1,
                sessions: HashMap::new(),
                models: vec![serde_json::json!({
                    "provider": "mock",
                    "id": "mock-1",
                    "name": "Mock",
                    "api": "openai-completions",
                    "reasoning": false,
                    "input": ["text"],
                    "contextWindow": 8192,
                    "maxTokens": 1024,
                    "cost": {"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
                    "supportedThinkingLevels": ["off"],
                    "authenticated": true
                })],
            })),
        }
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        let inner = self.inner.lock().expect("service");
        ServerSnapshot {
            server_id: inner.server_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            revision: inner.revision,
            sessions: inner
                .sessions
                .values()
                .map(|session| SessionMetadata {
                    id: session.snapshot.id.clone(),
                    created_at: session.snapshot.created_at,
                    updated_at: Some(session.snapshot.updated_at),
                    parent_session_id: None,
                    session_name: session.snapshot.name.clone(),
                    cwd: Some(session.snapshot.cwd.clone()),
                })
                .collect(),
            models: inner.models.clone(),
        }
    }

    fn create_session(
        &self,
        cwd: Option<String>,
        name: Option<String>,
        model: Option<ModelRef>,
    ) -> SessionSnapshot {
        let mut inner = self.inner.lock().expect("service");
        inner.revision += 1;
        let id = Uuid::now_v7().to_string();
        let now = now_ms();
        let snapshot = SessionSnapshot {
            id: id.clone(),
            name,
            cwd: cwd.unwrap_or_else(|| ".".into()),
            created_at: now,
            updated_at: now,
            phase: SessionPhase::Idle,
            model: model.unwrap_or(ModelRef {
                provider: "mock".into(),
                id: "mock-1".into(),
            }),
            thinking_level: "off".into(),
            attached: true,
            locked: true,
            revision: 1,
            transcript: Vec::new(),
            queued_steer: Vec::new(),
            queued_steer_count: 0,
        };
        inner.sessions.insert(
            id,
            LiveSession {
                snapshot: snapshot.clone(),
                attached: HashSet::new(),
            },
        );
        snapshot
    }

    fn mutate(
        &self,
        session_id: &str,
        connection_id: &str,
        require_attach: bool,
        op: impl FnOnce(&mut SessionSnapshot),
    ) -> Result<SessionSnapshot, ProtocolError> {
        let mut inner = self.inner.lock().expect("service");
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| ProtocolError {
                code: ProtocolErrorCode::NotFound,
                message: format!("session {session_id} not found"),
                details: None,
            })?;
        if require_attach && !session.attached.contains(connection_id) {
            return Err(ProtocolError {
                code: ProtocolErrorCode::InvalidRequest,
                message: "connection is not attached to this session".into(),
                details: None,
            });
        }
        op(&mut session.snapshot);
        session.snapshot.updated_at = now_ms();
        session.snapshot.revision += 1;
        session.snapshot.attached = session.attached.contains(connection_id);
        Ok(session.snapshot.clone())
    }

    fn attach(
        &self,
        session_id: &str,
        connection_id: &str,
    ) -> Result<SessionSnapshot, ProtocolError> {
        let mut inner = self.inner.lock().expect("service");
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| ProtocolError {
                code: ProtocolErrorCode::NotFound,
                message: format!("session {session_id} not found"),
                details: None,
            })?;
        session.attached.insert(connection_id.to_string());
        session.snapshot.attached = true;
        session.snapshot.revision += 1;
        Ok(session.snapshot.clone())
    }

    fn detach(&self, session_id: &str, connection_id: &str) -> Result<String, ProtocolError> {
        let mut inner = self.inner.lock().expect("service");
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.attached.remove(connection_id);
            session.snapshot.attached = false;
        }
        Ok(session_id.to_string())
    }
}

#[derive(Default)]
pub struct PiServer {
    pub service: MemoryService,
}

impl PiServer {
    pub fn handle(&self, connection_id: &str, message: ClientMessage) -> Vec<ServerMessage> {
        match message {
            ClientMessage::Hello { version } => {
                if !pi_protocol::is_supported_protocol_version(version) {
                    return vec![ServerMessage::HelloError {
                        error: ProtocolError {
                            code: ProtocolErrorCode::Version,
                            message: format!("unsupported protocol version {version}"),
                            details: None,
                        },
                    }];
                }
                vec![ServerMessage::Hello {
                    version: PROTOCOL_VERSION,
                    connection_id: connection_id.to_string(),
                    snapshot: self.service.snapshot(),
                }]
            }
            ClientMessage::Request { id, request } => {
                vec![self.dispatch(connection_id, id, request)]
            }
        }
    }

    fn dispatch(&self, connection_id: &str, id: String, request: Command) -> ServerMessage {
        let result = match request {
            Command::List => Ok(CommandResult::List {
                sessions: self.service.snapshot().sessions,
            }),
            Command::Create {
                cwd,
                name,
                model,
                thinking_level: _,
            } => {
                let mut snapshot = self.service.create_session(cwd, name, model);
                let _ = self.service.attach(&snapshot.id, connection_id);
                snapshot.attached = true;
                Ok(CommandResult::Create { session: snapshot })
            }
            Command::Attach { session_id } => self
                .service
                .attach(&session_id, connection_id)
                .map(|session| CommandResult::Attach { session }),
            Command::Detach { session_id } => self
                .service
                .detach(&session_id, connection_id)
                .map(|session_id| CommandResult::Detach { session_id }),
            Command::Prompt { session_id, text } => self
                .service
                .mutate(&session_id, connection_id, true, |snapshot| {
                    snapshot.phase = SessionPhase::Turn;
                    snapshot.transcript.push(serde_json::json!({
                        "role": "user",
                        "content": [{"type":"text","text": text}],
                    }));
                    snapshot.transcript.push(serde_json::json!({
                        "role": "assistant",
                        "status": "complete",
                        "content": [{"type":"text","text": format!("echo:{text}")}],
                    }));
                    snapshot.phase = SessionPhase::Idle;
                })
                .map(|session| CommandResult::Prompt { session }),
            Command::Steer { session_id, text } => self
                .service
                .mutate(&session_id, connection_id, true, |snapshot| {
                    snapshot
                        .queued_steer
                        .push(serde_json::json!({"text": text}));
                    snapshot.queued_steer_count = snapshot.queued_steer.len() as u32;
                })
                .map(|session| CommandResult::Steer { session }),
            Command::Abort { session_id } => self
                .service
                .mutate(&session_id, connection_id, true, |snapshot| {
                    snapshot.phase = SessionPhase::Idle;
                })
                .map(|session| CommandResult::Abort { session }),
            Command::SetModel { session_id, model } => self
                .service
                .mutate(&session_id, connection_id, true, |snapshot| {
                    snapshot.model = model;
                })
                .map(|session| CommandResult::SetModel { session }),
            Command::SetThinking {
                session_id,
                thinking_level,
            } => self
                .service
                .mutate(&session_id, connection_id, true, |snapshot| {
                    snapshot.thinking_level = thinking_level;
                })
                .map(|session| CommandResult::SetThinking { session }),
        };
        match result {
            Ok(result) => ServerMessage::Response {
                id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => ServerMessage::Response {
                id,
                ok: false,
                result: None,
                error: Some(error),
            },
        }
    }

    pub fn accept_bytes(&self, connection_id: &str, incoming: &[u8]) -> Result<Vec<u8>, String> {
        let mut decoder = ClientMessageDecoder::new();
        let messages = decoder.push(incoming).map_err(|error| error.to_string())?;
        decoder.end().map_err(|error| error.to_string())?;
        let mut outgoing = Vec::new();
        for message in messages {
            for reply in self.handle(connection_id, message) {
                outgoing.extend(encode_server_message(&reply).map_err(|error| error.to_string())?);
            }
        }
        Ok(outgoing)
    }
}

pub fn event_progress(session_id: &str) -> ServerMessage {
    ServerMessage::Event {
        event: ServerEvent::SessionProgress {
            session_id: session_id.to_string(),
            progress: serde_json::json!({"type":"item_started"}),
        },
    }
}

pub type Outbox = Arc<Mutex<VecDeque<Vec<u8>>>>;

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{encode_client_message, ServerMessageDecoder};

    #[test]
    fn handshake_and_create_prompt() {
        let server = PiServer::default();
        let hello = encode_client_message(&ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        })
        .unwrap();
        let reply = server.accept_bytes("c1", &hello).unwrap();
        let mut decoder = ServerMessageDecoder::new();
        let messages = decoder.push(&reply).unwrap();
        assert!(matches!(messages[0], ServerMessage::Hello { .. }));

        let create = encode_client_message(&ClientMessage::Request {
            id: "1".into(),
            request: Command::Create {
                cwd: Some("/tmp".into()),
                name: Some("demo".into()),
                model: None,
                thinking_level: None,
            },
        })
        .unwrap();
        let reply = server.accept_bytes("c1", &create).unwrap();
        let messages = ServerMessageDecoder::new().push(&reply).unwrap();
        let ServerMessage::Response {
            result: Some(CommandResult::Create { session }),
            ..
        } = &messages[0]
        else {
            panic!("expected create");
        };
        let prompt = encode_client_message(&ClientMessage::Request {
            id: "2".into(),
            request: Command::Prompt {
                session_id: session.id.clone(),
                text: "hi".into(),
            },
        })
        .unwrap();
        let reply = server.accept_bytes("c1", &prompt).unwrap();
        let messages = ServerMessageDecoder::new().push(&reply).unwrap();
        assert!(matches!(
            messages[0],
            ServerMessage::Response {
                ok: true,
                result: Some(CommandResult::Prompt { .. }),
                ..
            }
        ));
    }

    fn temp_socket_path(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "pi-server-{}-{}-{}.sock",
            label,
            std::process::id(),
            Uuid::now_v7()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn read_server_message(stream: &mut std::os::unix::net::UnixStream) -> ServerMessage {
        use std::io::Read;
        let mut decoder = ServerMessageDecoder::new();
        let mut buf = [0u8; 8192];
        let start = std::time::Instant::now();
        loop {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(2),
                "timed out waiting for server message"
            );
            match stream.read(&mut buf) {
                Ok(0) => panic!("connection closed before a server message arrived"),
                Ok(n) => {
                    let messages = decoder.push(&buf[..n]).unwrap();
                    if let Some(message) = messages.into_iter().next() {
                        return message;
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
        }
    }

    fn write_client_message(stream: &mut std::os::unix::net::UnixStream, message: &ClientMessage) {
        use std::io::Write;
        let bytes = encode_client_message(message).unwrap();
        stream.write_all(&bytes).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn unix_loopback_hello_create_prompt_detach() {
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let path = temp_socket_path("loopback");
        let _listener =
            listen_unix(PiServer::default(), &path, UnixListenerOptions::default()).unwrap();

        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        write_client_message(
            &mut stream,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
            },
        );
        assert!(matches!(
            read_server_message(&mut stream),
            ServerMessage::Hello { .. }
        ));

        write_client_message(
            &mut stream,
            &ClientMessage::Request {
                id: "1".into(),
                request: Command::Create {
                    cwd: Some("/tmp".into()),
                    name: Some("demo".into()),
                    model: None,
                    thinking_level: None,
                },
            },
        );
        let ServerMessage::Response {
            result: Some(CommandResult::Create { session }),
            ..
        } = read_server_message(&mut stream)
        else {
            panic!("expected create");
        };

        write_client_message(
            &mut stream,
            &ClientMessage::Request {
                id: "2".into(),
                request: Command::Prompt {
                    session_id: session.id.clone(),
                    text: "hi".into(),
                },
            },
        );
        assert!(matches!(
            read_server_message(&mut stream),
            ServerMessage::Response {
                ok: true,
                result: Some(CommandResult::Prompt { .. }),
                ..
            }
        ));

        write_client_message(
            &mut stream,
            &ClientMessage::Request {
                id: "3".into(),
                request: Command::Detach {
                    session_id: session.id,
                },
            },
        );
        assert!(matches!(
            read_server_message(&mut stream),
            ServerMessage::Response {
                ok: true,
                result: Some(CommandResult::Detach { .. }),
                ..
            }
        ));
    }

    #[test]
    fn unix_stale_socket_rebind() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let path = temp_socket_path("stale");
        {
            let _stale = UnixListener::bind(&path).unwrap();
        }
        assert!(path.exists());

        let first =
            listen_unix(PiServer::default(), &path, UnixListenerOptions::default()).unwrap();
        UnixStream::connect(first.path()).unwrap();

        let error =
            listen_unix(PiServer::default(), &path, UnixListenerOptions::default()).unwrap_err();
        assert!(
            error.contains("already running"),
            "expected live-listener error, got {error}"
        );
        UnixStream::connect(first.path()).unwrap();
    }

    #[test]
    fn unix_handshake_timeout() {
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let path = temp_socket_path("handshake");
        let _listener = listen_unix(
            PiServer::default(),
            &path,
            UnixListenerOptions {
                handshake_timeout_ms: 50,
            },
        )
        .unwrap();

        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        match read_server_message(&mut stream) {
            ServerMessage::HelloError { error } => {
                assert_eq!(error.code, ProtocolErrorCode::InvalidRequest);
                assert_eq!(error.message, "Handshake timeout");
            }
            other => panic!("expected handshake timeout, got {other:?}"),
        }
    }
}
