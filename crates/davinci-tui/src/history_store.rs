//! Prompt history kept per working directory, so `↑` recalls what was sent
//! in earlier sessions. One JSON object per line, appended on send; the file
//! is trimmed to its newest entries once it outgrows [`MAX_BYTES`].

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Entries loaded into the composer history, newest first.
pub const LOAD_LIMIT: usize = 500;
/// Size past which an append rewrites the file with its newest entries.
const MAX_BYTES: u64 = 512 * 1024;
/// A prompt longer than this is not kept; pasting a file is not history.
const MAX_ENTRY_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    /// The store for `cwd` under `agent_dir/history`.
    pub fn for_cwd(agent_dir: &Path, cwd: &Path) -> Self {
        let key = fnv1a(cwd.to_string_lossy().as_bytes());
        Self {
            path: agent_dir.join("history").join(format!("{key:016x}.jsonl")),
        }
    }

    /// The newest entries, newest first, without consecutive repeats. A
    /// missing or damaged file reads as less history, never an error.
    pub fn load(&self) -> Vec<String> {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for line in raw.lines().rev() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(text) = value.get("text").and_then(|text| text.as_str()) else {
                continue;
            };
            if out.last().is_some_and(|last| last == text) {
                continue;
            }
            out.push(text.to_owned());
            if out.len() >= LOAD_LIMIT {
                break;
            }
        }
        out
    }

    /// Prompts from every project, newest file first and newest entry first
    /// within it, without repeats.
    pub fn load_all(&self) -> Vec<String> {
        let Some(dir) = self.path.parent() else {
            return Vec::new();
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (_, path) in files {
            for text in (Self { path }).load() {
                if seen.insert(text.clone()) {
                    out.push(text);
                }
            }
            if out.len() >= LOAD_LIMIT * 2 {
                break;
            }
        }
        out
    }

    /// Record a sent prompt. Failures are ignored: history is a convenience.
    pub fn append(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_ENTRY_BYTES {
            return;
        }
        let _ = self.try_append(text);
    }

    fn try_append(&self, text: &str) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", serde_json::json!({ "text": text }))?;
        if file.metadata()?.len() > MAX_BYTES {
            self.trim()?;
        }
        Ok(())
    }

    fn trim(&self) -> std::io::Result<()> {
        let mut kept = self.load();
        // Newest first: keep entries until half the byte budget is used, so a
        // run of long prompts does not leave the file at the limit.
        let mut bytes = 0;
        kept.retain(|text| {
            bytes += text.len() + 12;
            bytes <= MAX_BYTES as usize / 2
        });
        kept.reverse();
        let body: String = kept
            .iter()
            .map(|text| format!("{}\n", serde_json::json!({ "text": text })))
            .collect();
        let tmp = self.path.with_extension("jsonl.tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, &self.path)
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = HistoryStore::for_cwd(dir.path(), Path::new("/work/a"));
        (dir, store)
    }

    #[test]
    fn what_was_sent_comes_back_newest_first_across_sessions() {
        let (dir, store) = store();
        store.append("first");
        store.append("second\nwith a line break");
        let again = HistoryStore::for_cwd(dir.path(), Path::new("/work/a"));
        assert_eq!(again.load(), ["second\nwith a line break", "first"]);
    }

    #[test]
    fn all_projects_merge_without_repeats() {
        let (dir, store) = store();
        store.append("shared");
        store.append("only a");
        let other = HistoryStore::for_cwd(dir.path(), Path::new("/work/b"));
        other.append("shared");
        other.append("only b");
        let mut all = store.load_all();
        all.sort();
        assert_eq!(all, ["only a", "only b", "shared"]);
    }

    #[test]
    fn consecutive_repeats_collapse_and_other_projects_stay_apart() {
        let (dir, store) = store();
        store.append("same");
        store.append("same");
        store.append("  ");
        assert_eq!(store.load(), ["same"]);
        let other = HistoryStore::for_cwd(dir.path(), Path::new("/work/b"));
        assert!(other.load().is_empty());
    }

    #[test]
    fn a_damaged_line_is_skipped_and_the_file_stays_bounded() {
        let (_dir, store) = store();
        store.append("kept");
        let mut file = OpenOptions::new().append(true).open(&store.path).unwrap();
        writeln!(file, "not json").unwrap();
        assert_eq!(store.load(), ["kept"]);
        let big = "x".repeat(8 * 1024);
        for n in 0..100 {
            store.append(&format!("{n} {big}"));
        }
        assert!(fs::metadata(&store.path).unwrap().len() <= MAX_BYTES + 16 * 1024);
        assert!(store.load()[0].starts_with("99 "));
    }
}
