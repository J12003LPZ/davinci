//! Unix listener matching `vendor/pi/packages/server/src/transports/unix/listener.ts`.

use std::fs::{self, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pi_protocol::DEFAULT_MAX_FRAME_LENGTH;
use sha2::{Digest, Sha256};

use crate::ServerError;

pub const DEFAULT_SOCKET_MODE: u32 = 0o600;
pub const DEFAULT_GRACEFUL_CLOSE_TIMEOUT_MS: u64 = 5_000;
pub const MAX_UINT32: u64 = 0xFFFF_FFFF;
pub const MAX_TIMER_DELAY_MS: u64 = 2_147_483_647;

pub fn max_unix_socket_path_bytes() -> usize {
    if cfg!(target_os = "linux") {
        107
    } else {
        103
    }
}

pub fn validate_unix_socket_path(path: &str, description: &str) -> Result<(), ServerError> {
    if path.is_empty() {
        return Err(ServerError::Io(format!("{description} must not be empty")));
    }
    let max = max_unix_socket_path_bytes();
    if path.len() > max {
        return Err(ServerError::Io(format!(
            "{description} is too long; maximum is {max} UTF-8 bytes"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct UnixListenerOptions {
    pub path: String,
    pub mode: u32,
    pub graceful_close_timeout_ms: u64,
    pub max_pending_bytes: u64,
}

impl UnixListenerOptions {
    pub fn new(path: impl Into<String>) -> Result<Self, ServerError> {
        resolve_unix_listener_options(UnixListenerOptionsBuilder {
            path: path.into(),
            mode: None,
            graceful_close_timeout_ms: None,
            max_frame_length: None,
            max_pending_bytes: None,
        })
    }
}

pub struct UnixListenerOptionsBuilder {
    pub path: String,
    pub mode: Option<u32>,
    pub graceful_close_timeout_ms: Option<u64>,
    pub max_frame_length: Option<u64>,
    pub max_pending_bytes: Option<u64>,
}

pub fn resolve_unix_listener_options(
    options: UnixListenerOptionsBuilder,
) -> Result<UnixListenerOptions, ServerError> {
    validate_unix_socket_path(&options.path, "PiServer Unix socket path")?;
    let mode = options.mode.unwrap_or(DEFAULT_SOCKET_MODE);
    if mode > 0o777 {
        return Err(ServerError::Io(
            "PiServer Unix socket mode must be an integer between 0 and 0o777".into(),
        ));
    }
    let max_frame_length = options
        .max_frame_length
        .unwrap_or(u64::from(DEFAULT_MAX_FRAME_LENGTH));
    if max_frame_length == 0 || max_frame_length > MAX_UINT32 {
        return Err(ServerError::Io(format!(
            "PiServer maxFrameLength must be an integer between 1 and {MAX_UINT32}"
        )));
    }
    let max_pending_bytes = options
        .max_pending_bytes
        .unwrap_or(max_frame_length.saturating_mul(4));
    if max_pending_bytes < max_frame_length + 4 {
        return Err(ServerError::Io(
            "PiServer maxPendingBytes must be a safe integer at least maxFrameLength + 4".into(),
        ));
    }
    let graceful_close_timeout_ms = options
        .graceful_close_timeout_ms
        .unwrap_or(DEFAULT_GRACEFUL_CLOSE_TIMEOUT_MS);
    if graceful_close_timeout_ms == 0 || graceful_close_timeout_ms > MAX_TIMER_DELAY_MS {
        return Err(ServerError::Io(format!(
            "PiServer gracefulCloseTimeoutMs must be an integer between 1 and {MAX_TIMER_DELAY_MS}"
        )));
    }
    Ok(UnixListenerOptions {
        path: options.path,
        mode,
        graceful_close_timeout_ms,
        max_pending_bytes,
    })
}

pub fn owned_bind_path(path: &str) -> PathBuf {
    let suffix = hex8(&Sha256::digest(path.as_bytes()));
    match Path::new(path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(format!(".p-{suffix}")),
        _ => PathBuf::from(format!(".p-{suffix}")),
    }
}

pub struct BoundUnixListener {
    pub listener: UnixListener,
    pub path: PathBuf,
    pub owned_bind_path: PathBuf,
    pub mode: u32,
    pub max_pending_bytes: u64,
    pub graceful_close_timeout_ms: u64,
}

pub fn bind_unix(path: &str) -> Result<BoundUnixListener, ServerError> {
    bind_unix_with(UnixListenerOptions::new(path)?)
}

pub fn bind_unix_with(options: UnixListenerOptions) -> Result<BoundUnixListener, ServerError> {
    let public_path = PathBuf::from(&options.path);
    let owned = owned_bind_path(&options.path);
    let owned_str = owned.to_string_lossy().into_owned();
    validate_unix_socket_path(&owned_str, "PiServer private Unix bind path")?;
    if let Some(parent) = public_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| ServerError::Io(err.to_string()))?;
            let _ = fs::set_permissions(parent, Permissions::from_mode(0o700));
        }
    }
    remove_stale_socket(&public_path)?;
    remove_stale_socket(&owned)?;
    let listener = UnixListener::bind(&owned).map_err(|err| ServerError::Io(err.to_string()))?;
    if owned.exists() {
        fs::hard_link(&owned, &public_path).map_err(|err| ServerError::Io(err.to_string()))?;
        let _ = fs::set_permissions(&public_path, Permissions::from_mode(options.mode));
    }
    Ok(BoundUnixListener {
        listener,
        path: public_path,
        owned_bind_path: owned,
        mode: options.mode,
        max_pending_bytes: options.max_pending_bytes,
        graceful_close_timeout_ms: options.graceful_close_timeout_ms,
    })
}

impl BoundUnixListener {
    pub fn accept(&self) -> Result<UnixStream, ServerError> {
        self.listener
            .accept()
            .map(|(stream, _)| stream)
            .map_err(|err| ServerError::Io(err.to_string()))
    }
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).fold(String::new(), |mut out, byte| {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn remove_stale_socket(path: &Path) -> Result<(), ServerError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ServerError::Io(err.to_string())),
    };
    if !metadata.file_type().is_socket() {
        return Err(ServerError::Io(format!(
            "Refusing to remove non-socket Unix listener path: {}",
            path.display()
        )));
    }
    if is_socket_live(path) {
        return Err(ServerError::Io(format!(
            "Unix listener is already running: {}",
            path.display()
        )));
    }
    fs::remove_file(path).or_else(|err| {
        if err.kind() == ErrorKind::NotFound {
            Ok(())
        } else {
            Err(ServerError::Io(err.to_string()))
        }
    })
}

fn is_socket_live(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => true,
        Err(err) => !matches!(
            err.kind(),
            ErrorKind::ConnectionRefused | ErrorKind::NotFound | ErrorKind::BrokenPipe
        ),
    }
}

pub struct UnixByteConnection {
    stream: Option<UnixStream>,
    graceful_close_timeout_ms: u64,
    max_pending_bytes: u64,
    pending_bytes: u64,
    closed: bool,
    closing: bool,
}

impl UnixByteConnection {
    pub fn new(stream: UnixStream, graceful_close_timeout_ms: u64, max_pending_bytes: u64) -> Self {
        Self {
            stream: Some(stream),
            graceful_close_timeout_ms,
            max_pending_bytes,
            pending_bytes: 0,
            closed: false,
            closing: false,
        }
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn send(&mut self, chunk: &[u8]) -> Result<(), ServerError> {
        if self.closed || self.closing {
            return Err(ServerError::Io("Unix connection is closed".into()));
        }
        if self.pending_bytes + chunk.len() as u64 > self.max_pending_bytes {
            return Err(ServerError::Io(
                "Unix connection exceeded its pending byte limit".into(),
            ));
        }
        self.pending_bytes += chunk.len() as u64;
        let result = self.write_all(chunk);
        self.pending_bytes = self.pending_bytes.saturating_sub(chunk.len() as u64);
        result
    }

    pub fn close(&mut self, final_chunk: Option<&[u8]>) -> Result<(), ServerError> {
        if self.closed || self.stream.is_none() {
            self.mark_closed();
            return Ok(());
        }
        if self.closing {
            return Ok(());
        }
        self.closing = true;
        let started = Instant::now();
        if let Some(chunk) = final_chunk {
            if let Err(error) = self.write_all(chunk) {
                self.mark_closed();
                return Err(error);
            }
        }
        if started.elapsed() > Duration::from_millis(self.graceful_close_timeout_ms) {
            self.mark_closed();
            return Ok(());
        }
        self.mark_closed();
        Ok(())
    }

    pub fn mark_closed(&mut self) {
        self.closed = true;
        self.closing = true;
        self.stream.take();
    }

    fn write_all(&mut self, chunk: &[u8]) -> Result<(), ServerError> {
        let Some(stream) = self.stream.as_mut() else {
            return Err(ServerError::Io("Unix connection is closed".into()));
        };
        stream.write_all(chunk).map_err(|err| {
            if err.kind() == ErrorKind::BrokenPipe
                || err.kind() == ErrorKind::ConnectionReset
                || err.kind() == ErrorKind::NotConnected
            {
                ServerError::Io("Unix connection closed during write".into())
            } else {
                ServerError::Io(err.to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    #[test]
    fn unix_byte_connection_send_close_and_pending_limit_match_ts() {
        let (left, mut right) = UnixStream::pair().unwrap();
        let mut conn = UnixByteConnection::new(left, 5_000, 8);
        conn.send(b"hello").unwrap();
        let mut buf = [0u8; 5];
        right.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        match conn.send(b"too-large") {
            Err(err) => assert_eq!(
                err.to_string(),
                "Unix connection exceeded its pending byte limit"
            ),
            Ok(()) => panic!("expected pending byte limit"),
        }
        conn.close(Some(b"!")).unwrap();
        match conn.send(b"x") {
            Err(err) => assert_eq!(err.to_string(), "Unix connection is closed"),
            Ok(()) => panic!("expected closed"),
        }
    }
}
