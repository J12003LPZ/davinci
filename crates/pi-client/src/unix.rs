//! Unix-domain `ByteTransport` matching `vendor/pi/packages/client/src/unix.ts`.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use pi_protocol::DEFAULT_MAX_FRAME_LENGTH;

use crate::connection::{ByteTransport, ByteTransportHandlers, TransportFactory};
use crate::ClientError;

pub fn max_unix_socket_path_bytes() -> usize {
    if cfg!(target_os = "linux") {
        107
    } else {
        103
    }
}

#[derive(Debug, Clone)]
pub struct UnixTransportOptions {
    pub path: String,
    pub max_pending_bytes: Option<u64>,
}

struct QueuedWrite {
    bytes: Vec<u8>,
    offset: usize,
}

/// TS `UnixByteTransport`.
pub struct UnixByteTransport {
    stream: UnixStream,
    max_pending_bytes: u64,
    pending_bytes: u64,
    closed: bool,
    queue: VecDeque<QueuedWrite>,
    handlers: ByteTransportHandlers,
}

impl UnixByteTransport {
    fn attach(stream: UnixStream, max_pending_bytes: u64, handlers: ByteTransportHandlers) -> Self {
        let _ = stream.set_nonblocking(true);
        Self {
            stream,
            max_pending_bytes,
            pending_bytes: 0,
            closed: false,
            queue: VecDeque::new(),
            handlers,
        }
    }

    pub fn pending_bytes(&self) -> u64 {
        self.pending_bytes
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    /// TS `send`: enqueue immediately, keep `pendingBytes` until the write finishes.
    pub fn send_chunk(&mut self, chunk: &[u8]) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::Io("Unix transport is closed".into()));
        }
        if self.pending_bytes + chunk.len() as u64 > self.max_pending_bytes {
            return Err(ClientError::Io(
                "Unix transport exceeded its pending byte limit".into(),
            ));
        }
        self.pending_bytes += chunk.len() as u64;
        self.queue.push_back(QueuedWrite {
            bytes: chunk.to_vec(),
            offset: 0,
        });
        self.flush_nonblocking()
    }

    pub fn flush_nonblocking(&mut self) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::Io("Unix transport is closed".into()));
        }
        let _ = self.stream.set_nonblocking(true);
        while let Some(front) = self.queue.front_mut() {
            match self.stream.write(&front.bytes[front.offset..]) {
                Ok(0) => {
                    return Err(self.fail_closed_during_write());
                }
                Ok(n) => {
                    front.offset += n;
                    if front.offset >= front.bytes.len() {
                        let total = front.bytes.len() as u64;
                        self.queue.pop_front();
                        self.pending_bytes = self.pending_bytes.saturating_sub(total);
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionReset
                            | ErrorKind::NotConnected
                    ) =>
                {
                    return Err(self.fail_closed_during_write());
                }
                Err(err) => return Err(ClientError::Io(err.to_string())),
            }
        }
        Ok(())
    }

    pub fn wait_idle(&mut self, timeout: Duration) -> Result<(), ClientError> {
        let started = Instant::now();
        loop {
            self.flush_nonblocking()?;
            if self.queue.is_empty() {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Err(ClientError::Io("Unix transport write timed out".into()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn pump_inbound(&mut self) -> Result<bool, ClientError> {
        if self.closed {
            return Ok(false);
        }
        let _ = self.stream.set_nonblocking(true);
        let mut buf = [0u8; 8192];
        let mut got = false;
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    self.mark_remote_close();
                    return Ok(got);
                }
                Ok(n) => {
                    got = true;
                    (self.handlers.on_data)(&buf[..n]);
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(got),
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => {
                    let error = ClientError::Io(err.to_string());
                    (self.handlers.on_error)(error.clone());
                    return Err(error);
                }
            }
        }
    }

    pub fn pump_until_data(&mut self, timeout: Duration) -> Result<(), ClientError> {
        let started = Instant::now();
        let mut last_data = Instant::now();
        let mut got_any = false;
        loop {
            if self.pump_inbound()? {
                got_any = true;
                last_data = Instant::now();
            }
            if self.closed {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Ok(());
            }
            if got_any && last_data.elapsed() > Duration::from_millis(20) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn fail_closed_during_write(&mut self) -> ClientError {
        self.mark_remote_close();
        ClientError::Io("Unix transport closed during write".into())
    }

    fn mark_remote_close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        (self.handlers.on_close)();
    }
}

impl ByteTransport for UnixByteTransport {
    fn send(&mut self, chunk: &[u8]) -> Result<(), ClientError> {
        self.send_chunk(chunk)?;
        self.wait_idle(Duration::from_secs(5))?;
        self.pump_until_data(Duration::from_secs(2))?;
        Ok(())
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

pub fn create_unix_transport_factory(
    options: UnixTransportOptions,
) -> Result<TransportFactory, ClientError> {
    let max_pending_bytes = resolve_unix_transport_options(&options)?;
    let path = options.path;
    Ok(Box::new(move |handlers| {
        connect_unix_transport(&path, max_pending_bytes, handlers)
            .map(|transport| Box::new(transport) as Box<dyn ByteTransport>)
    }))
}

pub fn connect_unix_transport(
    path: &str,
    max_pending_bytes: u64,
    handlers: ByteTransportHandlers,
) -> Result<UnixByteTransport, ClientError> {
    let stream = UnixStream::connect(path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            ClientError::Io(format!("ENOENT: {err}"))
        } else if err.kind() == ErrorKind::ConnectionRefused {
            ClientError::Io(format!("Unix transport closed before connecting: {err}"))
        } else {
            ClientError::Io(err.to_string())
        }
    })?;
    Ok(UnixByteTransport::attach(
        stream,
        max_pending_bytes,
        handlers,
    ))
}

pub fn resolve_unix_transport_options(options: &UnixTransportOptions) -> Result<u64, ClientError> {
    if options.path.is_empty() {
        return Err(ClientError::Protocol(
            "Unix transport path must not be empty".into(),
        ));
    }
    let max = max_unix_socket_path_bytes();
    if options.path.len() > max {
        return Err(ClientError::Protocol(format!(
            "Unix transport path is too long; maximum is {max} UTF-8 bytes"
        )));
    }
    if cfg!(windows) {
        return Err(ClientError::Io(
            "Unix transport is not supported on Windows".into(),
        ));
    }
    let max_pending_bytes = options
        .max_pending_bytes
        .unwrap_or(u64::from(DEFAULT_MAX_FRAME_LENGTH).saturating_mul(4));
    if max_pending_bytes == 0 {
        return Err(ClientError::Protocol(
            "Unix transport maxPendingBytes must be a positive safe integer".into(),
        ));
    }
    Ok(max_pending_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Connection;
    use pi_protocol::{
        encode_server_message, ClientMessage, ClientMessageDecoder, CommandResult, ServerMessage,
        ServerSnapshot, PROTOCOL_VERSION,
    };
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    fn empty_handlers() -> ByteTransportHandlers {
        ByteTransportHandlers {
            on_data: Box::new(|_| {}),
            on_close: Box::new(|| {}),
            on_error: Box::new(|_| {}),
        }
    }

    fn server_snapshot() -> ServerSnapshot {
        ServerSnapshot {
            server_id: "unix-server".into(),
            protocol_version: PROTOCOL_VERSION,
            revision: 4,
            sessions: Vec::new(),
            models: Vec::new(),
        }
    }

    fn factory_err(options: UnixTransportOptions) -> String {
        match create_unix_transport_factory(options) {
            Ok(_) => panic!("expected invalid Unix transport options"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn rejects_invalid_unix_transport_options() {
        let empty = factory_err(UnixTransportOptions {
            path: String::new(),
            max_pending_bytes: None,
        });
        assert!(empty.contains("must not be empty"));
        let long = factory_err(UnixTransportOptions {
            path: format!("/tmp/{}", "x".repeat(512)),
            max_pending_bytes: None,
        });
        assert!(long.contains("too long"));
        let zero = factory_err(UnixTransportOptions {
            path: "/tmp/pi.sock".into(),
            max_pending_bytes: Some(0),
        });
        assert!(zero.contains("positive"));
    }

    #[test]
    fn rejects_missing_socket_as_enoent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sock");
        let err = match connect_unix_transport(path.to_str().unwrap(), 1024, empty_handlers()) {
            Ok(_) => panic!("expected missing Unix socket"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("ENOENT"), "{err}");
    }

    #[test]
    fn bounds_pending_writes_preserves_order_and_reports_remote_end_once() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("pi.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let first = vec![1u8; 2 * 1024 * 1024];
        let second = vec![2u8; 2 * 1024 * 1024];
        let expected_length = first.len() + second.len();
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = ready_tx.send(());
            resume_rx.recv().unwrap();
            stream.set_nonblocking(true).unwrap();
            let started = Instant::now();
            while received_clone.lock().unwrap().len() < expected_length {
                let mut buf = [0u8; 65536];
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => received_clone.lock().unwrap().extend_from_slice(&buf[..n]),
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        if started.elapsed() > Duration::from_secs(5) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
            let _ = stream.write_all(&[9]);
            let _ = stream.shutdown(std::net::Shutdown::Write);
        });

        let inbound = Arc::new(Mutex::new(Vec::new()));
        let close_count = Arc::new(Mutex::new(0usize));
        let inbound_h = inbound.clone();
        let close_h = close_count.clone();
        let mut transport = connect_unix_transport(
            socket_path.to_str().unwrap(),
            expected_length as u64,
            ByteTransportHandlers {
                on_data: Box::new(move |chunk| inbound_h.lock().unwrap().extend_from_slice(chunk)),
                on_close: Box::new(move || *close_h.lock().unwrap() += 1),
                on_error: Box::new(|_| {}),
            },
        )
        .unwrap();

        ready_rx.recv().unwrap();
        transport.send_chunk(&first).unwrap();
        transport.send_chunk(&second).unwrap();
        match transport.send_chunk(&[3]) {
            Err(err) => assert!(err.to_string().contains("pending byte limit"), "{}", err),
            Ok(()) => panic!("expected pending byte limit"),
        }
        resume_tx.send(()).unwrap();
        transport
            .wait_idle(Duration::from_secs(5))
            .expect("queued writes should drain after the peer reads");
        transport
            .pump_until_data(Duration::from_secs(2))
            .expect("peer close byte");
        server.join().unwrap();
        let got = received.lock().unwrap().clone();
        assert_eq!(got.len(), expected_length);
        assert!(got[..first.len()].iter().all(|b| *b == 1));
        assert!(got[first.len()..].iter().all(|b| *b == 2));
        assert_eq!(*inbound.lock().unwrap(), vec![9]);
        assert_eq!(*close_count.lock().unwrap(), 1);
        transport.close();
    }

    #[test]
    fn pi_client_exchanges_fragmented_framed_messages_over_unix() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("pi.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let snapshot = server_snapshot();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut decoder = ClientMessageDecoder::new(None).unwrap();
            let mut buf = [0u8; 4096];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                let messages = decoder.push(&buf[..n]).unwrap();
                for message in messages {
                    match message {
                        ClientMessage::Hello { version } => {
                            let hello = encode_server_message(
                                &ServerMessage::Hello {
                                    version,
                                    connection_id: "unix-connection".into(),
                                    snapshot: snapshot.clone(),
                                },
                                None,
                            )
                            .unwrap();
                            for byte in hello {
                                stream.write_all(&[byte]).unwrap();
                            }
                        }
                        ClientMessage::Request { id, .. } => {
                            let response = encode_server_message(
                                &ServerMessage::Response {
                                    id,
                                    ok: true,
                                    result: Some(CommandResult::List {
                                        sessions: Vec::new(),
                                    }),
                                    error: None,
                                },
                                None,
                            )
                            .unwrap();
                            let split = response.len() / 2;
                            stream.write_all(&response[..split]).unwrap();
                            stream.write_all(&response[split..]).unwrap();
                        }
                    }
                }
            }
        });

        let mut factory = create_unix_transport_factory(UnixTransportOptions {
            path: socket_path.to_string_lossy().into_owned(),
            max_pending_bytes: None,
        })
        .unwrap();
        let connection = Connection::new(None).unwrap();
        let snapshot = connection.connect(&mut factory).unwrap();
        assert_eq!(snapshot.server_id, "unix-server");
        connection.disconnect("done");
        drop(connection);
        let _ = server.join();
    }
}
