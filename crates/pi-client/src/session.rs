//! SessionHandle + reconnecting client matching TypeScript `session-handle.ts` / `client.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pi_protocol::{
    encode_client_message, ClientMessage, Command, CommandResult, FrameDecoderOptions, ModelRef,
    ProtocolError, ServerEvent, ServerMessage, ServerSnapshot, SessionSnapshot, ThinkingLevel,
    PROTOCOL_VERSION,
};

use crate::connection::{loopback_factory, Connection, ConnectionState, TransportFactory};
use crate::state::{ClientState, Unsubscribe};
use crate::ClientError;

type DispatchFn = Box<dyn FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>)>;

const DISCONNECTED: &str = "Pi client is disconnected";
const DISPOSED: &str = "Pi client is disposed";

pub type SessionLeaseMode = &'static str;

#[derive(Clone)]
pub struct SessionClient {
    inner: Rc<RefCell<SessionClientInner>>,
}

struct SessionClientInner {
    dispatch: DispatchFn,
    factory: Option<TransportFactory>,
    connection: Option<Connection>,
    last_response: Option<ServerMessage>,
    state: ClientState,
    connected: bool,
    disposed: bool,
    request_seq: u64,
    lease_counts: HashMap<String, u32>,
    exclusive: HashSet<String>,
    generations: HashMap<String, u64>,
}

impl SessionClient {
    pub fn new(
        dispatch: impl FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>) + 'static,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(SessionClientInner {
                dispatch: Box::new(dispatch),
                factory: None,
                connection: None,
                last_response: None,
                state: ClientState::new(),
                connected: false,
                disposed: false,
                request_seq: 0,
                lease_counts: HashMap::new(),
                exclusive: HashSet::new(),
                generations: HashMap::new(),
            })),
        }
    }

    /// TS `new PiClient({ transportFactory })` — framed hello/handshake over `ByteTransport`.
    pub fn with_factory(factory: TransportFactory) -> Result<Self, ClientError> {
        Ok(Self {
            inner: Rc::new(RefCell::new(SessionClientInner {
                dispatch: Box::new(|_| {
                    (
                        ServerMessage::HelloError {
                            error: ProtocolError {
                                code: pi_protocol::ProtocolErrorCode::InternalError,
                                message:
                                    "SessionClient dispatch is unused with a transport factory"
                                        .into(),
                                details: None,
                            },
                        },
                        Vec::new(),
                    )
                }),
                factory: Some(factory),
                connection: Some(Connection::new(None)?),
                last_response: None,
                state: ClientState::new(),
                connected: false,
                disposed: false,
                request_seq: 0,
                lease_counts: HashMap::new(),
                exclusive: HashSet::new(),
                generations: HashMap::new(),
            })),
        })
    }

    /// Convenience: wrap in-process dispatch in a framed loopback `ByteTransport`.
    pub fn with_loopback(
        dispatch: impl FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>) + 'static,
    ) -> Result<Self, ClientError> {
        Self::with_factory(loopback_factory(dispatch))
    }

    pub fn connection_state(&self) -> ConnectionState {
        let inner = self.inner.borrow();
        if let Some(connection) = &inner.connection {
            return connection.state();
        }
        if inner.connected && !inner.disposed {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    pub fn connected(&self) -> bool {
        let inner = self.inner.borrow();
        if inner.disposed {
            return false;
        }
        if let Some(connection) = &inner.connection {
            return connection.state() == ConnectionState::Connected;
        }
        inner.connected
    }

    pub fn disposed(&self) -> bool {
        self.inner.borrow().disposed
    }

    pub fn snapshot(&self) -> Option<ServerSnapshot> {
        self.inner.borrow().state.snapshot()
    }

    pub fn connect(&self) -> Result<ServerSnapshot, ClientError> {
        if self.inner.borrow().disposed {
            return Err(ClientError::Protocol(DISPOSED.into()));
        }
        if self.inner.borrow().factory.is_some() {
            return self.connect_framed();
        }
        let mut inner = self.inner.borrow_mut();
        if !inner.connected {
            inner.state.reset();
        }
        let (message, events) = (inner.dispatch)(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        });
        match message {
            ServerMessage::Hello { snapshot, .. } => {
                inner.state.apply_server_snapshot(snapshot.clone());
                for event in events {
                    inner.state.apply_event(&event);
                }
                inner.connected = true;
                Ok(snapshot)
            }
            ServerMessage::HelloError { error } => Err(protocol_error(&error)),
            other => Err(ClientError::Protocol(format!(
                "Expected hello, got {other:?}"
            ))),
        }
    }

    fn connect_framed(&self) -> Result<ServerSnapshot, ClientError> {
        let (connection, mut factory) = {
            let mut inner = self.inner.borrow_mut();
            if inner
                .connection
                .as_ref()
                .is_some_and(|connection| connection.state() == ConnectionState::Disconnected)
            {
                inner.state.reset();
            }
            let connection = inner
                .connection
                .clone()
                .ok_or_else(|| ClientError::Protocol("Pi client is disconnected".into()))?;
            let factory = inner
                .factory
                .take()
                .ok_or_else(|| ClientError::Protocol("Pi client is disconnected".into()))?;
            (connection, factory)
        };
        let client = self.clone();
        connection.on_handshake(move |snapshot| {
            client
                .inner
                .borrow_mut()
                .state
                .apply_server_snapshot(snapshot.clone());
            Ok(())
        });
        let client = self.clone();
        connection.on_message(move |message| match message {
            ServerMessage::Event { event } => {
                let mut inner = client.inner.borrow_mut();
                if let ServerEvent::SessionRemoved { session_id } = event {
                    invalidate_one(&mut inner, session_id);
                }
                inner.state.apply_event(event);
            }
            ServerMessage::Response { .. } => {
                client.inner.borrow_mut().last_response = Some(message.clone());
            }
            _ => {}
        });
        let result = connection.connect(&mut factory);
        self.inner.borrow_mut().factory = Some(factory);
        match result {
            Ok(snapshot) => {
                self.inner.borrow_mut().connected = true;
                Ok(snapshot)
            }
            Err(error) => {
                self.inner.borrow_mut().connected = false;
                Err(error)
            }
        }
    }

    pub fn reconnect(&self) -> Result<ServerSnapshot, ClientError> {
        let connection = {
            let mut inner = self.inner.borrow_mut();
            inner.connected = false;
            inner.state.reset();
            inner.state.clear_attachments();
            invalidate_all(&mut inner);
            inner.connection.clone()
        };
        if let Some(connection) = connection {
            connection.disconnect("Client disconnected");
        }
        self.connect()
    }

    pub fn disconnect(&self) {
        let connection = {
            let mut inner = self.inner.borrow_mut();
            inner.connected = false;
            inner.state.clear_attachments();
            invalidate_all(&mut inner);
            inner.connection.clone()
        };
        if let Some(connection) = connection {
            connection.disconnect("Client disconnected");
        }
    }

    pub fn dispose(&self) {
        let connection = {
            let mut inner = self.inner.borrow_mut();
            inner.disposed = true;
            inner.connected = false;
            inner.state.dispose();
            invalidate_all(&mut inner);
            inner.connection.clone()
        };
        if let Some(connection) = connection {
            connection.disconnect("Pi client is disposed");
        }
    }

    pub fn subscribe(&self, listener: impl Fn(&ServerSnapshot) + 'static) -> Unsubscribe {
        self.inner.borrow().state.subscribe(listener)
    }

    pub fn on_event(&self, listener: impl Fn(&ServerEvent) + 'static) -> Unsubscribe {
        self.inner.borrow().state.on_event(listener)
    }

    pub fn list_sessions(&self) -> Result<Vec<pi_protocol::SessionMetadata>, ClientError> {
        match self.request(Command::List)? {
            CommandResult::List { sessions } => Ok(sessions),
            other => Err(ClientError::Protocol(format!(
                "Unexpected list result: {other:?}"
            ))),
        }
    }

    pub fn create_session(
        &self,
        cwd: Option<String>,
        name: Option<String>,
    ) -> Result<SessionHandle, ClientError> {
        let result = self.request(Command::Create {
            cwd,
            name,
            model: None,
            thinking_level: None,
        })?;
        let CommandResult::Create { session } = result else {
            return Err(ClientError::Protocol("Expected create result".into()));
        };
        self.reserve_lease(&session.id, "exclusive")?;
        Ok(self.lease(session.id, "exclusive"))
    }

    pub fn acquire_session(
        &self,
        session_id: &str,
        mode: SessionLeaseMode,
    ) -> Result<SessionHandle, ClientError> {
        self.reserve_lease(session_id, mode)?;
        if !self.inner.borrow().state.is_session_attached(session_id) {
            if let Err(err) = self.request(Command::Attach {
                session_id: session_id.to_string(),
            }) {
                self.release_lease(session_id, mode);
                return Err(err);
            }
        }
        Ok(self.lease(session_id.to_string(), mode))
    }

    pub fn attach_session(&self, session_id: &str) -> Result<SessionHandle, ClientError> {
        self.acquire_session(session_id, "shared")
    }

    fn request(&self, command: Command) -> Result<CommandResult, ClientError> {
        if self.inner.borrow().disposed {
            return Err(ClientError::Protocol(DISPOSED.into()));
        }
        if !self.connected() {
            return Err(ClientError::Protocol(DISCONNECTED.into()));
        }
        if self.inner.borrow().factory.is_some() {
            return self.request_framed(command);
        }
        let mut inner = self.inner.borrow_mut();
        inner.request_seq += 1;
        let id = format!("request-{}", inner.request_seq);
        let expected = command.name().to_string();
        let (message, events) = (inner.dispatch)(ClientMessage::Request {
            id: id.clone(),
            request: command,
        });
        for event in events {
            if let ServerEvent::SessionRemoved { session_id } = &event {
                invalidate_one(&mut inner, session_id);
            }
            inner.state.apply_event(&event);
        }
        apply_response(&mut inner, expected, message)
    }

    fn request_framed(&self, command: Command) -> Result<CommandResult, ClientError> {
        let (expected, frame, connection) = {
            let mut inner = self.inner.borrow_mut();
            inner.request_seq += 1;
            let id = format!("request-{}", inner.request_seq);
            let expected = command.name().to_string();
            let max_frame_length = inner
                .connection
                .as_ref()
                .map(Connection::max_frame_length)
                .unwrap_or(pi_protocol::DEFAULT_MAX_FRAME_LENGTH);
            let frame = encode_client_message(
                &ClientMessage::Request {
                    id,
                    request: command,
                },
                Some(FrameDecoderOptions {
                    max_frame_length: Some(max_frame_length),
                }),
            )
            .map_err(|err| ClientError::Protocol(err.to_string()))?;
            inner.last_response = None;
            let connection = inner
                .connection
                .clone()
                .ok_or_else(|| ClientError::Protocol(DISCONNECTED.into()))?;
            (expected, frame, connection)
        };
        connection.send(&frame)?;
        let message = self
            .inner
            .borrow_mut()
            .last_response
            .take()
            .ok_or_else(|| ClientError::Protocol("Unexpected response: missing".into()))?;
        let mut inner = self.inner.borrow_mut();
        apply_response(&mut inner, expected, message)
    }

    fn reserve_lease(&self, session_id: &str, mode: SessionLeaseMode) -> Result<(), ClientError> {
        let mut inner = self.inner.borrow_mut();
        if inner.disposed {
            return Err(ClientError::Protocol(DISPOSED.into()));
        }
        let count = inner.lease_counts.get(session_id).copied().unwrap_or(0);
        if mode == "exclusive" && count > 0 {
            return Err(ClientError::Protocol(format!(
                "Session {session_id} already has an active lease"
            )));
        }
        if mode == "shared" && inner.exclusive.contains(session_id) {
            return Err(ClientError::Protocol(format!(
                "Session {session_id} has an exclusive lease"
            )));
        }
        inner.lease_counts.insert(session_id.to_string(), count + 1);
        if mode == "exclusive" {
            inner.exclusive.insert(session_id.to_string());
        }
        Ok(())
    }

    fn release_lease(&self, session_id: &str, mode: SessionLeaseMode) {
        let mut inner = self.inner.borrow_mut();
        let count = inner.lease_counts.get(session_id).copied().unwrap_or(0);
        if count <= 1 {
            inner.lease_counts.remove(session_id);
        } else {
            inner.lease_counts.insert(session_id.to_string(), count - 1);
        }
        if mode == "exclusive" {
            inner.exclusive.remove(session_id);
        }
    }

    fn lease(&self, session_id: String, mode: SessionLeaseMode) -> SessionHandle {
        let generation = {
            let inner = self.inner.borrow();
            inner.generations.get(&session_id).copied().unwrap_or(0)
        };
        SessionHandle {
            id: session_id,
            client: self.clone(),
            generation,
            mode,
        }
    }
}

pub struct SessionHandle {
    pub id: String,
    client: SessionClient,
    generation: u64,
    mode: SessionLeaseMode,
}

impl SessionHandle {
    pub fn attached(&self) -> bool {
        self.active()
    }

    pub fn active(&self) -> bool {
        let inner = self.client.inner.borrow();
        let current = inner.generations.get(&self.id).copied().unwrap_or(0);
        current == self.generation
            && inner.connected
            && !inner.disposed
            && inner.state.is_session_attached(&self.id)
    }

    pub fn snapshot(&self) -> Option<SessionSnapshot> {
        if !self.active() {
            return None;
        }
        self.client
            .inner
            .borrow()
            .state
            .get_session_snapshot(&self.id)
    }

    pub fn subscribe(&self, listener: impl Fn(&SessionSnapshot) + 'static) -> Unsubscribe {
        let id = self.id.clone();
        let client = self.client.clone();
        let generation = self.generation;
        self.client
            .inner
            .borrow()
            .state
            .subscribe_session(&self.id, move |snapshot| {
                let inner = client.inner.borrow();
                let current = inner.generations.get(&id).copied().unwrap_or(0);
                if current == generation && inner.state.is_session_attached(&id) {
                    drop(inner);
                    listener(snapshot);
                }
            })
    }

    pub fn on_event(&self, listener: impl Fn(&ServerEvent) + 'static) -> Unsubscribe {
        self.client
            .inner
            .borrow()
            .state
            .on_session_event(&self.id, listener)
    }

    pub fn prompt(&self, text: &str) -> Result<SessionSnapshot, ClientError> {
        self.session_command(Command::Prompt {
            session_id: self.id.clone(),
            text: text.into(),
        })
    }

    pub fn steer(&self, text: &str) -> Result<SessionSnapshot, ClientError> {
        self.session_command(Command::Steer {
            session_id: self.id.clone(),
            text: text.into(),
        })
    }

    pub fn abort(&self) -> Result<SessionSnapshot, ClientError> {
        self.session_command(Command::Abort {
            session_id: self.id.clone(),
        })
    }

    pub fn set_model(&self, model: ModelRef) -> Result<SessionSnapshot, ClientError> {
        self.session_command(Command::SetModel {
            session_id: self.id.clone(),
            model,
        })
    }

    pub fn set_thinking(
        &self,
        thinking_level: ThinkingLevel,
    ) -> Result<SessionSnapshot, ClientError> {
        self.session_command(Command::SetThinking {
            session_id: self.id.clone(),
            thinking_level,
        })
    }

    pub fn detach(&self) -> Result<(), ClientError> {
        self.release(false)
    }

    pub fn dispose(&self) -> Result<(), ClientError> {
        self.release(true)
    }

    fn session_command(&self, command: Command) -> Result<SessionSnapshot, ClientError> {
        self.assert_active()?;
        let result = self.client.request(command)?;
        match result {
            CommandResult::Prompt { session }
            | CommandResult::Steer { session }
            | CommandResult::Abort { session }
            | CommandResult::SetModel { session }
            | CommandResult::SetThinking { session }
            | CommandResult::Attach { session }
            | CommandResult::Create { session } => Ok(session),
            _ => Err(ClientError::Protocol("Expected session result".into())),
        }
    }

    fn assert_active(&self) -> Result<(), ClientError> {
        let (disposed, connected) = {
            let inner = self.client.inner.borrow();
            (inner.disposed, inner.connected)
        };
        if disposed {
            return Err(ClientError::Protocol(DISPOSED.into()));
        }
        if !connected {
            return Err(ClientError::Protocol(DISCONNECTED.into()));
        }
        if !self.active() {
            return Err(ClientError::Protocol(format!(
                "Session {} is not attached",
                self.id
            )));
        }
        Ok(())
    }

    fn release(&self, relinquish_on_failure: bool) -> Result<(), ClientError> {
        if !self.active() {
            return Ok(());
        }
        let count = self
            .client
            .inner
            .borrow()
            .lease_counts
            .get(&self.id)
            .copied()
            .unwrap_or(0);
        if count <= 1 {
            match self.client.request(Command::Detach {
                session_id: self.id.clone(),
            }) {
                Ok(_) => self.client.release_lease(&self.id, self.mode),
                Err(err) if relinquish_on_failure => {
                    self.client.release_lease(&self.id, self.mode);
                    return Err(err);
                }
                Err(err) => return Err(err),
            }
        } else {
            self.client.release_lease(&self.id, self.mode);
        }
        Ok(())
    }
}

fn invalidate_all(inner: &mut SessionClientInner) {
    let ids: Vec<String> = inner.lease_counts.keys().cloned().collect();
    for id in ids {
        invalidate_one(inner, &id);
    }
}

fn invalidate_one(inner: &mut SessionClientInner, session_id: &str) {
    inner.lease_counts.remove(session_id);
    inner.exclusive.remove(session_id);
    let next = inner.generations.get(session_id).copied().unwrap_or(0) + 1;
    inner.generations.insert(session_id.to_string(), next);
}

fn protocol_error(error: &ProtocolError) -> ClientError {
    ClientError::Protocol(error.message.clone())
}

fn apply_response(
    inner: &mut SessionClientInner,
    expected: String,
    message: ServerMessage,
) -> Result<CommandResult, ClientError> {
    match message {
        ServerMessage::Response {
            ok: true,
            result: Some(result),
            ..
        } => {
            if result.command_name() != expected {
                return Err(ClientError::Protocol(format!(
                    "Response command {} does not match {expected}",
                    result.command_name()
                )));
            }
            inner.state.apply_result(&result);
            Ok(result)
        }
        ServerMessage::Response {
            error: Some(error), ..
        } => Err(protocol_error(&error)),
        other => Err(ClientError::Protocol(format!(
            "Unexpected response: {other:?}"
        ))),
    }
}

trait CommandName {
    fn command_name(&self) -> &'static str;
}

impl CommandName for CommandResult {
    fn command_name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Create { .. } => "create",
            Self::Attach { .. } => "attach",
            Self::Detach { .. } => "detach",
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::Abort { .. } => "abort",
            Self::SetModel { .. } => "set_model",
            Self::SetThinking { .. } => "set_thinking",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{SessionMetadata, SessionPhase};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct FakeServer {
        revision: u64,
        sessions: HashMap<String, SessionSnapshot>,
    }

    fn snapshot(id: &str, attached: bool, revision: u64, text: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: id.into(),
            name: Some("demo".into()),
            cwd: "/tmp".into(),
            created_at: 1,
            updated_at: 1,
            phase: SessionPhase::Idle,
            model: ModelRef {
                provider: "google".into(),
                id: "gemini".into(),
            },
            thinking_level: ThinkingLevel::Off,
            attached,
            locked: false,
            revision,
            transcript: if text.is_empty() {
                Vec::new()
            } else {
                vec![pi_protocol::TranscriptItem::User {
                    id: "u1".into(),
                    content: vec![pi_protocol::TextOrImage::Text { text: text.into() }],
                    timestamp: 1,
                }]
            },
            queued_steer: Vec::new(),
            queued_steer_count: 0,
        }
    }

    fn client_with(server: Rc<RefCell<FakeServer>>) -> SessionClient {
        SessionClient::new(move |message| match message {
            ClientMessage::Hello { version } => (
                ServerMessage::Hello {
                    version,
                    connection_id: "c1".into(),
                    snapshot: ServerSnapshot {
                        server_id: "s".into(),
                        protocol_version: PROTOCOL_VERSION,
                        revision: server.borrow().revision,
                        sessions: server
                            .borrow()
                            .sessions
                            .values()
                            .map(|item| SessionMetadata {
                                id: item.id.clone(),
                                created_at: item.created_at,
                                updated_at: Some(item.updated_at),
                                parent_session_id: None,
                                session_name: item.name.clone(),
                                cwd: Some(item.cwd.clone()),
                            })
                            .collect(),
                        models: Vec::new(),
                    },
                },
                Vec::new(),
            ),
            ClientMessage::Request { id, request } => {
                let mut server = server.borrow_mut();
                server.revision += 1;
                let result = match request {
                    Command::Create { .. } => {
                        let created = snapshot("sess-1", true, server.revision, "");
                        server.sessions.insert(created.id.clone(), created.clone());
                        CommandResult::Create { session: created }
                    }
                    Command::Attach { session_id } => {
                        let mut session = server
                            .sessions
                            .get(&session_id)
                            .cloned()
                            .unwrap_or_else(|| snapshot(&session_id, true, server.revision, ""));
                        session.attached = true;
                        session.revision = server.revision;
                        server.sessions.insert(session_id, session.clone());
                        CommandResult::Attach { session }
                    }
                    Command::Detach { session_id } => {
                        let revision = server.revision;
                        if let Some(session) = server.sessions.get_mut(&session_id) {
                            session.attached = false;
                            session.revision = revision;
                        }
                        CommandResult::Detach { session_id }
                    }
                    Command::Prompt { session_id, text } => {
                        let mut session = server.sessions[&session_id].clone();
                        session.revision = server.revision;
                        session.transcript.push(pi_protocol::TranscriptItem::User {
                            id: format!("u{}", session.revision),
                            content: vec![pi_protocol::TextOrImage::Text { text }],
                            timestamp: session.revision,
                        });
                        server.sessions.insert(session_id, session.clone());
                        CommandResult::Prompt { session }
                    }
                    Command::List => CommandResult::List {
                        sessions: Vec::new(),
                    },
                    _ => CommandResult::List {
                        sessions: Vec::new(),
                    },
                };
                (
                    ServerMessage::Response {
                        id,
                        ok: true,
                        result: Some(result),
                        error: None,
                    },
                    Vec::new(),
                )
            }
        })
    }

    #[test]
    fn create_prompt_detach_and_reconnect() {
        let server = Rc::new(RefCell::new(FakeServer::default()));
        let client = client_with(server);
        let disconnected = match client.create_session(None, None) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("expected disconnected"),
        };
        assert!(disconnected.contains(DISCONNECTED));
        client.connect().unwrap();
        let handle = client
            .create_session(Some("/tmp".into()), Some("demo".into()))
            .unwrap();
        assert!(handle.attached());
        let prompted = handle.prompt("hello").unwrap();
        assert!(prompted.transcript.iter().any(|item| matches!(
            item,
            pi_protocol::TranscriptItem::User { content, .. }
                if content.iter().any(|part| matches!(part, pi_protocol::TextOrImage::Text { text } if text == "hello"))
        )));
        handle.detach().unwrap();
        assert!(!handle.attached());
        let detached = match handle.prompt("again") {
            Err(err) => err.to_string(),
            Ok(_) => panic!("expected detached"),
        };
        assert!(detached.contains("is not attached"));
        client.reconnect().unwrap();
        assert!(client.connected());
        client.dispose();
        assert!(client.disposed());
        let disposed = match client.list_sessions() {
            Err(err) => err.to_string(),
            Ok(_) => panic!("expected disposed"),
        };
        assert!(disposed.contains(DISPOSED));
    }

    #[test]
    fn framed_loopback_connects_and_lists() {
        let server = Rc::new(RefCell::new(FakeServer::default()));
        let client = SessionClient::with_loopback({
            let server = server.clone();
            move |message| match message {
                ClientMessage::Hello { version } => (
                    ServerMessage::Hello {
                        version,
                        connection_id: "c1".into(),
                        snapshot: ServerSnapshot {
                            server_id: "s".into(),
                            protocol_version: PROTOCOL_VERSION,
                            revision: server.borrow().revision,
                            sessions: Vec::new(),
                            models: Vec::new(),
                        },
                    },
                    Vec::new(),
                ),
                ClientMessage::Request { id, request } => {
                    let result = match request {
                        Command::List => CommandResult::List {
                            sessions: Vec::new(),
                        },
                        _ => CommandResult::List {
                            sessions: Vec::new(),
                        },
                    };
                    (
                        ServerMessage::Response {
                            id,
                            ok: true,
                            result: Some(result),
                            error: None,
                        },
                        Vec::new(),
                    )
                }
            }
        })
        .unwrap();
        assert_eq!(client.connection_state(), ConnectionState::Disconnected);
        client.connect().unwrap();
        assert_eq!(client.connection_state(), ConnectionState::Connected);
        assert!(client.list_sessions().unwrap().is_empty());
        client.reconnect().unwrap();
        assert_eq!(client.connection_state(), ConnectionState::Connected);
    }

    #[test]
    fn exclusive_lease_rejects_second_acquire() {
        let server = Rc::new(RefCell::new(FakeServer::default()));
        let client = client_with(server);
        client.connect().unwrap();
        let handle = client.create_session(None, None).unwrap();
        let exclusive = match client.acquire_session(&handle.id, "exclusive") {
            Err(err) => err.to_string(),
            Ok(_) => panic!("expected exclusive lease error"),
        };
        assert!(exclusive.contains("already has an active lease"));
        let shared = match client.acquire_session(&handle.id, "shared") {
            Err(err) => err.to_string(),
            Ok(_) => panic!("expected shared lease error"),
        };
        assert!(shared.contains("has an exclusive lease"));
    }
}
