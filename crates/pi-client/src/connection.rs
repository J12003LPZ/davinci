//! Framed `Connection` + `ByteTransport` matching TypeScript `connection.ts` / `transport.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use pi_protocol::{
    create_client_message_decoder, create_server_message_decoder, encode_client_message,
    encode_server_message, ClientMessage, ClientMessageDecoder, FrameDecoderOptions, ServerEvent,
    ServerMessage, ServerMessageDecoder, ServerSnapshot, PROTOCOL_VERSION,
};

use crate::ClientError;

pub const MAX_UINT32: u64 = 0xffff_ffff;

pub type TransportFactory =
    Box<dyn FnMut(ByteTransportHandlers) -> Result<Box<dyn ByteTransport>, ClientError>>;
type OnData = Box<dyn FnMut(&[u8])>;
type HandshakeListener = Box<dyn FnMut(&ServerSnapshot) -> Result<(), ClientError>>;
type MessageListener = Box<dyn FnMut(&ServerMessage)>;
type StateChangeListener = Box<dyn FnMut(&ConnectionStateChange)>;
type DispatchFn = dyn FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>);
type SharedDispatch = Rc<RefCell<DispatchFn>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

impl ConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
        }
    }
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionStateChange {
    pub state: ConnectionState,
    pub error: Option<String>,
}

pub trait ByteTransport {
    fn send(&mut self, chunk: &[u8]) -> Result<(), ClientError>;
    fn close(&mut self);
}

pub struct ByteTransportHandlers {
    pub on_data: OnData,
    pub on_close: Box<dyn FnMut()>,
    pub on_error: Box<dyn FnMut(ClientError)>,
}

struct ConnectionInner {
    max_frame_length: u32,
    state: ConnectionState,
    sequence: u64,
    current_id: u64,
    decoder: Option<ServerMessageDecoder>,
    transport: Option<Box<dyn ByteTransport>>,
    transport_attached: bool,
    handshake: Option<Result<ServerSnapshot, ClientError>>,
    inbound: Vec<ServerMessage>,
    on_handshake: Option<HandshakeListener>,
    on_message: Option<MessageListener>,
    on_state_change: Option<StateChangeListener>,
}

pub struct Connection {
    inner: Rc<RefCell<ConnectionInner>>,
}

impl Connection {
    pub fn new(max_frame_length: Option<u64>) -> Result<Self, ClientError> {
        let value = max_frame_length.unwrap_or(u64::from(pi_protocol::DEFAULT_MAX_FRAME_LENGTH));
        if value == 0 || value > MAX_UINT32 {
            return Err(ClientError::Protocol(format!(
                "PiClient maxFrameLength must be an integer between 1 and {MAX_UINT32}"
            )));
        }
        Ok(Self {
            inner: Rc::new(RefCell::new(ConnectionInner {
                max_frame_length: value as u32,
                state: ConnectionState::Disconnected,
                sequence: 0,
                current_id: 0,
                decoder: None,
                transport: None,
                transport_attached: false,
                handshake: None,
                inbound: Vec::new(),
                on_handshake: None,
                on_message: None,
                on_state_change: None,
            })),
        })
    }

    pub fn on_handshake(
        &self,
        listener: impl FnMut(&ServerSnapshot) -> Result<(), ClientError> + 'static,
    ) {
        self.inner.borrow_mut().on_handshake = Some(Box::new(listener));
    }

    pub fn on_message(&self, listener: impl FnMut(&ServerMessage) + 'static) {
        self.inner.borrow_mut().on_message = Some(Box::new(listener));
    }

    pub fn on_state_change(&self, listener: impl FnMut(&ConnectionStateChange) + 'static) {
        self.inner.borrow_mut().on_state_change = Some(Box::new(listener));
    }

    pub fn state(&self) -> ConnectionState {
        self.inner.borrow().state
    }

    pub fn max_frame_length(&self) -> u32 {
        self.inner.borrow().max_frame_length
    }

    pub fn inbound(&self) -> Vec<ServerMessage> {
        self.inner.borrow().inbound.clone()
    }

    pub fn connect(&self, factory: &mut TransportFactory) -> Result<ServerSnapshot, ClientError> {
        if self.state() != ConnectionState::Disconnected {
            return Err(ClientError::Protocol(format!(
                "PiClient is already {}",
                self.state()
            )));
        }
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.sequence += 1;
            inner.current_id = inner.sequence;
            inner.handshake = None;
            inner.transport_attached = false;
            inner.inbound.clear();
            inner.decoder = Some(
                create_server_message_decoder(Some(FrameDecoderOptions {
                    max_frame_length: Some(inner.max_frame_length),
                }))
                .map_err(|err| ClientError::Protocol(err.to_string()))?,
            );
            inner.state = ConnectionState::Connecting;
            inner.current_id
        };
        self.emit_state(ConnectionState::Connecting, None);
        let handlers = self.make_handlers(id);
        let transport = match factory(handlers) {
            Ok(transport) => transport,
            Err(error) => {
                self.fail(error.clone());
                return Err(error);
            }
        };
        if !self.is_current(id) {
            let mut transport = transport;
            transport.close();
            return self.take_handshake();
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.transport = Some(transport);
            inner.transport_attached = true;
        }
        let hello = encode_client_message(
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
            },
            Some(FrameDecoderOptions {
                max_frame_length: Some(self.max_frame_length()),
            }),
        )
        .map_err(|err| ClientError::Protocol(err.to_string()))?;
        if let Err(error) = self.send_on_current_transport(&hello) {
            self.fail_and_close(error.clone());
            return Err(error);
        }
        self.take_handshake()
    }

    pub fn disconnect(&self, reason: impl Into<String>) {
        if self.state() == ConnectionState::Disconnected {
            return;
        }
        self.fail_and_close(ClientError::Protocol(reason.into()));
    }

    pub fn send(&self, frame: &[u8]) -> Result<(), ClientError> {
        if self.state() != ConnectionState::Connected {
            return Err(ClientError::Protocol("Pi client is disconnected".into()));
        }
        if let Err(error) = self.send_on_current_transport(frame) {
            self.fail_and_close(error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn make_handlers(&self, id: u64) -> ByteTransportHandlers {
        let data = self.clone_rc();
        let close = self.clone_rc();
        let error = self.clone_rc();
        ByteTransportHandlers {
            on_data: Box::new(move |chunk| data.handle_data(id, chunk)),
            on_close: Box::new(move || {
                if close.is_current(id) {
                    close.handle_close();
                }
            }),
            on_error: Box::new(move |err| {
                if error.is_current(id) {
                    error.fail_and_close(err);
                }
            }),
        }
    }

    fn handle_data(&self, id: u64, chunk: &[u8]) {
        if !self.is_current(id) {
            return;
        }
        {
            let inner = self.inner.borrow();
            if inner.state == ConnectionState::Connecting && !inner.transport_attached {
                drop(inner);
                self.fail_and_close(ClientError::Protocol(
                    "Received server data before the client hello was sent".into(),
                ));
                return;
            }
        }
        let messages = {
            let mut inner = self.inner.borrow_mut();
            let Some(decoder) = inner.decoder.as_mut() else {
                return;
            };
            match decoder.push(chunk) {
                Ok(messages) => messages,
                Err(error) => {
                    drop(inner);
                    self.fail_and_close(ClientError::Protocol(error.to_string()));
                    return;
                }
            }
        };
        for message in messages {
            if self.state() == ConnectionState::Disconnected {
                return;
            }
            self.handle_message(message);
        }
    }

    fn handle_message(&self, message: ServerMessage) {
        let state = self.state();
        if state == ConnectionState::Connecting {
            match &message {
                ServerMessage::HelloError { error } => {
                    self.fail_and_close(ClientError::Protocol(error.message.clone()));
                    return;
                }
                ServerMessage::Hello { snapshot, .. } => {
                    if !self.inner.borrow().transport_attached {
                        self.fail_and_close(ClientError::Protocol(
                            "Received server hello before the client hello was sent".into(),
                        ));
                        return;
                    }
                    let snapshot = snapshot.clone();
                    {
                        let mut inner = self.inner.borrow_mut();
                        inner.state = ConnectionState::Connected;
                    }
                    let handshake_err = {
                        let mut inner = self.inner.borrow_mut();
                        if let Some(listener) = inner.on_handshake.as_mut() {
                            listener(&snapshot).err()
                        } else {
                            None
                        }
                    };
                    if let Some(error) = handshake_err {
                        self.fail_and_close(error);
                        return;
                    }
                    if self.state() != ConnectionState::Connected {
                        return;
                    }
                    self.emit_state(ConnectionState::Connected, None);
                    if self.state() != ConnectionState::Connected {
                        return;
                    }
                    self.inner.borrow_mut().handshake = Some(Ok(snapshot));
                    return;
                }
                _ => {
                    self.fail_and_close(ClientError::Protocol(
                        "Expected server hello as first message".into(),
                    ));
                    return;
                }
            }
        }
        if state != ConnectionState::Connected {
            return;
        }
        if matches!(
            message,
            ServerMessage::Hello { .. } | ServerMessage::HelloError { .. }
        ) {
            self.fail_and_close(ClientError::Protocol("Unexpected handshake message".into()));
            return;
        }
        {
            let mut inner = self.inner.borrow_mut();
            if let Some(listener) = inner.on_message.as_mut() {
                listener(&message);
            }
            inner.inbound.push(message);
        }
    }

    fn handle_close(&self) {
        if self.state() == ConnectionState::Disconnected {
            return;
        }
        let decoder_error = {
            let mut inner = self.inner.borrow_mut();
            match inner.decoder.as_mut().map(ServerMessageDecoder::end) {
                Some(Err(error)) => Some(ClientError::Protocol(error.to_string())),
                _ => None,
            }
        };
        self.fail(
            decoder_error.unwrap_or_else(|| ClientError::Protocol("Byte transport closed".into())),
        );
    }

    fn send_on_current_transport(&self, frame: &[u8]) -> Result<(), ClientError> {
        let mut transport = self.inner.borrow_mut().transport.take();
        let result = match transport.as_mut() {
            Some(transport) => transport.send(frame),
            None => Err(ClientError::Protocol("Pi client is disconnected".into())),
        };
        let still_current = self.state() != ConnectionState::Disconnected;
        let mut inner = self.inner.borrow_mut();
        if still_current && inner.transport.is_none() {
            inner.transport = transport;
        } else if let Some(mut leftover) = transport {
            leftover.close();
        }
        result
    }

    fn fail_and_close(&self, error: ClientError) {
        let mut transport = {
            let mut inner = self.inner.borrow_mut();
            inner.transport.take()
        };
        self.fail(error);
        if let Some(transport) = transport.as_mut() {
            transport.close();
        }
    }

    fn fail(&self, error: ClientError) {
        if self.state() == ConnectionState::Disconnected {
            return;
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.state = ConnectionState::Disconnected;
            inner.decoder = None;
            inner.transport_attached = false;
            if inner.handshake.is_none() {
                inner.handshake = Some(Err(error.clone()));
            }
        }
        self.emit_state(ConnectionState::Disconnected, Some(error.to_string()));
    }

    fn emit_state(&self, state: ConnectionState, error: Option<String>) {
        let change = ConnectionStateChange { state, error };
        if let Some(listener) = self.inner.borrow_mut().on_state_change.as_mut() {
            listener(&change);
        }
    }

    fn take_handshake(&self) -> Result<ServerSnapshot, ClientError> {
        match self.inner.borrow_mut().handshake.take() {
            Some(result) => result,
            None => Err(ClientError::Protocol(
                "Expected server hello as first message".into(),
            )),
        }
    }

    fn is_current(&self, id: u64) -> bool {
        let inner = self.inner.borrow();
        inner.state != ConnectionState::Disconnected && inner.current_id == id
    }

    fn clone_rc(&self) -> Self {
        self.clone()
    }
}

impl Clone for Connection {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// In-process framed transport: `send` decodes client frames, dispatches, and writes server frames.
pub struct LoopbackTransport {
    dispatch: SharedDispatch,
    handlers: RefCell<ByteTransportHandlers>,
    decoder: ClientMessageDecoder,
}

impl LoopbackTransport {
    pub fn new(
        dispatch: impl FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>) + 'static,
        handlers: ByteTransportHandlers,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            dispatch: Rc::new(RefCell::new(dispatch)),
            handlers: RefCell::new(handlers),
            decoder: create_client_message_decoder(None)
                .map_err(|err| ClientError::Protocol(err.to_string()))?,
        })
    }

    pub fn from_shared(
        dispatch: SharedDispatch,
        handlers: ByteTransportHandlers,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            dispatch,
            handlers: RefCell::new(handlers),
            decoder: create_client_message_decoder(None)
                .map_err(|err| ClientError::Protocol(err.to_string()))?,
        })
    }
}

impl ByteTransport for LoopbackTransport {
    fn send(&mut self, chunk: &[u8]) -> Result<(), ClientError> {
        let messages = self
            .decoder
            .push(chunk)
            .map_err(|err| ClientError::Protocol(err.to_string()))?;
        for message in messages {
            let (response, events) = (self.dispatch.borrow_mut())(message);
            let encoded = encode_server_message(&response, None)
                .map_err(|err| ClientError::Protocol(err.to_string()))?;
            (self.handlers.borrow_mut().on_data)(&encoded);
            for event in events {
                let encoded = encode_server_message(&ServerMessage::Event { event }, None)
                    .map_err(|err| ClientError::Protocol(err.to_string()))?;
                (self.handlers.borrow_mut().on_data)(&encoded);
            }
        }
        Ok(())
    }

    fn close(&mut self) {
        (self.handlers.borrow_mut().on_close)();
    }
}

pub fn loopback_factory(
    dispatch: impl FnMut(ClientMessage) -> (ServerMessage, Vec<ServerEvent>) + 'static,
) -> TransportFactory {
    let dispatch: SharedDispatch = Rc::new(RefCell::new(dispatch));
    Box::new(move |handlers| {
        Ok(Box::new(LoopbackTransport::from_shared(
            dispatch.clone(),
            handlers,
        )?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{ProtocolError, ProtocolErrorCode, ServerSnapshot};

    fn empty_snapshot() -> ServerSnapshot {
        ServerSnapshot {
            server_id: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            revision: 1,
            sessions: Vec::new(),
            models: Vec::new(),
        }
    }

    fn hello_factory(snapshot: ServerSnapshot) -> TransportFactory {
        loopback_factory(move |message| match message {
            ClientMessage::Hello { version } => (
                ServerMessage::Hello {
                    version,
                    connection_id: "c1".into(),
                    snapshot: snapshot.clone(),
                },
                Vec::new(),
            ),
            ClientMessage::Request { id, .. } => (
                ServerMessage::Response {
                    id,
                    ok: true,
                    result: None,
                    error: None,
                },
                Vec::new(),
            ),
        })
    }

    #[test]
    fn framed_hello_connects_and_rejects_double_connect() {
        let connection = Connection::new(None).unwrap();
        let mut factory = hello_factory(empty_snapshot());
        let snapshot = connection.connect(&mut factory).unwrap();
        assert_eq!(snapshot.server_id, "s");
        assert_eq!(connection.state(), ConnectionState::Connected);
        let err = connection.connect(&mut factory).unwrap_err().to_string();
        assert_eq!(err, "PiClient is already connected");
    }

    #[test]
    fn rejects_data_before_client_hello() {
        let connection = Connection::new(None).unwrap();
        let mut factory: TransportFactory = Box::new(|mut handlers| {
            let frame = encode_server_message(
                &ServerMessage::Hello {
                    version: PROTOCOL_VERSION,
                    connection_id: "c1".into(),
                    snapshot: empty_snapshot(),
                },
                None,
            )
            .unwrap();
            (handlers.on_data)(&frame);
            Ok(Box::new(ClosedTransport))
        });
        let err = connection.connect(&mut factory).unwrap_err().to_string();
        assert_eq!(err, "Received server data before the client hello was sent");
        assert_eq!(connection.state(), ConnectionState::Disconnected);
    }

    #[test]
    fn rejects_non_hello_first_message() {
        let connection = Connection::new(None).unwrap();
        let mut factory = loopback_factory(|message| match message {
            ClientMessage::Hello { .. } => (
                ServerMessage::Response {
                    id: "x".into(),
                    ok: true,
                    result: None,
                    error: None,
                },
                Vec::new(),
            ),
            _ => (
                ServerMessage::Response {
                    id: "x".into(),
                    ok: true,
                    result: None,
                    error: None,
                },
                Vec::new(),
            ),
        });
        let err = connection.connect(&mut factory).unwrap_err().to_string();
        assert_eq!(err, "Expected server hello as first message");
    }

    #[test]
    fn hello_error_uses_server_message() {
        let connection = Connection::new(None).unwrap();
        let mut factory = loopback_factory(|message| match message {
            ClientMessage::Hello { .. } => (
                ServerMessage::HelloError {
                    error: ProtocolError {
                        code: ProtocolErrorCode::Version,
                        message: "Unsupported protocol version".into(),
                        details: None,
                    },
                },
                Vec::new(),
            ),
            _ => (
                ServerMessage::Response {
                    id: "x".into(),
                    ok: true,
                    result: None,
                    error: None,
                },
                Vec::new(),
            ),
        });
        let err = connection.connect(&mut factory).unwrap_err().to_string();
        assert_eq!(err, "Unsupported protocol version");
        assert_eq!(connection.state(), ConnectionState::Disconnected);
    }

    #[test]
    fn close_reports_byte_transport_closed() {
        let connection = Connection::new(None).unwrap();
        let mut factory = hello_factory(empty_snapshot());
        connection.connect(&mut factory).unwrap();
        connection.disconnect("Byte transport closed");
        assert_eq!(connection.state(), ConnectionState::Disconnected);
    }

    #[test]
    fn rejects_invalid_max_frame_length() {
        let err = match Connection::new(Some(0)) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("expected maxFrameLength error"),
        };
        assert_eq!(
            err,
            format!("PiClient maxFrameLength must be an integer between 1 and {MAX_UINT32}")
        );
        let err = match Connection::new(Some(MAX_UINT32 + 1)) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("expected maxFrameLength error"),
        };
        assert!(err.contains("maxFrameLength"));
    }

    #[test]
    fn unexpected_hello_after_connect_fails() {
        let connection = Connection::new(None).unwrap();
        let mut factory = hello_factory(empty_snapshot());
        connection.connect(&mut factory).unwrap();
        let frame = encode_server_message(
            &ServerMessage::Hello {
                version: PROTOCOL_VERSION,
                connection_id: "c2".into(),
                snapshot: empty_snapshot(),
            },
            None,
        )
        .unwrap();
        connection.handle_data(connection.inner.borrow().current_id, &frame);
        assert_eq!(connection.state(), ConnectionState::Disconnected);
    }

    struct ClosedTransport;

    impl ByteTransport for ClosedTransport {
        fn send(&mut self, _chunk: &[u8]) -> Result<(), ClientError> {
            Ok(())
        }
        fn close(&mut self) {}
    }
}
