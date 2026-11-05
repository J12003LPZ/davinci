//! The evidence store: full tool output kept on disk when only a bounded
//! projection goes into the model's context.
//!
//! No TypeScript counterpart. The principle is the one Codex's rollout
//! trace draws between model-visible conversation items and runtime work:
//! reduce what the model *sees*, never what is *debuggable*. A `batch`
//! whose operations overflow the visible budget, or a worker whose reply
//! runs long, writes the whole text here and tells the model the path, so
//! a targeted `read` with `offset`/`limit` can fetch the part it needs.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Files older than this are removed by `sweep`, which the product runs at
/// startup. A week covers any session the user is likely to resume.
pub const EVIDENCE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct EvidenceStore {
    dir: PathBuf,
}

impl EvidenceStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `text` under a fresh name with the given tag, returning the
    /// path the model can `read`. Failure is reported, not hidden: the
    /// caller then keeps the truncation note without a path.
    pub fn store(&self, tag: &str, text: &str) -> Result<PathBuf, String> {
        self.store_named("", tag, text)
    }

    /// Write evidence bound to one session. The file name carries a hash of
    /// the session id so the read-boundary exception can admit the current
    /// session's artifacts without admitting another session's.
    pub fn store_in_session(
        &self,
        session_id: &str,
        tag: &str,
        text: &str,
    ) -> Result<PathBuf, String> {
        self.store_named(&format!("s{}-", session_token(session_id)), tag, text)
    }

    fn store_named(&self, prefix: &str, tag: &str, text: &str) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|err| err.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|err| err.to_string())?;
        }
        let safe_tag: String = tag
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect();
        let name = format!(
            "{prefix}{safe_tag}-{}.txt",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        let path = self.dir.join(name);
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|err| err.to_string())?;
            file.write_all(text.as_bytes())
                .map_err(|err| err.to_string())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&path, text).map_err(|err| err.to_string())?;
        }
        Ok(path)
    }

    /// Store output and return both the file path and its cryptographic ArtifactRef.
    pub fn store_with_artifact_ref(
        &self,
        tag: &str,
        text: &str,
        media_type: &str,
    ) -> Result<(PathBuf, crate::runtime::evidence::ArtifactRef), String> {
        let path = self.store(tag, text)?;
        let artifact = self.to_artifact_ref(&path, media_type)?;
        Ok((path, artifact))
    }

    /// Convert an existing stored file into a verifiable ArtifactRef.
    pub fn to_artifact_ref(
        &self,
        path: &Path,
        media_type: &str,
    ) -> Result<crate::runtime::evidence::ArtifactRef, String> {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha256 = format!("{:x}", davinci_sys::hex::Lower(&hasher.finalize()));

        let rel_path = path
            .strip_prefix(&self.dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));

        let id = format!("overflow_{}", &sha256[..12]);
        Ok(crate::runtime::evidence::ArtifactRef {
            id,
            sha256,
            media_type: media_type.to_string(),
            size: bytes.len() as u64,
            relative_store_path: rel_path,
            redaction: None,
        })
    }

    /// Delete files older than `ttl`. Best effort; a store that cannot be
    /// listed simply keeps its files.
    pub fn sweep(&self, ttl: Duration) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let stale = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > ttl);
            if stale && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Stable, non-reversible token for a session's evidence filenames. The raw
/// session id never appears in a path the model can read.
pub fn session_token(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    let digest = format!("{:x}", davinci_sys::hex::Lower(&hasher.finalize()));
    digest[..12].to_string()
}

/// URI schemes of Script evidence that `retrieve_context` can resolve. A URI
/// names a host-created file in the evidence store by its file stem; it never
/// carries a path.
pub const SCRIPT_LOG_URI: &str = "script-log://";
pub const SCRIPT_OUTPUT_URI: &str = "script-output://";

/// Cut `text` at a char boundary at or before `cap` bytes.
pub fn cut_at_char_boundary(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_sweeps() {
        let dir = tempfile::tempdir().unwrap();
        let store = EvidenceStore::new(dir.path().join("evidence"));
        let path = store.store("batch:1", "hello").unwrap();
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("batch_1-"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        // Nothing is older than a week yet.
        assert_eq!(store.sweep(EVIDENCE_TTL), 0);
        // Everything is older than zero seconds after a short pause.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(store.sweep(Duration::from_millis(1)), 1);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stored_evidence_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = EvidenceStore::new(dir.path().join("evidence"));
        let path = store.store("script-output", "secret").unwrap();
        let dir_mode = std::fs::metadata(store.dir()).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn cuts_on_char_boundaries() {
        let text = "héllo wörld";
        let cut = cut_at_char_boundary(text, 2);
        assert_eq!(cut, "h");
        assert_eq!(cut_at_char_boundary(text, 100), text);
    }

    #[test]
    fn test_oversize_output_retains_retrieval_reference() {
        let dir = tempfile::tempdir().unwrap();
        let store = EvidenceStore::new(dir.path().join("evidence"));
        let large_content = "a".repeat(10_000);
        let (path, artifact) = store
            .store_with_artifact_ref("test_overflow", &large_content, "text/plain")
            .unwrap();

        assert!(path.exists());
        assert_eq!(artifact.size, 10_000);
        assert!(!artifact.sha256.is_empty());
        assert!(artifact.relative_store_path.contains("test_overflow-"));
    }
}
