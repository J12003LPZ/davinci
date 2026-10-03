//! Content-addressed file snapshots shared by graph runs.
use std::path::{Path, PathBuf};

pub fn dir(cwd: &Path) -> PathBuf {
    super::store::runs_root_dir(cwd)
        .parent()
        .unwrap_or(cwd)
        .join("blobs")
}

fn path(dir: &Path, hash: &str) -> Option<PathBuf> {
    (hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then(|| dir.join(&hash[..2]).join(hash))
}

pub fn put(dir: &Path, hash: &str, bytes: &[u8]) -> std::io::Result<()> {
    let Some(target) = path(dir, hash) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid blob hash",
        ));
    };
    if target.exists() {
        return Ok(());
    }
    davinci_sys::fs::atomic_write(&target, bytes)
}

pub fn get(dir: &Path, hash: &str) -> Option<Vec<u8>> {
    std::fs::read(path(dir, hash)?).ok()
}
