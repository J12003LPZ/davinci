//! Unix listener matching `vendor/pi/packages/server/src/transports/unix/listener.ts`.

use std::fs::{self, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
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
    listener: Option<UnixListener>,
    pub path: PathBuf,
    pub owned_bind_path: PathBuf,
    pub mode: u32,
    pub max_pending_bytes: u64,
    pub graceful_close_timeout_ms: u64,
    socket_identity: Option<(u64, u64)>,
    closed: bool,
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
    let stats = fs::symlink_metadata(&owned).map_err(|err| ServerError::Io(err.to_string()))?;
    if !stats.file_type().is_socket() {
        return Err(ServerError::Io(format!(
            "Unix listener path is not a socket after binding: {}",
            owned.display()
        )));
    }
    let socket_identity = Some((stats.dev(), stats.ino()));
    if owned.exists() {
        fs::hard_link(&owned, &public_path).map_err(|err| ServerError::Io(err.to_string()))?;
        let _ = fs::set_permissions(&public_path, Permissions::from_mode(options.mode));
    }
    Ok(BoundUnixListener {
        listener: Some(listener),
        path: public_path,
        owned_bind_path: owned,
        mode: options.mode,
        max_pending_bytes: options.max_pending_bytes,
        graceful_close_timeout_ms: options.graceful_close_timeout_ms,
        socket_identity,
        closed: false,
    })
}

impl BoundUnixListener {
    pub fn accept(&self) -> Result<UnixStream, ServerError> {
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| ServerError::Io("Unix listener is closing or closed".into()))?;
        listener
            .accept()
            .map(|(stream, _)| stream)
            .map_err(|err| ServerError::Io(err.to_string()))
    }

    pub fn close(&mut self) -> Result<(), ServerError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.listener.take();
        let cleanup = self.cleanup_owned_socket();
        let owned = remove_path(&self.owned_bind_path);
        cleanup.and(owned)
    }

    fn cleanup_owned_socket(&mut self) -> Result<(), ServerError> {
        let Some(identity) = self.socket_identity.take() else {
            return Ok(());
        };
        let current = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(ServerError::Io(err.to_string())),
        };
        if !current.file_type().is_socket()
            || current.dev() != identity.0
            || current.ino() != identity.1
        {
            return Ok(());
        }
        let preserved = sibling_temp_path(&self.path, ".c-");
        match fs::rename(&self.path, &preserved) {
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(ServerError::Io(err.to_string())),
            Ok(()) => {}
        }
        let moved =
            fs::symlink_metadata(&preserved).map_err(|err| ServerError::Io(err.to_string()))?;
        if moved.file_type().is_socket() && moved.dev() == identity.0 && moved.ino() == identity.1 {
            return remove_path(&preserved);
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs::rename(&preserved, &self.path)
                    .map_err(|err| ServerError::Io(err.to_string()))?;
            }
            Err(err) => return Err(ServerError::Io(err.to_string())),
        }
        Err(ServerError::Io(format!(
            "Unix listener path changed during cleanup; preserved replacement at {}",
            preserved.display()
        )))
    }
}

impl Drop for BoundUnixListener {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).fold(String::new(), |mut out, byte| {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn sibling_temp_path(path: &Path, prefix: &str) -> PathBuf {
    let suffix = uuid::Uuid::new_v4().to_string();
    let name = format!("{prefix}{}", &suffix[..6]);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

fn remove_path(path: &Path) -> Result<(), ServerError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(ServerError::Io(err.to_string())),
    }
}

fn remove_stale_socket(path: &Path) -> Result<(), ServerError> {
    let original = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ServerError::Io(err.to_string())),
    };
    if !original.file_type().is_socket() {
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
    let identity = (original.dev(), original.ino());
    let preserved = sibling_temp_path(path, ".s-");
    match fs::rename(path, &preserved) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ServerError::Io(err.to_string())),
        Ok(()) => {}
    }
    let current =
        fs::symlink_metadata(&preserved).map_err(|err| ServerError::Io(err.to_string()))?;
    if !current.file_type().is_socket()
        || current.dev() != identity.0
        || current.ino() != identity.1
    {
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs::rename(&preserved, path).map_err(|err| ServerError::Io(err.to_string()))?;
            }
            Err(err) => return Err(ServerError::Io(err.to_string())),
        }
        return Err(ServerError::Io(format!(
            "Unix listener path changed while checking for a stale socket: {}",
            path.display()
        )));
    }
    remove_path(&preserved)
}

fn is_socket_live(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => true,
        Err(err) => !matches!(
            err.kind(),
            ErrorKind::ConnectionRefused
                | ErrorKind::NotFound
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
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

    fn socket_identity(path: &Path) -> (u64, u64) {
        let metadata = fs::symlink_metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    #[test]
    fn rejects_a_live_listener_without_unlinking_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.sock");
        let path_str = path.to_string_lossy().into_owned();
        let first = bind_unix(&path_str).unwrap();
        let first_identity = socket_identity(&path);
        match bind_unix(&path_str) {
            Err(err) => assert!(err.to_string().contains("already running")),
            Ok(_) => panic!("expected already-running Unix listener"),
        }
        assert!(path.metadata().unwrap().file_type().is_socket());
        assert_eq!(socket_identity(&path), first_identity);
        drop(first);
    }

    #[test]
    fn never_unlinks_a_regular_file_at_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.sock");
        fs::write(&path, "do not remove").unwrap();
        match bind_unix(&path.to_string_lossy()) {
            Err(err) => assert!(err.to_string().contains("non-socket")),
            Ok(_) => panic!("expected non-socket refusal"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "do not remove");
    }

    #[test]
    fn creates_nested_parents_restricts_permissions_and_removes_its_own_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").join("n").join("server.sock");
        let path_str = path.to_string_lossy().into_owned();
        let mut bound = bind_unix(&path_str).unwrap();
        let stats = fs::symlink_metadata(&path).unwrap();
        assert!(stats.file_type().is_socket());
        assert_eq!(stats.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        bound.close().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn does_not_remove_a_replacement_inode_during_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.sock");
        let path_str = path.to_string_lossy().into_owned();
        let mut bound = bind_unix(&path_str).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, "replacement").unwrap();
        bound.close().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
    }

    #[test]
    fn removes_a_genuinely_stale_socket_before_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.sock");
        {
            let _dead = UnixListener::bind(&path).unwrap();
        }
        assert!(path.exists());
        let mut bound = bind_unix(&path.to_string_lossy()).unwrap();
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
        bound.close().unwrap();
        assert!(!path.exists());
    }
}
