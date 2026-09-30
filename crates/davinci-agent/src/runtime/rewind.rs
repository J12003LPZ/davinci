//! Three-way inverse planning, manual-edit conflict detection, and task rewind preview.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

use super::checkpoints::BlobStore;
use super::effects::{ExternalEffectReceipt, OwnedFileEffect};

pub fn restore_kind(before: &str, after: &str, current: &str, overlap: bool) -> &'static str {
    if current == before {
        "already_restored"
    } else if current == after {
        "inverse"
    } else if overlap {
        "conflict"
    } else {
        "three_way_preview"
    }
}

pub fn can_apply_rewind(
    preview_digest: &str,
    current_digest: &str,
    conflict_count: usize,
    mutation_lane_quiescent: bool,
) -> bool {
    preview_digest == current_digest && conflict_count == 0 && mutation_lane_quiescent
}

pub fn restore_domains(code: bool, tasks: bool, transcript: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if code {
        out.push("code");
    }
    if tasks {
        out.push("tasks");
    }
    if transcript {
        out.push("transcript");
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindSelection {
    pub code: bool,
    pub task_state: bool,
    pub transcript: bool,
}

impl RewindSelection {
    pub fn domains(&self) -> Vec<&'static str> {
        restore_domains(self.code, self.task_state, self.transcript)
    }

    pub fn is_empty(&self) -> bool {
        !self.code && !self.task_state && !self.transcript
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRewindPlan {
    pub path: String,
    pub classification: String,
    pub resolved_content: Option<Vec<u8>>,
    pub is_conflict: bool,
    pub conflict_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_rewind_hash: Option<String>,
}

impl FileRewindPlan {
    pub fn new(
        path: impl Into<String>,
        classification: impl Into<String>,
        resolved_content: Option<Vec<u8>>,
        is_conflict: bool,
        conflict_reason: Option<String>,
    ) -> Self {
        Self {
            path: path.into(),
            classification: classification.into(),
            resolved_content,
            is_conflict,
            conflict_reason,
            pre_rewind_hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindPreview {
    pub checkpoint_id: String,
    pub preview_digest: String,
    pub files: Vec<FileRewindPlan>,
    pub conflict_count: usize,
    pub irreversible_effects: Vec<ExternalEffectReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextHunk {
    start: usize,
    end: usize,
    replacement: Vec<String>,
}

#[allow(clippy::needless_range_loop)]
fn compute_hunks(base_lines: &[&str], target_lines: &[&str]) -> Vec<TextHunk> {
    let n = base_lines.len();
    let m = target_lines.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            if base_lines[i] == target_lines[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
            }
        }
    }
    let mut i = n;
    let mut j = m;
    let mut edits = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && base_lines[i - 1] == target_lines[j - 1] {
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            edits.push((i, i, Some(target_lines[j - 1])));
            j -= 1;
        } else if i > 0 && (j == 0 || dp[i][j - 1] < dp[i - 1][j]) {
            edits.push((i - 1, i, None));
            i -= 1;
        }
    }
    edits.reverse();
    let mut hunks: Vec<TextHunk> = Vec::new();
    for (start, end, ins) in edits {
        if let Some(last) = hunks.last_mut() {
            if last.end == start {
                last.end = end;
                if let Some(s) = ins {
                    last.replacement.push(s.to_string());
                }
                continue;
            }
        }
        let replacement = if let Some(s) = ins {
            vec![s.to_string()]
        } else {
            Vec::new()
        };
        hunks.push(TextHunk {
            start,
            end,
            replacement,
        });
    }
    hunks
}

fn three_way_merge_text(
    before_text: &str,
    after_text: &str,
    current_text: &str,
) -> Result<String, String> {
    // Check for ambiguous repeated identical lines in after_text that could make matching unstable
    let after_lines: Vec<&str> = after_text.lines().collect();
    let before_lines: Vec<&str> = before_text.lines().collect();
    let current_lines: Vec<&str> = current_text.lines().collect();

    // Compute hunks from base (after) -> inverse target (before)
    let inverse_hunks = compute_hunks(&after_lines, &before_lines);
    // Compute hunks from base (after) -> current user modifications
    let user_hunks = compute_hunks(&after_lines, &current_lines);

    // Check for overlapping hunks between inverse and user edits
    for ih in &inverse_hunks {
        for uh in &user_hunks {
            let overlaps = if ih.start == ih.end && uh.start == uh.end {
                ih.start == uh.start
            } else {
                ih.start < uh.end && uh.start < ih.end
            };
            if overlaps {
                if ih.replacement == uh.replacement {
                    continue;
                }
                return Err(format!(
                    "Overlapping manual edit conflict at base lines {}..{}",
                    ih.start, ih.end
                ));
            }
        }
    }

    // Check for ambiguous repeated patterns: if base has non-unique identical lines in changed regions
    let mut counts = std::collections::HashMap::new();
    for &line in &after_lines {
        *counts.entry(line).or_insert(0usize) += 1;
    }
    for ih in &inverse_hunks {
        for i in ih.start..ih.end {
            if let Some(line) = after_lines.get(i) {
                if !line.trim().is_empty() && counts.get(line).copied().unwrap_or(0) > 3 {
                    return Err(format!(
                        "Ambiguous anchor: repeated identical line '{line}'"
                    ));
                }
            }
        }
    }

    // Apply all disjoint hunks to after_lines
    let mut all_hunks = Vec::new();
    all_hunks.extend(inverse_hunks);
    all_hunks.extend(user_hunks);
    // Sort descending by start position so replacement doesn't shift earlier indices
    all_hunks.sort_by(|a, b| b.start.cmp(&a.start).then_with(|| b.end.cmp(&a.end)));

    let mut result_lines: Vec<String> = after_lines.iter().map(|s| s.to_string()).collect();
    for hunk in all_hunks {
        let replacement = hunk.replacement;
        result_lines.splice(hunk.start..hunk.end, replacement);
    }

    let has_trailing_newline =
        current_text.ends_with('\n') || before_text.ends_with('\n') || after_text.ends_with('\n');
    let mut out = result_lines.join("\n");
    if has_trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

pub fn plan_file_rewind(
    path: &str,
    before_bytes: Option<&[u8]>,
    after_bytes: Option<&[u8]>,
    current_bytes: Option<&[u8]>,
) -> FileRewindPlan {
    let norm_path = path.replace('\\', "/");

    // Case 1: Task created the file
    if before_bytes.is_none() && after_bytes.is_some() {
        let after = after_bytes.unwrap();
        if current_bytes.is_none() {
            return FileRewindPlan::new(norm_path, "already_restored", None, false, None);
        }
        let current = current_bytes.unwrap();
        if current == after {
            return FileRewindPlan::new(norm_path, "inverse", None, false, None);
        } else {
            return FileRewindPlan::new(
                norm_path,
                "conflict",
                None,
                true,
                Some("File created by task was modified by subsequent manual edits".into()),
            );
        }
    }

    // Case 2: Task deleted the file
    if before_bytes.is_some() && after_bytes.is_none() {
        let before = before_bytes.unwrap();
        if current_bytes.is_none() {
            return FileRewindPlan::new(norm_path, "inverse", Some(before.to_vec()), false, None);
        }
        let current = current_bytes.unwrap();
        if current == before {
            return FileRewindPlan::new(
                norm_path,
                "already_restored",
                Some(before.to_vec()),
                false,
                None,
            );
        } else {
            return FileRewindPlan::new(
                norm_path,
                "conflict",
                None,
                true,
                Some("File deleted by task was recreated with different content".into()),
            );
        }
    }

    // Case 3: Task modified the file
    let before = before_bytes.unwrap_or_default();
    let after = after_bytes.unwrap_or_default();

    let Some(current) = current_bytes else {
        return FileRewindPlan::new(
            norm_path,
            "conflict",
            None,
            true,
            Some("File modified by task was subsequently deleted".into()),
        );
    };

    if current == before {
        return FileRewindPlan::new(
            norm_path,
            "already_restored",
            Some(before.to_vec()),
            false,
            None,
        );
    }

    if current == after {
        return FileRewindPlan::new(norm_path, "inverse", Some(before.to_vec()), false, None);
    }

    // Binary file check
    let is_binary = before.contains(&0)
        || after.contains(&0)
        || current.contains(&0)
        || std::str::from_utf8(before).is_err()
        || std::str::from_utf8(after).is_err()
        || std::str::from_utf8(current).is_err();

    if is_binary {
        return FileRewindPlan::new(
            norm_path,
            "conflict",
            None,
            true,
            Some("Binary file was modified after task".into()),
        );
    }

    let before_str = std::str::from_utf8(before);
    let after_str = std::str::from_utf8(after);
    let current_str = std::str::from_utf8(current);

    match (before_str, after_str, current_str) {
        (Ok(b), Ok(a), Ok(c)) => match three_way_merge_text(b, a, c) {
            Ok(merged) => FileRewindPlan::new(
                norm_path,
                "three_way_preview",
                Some(merged.into_bytes()),
                false,
                None,
            ),
            Err(err) => FileRewindPlan::new(norm_path, "conflict", None, true, Some(err)),
        },
        _ => FileRewindPlan::new(
            norm_path,
            "conflict",
            None,
            true,
            Some("Binary file was modified after task".into()),
        ),
    }
}

pub fn check_rename_conflict(
    _source_path: &str,
    dest_path: &str,
    base_dir: &Path,
) -> Option<String> {
    let dest_full = base_dir.join(dest_path);
    if dest_full.exists() {
        // Check for Windows case-only rename
        #[cfg(windows)]
        {
            if _source_path.eq_ignore_ascii_case(dest_path) {
                return None;
            }
        }
        return Some(format!("Rename destination occupied: {dest_path}"));
    }
    None
}

pub fn validate_rewind_path(
    base_dir: &Path,
    relative_path: &str,
) -> Result<std::path::PathBuf, String> {
    let resolved = crate::apply_patch::sanitize_relative_path(base_dir, relative_path)?;
    let relative = resolved
        .strip_prefix(base_dir)
        .map_err(|error| error.to_string())?;
    let mut component_path = base_dir.to_path_buf();
    // A replacement link can stay inside the workspace while redirecting an
    // owned pathname to an unrelated file whose bytes happen to match. Rewind
    // therefore refuses every symlink below the workspace, including parents.
    for component in relative.components() {
        component_path.push(component);
        match std::fs::symlink_metadata(&component_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("Symlink path rejected by rewind: {relative_path}"));
            }
            Ok(metadata) if component_path == resolved && !metadata.is_file() => {
                return Err(format!(
                    "Non-regular file rejected by rewind: {relative_path}"
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Inspect rewind path {relative_path}: {error}")),
        }
    }
    // Matching bytes do not establish ownership of a shared inode. Check the
    // opened source with the existing Unix/Windows transaction identity guard;
    // unavailable identity or link-count metadata fails closed.
    match std::fs::File::open(&resolved) {
        Ok(file) => super::transactions::require_unaliased_file(&file)
            .map_err(|error| format!("Unsafe rewind file {relative_path}: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Inspect rewind file {relative_path}: {error}")),
    }
    Ok(resolved)
}

fn read_current_rewind_content(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Cannot read current rewind file {}: {error}",
            path.display()
        )),
    }
}

/// Every apply, rollback and recovery write passes the same alias guard just
/// before mutation. The earlier preview/preflight check is not sufficient.
fn restore_rewind_content(
    workspace_root: &Path,
    relative_path: &str,
    content: Option<&[u8]>,
) -> Result<(), String> {
    let target = validate_rewind_path(workspace_root, relative_path)?;
    match content {
        Some(bytes) => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&target, bytes).map_err(|error| error.to_string())
        }
        None => match std::fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn rollback_rewind_entries(workspace_root: &Path, entries: &[RewindJournalEntry]) -> Vec<String> {
    let mut errors = Vec::new();
    for entry in entries.iter().rev() {
        if let Err(error) = restore_rewind_content(
            workspace_root,
            &entry.relative_path,
            entry.expected_pre_rewind_content.as_deref(),
        ) {
            errors.push(format!("{}: {error}", entry.relative_path));
        }
    }
    errors
}

pub fn build_rewind_preview(
    checkpoint_id: impl Into<String>,
    effects: &[OwnedFileEffect],
    blob_store: &BlobStore,
    base_dir: &Path,
    irreversible: Vec<ExternalEffectReceipt>,
) -> RewindPreview {
    let checkpoint_id = checkpoint_id.into();
    let mut files = Vec::new();
    let mut conflict_count = 0;

    // Group effects by normalized path
    let mut by_path: BTreeMap<String, Vec<&OwnedFileEffect>> = BTreeMap::new();
    for effect in effects {
        by_path
            .entry(effect.path.replace('\\', "/"))
            .or_default()
            .push(effect);
    }

    for (path, file_effects) in by_path {
        if let Err(err) = validate_rewind_path(base_dir, &path) {
            conflict_count += 1;
            files.push(FileRewindPlan::new(
                path,
                "conflict",
                None,
                true,
                Some(format!("Invalid path: {err}")),
            ));
            continue;
        }

        let first = file_effects.first().unwrap();
        let last = file_effects.last().unwrap();

        // A user (or an untracked shell command) may edit between prompt
        // effects. Collapsing across that gap would silently erase their edit.
        if file_effects
            .windows(2)
            .any(|pair| pair[0].after_blob != pair[1].before_blob)
        {
            conflict_count += 1;
            files.push(FileRewindPlan::new(
                path,
                "conflict",
                None,
                true,
                Some(
                    "Untracked changes between recorded file edits; restore prompts separately"
                        .into(),
                ),
            ));
            continue;
        }

        let before_bytes = first
            .before_blob
            .as_deref()
            .and_then(|h| blob_store.get_blob(h));
        let after_bytes = last
            .after_blob
            .as_deref()
            .and_then(|h| blob_store.get_blob(h));

        let missing_before = first.before_blob.is_some() && before_bytes.is_none();
        let missing_after = last.after_blob.is_some() && after_bytes.is_none();
        if missing_before || missing_after {
            conflict_count += 1;
            files.push(FileRewindPlan::new(
                path,
                "conflict",
                None,
                true,
                Some("The recorded file content is no longer available".into()),
            ));
            continue;
        }

        let current_path = base_dir.join(&path);
        let current_bytes = match read_current_rewind_content(&current_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                conflict_count += 1;
                files.push(FileRewindPlan::new(
                    path,
                    "conflict",
                    None,
                    true,
                    Some(error),
                ));
                continue;
            }
        };
        let pre_rewind_hash = current_bytes
            .as_ref()
            .map(|b| format!("{:x}", Sha256::digest(b)));

        let mut plan = plan_file_rewind(
            &path,
            before_bytes.as_deref(),
            after_bytes.as_deref(),
            current_bytes.as_deref(),
        );
        plan.pre_rewind_hash = pre_rewind_hash;
        if plan.is_conflict {
            conflict_count += 1;
        }
        files.push(plan);
    }

    let mut hasher = Sha256::new();
    hasher.update(checkpoint_id.as_bytes());
    for f in &files {
        hasher.update(f.path.as_bytes());
        hasher.update(f.classification.as_bytes());
        if let Some(h) = &f.pre_rewind_hash {
            hasher.update(h.as_bytes());
        }
        if let Some(content) = &f.resolved_content {
            hasher.update(content);
        }
    }
    let preview_digest = format!("{:x}", hasher.finalize());

    RewindPreview {
        checkpoint_id,
        preview_digest,
        files,
        conflict_count,
        irreversible_effects: irreversible,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindJournalEntry {
    pub relative_path: String,
    pub expected_pre_rewind_content: Option<Vec<u8>>,
    pub target_restored_content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindJournal {
    pub timestamp: u64,
    pub checkpoint_id: String,
    pub preview_digest: String,
    pub entries: Vec<RewindJournalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindTransactionReport {
    pub checkpoint_id: String,
    pub restored_count: usize,
}

pub fn has_incomplete_rewind_journal(workspace_root: &Path) -> bool {
    workspace_root
        .join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME)
        .exists()
        || workspace_root
            .join(crate::apply_patch::LEGACY_REWIND_JOURNAL_FILE_NAME)
            .exists()
}

pub fn apply_rewind_transaction(
    workspace_root: &Path,
    preview: &RewindPreview,
    mutation_lane_quiescent: bool,
) -> Result<RewindTransactionReport, String> {
    if !mutation_lane_quiescent {
        return Err("Mutation lane is not quiescent; cannot apply rewind".into());
    }

    if preview.conflict_count > 0 {
        return Err(format!(
            "Cannot apply rewind: {} unresolved conflict(s) present",
            preview.conflict_count
        ));
    }

    let patch_journal = workspace_root.join(crate::apply_patch::JOURNAL_FILE_NAME);
    let legacy_patch = workspace_root.join(crate::apply_patch::LEGACY_JOURNAL_FILE_NAME);
    let rewind_journal = workspace_root.join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
    let legacy_rewind = workspace_root.join(crate::apply_patch::LEGACY_REWIND_JOURNAL_FILE_NAME);

    if patch_journal.exists()
        || legacy_patch.exists()
        || rewind_journal.exists()
        || legacy_rewind.exists()
    {
        return Err(
            "An existing patch or rewind journal requires explicit, authorized recovery; no files changed"
                .into(),
        );
    }

    let mut journal_entries = Vec::new();
    let mut planned_mutations = Vec::new();

    for f in &preview.files {
        let target = validate_rewind_path(workspace_root, &f.path)?;

        if f.is_conflict {
            return Err(format!(
                "Unresolved conflict in {}: {}",
                f.path,
                f.conflict_reason.as_deref().unwrap_or("unknown conflict")
            ));
        }

        let current_bytes = read_current_rewind_content(&target)?;
        let current_hash = current_bytes
            .as_ref()
            .map(|b| format!("{:x}", Sha256::digest(b)));
        if current_hash != f.pre_rewind_hash {
            return Err(format!(
                "Stale preview: {} was modified after preview generation",
                f.path
            ));
        }

        if f.classification == "already_restored" {
            continue;
        }

        journal_entries.push(RewindJournalEntry {
            relative_path: f.path.clone(),
            expected_pre_rewind_content: current_bytes,
            target_restored_content: f.resolved_content.clone(),
        });
        planned_mutations.push((target, f.clone()));
    }

    let journal = RewindJournal {
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        checkpoint_id: preview.checkpoint_id.clone(),
        preview_digest: preview.preview_digest.clone(),
        entries: journal_entries,
    };

    let journal_bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|e| format!("Failed serializing rewind journal: {e}"))?;

    let mut journal_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&rewind_journal)
        .map_err(|e| format!("Failed to create exclusive rewind journal: {e}"))?;

    use std::io::Write;
    journal_file
        .write_all(&journal_bytes)
        .and_then(|_| journal_file.sync_all())
        .map_err(|e| format!("Failed to persist rewind journal: {e}"))?;
    drop(journal_file);

    let mut applied_so_far: Vec<RewindJournalEntry> = Vec::new();
    for (target, plan) in planned_mutations {
        let entry = journal
            .entries
            .iter()
            .find(|e| e.relative_path == plan.path)
            .cloned();

        // Revalidate after the journal flush as well as during preflight. A
        // link or content replacement while preparing the transaction must
        // enter rollback instead of redirecting a write to another file.
        let validation = validate_rewind_path(workspace_root, &plan.path).and_then(|_| {
            let current_hash = read_current_rewind_content(&target)?
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
            if current_hash != plan.pre_rewind_hash {
                return Err(format!(
                    "Stale preview: {} changed before mutation",
                    plan.path
                ));
            }
            Ok(())
        });
        let mutation_started = validation.is_ok();
        let mutation_result = match validation {
            Err(error) => Err(error),
            Ok(()) => {
                restore_rewind_content(workspace_root, &plan.path, plan.resolved_content.as_deref())
            }
        };

        if let Err(err) = mutation_result {
            if mutation_started {
                if let Some(e) = entry {
                    applied_so_far.push(e);
                }
            }
            let rollback_errors = rollback_rewind_entries(workspace_root, &applied_so_far);

            if !rollback_errors.is_empty() {
                return Err(format!(
                    "Rewind mutation failed: {err}; rollback incomplete, journal retained: {}",
                    rollback_errors.join("; ")
                ));
            }

            let _ = std::fs::remove_file(&rewind_journal);
            return Err(format!(
                "Rewind mutation failed, rolled back changes: {err}"
            ));
        }

        if let Some(e) = entry {
            applied_so_far.push(e);
        }
    }

    let restored_count = journal.entries.len();
    std::fs::remove_file(&rewind_journal)
        .map_err(|e| format!("Rewind applied but journal cleanup failed: {e}"))?;

    Ok(RewindTransactionReport {
        checkpoint_id: preview.checkpoint_id.clone(),
        restored_count,
    })
}

pub fn recover_incomplete_rewind_journal(workspace_root: &Path) -> Result<String, String> {
    let davinci_journal = workspace_root.join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
    let legacy_journal = workspace_root.join(crate::apply_patch::LEGACY_REWIND_JOURNAL_FILE_NAME);
    let journal_path = if davinci_journal.exists() {
        davinci_journal
    } else if legacy_journal.exists() {
        legacy_journal
    } else {
        return Ok("No incomplete rewind journal found".into());
    };

    let raw = std::fs::read_to_string(&journal_path)
        .map_err(|e| format!("Failed reading rewind journal: {e}"))?;
    let journal: RewindJournal =
        serde_json::from_str(&raw).map_err(|e| format!("Corrupt rewind journal: {e}"))?;

    // Pre-flight check: validate each entry and detect manual conflicts
    for entry in &journal.entries {
        let target = validate_rewind_path(workspace_root, &entry.relative_path)?;
        let current_bytes = read_current_rewind_content(&target)?;
        let matches_pre = current_bytes == entry.expected_pre_rewind_content;
        let matches_target = current_bytes == entry.target_restored_content;
        if !matches_pre && !matches_target {
            return Err(format!(
                "Recovery conflict for {}: file was modified since incomplete transaction; journal retained",
                entry.relative_path
            ));
        }
    }

    // Roll back entries in reverse order
    for entry in journal.entries.iter().rev() {
        let res = restore_rewind_content(
            workspace_root,
            &entry.relative_path,
            entry.expected_pre_rewind_content.as_deref(),
        );

        if let Err(err) = res {
            return Err(format!(
                "Recovery rollback failed for {}: {err}; journal retained",
                entry.relative_path
            ));
        }
    }

    std::fs::remove_file(&journal_path)
        .map_err(|e| format!("Recovery applied but journal cleanup failed: {e}"))?;

    Ok("Successfully recovered and rolled back incomplete rewind transaction".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindOutcome {
    pub checkpoint_id: String,
    pub domains: Vec<String>,
    pub code_restored: bool,
    pub task_state_restored: bool,
    pub transcript_restored: bool,
    pub new_attempt: Option<u32>,
    pub stale_tasks: Vec<crate::runtime::ids::TaskId>,
    pub stale_evidence: Vec<crate::runtime::ids::EvidenceId>,
    pub warning: Option<String>,
}

pub fn execute_rewind(
    workspace_root: &Path,
    selection: &RewindSelection,
    preview: &RewindPreview,
    task_registry: Option<&crate::runtime::tasks::TaskRegistry>,
    target_task_id: Option<crate::runtime::ids::TaskId>,
    living_plan: Option<&mut crate::LivingPlan>,
    mutation_lane_quiescent: bool,
) -> Result<RewindOutcome, String> {
    if selection.is_empty() {
        return Err("Empty rewind selection: at least one domain must be selected".into());
    }

    let domains: Vec<String> = selection
        .domains()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let mut code_restored = false;
    let mut task_state_restored = false;
    let mut transcript_restored = false;
    let mut new_attempt = None;
    let mut stale_tasks = Vec::new();
    let mut stale_evidence = Vec::new();
    let mut warning = None;

    // 1. Code restore (must succeed first before publishing task/plan changes)
    if selection.code {
        apply_rewind_transaction(workspace_root, preview, mutation_lane_quiescent)?;
        code_restored = true;
    }

    // 2. Code-only invalidation
    if selection.code && !selection.task_state {
        if let (Some(registry), Some(task_id)) = (task_registry, target_task_id) {
            let (st, se) = registry
                .invalidate_for_code_rewind(task_id, &preview.checkpoint_id)
                .map_err(|e| e.to_string())?;
            stale_tasks = st;
            stale_evidence = se;
        }
    }

    // 3. State-only warning
    if !selection.code && selection.task_state {
        warning = Some(
            "Code is unchanged; verification and task state may not match current workspace content"
                .into(),
        );
    }

    // 4. Task-state restore
    if selection.task_state {
        task_state_restored = true;
        if let (Some(registry), Some(task_id)) = (task_registry, target_task_id) {
            let (rec, st, se) = registry
                .rewind_task_state(task_id, &preview.checkpoint_id)
                .map_err(|e| e.to_string())?;
            new_attempt = Some(rec.attempt);
            stale_tasks = st;
            stale_evidence = se;
        }
    }

    // 5. Clear plan approval if code or tasks are rewound
    if selection.code || selection.task_state {
        if let Some(plan) = living_plan {
            plan.clear_approval();
        }
    }

    // 6. Transcript restore
    if selection.transcript {
        transcript_restored = true;
    }

    Ok(RewindOutcome {
        checkpoint_id: preview.checkpoint_id.clone(),
        domains,
        code_restored,
        task_state_restored,
        transcript_restored,
        new_attempt,
        stale_tasks,
        stale_evidence,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f04_restore_classification() {
        assert_eq!(restore_kind("before", "after", "after", false), "inverse");
        assert_eq!(
            restore_kind("before", "after", "before", false),
            "already_restored"
        );
        assert_eq!(restore_kind("before", "after", "manual", true), "conflict");
        assert_eq!(
            restore_kind("before", "after", "manual", false),
            "three_way_preview"
        );
    }

    #[test]
    fn test_preexisting_dirty_line_survives() {
        let before = "dirty_header = true\nitem_count = 0\nfooter = true\n";
        let after = "dirty_header = true\nitem_count = 10\nfooter = true\n";
        // User later edited the footer
        let current = "dirty_header = true\nitem_count = 10\nfooter = updated_by_user\n";

        let plan = plan_file_rewind(
            "config.toml",
            Some(before.as_bytes()),
            Some(after.as_bytes()),
            Some(current.as_bytes()),
        );

        assert!(!plan.is_conflict);
        assert_eq!(plan.classification, "three_way_preview");
        let merged = String::from_utf8(plan.resolved_content.unwrap()).unwrap();
        assert_eq!(
            merged,
            "dirty_header = true\nitem_count = 0\nfooter = updated_by_user\n"
        );
    }

    #[test]
    fn test_nonoverlapping_later_manual_line_survives() {
        let before = "A\nB\nC\n";
        let after = "A_task\nB\nC\n";
        let current = "A_task\nB\nC_user\n";

        let plan = plan_file_rewind(
            "file.txt",
            Some(before.as_bytes()),
            Some(after.as_bytes()),
            Some(current.as_bytes()),
        );

        assert!(!plan.is_conflict);
        assert_eq!(plan.classification, "three_way_preview");
        let merged = String::from_utf8(plan.resolved_content.unwrap()).unwrap();
        assert_eq!(merged, "A\nB\nC_user\n");
    }

    #[test]
    fn test_overlapping_manual_line_conflicts() {
        let before = "A\nB\nC\n";
        let after = "A_task\nB\nC\n";
        let current = "A_user\nB\nC\n";

        let plan = plan_file_rewind(
            "file.txt",
            Some(before.as_bytes()),
            Some(after.as_bytes()),
            Some(current.as_bytes()),
        );

        assert!(plan.is_conflict);
        assert_eq!(plan.classification, "conflict");
        assert!(plan.conflict_reason.unwrap().contains("Overlapping"));
    }

    #[test]
    fn test_binary_changed_after_task_conflicts() {
        let before = vec![0x00, 0x01, 0x02];
        let after = vec![0x00, 0x01, 0x03];
        let current = vec![0x00, 0x01, 0x04];

        let plan = plan_file_rewind("data.bin", Some(&before), Some(&after), Some(&current));

        assert!(plan.is_conflict);
        assert_eq!(plan.classification, "conflict");
        assert!(plan.conflict_reason.unwrap().contains("Binary"));
    }

    #[test]
    fn test_user_already_restored() {
        let before = "original text\n";
        let after = "task text\n";
        let current = "original text\n";

        let plan = plan_file_rewind(
            "doc.md",
            Some(before.as_bytes()),
            Some(after.as_bytes()),
            Some(current.as_bytes()),
        );

        assert!(!plan.is_conflict);
        assert_eq!(plan.classification, "already_restored");
    }

    #[test]
    fn test_rename_destination_occupied_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let occupied = dir.path().join("dest.txt");
        std::fs::write(&occupied, "existing content").unwrap();

        let conflict = check_rename_conflict("src.txt", "dest.txt", dir.path());
        assert!(conflict.is_some());
        assert!(conflict.unwrap().contains("Rename destination occupied"));
    }

    #[test]
    fn test_windows_case_only_rename() {
        let dir = tempfile::tempdir().unwrap();
        let occupied = dir.path().join("File.txt");
        std::fs::write(&occupied, "content").unwrap();

        #[cfg(windows)]
        {
            let conflict = check_rename_conflict("file.txt", "File.txt", dir.path());
            assert!(conflict.is_none());
        }
    }

    #[test]
    fn a_missing_before_blob_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let current = b"task-created content";
        std::fs::write(dir.path().join("a.txt"), current).unwrap();

        let blob_store = BlobStore::new();
        let task_id = crate::runtime::ids::TaskId::new();
        let after_blob = blob_store.store_blob_for_task(task_id, current).unwrap();
        let mut effect = OwnedFileEffect::new(
            "op_missing_before",
            crate::runtime::ids::AgentId::new(),
            1,
            "a.txt",
            crate::runtime::effects::FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some("missing-before-blob".into());
        effect.after_blob = Some(after_blob);

        let preview = build_rewind_preview(
            "cp_missing_before",
            &[effect],
            &blob_store,
            dir.path(),
            Vec::new(),
        );
        assert_eq!(preview.files[0].classification, "conflict");
        assert!(preview.files[0].is_conflict);
        assert_eq!(preview.conflict_count, 1);
    }

    #[test]
    fn test_traversal_path_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let blob_store = BlobStore::new();
        let effect = OwnedFileEffect::new(
            "op1",
            crate::runtime::ids::AgentId::new(),
            1,
            "../escape.txt",
            crate::runtime::effects::FileEffectKind::Created,
            crate::runtime::ids::TaskId::new(),
        );
        let preview = build_rewind_preview("cp1", &[effect], &blob_store, dir.path(), Vec::new());
        assert_eq!(preview.conflict_count, 1);
        assert!(preview.files[0].is_conflict);
        assert!(preview.files[0]
            .conflict_reason
            .as_ref()
            .unwrap()
            .contains("Invalid path"));
    }

    #[test]
    fn f04_stale_preview() {
        assert!(can_apply_rewind("p1", "p1", 0, true));
        assert!(!can_apply_rewind("p1", "p2", 0, true));
        assert!(!can_apply_rewind("p1", "p1", 1, true));
        assert!(!can_apply_rewind("p1", "p1", 0, false));
    }

    #[test]
    fn test_apply_rewind_transaction_success() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn main() { println!(\"after\"); }\n").unwrap();

        let blob_store = BlobStore::new();
        let task_id = crate::runtime::ids::TaskId::new();
        let before_blob = blob_store
            .store_blob_for_task(task_id, b"fn main() { println!(\"before\"); }\n")
            .unwrap();
        let after_blob = blob_store
            .store_blob_for_task(task_id, b"fn main() { println!(\"after\"); }\n")
            .unwrap();

        let mut effect = OwnedFileEffect::new(
            "op1",
            crate::runtime::ids::AgentId::new(),
            1,
            "code.rs",
            crate::runtime::effects::FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some(before_blob);
        effect.after_blob = Some(after_blob);

        let preview = build_rewind_preview("cp1", &[effect], &blob_store, dir.path(), Vec::new());
        assert_eq!(preview.conflict_count, 0);

        let report = apply_rewind_transaction(dir.path(), &preview, true).unwrap();
        assert_eq!(report.restored_count, 1);
        assert!(!has_incomplete_rewind_journal(dir.path()));

        let restored = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(restored, "fn main() { println!(\"before\"); }\n");
    }

    #[test]
    fn test_edit_between_preview_and_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "task content\n").unwrap();

        let blob_store = BlobStore::new();
        let task_id = crate::runtime::ids::TaskId::new();
        let before_blob = blob_store
            .store_blob_for_task(task_id, b"original content\n")
            .unwrap();
        let after_blob = blob_store
            .store_blob_for_task(task_id, b"task content\n")
            .unwrap();

        let mut effect = OwnedFileEffect::new(
            "op1",
            crate::runtime::ids::AgentId::new(),
            1,
            "code.rs",
            crate::runtime::effects::FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some(before_blob);
        effect.after_blob = Some(after_blob);

        let preview = build_rewind_preview("cp1", &[effect], &blob_store, dir.path(), Vec::new());
        assert_eq!(preview.conflict_count, 0);

        // User makes a manual edit on disk before confirmation
        std::fs::write(&file_path, "user edited content\n").unwrap();

        let result = apply_rewind_transaction(dir.path(), &preview, true);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Stale preview") || err.contains("was modified"));

        // Content remains protected
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "user edited content\n"
        );
        assert!(!has_incomplete_rewind_journal(dir.path()));
    }

    #[test]
    fn test_journal_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let journal_path = dir
            .path()
            .join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
        std::fs::write(&journal_path, "{}").unwrap();

        let preview = RewindPreview {
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            files: Vec::new(),
            conflict_count: 0,
            irreversible_effects: Vec::new(),
        };

        let result = apply_rewind_transaction(dir.path(), &preview, true);
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .contains("requires explicit, authorized recovery"));
    }

    #[test]
    fn test_crash_and_recovery_incomplete_journal() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("recovered.txt");
        // State on disk: partially applied "rewound" content
        std::fs::write(&file_path, "rewound_target_content").unwrap();

        let journal = RewindJournal {
            timestamp: 100,
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            entries: vec![RewindJournalEntry {
                relative_path: "recovered.txt".into(),
                expected_pre_rewind_content: Some(b"expected_pre_rewind".to_vec()),
                target_restored_content: Some(b"rewound_target_content".to_vec()),
            }],
        };
        let journal_path = dir
            .path()
            .join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
        std::fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();

        assert!(has_incomplete_rewind_journal(dir.path()));
        let res = recover_incomplete_rewind_journal(dir.path());
        assert!(res.is_ok());
        assert!(!has_incomplete_rewind_journal(dir.path()));

        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "expected_pre_rewind"
        );
    }

    #[test]
    fn test_recovery_conflict_preserves_journal() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("conflict.txt");
        // User edited the file after the crash!
        std::fs::write(&file_path, "unrelated_user_content").unwrap();

        let journal = RewindJournal {
            timestamp: 100,
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            entries: vec![RewindJournalEntry {
                relative_path: "conflict.txt".into(),
                expected_pre_rewind_content: Some(b"pre".to_vec()),
                target_restored_content: Some(b"target".to_vec()),
            }],
        };
        let journal_path = dir
            .path()
            .join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
        std::fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();

        let res = recover_incomplete_rewind_journal(dir.path());
        assert!(res.is_err());
        assert!(res.err().unwrap().contains("Recovery conflict"));
        // Journal is preserved!
        assert!(has_incomplete_rewind_journal(dir.path()));
        // User content is preserved!
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "unrelated_user_content"
        );
    }

    #[test]
    fn f04_restore_selection() {
        assert_eq!(restore_domains(true, false, false), vec!["code"]);
        assert_eq!(
            restore_domains(false, true, true),
            vec!["tasks", "transcript"]
        );
        assert_eq!(
            restore_domains(true, true, true),
            vec!["code", "tasks", "transcript"]
        );
    }

    #[test]
    fn test_all_seven_nonempty_selections() {
        let s1 = RewindSelection {
            code: true,
            task_state: false,
            transcript: false,
        };
        assert_eq!(s1.domains(), vec!["code"]);
        assert!(!s1.is_empty());

        let s2 = RewindSelection {
            code: false,
            task_state: true,
            transcript: false,
        };
        assert_eq!(s2.domains(), vec!["tasks"]);
        assert!(!s2.is_empty());

        let s3 = RewindSelection {
            code: false,
            task_state: false,
            transcript: true,
        };
        assert_eq!(s3.domains(), vec!["transcript"]);
        assert!(!s3.is_empty());

        let s4 = RewindSelection {
            code: true,
            task_state: true,
            transcript: false,
        };
        assert_eq!(s4.domains(), vec!["code", "tasks"]);
        assert!(!s4.is_empty());

        let s5 = RewindSelection {
            code: true,
            task_state: false,
            transcript: true,
        };
        assert_eq!(s5.domains(), vec!["code", "transcript"]);
        assert!(!s5.is_empty());

        let s6 = RewindSelection {
            code: false,
            task_state: true,
            transcript: true,
        };
        assert_eq!(s6.domains(), vec!["tasks", "transcript"]);
        assert!(!s6.is_empty());

        let s7 = RewindSelection {
            code: true,
            task_state: true,
            transcript: true,
        };
        assert_eq!(s7.domains(), vec!["code", "tasks", "transcript"]);
        assert!(!s7.is_empty());

        let empty = RewindSelection {
            code: false,
            task_state: false,
            transcript: false,
        };
        assert!(empty.is_empty());
        assert!(empty.domains().is_empty());
    }

    #[test]
    fn test_code_failure_prevents_combined_state_publication() {
        let dir = tempfile::tempdir().unwrap();
        let registry = crate::runtime::tasks::TaskRegistry::new();
        let run_id = crate::runtime::ids::RunId::new();
        let mut task = crate::runtime::tasks::TaskRecord::new(run_id, "Task 1");
        let task_id = task.id;
        task.state = crate::runtime::tasks::TaskState::Completed;
        task.attempt = 1;
        registry.create_task(task).unwrap();

        // Preview with a conflict
        let preview = RewindPreview {
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            files: vec![FileRewindPlan::new(
                "test.txt",
                "conflict",
                None,
                true,
                Some("conflict".into()),
            )],
            conflict_count: 1,
            irreversible_effects: Vec::new(),
        };

        let selection = RewindSelection {
            code: true,
            task_state: true,
            transcript: false,
        };
        let res = execute_rewind(
            dir.path(),
            &selection,
            &preview,
            Some(&registry),
            Some(task_id),
            None,
            true,
        );
        assert!(res.is_err());

        // Task attempt must NOT have advanced!
        let t = registry.get_task(&task_id).unwrap();
        assert_eq!(t.attempt, 1);
        assert_eq!(t.state, crate::runtime::tasks::TaskState::Ready);
    }

    #[test]
    fn test_active_plan_approval_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let preview = RewindPreview {
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            files: Vec::new(),
            conflict_count: 0,
            irreversible_effects: Vec::new(),
        };

        let mut plan = crate::LivingPlan {
            revision: 2,
            approved_revision: Some(2),
            ..Default::default()
        };

        let selection = RewindSelection {
            code: false,
            task_state: true,
            transcript: false,
        };
        let outcome = execute_rewind(
            dir.path(),
            &selection,
            &preview,
            None,
            None,
            Some(&mut plan),
            true,
        )
        .unwrap();
        assert!(outcome.task_state_restored);
        assert!(plan.approved_revision.is_none());
    }

    #[test]
    fn test_dependent_evidence_stale() {
        let registry = crate::runtime::tasks::TaskRegistry::new();
        let run_id = crate::runtime::ids::RunId::new();

        let mut t1 = crate::runtime::tasks::TaskRecord::new(run_id, "Root Task");
        let t1_id = t1.id;
        let e1 = crate::runtime::ids::EvidenceId::new();
        t1.attach_evidence(e1).unwrap();
        t1.state = crate::runtime::tasks::TaskState::Completed;
        registry.create_task(t1).unwrap();

        let mut t2 = crate::runtime::tasks::TaskRecord::new(run_id, "Dependent Task");
        t2.dependencies = vec![t1_id];
        let t2_id = t2.id;
        let e2 = crate::runtime::ids::EvidenceId::new();
        t2.attach_evidence(e2).unwrap();
        t2.state = crate::runtime::tasks::TaskState::Completed;
        registry.create_task(t2).unwrap();

        let (t1_new, stale_tasks, stale_evidence) =
            registry.rewind_task_state(t1_id, "cp1").unwrap();
        assert_eq!(t1_new.attempt, 1);
        assert_eq!(t1_new.state, crate::runtime::tasks::TaskState::Ready);
        assert!(stale_tasks.contains(&t2_id));
        assert!(stale_evidence.contains(&e1));
        assert!(stale_evidence.contains(&e2));

        let t2_post = registry.get_task(&t2_id).unwrap();
        assert_eq!(t2_post.state, crate::runtime::tasks::TaskState::Blocked);
        assert!(t2_post
            .blocked_reasons
            .iter()
            .any(|r| r.code == "dependency_rewound"));
    }

    #[test]
    fn test_state_only_restore_warning() {
        let dir = tempfile::tempdir().unwrap();
        let preview = RewindPreview {
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            files: Vec::new(),
            conflict_count: 0,
            irreversible_effects: Vec::new(),
        };

        let selection = RewindSelection {
            code: false,
            task_state: true,
            transcript: false,
        };
        let outcome =
            execute_rewind(dir.path(), &selection, &preview, None, None, None, true).unwrap();
        assert!(outcome.warning.is_some());
        assert!(outcome.warning.unwrap().contains("Code is unchanged"));
    }
}

/// Only direct file tools have preimages. Shell commands and external effects
/// cannot be restored by normal-conversation rewind.
pub const SHELL_REWIND_LIMITATION: &str =
    "Changes made through shell commands are not tracked and cannot be restored by rewind.";
const PROMPT_REWIND_ENTRY: &str = "davinci.prompt_rewind.v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PromptRewindState {
    #[serde(skip)]
    pub binding: Option<String>,
    checkpoints: Vec<PromptCheckpoint>,
    #[serde(default)]
    active_checkpoint: Option<String>,
    #[serde(default)]
    report_source: Option<std::path::PathBuf>,
    restored_effects: std::collections::BTreeSet<usize>,
    #[serde(skip)]
    authority: Option<RewindPreview>,
    #[serde(skip)]
    persistence_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptCheckpoint {
    pub id: String,
    pub prompt: String,
    pub created_at_ms: u64,
    pub workspace: std::path::PathBuf,
    pub conversation_parent: Option<String>,
    pub user_entry_id: Option<String>,
    pub effect_start: usize,
    pub effect_end: Option<usize>,
    #[serde(skip)]
    messages_before: Vec<davinci_ai::ChatMessage>,
}

pub(crate) type PreparedPromptRewind = (PromptRewindState, BlobStore, Vec<OwnedFileEffect>);

/// Validate rewind data before the host replaces its active conversation.
pub(crate) fn prepare_prompt_rewind(
    session: &davinci_session::JsonlSession,
) -> Result<PreparedPromptRewind, String> {
    let mut state =
        davinci_session::build_session_path(&session.entries, session.leaf_id.as_deref())
            .into_iter()
            .rev()
            .find(|entry| entry.custom_type.as_deref() == Some(PROMPT_REWIND_ENTRY))
            .map(|entry| {
                serde_json::from_value::<PromptRewindState>(
                    entry.extra.get("data").cloned().unwrap_or_default(),
                )
            })
            .transpose()
            .map_err(|error| format!("Invalid prompt rewind checkpoint: {error}"))?
            .unwrap_or_default();
    let report_path = session.path.with_extension("rewind-effects.jsonl");
    let inherited = state
        .report_source
        .as_ref()
        .filter(|source| **source != report_path)
        .cloned();
    let reports = if report_path.exists() {
        super::effects::read_effect_report(&report_path)?
    } else if let Some(source) = &inherited {
        // Forks and clones inherit settled checkpoint metadata. Read only the
        // associated effect report in the same session storage namespace.
        let expected_parent = session
            .path
            .parent()
            .ok_or("Session storage directory missing")?;
        let source_parent = source
            .parent()
            .ok_or("Rewind effect source directory missing")?;
        if std::fs::canonicalize(expected_parent).map_err(|error| error.to_string())?
            != std::fs::canonicalize(source_parent).map_err(|error| error.to_string())?
            || !source
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".rewind-effects.jsonl"))
        {
            return Err("Inherited rewind effect report is outside this session namespace".into());
        }
        if source.exists() {
            super::effects::read_effect_report(source)?
        } else {
            return Err("Inherited prompt rewind effect report is missing".into());
        }
    } else {
        Vec::new()
    };
    if state.report_source.is_some() && !report_path.exists() && inherited.is_none() {
        return Err("Prompt rewind effect report is missing; restore it before reopening".into());
    }
    // A historical tree snapshot can precede the end marker while the shared
    // report already contains later branch effects. Its unfinished range has
    // no authoritative boundary, even when it stays in the original session.
    state
        .checkpoints
        .retain(|checkpoint| checkpoint.effect_end.is_some());
    if inherited.is_some()
        || state.active_checkpoint.as_ref().is_some_and(|active| {
            !state
                .checkpoints
                .iter()
                .any(|checkpoint| &checkpoint.id == active)
        })
    {
        state.active_checkpoint = None;
    }
    if state.checkpoints.iter().any(|checkpoint| {
        checkpoint.effect_start > reports.len()
            || checkpoint
                .effect_end
                .is_some_and(|end| end < checkpoint.effect_start || end > reports.len())
    }) {
        return Err(
            "Prompt rewind effects are missing; original session report is required".into(),
        );
    }
    let store = BlobStore::new();
    for report in &reports {
        for bytes in [
            report.before_bytes.as_deref(),
            report.after_bytes.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            store
                .store_blob_for_task(report.effect.task_id, bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    if inherited.is_some() && !report_path.exists() {
        super::effects::ensure_effect_report(&report_path)?;
        for report in &reports {
            super::effects::append_effect_report(
                &report_path,
                &report.effect,
                report.before_bytes.as_deref(),
                report.after_bytes.as_deref(),
            )?;
        }
    }
    state.binding = Some(format!("{}:{}", session.header.id, session.path.display()));
    state.report_source = Some(report_path);
    Ok((
        state,
        store,
        reports.into_iter().map(|report| report.effect).collect(),
    ))
}

impl crate::Agent {
    pub(crate) fn prompt_rewind_binding(&self) -> Option<String> {
        self.session
            .as_ref()
            .map(|session| format!("{}:{}", session.header.id, session.path.display()))
    }

    pub(crate) fn begin_prompt_checkpoint(&mut self, text: &str) -> PromptCheckpoint {
        let binding = self.prompt_rewind_binding();
        if self.prompt_rewind.binding != binding {
            self.prompt_rewind = PromptRewindState {
                binding,
                ..Default::default()
            };
        }
        if self.runtime.is_none() {
            let bus = super::RuntimeBus::new();
            let runtime = super::RuntimeHandle::new(
                super::RunId::new(),
                self.tool_context.transaction_owner.agent_id,
                bus.clone(),
            );
            let identity = runtime.file_capture_identity();
            self.set_runtime(runtime);
            self.prompt_checkpoint_bus = Some((identity, bus));
        }
        if let Some(path) = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.prompt_effect_report_path.as_ref())
        {
            if let Err(error) = super::effects::ensure_effect_report(path) {
                self.prompt_rewind.persistence_error = Some(error);
            }
        }
        if let Err(error) = self.settle_prompt_checkpoint() {
            self.prompt_rewind.persistence_error = Some(error);
        }
        self.prompt_rewind.authority = None;
        let effect_start = self
            .runtime
            .as_ref()
            .and_then(|runtime| {
                runtime
                    .effect_ledger
                    .read()
                    .ok()
                    .map(|effects| effects.len())
            })
            .unwrap_or(0);
        PromptCheckpoint {
            id: uuid::Uuid::new_v4().to_string(),
            // Bound picker labels; the actual prompt remains in the session tree.
            prompt: text.chars().take(240).collect(),
            created_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
            workspace: std::fs::canonicalize(&self.cwd).unwrap_or_else(|_| self.cwd.clone()),
            conversation_parent: self
                .session
                .as_ref()
                .and_then(|session| session.leaf_id.clone()),
            user_entry_id: None,
            effect_start,
            effect_end: None,
            messages_before: if self.session.is_none() {
                self.messages.clone()
            } else {
                Vec::new()
            },
        }
    }

    pub(crate) fn record_prompt_checkpoint(&mut self, mut checkpoint: PromptCheckpoint) {
        checkpoint.user_entry_id = self
            .session
            .as_ref()
            .and_then(|session| session.leaf_id.clone());
        self.prompt_rewind.active_checkpoint = Some(checkpoint.id.clone());
        self.prompt_rewind.checkpoints.push(checkpoint);
        if let Err(error) = self.persist_prompt_checkpoints() {
            self.prompt_rewind.persistence_error = Some(error);
        }
    }

    fn persist_prompt_checkpoints(&mut self) -> Result<(), String> {
        self.prompt_rewind.report_source = self
            .session
            .as_ref()
            .map(|session| session.path.with_extension("rewind-effects.jsonl"));
        let data = serde_json::to_value(&self.prompt_rewind).map_err(|error| error.to_string())?;
        if let Some(session) = &mut self.session {
            session
                .append_entry(davinci_session::SessionEntry {
                    id: String::new(),
                    entry_type: "custom".into(),
                    parent_id: None,
                    seq: 0,
                    timestamp: 0,
                    message: None,
                    custom_type: Some(PROMPT_REWIND_ENTRY.into()),
                    extra: serde_json::Map::from_iter([("data".into(), data)]),
                })
                .map_err(|error| format!("Prompt rewind persistence failed: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn settle_prompt_checkpoint(&mut self) -> Result<(), String> {
        if let Some(error) = &self.prompt_rewind.persistence_error {
            return Err(error.clone());
        }
        let end = self
            .runtime
            .as_ref()
            .map(|runtime| {
                runtime
                    .effect_ledger
                    .read()
                    .map(|effects| effects.len())
                    .map_err(|_| "Rewind effect ledger unavailable".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        if let Some(checkpoint) = self.prompt_rewind.checkpoints.last_mut() {
            if self.prompt_rewind.active_checkpoint.as_deref() == Some(checkpoint.id.as_str())
                && checkpoint.effect_end != Some(end)
            {
                checkpoint.effect_end = Some(end);
                self.prompt_rewind.authority = None;
                self.persist_prompt_checkpoints()?;
            }
        }
        Ok(())
    }

    pub(crate) fn require_prompt_checkpoint_persistence(&self) -> Result<(), String> {
        self.prompt_rewind
            .persistence_error
            .clone()
            .map_or(Ok(()), Err)
    }

    pub(crate) fn restore_prompt_checkpoints(&mut self) -> Result<(), String> {
        let Some(session) = &self.session else {
            self.prompt_rewind = PromptRewindState::default();
            return Ok(());
        };
        let prepared = prepare_prompt_rewind(session)?;
        self.install_prompt_checkpoints(prepared);
        Ok(())
    }

    pub(crate) fn install_prompt_checkpoints(&mut self, prepared: PreparedPromptRewind) {
        let (state, blobs, effects) = prepared;
        if let Some(runtime) = &mut self.runtime {
            runtime.blob_store = blobs;
            runtime.effect_ledger = std::sync::Arc::new(std::sync::RwLock::new(effects));
        }
        self.tool_context.runtime = self.runtime.clone();
        self.prompt_rewind = state;
    }

    pub fn prompt_checkpoints(&self) -> Vec<PromptCheckpoint> {
        self.prompt_rewind
            .checkpoints
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    fn require_rewind_idle(&self) -> Result<(), String> {
        if self.prompt_rewind.binding != self.prompt_rewind_binding() {
            return Err("Rewind checkpoints belong to a different session".into());
        }
        if self.is_streaming || self.is_compacting {
            return Err("Cannot rewind while the agent is processing".into());
        }
        if self
            .tool_context
            .jobs
            .lock()
            .map_err(|_| "Job state unavailable")?
            .running()
            > 0
        {
            return Err("Cannot rewind while background shell jobs are running".into());
        }
        if let Some(runtime) = &self.runtime {
            if runtime.registry.snapshot().iter().any(|record| {
                record.id != runtime.agent_id
                    && !matches!(
                        record.state,
                        super::events::AgentState::Completed
                            | super::events::AgentState::Failed
                            | super::events::AgentState::Cancelled
                    )
            }) {
                return Err("Cannot rewind while workers are alive; stop them first".into());
            }
        }
        if let Some(error) = &self.prompt_rewind.persistence_error {
            return Err(error.clone());
        }
        Ok(())
    }

    fn compute_prompt_rewind_preview(&self, checkpoint_id: &str) -> Result<RewindPreview, String> {
        self.require_rewind_idle()?;
        let position = self
            .prompt_rewind
            .checkpoints
            .iter()
            .position(|checkpoint| checkpoint.id == checkpoint_id)
            .ok_or("Prompt checkpoint not found on the current branch")?;
        let workspace = std::fs::canonicalize(&self.cwd).map_err(|error| error.to_string())?;
        if self.prompt_rewind.checkpoints[position..]
            .iter()
            .any(|checkpoint| checkpoint.workspace != workspace)
        {
            return Err(
                "Rewind checkpoints belong to a different workspace; switch back before rewinding"
                    .into(),
            );
        }
        let runtime = self.runtime.as_ref().ok_or("Rewind runtime unavailable")?;
        let ledger = runtime
            .effect_ledger
            .read()
            .map_err(|_| "Rewind effect ledger unavailable")?;
        let mut effects = Vec::new();
        for checkpoint in &self.prompt_rewind.checkpoints[position..] {
            let end = checkpoint.effect_end.unwrap_or(ledger.len());
            if checkpoint.effect_start > end || end > ledger.len() {
                return Err("Prompt checkpoint effect range unavailable".into());
            }
            effects.extend(
                ledger[checkpoint.effect_start..end]
                    .iter()
                    .enumerate()
                    .filter(|(offset, _)| {
                        !self
                            .prompt_rewind
                            .restored_effects
                            .contains(&(checkpoint.effect_start + offset))
                    })
                    .map(|(_, effect)| effect.clone()),
            );
        }
        let mut preview = build_rewind_preview(
            checkpoint_id,
            &effects,
            &runtime.blob_store,
            &self.cwd,
            Vec::new(),
        );
        // File digest alone is insufficient for conversation-only rewinds. Bind
        // host authority to the branch, checkpoint ranges, transcript and cwd.
        let mut digest = Sha256::new();
        digest.update(preview.preview_digest.as_bytes());
        // Even conflicts must become stale when their current bytes change;
        // some conflict plans intentionally have no resolved-content hash.
        for file in &preview.files {
            if let Ok(path) = validate_rewind_path(&self.cwd, &file.path) {
                digest.update(file.path.as_bytes());
                match std::fs::read(path) {
                    Ok(bytes) => {
                        digest.update([1]);
                        digest.update(Sha256::digest(bytes));
                    }
                    Err(error) => {
                        digest.update([0]);
                        digest.update(format!("{:?}", error.kind()).as_bytes());
                    }
                }
            }
        }
        digest.update(serde_json::to_vec(&self.prompt_rewind).map_err(|error| error.to_string())?);
        digest.update(serde_json::to_vec(&self.messages).map_err(|error| error.to_string())?);
        digest.update(self.cwd.to_string_lossy().as_bytes());
        digest.update(self.prompt_rewind_binding().unwrap_or_default().as_bytes());
        digest.update(
            self.session
                .as_ref()
                .and_then(|session| session.leaf_id.as_deref())
                .unwrap_or_default()
                .as_bytes(),
        );
        digest.update(ledger.len().to_le_bytes());
        preview.preview_digest = format!("{:x}", digest.finalize());
        Ok(preview)
    }

    /// Mint host-owned authority. Apply accepts only this preview's digest.
    pub fn preview_prompt_rewind(&mut self, checkpoint_id: &str) -> Result<RewindPreview, String> {
        self.require_rewind_idle()?;
        self.settle_prompt_checkpoint()?;
        let preview = self.compute_prompt_rewind_preview(checkpoint_id)?;
        self.prompt_rewind.authority = Some(preview.clone());
        Ok(preview)
    }

    pub fn apply_prompt_rewind(
        &mut self,
        checkpoint_id: &str,
        preview_digest: &str,
        selection: &RewindSelection,
    ) -> Result<RewindOutcome, String> {
        self.require_rewind_idle()?;
        if selection.is_empty() || selection.task_state {
            return Err("Select code, conversation, or both for a prompt rewind".into());
        }
        let approved = self
            .prompt_rewind
            .authority
            .as_ref()
            .ok_or("Preview required before applying rewind")?;
        if approved.checkpoint_id != checkpoint_id || approved.preview_digest != preview_digest {
            return Err("Rewind preview authority does not match; preview again".into());
        }
        let current = self.compute_prompt_rewind_preview(checkpoint_id)?;
        if &current != approved {
            self.prompt_rewind.authority = None;
            return Err("Rewind preview is stale; preview again".into());
        }
        let position = self
            .prompt_rewind
            .checkpoints
            .iter()
            .position(|checkpoint| checkpoint.id == checkpoint_id)
            .ok_or("Prompt checkpoint not found")?;
        let checkpoint = self.prompt_rewind.checkpoints[position].clone();
        let old_state = self.prompt_rewind.clone();
        let old_leaf = self
            .session
            .as_ref()
            .and_then(|session| session.leaf_id.clone());
        if selection.code {
            apply_rewind_transaction(&self.cwd, &current, true)?;
            if !current.files.is_empty() {
                self.record_successful_mutation_paths(
                    current
                        .files
                        .iter()
                        .map(|file| self.cwd.join(&file.path))
                        .collect(),
                );
                if let Some(runtime) = &self.runtime {
                    runtime.cache.clear_memory(None);
                }
            }
            let end = self
                .runtime
                .as_ref()
                .and_then(|runtime| {
                    runtime
                        .effect_ledger
                        .read()
                        .ok()
                        .map(|effects| effects.len())
                })
                .unwrap_or(0);
            for checkpoint in &self.prompt_rewind.checkpoints[position..] {
                self.prompt_rewind
                    .restored_effects
                    .extend(checkpoint.effect_start..checkpoint.effect_end.unwrap_or(end));
            }
        }
        if selection.transcript {
            self.prompt_rewind.checkpoints.truncate(position);
            if let Some(session) = &mut self.session {
                session.set_leaf(checkpoint.conversation_parent);
            }
        }
        self.prompt_rewind.authority = None;
        self.prompt_rewind.active_checkpoint = None;
        if let Err(error) = self.persist_prompt_checkpoints() {
            self.prompt_rewind = old_state;
            self.prompt_rewind.authority = None;
            self.prompt_rewind.persistence_error = Some(error.clone());
            if let Some(session) = &mut self.session {
                session.set_leaf(old_leaf);
            }
            return Err(format!(
                "{error}; code restoration may have completed; reconcile before retrying"
            ));
        }
        if selection.transcript {
            self.messages = self
                .session
                .as_ref()
                .map(crate::messages_from_session)
                .unwrap_or(checkpoint.messages_before);
            self.pending_prompt_messages.clear();
            self.last_real_user_request =
                crate::last_real_user_request_from_messages(&self.messages);
            self.delegation_forbidden = crate::delegation_forbidden_from_messages(&self.messages);
            self.restore_living_plan();
        }
        Ok(RewindOutcome {
            checkpoint_id: checkpoint_id.into(),
            domains: selection
                .domains()
                .into_iter()
                .map(str::to_string)
                .collect(),
            code_restored: selection.code,
            task_state_restored: false,
            transcript_restored: selection.transcript,
            new_attempt: None,
            stale_tasks: Vec::new(),
            stale_evidence: Vec::new(),
            warning: Some(SHELL_REWIND_LIMITATION.into()),
        })
    }
}

#[cfg(test)]
mod prompt_rewind_tests {
    use super::*;
    use crate::{Agent, PermissionMode};
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    use serde_json::json;
    use tempfile::tempdir;

    pub(super) fn agent_at(path: &Path) -> Agent {
        let mut agent = Agent::new("offline prompt rewind fixture");
        agent.cwd = path.to_path_buf();
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.auto_retry = false;
        agent.auto_compaction = false;
        agent
    }

    pub(super) fn tool_turn(agent: &mut Agent, tool: &str, arguments: serde_json::Value) {
        let mut called = false;
        let call_id = uuid::Uuid::new_v4().to_string();
        // The second fixture response intentionally stops the run. This covers
        // the abort/error cleanup path, without provider calls or retry sleeps.
        let result = agent.run_loop(|_| {
            if called {
                return Err("offline fixture interruption".into());
            }
            called = true;
            Ok(AssistantMessage {
                id: "fixture-response".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::ToolCall {
                    id: call_id.clone(),
                    name: tool.into(),
                    arguments: arguments.clone(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
                extra: Default::default(),
            })
        });
        assert!(result.unwrap_err().contains("offline fixture interruption"));
        assert!(!agent.is_streaming);
        assert!(
            agent
                .messages
                .iter()
                .filter(|message| message.role == "toolResult")
                .all(|message| !message.is_error.unwrap_or(false)),
            "{:?}",
            agent.messages
        );
    }

    fn select(code: bool, transcript: bool) -> RewindSelection {
        RewindSelection {
            code,
            task_state: false,
            transcript,
        }
    }

    #[test]
    fn prompt_rewind_restores_write_edit_patch_and_keeps_code_only_transcript() {
        for tool in ["write", "edit", "apply_patch"] {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("a.txt"), "before\n").unwrap();
            if tool == "apply_patch" {
                std::fs::write(dir.path().join("deleted.txt"), "restore me\n").unwrap();
            }
            let mut agent = agent_at(dir.path());
            agent.prompt("change the file");
            let args = match tool {
                "write" => json!({"path":"a.txt","content":"after\n"}),
                "edit" => json!({"path":"a.txt","oldText":"before","newText":"after"}),
                _ => {
                    json!({"input":"*** Begin Patch\n*** Update File: a.txt\n@@\n-before\n+after\n*** Add File: nested/new.txt\n+new\n*** Delete File: deleted.txt\n*** End Patch"})
                }
            };
            tool_turn(&mut agent, tool, args);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
                "after\n"
            );
            let checkpoint = agent.prompt_checkpoints().remove(0);
            let messages = agent.messages.clone();
            let preview = agent.preview_prompt_rewind(&checkpoint.id).unwrap();
            assert_eq!(preview.conflict_count, 0);
            let outcome = agent
                .apply_prompt_rewind(
                    &checkpoint.id,
                    &preview.preview_digest,
                    &select(true, false),
                )
                .unwrap();
            assert!(outcome.code_restored);
            assert_eq!(agent.messages, messages);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
                "before\n"
            );
            assert!(!dir.path().join("nested/new.txt").exists());
            if tool == "apply_patch" {
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("deleted.txt")).unwrap(),
                    "restore me\n"
                );
            }
        }
    }

    #[test]
    fn prompt_rewind_multiple_prompts_survive_runtime_refresh_and_consumed_edits() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "before\n").unwrap();
        let mut agent = agent_at(dir.path());
        agent.prompt("first");
        let first = agent.prompt_checkpoints()[0].id.clone();
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"one\n"}),
        );
        agent.set_runtime(super::super::RuntimeHandle::new(
            super::super::RunId::new(),
            super::super::AgentId::new(),
            super::super::RuntimeBus::new(),
        ));
        agent.prompt("second");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"two\n"}),
        );
        assert_eq!(agent.prompt_checkpoints().len(), 2);
        let preview = agent.preview_prompt_rewind(&first).unwrap();
        assert_eq!(preview.files.len(), 1);
        agent
            .apply_prompt_rewind(&first, &preview.preview_digest, &select(true, false))
            .unwrap();
        let preview = agent.preview_prompt_rewind(&first).unwrap();
        assert!(
            preview.files.is_empty(),
            "already restored effects must not be replayed"
        );
        agent.prompt("third");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"three\n"}),
        );
        let preview = agent.preview_prompt_rewind(&first).unwrap();
        agent
            .apply_prompt_rewind(&first, &preview.preview_digest, &select(true, true))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "before\n"
        );
        assert!(agent.messages.is_empty());
    }

    #[test]
    fn prompt_rewind_user_conflict_allows_conversation_only() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "before\n").unwrap();
        let mut agent = agent_at(dir.path());
        agent.prompt("first");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"after\n"}),
        );
        std::fs::write(dir.path().join("a.txt"), "manual\n").unwrap();
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert_eq!(preview.conflict_count, 1);
        assert!(agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(true, true))
            .is_err());
        assert!(!agent.messages.is_empty());
        agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(false, true))
            .unwrap();
        assert!(agent.messages.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "manual\n"
        );
    }

    #[test]
    fn prompt_rewind_rejects_user_changes_between_prompts_without_erasing_them() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a\nb\nc\n").unwrap();
        let mut agent = agent_at(dir.path());
        agent.prompt("first");
        let first = agent.prompt_checkpoints()[0].id.clone();
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"A\nb\nc\n"}),
        );
        std::fs::write(dir.path().join("a.txt"), "A\nmanual\nc\n").unwrap();
        agent.prompt("second");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"A\nmanual\nC\n"}),
        );
        let preview = agent.preview_prompt_rewind(&first).unwrap();
        assert_eq!(preview.conflict_count, 1);
        assert!(agent
            .apply_prompt_rewind(&first, &preview.preview_digest, &select(true, false))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "A\nmanual\nC\n"
        );
    }

    #[test]
    fn prompt_rewind_preview_authority_rejects_forgery_stale_files_and_new_conversation() {
        let dir = tempdir().unwrap();
        let mut agent = agent_at(dir.path());
        agent.prompt("first");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"after\n"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        assert!(agent
            .apply_prompt_rewind(&id, "fake", &select(false, true))
            .is_err());
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert!(agent
            .apply_prompt_rewind(&id, "fake", &select(true, false))
            .is_err());
        std::fs::write(dir.path().join("a.txt"), "manual\n").unwrap();
        assert!(agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(false, true))
            .unwrap_err()
            .contains("stale"));
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        agent.prompt_with("synthetic hook feedback", &[]);
        assert_eq!(agent.prompt_checkpoints().len(), 1);
        assert!(agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(false, true))
            .unwrap_err()
            .contains("stale"));
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        agent.is_streaming = true;
        assert!(agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(false, true))
            .unwrap_err()
            .contains("processing"));
    }

    #[test]
    fn prompt_rewind_aborted_turn_and_branch_survive_session_reopen() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("a.txt"), "before\n").unwrap();
        let session = davinci_session::JsonlSession::create(
            &dir.path().join("sessions"),
            &workspace.to_string_lossy(),
            None,
        )
        .unwrap();
        let path = session.path.clone();
        let mut agent = agent_at(&workspace);
        agent.load_from_session(session).unwrap();
        agent.prompt("first");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"one\n"}),
        );
        agent.prompt("second");
        tool_turn(
            &mut agent,
            "write",
            json!({"path":"a.txt","content":"two\n"}),
        );
        let second = agent.prompt_checkpoints()[0].id.clone();
        drop(agent);
        let mut resumed = agent_at(&workspace);
        resumed
            .load_from_session(davinci_session::JsonlSession::open(&path).unwrap())
            .unwrap();
        let preview = resumed.preview_prompt_rewind(&second).unwrap();
        resumed
            .apply_prompt_rewind(&second, &preview.preview_digest, &select(true, true))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
            "one\n"
        );
        let entries = resumed.session.as_ref().unwrap().entries.len();
        drop(resumed);
        let mut resumed = agent_at(&workspace);
        resumed
            .load_from_session(davinci_session::JsonlSession::open(&path).unwrap())
            .unwrap();
        assert_eq!(resumed.prompt_checkpoints().len(), 1);
        assert_eq!(resumed.session.as_ref().unwrap().entries.len(), entries);
        assert!(resumed
            .messages
            .iter()
            .all(|message| !davinci_ai::content_text(&message.content).contains("second")));
        assert!(
            std::fs::read_to_string(&path).unwrap().contains("second"),
            "abandoned tree must remain durable"
        );
    }

    #[test]
    fn prompt_rewind_shell_changes_remain_untracked_and_limitation_is_disclosed() {
        let dir = tempdir().unwrap();
        let mut agent = agent_at(dir.path());
        agent.prompt("shell-only mutation");
        // A shell-made write has no captured file-tool effect.
        std::fs::write(dir.path().join("shell.txt"), "untracked").unwrap();
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert!(preview.files.is_empty());
        let outcome = agent
            .apply_prompt_rewind(&id, &preview.preview_digest, &select(true, true))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("shell.txt")).unwrap(),
            "untracked"
        );
        assert_eq!(outcome.warning.as_deref(), Some(SHELL_REWIND_LIMITATION));
    }
}

#[cfg(test)]
mod rewind_branch_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_rewind_historical_tree_does_not_claim_later_prompt_effects() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(
            &dir.path().join("sessions"),
            &dir.path().to_string_lossy(),
            None,
        )
        .unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.load_from_session(session).unwrap();
        agent.prompt("first");
        let initial = agent.session.as_ref().unwrap().leaf_id.clone().unwrap();
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"first.txt","content":"one"}),
        );
        agent.prompt("later");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"later.txt","content":"two"}),
        );
        agent
            .navigate_tree_entry(&initial, false, None, false, 0)
            .unwrap();
        assert!(agent.prompt_checkpoints().is_empty());
        agent.prompt("new branch");
        let id = agent.prompt_checkpoints()[0].id.clone();
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"new.txt","content":"three"}),
        );
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert_eq!(preview.files.len(), 1);
        assert_eq!(preview.files[0].path, "new.txt");
    }

    #[test]
    fn prompt_rewind_conversation_only_does_not_claim_abandoned_branch_effects() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.prompt("first");
        let first = agent.prompt_checkpoints()[0].id.clone();
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"first.txt","content":"one"}),
        );
        agent.prompt("abandoned");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"abandoned.txt","content":"two"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        agent
            .apply_prompt_rewind(
                &id,
                &preview.preview_digest,
                &RewindSelection {
                    code: false,
                    task_state: false,
                    transcript: true,
                },
            )
            .unwrap();
        agent.prompt("new branch");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            json!({"path":"new.txt","content":"three"}),
        );
        let preview = agent.preview_prompt_rewind(&first).unwrap();
        assert_eq!(
            preview
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["first.txt", "new.txt"]
        );
        agent
            .apply_prompt_rewind(
                &first,
                &preview.preview_digest,
                &RewindSelection {
                    code: true,
                    task_state: false,
                    transcript: true,
                },
            )
            .unwrap();
        assert!(!dir.path().join("first.txt").exists());
        assert!(!dir.path().join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("abandoned.txt")).unwrap(),
            "two"
        );
    }

    #[test]
    fn prompt_rewind_does_not_change_stable_prompt_or_tool_schema() {
        let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
        let prompt = agent.system_prompt.clone();
        let schema = agent.provider_tool_schema_identity();
        agent.prompt("hello");
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        agent
            .apply_prompt_rewind(
                &id,
                &preview.preview_digest,
                &RewindSelection {
                    code: true,
                    task_state: false,
                    transcript: true,
                },
            )
            .unwrap();
        assert_eq!(agent.system_prompt, prompt);
        assert_eq!(agent.provider_tool_schema_identity(), schema);
    }

    #[cfg(unix)]
    #[test]
    fn prompt_rewind_actual_shell_write_has_no_owned_file_effect() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.prompt("shell write");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "bash",
            json!({"command":"printf untracked > shell.txt"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert!(preview.files.is_empty());
        let result = agent
            .apply_prompt_rewind(
                &id,
                &preview.preview_digest,
                &RewindSelection {
                    code: true,
                    task_state: false,
                    transcript: true,
                },
            )
            .unwrap();
        assert_eq!(result.warning.as_deref(), Some(SHELL_REWIND_LIMITATION));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("shell.txt")).unwrap(),
            "untracked"
        );
    }
}

#[cfg(test)]
mod rewind_persistence_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_rewind_fork_keeps_settled_checkpoint_bytes_and_has_independent_report() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("a.txt"), "before").unwrap();
        let root = dir.path().join("sessions");
        let session =
            davinci_session::JsonlSession::create(&root, &workspace.to_string_lossy(), None)
                .unwrap();
        let mut source = super::prompt_rewind_tests::agent_at(&workspace);
        source.load_from_session(session).unwrap();
        source.prompt("first");
        super::prompt_rewind_tests::tool_turn(
            &mut source,
            "write",
            json!({"path":"a.txt","content":"after"}),
        );
        let id = source.prompt_checkpoints()[0].id.clone();
        let store = source.session.as_ref().unwrap();
        let source_report = store.path.with_extension("rewind-effects.jsonl");
        let fork = store
            .fork(store.leaf_id.as_deref().unwrap(), &root)
            .unwrap();
        let fork_report = fork.path.with_extension("rewind-effects.jsonl");
        source.load_from_session(fork).unwrap();
        assert!(fork_report.is_file());
        assert_ne!(fork_report, source_report);
        let preview = source.preview_prompt_rewind(&id).unwrap();
        source
            .apply_prompt_rewind(
                &id,
                &preview.preview_digest,
                &RewindSelection {
                    code: true,
                    task_state: false,
                    transcript: false,
                },
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
            "before"
        );
        assert_eq!(
            super::super::effects::read_effect_report(&source_report)
                .unwrap()
                .len(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(fork_report).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn prompt_rewind_missing_report_fails_before_session_switch_and_persistence_failure_prevents_model_call(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sessions");
        let first = davinci_session::JsonlSession::create(&root, "/fixture", None).unwrap();
        let first_path = first.path.clone();
        let mut current = crate::Agent::new("fixture");
        current.load_from_session(first).unwrap();
        current.prompt("first");
        current.settle_prompt_checkpoint().unwrap();
        let second = davinci_session::JsonlSession::create(&root, "/fixture", None).unwrap();
        let second_path = second.path.clone();
        let mut other = crate::Agent::new("fixture");
        other.load_from_session(second).unwrap();
        other.prompt("second");
        other.settle_prompt_checkpoint().unwrap();
        drop(other);
        std::fs::remove_file(second_path.with_extension("rewind-effects.jsonl")).unwrap();
        let error = current
            .load_from_session(davinci_session::JsonlSession::open(&second_path).unwrap())
            .unwrap_err();
        assert!(error.contains("report is missing"));
        assert_eq!(current.session.as_ref().unwrap().path, first_path);
        assert_eq!(current.prompt_checkpoints()[0].prompt, "first");

        current.prompt_rewind.persistence_error = Some("fixture durable storage failed".into());
        let result = current.run_loop::<_, davinci_ai::AssistantMessage>(|_| {
            panic!("storage failure must prevent model dispatch")
        });
        assert!(result
            .unwrap_err()
            .contains("fixture durable storage failed"));
    }
}

#[cfg(test)]
mod rewind_workspace_tests {
    #[test]
    fn prompt_rewind_preserves_graph_transaction_owner_when_creating_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.tool_context.transaction_owner.graph_node = Some("writer-1".into());
        let owner = agent.tool_context.transaction_owner.agent_id;
        assert!(agent.runtime.is_none());
        agent.prompt("first writer prompt");
        assert_eq!(agent.runtime.as_ref().unwrap().agent_id, owner);
        // Hosts carry session state from this synthetic rewind runtime. Its
        // identity must be the existing writer owner across later prompts.
        let refreshed = crate::runtime::RuntimeHandle::new(
            crate::runtime::RunId::new(),
            crate::runtime::AgentId::new(),
            crate::runtime::RuntimeBus::new(),
        )
        .with_session_state_from(agent.runtime.as_ref().unwrap());
        agent.set_runtime(refreshed);
        agent.prompt("second writer prompt");
        assert_eq!(agent.runtime.as_ref().unwrap().agent_id, owner);
        assert_eq!(agent.tool_context.transaction_owner.agent_id, owner);
        assert_eq!(
            agent.tool_context.transaction_owner.graph_node.as_deref(),
            Some("writer-1")
        );
    }

    #[test]
    fn prompt_rewind_user_hard_link_alias_is_a_conflict_and_preserves_unowned_file() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("a.txt");
        let unrelated = dir.path().join("b.txt");
        std::fs::write(&owned, "before").unwrap();
        std::fs::write(&unrelated, "after").unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.prompt("change owned file");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            serde_json::json!({"path":"a.txt","content":"after"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        std::fs::remove_file(&owned).unwrap();
        std::fs::hard_link(&unrelated, &owned).unwrap();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        let applied = agent.apply_prompt_rewind(
            &id,
            &preview.preview_digest,
            &super::RewindSelection {
                code: true,
                task_state: false,
                transcript: false,
            },
        );
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "after");
        assert_eq!(preview.conflict_count, 1, "{preview:?}");
        assert!(
            applied.is_err(),
            "user hard-link replacement must refuse apply"
        );
        assert_eq!(std::fs::read_to_string(owned).unwrap(), "after");
    }

    #[test]
    fn prompt_rewind_same_bytes_hard_link_replacement_invalidates_preview_and_engine_apply() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("a.txt");
        let unrelated = dir.path().join("b.txt");
        std::fs::write(&owned, "before").unwrap();
        std::fs::write(&unrelated, "after").unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.prompt("change owned file");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            serde_json::json!({"path":"a.txt","content":"after"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert_eq!(preview.conflict_count, 0);
        std::fs::remove_file(&owned).unwrap();
        std::fs::hard_link(&unrelated, &owned).unwrap();
        let host_apply = agent.apply_prompt_rewind(
            &id,
            &preview.preview_digest,
            &super::RewindSelection {
                code: true,
                task_state: false,
                transcript: false,
            },
        );
        let engine_apply = super::apply_rewind_transaction(dir.path(), &preview, true);
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "after");
        assert!(
            host_apply.is_err(),
            "hard-link replacement must stale the host preview"
        );
        assert!(
            engine_apply.is_err(),
            "raw engine must refuse hard-link replacement"
        );
        assert_eq!(std::fs::read_to_string(owned).unwrap(), "after");
        assert!(!super::has_incomplete_rewind_journal(dir.path()));
    }

    #[test]
    fn rewind_recovery_hard_link_alias_refuses_and_retains_journal() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("a.txt");
        let unrelated = dir.path().join("b.txt");
        std::fs::write(&unrelated, "rewound").unwrap();
        std::fs::hard_link(&unrelated, &owned).unwrap();
        let journal = super::RewindJournal {
            timestamp: 100,
            checkpoint_id: "cp1".into(),
            preview_digest: "digest".into(),
            entries: vec![super::RewindJournalEntry {
                relative_path: "a.txt".into(),
                expected_pre_rewind_content: Some(b"after".to_vec()),
                target_restored_content: Some(b"rewound".to_vec()),
            }],
        };
        let journal_path = dir
            .path()
            .join(crate::apply_patch::REWIND_JOURNAL_FILE_NAME);
        std::fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
        let result = super::recover_incomplete_rewind_journal(dir.path());
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "rewound");
        assert!(
            result.is_err(),
            "recovery must refuse aliased rollback target"
        );
        assert!(
            journal_path.exists(),
            "unsafe recovery must retain its journal"
        );
        assert_eq!(std::fs::read_to_string(owned).unwrap(), "rewound");
    }

    #[test]
    fn rewind_failed_mutation_rollback_hard_link_alias_preserves_unowned_file() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("a.txt");
        let unrelated = dir.path().join("b.txt");
        // A successfully applied entry becomes aliased before a later entry
        // fails. Exercise the exact rollback routine called by that failure.
        std::fs::write(&unrelated, "rewound").unwrap();
        std::fs::hard_link(&unrelated, &owned).unwrap();
        let entries = [super::RewindJournalEntry {
            relative_path: "a.txt".into(),
            expected_pre_rewind_content: Some(b"after".to_vec()),
            target_restored_content: Some(b"rewound".to_vec()),
        }];
        let errors = super::rollback_rewind_entries(dir.path(), &entries);
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "rewound");
        assert_eq!(
            errors.len(),
            1,
            "aliased rollback must retain recovery: {errors:?}"
        );
        assert!(errors[0].contains("linked transaction source denied"));
        assert_eq!(std::fs::read_to_string(owned).unwrap(), "rewound");
    }

    #[cfg(unix)]
    #[test]
    fn prompt_rewind_unreadable_recreated_file_is_not_absent() {
        use std::os::unix::fs::PermissionsExt;
        for stale_apply in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a.txt");
            std::fs::write(&path, "original deleted content").unwrap();
            let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
            agent.prompt("delete owned file");
            super::prompt_rewind_tests::tool_turn(
                &mut agent,
                "apply_patch",
                serde_json::json!({"input":"*** Begin Patch\n*** Delete File: a.txt\n*** End Patch"}),
            );
            let id = agent.prompt_checkpoints()[0].id.clone();
            let clean = agent.preview_prompt_rewind(&id).unwrap();
            assert_eq!(clean.conflict_count, 0);
            std::fs::write(&path, "user recreated content").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o200)).unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            if stale_apply {
                let result = super::apply_rewind_transaction(dir.path(), &clean, true);
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                assert!(result.is_err(), "unreadable recreation must refuse apply");
            } else {
                let conflicted = agent.preview_prompt_rewind(&id).unwrap();
                let result = agent.apply_prompt_rewind(
                    &id,
                    &conflicted.preview_digest,
                    &super::RewindSelection {
                        code: true,
                        task_state: false,
                        transcript: false,
                    },
                );
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                assert_eq!(conflicted.conflict_count, 1, "{conflicted:?}");
                assert!(result.is_err());
            }
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "user recreated content"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn prompt_rewind_user_symlink_alias_is_a_conflict_and_never_changes_its_target() {
        for parent_alias in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let owned = if parent_alias {
                "nested/a.txt"
            } else {
                "a.txt"
            };
            std::fs::create_dir_all(dir.path().join("nested")).unwrap();
            std::fs::create_dir_all(dir.path().join("unrelated")).unwrap();
            std::fs::write(dir.path().join(owned), "before").unwrap();
            let unrelated = dir.path().join("unrelated/a.txt");
            std::fs::write(&unrelated, "after").unwrap();
            let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
            agent.prompt("change owned file");
            super::prompt_rewind_tests::tool_turn(
                &mut agent,
                "write",
                serde_json::json!({"path":owned,"content":"after"}),
            );
            let id = agent.prompt_checkpoints()[0].id.clone();
            if parent_alias {
                std::fs::rename(
                    dir.path().join("nested"),
                    dir.path().join("original-nested"),
                )
                .unwrap();
                std::os::unix::fs::symlink("unrelated", dir.path().join("nested")).unwrap();
            } else {
                std::fs::remove_file(dir.path().join(owned)).unwrap();
                std::os::unix::fs::symlink("unrelated/a.txt", dir.path().join(owned)).unwrap();
            }
            let preview = agent.preview_prompt_rewind(&id).unwrap();
            assert_eq!(preview.conflict_count, 1, "{preview:?}");
            assert!(agent
                .apply_prompt_rewind(
                    &id,
                    &preview.preview_digest,
                    &super::RewindSelection {
                        code: true,
                        task_state: false,
                        transcript: false,
                    },
                )
                .is_err());
            assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "after");
        }
    }

    #[cfg(unix)]
    #[test]
    fn prompt_rewind_same_bytes_symlink_replacement_invalidates_preview_and_engine_apply() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "before").unwrap();
        std::fs::write(dir.path().join("b.txt"), "after").unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(dir.path());
        agent.prompt("change owned file");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            serde_json::json!({"path":"a.txt","content":"after"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        let preview = agent.preview_prompt_rewind(&id).unwrap();
        assert_eq!(preview.conflict_count, 0);
        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        std::os::unix::fs::symlink("b.txt", dir.path().join("a.txt")).unwrap();
        assert!(agent
            .apply_prompt_rewind(
                &id,
                &preview.preview_digest,
                &super::RewindSelection {
                    code: true,
                    task_state: false,
                    transcript: false,
                },
            )
            .is_err());
        assert!(super::apply_rewind_transaction(dir.path(), &preview, true).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "after"
        );
    }

    #[test]
    fn prompt_rewind_refuses_to_restore_a_same_named_file_in_a_different_workspace() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut agent = super::prompt_rewind_tests::agent_at(first.path());
        agent.prompt("first workspace");
        super::prompt_rewind_tests::tool_turn(
            &mut agent,
            "write",
            serde_json::json!({"path":"a.txt","content":"after"}),
        );
        let id = agent.prompt_checkpoints()[0].id.clone();
        std::fs::write(second.path().join("a.txt"), "after").unwrap();
        agent.cwd = second.path().into();
        assert!(agent
            .preview_prompt_rewind(&id)
            .unwrap_err()
            .contains("different workspace"));
        assert_eq!(
            std::fs::read_to_string(second.path().join("a.txt")).unwrap(),
            "after"
        );
    }
}
