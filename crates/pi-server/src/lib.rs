//! Protocol server matching `@earendil-works/pi-server`.

mod unix;

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use pi_agent::{default_system_prompt, Agent, AgentEvent};
use pi_ai::{
    content_text, get_supported_thinking_levels, AssistantMessage, ContentBlock, StopReason,
};
use pi_protocol::{
    encode_server_message, AssistantContent, ClientMessage, ClientMessageDecoder, Command,
    CommandResult, ModelCost, ModelMetadata, ModelRef, ProtocolError, ProtocolErrorCode,
    ServerEvent, ServerMessage, ServerSnapshot, SessionPhase, SessionSnapshot, TextOrImage,
    ThinkingLevel, TranscriptItem, TranscriptProgress, PROTOCOL_VERSION,
};
use pi_session::{discover_sessions, JsonlSession};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub use unix::{
    bind_unix, bind_unix_with, max_unix_socket_path_bytes, owned_bind_path,
    resolve_unix_listener_options, validate_unix_socket_path, BoundUnixListener,
    UnixByteConnection, UnixListenerOptions, UnixListenerOptionsBuilder, DEFAULT_SOCKET_MODE,
};

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Protocol(String),
}

struct LiveSession {
    session: JsonlSession,
    agent: Agent,
    phase: SessionPhase,
    model: ModelRef,
    thinking_level: ThinkingLevel,
    queued_steer: Vec<TranscriptItem>,
    connections: HashSet<String>,
    operation_count: u32,
    terminal: bool,
    disposing: bool,
}

struct ConnectionState {
    session_ids: HashSet<String>,
}

fn new_runtime_agent(cwd: &str) -> Agent {
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = PathBuf::from(cwd);
    agent
}

pub struct PiServer {
    pub server_id: String,
    pub sessions_dir: PathBuf,
    pub handshake_timeout: Duration,
    pub revision: u64,
    connections: HashMap<String, ConnectionState>,
    current_connection: String,
    live: HashMap<String, LiveSession>,
    pending_events: Vec<ServerEvent>,
}

impl PiServer {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self {
            server_id: Uuid::new_v4().to_string(),
            sessions_dir,
            handshake_timeout: Duration::from_secs(5),
            revision: 0,
            connections: HashMap::new(),
            current_connection: "memory".into(),
            live: HashMap::new(),
            pending_events: Vec::new(),
        }
    }

    fn ensure_connection(&mut self, connection_id: &str) {
        self.connections
            .entry(connection_id.to_string())
            .or_insert_with(|| ConnectionState {
                session_ids: HashSet::new(),
            });
        self.current_connection = connection_id.to_string();
    }

    pub fn take_events(&mut self) -> Vec<ServerEvent> {
        std::mem::take(&mut self.pending_events)
    }

    fn emit_session_snapshot(&mut self, session_id: &str) {
        if let Ok(snapshot) = self.live_snapshot(session_id) {
            self.pending_events
                .push(ServerEvent::SessionSnapshot { snapshot });
        }
    }

    fn emit_server_snapshot(&mut self) {
        self.pending_events.push(ServerEvent::ServerSnapshot {
            snapshot: self.snapshot(),
        });
    }

    fn emit_runtime_progress(&mut self, session_id: &str, events: &[AgentEvent]) {
        for event in events {
            match event {
                AgentEvent::MessageStart { message } if message.role == "user" => {
                    self.pending_events.push(ServerEvent::SessionProgress {
                        session_id: session_id.into(),
                        progress: TranscriptProgress::ItemStarted {
                            item: TranscriptItem::User {
                                id: "runtime-user".into(),
                                content: vec![TextOrImage::Text {
                                    text: content_text(&message.content),
                                }],
                                timestamp: 0,
                            },
                        },
                    });
                }
                AgentEvent::MessageUpdate {
                    assistant_message_event: pi_ai::AssistantMessageEvent::TextDelta { delta, .. },
                    ..
                } if !delta.is_empty() => {
                    self.pending_events.push(ServerEvent::SessionProgress {
                        session_id: session_id.into(),
                        progress: TranscriptProgress::AssistantDelta {
                            message_id: "runtime-assistant".into(),
                            content_index: 0,
                            kind: "text".into(),
                            delta: delta.clone(),
                        },
                    });
                }
                _ => {}
            }
        }
    }

    fn emit_prompt_progress(&mut self, session_id: &str, snapshot: &SessionSnapshot) {
        if let Some(item) = snapshot
            .transcript
            .iter()
            .rev()
            .find(|item| matches!(item, TranscriptItem::User { .. }))
        {
            self.pending_events.push(ServerEvent::SessionProgress {
                session_id: session_id.into(),
                progress: TranscriptProgress::ItemStarted { item: item.clone() },
            });
            self.pending_events.push(ServerEvent::SessionProgress {
                session_id: session_id.into(),
                progress: TranscriptProgress::ItemFinished { item: item.clone() },
            });
        }
        if let Some(TranscriptItem::Assistant { id, content, .. }) = snapshot
            .transcript
            .iter()
            .rev()
            .find(|item| matches!(item, TranscriptItem::Assistant { .. }))
        {
            let delta = content
                .iter()
                .find_map(|part| match part {
                    AssistantContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if !delta.is_empty() {
                self.pending_events.push(ServerEvent::SessionProgress {
                    session_id: session_id.into(),
                    progress: TranscriptProgress::AssistantDelta {
                        message_id: id.clone(),
                        content_index: 0,
                        kind: "text".into(),
                        delta,
                    },
                });
            }
        }
    }

    fn queue_result_events(&mut self, result: &CommandResult) {
        let session_id = match result {
            CommandResult::List { .. } => return,
            CommandResult::Detach { session_id } => session_id.clone(),
            CommandResult::Create { session }
            | CommandResult::Attach { session }
            | CommandResult::Prompt { session }
            | CommandResult::Steer { session }
            | CommandResult::Abort { session }
            | CommandResult::SetModel { session }
            | CommandResult::SetThinking { session } => session.id.clone(),
        };
        self.emit_session_snapshot(&session_id);
        if matches!(
            result,
            CommandResult::Create { .. } | CommandResult::Detach { .. }
        ) {
            self.emit_server_snapshot();
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
                let connection_id = Uuid::new_v4().to_string();
                self.ensure_connection(&connection_id);
                ServerMessage::Hello {
                    version: PROTOCOL_VERSION,
                    connection_id,
                    snapshot: self.snapshot(),
                }
            }
            ClientMessage::Request { id, request } => match self.dispatch(request) {
                Ok(result) => {
                    self.queue_result_events(&result);
                    ServerMessage::Response {
                        id,
                        ok: true,
                        result: Some(result),
                        error: None,
                    }
                }
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
                    agent: new_runtime_agent(&cwd),
                    session,
                    phase: SessionPhase::Idle,
                    model: model.unwrap_or_else(pi_protocol::default_model_ref),
                    thinking_level: thinking_level.unwrap_or(ThinkingLevel::Off),
                    queued_steer: Vec::new(),
                    connections: HashSet::new(),
                    operation_count: 0,
                    terminal: false,
                    disposing: false,
                };
                self.live.insert(id.clone(), live);
                self.attach_connection(&id)?;
                self.revision += 1;
                Ok(CommandResult::Create {
                    session: self.live_snapshot(&id)?,
                })
            }
            Command::Attach { session_id } => {
                self.acquire(&session_id)?;
                self.attach_connection(&session_id)?;
                Ok(CommandResult::Attach {
                    session: self.live_snapshot(&session_id)?,
                })
            }
            Command::Detach { session_id } => {
                self.detach_connection(&session_id);
                Ok(CommandResult::Detach { session_id })
            }
            Command::Prompt { session_id, text } => {
                self.require_attached(&session_id)?;
                {
                    let live = self
                        .live
                        .get(&session_id)
                        .ok_or_else(|| not_found(&session_id))?;
                    if live.phase != SessionPhase::Idle {
                        return Err(busy("A prompt is already running"));
                    }
                }
                self.begin_operation(&session_id);
                let result = self.run_prompt(&session_id, &text);
                self.end_operation(&session_id);
                result
            }
            Command::Steer { session_id, text } => {
                self.require_attached(&session_id)?;
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
                self.require_attached(&session_id)?;
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
                self.require_attached(&session_id)?;
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
                self.require_attached(&session_id)?;
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

    fn acquire(&mut self, session_id: &str) -> Result<(), ProtocolError> {
        if let Some(live) = self.live.get(session_id) {
            if live.terminal || live.disposing {
                return Err(session_locked(&format!(
                    "Session runtime is terminating: {session_id}"
                )));
            }
            return Ok(());
        }
        self.open_live(session_id)
    }

    fn open_live(&mut self, session_id: &str) -> Result<(), ProtocolError> {
        if self.live.contains_key(session_id) {
            return Ok(());
        }
        let session = open_named(&self.sessions_dir, session_id)?;
        let mut agent = new_runtime_agent(&session.header.cwd);
        hydrate_agent_messages(&mut agent, &session);
        self.live.insert(
            session_id.to_string(),
            LiveSession {
                agent,
                session,
                phase: SessionPhase::Idle,
                model: pi_protocol::default_model_ref(),
                thinking_level: ThinkingLevel::Off,
                queued_steer: Vec::new(),
                connections: HashSet::new(),
                operation_count: 0,
                terminal: false,
                disposing: false,
            },
        );
        Ok(())
    }

    fn attach_connection(&mut self, session_id: &str) -> Result<(), ProtocolError> {
        self.ensure_connection(&self.current_connection.clone());
        let connection_id = self.current_connection.clone();
        let live = self
            .live
            .get_mut(session_id)
            .ok_or_else(|| not_found(session_id))?;
        if live.terminal || live.disposing {
            return Err(session_locked(&format!(
                "Session runtime is terminating: {session_id}"
            )));
        }
        live.connections.insert(connection_id.clone());
        if let Some(connection) = self.connections.get_mut(&connection_id) {
            connection.session_ids.insert(session_id.to_string());
        }
        Ok(())
    }

    fn detach_connection(&mut self, session_id: &str) {
        let connection_id = self.current_connection.clone();
        if let Some(connection) = self.connections.get_mut(&connection_id) {
            connection.session_ids.remove(session_id);
        }
        if let Some(live) = self.live.get_mut(session_id) {
            live.connections.remove(&connection_id);
        }
        self.maybe_dispose(session_id);
    }

    fn require_attached(&mut self, session_id: &str) -> Result<(), ProtocolError> {
        self.ensure_connection(&self.current_connection.clone());
        let connection_id = self.current_connection.clone();
        let attached = self
            .connections
            .get(&connection_id)
            .is_some_and(|connection| connection.session_ids.contains(session_id));
        if !attached {
            return Err(invalid_request(&format!(
                "Connection is not attached to session {session_id}"
            )));
        }
        let live = self
            .live
            .get(session_id)
            .ok_or_else(|| not_live(session_id))?;
        if live.terminal || live.disposing {
            return Err(not_live(session_id));
        }
        Ok(())
    }

    fn maybe_dispose(&mut self, session_id: &str) {
        let should_dispose = self.live.get(session_id).is_some_and(|live| {
            !live.disposing
                && live.connections.is_empty()
                && live.operation_count == 0
                && (live.terminal || live.phase == SessionPhase::Idle)
        });
        if !should_dispose {
            return;
        }
        if let Some(live) = self.live.get_mut(session_id) {
            live.disposing = true;
        }
        self.live.remove(session_id);
    }

    fn live_snapshot(&self, session_id: &str) -> Result<SessionSnapshot, ProtocolError> {
        let live = self
            .live
            .get(session_id)
            .ok_or_else(|| not_found(session_id))?;
        let mut snapshot = snapshot_from_live(live);
        snapshot.attached = self
            .connections
            .get(&self.current_connection)
            .is_some_and(|connection| connection.session_ids.contains(session_id));
        Ok(snapshot)
    }

    pub fn disconnect(&mut self, connection_id: &str) {
        let sessions = self
            .connections
            .remove(connection_id)
            .map(|connection| connection.session_ids)
            .unwrap_or_default();
        for session_id in sessions {
            if let Some(live) = self.live.get_mut(&session_id) {
                live.connections.remove(connection_id);
            }
            self.maybe_dispose(&session_id);
        }
        if self.current_connection == connection_id {
            self.current_connection = "memory".into();
        }
    }

    pub fn mark_terminal(&mut self, session_id: &str) {
        if let Some(live) = self.live.get_mut(session_id) {
            live.terminal = true;
        }
    }

    fn run_prompt(&mut self, session_id: &str, text: &str) -> Result<CommandResult, ProtocolError> {
        let mut loop_events = Vec::new();
        {
            let live = self
                .live
                .get_mut(session_id)
                .ok_or_else(|| not_found(session_id))?;
            live.session
                .append_entry(pi_session::SessionEntry::message(
                    "user",
                    serde_json::json!([{"type":"text","text": text}]),
                ))
                .map_err(internal)?;
            live.agent.prompt(text);
            live.phase = SessionPhase::Turn;
            live.queued_steer.clear();
            if std::env::var("PI_SERVER_KEEP_TURN").is_err() {
                let user_text = text.to_string();
                loop_events = live
                    .agent
                    .run_loop(|_| {
                        let reply = std::env::var("PI_SERVER_PROMPT_REPLY")
                            .ok()
                            .map(|value| {
                                if value.is_empty() {
                                    format!("reply:{user_text}")
                                } else {
                                    value
                                }
                            })
                            .unwrap_or_else(|| format!("reply:{user_text}"));
                        Ok(AssistantMessage {
                            id: pi_agent::new_message_id(),
                            role: "assistant".into(),
                            content: vec![ContentBlock::Text { text: reply }],
                            model: "server".into(),
                            usage: None,
                            stop_reason: Some(StopReason::Stop),
                            error_message: None,
                        })
                    })
                    .map_err(internal)?;
                if let Some(reply) = live.agent.last_assistant_text() {
                    live.session
                        .append_entry(pi_session::SessionEntry::message(
                            "assistant",
                            serde_json::json!([{"type":"text","text": reply}]),
                        ))
                        .map_err(internal)?;
                }
                live.phase = SessionPhase::Idle;
            }
        }
        let session = self.live_snapshot(session_id)?;
        self.emit_runtime_progress(session_id, &loop_events);
        self.emit_prompt_progress(session_id, &session);
        Ok(CommandResult::Prompt { session })
    }

    fn begin_operation(&mut self, session_id: &str) {
        if let Some(live) = self.live.get_mut(session_id) {
            live.operation_count = live.operation_count.saturating_add(1);
        }
    }

    fn end_operation(&mut self, session_id: &str) {
        if let Some(live) = self.live.get_mut(session_id) {
            live.operation_count = live.operation_count.saturating_sub(1);
        }
        self.maybe_dispose(session_id);
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
        attached: !live.connections.is_empty(),
        locked: live.phase != SessionPhase::Idle || live.terminal,
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
            } else {
                item.get("thinking")
                    .and_then(Value::as_str)
                    .map(|thinking| AssistantContent::Thinking {
                        thinking: thinking.into(),
                        redacted: item.get("redacted").and_then(Value::as_bool),
                    })
            }
        })
        .collect()
}

fn builtin_models() -> Vec<ModelMetadata> {
    pi_ai::load_builtin_models()
        .into_iter()
        .map(|model| {
            let supported_thinking_levels = get_supported_thinking_levels(&model);
            ModelMetadata {
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
                supported_thinking_levels,
                authenticated: false,
            }
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

fn session_locked(message: &str) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::SessionLocked,
        message: message.into(),
        details: None,
    }
}

fn invalid_request(message: &str) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::InvalidRequest,
        message: message.into(),
        details: None,
    }
}

fn not_live(session_id: &str) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::NotFound,
        message: format!("Session is not live: {session_id}"),
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

fn hydrate_agent_messages(agent: &mut Agent, session: &JsonlSession) {
    agent.messages = session
        .entries
        .iter()
        .filter_map(|entry| {
            let message = entry.message.as_ref()?;
            let role = message.get("role").and_then(Value::as_str)?.to_string();
            let text = message
                .get("content")
                .map(|content| {
                    if let Some(text) = content.as_str() {
                        return text.to_string();
                    }
                    content
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            Some(pi_ai::ChatMessage::text(&role, text))
        })
        .collect();
}

/// Transport-level auth preamble sent before protocol bytes (TS listener auth).
pub fn encode_auth_preamble(token: &str) -> Vec<u8> {
    format!("AUTH {token}\n").into_bytes()
}

pub fn authorize_transport(expected: Option<&str>, preamble: &[u8]) -> Result<(), ServerError> {
    let Some(expected) = expected.filter(|token| !token.is_empty()) else {
        return Ok(());
    };
    let text = std::str::from_utf8(preamble).unwrap_or("");
    let line = text.lines().next().unwrap_or("").trim();
    let got = line.strip_prefix("AUTH ").map(str::trim);
    if got == Some(expected) {
        Ok(())
    } else {
        Err(ServerError::Protocol("Unauthorized".into()))
    }
}

fn read_auth_line<R: Read>(stream: &mut R) -> Result<Vec<u8>, ServerError> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream
            .read(&mut byte)
            .map_err(|err| ServerError::Io(err.to_string()))?;
        if n == 0 {
            break;
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' || buf.len() > 4096 {
            break;
        }
    }
    Ok(buf)
}

pub fn bind_tcp(addr: &str) -> Result<TcpListener, ServerError> {
    TcpListener::bind(addr).map_err(|err| ServerError::Io(err.to_string()))
}

pub fn serve_stream_with_auth<S: Read + Write>(
    server: &mut PiServer,
    mut stream: S,
    expected_token: Option<&str>,
) -> Result<(), ServerError> {
    if expected_token.is_some() {
        let preamble = read_auth_line(&mut stream)?;
        authorize_transport(expected_token, &preamble)?;
    }
    serve_stream(server, stream)
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
            let mut outgoing = vec![response];
            outgoing.extend(
                server
                    .take_events()
                    .into_iter()
                    .map(|event| ServerMessage::Event { event }),
            );
            for message in outgoing {
                let bytes = encode_server_message(&message, None)
                    .map_err(|err| ServerError::Protocol(err.to_string()))?;
                stream
                    .write_all(&bytes)
                    .map_err(|err| ServerError::Io(err.to_string()))?;
            }
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

    #[test]
    fn mutating_commands_emit_session_and_server_snapshots() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        let events = server.take_events();
        assert!(events.iter().any(|event| matches!(
            event,
            ServerEvent::SessionSnapshot { snapshot } if snapshot.id == created.id
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, ServerEvent::ServerSnapshot { .. })));
        let _ = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "a1".into(),
                request: Command::Attach {
                    session_id: created.id.clone(),
                },
            },
        );
        let attach_events = server.take_events();
        assert!(attach_events.iter().any(|event| matches!(
            event,
            ServerEvent::SessionSnapshot { snapshot } if snapshot.id == created.id && snapshot.attached
        )));
    }

    #[test]
    fn prompt_emits_session_progress() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        let _ = server.take_events();
        std::env::set_var("PI_SERVER_PROMPT_REPLY", "delta-text");
        let _ = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "hi".into(),
                },
            },
        );
        std::env::remove_var("PI_SERVER_PROMPT_REPLY");
        let events = server.take_events();
        assert!(events.iter().any(|event| matches!(
            event,
            ServerEvent::SessionProgress {
                session_id,
                progress: TranscriptProgress::ItemStarted { .. },
            } if session_id == &created.id
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ServerEvent::SessionProgress {
                progress: TranscriptProgress::AssistantDelta { kind, delta, .. },
                ..
            }             if kind == "text" && delta == "delta-text"
        )));
    }

    #[test]
    fn prompt_runs_agent_loop_without_fixture() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_SERVER_PROMPT_REPLY");
        std::env::remove_var("PI_SERVER_KEEP_TURN");
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        let prompted = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "hello-loop".into(),
                },
            },
        ) {
            ServerMessage::Response {
                result: Some(CommandResult::Prompt { session }),
                ..
            } => session,
            other => panic!("expected prompt: {other:?}"),
        };
        assert_eq!(prompted.phase, SessionPhase::Idle);
        assert!(prompted.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Assistant { content, .. }
                if content.iter().any(|part| matches!(part, AssistantContent::Text { text } if text == "reply:hello-loop"))
        )));
    }

    #[test]
    fn authorize_transport_requires_auth_preamble() {
        assert!(authorize_transport(None, b"").is_ok());
        assert!(authorize_transport(Some("secret"), &encode_auth_preamble("secret")).is_ok());
        assert_eq!(
            authorize_transport(Some("secret"), b"AUTH wrong\n")
                .unwrap_err()
                .to_string(),
            "Unauthorized"
        );
    }

    #[test]
    fn unix_listener_validates_path_and_binds_private_link() {
        assert_eq!(
            validate_unix_socket_path("", "PiServer Unix socket path")
                .unwrap_err()
                .to_string(),
            "PiServer Unix socket path must not be empty"
        );
        let too_long = "x".repeat(max_unix_socket_path_bytes() + 1);
        assert!(
            validate_unix_socket_path(&too_long, "PiServer Unix socket path")
                .unwrap_err()
                .to_string()
                .contains("is too long; maximum is")
        );
        let dir = tempdir().unwrap();
        let path = dir.path().join("pi.sock");
        let path_str = path.to_string_lossy().into_owned();
        let bound = bind_unix(&path_str).unwrap();
        assert!(path.exists());
        assert!(bound.owned_bind_path.exists());
        assert_eq!(bound.owned_bind_path, owned_bind_path(&path_str));
        let again = bind_unix(&path_str);
        match again {
            Err(err) => assert!(err.to_string().contains("Unix listener is already running")),
            Ok(_) => panic!("expected already-running Unix listener"),
        }
        drop(bound);
    }

    #[test]
    fn attach_sets_and_exclusive_acquire_match_ts() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        assert!(created.attached);
        let first = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "a1".into(),
                request: Command::Attach {
                    session_id: created.id.clone(),
                },
            },
        );
        match first {
            ServerMessage::Response {
                result: Some(CommandResult::Attach { session }),
                ..
            } => assert!(session.attached),
            other => panic!("expected attach: {other:?}"),
        }
        let _ = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "d1".into(),
                request: Command::Detach {
                    session_id: created.id.clone(),
                },
            },
        );
        assert!(!server.live.contains_key(&created.id));
        let prompt = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "hi".into(),
                },
            },
        );
        match prompt {
            ServerMessage::Response {
                ok: false,
                error: Some(error),
                ..
            } => {
                assert_eq!(error.code, ProtocolErrorCode::InvalidRequest);
                assert_eq!(
                    error.message,
                    format!("Connection is not attached to session {}", created.id)
                );
            }
            other => panic!("expected not attached: {other:?}"),
        }
        let _ = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "a2".into(),
                request: Command::Attach {
                    session_id: created.id.clone(),
                },
            },
        );
        server.mark_terminal(&created.id);
        let locked = memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "a3".into(),
                request: Command::Attach {
                    session_id: created.id.clone(),
                },
            },
        );
        match locked {
            ServerMessage::Response {
                ok: false,
                error: Some(error),
                ..
            } => {
                assert_eq!(error.code, ProtocolErrorCode::SessionLocked);
                assert_eq!(
                    error.message,
                    format!("Session runtime is terminating: {}", created.id)
                );
            }
            other => panic!("expected session_locked: {other:?}"),
        }
    }
}
