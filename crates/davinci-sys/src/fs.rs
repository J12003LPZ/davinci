//! Crash-safe file publication. A reader sees the old file or the new one,
//! never a torn one: bytes go to a temporary file in the same directory, are
//! flushed, and replace the target with one rename. Also holds the bounded
//! readers and opened-file identity checks used to admit untrusted inputs.

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

/// Create `path` and any missing parents so only the owner can enter them.
/// On Unix new directories are created 0700, and an existing `path` that
/// grants group or other access is tightened to 0700 (its parents are left
/// alone). Use it for directories that hold transcripts or tool output.
///
/// Tightening is best effort: a directory owned by someone else cannot be
/// chmod-ed, and refusing to work there would break existing setups. Files
/// written into it should still be created owner-only.
pub fn create_private_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        let metadata = fs::metadata(path)?;
        if metadata.permissions().mode() & 0o077 != 0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Open a new file for writing that must not already exist. On Unix it is
/// created 0600 so only its owner can read it.
pub fn create_new_private(path: &Path) -> io::Result<fs::File> {
    private_options().write(true).create_new(true).open(path)
}

/// Open `path` for appending, creating it 0600 on Unix when missing.
pub fn open_append_private(path: &Path) -> io::Result<fs::File> {
    private_options().create(true).append(true).open(path)
}

/// Restrict an existing file to its owner (0600 on Unix). No-op elsewhere.
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn private_options() -> fs::OpenOptions {
    #[allow(unused_mut)]
    let mut options = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Read at most `cap` bytes from an already opened file. The declared length
/// rejects an oversized file before any read; the read itself stops at
/// `cap + 1` bytes, so a file that grows after the check still cannot allocate
/// more than that. An oversized file is `ErrorKind::FileTooLarge`.
pub fn read_bounded(file: &mut fs::File, cap: u64) -> io::Result<Vec<u8>> {
    let declared = file.metadata()?.len();
    if declared > cap {
        return Err(too_large(cap));
    }
    read_capped(file, declared, cap)
}

/// Read `reader` to its end, refusing more than `cap` bytes. `hint` only sizes
/// the buffer; the `cap + 1` read limit is what bounds memory.
pub fn read_capped<R: Read>(reader: R, hint: u64, cap: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(usize::try_from(hint.min(cap)).unwrap_or(0));
    reader.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(too_large(cap));
    }
    Ok(bytes)
}

/// Open `path` and [`read_bounded`] it.
pub fn read_path_bounded(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    read_bounded(&mut fs::File::open(path)?, cap)
}

fn too_large(cap: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        format!("input exceeds the {cap} byte limit"),
    )
}

/// Append one line (without its newline) from `reader` to `line`, keeping at
/// most `cap` of its bytes. Returns the bytes consumed, `0` at end of input.
/// A line longer than `cap` is `ErrorKind::InvalidData`: the rest of it is
/// consumed and dropped without being buffered, so the reader is left at the
/// next line and memory stays bounded by `cap`, which `read_line` cannot offer.
pub fn read_line_bounded<R: io::BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    cap: usize,
) -> io::Result<usize> {
    let mut consumed = 0;
    let mut overflow = false;
    loop {
        let (done, used) = {
            let available = match reader.fill_buf() {
                Ok(available) => available,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            if available.is_empty() {
                break;
            }
            let (chunk, done, used) = match available.iter().position(|byte| *byte == b'\n') {
                Some(end) => (&available[..end], true, end + 1),
                None => (available, false, available.len()),
            };
            if !overflow {
                let room = cap.saturating_sub(line.len());
                if chunk.len() > room {
                    line.extend_from_slice(&chunk[..room]);
                    overflow = true;
                } else {
                    line.extend_from_slice(chunk);
                }
            }
            (done, used)
        };
        reader.consume(used);
        consumed += used;
        if done {
            break;
        }
    }
    if overflow {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("line exceeds the {cap} byte limit"),
        ));
    }
    Ok(consumed)
}

/// Stable identity of an opened file plus its hard-link count. Two handles to
/// the same file compare equal whatever path reached them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    /// Volume or device the file lives on.
    pub volume: u64,
    /// File index or inode on that volume (128 bits on Windows).
    pub index: u128,
    /// Number of directory entries naming the file.
    pub links: u64,
}

/// Inspect an opened file. Fails (`Unsupported`) where the platform cannot
/// report an identity, so a protected caller can fail closed.
pub fn file_identity(file: &fs::File) -> io::Result<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        Ok(FileIdentity {
            volume: meta.dev(),
            index: u128::from(meta.ino()),
            links: meta.nlink(),
        })
    }
    #[cfg(windows)]
    {
        windows_identity(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[cfg(windows)]
fn windows_identity(file: &fs::File) -> io::Result<FileIdentity> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    const FILE_STANDARD_INFO_CLASS: i32 = 1;
    const FILE_ID_INFO_CLASS: i32 = 18;

    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }
    #[repr(C)]
    struct FileIdInfo {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            file: *mut c_void,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }

    let handle = file.as_raw_handle();
    let mut id = FileIdInfo {
        volume_serial_number: 0,
        file_id: [0; 16],
    };
    let mut standard = FileStandardInfo {
        allocation_size: 0,
        end_of_file: 0,
        number_of_links: 0,
        delete_pending: 0,
        directory: 0,
    };
    // SAFETY: `handle` is a live file handle for both calls, and each buffer
    // is a `repr(C)` struct of the size passed alongside it.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_ID_INFO_CLASS,
            (&mut id as *mut FileIdInfo).cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        ) != 0
            && GetFileInformationByHandleEx(
                handle,
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            ) != 0
    };
    if !ok {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity {
        volume: id.volume_serial_number,
        index: u128::from_le_bytes(id.file_id),
        links: u64::from(standard.number_of_links),
    })
}

/// The path an opened file or directory actually resolves to, as the
/// operating system reports it for the handle (symlinks, junctions and
/// renames already applied). Checking this on the handle that is read closes
/// the gap between confining a path and opening it. `Unsupported` where the
/// platform cannot say, so a protected caller can fail closed.
pub fn final_path(file: &fs::File) -> io::Result<std::path::PathBuf> {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStringExt;
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFinalPathNameByHandleW(
                file: *mut c_void,
                path: *mut u16,
                len: u32,
                flags: u32,
            ) -> u32;
        }
        let handle = file.as_raw_handle();
        let mut buffer = vec![0u16; 512];
        loop {
            // SAFETY: `handle` is live and `buffer` has `len` writable units.
            let written = unsafe {
                GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0)
            } as usize;
            if written == 0 {
                return Err(io::Error::last_os_error());
            }
            if written < buffer.len() {
                buffer.truncate(written);
                return Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                    &buffer,
                )));
            }
            buffer.resize(written + 1, 0);
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let mut buffer = [0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX.
        let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        Ok(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            &buffer[..end],
        )))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = file;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Open a directory for inspection of its identity or final path. On Windows
/// that needs backup semantics; a reparse point is opened as the link itself
/// is not wanted, so one that resolves elsewhere shows in [`final_path`].
pub fn open_directory(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

/// [`open_directory`] that also forbids deleting or renaming the directory (or
/// any ancestor) while the handle is open. A caller that checks a directory's
/// identity and then acts on its pathname holds this across both steps, so the
/// name cannot be pointed at another directory in between. Where the platform
/// has no such sharing mode the handle is a plain one and the caller must
/// re-check identity after acting.
pub fn open_directory_pinned(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

/// [`open_directory_pinned`] with the right to read and rewrite the directory's
/// DACL, so security can be changed through this very handle instead of by
/// pathname. The handle is opened once and the ACL calls name it, never the
/// path, so a directory swapped in under the name is not the one modified.
pub fn open_directory_for_acl(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;
        const READ_CONTROL: u32 = 0x0002_0000;
        const WRITE_DAC: u32 = 0x0004_0000;
        const FILE_READ_ATTRIBUTES: u32 = 0x80;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        // Listing the directory counts as data access, which is what makes the
        // share mode below bind: a handle opened for attributes and security
        // alone would not stop a rename.
        const FILE_LIST_DIRECTORY: u32 = 0x1;
        fs::OpenOptions::new()
            .access_mode(
                READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES | SYNCHRONIZE | FILE_LIST_DIRECTORY,
            )
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

/// Why [`ensure_single_link`] refused a file: it is named by `0` entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardLinked(pub u64);

impl std::fmt::Display for HardLinked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file has {} hard links", self.0)
    }
}

impl std::error::Error for HardLinked {}

/// True when `err` is the refusal [`ensure_single_link`] makes for a file with
/// more than one hard link, as opposed to any other failure to open or inspect.
pub fn is_hard_link_refusal(err: &io::Error) -> bool {
    err.get_ref().is_some_and(|inner| inner.is::<HardLinked>())
}

/// Fail closed unless `file` is named by exactly one directory entry. A second
/// hard link can alias a file outside the workspace under an in-workspace name,
/// which path canonicalization cannot see. A platform that cannot report the
/// link count is an error too.
pub fn ensure_single_link(file: &fs::File) -> io::Result<()> {
    let identity = file_identity(file)?;
    if identity.links == 1 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            HardLinked(identity.links),
        ))
    }
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
    fn private_dir_is_created_and_tightened_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        create_private_dir_all(&nested).unwrap();
        for path in [dir.path().join("a"), nested.clone()] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", path.display());
        }
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir_all(&nested).unwrap();
        let mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let created = dir.path().join("new.jsonl");
        create_new_private(&created).unwrap();
        let appended = dir.path().join("log.jsonl");
        open_append_private(&appended).unwrap();
        let legacy = dir.path().join("legacy.jsonl");
        std::fs::write(&legacy, b"x").unwrap();
        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o644)).unwrap();
        restrict_to_owner(&legacy).unwrap();
        for path in [created, appended, legacy] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", path.display());
        }
    }

    #[test]
    fn create_new_private_refuses_an_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("taken");
        std::fs::write(&path, b"x").unwrap();
        assert!(create_new_private(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
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

    #[test]
    fn audit_wp01_read_bounded_rejects_oversized_and_growing_inputs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("data");
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(read_path_bounded(&path, 5).unwrap(), b"12345");
        let err = read_path_bounded(&path, 4).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);

        // A reader that lies about its length is still stopped at cap + 1.
        struct Endless(u64);
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.0 += buf.len() as u64;
                buf.fill(b'x');
                Ok(buf.len())
            }
        }
        let mut endless = Endless(0);
        let err = read_capped(&mut endless, 0, 10).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
        assert_eq!(endless.0, 11, "read exactly cap + 1 bytes");
    }

    #[test]
    fn audit_wp01_read_line_bounded_never_buffers_past_the_cap() {
        let mut reader = io::BufReader::with_capacity(4, &b"ab\ncdefghij\nlast"[..]);
        let mut line = Vec::new();
        assert_eq!(read_line_bounded(&mut reader, &mut line, 7).unwrap(), 3);
        assert_eq!(line, b"ab");

        // The over-long line is dropped whole; the reader sits at the next one.
        line.clear();
        let err = read_line_bounded(&mut reader, &mut line, 7).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(line.len() <= 7, "buffered {} bytes", line.len());

        line.clear();
        assert_eq!(read_line_bounded(&mut reader, &mut line, 7).unwrap(), 4);
        assert_eq!(line, b"last");
        assert_eq!(read_line_bounded(&mut reader, &mut line, 7).unwrap(), 0);

        // A line of exactly the cap is kept.
        let mut exact = &b"1234567\n"[..];
        line.clear();
        assert_eq!(read_line_bounded(&mut exact, &mut line, 7).unwrap(), 8);
        assert_eq!(line, b"1234567");
    }

    #[test]
    fn audit_wp01_final_path_names_the_object_the_handle_reached() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let handle = fs::File::open(&file).unwrap();
        let real = final_path(&handle).unwrap();
        assert_eq!(real, std::fs::canonicalize(&file).unwrap());
        let directory = open_directory(dir.path()).unwrap();
        assert_eq!(
            final_path(&directory).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        // A rename after opening is followed by the handle, not by the old name.
        let moved = dir.path().join("b.txt");
        std::fs::rename(&file, &moved).unwrap_or(());
        if moved.exists() {
            assert_eq!(
                final_path(&handle).unwrap(),
                std::fs::canonicalize(&moved).unwrap()
            );
        }
    }

    #[test]
    fn audit_wp01_file_identity_counts_hard_links() {
        let dir = tempdir().unwrap();
        let original = dir.path().join("original");
        let alias = dir.path().join("alias");
        std::fs::write(&original, b"canary").unwrap();
        let single = fs::File::open(&original).unwrap();
        let before = file_identity(&single).unwrap();
        assert_eq!(before.links, 1);
        ensure_single_link(&single).unwrap();

        std::fs::hard_link(&original, &alias).unwrap();
        let linked = fs::File::open(&alias).unwrap();
        let after = file_identity(&linked).unwrap();
        assert_eq!(after.links, 2);
        assert_eq!((after.volume, after.index), (before.volume, before.index));
        let err = ensure_single_link(&linked).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(is_hard_link_refusal(&err));
        assert!(!is_hard_link_refusal(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert_eq!(std::fs::read(&original).unwrap(), b"canary");
    }
}
