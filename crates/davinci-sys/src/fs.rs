//! Crash-safe file publication. A reader sees the old file or the new one,
//! never a torn one: bytes go to a temporary file in the same directory, are
//! flushed, and replace the target with one rename.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

/// Publish `bytes` at `path`. Creates the parent directory if needed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, false)
}

/// Same as [`atomic_write`], and on Unix the file is created 0600 so only
/// its owner can read it. Use it for credentials and tokens.
pub fn atomic_write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, true)
}

/// Remove an incomplete final line from an append-only JSONL file.
///
/// Returns the number of bytes removed. A missing file is treated as empty.
/// The caller must exclude concurrent writers while scanning and truncating.
pub fn truncate_torn_tail(path: &Path) -> io::Result<u64> {
    const CHUNK_SIZE: u64 = 64 * 1024;

    let mut file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let original_len = file.metadata()?.len();
    if original_len == 0 {
        return Ok(0);
    }

    let mut buffer = [0_u8; CHUNK_SIZE as usize];
    let mut end = original_len;
    while end > 0 {
        let start = end.saturating_sub(CHUNK_SIZE);
        let chunk_len = usize::try_from(end - start).expect("chunk is bounded by CHUNK_SIZE");
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..chunk_len])?;

        if let Some(last_newline) = buffer[..chunk_len].iter().rposition(|byte| *byte == b'\n') {
            let complete_len = start + last_newline as u64 + 1;
            if complete_len == original_len {
                return Ok(0);
            }
            file.set_len(complete_len)?;
            file.sync_all()?;
            return Ok(original_len - complete_len);
        }

        end = start;
    }

    file.set_len(0)?;
    file.sync_all()?;
    Ok(original_len)
}

/// Make a completed rename durable. Windows has no directory fsync; NTFS
/// journals the rename itself, so this does nothing there.
pub fn sync_parent(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

fn write_with(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent)?;

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = private;
    }

    // Opening with create_new makes this temp path ours. If opening fails,
    // do not remove a colliding file that may belong to another writer.
    let mut file = options.open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rename_with_retry(&temp, path)?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// On Windows an antivirus scanner or the indexer can hold the target open
/// for a few milliseconds, and the rename fails with a sharing violation
/// (`PermissionDenied`). Retry briefly before giving up.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 5;
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err)
                if cfg!(windows)
                    && err.kind() == io::ErrorKind::PermissionDenied
                    && attempt + 1 < ATTEMPTS =>
            {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_creates_parent_and_replaces_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"x").unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["state.json".to_string()]);
    }

    #[test]
    fn failed_write_leaves_nonempty_directory_unchanged() {
        let dir = tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(atomic_write(&blocked, b"new").is_err());
        assert!(blocked.join("child").is_dir());
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["blocked".to_string()]);
    }

    #[test]
    fn truncate_torn_tail_removes_only_bytes_after_last_newline() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, b"first\nsecond\ntrail").unwrap();

        assert_eq!(truncate_torn_tail(&path).unwrap(), 5);
        assert_eq!(std::fs::read(&path).unwrap(), b"first\nsecond\n");
    }

    #[test]
    fn truncate_torn_tail_leaves_complete_file_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let contents = b"first\nsecond\n";
        std::fs::write(&path, contents).unwrap();

        assert_eq!(truncate_torn_tail(&path).unwrap(), 0);
        assert_eq!(std::fs::read(&path).unwrap(), contents);
    }

    #[test]
    fn truncate_torn_tail_empties_file_with_no_complete_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, b"torn!").unwrap();

        assert_eq!(truncate_torn_tail(&path).unwrap(), 5);
        assert_eq!(std::fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn truncate_torn_tail_finds_newline_before_large_tail() {
        const TORN_BYTES: usize = 200_000;
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut contents = b"complete\n".to_vec();
        contents.extend(std::iter::repeat_n(b'x', TORN_BYTES));
        std::fs::write(&path, contents).unwrap();

        assert_eq!(truncate_torn_tail(&path).unwrap(), TORN_BYTES as u64);
        assert_eq!(std::fs::read(&path).unwrap(), b"complete\n");
    }

    #[test]
    fn truncate_torn_tail_ignores_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.jsonl");

        assert_eq!(truncate_torn_tail(&path).unwrap(), 0);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn failed_replace_keeps_the_previous_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"old").unwrap();
        // Deny delete sharing so rename cannot replace the existing target.
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(atomic_write(&path, b"new").is_err());
        drop(guard);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["state.json".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn private_write_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        atomic_write_private(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
