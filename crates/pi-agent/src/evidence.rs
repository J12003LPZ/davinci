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
        std::fs::create_dir_all(&self.dir).map_err(|err| err.to_string())?;
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
            "{safe_tag}-{}.txt",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        let path = self.dir.join(name);
        std::fs::write(&path, text).map_err(|err| err.to_string())?;
        Ok(path)
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

    #[test]
    fn cuts_on_char_boundaries() {
        let text = "héllo wörld";
        let cut = cut_at_char_boundary(text, 2);
        assert_eq!(cut, "h");
        assert_eq!(cut_at_char_boundary(text, 100), text);
    }
}
