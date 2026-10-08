//! Regression tests for client-side IPC integrity: response correlation
//! (WOR-48), reentrant callbacks (WOR-49) and the `SessionHandle` release
//! lifecycle (WOR-50).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use davinci_client::{
    loopback_factory, ByteTransport, ByteTransportHandlers, ClientError, Connection,
    ConnectionState, SessionClient, TransportFactory,
};
use davinci_protocol::{
    create_client_message_decoder, encode_client_message, encode_server_message, ClientMessage,
    ClientMessageDecoder, Command, CommandResult, ModelRef, ProtocolError, ProtocolErrorCode,
    ServerEvent, ServerMessage, ServerSnapshot, SessionMetadata, SessionPhase, SessionSnapshot,
    ThinkingLevel, PROTOCOL_VERSION,
};

fn server_snapshot() -> ServerSnapshot {
    ServerSnapshot {
        server_id: "s".into(),
        protocol_version: PROTOCOL_VERSION,
        revision: 1,
        sessions: Vec::new(),
        models: Vec::new(),
    }
}

fn hello(version: u32) -> ServerMessage {
    ServerMessage::Hello {
        version,
        connection_id: "c1".into(),
        snapshot: server_snapshot(),
    }
}

fn session(id: &str, attached: bool, revision: u64) -> SessionSnapshot {
    SessionSnapshot {
        id: id.into(),
        name: None,
        cwd: "/tmp".into(),
        created_at: 1,
        updated_at: 1,
        phase: SessionPhase::Idle,
        model: ModelRef {
            provider: "p".into(),
            id: "m".into(),
        },
        thinking_level: ThinkingLevel::Off,
        attached,
        locked: false,
        revision,
        transcript: Vec::new(),
        queued_steer: Vec::new(),
        queued_steer_count: 0,
    }
}

fn listing(marker: &str) -> CommandResult {
    CommandResult::List {
        sessions: vec![SessionMetadata {
            id: marker.into(),
            created_at: 1,
            updated_at: None,
            parent_session_id: None,
            session_name: None,
            cwd: None,
        }],
    }
}

fn ok(id: impl Into<String>, result: CommandResult) -> ServerMessage {
    ServerMessage::Response {
        id: id.into(),
        ok: true,
        result: Some(result),
        error: None,
    }
}

/// In-memory server: tracks attachment per session and counts detaches.
#[derive(Default)]
struct FakeServer {
    revision: u64,
    sessions: HashMap<String, SessionSnapshot>,
    detaches: u32,
}

impl FakeServer {
    fn handle(&mut self, message: ClientMessage) -> (ServerMessage, Vec<ServerEvent>) {
        let (id, request) = match message {
            ClientMessage::Hello { version } => return (hello(version), Vec::new()),
            ClientMessage::Request { id, request } => (id, request),
        };
        self.revision += 1;
        let revision = self.revision;
        let result = match request {
            Command::Create { .. } => {
                let created = session("sess-1", true, revision);
                self.sessions.insert(created.id.clone(), created.clone());
                CommandResult::Create { session: created }
            }
            Command::Attach { session_id } => {
                let attached = session(&session_id, true, revision);
                self.sessions.insert(session_id, attached.clone());
                CommandResult::Attach { session: attached }
            }
            Command::Detach { session_id } => {
                self.detaches += 1;
                if let Some(item) = self.sessions.get_mut(&session_id) {
                    item.attached = false;
                    item.revision = revision;
                }
                CommandResult::Detach { session_id }
            }
            Command::Prompt { session_id, .. } => {
                let mut item = self.sessions[&session_id].clone();
                item.revision = revision;
                self.sessions.insert(session_id, item.clone());
                CommandResult::Prompt { session: item }
            }
            _ => listing("live"),
        };
        (ok(id, result), Vec::new())
    }
}

fn dispatch_client(server: Rc<RefCell<FakeServer>>) -> SessionClient {
    SessionClient::new(move |message| server.borrow_mut().handle(message))
}

// ---------------------------------------------------------------- WOR-48

/// Transport whose server answers the Nth request with the Nth scripted list
/// of frames, so a test can withhold a response and deliver it late.
struct ScriptedTransport {
    handlers: ByteTransportHandlers,
    decoder: ClientMessageDecoder,
    replies: Rc<RefCell<Vec<Vec<ServerMessage>>>>,
}

impl ByteTransport for ScriptedTransport {
    fn send(&mut self, chunk: &[u8]) -> Result<(), ClientError> {
        let messages = self
            .decoder
            .push(chunk)
            .map_err(|err| ClientError::Protocol(err.to_string()))?;
        for message in messages {
            let frames = match message {
                ClientMessage::Hello { version } => vec![hello(version)],
                ClientMessage::Request { .. } => {
                    let mut replies = self.replies.borrow_mut();
                    if replies.is_empty() {
                        Vec::new()
                    } else {
                        replies.remove(0)
                    }
                }
            };
            for frame in frames {
                let bytes = encode_server_message(&frame, None).unwrap();
                (self.handlers.on_data)(&bytes);
            }
        }
        Ok(())
    }

    fn close(&mut self) {}
}

fn scripted_client(replies: Vec<Vec<ServerMessage>>) -> SessionClient {
    let replies = Rc::new(RefCell::new(replies));
    let factory: TransportFactory = Box::new(move |handlers| {
        Ok(Box::new(ScriptedTransport {
            handlers,
            decoder: create_client_message_decoder(None).unwrap(),
            replies: replies.clone(),
        }))
    });
    let client = SessionClient::with_factory(factory).unwrap();
    client.connect().unwrap();
    client
}

fn first_id(sessions: &[SessionMetadata]) -> &str {
    sessions.first().map(|item| item.id.as_str()).unwrap_or("")
}

#[test]
fn wor48_late_response_for_timed_out_request_is_not_accepted_by_next_request() {
    // request-1 gets no reply in time; its reply arrives after request-2's own.
    let client = scripted_client(vec![
        Vec::new(),
        vec![
            ok("request-2", listing("fresh")),
            ok("request-1", listing("stale")),
        ],
    ]);
    assert!(client.list_sessions().is_err(), "request-1 had no response");
    let sessions = client
        .list_sessions()
        .expect("request-2 has its own response");
    assert_eq!(first_id(&sessions), "fresh");
}

#[test]
fn wor48_only_a_stale_response_fails_the_request_instead_of_answering_it() {
    let client = scripted_client(vec![Vec::new(), vec![ok("request-1", listing("stale"))]]);
    assert!(client.list_sessions().is_err());
    let error = client
        .list_sessions()
        .expect_err("a response for request-1 must not satisfy request-2");
    assert!(error.to_string().contains("missing"), "{error}");
    assert!(client.connected());
}

#[test]
fn wor48_dispatch_response_with_wrong_id_is_rejected() {
    let client = SessionClient::new(|message| match message {
        ClientMessage::Hello { version } => (hello(version), Vec::new()),
        ClientMessage::Request { .. } => (ok("request-0", listing("stale")), Vec::new()),
    });
    client.connect().unwrap();
    let error = client.list_sessions().expect_err("mismatched id accepted");
    assert!(error.to_string().contains("request-0"), "{error}");
}

// ---------------------------------------------------------------- WOR-49

#[test]
fn wor49_session_listener_runs_during_prompt_without_borrow_panic() {
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = dispatch_client(server);
    client.connect().unwrap();
    let handle = client.create_session(None, None).unwrap();
    let seen = Rc::new(Cell::new(0u64));
    let observed = seen.clone();
    let _unsubscribe = handle.subscribe(move |snapshot| observed.set(snapshot.revision));
    let prompted = handle.prompt("hi").unwrap();
    assert_eq!(seen.get(), prompted.revision);
}

#[test]
fn wor49_event_listener_can_query_client_during_framed_dispatch() {
    let client = SessionClient::with_loopback(|message| match message {
        ClientMessage::Hello { version } => (hello(version), Vec::new()),
        ClientMessage::Request { id, .. } => (
            ok(id, listing("live")),
            vec![ServerEvent::SessionSnapshot {
                snapshot: session("sess-9", false, 5),
            }],
        ),
    })
    .unwrap();
    client.connect().unwrap();
    let probe = client.clone();
    let observed = Rc::new(Cell::new(None));
    let sink = observed.clone();
    let _unsubscribe = client.on_event(move |_| {
        sink.set(Some((probe.connected(), probe.snapshot().is_some())));
    });
    client.list_sessions().unwrap();
    assert_eq!(observed.get(), Some((true, true)));
}

#[test]
fn wor49_snapshot_listener_can_query_client_during_dispatch_connect() {
    let client = SessionClient::new(|message| match message {
        ClientMessage::Hello { version } => (hello(version), Vec::new()),
        ClientMessage::Request { id, .. } => (ok(id, listing("live")), Vec::new()),
    });
    let probe = client.clone();
    let observed = Rc::new(Cell::new(false));
    let sink = observed.clone();
    let _unsubscribe = client.subscribe(move |_| sink.set(probe.snapshot().is_some()));
    client.connect().unwrap();
    assert!(observed.get());
}

fn list_loopback() -> TransportFactory {
    loopback_factory(|message| match message {
        ClientMessage::Hello { version } => (hello(version), Vec::new()),
        ClientMessage::Request { id, .. } => (ok(id, listing("live")), Vec::new()),
    })
}

#[test]
fn wor49_connection_listeners_can_reenter_the_connection() {
    let connection = Connection::new(None).unwrap();
    let states = Rc::new(RefCell::new(Vec::new()));
    {
        let probe = connection.clone();
        let states = states.clone();
        connection.on_state_change(move |change| {
            states.borrow_mut().push((change.state, probe.state()));
        });
    }
    {
        let probe = connection.clone();
        connection.on_handshake(move |_| {
            assert_eq!(probe.state(), ConnectionState::Connected);
            Ok(())
        });
    }
    let messages = Rc::new(Cell::new(0));
    {
        let probe = connection.clone();
        let messages = messages.clone();
        connection.on_message(move |_| {
            assert_eq!(probe.state(), ConnectionState::Connected);
            assert_eq!(probe.inbound().len(), messages.get() + 1);
            messages.set(messages.get() + 1);
        });
    }
    connection.connect(&mut list_loopback()).unwrap();
    let frame = encode_client_message(
        &ClientMessage::Request {
            id: "r1".into(),
            request: Command::List,
        },
        None,
    )
    .unwrap();
    connection.send(&frame).unwrap();
    assert_eq!(connection.inbound().len(), 1);
    connection.disconnect("bye");
    assert_eq!(messages.get(), 1);
    assert_eq!(
        *states.borrow(),
        vec![
            (ConnectionState::Connecting, ConnectionState::Connecting),
            (ConnectionState::Connected, ConnectionState::Connected),
            (ConnectionState::Disconnected, ConnectionState::Disconnected),
        ]
    );
}

#[test]
fn wor49_listener_replaced_from_inside_its_own_callback_keeps_the_replacement() {
    let connection = Connection::new(None).unwrap();
    let replacement_calls = Rc::new(Cell::new(0));
    {
        let probe = connection.clone();
        let calls = replacement_calls.clone();
        connection.on_state_change(move |_| {
            let calls = calls.clone();
            probe.on_state_change(move |_| calls.set(calls.get() + 1));
        });
    }
    connection.connect(&mut list_loopback()).unwrap();
    // Connecting ran the original, which installed the replacement;
    // Connected ran the replacement, which must not be overwritten.
    assert_eq!(replacement_calls.get(), 1);
    connection.disconnect("bye");
    assert_eq!(replacement_calls.get(), 2);
}

#[test]
fn wor49_handshake_listener_can_disconnect_from_inside_the_callback() {
    let connection = Connection::new(None).unwrap();
    {
        let probe = connection.clone();
        connection.on_handshake(move |_| {
            probe.disconnect("listener aborted");
            Ok(())
        });
    }
    let error = connection
        .connect(&mut list_loopback())
        .expect_err("disconnect inside handshake must fail connect");
    assert!(error.to_string().contains("listener aborted"), "{error}");
    assert_eq!(connection.state(), ConnectionState::Disconnected);
}

// ---------------------------------------------------------------- WOR-50

#[test]
fn wor50_double_release_of_shared_handle_does_not_steal_another_lease() {
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = dispatch_client(server.clone());
    client.connect().unwrap();
    let a = client.attach_session("sess-1").unwrap();
    let b = client.attach_session("sess-1").unwrap();

    a.detach().unwrap();
    a.detach().unwrap();
    a.dispose().unwrap();

    assert!(!a.active(), "released handle still reports active");
    assert!(b.active(), "repeat release on A detached B's session");
    assert_eq!(server.borrow().detaches, 0);
    b.prompt("still mine").unwrap();

    b.detach().unwrap();
    assert_eq!(server.borrow().detaches, 1);
    assert!(!b.active());
}

#[test]
fn wor50_released_handle_is_not_reactivated_by_a_new_lease() {
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = dispatch_client(server.clone());
    client.connect().unwrap();
    let old = client.create_session(None, None).unwrap();
    old.detach().unwrap();

    let fresh = client.acquire_session(&old.id, "exclusive").unwrap();
    assert!(fresh.active());
    assert!(!old.active(), "released handle came back to life");
    assert!(old.snapshot().is_none());
    let error = old
        .prompt("ghost")
        .expect_err("stale handle drove the session");
    assert!(error.to_string().contains("is not attached"), "{error}");

    // The stale handle cannot release the new lease either.
    old.dispose().unwrap();
    assert!(fresh.active());
    assert_eq!(server.borrow().detaches, 1);
}

#[test]
fn wor50_failed_detach_keeps_handle_usable_and_retryable() {
    let fail = Rc::new(Cell::new(true));
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = {
        let fail = fail.clone();
        let server = server.clone();
        SessionClient::new(move |message| match message {
            ClientMessage::Request {
                id,
                request: Command::Detach { .. },
            } if fail.get() => (
                ServerMessage::Response {
                    id,
                    ok: false,
                    result: None,
                    error: Some(ProtocolError {
                        code: ProtocolErrorCode::InternalError,
                        message: "detach failed".into(),
                        details: None,
                    }),
                },
                Vec::new(),
            ),
            other => server.borrow_mut().handle(other),
        })
    };
    client.connect().unwrap();
    let handle = client.create_session(None, None).unwrap();
    assert!(handle.detach().is_err());
    assert!(
        handle.active(),
        "a failed detach must not release the lease"
    );
    fail.set(false);
    handle.detach().unwrap();
    assert!(!handle.active());
    assert_eq!(server.borrow().detaches, 1);
}

#[test]
fn wor50_dispose_after_failed_detach_relinquishes_once() {
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = SessionClient::new({
        let server = server.clone();
        move |message| match message {
            ClientMessage::Request {
                id,
                request: Command::Detach { .. },
            } => (
                ServerMessage::Response {
                    id,
                    ok: false,
                    result: None,
                    error: Some(ProtocolError {
                        code: ProtocolErrorCode::InternalError,
                        message: "detach failed".into(),
                        details: None,
                    }),
                },
                Vec::new(),
            ),
            other => server.borrow_mut().handle(other),
        }
    });
    client.connect().unwrap();
    let handle = client.create_session(None, None).unwrap();
    assert!(handle.dispose().is_err());
    assert!(!handle.active(), "dispose relinquishes even on failure");
    handle.dispose().unwrap();
    // The lease is gone, so a fresh exclusive lease is allowed.
    client.acquire_session(&handle.id, "exclusive").unwrap();
}

#[test]
fn wor49_snapshot_listener_can_issue_requests_during_framed_connect() {
    let client = SessionClient::with_loopback(|message| match message {
        ClientMessage::Hello { version } => (hello(version), Vec::new()),
        ClientMessage::Request { id, .. } => (ok(id, listing("live")), Vec::new()),
    })
    .unwrap();
    let probe = client.clone();
    let listed = Rc::new(RefCell::new(None));
    let sink = listed.clone();
    let _unsubscribe = client.subscribe(move |_| {
        *sink.borrow_mut() = Some(probe.list_sessions().map(|sessions| sessions.len()));
    });
    client.connect().unwrap();
    let outcome = listed.borrow_mut().take().expect("snapshot listener ran");
    assert_eq!(outcome.expect("request from snapshot listener"), 1);
    assert!(client.connected());
}

#[test]
fn wor50_session_removed_during_attach_yields_no_live_handle() {
    let server = Rc::new(RefCell::new(FakeServer::default()));
    let client = SessionClient::new({
        let server = server.clone();
        move |message| {
            let removed = matches!(
                &message,
                ClientMessage::Request {
                    request: Command::Attach { .. },
                    ..
                }
            ) && server.borrow().revision == 0;
            let (response, mut events) = server.borrow_mut().handle(message);
            if removed {
                events.push(ServerEvent::SessionRemoved {
                    session_id: "sess-1".into(),
                });
            }
            (response, events)
        }
    });
    client.connect().unwrap();
    let error = client
        .acquire_session("sess-1", "shared")
        .err()
        .expect("removal during attach must not hand out a lease");
    assert!(error.to_string().contains("removed"), "{error}");

    // No phantom handle exists, so an exclusive lease is free and its owner
    // is the only one that can detach the session.
    let owner = client.acquire_session("sess-1", "exclusive").unwrap();
    assert!(owner.active());
    owner.detach().unwrap();
    assert_eq!(server.borrow().detaches, 1);
}
