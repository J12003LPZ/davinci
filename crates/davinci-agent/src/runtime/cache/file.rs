use super::{
    digest, directory::Directory, CacheError, CacheKey, CacheNamespace, CachePolicy, CacheRequest,
    CacheRuntime,
};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// A fresh bounded read. Exact bytes, rather than timestamps, establish freshness.
pub struct FileSnapshot {
    pub canonical_path: PathBuf,
    pub content_hash: String,
    pub bytes: Vec<u8>,
    pub modified: Option<SystemTime>,
}
pub fn read_current_file(
    root: &Path,
    relative: &Path,
    max_bytes: usize,
    authorize: impl Fn() -> Result<(), CacheError>,
) -> Result<FileSnapshot, CacheError> {
    let mut snapshot = read_bytes(root, relative, max_bytes, &authorize)?;
    snapshot.content_hash = digest(&snapshot.bytes);
    Ok(snapshot)
}

impl CacheRuntime {
    /// Always reread authorized bytes. Reuse the digest only after exact byte
    /// equality, avoiding repeated SHA work without relying on mtime or size.
    pub fn read_current_file(
        &self,
        root: &Path,
        relative: &Path,
        max_bytes: usize,
        authorize: impl Fn() -> Result<(), CacheError>,
    ) -> Result<FileSnapshot, CacheError> {
        let mut snapshot = read_bytes(root, relative, max_bytes, &authorize)?;
        self.record_source_read(CacheNamespace::File, snapshot.bytes.len());
        let request = CacheRequest::new(
            CacheKey::new(
                CacheNamespace::File,
                format!("fresh-bytes:{}", snapshot.canonical_path.display()),
                1,
                "exact-bytes-v1",
                vec![],
            ),
            CachePolicy::MemoryOnly,
        );
        if let Some(previous) = self.get::<(Vec<u8>, String)>(&request, &authorize)? {
            if previous.0 == snapshot.bytes {
                snapshot.content_hash = previous.1.clone();
                return Ok(snapshot);
            }
        }
        snapshot.content_hash = digest(&snapshot.bytes);
        self.put(
            &request,
            (snapshot.bytes.clone(), snapshot.content_hash.clone()),
            &authorize,
        )?;
        Ok(snapshot)
    }
}

fn read_bytes(
    root: &Path,
    relative: &Path,
    max_bytes: usize,
    authorize: impl Fn() -> Result<(), CacheError>,
) -> Result<FileSnapshot, CacheError> {
    authorize()?;
    if relative
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(CacheError::Denied);
    }
    let root = root.canonicalize().map_err(|_| CacheError::Denied)?;
    let target = root.join(relative);
    let parent = target.parent().ok_or(CacheError::Denied)?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(CacheError::Denied)?;
    let dir = Directory::open(parent, false).map_err(|_| CacheError::Denied)?;
    let file = dir.source_file(name).map_err(|_| CacheError::Denied)?;
    let metadata = file.metadata().map_err(|_| CacheError::Denied)?;
    if metadata.len() > max_bytes as u64 {
        return Err(CacheError::Compute("file exceeds cache read limit".into()));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CacheError::Denied)?;
    if bytes.len() > max_bytes {
        return Err(CacheError::Compute("file exceeds cache read limit".into()));
    }
    authorize()?;
    Ok(FileSnapshot {
        canonical_path: target,
        content_hash: String::new(),
        bytes,
        modified: metadata.modified().ok(),
    })
}
