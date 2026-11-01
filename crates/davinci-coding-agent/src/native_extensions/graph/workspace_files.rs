//! Workspace scans exclude redirected ancestors and never read symlink targets.
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// A symlink on Unix, or a symlink *or* junction on Windows. `is_symlink`
/// alone misses directory junctions, which are the common reparse point in a
/// Windows checkout.
pub(super) fn is_redirected(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

pub(super) fn checked_path(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut path = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if !matches!(component, Component::Normal(_)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "non-relative workspace path",
            ));
        }
        path.push(component);
        if components.peek().is_some() {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || is_redirected(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "redirected workspace ancestor",
                ));
            }
        }
    }
    Ok(path)
}

/// A scanned workspace entry that is either left out on purpose or present.
/// `Excluded` is a symlink, junction, non-regular entry or redirected
/// ancestor; every real failure is an `Err` instead.
pub(super) enum Entry<T> {
    Excluded,
    Present(T),
}

/// Streamed SHA-256 and length of a workspace file, plus its bytes when the
/// file fit within the caller's keep limit.
pub(super) struct Fingerprinted {
    pub hash: String,
    pub len: u64,
    pub bytes: Option<Vec<u8>>,
}

/// Opens an ordinary file below `root`. Malformed paths and I/O failures are
/// errors; anything that is not an ordinary file is `Excluded`.
fn open_ordinary(root: &Path, relative: &Path) -> io::Result<Entry<(std::fs::File, u64)>> {
    let mut path = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if !matches!(component, Component::Normal(_)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "non-relative workspace path",
            ));
        }
        path.push(component);
        if components.peek().is_some() {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || is_redirected(&metadata) {
                return Ok(Entry::Excluded);
            }
        }
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || is_redirected(&metadata) {
        return Ok(Entry::Excluded);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(Entry::Excluded);
    }
    Ok(Entry::Present((file, metadata.len())))
}

/// Hashes the whole file with no size limit. Bytes are retained only while the
/// file stays within `keep_limit`.
pub(super) fn fingerprint(
    root: &Path,
    relative: &Path,
    keep_limit: u64,
) -> io::Result<Entry<Fingerprinted>> {
    use sha2::{Digest, Sha256};
    let (mut file, _) = match open_ordinary(root, relative)? {
        Entry::Excluded => return Ok(Entry::Excluded),
        Entry::Present(opened) => opened,
    };
    let mut hasher = Sha256::new();
    let mut kept = Some(Vec::new());
    let mut len = 0u64;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
        len += n as u64;
        if len > keep_limit {
            kept = None;
        } else if let Some(kept) = kept.as_mut() {
            kept.extend_from_slice(&chunk[..n]);
        }
    }
    let mut hash = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write;
        let _ = write!(hash, "{byte:02x}");
    }
    Ok(Entry::Present(Fingerprinted {
        hash,
        len,
        bytes: kept,
    }))
}

pub(super) fn read(root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    const MAX_WORKSPACE_FILE_BYTES: u64 = 64 * 1024 * 1024;
    let (mut file, len) = match open_ordinary(root, relative)? {
        Entry::Excluded => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an ordinary workspace file",
            ));
        }
        Entry::Present(opened) => opened,
    };
    if len > MAX_WORKSPACE_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "workspace file is larger than the {MAX_WORKSPACE_FILE_BYTES}-byte graph capture limit"
            ),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(entry: io::Result<Entry<Fingerprinted>>) -> Fingerprinted {
        match entry.unwrap() {
            Entry::Present(value) => value,
            Entry::Excluded => panic!("ordinary file excluded"),
        }
    }

    #[test]
    fn fingerprint_hashes_oversized_files_and_sees_same_length_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        std::fs::write(&path, b"aaaaaaaa").unwrap();
        let first = present(fingerprint(dir.path(), Path::new("big.bin"), 4));
        assert_eq!(first.len, 8);
        assert!(first.bytes.is_none(), "bytes beyond the keep limit");
        std::fs::write(&path, b"aaaaaaab").unwrap();
        let second = present(fingerprint(dir.path(), Path::new("big.bin"), 4));
        assert_eq!(second.len, 8);
        assert_ne!(first.hash, second.hash);
        let kept = present(fingerprint(dir.path(), Path::new("big.bin"), 8));
        assert_eq!(kept.bytes.as_deref(), Some(&b"aaaaaaab"[..]));
        assert_eq!(kept.hash, second.hash);
    }

    #[test]
    fn malformed_paths_are_failures_not_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../outside", "/absolute"] {
            match fingerprint(dir.path(), Path::new(bad), 4) {
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::InvalidInput),
                Ok(_) => panic!("{bad} was accepted"),
            }
        }
    }

    #[test]
    fn directories_are_excluded_and_missing_files_fail() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(matches!(
            fingerprint(dir.path(), Path::new("sub"), 4),
            Ok(Entry::Excluded)
        ));
        match fingerprint(dir.path(), Path::new("gone"), 4) {
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
            Ok(_) => panic!("missing file was accepted"),
        }
    }
}
