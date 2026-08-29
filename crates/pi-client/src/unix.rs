//! Unix-domain socket transport for [`PiClient`].
//!
//! Connects to a live [`pi_server`] Unix listener and exchanges framed CBOR
//! messages. There is no protocol-level authentication.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use pi_protocol::{encode_client_message, ClientMessage, ServerMessage, ServerMessageDecoder};

/// Connected Unix-domain socket that reads and writes framed protocol messages.
pub struct UnixTransport {
    stream: UnixStream,
    decoder: ServerMessageDecoder,
    pending: VecDeque<ServerMessage>,
}

impl UnixTransport {
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err("Unix transport path must not be empty".into());
        }
        let stream = UnixStream::connect(path).map_err(|error| error.to_string())?;
        Ok(Self {
            stream,
            decoder: ServerMessageDecoder::new(),
            pending: VecDeque::new(),
        })
    }

    pub fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        let bytes = encode_client_message(message).map_err(|error| error.to_string())?;
        self.stream
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        self.stream.flush().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<ServerMessage, String> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        let mut buf = [0u8; 8192];
        loop {
            let n = self
                .stream
                .read(&mut buf)
                .map_err(|error| error.to_string())?;
            if n == 0 {
                return Err("unix transport closed".into());
            }
            let mut messages = self
                .decoder
                .push(&buf[..n])
                .map_err(|error| error.to_string())?;
            if messages.is_empty() {
                continue;
            }
            let first = messages.remove(0);
            self.pending.extend(messages);
            return Ok(first);
        }
    }
}
