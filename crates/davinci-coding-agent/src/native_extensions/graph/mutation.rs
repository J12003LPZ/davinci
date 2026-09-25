//! Graph-owned mutation provenance and delta tracking.
//!
//! Captures a workspace baseline before writer mutation, and computes
//! graph-owned deltas post-mutation so that pre-existing uncommitted user
//! edits in a dirty workspace are not attributed to the graph.

use super::replay::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprint {
    pub hash: String,
    pub len: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationBaseline {
    pub files: BTreeMap<String, FileFingerprint>,
    /// Legacy-only inline bytes. Old checkpoints still deserialize, but new
    /// checkpoints keep file bytes in the content-addressed blob store.
    #[serde(default, skip_serializing)]
    pub contents: BTreeMap<String, Vec<u8>>,
}

impl MutationBaseline {
    pub fn old_bytes(&self, path: &str, blob_dir: &Path) -> Vec<u8> {
        if let Some(bytes) = self.contents.get(path) {
            return bytes.clone();
        }
        self.files
            .get(path)
            .and_then(|fingerprint| super::blobs::get(blob_dir, &fingerprint.hash))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
}

impl ChangedFile {
    pub fn added(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            status: "added".to_string(),
        }
    }

    pub fn modified(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            status: "modified".to_string(),
        }
    }

    pub fn deleted(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            status: "deleted".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchChunk {
    pub file: String,
    pub patch: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMutation {
    pub files: Vec<ChangedFile>,
    pub patch_chunks: Vec<PatchChunk>,
}

impl GraphMutation {
    pub fn diff(&self) -> String {
        self.patch_chunks
            .iter()
            .map(|chunk| chunk.patch.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn assess_risk(&self) -> crate::native_extensions::ecosystem::risk::RiskAssessment {
        crate::native_extensions::ecosystem::risk::assess_change_risk(self)
    }
}

pub fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

// Recovery state must not capture itself inside the next mutation baseline.
// Neither may other harness runtime state: the operation journal is a SQLite
// file that changes on every tool call, and inlining it made one failed run's
// state.json 14 MB and put the database into the graph's own diff.
fn is_transaction_journal(path: &str) -> bool {
    let path = normalize_rel_path(path);
    [
        ".davinci-transactions/",
        ".davinci/graph/",
        ".pi/graph/",
        ".davinci/operations/",
        ".pi/operations/",
        ".davinci/vector-memory/",
        ".pi/vector-memory/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn list_workspace_files(cwd: &Path) -> Vec<String> {
    if cwd.join(".git").exists() {
        let mut list = Vec::new();
        if let Ok(output) = super::git::run(cwd, &["ls-files"]) {
            for line in String::from_utf8_lossy(&output).lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    list.push(normalize_rel_path(trimmed));
                }
            }
        }
        if let Ok(output) = super::git::run(cwd, &["ls-files", "--others", "--exclude-standard"]) {
            for line in String::from_utf8_lossy(&output).lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    list.push(normalize_rel_path(trimmed));
                }
            }
        }
        list.sort();
        list.dedup();
        list
    } else {
        walk_dir_files(cwd)
    }
}

fn walk_dir_files(root: &Path) -> Vec<String> {
    let mut result = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == ".git"
                || (dir == root && name == ".davinci-transactions")
                || name == ".pi"
                || name == ".davinci"
                || name == "target"
                || name == "node_modules"
            {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    result.push(normalize_rel_path(&rel.to_string_lossy()));
                }
            }
        }
    }
    result.sort();
    result
}

/// Capture baseline workspace fingerprints and contents before writer mutation.
pub fn capture_baseline(cwd: &Path) -> Result<MutationBaseline, String> {
    let blob_dir = super::blobs::dir(cwd);
    let mut files = BTreeMap::new();

    let paths = list_workspace_files(cwd);
    for rel_path in paths {
        if is_transaction_journal(&rel_path) {
            continue;
        }
        let full = cwd.join(&rel_path);
        if let Ok(bytes) = std::fs::read(&full) {
            let hash = sha256_hex(&bytes);
            if bytes.len() <= 512 * 1024 {
                super::blobs::put(&blob_dir, &hash, &bytes).map_err(|error| error.to_string())?;
            }
            files.insert(
                rel_path,
                FileFingerprint {
                    hash,
                    len: bytes.len() as u64,
                },
            );
        }
    }

    Ok(MutationBaseline {
        files,
        contents: BTreeMap::new(),
    })
}

/// Compute graph-owned delta against a captured baseline.
pub fn capture_graph_delta(
    cwd: &Path,
    baseline: &MutationBaseline,
) -> Result<GraphMutation, String> {
    let blob_dir = super::blobs::dir(cwd);
    let current_paths = list_workspace_files(cwd);
    let mut current_map = BTreeMap::new();

    for rel_path in current_paths {
        if is_transaction_journal(&rel_path) {
            continue;
        }
        let full = cwd.join(&rel_path);
        if let Ok(bytes) = std::fs::read(&full) {
            let hash = sha256_hex(&bytes);
            current_map.insert(
                rel_path,
                FileFingerprint {
                    hash,
                    len: bytes.len() as u64,
                },
            );
        }
    }

    let mut changed_files = Vec::new();
    let mut patch_chunks = Vec::new();

    // Check for added or modified files
    for (path, fingerprint) in &current_map {
        match baseline.files.get(path) {
            None => {
                // Newly added by graph
                changed_files.push(ChangedFile::added(path));
                let current_bytes = std::fs::read(cwd.join(path)).unwrap_or_default();
                let patch = format_added_file_diff(path, &current_bytes);
                patch_chunks.push(PatchChunk {
                    file: path.clone(),
                    patch,
                });
            }
            Some(old_fp) if old_fp.hash != fingerprint.hash => {
                // Modified by graph
                changed_files.push(ChangedFile::modified(path));
                let old_bytes = baseline.old_bytes(path, &blob_dir);
                let current_bytes = std::fs::read(cwd.join(path)).unwrap_or_default();
                let patch = format_modified_file_diff(path, &old_bytes, &current_bytes);
                patch_chunks.push(PatchChunk {
                    file: path.clone(),
                    patch,
                });
            }
            _ => {
                // Untouched by graph (even if dirty before baseline)
            }
        }
    }

    // Check for deleted files
    for old_path in baseline.files.keys() {
        if is_transaction_journal(old_path) {
            continue;
        }
        if !current_map.contains_key(old_path) {
            changed_files.push(ChangedFile::deleted(old_path));
            let old_bytes = baseline.old_bytes(old_path, &blob_dir);
            let patch = format_deleted_file_diff(old_path, &old_bytes);
            patch_chunks.push(PatchChunk {
                file: old_path.clone(),
                patch,
            });
        }
    }

    Ok(GraphMutation {
        files: changed_files,
        patch_chunks,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedDiffReport {
    pub owned_diff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattributed_diff: Option<String>,
    pub changed_files: Vec<ChangedFile>,
    pub has_unattributed: bool,
}

/// Computes task-owned diff, partitioning out any prior dirty files as unattributed diff.
#[allow(dead_code)]
pub fn compute_owned_diff(
    cwd: &Path,
    baseline: &MutationBaseline,
    prior_dirty_files: &[String],
) -> Result<OwnedDiffReport, String> {
    let delta = capture_graph_delta(cwd, baseline)?;
    let mut owned_chunks = Vec::new();
    let mut unattributed_chunks = Vec::new();

    for chunk in delta.patch_chunks {
        let is_prior_dirty = prior_dirty_files
            .iter()
            .any(|f| normalize_rel_path(f) == chunk.file);
        if is_prior_dirty {
            unattributed_chunks.push(chunk.patch);
        } else {
            owned_chunks.push(chunk.patch);
        }
    }

    let owned_diff = owned_chunks.join("\n");
    let has_unattributed = !unattributed_chunks.is_empty();
    let unattributed_diff = if has_unattributed {
        Some(unattributed_chunks.join("\n"))
    } else {
        None
    };

    Ok(OwnedDiffReport {
        owned_diff,
        unattributed_diff,
        changed_files: delta.files,
        has_unattributed,
    })
}

fn format_added_file_diff(file: &str, bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        return format!(
            "diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n@@ new binary file, {} bytes @@\n",
            bytes.len()
        );
    }
    let text = String::from_utf8_lossy(bytes);
    let count = text.lines().count();
    let mut diff = format!(
        "diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n@@ -0,0 +1,{count} @@\n"
    );
    for line in text.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

fn format_deleted_file_diff(file: &str, bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        return format!(
            "diff --git a/{file} b/{file}\ndeleted file mode 100644\n--- a/{file}\n+++ /dev/null\n@@ deleted binary file, {} bytes @@\n",
            bytes.len()
        );
    }
    let text = String::from_utf8_lossy(bytes);
    let count = text.lines().count();
    let mut diff = format!(
        "diff --git a/{file} b/{file}\ndeleted file mode 100644\n--- a/{file}\n+++ /dev/null\n@@ -1,{count} +0,0 @@\n"
    );
    for line in text.lines() {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

fn format_modified_file_diff(file: &str, old_bytes: &[u8], new_bytes: &[u8]) -> String {
    if old_bytes.contains(&0) || new_bytes.contains(&0) {
        return format!(
            "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ binary file modified @@\n"
        );
    }
    let old_text = String::from_utf8_lossy(old_bytes);
    let new_text = String::from_utf8_lossy(new_bytes);

    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    // Line-level LCS diff
    let n = old_lines.len();
    let m = new_lines.len();

    if n > 2000 || m > 2000 {
        return format!(
            "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1,{n} +1,{m} @@\n@@ large file modified @@\n"
        );
    }

    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            if old_lines[i] == new_lines[j] {
                dp[i][j] = 1 + dp[i + 1][j + 1];
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut ops = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if old_lines[i] == new_lines[j] {
            ops.push((' ', old_lines[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(('-', old_lines[i]));
            i += 1;
        } else {
            ops.push(('+', new_lines[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push(('-', old_lines[i]));
        i += 1;
    }
    while j < m {
        ops.push(('+', new_lines[j]));
        j += 1;
    }

    let mut diff =
        format!("diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1,{n} +1,{m} @@\n");
    for (op, line) in ops {
        diff.push(op);
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

#[cfg(test)]
mod tests {
    #[test]
    fn baseline_serializes_without_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "secret-ish content").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        let json = serde_json::to_string(&baseline).unwrap();
        assert!(!json.contains("contents"), "{json}");
        assert!(json.contains("a.txt"));
        let hash = &baseline.files["a.txt"].hash;
        assert_eq!(
            super::super::blobs::get(&super::super::blobs::dir(dir.path()), hash).unwrap(),
            b"secret-ish content"
        );
    }

    #[test]
    fn delta_reads_old_bytes_from_the_blob_store() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        let reloaded: MutationBaseline =
            serde_json::from_str(&serde_json::to_string(&baseline).unwrap()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        let delta = capture_graph_delta(dir.path(), &reloaded).unwrap();
        let patch = delta.diff();
        assert!(patch.contains("-one") && patch.contains("+two"), "{patch}");
    }

    #[test]
    fn legacy_inline_contents_still_deserialize() {
        let legacy = r#"{"files":{"a.txt":{"hash":"h","len":3}},"contents":{"a.txt":[97,98,99]}}"#;
        let baseline: MutationBaseline = serde_json::from_str(legacy).unwrap();
        assert_eq!(baseline.contents["a.txt"], b"abc");
    }

    #[test]
    fn graph_checkpoint_files_are_not_part_of_git_mutation_baselines() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(dir.path());
        std::fs::write(dir.path().join("source.txt"), "source").unwrap();
        for root in [".davinci/graph/run", ".pi/graph/run"] {
            std::fs::create_dir_all(dir.path().join(root)).unwrap();
            std::fs::write(dir.path().join(root).join("state.json"), "checkpoint").unwrap();
        }
        // Harness runtime state beside the checkpoints: the operation
        // journal (a growing SQLite file) and the vector-memory store.
        for (root, file) in [
            (".davinci/operations", "operations.sqlite3"),
            (".davinci/operations", "operations.sqlite3-wal"),
            (".pi/operations", "operations.sqlite3"),
            (".davinci/vector-memory", "records.jsonl"),
            (".pi/vector-memory", "records.jsonl"),
        ] {
            std::fs::create_dir_all(dir.path().join(root)).unwrap();
            std::fs::write(dir.path().join(root).join(file), "runtime state").unwrap();
        }
        let baseline = capture_baseline(dir.path()).unwrap();
        assert_eq!(
            baseline.files.keys().collect::<Vec<_>>(),
            [&"source.txt".to_string()]
        );
        std::fs::write(
            dir.path().join(".davinci/graph/run/state.json"),
            "next checkpoint",
        )
        .unwrap();
        assert!(capture_graph_delta(dir.path(), &baseline)
            .unwrap()
            .files
            .is_empty());
    }

    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn setup_git_repo(path: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn transaction_journals_never_enter_product_review_in_git_or_plain_workspaces() {
        for git in [false, true] {
            let dir = tempdir().unwrap();
            if git {
                setup_git_repo(dir.path());
            }
            let journals = dir.path().join(".davinci-transactions");
            std::fs::create_dir(&journals).unwrap();
            std::fs::write(journals.join("old.json"), "old recovery state").unwrap();
            let mut baseline = capture_baseline(dir.path()).unwrap();
            assert!(!baseline
                .files
                .contains_key(".davinci-transactions/old.json"));
            // A persisted baseline from an older build may already contain journals.
            baseline.files.insert(
                ".davinci-transactions/old.json".into(),
                FileFingerprint {
                    hash: "legacy".into(),
                    len: 18,
                },
            );
            baseline.contents.insert(
                ".davinci-transactions/old.json".into(),
                b"old recovery state".to_vec(),
            );
            std::fs::remove_file(journals.join("old.json")).unwrap();
            std::fs::write(journals.join("new.json"), "new recovery state").unwrap();
            std::fs::write(journals.join("active.lock"), "lock").unwrap();
            std::fs::write(dir.path().join("product.rs"), "fn product() {}\n").unwrap();
            let delta = capture_graph_delta(dir.path(), &baseline).unwrap();
            assert_eq!(delta.files, vec![ChangedFile::added("product.rs")]);
            assert!(!delta.diff().contains(".davinci-transactions"));
            assert!(journals.join("new.json").exists());
            assert!(journals.join("active.lock").exists());
        }
    }

    #[test]
    fn graph_mutation_excludes_preexisting_uncommitted_user_edits() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());

        // 1. Initial committed file
        let file_a = dir.path().join("file_a.txt");
        std::fs::write(&file_a, "initial content for a\n").unwrap();
        Command::new("git")
            .args(["add", "file_a.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // 2. Pre-existing dirty uncommitted user edit
        std::fs::write(
            &file_a,
            "initial content for a\nuser dirty uncommitted edit\n",
        )
        .unwrap();

        // 3. Baseline captured immediately before writer mutation
        let baseline = capture_baseline(dir.path()).expect("baseline captured");
        assert!(baseline.files.contains_key("file_a.txt"));

        // 4. Graph writer mutation: edits file_b.txt
        let file_b = dir.path().join("file_b.txt");
        std::fs::write(&file_b, "graph writer created file b\n").unwrap();

        // 5. Compute graph delta
        let delta = capture_graph_delta(dir.path(), &baseline).expect("delta computed");

        // Pre-existing edit in file_a.txt MUST NOT be attributed to the graph!
        assert_eq!(delta.files.len(), 1);
        assert_eq!(delta.files[0].path, "file_b.txt");
        assert_eq!(delta.files[0].status, "added");
        assert!(!delta.files.iter().any(|f| f.path == "file_a.txt"));

        let diff = delta.diff();
        assert!(diff.contains("file_b.txt"));
        assert!(!diff.contains("user dirty uncommitted edit"));
    }

    #[test]
    fn graph_mutation_captures_graph_modifications_and_deletions() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());

        let file_m = dir.path().join("modify_me.txt");
        let file_d = dir.path().join("delete_me.txt");
        std::fs::write(&file_m, "line 1\nline 2\n").unwrap();
        std::fs::write(&file_d, "temporary\n").unwrap();

        let baseline = capture_baseline(dir.path()).expect("baseline");

        // Writer modifies one, deletes one
        std::fs::write(&file_m, "line 1\nline 2 modified\nline 3\n").unwrap();
        std::fs::remove_file(&file_d).unwrap();

        let delta = capture_graph_delta(dir.path(), &baseline).expect("delta");
        assert_eq!(delta.files.len(), 2);

        let mod_entry = delta
            .files
            .iter()
            .find(|f| f.path == "modify_me.txt")
            .unwrap();
        assert_eq!(mod_entry.status, "modified");

        let del_entry = delta
            .files
            .iter()
            .find(|f| f.path == "delete_me.txt")
            .unwrap();
        assert_eq!(del_entry.status, "deleted");

        let diff = delta.diff();
        assert!(diff.contains("+line 2 modified"));
        assert!(diff.contains("-temporary"));
    }

    #[test]
    fn compute_owned_diff_partitions_correctly() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());

        let baseline = capture_baseline(dir.path()).expect("baseline");

        // Write file_owned.txt and file_unattributed.txt
        let file_owned = dir.path().join("file_owned.txt");
        let file_unatt = dir.path().join("file_unatt.txt");
        std::fs::write(&file_owned, "owned content\n").unwrap();
        std::fs::write(&file_unatt, "dirty content\n").unwrap();

        let report = compute_owned_diff(dir.path(), &baseline, &["file_unatt.txt".to_string()])
            .expect("report");

        assert!(report.has_unattributed);
        assert!(report.owned_diff.contains("file_owned.txt"));
        assert!(!report.owned_diff.contains("file_unatt.txt"));
        assert!(report
            .unattributed_diff
            .as_ref()
            .unwrap()
            .contains("file_unatt.txt"));
    }
}
