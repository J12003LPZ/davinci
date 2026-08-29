//! Transport-neutral client for remote pi sessions.

use pi_protocol::{
    encode_client_message, parse_server_message, ClientMessageDecoder, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("{0}")]
    Message(String),
}

pub enum Transport {
    Memory { inbound: Vec<u8>, outbound: Vec<u8> },
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Transport {
    pub fn connect_unix(path: impl AsRef<std::path::Path>) -> Result<Self, ClientError> {
        UnixStream::connect(path)
            .map(Self::Unix)
            .map_err(|e| ClientError::Message(e.to_string()))
    }

    pub fn connect_tcp(addr: impl ToSocketAddrs) -> Result<Self, ClientError> {
        TcpStream::connect(addr)
            .map(Self::Tcp)
            .map_err(|e| ClientError::Message(e.to_string()))
    }

    pub fn memory() -> Self {
        Self::Memory {
            inbound: Vec::new(),
            outbound: Vec::new(),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ClientError> {
        match self {
            Self::Unix(s) => s.set_read_timeout(timeout),
            Self::Tcp(s) => s.set_read_timeout(timeout),
            Self::Memory { .. } => return Ok(()),
        }
        .map_err(|e| ClientError::Message(e.to_string()))
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        match self {
            Self::Memory { outbound, .. } => {
                outbound.extend_from_slice(bytes);
                Ok(())
            }
            Self::Unix(s) => s
                .write_all(bytes)
                .map_err(|e| ClientError::Message(e.to_string())),
            Self::Tcp(s) => s
                .write_all(bytes)
                .map_err(|e| ClientError::Message(e.to_string())),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ClientError> {
        match self {
            Self::Memory { inbound, .. } => {
                let n = inbound.len().min(buf.len());
                buf[..n].copy_from_slice(&inbound[..n]);
                inbound.drain(..n);
                Ok(n)
            }
            Self::Unix(s) => s.read(buf).map_err(|e| ClientError::Message(e.to_string())),
            Self::Tcp(s) => s.read(buf).map_err(|e| ClientError::Message(e.to_string())),
        }
    }
}

pub struct Client {
    pub transport: Transport,
    pub handshake_timeout: Duration,
}

impl Client {
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            handshake_timeout: Duration::from_secs(10),
        }
    }

    pub fn hello(&mut self) -> Result<Value, ClientError> {
        let hello = json!({ "type": "hello", "version": PROTOCOL_VERSION });
        let frame = encode_client_message(&hello, None).map_err(|e| ClientError::Message(e.0))?;
        self.transport
            .set_read_timeout(Some(self.handshake_timeout))?;
        self.transport.write_all(&frame)?;
        let mut buf = [0u8; 8192];
        let n = self.transport.read(&mut buf).map_err(|e| {
            if e.to_string().contains("timed out") {
                ClientError::HandshakeTimeout
            } else {
                e
            }
        })?;
        if n == 0 {
            return Err(ClientError::HandshakeTimeout);
        }
        let decoder = ClientMessageDecoder::new(None).map_err(|e| ClientError::Message(e.0))?;
        // Server hello is a server message; decode framed CBOR via protocol helper.
        let decoded = pi_protocol::decode_cbor(
            {
                let frames = pi_protocol::FrameDecoder::new(None)
                    .map_err(|e| ClientError::Message(e.0))?
                    .push(&buf[..n])
                    .map_err(|e| ClientError::Message(e.0))?;
                frames.into_iter().next().unwrap_or_default()
            }
            .as_slice(),
            None,
        )
        .map_err(|e| ClientError::Message(e.0))?;
        let _ = decoder;
        parse_server_message(&decoded.to_json()).map_err(|e| ClientError::Message(e.0))
    }

    pub fn request(&mut self, command: Value) -> Result<String, ClientError> {
        let id = Uuid::new_v4().to_string();
        let envelope = json!({
            "type": "request",
            "id": id,
            "request": command
        });
        let frame =
            encode_client_message(&envelope, None).map_err(|e| ClientError::Message(e.0))?;
        self.transport.write_all(&frame)?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_transport_writes_hello_frame() {
        let mut client = Client::new(Transport::memory());
        let hello = json!({ "type": "hello", "version": 1 });
        let frame = encode_client_message(&hello, None).unwrap();
        client.transport.write_all(&frame).unwrap();
        match &client.transport {
            Transport::Memory { outbound, .. } => assert_eq!(outbound, &frame),
            _ => unreachable!(),
        }
    }
}
