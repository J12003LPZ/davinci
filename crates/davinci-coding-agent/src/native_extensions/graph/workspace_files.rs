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

pub(super) fn read(root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    const MAX_WORKSPACE_FILE_BYTES: u64 = 64 * 1024 * 1024;
    let path = checked_path(root, relative)?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || is_redirected(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an ordinary workspace file",
        ));
    }
    if metadata.len() > MAX_WORKSPACE_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "workspace file is larger than the {MAX_WORKSPACE_FILE_BYTES}-byte graph capture limit"
            ),
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an ordinary workspace file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
