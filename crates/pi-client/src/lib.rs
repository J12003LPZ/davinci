//! Protocol client matching `@earendil-works/pi-client`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use pi_protocol::{
    encode_client_message, ClientMessage, ClientMessageDecoder, Command, ProtocolError,
    ProtocolErrorCode, ServerMessage, PROTOCOL_VERSION,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Protocol(String),
    #[error("Handshake timed out")]
    HandshakeTimeout,
}

pub enum Transport {
    Unix(UnixStream),
    Tcp(TcpStream),
    Memory(MemoryPipe),
}

#[derive(Default)]
pub struct MemoryPipe {
    pub incoming: Vec<u8>,
    pub outgoing: Vec<u8>,
}

impl MemoryPipe {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct PiClient {
    pub connection_id: Option<String>,
    pending: HashMap<String, String>,
    handshake_timeout: Duration,
}

impl Default for PiClient {
    fn default() -> Self {
        Self {
            connection_id: None,
            pending: HashMap::new(),
            handshake_timeout: Duration::from_secs(5),
        }
    }
}

impl PiClient {
    pub fn set_handshake_timeout(&mut self, timeout: Duration) {
        self.handshake_timeout = timeout;
    }

    pub fn hello_message() -> ClientMessage {
        ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        }
    }

    pub fn request(&mut self, command: Command) -> (String, ClientMessage) {
        let id = Uuid::new_v4().to_string();
        self.pending.insert(id.clone(), command.name().to_string());
        (
            id.clone(),
            ClientMessage::Request {
                id,
                request: command,
            },
        )
    }

    pub fn correlate<'a>(&self, message: &'a ServerMessage) -> Option<&'a str> {
        match message {
            ServerMessage::Response { id, .. } => Some(id.as_str()),
            _ => None,
        }
    }

    pub fn encode(&self, message: &ClientMessage) -> Result<Vec<u8>, ClientError> {
        encode_client_message(message, None).map_err(|err| ClientError::Protocol(err.to_string()))
    }

    pub fn handshake_error(version: u32) -> ServerMessage {
        ServerMessage::HelloError {
            error: ProtocolError {
                code: ProtocolErrorCode::Version,
                message: format!("Unsupported protocol version {version}"),
                details: None,
            },
        }
    }
}

pub fn connect_unix(path: &str) -> Result<UnixStream, ClientError> {
    UnixStream::connect(path).map_err(|err| ClientError::Io(err.to_string()))
}

pub fn connect_tcp(addr: &str) -> Result<TcpStream, ClientError> {
    TcpStream::connect(addr).map_err(|err| ClientError::Io(err.to_string()))
}

pub fn write_message<W: Write>(writer: &mut W, message: &ClientMessage) -> Result<(), ClientError> {
    let bytes = encode_client_message(message, None)
        .map_err(|err| ClientError::Protocol(err.to_string()))?;
    writer
        .write_all(&bytes)
        .map_err(|err| ClientError::Io(err.to_string()))
}

pub fn read_messages<R: Read>(
    reader: &mut R,
    decoder: &mut ClientMessageDecoder,
) -> Result<Vec<ClientMessage>, ClientError> {
    let mut buf = [0u8; 4096];
    let n = reader
        .read(&mut buf)
        .map_err(|err| ClientError::Io(err.to_string()))?;
    decoder
        .push(&buf[..n])
        .map_err(|err| ClientError::Protocol(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{create_client_message_decoder, CommandResult};

    #[test]
    fn request_correlation_and_hello() {
        let mut client = PiClient::default();
        let (id, message) = client.request(Command::List);
        let encoded = client.encode(&message).unwrap();
        let mut decoder = create_client_message_decoder(None).unwrap();
        let decoded = decoder.push(&encoded).unwrap();
        decoder.end().unwrap();
        match &decoded[0] {
            ClientMessage::Request {
                id: decoded_id,
                request,
            } => {
                assert_eq!(decoded_id, &id);
                assert!(matches!(request, Command::List));
            }
            _ => panic!("expected request"),
        }
        assert_eq!(
            client.correlate(&ServerMessage::Response {
                id: id.clone(),
                ok: true,
                result: Some(CommandResult::List { sessions: vec![] }),
                error: None,
            }),
            Some(id.as_str())
        );
    }
}
