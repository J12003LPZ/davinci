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
    crate::apply_patch::sanitize_relative_path(base_dir, relative_path)
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
        let current_bytes = std::fs::read(&current_path).ok();
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

        if let Ok(meta) = target.symlink_metadata() {
            if meta.file_type().is_symlink() {
                let canon = target.canonicalize().map_err(|e| {
                    format!("Symlink traversal validation failed for {}: {e}", f.path)
                })?;
                let root_canon = workspace_root
                    .canonicalize()
                    .map_err(|e| format!("Workspace root canonicalize failed: {e}"))?;
                if !canon.starts_with(&root_canon) {
                    return Err(format!(
                        "Symlink traversal outside workspace root rejected: {}",
                        f.path
                    ));
                }
            }
        }

        if f.is_conflict {
            return Err(format!(
                "Unresolved conflict in {}: {}",
                f.path,
                f.conflict_reason.as_deref().unwrap_or("unknown conflict")
            ));
        }

        let current_bytes = std::fs::read(&target).ok();
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
        planned_mutations.push((target, f.resolved_content.clone()));
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
    for (target, target_content) in planned_mutations {
        let entry = journal
            .entries
            .iter()
            .find(|e| {
                validate_rewind_path(workspace_root, &e.relative_path)
                    .map(|p| p == target)
                    .unwrap_or(false)
            })
            .cloned();

        let mutation_result = match &target_content {
            Some(content) => {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&target, content)
            }
            None => match std::fs::remove_file(&target) {
                Ok(_) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
        };

        if let Err(err) = mutation_result {
            if let Some(e) = entry {
                applied_so_far.push(e);
            }
            let mut rollback_errors = Vec::new();
            for e in applied_so_far.into_iter().rev() {
                if let Ok(t) = validate_rewind_path(workspace_root, &e.relative_path) {
                    let res = match &e.expected_pre_rewind_content {
                        Some(orig) => {
                            if let Some(parent) = t.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            std::fs::write(&t, orig)
                        }
                        None => match std::fs::remove_file(&t) {
                            Ok(_) => Ok(()),
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                            Err(err) => Err(err),
                        },
                    };
                    if let Err(r_err) = res {
                        rollback_errors.push(format!("{}: {r_err}", e.relative_path));
                    }
                }
            }

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
        let current_bytes = std::fs::read(&target).ok();
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
        let target = validate_rewind_path(workspace_root, &entry.relative_path)?;
        let res = match &entry.expected_pre_rewind_content {
            Some(orig) => {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&target, orig)
            }
            None => match std::fs::remove_file(&target) {
                Ok(_) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
        };

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
