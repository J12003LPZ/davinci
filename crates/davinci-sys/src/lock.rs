//! A cross-process lock held by owning a `<target>.lock` file.
//!
//! `create_new` is atomic on every platform we ship. A holder that crashed
//! leaves the file behind, so a lock whose mtime is older than
//! `stale_after` is treated as abandoned and taken over.
//!
//! Known window: two waiters that see the same stale lock within ~20 ms can
//! both remove it. Holders keep locks for milliseconds and the default stale
//! threshold is 10 s, so this needs a crashed holder plus two simultaneous
//! waiters. proper-lockfile (used by TypeScript pi) has the same window.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// proper-lockfile's default, so TS pi and davinci agree on staleness.
pub const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(10);

const RETRY_EVERY: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub struct LockFile {
    path: PathBuf,
    token: String,
}

impl LockFile {
    pub fn acquire(path: &Path, wait: Duration, stale_after: Duration) -> io::Result<Self> {
        let started = Instant::now();
        let token = uuid::Uuid::new_v4().to_string();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    writeln!(file, "{} {token}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                        token,
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if is_stale(path, stale_after) {
                        // Never unlink the stale path in-place. An atomic rename
                        // makes exactly one contender the takeover winner; other
                        // contenders either observe the replacement lock or retry.
                        let stale = path.with_extension(format!(
                            "stale.{}.{}",
                            std::process::id(),
                            uuid::Uuid::new_v4()
                        ));
                        match fs::rename(path, &stale) {
                            Ok(()) => {
                                let _ = fs::remove_file(stale);
                                continue;
                            }
                            Err(rename_err) if rename_err.kind() == io::ErrorKind::NotFound => {
                                continue;
                            }
                            Err(_) => {
                                // A failed stale takeover must obey the same
                                // bounded wait as an ordinary held lock.
                            }
                        }
                    }
                    if started.elapsed() >= wait {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            format!("{} is held by another process", path.display()),
                        ));
                    }
                    std::thread::sleep(RETRY_EVERY);
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        // A stale owner may wake up after another process has taken over.
        // Remove only the path that still carries this guard's unique token.
        let owned = fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|text| text.split_whitespace().nth(1) == Some(self.token.as_str()));
        if owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn lock_path_for(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    target.with_file_name(name)
}

fn is_stale(path: &Path, stale_after: Duration) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age >= stale_after)
        .unwrap_or(false)
}

/// A lock held by the operating system until its owner exits, crash
/// included: `flock(LOCK_EX)` on Unix and an open with no sharing on Windows.
/// Keep this file in place after unlocking; deleting a locked path could let
/// another process lock a different inode at the same name.
#[derive(Debug)]
pub struct ExclusiveFileLock {
    #[allow(dead_code)] // held for its lifetime; Drop releases the OS lock
    file: fs::File,
}

impl ExclusiveFileLock {
    pub fn try_acquire(path: &Path) -> io::Result<Self> {
        #[cfg(any(unix, windows))]
        {
            Self::try_acquire_supported(path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "exclusive file locks are unsupported on this platform",
            ))
        }
    }

    #[cfg(any(unix, windows))]
    fn try_acquire_supported(path: &Path) -> io::Result<Self> {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not an ordinary file", path.display()),
                ));
            }
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // share_mode(0): a second open fails until this handle is dropped.
            options.share_mode(0).custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }

        let file = match options.open(path) {
            Ok(file) => file,
            // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33).
            #[cfg(windows)]
            Err(err) if matches!(err.raw_os_error(), Some(32 | 33)) => return Err(held(path)),
            Err(err) => return Err(err),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not an ordinary file", path.display()),
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is a reparse point", path.display()),
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            loop {
                // SAFETY: `file` owns this live descriptor for the whole call.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    break;
                }
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(if err.kind() == io::ErrorKind::WouldBlock {
                    held(path)
                } else {
                    err
                });
            }
        }
        Ok(Self { file })
    }

    pub fn acquire(path: &Path, wait: Duration) -> io::Result<Self> {
        let started = Instant::now();
        loop {
            match Self::try_acquire(path) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock && started.elapsed() < wait => {
                    std::thread::sleep(RETRY_EVERY);
                }
                other => return other,
            }
        }
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Closing alone may leave a forked child holding the description.
            // SAFETY: `self.file` still owns the descriptor here.
            unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn held(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!("{} is held by another process", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn second_acquire_waits_then_fails_while_held() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json.lock");
        let _held = LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap();
        let started = Instant::now();
        let err = LockFile::acquire(&path, Duration::from_millis(100), DEFAULT_STALE_AFTER)
            .expect_err("lock is held");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn drop_releases_the_lock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.lock");
        drop(LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap());
        assert!(!path.exists());
        LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap();
    }

    #[test]
    fn stale_takeover_does_not_remove_the_new_owners_lock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let old = LockFile::acquire(&path, Duration::ZERO, Duration::from_secs(10)).unwrap();

        // Simulate a stale-path takeover while the old guard is still alive.
        let displaced = dir.path().join("displaced.lock");
        std::fs::rename(&path, &displaced).unwrap();
        let new = LockFile::acquire(&path, Duration::ZERO, Duration::from_secs(10)).unwrap();

        drop(old);
        assert!(
            path.exists(),
            "old owner must not delete the replacement lock"
        );
        drop(new);
        assert!(!path.exists());
    }

    #[test]
    fn stale_lock_left_by_a_crash_is_taken_over() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, "12345\n").unwrap();
        // Zero stale_after: any existing lock counts as abandoned.
        let lock = LockFile::acquire(&path, Duration::ZERO, Duration::ZERO).unwrap();
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn lock_path_appends_dot_lock() {
        assert_eq!(
            lock_path_for(std::path::Path::new("/a/settings.json")),
            std::path::PathBuf::from("/a/settings.json.lock")
        );
    }

    #[test]
    fn exclusive_lock_blocks_a_second_holder_until_dropped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl.lock");
        let first = ExclusiveFileLock::try_acquire(&path).unwrap();
        let err = ExclusiveFileLock::try_acquire(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        drop(first);
        assert!(path.is_file(), "exclusive lock file must remain in place");
        ExclusiveFileLock::try_acquire(&path).unwrap();
    }

    #[test]
    fn exclusive_lock_acquire_waits_then_gives_up() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json.lock");
        let _held = ExclusiveFileLock::try_acquire(&path).unwrap();
        let started = Instant::now();
        let err = ExclusiveFileLock::acquire(&path, Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_lock_refuses_a_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "").unwrap();
        let link = dir.path().join("x.lock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = ExclusiveFileLock::try_acquire(&link).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
