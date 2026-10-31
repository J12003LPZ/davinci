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

/// What is known of the source tree under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceState {
    /// Git definitively reports no work tree here (or git is not installed):
    /// no evidence either way, so the caller keeps its behavior from before
    /// fingerprints existed.
    NotGit,
    /// A git workspace whose state cannot be fingerprinted (git or a file read
    /// failed, HEAD does not resolve, the changes are too many or too large,
    /// or a directory entry such as a dirty submodule hides its own changes).
    /// Nothing stored against it can be shown to still match.
    Unavailable,
    /// Git HEAD plus a digest of the working-tree status and of the contents of
    /// every modified or untracked file.
    Digest(String),
}

/// The digest of [`SourceState::Digest`]; `None` for every other state.
pub(crate) fn source_fingerprint(root: &Path) -> Option<String> {
    match source_state(root) {
        SourceState::Digest(digest) => Some(digest),
        SourceState::NotGit | SourceState::Unavailable => None,
    }
}

pub(crate) fn source_state(root: &Path) -> SourceState {
    match probe_work_tree(root) {
        WorkTree::Inside => {}
        WorkTree::Outside => return SourceState::NotGit,
        WorkTree::Unknown => return SourceState::Unavailable,
    }
    let git =
        |args: &[&str], limit: usize| crate::runtime::transactions::observe_git(root, args, limit);
    // Inside a work tree, a HEAD that does not resolve (unborn, corrupt) is a
    // tree that cannot be fingerprinted, not a tree without git.
    let (Ok(head), Ok(top)) = (
        git(&["rev-parse", "HEAD"], 256),
        git(&["rev-parse", "--show-toplevel"], 8192),
    ) else {
        return SourceState::Unavailable;
    };
    match digest_working_tree(root, &head, &top) {
        Some(digest) => SourceState::Digest(digest),
        None => SourceState::Unavailable,
    }
}

enum WorkTree {
    Inside,
    /// Git definitively reports there is no work tree here.
    Outside,
    /// Git could not answer, or failed for a reason other than "no repository".
    Unknown,
}

/// Ask git whether `root` is inside a work tree. `Outside` only when `root` is
/// not a directory, git is not installed, git prints `false`, or git says "not a
/// git repository" and no `.git` entry exists at or above `root`. A `.git` entry
/// that git rejects (a corrupt `.git/HEAD` also reads as "not a git repository")
/// is a damaged workspace, so it is `Unknown`, as is every other failure.
fn probe_work_tree(root: &Path) -> WorkTree {
    if !root.is_dir() {
        return WorkTree::Outside;
    }
    let mut command = std::process::Command::new("git");
    command
        .current_dir(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        // The "not a git repository" message is matched below: keep it untranslated.
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = match command.output() {
        Ok(output) => output,
        // git is not on PATH: nothing can be fingerprinted, which is the
        // behavior from before fingerprints existed.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return WorkTree::Outside,
        Err(_) => return WorkTree::Unknown,
    };
    if output.status.success() {
        return match output.stdout.trim_ascii() {
            b"true" => WorkTree::Inside,
            b"false" => WorkTree::Outside,
            _ => WorkTree::Unknown,
        };
    }
    let message = String::from_utf8_lossy(&output.stderr);
    let has_git_entry = root
        .ancestors()
        .any(|dir| std::fs::symlink_metadata(dir.join(".git")).is_ok());
    if message.contains("not a git repository") && !has_git_entry {
        WorkTree::Outside
    } else {
        WorkTree::Unknown
    }
}

fn digest_working_tree(root: &Path, head: &[u8], top: &[u8]) -> Option<String> {
    let top = PathBuf::from(std::str::from_utf8(top).ok()?.trim());
    let status = crate::runtime::transactions::observe_git(
        root,
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
/// marker for a path that is gone (a deletion). `None` when it cannot be
/// hashed, including a directory: a submodule or nested repository shows in
/// status as one entry whose own changes a placeholder would hide.
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
    } else if metadata.is_dir() {
        return None;
    } else {
        hasher.update(b"other");
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
    }

    #[test]
    fn a_plain_directory_is_not_git() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(source_state(tmp.path()), SourceState::NotGit);
        assert_eq!(
            source_state(&tmp.path().join("nowhere")),
            SourceState::NotGit
        );
    }

    #[test]
    fn a_repository_without_a_resolvable_head_is_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q"]);
        // Unborn HEAD: a git workspace with no commit to fingerprint.
        assert_eq!(source_state(tmp.path()), SourceState::Unavailable);
        git(tmp.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        assert!(matches!(source_state(tmp.path()), SourceState::Digest(_)));
    }

    #[test]
    fn a_corrupt_git_head_is_unavailable_not_git() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q"]);
        git(tmp.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        std::fs::write(tmp.path().join(".git").join("HEAD"), "garbage\n").unwrap();
        assert_eq!(source_state(tmp.path()), SourceState::Unavailable);
    }
}
