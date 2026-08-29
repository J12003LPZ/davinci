//! Unix-domain socket listener for [`PiServer`].
//!
//! Framed CBOR, no protocol-level auth. Stale socket files are unlinked before
//! bind; a live listener at the same path is left untouched and reported as an
//! error.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pi_protocol::{
    encode_server_message, ClientMessageDecoder, ProtocolError, ProtocolErrorCode, ServerMessage,
};
use uuid::Uuid;

use crate::PiServer;

/// Default time a new connection may sit idle before hello.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 5_000;

const SOCKET_MODE: u32 = 0o600;
const ACCEPT_POLL_MS: u64 = 5;

#[derive(Debug, Clone)]
pub struct UnixListenerOptions {
    pub handshake_timeout_ms: u64,
}

impl Default for UnixListenerOptions {
    fn default() -> Self {
        Self {
            handshake_timeout_ms: DEFAULT_HANDSHAKE_TIMEOUT_MS,
        }
    }
}

/// Running Unix listener. Dropping it stops accepts and unlinks the socket path.
pub struct UnixServer {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for UnixServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnixServer")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl UnixServer {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UnixServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Bind `server` on `path`, accept connections, and reply with framed `ServerMessage`s.
pub fn listen_unix(
    server: PiServer,
    path: impl AsRef<Path>,
    options: UnixListenerOptions,
) -> Result<UnixServer, String> {
    let path = path.as_ref().to_path_buf();
    if path.as_os_str().is_empty() {
        return Err("Unix socket path must not be empty".into());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    prepare_bind_path(&path)?;
    let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(SOCKET_MODE));
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;

    let handshake_timeout_ms = if options.handshake_timeout_ms == 0 {
        DEFAULT_HANDSHAKE_TIMEOUT_MS
    } else {
        options.handshake_timeout_ms
    };
    let handshake_timeout = Duration::from_millis(handshake_timeout_ms);
    let server = Arc::new(server);
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_thread = Arc::clone(&shutdown);
    let join = thread::spawn(move || {
        accept_loop(listener, server, handshake_timeout, shutdown_thread);
    });

    Ok(UnixServer {
        path,
        shutdown,
        join: Some(join),
    })
}

fn prepare_bind_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_socket() {
        return Err(format!(
            "Refusing to remove non-socket Unix listener path: {}",
            path.display()
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(format!(
            "Unix listener is already running: {}",
            path.display()
        )),
        Err(_) => fs::remove_file(path).map_err(|error| error.to_string()),
    }
}

fn accept_loop(
    listener: UnixListener,
    server: Arc<PiServer>,
    handshake_timeout: Duration,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let server = Arc::clone(&server);
                thread::spawn(move || handle_connection(stream, server, handshake_timeout));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(ACCEPT_POLL_MS));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(ACCEPT_POLL_MS));
            }
        }
    }
}

fn handle_connection(mut stream: UnixStream, server: Arc<PiServer>, handshake_timeout: Duration) {
    let connection_id = Uuid::now_v7().to_string();
    let mut decoder = ClientMessageDecoder::new();
    let mut handshake_done = false;
    let deadline = Instant::now() + handshake_timeout;
    let mut buf = [0u8; 8192];

    loop {
        if !handshake_done {
            let now = Instant::now();
            if now >= deadline {
                send_handshake_timeout(&mut stream);
                return;
            }
            if stream.set_read_timeout(Some(deadline - now)).is_err() {
                return;
            }
        }

        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let messages = match decoder.push(&buf[..n]) {
                    Ok(messages) => messages,
                    Err(_) => return,
                };
                for message in messages {
                    if !handshake_done {
                        handshake_done = true;
                        let _ = stream.set_read_timeout(None);
                    }
                    for reply in server.handle(&connection_id, message) {
                        match encode_server_message(&reply) {
                            Ok(bytes) => {
                                if stream.write_all(&bytes).is_err() {
                                    return;
                                }
                                let _ = stream.flush();
                            }
                            Err(_) => return,
                        }
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if !handshake_done {
                    send_handshake_timeout(&mut stream);
                }
                return;
            }
            Err(_) => return,
        }
    }
}

fn send_handshake_timeout(stream: &mut UnixStream) {
    let message = ServerMessage::HelloError {
        error: ProtocolError {
            code: ProtocolErrorCode::InvalidRequest,
            message: "Handshake timeout".into(),
            details: None,
        },
    };
    if let Ok(bytes) = encode_server_message(&message) {
        let _ = stream.write_all(&bytes);
        let _ = stream.flush();
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}
