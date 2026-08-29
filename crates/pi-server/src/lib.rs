//! Protocol server matching `@earendil-works/pi-server`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use pi_protocol::{
    encode_server_message, AssistantContent, ClientMessage, ClientMessageDecoder, Command,
    CommandResult, ModelCost, ModelMetadata, ModelRef, ProtocolError, ProtocolErrorCode,
    ServerMessage, ServerSnapshot, SessionPhase, SessionSnapshot, TextOrImage, ThinkingLevel,
    TranscriptItem, PROTOCOL_VERSION,
};
use pi_session::{discover_sessions, JsonlSession};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Protocol(String),
}

struct LiveSession {
    session: JsonlSession,
    phase: SessionPhase,
    model: ModelRef,
    thinking_level: ThinkingLevel,
    queued_steer: Vec<TranscriptItem>,
    attached: bool,
}

pub struct PiServer {
    pub server_id: String,
    pub sessions_dir: PathBuf,
    pub handshake_timeout: Duration,
    pub revision: u64,
    attachments: HashMap<String, String>,
    live: HashMap<String, LiveSession>,
}

impl PiServer {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self {
            server_id: Uuid::new_v4().to_string(),
            sessions_dir,
            handshake_timeout: Duration::from_secs(5),
            revision: 0,
            attachments: HashMap::new(),
            live: HashMap::new(),
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
            models: builtin_models(),
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
                    .map_err(internal)?;
                let id = session.header.id.clone();
                let live = LiveSession {
                    session,
                    phase: SessionPhase::Idle,
                    model: model.unwrap_or_else(pi_protocol::default_model_ref),
                    thinking_level: thinking_level.unwrap_or(ThinkingLevel::Off),
                    queued_steer: Vec::new(),
                    attached: true,
                };
                self.live.insert(id.clone(), live);
                self.revision += 1;
                Ok(CommandResult::Create {
                    session: self.live_snapshot(&id)?,
                })
            }
            Command::Attach { session_id } => {
                self.attachments
                    .insert(session_id.clone(), "attached".into());
                self.ensure_live(&session_id)?;
                if let Some(live) = self.live.get_mut(&session_id) {
                    live.attached = true;
                }
                Ok(CommandResult::Attach {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::Detach { session_id } => {
                self.attachments.remove(&session_id);
                if let Some(live) = self.live.get_mut(&session_id) {
                    live.attached = false;
                }
                Ok(CommandResult::Detach { session_id })
            }
            Command::Prompt { session_id, text } => {
                self.ensure_live(&session_id)?;
                {
                    let live = self
                        .live
                        .get_mut(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase != SessionPhase::Idle {
                        return Err(busy("A prompt is already running"));
                    }
                    live.session
                        .append_entry(pi_session::SessionEntry::message(
                            "user",
                            serde_json::json!([{"type":"text","text": text}]),
                        ))
                        .map_err(internal)?;
                    live.phase = SessionPhase::Turn;
                    live.queued_steer.clear();
                    if let Ok(reply) = std::env::var("PI_SERVER_PROMPT_REPLY") {
                        let reply = if reply.is_empty() {
                            format!("reply:{text}")
                        } else {
                            reply
                        };
                        live.session
                            .append_entry(pi_session::SessionEntry::message(
                                "assistant",
                                serde_json::json!([{"type":"text","text": reply}]),
                            ))
                            .map_err(internal)?;
                        live.phase = SessionPhase::Idle;
                    } else if std::env::var("PI_SERVER_KEEP_TURN").is_err() {
                        live.phase = SessionPhase::Idle;
                    }
                }
                Ok(CommandResult::Prompt {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::Steer { session_id, text } => {
                self.ensure_live(&session_id)?;
                {
                    let live = self
                        .live
                        .get_mut(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase == SessionPhase::Idle {
                        return Err(busy("There is no active prompt to steer"));
                    }
                    let item = TranscriptItem::User {
                        id: format!("steer-{}", live.session.entries.len() + 1),
                        content: vec![TextOrImage::Text { text }],
                        timestamp: live.session.entries.len() as u64 + 1,
                    };
                    live.queued_steer.push(item);
                }
                Ok(CommandResult::Steer {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::Abort { session_id } => {
                self.ensure_live(&session_id)?;
                {
                    let live = self
                        .live
                        .get_mut(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase == SessionPhase::Idle {
                        return Err(busy("There is no active prompt to abort"));
                    }
                    live.session
                        .append_entry(pi_session::SessionEntry::message(
                            "assistant",
                            serde_json::json!([{"type":"text","text": ""}]),
                        ))
                        .map_err(internal)?;
                    live.phase = SessionPhase::Idle;
                    live.queued_steer.clear();
                }
                Ok(CommandResult::Abort {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::SetModel { session_id, model } => {
                self.ensure_live(&session_id)?;
                {
                    let live = self
                        .live
                        .get_mut(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase != SessionPhase::Idle {
                        return Err(busy("Session is busy"));
                    }
                    live.model = model;
                }
                Ok(CommandResult::SetModel {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::SetThinking {
                session_id,
                thinking_level,
            } => {
                self.ensure_live(&session_id)?;
                {
                    let live = self
                        .live
                        .get_mut(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase != SessionPhase::Idle {
                        return Err(busy("Session is busy"));
                    }
                    live.thinking_level = thinking_level;
                }
                Ok(CommandResult::SetThinking {
                    session: self.live_snapshot(&session_id)?,
                })
            }
        }
    }

    fn ensure_live(&mut self, session_id: &str) -> Result<(), ProtocolError> {
        if self.live.contains_key(session_id) {
            return Ok(());
        }
        let session = open_named(&self.sessions_dir, session_id)?;
        self.live.insert(
            session_id.to_string(),
            LiveSession {
                session,
                phase: SessionPhase::Idle,
                model: pi_protocol::default_model_ref(),
                thinking_level: ThinkingLevel::Off,
                queued_steer: Vec::new(),
                attached: self.attachments.contains_key(session_id),
            },
        );
        Ok(())
    }

    fn live_snapshot(&self, session_id: &str) -> Result<SessionSnapshot, ProtocolError> {
        let live = self
            .live
            .get(session_id)
            .ok_or_else(|| not_found(session_id))?;
        Ok(snapshot_from_live(live))
    }
}

fn open_named(dir: &std::path::Path, session_id: &str) -> Result<JsonlSession, ProtocolError> {
    let summary = pi_session::resolve_session_ref(dir, None, session_id)
        .map_err(|_| not_found(session_id))?;
    JsonlSession::open(&summary.path).map_err(|err| ProtocolError {
        code: ProtocolErrorCode::NotFound,
        message: err.to_string(),
        details: None,
    })
}

fn snapshot_from_live(live: &LiveSession) -> SessionSnapshot {
    SessionSnapshot {
        id: live.session.header.id.clone(),
        name: live.session.display_name(),
        cwd: live.session.header.cwd.clone(),
        created_at: live.session.header.created_at,
        updated_at: live.session.header.created_at,
        phase: live.phase,
        model: live.model.clone(),
        thinking_level: live.thinking_level,
        attached: live.attached,
        locked: live.phase != SessionPhase::Idle,
        revision: live.session.entries.len() as u64,
        transcript: transcript_from_jsonl(&live.session, &live.model),
        queued_steer: live.queued_steer.clone(),
        queued_steer_count: live.queued_steer.len() as u64,
    }
}

fn transcript_from_jsonl(session: &JsonlSession, model: &ModelRef) -> Vec<TranscriptItem> {
    session
        .entries
        .iter()
        .filter_map(|entry| {
            if entry.entry_type != "message" {
                return None;
            }
            let message = entry.message.as_ref()?;
            let role = message.get("role").and_then(Value::as_str)?;
            let content = message.get("content")?;
            match role {
                "user" => Some(TranscriptItem::User {
                    id: entry.id.clone(),
                    content: text_or_images(content),
                    timestamp: entry.timestamp,
                }),
                "assistant" => Some(TranscriptItem::Assistant {
                    id: entry.id.clone(),
                    content: assistant_content(content),
                    model: model.clone(),
                    response_model: None,
                    usage: None,
                    timestamp: entry.timestamp,
                    status: "complete".into(),
                    stop_reason: Some("stop".into()),
                    error_message: None,
                }),
                _ => None,
            }
        })
        .collect()
}

fn text_or_images(content: &Value) -> Vec<TextOrImage> {
    if let Some(text) = content.as_str() {
        return vec![TextOrImage::Text { text: text.into() }];
    }
    let Some(items) = content.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                Some(TextOrImage::Text { text: text.into() })
            } else if let (Some(data), Some(mime)) = (
                item.get("data").and_then(Value::as_str),
                item.get("mimeType")
                    .or_else(|| item.get("mime_type"))
                    .and_then(Value::as_str),
            ) {
                Some(TextOrImage::Image {
                    data: data.into(),
                    mime_type: mime.into(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn assistant_content(content: &Value) -> Vec<AssistantContent> {
    if let Some(text) = content.as_str() {
        return vec![AssistantContent::Text { text: text.into() }];
    }
    let Some(items) = content.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                Some(AssistantContent::Text { text: text.into() })
            } else if let Some(thinking) = item.get("thinking").and_then(Value::as_str) {
                Some(AssistantContent::Thinking {
                    thinking: thinking.into(),
                    redacted: item.get("redacted").and_then(Value::as_bool),
                })
            } else {
                None
            }
        })
        .collect()
}

fn builtin_models() -> Vec<ModelMetadata> {
    pi_ai::load_builtin_models()
        .into_iter()
        .map(|model| ModelMetadata {
            provider: model.provider,
            id: model.id,
            name: model.name,
            api: model.api,
            reasoning: model.reasoning,
            input: model.input,
            context_window: model.context_window,
            max_tokens: model.max_tokens,
            cost: ModelCost {
                input: model.cost.input,
                output: model.cost.output,
                cache_read: model.cost.cache_read,
                cache_write: model.cost.cache_write,
            },
            supported_thinking_levels: if model.reasoning {
                ThinkingLevel::all().to_vec()
            } else {
                vec![ThinkingLevel::Off]
            },
            authenticated: false,
        })
        .collect()
}

fn busy(message: &str) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::Busy,
        message: message.into(),
        details: None,
    }
}

fn not_found(session_id: &str) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::NotFound,
        message: format!("Session not found: {session_id}"),
        details: None,
    }
}

fn internal(err: impl ToString) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::InternalError,
        message: err.to_string(),
        details: None,
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
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn create_session(server: &mut PiServer) -> SessionSnapshot {
        match memory_roundtrip(
            server,
            ClientMessage::Request {
                id: "req-1".into(),
                request: Command::Create {
                    cwd: Some("/tmp/work".into()),
                    name: Some("demo".into()),
                    model: None,
                    thinking_level: None,
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Create { session }),
                ..
            } => session,
            other => panic!("expected create: {other:?}"),
        }
    }

    #[test]
    fn hello_and_create_over_memory() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let hello = memory_roundtrip(&mut server, ClientMessage::Hello { version: 1 });
        match hello {
            ServerMessage::Hello {
                version: 1,
                snapshot,
                ..
            } => {
                assert!(!snapshot.models.is_empty());
            }
            _ => panic!("expected hello"),
        }
        let created = create_session(&mut server);
        assert_eq!(created.phase, SessionPhase::Idle);
        assert!(created.transcript.is_empty());
        let _ = encode_client_message(&ClientMessage::Hello { version: 1 }, None).unwrap();
    }

    #[test]
    fn prompt_reply_fills_transcript_and_busy_rejects_second() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        std::env::set_var("PI_SERVER_PROMPT_REPLY", "hello-back");
        let prompted = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "hi".into(),
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Prompt { session }),
                ..
            } => session,
            other => panic!("expected prompt: {other:?}"),
        };
        std::env::remove_var("PI_SERVER_PROMPT_REPLY");
        assert_eq!(prompted.phase, SessionPhase::Idle);
        assert!(prompted.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::User { content, .. } if content.iter().any(|part| matches!(part, TextOrImage::Text { text } if text == "hi"))
        )));
        assert!(prompted.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Assistant { content, .. } if content.iter().any(|part| matches!(part, AssistantContent::Text { text } if text == "hello-back"))
        )));
    }

    #[test]
    fn keep_turn_allows_steer_and_abort() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        std::env::remove_var("PI_SERVER_PROMPT_REPLY");
        std::env::set_var("PI_SERVER_KEEP_TURN", "1");
        let prompted = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "hi".into(),
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Prompt { session }),
                ..
            } => session,
            other => panic!("expected prompt: {other:?}"),
        };
        assert_eq!(prompted.phase, SessionPhase::Turn);
        let busy_again = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p2".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "again".into(),
                },
            },
        );
        match busy_again {
            ServerMessage::Response {
                ok: false,
                error: Some(error),
                ..
            } => {
                assert_eq!(error.code, ProtocolErrorCode::Busy);
                assert_eq!(error.message, "A prompt is already running");
            }
            other => panic!("expected busy: {other:?}"),
        }
        let steered = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "s1".into(),
                request: Command::Steer {
                    session_id: created.id.clone(),
                    text: "more".into(),
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Steer { session }),
                ..
            } => session,
            other => panic!("expected steer: {other:?}"),
        };
        assert_eq!(steered.queued_steer_count, 1);
        let aborted = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "a1".into(),
                request: Command::Abort {
                    session_id: created.id.clone(),
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Abort { session }),
                ..
            } => session,
            other => panic!("expected abort: {other:?}"),
        };
        std::env::remove_var("PI_SERVER_KEEP_TURN");
        assert_eq!(aborted.phase, SessionPhase::Idle);
        assert_eq!(aborted.queued_steer_count, 0);
    }
}
