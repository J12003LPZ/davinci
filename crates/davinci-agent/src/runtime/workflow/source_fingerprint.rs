//! A fingerprint of the source tree a workflow artifact was produced against,
//! so a resume never reuses an artifact over a tree that has since changed.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Most changed files a fingerprint hashes the contents of.
const MAX_FILES: usize = 2_048;
/// Most content bytes a fingerprint hashes in all.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const GIT_STATUS_LIMIT: usize = 2 * 1024 * 1024;
/// Runtime state and VCS metadata: not source, and it churns on its own.
const EXCLUDED_ROOTS: [&str; 3] = [".git", ".davinci", ".pi"];

/// Git HEAD plus a digest of the working-tree status and of the contents of
/// every modified or untracked file under `root`. `None` when `root` is not a
/// git repository with a commit, or when the changes are too many or too large
/// to hash: the caller then has no evidence either way and keeps its behavior
/// from before fingerprints existed.
pub(crate) fn source_fingerprint(root: &Path) -> Option<String> {
    let git =
        |args: &[&str], limit: usize| crate::runtime::transactions::observe_git(root, args, limit);
    let head = git(&["rev-parse", "HEAD"], 256).ok()?;
    let top = git(&["rev-parse", "--show-toplevel"], 8192).ok()?;
    let top = PathBuf::from(std::str::from_utf8(&top).ok()?.trim());
    let status = git(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        GIT_STATUS_LIMIT,
    )
    .ok()?;
    let changed = crate::completion::parse_git_status(&status)?;

    let mut hasher = Sha256::new();
    hasher.update(b"head\0");
    hasher.update(head.trim_ascii());
    let (mut files, mut bytes) = (0usize, 0u64);
    // BTreeMap order: the digest does not depend on git's listing order.
    for (path, state) in &changed {
        if path
            .components()
            .next()
            .is_some_and(|first| EXCLUDED_ROOTS.iter().any(|root| first.as_os_str() == *root))
        {
            continue;
        }
        files += 1;
        if files > MAX_FILES {
            return None;
        }
        hasher.update(b"\0path\0");
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(b"\0state\0");
        hasher.update(state);
        hasher.update(b"\0content\0");
        hash_content(&mut hasher, &top.join(path), &mut bytes)?;
    }
    Some(format!("{:x}", davinci_sys::hex::Lower(&hasher.finalize())))
}

/// Feed `path`'s identity into `hasher`: its bytes, its link target, or a
/// marker for a path that is gone (a deletion) or is a directory.
fn hash_content(hasher: &mut Sha256, path: &Path, total: &mut u64) -> Option<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hasher.update(b"absent");
            return Some(());
        }
        Err(_) => return None,
    };
    if metadata.file_type().is_symlink() {
        hasher.update(b"link\0");
        hasher.update(std::fs::read_link(path).ok()?.to_string_lossy().as_bytes());
    } else if metadata.is_file() {
        *total = total.saturating_add(metadata.len());
        if *total > MAX_BYTES {
            return None;
        }
        hasher.update(b"file\0");
        let mut file = std::fs::File::open(path).ok()?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    } else {
        hasher.update(b"other");
    }
    Some(())
}
