//! Protocol server matching `@earendil-works/pi-server`.

#[cfg(unix)]
mod unix;

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use davinci_agent::{Agent, AgentEvent, PromptProfile};
use davinci_ai::{content_text, get_supported_thinking_levels};
#[cfg(test)]
use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
use davinci_protocol::{
    encode_server_message, AssistantContent, ClientMessage, ClientMessageDecoder, Command,
    CommandResult, ModelCost, ModelMetadata, ModelRef, ProtocolError, ProtocolErrorCode,
    ServerEvent, ServerMessage, ServerSnapshot, SessionPhase, SessionSnapshot, TextOrImage,
    ThinkingLevel, TranscriptItem, TranscriptProgress, PROTOCOL_VERSION,
};
use davinci_session::{discover_session_headers, JsonlSession};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[cfg(unix)]
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
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
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
                    assistant_message_event:
                        davinci_ai::AssistantMessageEvent::TextDelta { delta, .. },
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
        let sessions = discover_session_headers(&self.sessions_dir, None)
            .unwrap_or_default()
            .into_iter()
            .map(|s| davinci_protocol::SessionMetadata {
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
                let mut session = JsonlSession::create(&self.sessions_dir, &cwd, name.as_deref())
                    .map_err(internal)?;
                let model = model.unwrap_or_else(davinci_protocol::default_model_ref);
                let thinking_level = thinking_level.unwrap_or(ThinkingLevel::Off);
                persist_model(&mut session, &model)?;
                persist_thinking(&mut session, thinking_level)?;
                let id = session.header.id.clone();
                let live = LiveSession {
                    agent: new_runtime_agent(&cwd),
                    session,
                    phase: SessionPhase::Idle,
                    model,
                    thinking_level,
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
                        .append_entry(davinci_session::SessionEntry::message(
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
                    persist_model(&mut live.session, &model)?;
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
                    persist_thinking(&mut live.session, thinking_level)?;
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
        let (model, thinking_level) = session_preferences(&session);
        let mut agent = new_runtime_agent(&session.header.cwd);
        hydrate_agent_messages(&mut agent, &session);
        self.live.insert(
            session_id.to_string(),
            LiveSession {
                agent,
                session,
                phase: SessionPhase::Idle,
                model,
                thinking_level,
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
            live.agent
                .load_from_session(live.session.clone())
                .map_err(internal)?;
            live.agent.provider = live.model.provider.clone();
            live.agent.model_id = live.model.id.clone();
            live.agent.thinking_level = live.thinking_level;
            live.agent.prompt(text);
            live.session = live.agent.session.as_ref().expect("bound session").clone();
            live.agent.ensure_session_persistence().map_err(internal)?;
            live.phase = SessionPhase::Turn;
            live.queued_steer.clear();
            let keep_turn = cfg!(test) && std::env::var("PI_SERVER_KEEP_TURN").is_ok();
            if !keep_turn {
                let result = live.agent.run_loop(complete_server_prompt);
                live.session = live.agent.session.as_ref().expect("bound session").clone();
                live.phase = SessionPhase::Idle;
                loop_events = result.map_err(internal)?;
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

fn complete_server_prompt(agent: &Agent) -> Result<davinci_agent::CompleteOutput, String> {
    #[cfg(test)]
    if let Ok(reply) = std::env::var("PI_SERVER_PROMPT_REPLY") {
        return Ok(AssistantMessage {
            extra: Default::default(),
            id: davinci_agent::new_message_id(),
            role: "assistant".into(),
            content: vec![ContentBlock::Text { text: reply }],
            model: format!("{}/{}", agent.provider, agent.model_id),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        }
        .into());
    }
    let auth_path = davinci_ai::try_default_auth_path().map_err(|error| error.to_string())?;
    let agent_dir = auth_path
        .parent()
        .ok_or("credential directory unavailable")?;
    let config = davinci_ai::ModelConfig::load(&davinci_ai::models_json_path(agent_dir));
    let models = config.apply(&davinci_ai::load_builtin_models())?;
    let model =
        davinci_ai::find_model(&models, &agent.provider, &agent.model_id).ok_or_else(|| {
            format!(
                "No model available for {}/{}",
                agent.provider, agent.model_id
            )
        })?;
    let storage = davinci_ai::AuthStorage::open(&auth_path).map_err(|error| error.to_string())?;
    let env = std::env::vars().collect();
    let mut auth = davinci_ai::resolve_provider_auth(&agent.provider, &storage, &env, true)
        .unwrap_or(davinci_ai::ResolvedAuth {
            api_key: None,
            headers: Default::default(),
            source: "none".into(),
        });
    if agent.provider == "openai-codex" {
        if !auth.source.eq_ignore_ascii_case("oauth") {
            return Err(
                "ChatGPT subscription authentication unavailable; log in with /login openai-codex"
                    .into(),
            );
        }
    } else {
        davinci_ai::apply_config_auth(&mut auth, &config, &agent.provider, Some(model), &env);
        if auth.api_key.is_none() && auth.headers.is_empty() && auth.source == "none" {
            return Err(format!("No credentials available for {}", agent.provider));
        }
    }
    let messages = agent.messages_for_provider();
    let tools = agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| davinci_ai::ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: None,
        })
        .collect::<Vec<_>>();
    let system = agent.provider_system_prompt();
    let envelope = davinci_ai::live_complete_streaming_with_sink_envelope(
        model,
        &messages,
        &auth,
        Some(&system),
        &tools,
        &davinci_ai::StreamOptions {
            service_tier: Some(agent.service_tier),
            thinking_level: Some(agent.request_thinking_level()),
            thinking_budgets: agent.thinking_budgets.clone(),
            timeout_ms: agent.provider_timeout_ms,
            max_retries: agent.provider_max_retries,
            max_retry_delay_ms: Some(agent.provider_max_retry_delay_ms),
            max_tokens: agent.context_vm_provider_output_limit(),
            transport: agent.transport.clone(),
            session_id: agent
                .session
                .as_ref()
                .map(|session| session.header.id.clone()),
            native_responses_resume: agent.native_responses_resume_record_for(&messages),
            install_telemetry: Some(agent.install_telemetry),
            abort_signal: agent.abort_signal.clone(),
            output_schema: agent.output_schema.clone(),
            ..Default::default()
        },
        &mut |_| {},
    )?;
    let native_responses_resume = envelope.native_responses.map(|turn| {
        let mut projection = messages;
        projection.push(davinci_ai::assistant_to_chat(&envelope.message));
        davinci_ai::NativeResponsesResumeRecord {
            turn,
            resume_provider_message_count: projection.len(),
            resume_provider_messages_fingerprint: davinci_ai::provider_messages_fingerprint(
                &projection,
            ),
        }
    });
    Ok(davinci_agent::CompleteOutput {
        message: envelope.message,
        stream_events: Some(envelope.stream_events),
        native_responses_resume,
        streamed_live: false,
    })
}

fn open_named(dir: &std::path::Path, session_id: &str) -> Result<JsonlSession, ProtocolError> {
    let summary = davinci_session::resolve_session_ref(dir, None, session_id)
        .map_err(|_| not_found(session_id))?;
    JsonlSession::open(&summary.path).map_err(|err| ProtocolError {
        code: ProtocolErrorCode::NotFound,
        message: err.to_string(),
        details: None,
    })
}

fn persist_model(session: &mut JsonlSession, model: &ModelRef) -> Result<(), ProtocolError> {
    let mut entry = davinci_session::SessionEntry::message("", Value::Null);
    entry.entry_type = "model_change".into();
    entry.message = None;
    entry
        .extra
        .insert("provider".into(), Value::String(model.provider.clone()));
    entry
        .extra
        .insert("modelId".into(), Value::String(model.id.clone()));
    session.append_entry(entry).map_err(internal)
}

fn persist_thinking(session: &mut JsonlSession, level: ThinkingLevel) -> Result<(), ProtocolError> {
    let mut entry = davinci_session::SessionEntry::message("", Value::Null);
    entry.entry_type = "thinking_level_change".into();
    entry.message = None;
    entry
        .extra
        .insert("thinkingLevel".into(), Value::String(level.as_str().into()));
    session.append_entry(entry).map_err(internal)
}

fn session_preferences(session: &JsonlSession) -> (ModelRef, ThinkingLevel) {
    let mut model = davinci_protocol::default_model_ref();
    let mut thinking = ThinkingLevel::Off;
    for entry in davinci_session::branch_entries(&session.entries, session.leaf_id.as_deref()) {
        if entry.entry_type == "model_change" {
            if let (Some(provider), Some(id)) = (
                entry.extra.get("provider").and_then(Value::as_str),
                entry.extra.get("modelId").and_then(Value::as_str),
            ) {
                model = ModelRef {
                    provider: provider.into(),
                    id: id.into(),
                };
            }
        } else if entry.entry_type == "thinking_level_change" {
            if let Some(level) = entry
                .extra
                .get("thinkingLevel")
                .and_then(Value::as_str)
                .and_then(ThinkingLevel::parse)
            {
                thinking = level;
            }
        }
    }
    (model, thinking)
}

fn snapshot_from_live(live: &LiveSession) -> SessionSnapshot {
    SessionSnapshot {
        id: live.session.header.id.clone(),
        name: live.session.display_name(),
        cwd: live.session.header.cwd.clone(),
        created_at: live.session.header.created_at,
        updated_at: live
            .session
            .entries
            .iter()
            .map(|entry| entry.timestamp)
            .chain(live.session.records.iter().map(|record| record.timestamp))
            .max()
            .unwrap_or(live.session.header.created_at)
            .max(live.session.header.created_at),
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
    davinci_session::branch_entries(&session.entries, session.leaf_id.as_deref())
        .into_iter()
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
                    model: historical_model(message, model),
                    response_model: message
                        .get("responseModel")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    usage: message
                        .get("usage")
                        .and_then(|value| serde_json::from_value(value.clone()).ok()),
                    timestamp: entry.timestamp,
                    status: message
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| {
                            match message.get("stopReason").and_then(Value::as_str) {
                                Some("error") => "error",
                                Some("aborted") => "aborted",
                                _ => "complete",
                            }
                        })
                        .into(),
                    stop_reason: message
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    error_message: message
                        .get("errorMessage")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                }),
                _ => None,
            }
        })
        .collect()
}

fn historical_model(message: &Value, fallback: &ModelRef) -> ModelRef {
    let Some(recorded) = message.get("model").and_then(Value::as_str) else {
        return fallback.clone();
    };
    let (provider, id) = if let Some(provider) = message.get("provider").and_then(Value::as_str) {
        (
            provider,
            recorded
                .strip_prefix(&format!("{provider}/"))
                .unwrap_or(recorded),
        )
    } else {
        recorded
            .split_once('/')
            .unwrap_or((&fallback.provider, recorded))
    };
    ModelRef {
        provider: provider.into(),
        id: id.into(),
    }
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
    davinci_ai::load_builtin_models()
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
    agent.messages = davinci_session::branch_entries(&session.entries, session.leaf_id.as_deref())
        .into_iter()
        .filter_map(|entry| {
            if entry.entry_type != "message" {
                return None;
            }
            let mut message = entry.message.clone()?;
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                message["content"] = serde_json::json!([{"type":"text","text":text}]);
            }
            serde_json::from_value(message).ok()
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
    let mut greeted = false;
    let mut connection_id = None;
    let result = (|| {
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
                let response = match message {
                    ClientMessage::Request { id, .. } if !greeted => ServerMessage::Response {
                        id,
                        ok: false,
                        result: None,
                        error: Some(ProtocolError {
                            code: ProtocolErrorCode::InvalidRequest,
                            message: "Hello must be the first protocol message".into(),
                            details: None,
                        }),
                    },
                    message => {
                        let response = server.handle(message);
                        if let ServerMessage::Hello {
                            connection_id: id, ..
                        } = &response
                        {
                            greeted = true;
                            connection_id = Some(id.clone());
                        }
                        response
                    }
                };
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
        decoder
            .end()
            .map_err(|err| ServerError::Protocol(err.to_string()))?;
        Ok(())
    })();
    if let Some(connection_id) = connection_id {
        server.disconnect(&connection_id);
    }
    result
}

pub fn memory_roundtrip(server: &mut PiServer, message: ClientMessage) -> ServerMessage {
    server.handle(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::encode_client_message;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn historical_models_keep_provider_qualified_model_ids() {
        let fallback = davinci_protocol::default_model_ref();
        for recorded in [
            "anthropic/claude-fixture",
            "openrouter/anthropic/claude-fixture",
        ] {
            let model = historical_model(
                &serde_json::json!({
                    "provider":"openrouter", "model":recorded
                }),
                &fallback,
            );
            assert_eq!(
                model,
                ModelRef {
                    provider: "openrouter".into(),
                    id: "anthropic/claude-fixture".into()
                }
            );
        }
    }

    #[test]
    fn regression_selected_settings_survive_session_reopen() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        let selected = ModelRef {
            provider: "openai-codex".into(),
            id: "gpt-6-luna".into(),
        };
        server
            .dispatch(Command::SetModel {
                session_id: created.id.clone(),
                model: selected.clone(),
            })
            .unwrap();
        server
            .dispatch(Command::SetThinking {
                session_id: created.id.clone(),
                thinking_level: ThinkingLevel::High,
            })
            .unwrap();
        server
            .dispatch(Command::Detach {
                session_id: created.id.clone(),
            })
            .unwrap();
        let reopened = server
            .dispatch(Command::Attach {
                session_id: created.id.clone(),
            })
            .unwrap();
        let CommandResult::Attach { session } = reopened else {
            panic!("expected attach")
        };
        assert_eq!(session.model, selected);
        assert_eq!(session.thinking_level, ThinkingLevel::High);
    }

    #[test]
    fn regression_server_transcript_and_hydration_follow_active_branch() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), ".", None).unwrap();
        session
            .append_entry(davinci_session::SessionEntry::message(
                "user",
                serde_json::json!("root"),
            ))
            .unwrap();
        let root = session.leaf_id.clone();
        session
            .append_entry(davinci_session::SessionEntry::message(
                "user",
                serde_json::json!("abandoned"),
            ))
            .unwrap();
        session.set_leaf(root);
        session
            .append_entry(davinci_session::SessionEntry::message(
                "user",
                serde_json::json!("active"),
            ))
            .unwrap();
        let transcript = transcript_from_jsonl(&session, &davinci_protocol::default_model_ref());
        assert_eq!(transcript.len(), 2);
        let mut agent = new_runtime_agent(".");
        hydrate_agent_messages(&mut agent, &session);
        assert_eq!(agent.messages.len(), 2);
        assert_eq!(content_text(&agent.messages[1].content), "active");
    }

    #[test]
    fn regression_server_hydration_preserves_structured_content_and_tool_metadata() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), ".", None).unwrap();
        for value in [
            serde_json::json!({"role":"user","content":[{"type":"image","data":"image-fixture","mimeType":"image/png"}]}),
            serde_json::json!({"role":"assistant","content":[
                {"type":"thinking","thinking":"thought","signature":"opaque","redacted":false},
                {"type":"toolCall","id":"call","name":"read","arguments":{"path":"file"}}],
                "model":"old/model","stopReason":"toolUse"}),
            serde_json::json!({"role":"toolResult","toolCallId":"call","toolName":"read","isError":false,
                "content":[{"type":"text","text":"output"}],"details":{"fixture":true}}),
        ] {
            let mut entry = davinci_session::SessionEntry::message("", Value::Null);
            entry.message = Some(value);
            session.append_entry(entry).unwrap();
        }
        let mut agent = new_runtime_agent(".");
        hydrate_agent_messages(&mut agent, &session);
        assert!(
            matches!(&agent.messages[0].content[0], davinci_ai::MessageContent::Image { data, .. } if data == "image-fixture")
        );
        assert!(
            matches!(&agent.messages[1].content[0], davinci_ai::MessageContent::Thinking { signature: Some(value), .. } if value == "opaque")
        );
        assert!(
            matches!(&agent.messages[1].content[1], davinci_ai::MessageContent::ToolCall { id, .. } if id == "call")
        );
        assert_eq!(agent.messages[2].tool_call_id.as_deref(), Some("call"));
        assert_eq!(agent.messages[2].tool_name.as_deref(), Some("read"));
        assert_eq!(agent.messages[2].is_error, Some(false));
    }

    #[test]
    fn regression_server_preserves_assistant_metadata_and_updated_timestamp() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        let live = server.live.get_mut(&created.id).unwrap();
        let usage = davinci_protocol::Usage::from_tokens(
            5,
            2,
            1,
            0,
            &ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        );
        let mut entry = davinci_session::SessionEntry::message("assistant", serde_json::json!([]));
        entry.message = Some(serde_json::json!({
            "role":"assistant","content":[],"provider":"openai-codex","model":"gpt-old",
            "responseModel":"resolved-old","usage":usage,"stopReason":"aborted","errorMessage":"cancelled"
        }));
        entry.timestamp = created.created_at + 100;
        live.session.append_entry(entry).unwrap();
        let snapshot = snapshot_from_live(live);
        assert!(snapshot.updated_at > snapshot.created_at);
        let TranscriptItem::Assistant {
            model,
            response_model,
            usage: actual_usage,
            status,
            stop_reason,
            error_message,
            ..
        } = &snapshot.transcript[0]
        else {
            panic!("assistant")
        };
        assert_eq!(model.provider, "openai-codex");
        assert_eq!(model.id, "gpt-old");
        assert_eq!(response_model.as_deref(), Some("resolved-old"));
        assert_eq!(actual_usage.as_ref(), Some(&usage));
        assert_eq!(status, "aborted");
        assert_eq!(stop_reason.as_deref(), Some("aborted"));
        assert_eq!(error_message.as_deref(), Some("cancelled"));
    }

    #[test]
    fn regression_transport_failures_release_connections_and_sessions() {
        struct BrokenStream {
            input: std::io::Cursor<Vec<u8>>,
            read_error: bool,
            write_error: bool,
            writes: usize,
        }
        impl Read for BrokenStream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.input.position() == self.input.get_ref().len() as u64 && self.read_error {
                    return Err(std::io::Error::other("fixture read failure"));
                }
                self.input.read(buf)
            }
        }
        impl Write for BrokenStream {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                if self.write_error && self.writes > 1 {
                    Err(std::io::Error::other("fixture write failure"))
                } else {
                    Ok(buf.len())
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for (read_error, write_error, incomplete) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let dir = tempdir().unwrap();
            let mut server = PiServer::new(dir.path().to_path_buf());
            let mut bytes = encode_client_message(
                &ClientMessage::Hello {
                    version: PROTOCOL_VERSION,
                },
                None,
            )
            .unwrap();
            bytes.extend(
                encode_client_message(
                    &ClientMessage::Request {
                        id: "create".into(),
                        request: Command::Create {
                            cwd: Some(dir.path().display().to_string()),
                            name: None,
                            model: None,
                            thinking_level: None,
                        },
                    },
                    None,
                )
                .unwrap(),
            );
            if incomplete {
                bytes.extend([0, 0]);
            }
            let result = serve_stream(
                &mut server,
                BrokenStream {
                    input: std::io::Cursor::new(bytes),
                    read_error,
                    write_error,
                    writes: 0,
                },
            );
            assert!(result.is_err());
            assert!(server.connections.is_empty());
            assert!(server.live.is_empty());
        }
    }

    #[test]
    fn regression_server_does_not_echo_when_selected_model_is_unavailable() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        server
            .dispatch(Command::SetModel {
                session_id: created.id.clone(),
                model: ModelRef {
                    provider: "unavailable-fixture".into(),
                    id: "unavailable-fixture".into(),
                },
            })
            .unwrap();
        let result = server.dispatch(Command::Prompt {
            session_id: created.id.clone(),
            text: "hello".into(),
        });
        assert!(result.is_err());
        let live = server.live.get(&created.id).unwrap();
        assert_eq!(live.phase, SessionPhase::Idle);
        assert!(!snapshot_from_live(live)
            .transcript
            .iter()
            .any(|item| matches!(
                item, TranscriptItem::Assistant { content, .. } if content.iter().any(|part|
                    matches!(part, AssistantContent::Text { text } if text == "reply:hello"))
            )));
    }

    fn create_session(server: &mut PiServer) -> SessionSnapshot {
        let cwd = server.sessions_dir.join("workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        match memory_roundtrip(
            server,
            ClientMessage::Request {
                id: "req-1".into(),
                request: Command::Create {
                    cwd: Some(cwd.display().to_string()),
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
    fn stream_rejects_request_before_hello() {
        let dir = tempdir().unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let request = ClientMessage::Request {
            id: "early".into(),
            request: Command::List,
        };
        let input = encode_client_message(&request, None).unwrap();
        let stream = std::io::Cursor::new(input);
        let mut output = Vec::new();
        struct Duplex<'a> {
            input: std::io::Cursor<Vec<u8>>,
            output: &'a mut Vec<u8>,
        }
        impl Read for Duplex<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.input.read(buf)
            }
        }
        impl Write for Duplex<'_> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.output.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let duplex = Duplex {
            input: stream,
            output: &mut output,
        };
        serve_stream(&mut server, duplex).unwrap();
        let mut decoder = davinci_protocol::create_server_message_decoder(None).unwrap();
        let decoded = decoder.push(&output).unwrap();
        decoder.end().unwrap();
        match &decoded[0] {
            ServerMessage::Response {
                ok: false,
                error: Some(error),
                ..
            } => {
                assert_eq!(error.code, ProtocolErrorCode::InvalidRequest);
            }
            other => panic!("expected pre-hello rejection: {other:?}"),
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
    fn prompt_runs_selected_provider_and_reasoning_through_localhost() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_SERVER_PROMPT_REPLY");
        std::env::remove_var("PI_SERVER_KEEP_TURN");
        let dir = tempdir().unwrap();
        struct RestoreDirs(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for RestoreDirs {
            fn drop(&mut self) {
                for (name, value) in [
                    ("DAVINCI_CODING_AGENT_DIR", &self.0),
                    ("PI_CODING_AGENT_DIR", &self.1),
                ] {
                    if let Some(value) = value {
                        std::env::set_var(name, value);
                    } else {
                        std::env::remove_var(name);
                    }
                }
            }
        }
        let _restore = RestoreDirs(
            std::env::var_os("DAVINCI_CODING_AGENT_DIR"),
            std::env::var_os("PI_CODING_AGENT_DIR"),
        );
        std::env::set_var("DAVINCI_CODING_AGENT_DIR", dir.path());
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let provider = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "provider was never called"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // Windows inherits the listener's nonblocking mode on accept.
            // Request fragments may arrive after accept even on localhost.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 8192];
            let body = loop {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(start) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..start]);
                    assert!(headers.starts_with("POST /v1/responses "));
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= start + 4 + length {
                        break serde_json::from_slice::<Value>(
                            &request[start + 4..start + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            let response = serde_json::json!({
                "id":"fixture-response","model":"fixture-model","status":"completed",
                "output":[{"id":"message-fixture","type":"message","role":"assistant","status":"completed",
                    "content":[{"type":"output_text","text":"provider answer"}]}],
                "usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}
            }).to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
            body
        });
        std::fs::write(dir.path().join("models.json"), serde_json::json!({
            "providers":{"fixture-provider":{
                "api":"openai-responses","baseUrl":format!("http://{address}/v1"),
                "apiKey":"disposable-fixture",
                "models":[{"id":"fixture-model","name":"Fixture","reasoning":true,"input":["text"],
                    "contextWindow":16384,"maxTokens":2048,
                    "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}]
            }}
        }).to_string()).unwrap();
        let mut server = PiServer::new(dir.path().to_path_buf());
        let created = create_session(&mut server);
        server
            .dispatch(Command::SetModel {
                session_id: created.id.clone(),
                model: ModelRef {
                    provider: "fixture-provider".into(),
                    id: "fixture-model".into(),
                },
            })
            .unwrap();
        server
            .dispatch(Command::SetThinking {
                session_id: created.id.clone(),
                thinking_level: ThinkingLevel::High,
            })
            .unwrap();
        let prompted = match memory_roundtrip(
            &mut server,
            ClientMessage::Request {
                id: "p1".into(),
                request: Command::Prompt {
                    session_id: created.id.clone(),
                    text: "Say provider answer".into(),
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
        let request = provider.join().unwrap();
        assert_eq!(request["model"], "fixture-model");
        assert_eq!(request["reasoning"]["effort"], "high");
        assert!(request["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "user"
                && message["content"][0]["text"] == "Say provider answer"));
        assert!(prompted.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Assistant { content, .. }
                if content.iter().any(|part| matches!(part, AssistantContent::Text { text } if text == "provider answer"))
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

    #[cfg(unix)]
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

    #[test]
    fn new_runtime_agent_activates_stable_prompt_profile() {
        let agent = new_runtime_agent("/test");
        assert!(agent.prompt_session.is_builtin());
        assert_eq!(agent.prompt_session.profile(), Some(PromptProfile::Stable));
        let manifest = agent.prompt_manifest.as_ref().expect("manifest");
        assert_eq!(manifest.profile, "stable");
        assert!(agent.system_prompt.contains("DaVinci"));
    }
}
