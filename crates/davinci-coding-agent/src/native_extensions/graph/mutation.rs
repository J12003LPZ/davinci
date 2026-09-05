//! Graph-owned mutation provenance and delta tracking.
//!
//! Captures a workspace baseline before writer mutation, and computes
//! graph-owned deltas post-mutation so that pre-existing uncommitted user
//! edits in a dirty workspace are not attributed to the graph.

use super::replay::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub contents: BTreeMap<String, Vec<u8>>,
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

fn list_workspace_files(cwd: &Path) -> Vec<String> {
    if cwd.join(".git").exists() {
        let mut list = Vec::new();
        if let Ok(output) = Command::new("git")
            .args(["ls-files"])
            .current_dir(cwd)
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        list.push(normalize_rel_path(trimmed));
                    }
                }
            }
        }
        if let Ok(output) = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .current_dir(cwd)
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        list.push(normalize_rel_path(trimmed));
                    }
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
    let mut files = BTreeMap::new();
    let mut contents = BTreeMap::new();

    let paths = list_workspace_files(cwd);
    for rel_path in paths {
        let full = cwd.join(&rel_path);
        if let Ok(bytes) = std::fs::read(&full) {
            let hash = sha256_hex(&bytes);
            files.insert(
                rel_path.clone(),
                FileFingerprint {
                    hash,
                    len: bytes.len() as u64,
                },
            );
            if bytes.len() <= 512 * 1024 {
                contents.insert(rel_path, bytes);
            }
        }
    }

    Ok(MutationBaseline { files, contents })
}

/// Compute graph-owned delta against a captured baseline.
pub fn capture_graph_delta(
    cwd: &Path,
    baseline: &MutationBaseline,
) -> Result<GraphMutation, String> {
    let current_paths = list_workspace_files(cwd);
    let mut current_map = BTreeMap::new();

    for rel_path in current_paths {
        let full = cwd.join(&rel_path);
        if let Ok(bytes) = std::fs::read(&full) {
            let hash = sha256_hex(&bytes);
            current_map.insert(
                rel_path,
                (
                    FileFingerprint {
                        hash,
                        len: bytes.len() as u64,
                    },
                    bytes,
                ),
            );
        }
    }

    let mut changed_files = Vec::new();
    let mut patch_chunks = Vec::new();

    // Check for added or modified files
    for (path, (fingerprint, current_bytes)) in &current_map {
        match baseline.files.get(path) {
            None => {
                // Newly added by graph
                changed_files.push(ChangedFile::added(path));
                let patch = format_added_file_diff(path, current_bytes);
                patch_chunks.push(PatchChunk {
                    file: path.clone(),
                    patch,
                });
            }
            Some(old_fp) if old_fp.hash != fingerprint.hash => {
                // Modified by graph
                changed_files.push(ChangedFile::modified(path));
                let old_bytes = baseline.contents.get(path).cloned().unwrap_or_default();
                let patch = format_modified_file_diff(path, &old_bytes, current_bytes);
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
        if !current_map.contains_key(old_path) {
            changed_files.push(ChangedFile::deleted(old_path));
            let old_bytes = baseline.contents.get(old_path).cloned().unwrap_or_default();
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
    use super::*;
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
}
