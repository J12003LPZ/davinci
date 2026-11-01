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
    /// Symlinks and junctions: path to link text. Recorded with `read_link`,
    /// never followed, and kept out of `files`. Old checkpoints have none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, String>,
}

impl MutationBaseline {
    pub fn old_bytes(&self, path: &str, blob_dir: &Path) -> Option<Vec<u8>> {
        if let Some(bytes) = self.contents.get(path) {
            return Some(bytes.clone());
        }
        self.files.get(path).and_then(|fingerprint| {
            super::blobs::get(blob_dir, &fingerprint.hash).filter(|bytes| {
                bytes.len() as u64 == fingerprint.len && sha256_hex(bytes) == fingerprint.hash
            })
        })
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchChunk {
    pub file: String,
    pub patch: String,
    /// The patch is a summary: the reviewer did not see the content (oversized
    /// file, or a link whose target is never followed). Not an applyable patch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub content_unavailable: bool,
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
        use crate::native_extensions::ecosystem::risk::{ChangeRisk, RiskReason};
        let mut assessment = crate::native_extensions::ecosystem::risk::assess_change_risk(self);
        // Content nobody saw cannot be called low risk.
        for chunk in self.patch_chunks.iter().filter(|c| c.content_unavailable) {
            if !assessment
                .reasons
                .iter()
                .any(|r| r.surface == "content_unavailable" && r.path == chunk.file)
            {
                assessment.reasons.push(RiskReason {
                    surface: "content_unavailable",
                    path: chunk.file.clone(),
                });
            }
            assessment.level = ChangeRisk::High;
        }
        assessment
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
        // Security scan artifacts belong to the agent directory; an older
        // build could leave them in the repository. Never a graph mutation.
        ".davinci/security-scans/",
        ".pi/security-scans/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn list_workspace_files(cwd: &Path) -> Result<Vec<String>, String> {
    if cwd.join(".git").exists() {
        let mut list = Vec::new();
        for args in [
            vec!["ls-files", "-z"],
            vec!["ls-files", "-z", "--others", "--exclude-standard"],
        ] {
            let output = super::git::run(cwd, &args)?;
            for path in output
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                // A lossy substitute could read a different file. The checkpoint
                // schema uses UTF-8 keys, so fail rather than omit evidence.
                let path = std::str::from_utf8(path).map_err(|_| {
                    "Mutation baseline cannot represent a non-UTF-8 Git filename".to_string()
                })?;
                list.push(path.to_owned());
            }
        }
        list.sort();
        list.dedup();
        Ok(list)
    } else {
        Ok(walk_dir_files(cwd))
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
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            // A link (including a Windows junction) is listed, never entered.
            let is_link = workspace_files::is_redirected(&metadata);
            if metadata.is_dir() && !is_link {
                stack.push(path);
            } else if is_link || metadata.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    result.push(normalize_rel_path(&rel.to_string_lossy()));
                }
            }
        }
    }
    result.sort();
    result
}

use super::workspace_files::{self, Entry};

/// Baseline file bytes up to this size are kept in the blob store.
const BASELINE_BLOB_LIMIT: u64 = 512 * 1024;
/// Largest file whose bytes are read to produce a textual patch.
const PATCH_CONTENT_LIMIT: u64 = 64 * 1024 * 1024;

/// Capture baseline workspace fingerprints and contents before writer mutation.
pub fn capture_baseline(cwd: &Path) -> Result<MutationBaseline, String> {
    let blob_dir = super::blobs::dir(cwd);
    let mut files = BTreeMap::new();
    let mut links = BTreeMap::new();

    let paths = list_workspace_files(cwd)?;
    for rel_path in paths {
        if is_transaction_journal(&rel_path) {
            continue;
        }
        match workspace_files::link_target(cwd, Path::new(&rel_path)) {
            Ok(Some(target)) => {
                links.insert(rel_path, target);
                continue;
            }
            Ok(None) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Cannot read baseline file {rel_path}: {error}"));
            }
        }
        let captured =
            match workspace_files::fingerprint(cwd, Path::new(&rel_path), BASELINE_BLOB_LIMIT) {
                Ok(Entry::Present(captured)) => captured,
                Ok(Entry::Excluded) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!("Cannot read baseline file {rel_path}: {error}"));
                }
            };
        if let Some(bytes) = &captured.bytes {
            super::blobs::put(&blob_dir, &captured.hash, bytes)
                .map_err(|error| error.to_string())?;
        }
        files.insert(
            rel_path,
            FileFingerprint {
                hash: captured.hash,
                len: captured.len,
            },
        );
    }

    Ok(MutationBaseline {
        files,
        contents: BTreeMap::new(),
        links,
    })
}

/// Compute graph-owned delta against a captured baseline.
pub fn capture_graph_delta(
    cwd: &Path,
    baseline: &MutationBaseline,
) -> Result<GraphMutation, String> {
    capture_graph_delta_with_limit(cwd, baseline, PATCH_CONTENT_LIMIT)
}

fn capture_graph_delta_with_limit(
    cwd: &Path,
    baseline: &MutationBaseline,
    patch_limit: u64,
) -> Result<GraphMutation, String> {
    let blob_dir = super::blobs::dir(cwd);
    let current_paths = list_workspace_files(cwd)?;
    let mut current_map = BTreeMap::new();
    let mut current_links = BTreeMap::new();

    for rel_path in current_paths {
        if is_transaction_journal(&rel_path) {
            continue;
        }
        match workspace_files::link_target(cwd, Path::new(&rel_path)) {
            Ok(Some(target)) => {
                current_links.insert(rel_path, target);
                continue;
            }
            Ok(None) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Cannot read mutation file {rel_path}: {error}"));
            }
        }
        let captured = match workspace_files::fingerprint(cwd, Path::new(&rel_path), 0) {
            Ok(Entry::Present(captured)) => captured,
            Ok(Entry::Excluded) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Cannot read mutation file {rel_path}: {error}"));
            }
        };
        current_map.insert(
            rel_path,
            FileFingerprint {
                hash: captured.hash,
                len: captured.len,
            },
        );
    }

    let mut changed_files = Vec::new();
    let mut patch_chunks = Vec::new();

    // Check for added or modified files
    for (path, fingerprint) in &current_map {
        if let Some(old_target) = baseline.links.get(path) {
            // A link replaced by a regular file; reported as that, not as an add.
            changed_files.push(ChangedFile::modified(path));
            patch_chunks.push(link_chunk(
                path,
                &link_side(old_target),
                &file_side(fingerprint),
            ));
            continue;
        }
        match baseline.files.get(path) {
            None => {
                // Newly added by graph
                changed_files.push(ChangedFile::added(path));
                let patch = if fingerprint.len > patch_limit {
                    content_unavailable_summary(path, None, fingerprint, patch_limit)
                } else {
                    let current_bytes = workspace_files::read(cwd, Path::new(path))
                        .map_err(|error| format!("Cannot read mutation file {path}: {error}"))?;
                    format_added_file_diff(path, &current_bytes)
                };
                patch_chunks.push(PatchChunk {
                    file: path.clone(),
                    patch,
                    content_unavailable: fingerprint.len > patch_limit,
                });
            }
            Some(old_fp) if old_fp.hash != fingerprint.hash => {
                // Modified by graph
                changed_files.push(ChangedFile::modified(path));
                let old_bytes = baseline.old_bytes(path, &blob_dir);
                // Without the original the reviewer sees no content change.
                let original_missing = old_bytes.is_none();
                let patch = if fingerprint.len > patch_limit {
                    content_unavailable_summary(path, Some(old_fp), fingerprint, patch_limit)
                } else {
                    let current_bytes = workspace_files::read(cwd, Path::new(path))
                        .map_err(|error| format!("Cannot read mutation file {path}: {error}"))?;
                    match old_bytes {
                        Some(old_bytes) => {
                            format_modified_file_diff(path, &old_bytes, &current_bytes)
                        }
                        None => unavailable_original_summary(path, old_fp, Some(fingerprint)),
                    }
                };
                patch_chunks.push(PatchChunk {
                    file: path.clone(),
                    patch,
                    content_unavailable: fingerprint.len > patch_limit || original_missing,
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
        if let Some(new_target) = current_links.get(old_path) {
            // A regular file replaced by a link; reported as that, not as a deletion.
            changed_files.push(ChangedFile::modified(old_path));
            patch_chunks.push(link_chunk(
                old_path,
                &file_side(&baseline.files[old_path]),
                &link_side(new_target),
            ));
        } else if !current_map.contains_key(old_path) {
            changed_files.push(ChangedFile::deleted(old_path));
            let old_bytes = baseline.old_bytes(old_path, &blob_dir);
            let original_missing = old_bytes.is_none();
            let patch = match old_bytes {
                Some(old_bytes) => format_deleted_file_diff(old_path, &old_bytes),
                None => unavailable_original_summary(old_path, &baseline.files[old_path], None),
            };
            patch_chunks.push(PatchChunk {
                file: old_path.clone(),
                patch,
                content_unavailable: original_missing,
            });
        }
    }

    // Links that were links before or are links now. A path that was or became
    // a regular file was reported above.
    for (path, new_target) in &current_links {
        if baseline.files.contains_key(path) {
            continue;
        }
        match baseline.links.get(path) {
            None => {
                changed_files.push(ChangedFile::added(path));
                patch_chunks.push(link_chunk(path, "absent", &link_side(new_target)));
            }
            Some(old_target) if old_target != new_target => {
                changed_files.push(ChangedFile::modified(path));
                patch_chunks.push(link_chunk(
                    path,
                    &link_side(old_target),
                    &link_side(new_target),
                ));
            }
            Some(_) => {}
        }
    }
    for (path, old_target) in &baseline.links {
        if is_transaction_journal(path)
            || current_links.contains_key(path)
            || current_map.contains_key(path)
        {
            continue;
        }
        changed_files.push(ChangedFile::deleted(path));
        patch_chunks.push(link_chunk(path, &link_side(old_target), "absent"));
    }

    Ok(GraphMutation {
        files: changed_files,
        patch_chunks,
    })
}

fn link_side(target: &str) -> String {
    // Debug-quoted: the link text is workspace-controlled and may hold newlines.
    format!("link -> {target:?}")
}

fn file_side(fingerprint: &FileFingerprint) -> String {
    format!(
        "regular file sha256 {} ({} bytes)",
        fingerprint.hash, fingerprint.len
    )
}

// Evidence, not an applyable patch: the link target is never opened or followed.
fn link_chunk(path: &str, old: &str, new: &str) -> PatchChunk {
    PatchChunk {
        file: path.to_string(),
        patch: format!(
            "diff --git a/{path} b/{path}\nlink change; content unavailable, link targets are not followed\nold: {old}\nnew: {new}\n"
        ),
        content_unavailable: true,
    }
}

// The file was fingerprinted in full but is too large to read into a patch.
fn content_unavailable_summary(
    path: &str,
    old: Option<&FileFingerprint>,
    new: &FileFingerprint,
    limit: u64,
) -> String {
    let mut summary = format!(
        "diff --git a/{path} b/{path}\ncontent unavailable; file exceeds the {limit}-byte patch limit\n"
    );
    if let Some(old) = old {
        summary.push_str(&format!(
            "old sha256 {} ({} bytes)\nmodified; ",
            old.hash, old.len
        ));
    } else {
        summary.push_str("added; ");
    }
    summary.push_str(&format!("new sha256 {} ({} bytes)\n", new.hash, new.len));
    summary
}

// This is evidence, not an applyable textual patch: absence is not empty content.
fn unavailable_original_summary(
    path: &str,
    old: &FileFingerprint,
    new: Option<&FileFingerprint>,
) -> String {
    let mut summary = format!(
        "diff --git a/{path} b/{path}\noriginal bytes unavailable; old sha256 {} ({} bytes)\n",
        old.hash, old.len
    );
    if let Some(new) = new {
        summary.push_str(&format!(
            "modified; new sha256 {} ({} bytes)\n",
            new.hash, new.len
        ));
    } else {
        summary.push_str("deleted\n");
    }
    summary
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

/// For a large file, avoid the O(n*m) LCS matrix. Trim the common prefix and
/// suffix, then emit an index-aligned edit script for the changed middle. It
/// is not minimal, but it carries every changed line and can be chunked by the
/// review pipeline; the old path returned a synthetic header with no content.
fn format_large_modified_file_diff(file: &str, old: &[&str], new: &[&str]) -> String {
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old.len().saturating_sub(prefix)
        && suffix < new.len().saturating_sub(prefix)
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];
    let mut diff = format!(
        "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -{},{} +{},{} @@\n",
        prefix + 1,
        old_mid.len(),
        prefix + 1,
        new_mid.len()
    );
    let (mut i, mut j) = (0usize, 0usize);
    while i < old_mid.len() || j < new_mid.len() {
        if i < old_mid.len() && j < new_mid.len() && old_mid[i] == new_mid[j] {
            diff.push(' ');
            diff.push_str(old_mid[i]);
            diff.push('\n');
            i += 1;
            j += 1;
        } else if i < old_mid.len() && j < new_mid.len() {
            if i + 1 < old_mid.len() && old_mid[i + 1] == new_mid[j] {
                diff.push('-');
                diff.push_str(old_mid[i]);
                diff.push('\n');
                i += 1;
            } else if j + 1 < new_mid.len() && old_mid[i] == new_mid[j + 1] {
                diff.push('+');
                diff.push_str(new_mid[j]);
                diff.push('\n');
                j += 1;
            } else {
                diff.push('-');
                diff.push_str(old_mid[i]);
                diff.push('\n');
                diff.push('+');
                diff.push_str(new_mid[j]);
                diff.push('\n');
                i += 1;
                j += 1;
            }
        } else if i < old_mid.len() {
            diff.push('-');
            diff.push_str(old_mid[i]);
            diff.push('\n');
            i += 1;
        } else {
            diff.push('+');
            diff.push_str(new_mid[j]);
            diff.push('\n');
            j += 1;
        }
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
        return format_large_modified_file_diff(file, &old_lines, &new_lines);
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
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn graph_integrity_git_enumeration_failure_is_not_empty_evidence() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("source.txt"), b"source").unwrap();
        assert!(capture_baseline(dir.path()).is_err());
        assert!(capture_graph_delta(dir.path(), &MutationBaseline::default()).is_err());
    }

    #[test]
    fn graph_integrity_large_and_missing_baselines_are_explicit() {
        for size in [12, 512 * 1024 + 1] {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("large.txt"), vec![b'a'; size]).unwrap();
            let baseline = capture_baseline(dir.path()).unwrap();
            let fp = &baseline.files["large.txt"];
            if size == 12 {
                let blob = super::super::blobs::dir(dir.path())
                    .join(&fp.hash[..2])
                    .join(&fp.hash);
                std::fs::remove_file(blob).unwrap();
            }
            std::fs::write(dir.path().join("large.txt"), b"new contents").unwrap();
            let modified_delta = capture_graph_delta(dir.path(), &baseline).unwrap();
            let modified = modified_delta.diff();
            assert!(
                modified.contains("original bytes unavailable"),
                "{modified}"
            );
            assert!(modified.contains(&fp.hash));
            assert!(modified.contains(&format!("{} bytes", fp.len)));
            assert!(!modified.contains("@@ -0,0"));
            std::fs::remove_file(dir.path().join("large.txt")).unwrap();
            let deleted_delta = capture_graph_delta(dir.path(), &baseline).unwrap();
            let deleted = deleted_delta.diff();
            assert!(deleted.contains("original bytes unavailable"));
            assert!(deleted.contains(&fp.hash));
            // The reviewer saw nothing of the change, so risk must say so.
            for delta in [&modified_delta, &deleted_delta] {
                assert!(delta.patch_chunks.iter().all(|c| c.content_unavailable));
                assert_eq!(
                    delta.assess_risk().level,
                    crate::native_extensions::ecosystem::risk::ChangeRisk::High
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn graph_integrity_baseline_does_not_follow_symlinks() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked-dir")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            dir.path().join("linked-file"),
        )
        .unwrap();
        assert!(capture_baseline(dir.path()).unwrap().files.is_empty());
        // Also exercise Git's tracked/untracked path enumeration.
        super::super::git::run(dir.path(), &["init"]).unwrap();
        assert!(capture_baseline(dir.path()).unwrap().files.is_empty());
        std::fs::write(dir.path().join("plain.txt"), b"plain").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        assert_eq!(baseline.files.keys().collect::<Vec<_>>(), ["plain.txt"]);
        let delta = capture_graph_delta(dir.path(), &baseline).unwrap();
        assert!(delta.files.is_empty(), "{delta:?}");
    }

    #[cfg(unix)]
    #[test]
    fn graph_integrity_unreadable_file_stays_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        std::fs::write(&path, b"secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&path).is_ok() {
            return; // running as root: permissions do not block the read
        }
        let error = capture_baseline(dir.path()).unwrap_err();
        assert!(error.contains("locked.txt"), "{error}");
        assert!(capture_graph_delta(dir.path(), &MutationBaseline::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn graph_integrity_link_changes_are_reported_without_following() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"TOP-SECRET-CONTENT").unwrap();
        std::fs::write(outside.path().join("other"), b"OTHER-CONTENT").unwrap();
        let link = dir.path().join("config");
        let baseline = capture_baseline(dir.path()).unwrap();

        std::os::unix::fs::symlink(outside.path().join("secret"), &link).unwrap();
        let added = capture_graph_delta(dir.path(), &baseline).unwrap();
        assert_eq!(added.files, vec![ChangedFile::added("config")]);
        assert!(added.diff().contains("secret"), "{}", added.diff());
        assert!(added.patch_chunks[0].content_unavailable);

        let with_link = capture_baseline(dir.path()).unwrap();
        assert!(with_link.files.is_empty());
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(outside.path().join("other"), &link).unwrap();
        let retargeted = capture_graph_delta(dir.path(), &with_link).unwrap();
        assert_eq!(retargeted.files, vec![ChangedFile::modified("config")]);
        let diff = retargeted.diff();
        assert!(diff.contains("secret") && diff.contains("other"), "{diff}");

        std::fs::remove_file(&link).unwrap();
        let removed = capture_graph_delta(dir.path(), &with_link).unwrap();
        assert_eq!(removed.files, vec![ChangedFile::deleted("config")]);

        for delta in [&added, &retargeted, &removed] {
            let diff = delta.diff();
            assert!(!diff.contains("TOP-SECRET-CONTENT"), "{diff}");
            assert!(!diff.contains("OTHER-CONTENT"), "{diff}");
        }
    }

    #[test]
    fn old_checkpoint_without_links_still_loads() {
        let old = r#"{"files":{"a.txt":{"hash":"h","len":3}}}"#;
        let baseline: MutationBaseline = serde_json::from_str(old).unwrap();
        assert!(baseline.links.is_empty());
        assert!(!serde_json::to_string(&baseline).unwrap().contains("links"));
        let chunk: PatchChunk = serde_json::from_str(r#"{"file":"a","patch":"p"}"#).unwrap();
        assert!(!chunk.content_unavailable);
    }

    #[test]
    fn unseen_content_raises_risk_to_the_highest_level() {
        use crate::native_extensions::ecosystem::risk::ChangeRisk;
        let mut mutation = GraphMutation {
            files: vec![ChangedFile::added("big.txt")],
            patch_chunks: vec![PatchChunk {
                file: "big.txt".into(),
                patch: "summary".into(),
                content_unavailable: false,
            }],
        };
        assert_eq!(mutation.assess_risk().level, ChangeRisk::Low);
        mutation.patch_chunks[0].content_unavailable = true;
        let assessment = mutation.assess_risk();
        assert_eq!(assessment.level, ChangeRisk::High);
        assert_eq!(assessment.reasons[0].path, "big.txt");
    }

    #[test]
    fn graph_integrity_oversized_file_is_fingerprinted_and_summarized() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), b"0123456789").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        // Same length, different content: the full-content hash still sees it.
        std::fs::write(dir.path().join("big.txt"), b"0123456780").unwrap();
        std::fs::write(dir.path().join("new.txt"), b"abcdefghij").unwrap();
        let delta = capture_graph_delta_with_limit(dir.path(), &baseline, 4).unwrap();
        let diff = delta.diff();
        assert_eq!(delta.files.len(), 2, "{delta:?}");
        assert_eq!(diff.matches("content unavailable").count(), 2, "{diff}");
        assert!(diff.contains(&baseline.files["big.txt"].hash), "{diff}");
        assert!(!diff.contains("0123456780"), "{diff}");
        assert!(delta.patch_chunks.iter().all(|c| c.content_unavailable));
    }

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
    fn a_large_modified_file_still_carries_changed_lines() {
        let old: String = (0..2500).map(|index| format!("line {index}\n")).collect();
        let new = old.replace("line 1250\n", "line 1250 changed\n");
        let diff = format_modified_file_diff("big.rs", old.as_bytes(), new.as_bytes());
        assert!(diff.contains("-line 1250\n"), "{diff}");
        assert!(diff.contains("+line 1250 changed\n"), "{diff}");
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
