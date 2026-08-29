//! Protocol server matching `@earendil-works/pi-server`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use pi_protocol::{
    encode_server_message, ClientMessage, ClientMessageDecoder, Command, CommandResult,
    ProtocolError, ProtocolErrorCode, ServerMessage, ServerSnapshot, SessionSnapshot,
    PROTOCOL_VERSION,
};
use pi_session::{discover_sessions, JsonlSession};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Protocol(String),
}

pub struct PiServer {
    pub server_id: String,
    pub sessions_dir: PathBuf,
    pub handshake_timeout: Duration,
    pub revision: u64,
    attachments: HashMap<String, String>,
}

impl PiServer {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self {
            server_id: Uuid::new_v4().to_string(),
            sessions_dir,
            handshake_timeout: Duration::from_secs(5),
            revision: 0,
            attachments: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        let sessions = discover_sessions(&self.sessions_dir, None)
            .unwrap_or_default()
            .into_iter()
            .map(|s| pi_protocol::SessionMetadata {
                id: s.id,
                created_at: s.created_at,
                updated_at: Some(s.modified_at),
                parent_session_id: s.parent_session_id,
                session_name: s.name,
                cwd: Some(s.cwd),
            })
            .collect();
        ServerSnapshot {
            server_id: self.server_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            revision: self.revision,
            sessions,
            models: Vec::new(),
        }
    }

    pub fn handle(&mut self, message: ClientMessage) -> ServerMessage {
        match message {
            ClientMessage::Hello { version } => {
                if version != PROTOCOL_VERSION {
                    return ServerMessage::HelloError {
                        error: ProtocolError {
                            code: ProtocolErrorCode::Version,
                            message: format!("Unsupported protocol version {version}"),
                            details: None,
                        },
                    };
                }
                ServerMessage::Hello {
                    version: PROTOCOL_VERSION,
                    connection_id: Uuid::new_v4().to_string(),
                    snapshot: self.snapshot(),
                }
            }
            ClientMessage::Request { id, request } => match self.dispatch(request) {
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
            },
        }
    }

    fn dispatch(&mut self, command: Command) -> Result<CommandResult, ProtocolError> {
        match command {
            Command::List => Ok(CommandResult::List {
                sessions: self.snapshot().sessions,
            }),
            Command::Create {
                cwd,
                name,
                model,
                thinking_level,
            } => {
                let cwd = cwd.unwrap_or_else(|| ".".into());
                let session = JsonlSession::create(&self.sessions_dir, &cwd, name.as_deref())
                    .map_err(|err| ProtocolError {
                        code: ProtocolErrorCode::InternalError,
                        message: err.to_string(),
                        details: None,
                    })?;
                self.revision += 1;
                Ok(CommandResult::Create {
                    session: snapshot_from_jsonl(
                        &session,
                        model,
                        thinking_level.unwrap_or(pi_protocol::ThinkingLevel::Off),
                    ),
                })
            }
            Command::Attach { session_id } => {
                self.attachments
                    .insert(session_id.clone(), "attached".into());
                let session = open_named(&self.sessions_dir, &session_id)?;
                Ok(CommandResult::Attach {
                    session: snapshot_from_jsonl(&session, None, pi_protocol::ThinkingLevel::Off),
                })
            }
            Command::Detach { session_id } => {
                self.attachments.remove(&session_id);
                Ok(CommandResult::Detach { session_id })
            }
            Command::Prompt { session_id, text } => {
                let mut session = open_named(&self.sessions_dir, &session_id)?;
                session
                    .append_entry(pi_session::SessionEntry::message(
                        "user",
                        serde_json::json!([{"type":"text","text": text}]),
                    ))
                    .map_err(|err| ProtocolError {
                        code: ProtocolErrorCode::InternalError,
                        message: err.to_string(),
                        details: None,
                    })?;
                Ok(CommandResult::Prompt {
                    session: snapshot_from_jsonl(&session, None, pi_protocol::ThinkingLevel::Off),
                })
            }
            Command::Steer { session_id, text } => {
                let mut session = open_named(&self.sessions_dir, &session_id)?;
                session
                    .append_entry(pi_session::SessionEntry::message(
                        "user",
                        serde_json::json!([{"type":"text","text": text}]),
                    ))
                    .map_err(|err| ProtocolError {
                        code: ProtocolErrorCode::InternalError,
                        message: err.to_string(),
                        details: None,
                    })?;
                Ok(CommandResult::Steer {
                    session: snapshot_from_jsonl(&session, None, pi_protocol::ThinkingLevel::Off),
                })
            }
            Command::Abort { session_id } => {
                let session = open_named(&self.sessions_dir, &session_id)?;
                Ok(CommandResult::Abort {
                    session: snapshot_from_jsonl(&session, None, pi_protocol::ThinkingLevel::Off),
                })
            }
            Command::SetModel { session_id, model } => {
                let session = open_named(&self.sessions_dir, &session_id)?;
                Ok(CommandResult::SetModel {
                    session: snapshot_from_jsonl(
                        &session,
                        Some(model),
                        pi_protocol::ThinkingLevel::Off,
                    ),
                })
            }
            Command::SetThinking {
                session_id,
                thinking_level,
            } => {
                let session = open_named(&self.sessions_dir, &session_id)?;
                Ok(CommandResult::SetThinking {
                    session: snapshot_from_jsonl(&session, None, thinking_level),
                })
            }
        }
    }
}

fn open_named(dir: &std::path::Path, session_id: &str) -> Result<JsonlSession, ProtocolError> {
    let summary =
        pi_session::resolve_session_ref(dir, None, session_id).map_err(|_| ProtocolError {
            code: ProtocolErrorCode::NotFound,
            message: format!("Session not found: {session_id}"),
            details: None,
        })?;
    JsonlSession::open(&summary.path).map_err(|err| ProtocolError {
        code: ProtocolErrorCode::NotFound,
        message: err.to_string(),
        details: None,
    })
}

fn snapshot_from_jsonl(
    session: &JsonlSession,
    model: Option<pi_protocol::ModelRef>,
    thinking_level: pi_protocol::ThinkingLevel,
) -> SessionSnapshot {
    SessionSnapshot {
        id: session.header.id.clone(),
        name: session.display_name(),
        cwd: session.header.cwd.clone(),
        created_at: session.header.created_at,
        updated_at: session.header.created_at,
        phase: pi_protocol::SessionPhase::Idle,
        model: model.unwrap_or_else(pi_protocol::default_model_ref),
        thinking_level,
        attached: true,
        locked: false,
        revision: session.entries.len() as u64,
        transcript: Vec::new(),
        queued_steer: Vec::new(),
        queued_steer_count: 0,
    }
}

pub fn bind_unix(path: &str) -> Result<UnixListener, ServerError> {
    UnixListener::bind(path).map_err(|err| ServerError::Io(err.to_string()))
}

pub fn bind_tcp(addr: &str) -> Result<TcpListener, ServerError> {
    TcpListener::bind(addr).map_err(|err| ServerError::Io(err.to_string()))
}

pub fn serve_stream<S: Read + Write>(
    server: &mut PiServer,
    mut stream: S,
) -> Result<(), ServerError> {
    let mut decoder =
        ClientMessageDecoder::new(None).map_err(|err| ServerError::Protocol(err.to_string()))?;
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .map_err(|err| ServerError::Io(err.to_string()))?;
        if n == 0 {
            break;
        }
        for message in decoder
            .push(&buf[..n])
            .map_err(|err| ServerError::Protocol(err.to_string()))?
        {
            let response = server.handle(message);
            let bytes = encode_server_message(&response, None)
                .map_err(|err| ServerError::Protocol(err.to_string()))?;
            stream
                .write_all(&bytes)
                .map_err(|err| ServerError::Io(err.to_string()))?;
        }
    }
    Ok(())
}

pub fn memory_roundtrip(server: &mut PiServer, message: ClientMessage) -> ServerMessage {
    server.handle(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::encode_client_message;
    use tempfile::tempdir;

    #[test]
    fn hello_and_create_over_memory() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let hello = memory_roundtrip(&mut server, ClientMessage::Hello { version: 1 });
        assert!(matches!(hello, ServerMessage::Hello { version: 1, .. }));
        let created = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "req-1".into(),
                request: Command::Create {
                    cwd: Some("/tmp/work".into()),
                    name: Some("demo".into()),
                    model: None,
                    thinking_level: None,
                },
            },
        );
        match created {
            ServerMessage::Response { ok, id, .. } => {
                assert!(ok);
                assert_eq!(id, "req-1");
            }
            _ => panic!("expected response"),
        }
        let _ = encode_client_message(&ClientMessage::Hello { version: 1 }, None).unwrap();
    }
}
