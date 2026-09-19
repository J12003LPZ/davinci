//! Codex `apply_patch` grammar parser, transactional commit, and rollback matching §10.1.
//! Preserves applicable MIT attribution from openai/codex.
//!
//! Copyright (c) OpenAI. All rights reserved.
//! Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const JOURNAL_FILE_NAME: &str = ".davinci_patch_journal.json";
pub const LEGACY_JOURNAL_FILE_NAME: &str = ".pi_patch_journal.json";
pub const REWIND_JOURNAL_FILE_NAME: &str = ".davinci_rewind_journal.json";
pub const LEGACY_REWIND_JOURNAL_FILE_NAME: &str = ".pi_rewind_journal.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, hunks: Vec<Hunk> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPatch {
    pub actions: Vec<FileAction>,
    pub raw_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub relative_path: String,
    /// `None` if the file was newly created by the patch.
    pub original_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchJournal {
    pub timestamp: u64,
    pub entries: Vec<JournalEntry>,
}

/// Lexically normalizes paths without filesystem resolution.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Normalizes path strings: rejects absolute paths, path traversal (`..`), and symlink escapes.
pub fn sanitize_relative_path(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("Empty file path in patch".into());
    }
    // Reject absolute paths across platforms (Unix leading '/', Windows drive prefix 'C:', UNC '\\')
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || Path::new(trimmed).is_absolute() {
        return Err(format!("Absolute path rejected: {raw_path}"));
    }
    let p = Path::new(trimmed);
    if p.components()
        .find_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .is_some_and(|name| {
            let trimmed = name.trim_end_matches(['.', ' ']);
            trimmed.eq_ignore_ascii_case(JOURNAL_FILE_NAME)
                || trimmed.eq_ignore_ascii_case(LEGACY_JOURNAL_FILE_NAME)
                || trimmed.eq_ignore_ascii_case(REWIND_JOURNAL_FILE_NAME)
                || trimmed.eq_ignore_ascii_case(LEGACY_REWIND_JOURNAL_FILE_NAME)
        })
    {
        return Err("The patch journal path is reserved".into());
    }
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                return Err(format!("Directory traversal `..` rejected: {raw_path}"));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!("Absolute path rejected: {raw_path}"));
            }
            _ => {}
        }
    }

    let resolved = workspace_root.join(p);

    // Lexical containment check
    let normalized = normalize_lexically(&resolved);
    let norm_root = normalize_lexically(workspace_root);
    if !normalized.starts_with(&norm_root) {
        return Err(format!("Path escapes workspace root: {raw_path}"));
    }

    // Symlink escape check: ensure resolved path (or existing ancestors) never escape workspace root
    if let (Ok(root_canon), Ok(target_canon)) =
        (workspace_root.canonicalize(), resolved.canonicalize())
    {
        if !target_canon.starts_with(&root_canon) {
            return Err(format!(
                "Symlink traversal outside workspace root rejected: {raw_path}"
            ));
        }
    } else if let Ok(root_canon) = workspace_root.canonicalize() {
        let mut ancestor = resolved.as_path();
        while let Some(parent) = ancestor.parent() {
            if parent.exists() {
                if let Ok(parent_canon) = parent.canonicalize() {
                    if !parent_canon.starts_with(&root_canon) {
                        return Err(format!(
                            "Symlink traversal outside workspace root rejected: {raw_path}"
                        ));
                    }
                }
                break;
            }
            ancestor = parent;
        }
    }

    Ok(resolved)
}

/// Checks if a line is a genuine Codex patch control header rather than file content.
fn is_patch_control_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("*** Begin Patch")
        || trimmed.starts_with("*** Add File:")
        || trimmed.starts_with("*** Update File:")
        || trimmed.starts_with("*** Delete File:")
        || trimmed.starts_with("*** End Patch")
}

/// Parses the full `apply_patch` input string according to the Codex grammar.
pub fn parse_codex_patch(input: &str) -> Result<ParsedPatch, String> {
    let trimmed = input.trim();
    if !trimmed.starts_with("*** Begin Patch") {
        return Err("Malformed patch: missing `*** Begin Patch` header".into());
    }
    if !trimmed.ends_with("*** End Patch") && !trimmed.ends_with("*** End Patch ***") {
        return Err("Malformed patch: missing `*** End Patch` footer".into());
    }

    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    let raw_digest = format!("{:x}", hasher.finalize());

    let lines: Vec<&str> = trimmed.lines().collect();
    let mut i = 0;
    while i < lines.len() && !lines[i].starts_with("*** Begin Patch") {
        i += 1;
    }
    i += 1; // skip Begin Patch

    let mut actions = Vec::new();

    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("*** End Patch") {
            break;
        }

        if let Some(rest) = line.strip_prefix("*** Add File:") {
            let path = rest.trim().trim_matches('*').trim().to_string();
            i += 1;
            let mut content_lines = Vec::new();
            while i < lines.len() && !is_patch_control_header(lines[i]) {
                let l = lines[i];
                let body = if let Some(stripped) = l.strip_prefix('+') {
                    stripped
                } else {
                    l
                };
                content_lines.push(body);
                i += 1;
            }
            let mut content = content_lines.join("\n");
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            actions.push(FileAction::Add { path, content });
            continue;
        }

        if let Some(rest) = line.strip_prefix("*** Delete File:") {
            let path = rest.trim().trim_matches('*').trim().to_string();
            i += 1;
            actions.push(FileAction::Delete { path });
            continue;
        }

        if let Some(rest) = line.strip_prefix("*** Update File:") {
            let path = rest.trim().trim_matches('*').trim().to_string();
            i += 1;
            let mut hunks = Vec::new();

            while i < lines.len() && !is_patch_control_header(lines[i]) {
                let cur = lines[i];
                if cur.trim().starts_with("@@") {
                    let header = cur.trim().to_string();
                    i += 1;
                    let mut hunk_lines = Vec::new();
                    while i < lines.len()
                        && !lines[i].trim().starts_with("@@")
                        && !is_patch_control_header(lines[i])
                    {
                        let hl = lines[i];
                        if let Some(rest) = hl.strip_prefix('+') {
                            hunk_lines.push(HunkLine::Add(rest.to_string()));
                        } else if let Some(rest) = hl.strip_prefix('-') {
                            hunk_lines.push(HunkLine::Remove(rest.to_string()));
                        } else if let Some(rest) = hl.strip_prefix(' ') {
                            hunk_lines.push(HunkLine::Context(rest.to_string()));
                        } else {
                            // Unprefixed line treated as context
                            hunk_lines.push(HunkLine::Context(hl.to_string()));
                        }
                        i += 1;
                    }
                    hunks.push(Hunk {
                        header,
                        lines: hunk_lines,
                    });
                } else {
                    i += 1;
                }
            }

            if hunks.is_empty() {
                return Err(format!("Update File for `{path}` had no hunks"));
            }
            actions.push(FileAction::Update { path, hunks });
            continue;
        }

        i += 1;
    }

    if actions.is_empty() {
        return Err("Patch contains no file operations".into());
    }

    Ok(ParsedPatch {
        actions,
        raw_digest,
    })
}

/// Applies a sequence of hunks to the original text. Returns new text or error on context mismatch.
pub fn apply_hunks_to_content(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    let mut file_lines: Vec<&str> = original.lines().collect();
    let had_trailing_newline = original.ends_with('\n');
    let uses_crlf = original.contains("\r\n");
    let line_sep = if uses_crlf { "\r\n" } else { "\n" };
    let mut search_from = 0;

    for hunk in hunks {
        let mut match_pattern = Vec::new();
        for hl in &hunk.lines {
            match hl {
                HunkLine::Context(s) | HunkLine::Remove(s) => match_pattern.push(s.as_str()),
                HunkLine::Add(_) => {}
            }
        }

        if match_pattern.is_empty() {
            // Addition only hunk
            let mut additions: Vec<&str> = hunk
                .lines
                .iter()
                .filter_map(|hl| match hl {
                    HunkLine::Add(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            file_lines.append(&mut additions);
            search_from = file_lines.len();
            continue;
        }

        // Find match position in file_lines: search from search_from first, then fallback
        let pattern_len = match_pattern.len();
        let mut found_index = None;
        if file_lines.len() >= pattern_len {
            for start in search_from..=(file_lines.len() - pattern_len) {
                let window = &file_lines[start..start + pattern_len];
                let matches = window
                    .iter()
                    .zip(&match_pattern)
                    .all(|(actual, expected)| actual.trim_end() == expected.trim_end());
                if matches {
                    found_index = Some(start);
                    break;
                }
            }
            if found_index.is_none() && search_from > 0 {
                let limit = search_from.min(file_lines.len().saturating_sub(pattern_len) + 1);
                for start in 0..limit {
                    let window = &file_lines[start..start + pattern_len];
                    let matches = window
                        .iter()
                        .zip(&match_pattern)
                        .all(|(actual, expected)| actual.trim_end() == expected.trim_end());
                    if matches {
                        found_index = Some(start);
                        break;
                    }
                }
            }
        }

        let start = found_index.ok_or_else(|| {
            format!(
                "Context mismatch for hunk `{}`: pattern not found in file",
                hunk.header
            )
        })?;

        // Reconstruct lines replacement
        let mut new_hunk_lines = Vec::new();
        for hl in &hunk.lines {
            match hl {
                HunkLine::Context(s) => new_hunk_lines.push(s.as_str()),
                HunkLine::Add(s) => new_hunk_lines.push(s.as_str()),
                HunkLine::Remove(_) => {}
            }
        }

        let replacement_len = new_hunk_lines.len();
        file_lines.splice(start..start + pattern_len, new_hunk_lines);
        search_from = start + replacement_len;
    }

    let mut result = file_lines.join(line_sep);
    if (had_trailing_newline || !result.is_empty()) && !result.ends_with('\n') {
        result.push_str(line_sep);
    }
    Ok(result)
}

/// Explicit recovery only: the caller must authorize every journal target.
/// Repository journals are untrusted and must never be replayed by an ordinary patch.
pub fn recover_incomplete_journal_if_any(workspace_root: &Path) -> Result<(), String> {
    for name in [JOURNAL_FILE_NAME, LEGACY_JOURNAL_FILE_NAME] {
        match fs::symlink_metadata(workspace_root.join(name)) {
            Ok(_) => return Err("Legacy journal lacks owned postimage identity; automatic recovery is unsafe. Preserve the journal and reconcile its targets explicitly.".into()),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(format!("Failed inspecting journal: {error}")),
        }
    }
    Ok(())
}

/// Existing grammar and hunk application, backed by the shared transaction lifecycle.
pub fn execute_apply_patch(workspace_root: &Path, input: &str) -> Result<String, String> {
    use crate::runtime::transactions::{TransactionCoordinator, TransactionOwner};
    let coordinator = TransactionCoordinator::new(workspace_root, TransactionOwner::default())?;
    let (changes, message) = prepare_patch(workspace_root, input, |path| {
        let relative = path
            .strip_prefix(workspace_root)
            .map_err(|e| e.to_string())?;
        coordinator.snapshot(relative.to_str().ok_or("patch path is not UTF-8")?)
    })?;
    let preview = coordinator.preview(changes)?;
    let applied = coordinator
        .apply(&preview.id, &|_| Ok(()), None)
        .map_err(|e| format!("{e}; transaction {}", preview.id))?;
    Ok(format!("{message} Transaction: {}.", applied.id))
}

/// Compute all changes against opaque source snapshots; no mutation or unconfined reads.
pub(crate) fn prepare_patch(
    workspace_root: &Path,
    input: &str,
    snapshot: impl Fn(&Path) -> Result<crate::runtime::transactions::SourceSnapshot, String>,
) -> Result<(Vec<crate::runtime::transactions::ProposedChange>, String), String> {
    let parsed = parse_codex_patch(input)?;
    let mut changes = Vec::new();
    let (mut modified, mut added, mut deleted) = (0, 0, 0);
    for action in &parsed.actions {
        let path = match action {
            FileAction::Add { path, .. }
            | FileAction::Delete { path }
            | FileAction::Update { path, .. } => path,
        };
        let target = sanitize_relative_path(workspace_root, path)?;
        let source = snapshot(&target)?;
        let bytes = match action {
            FileAction::Add { content, .. } => {
                added += 1;
                Some(content.as_bytes().to_vec())
            }
            FileAction::Delete { .. } => {
                if source.bytes().is_none() {
                    return Err(format!("File to delete does not exist: {path}"));
                }
                deleted += 1;
                None
            }
            FileAction::Update { hunks, .. } => {
                let updated = apply_hunks_to_content(source.text()?, hunks)?;
                modified += 1;
                Some(updated.into_bytes())
            }
        };
        changes.push(source.change(bytes));
    }
    Ok((
        changes,
        format!(
            "Applied patch (digest: {}): {modified} modified, {added} added, {deleted} deleted.",
            &parsed.raw_digest[..8]
        ),
    ))
}

pub type ByteReplacement = (usize, usize, String);
pub type FileByteReplacements = (PathBuf, Vec<ByteReplacement>);

/// Translates byte-range replacements into updated content, validating non-overlapping ranges
/// and applying replacements in descending byte-offset order.
pub fn apply_byte_replacements(
    original: &str,
    replacements: &[ByteReplacement],
) -> Result<String, String> {
    if replacements.is_empty() {
        return Ok(original.to_string());
    }
    let mut sorted = replacements.to_vec();
    sorted.sort_by_key(|r| r.0);
    for window in sorted.windows(2) {
        if window[0].1 > window[1].0 {
            return Err("Overlapping replacement ranges".into());
        }
    }
    sorted.reverse();
    let mut modified = original.to_string();
    for (start, end, ref new_text) in sorted {
        if start > end || end > modified.len() {
            return Err(format!(
                "Replacement range [{}, {}] out of bounds for document length {}",
                start,
                end,
                modified.len()
            ));
        }
        if !modified.is_char_boundary(start) || !modified.is_char_boundary(end) {
            return Err("Replacement range splits UTF-8 character boundary".into());
        }
        modified.replace_range(start..end, new_text);
    }
    Ok(modified)
}

/// Applies file text replacements transactionally using the journal rollback mechanism.
pub fn apply_transactional_replacements(
    workspace_root: &Path,
    file_replacements: &[FileByteReplacements],
) -> Result<String, String> {
    use crate::runtime::transactions::{TransactionCoordinator, TransactionOwner};
    let coordinator = TransactionCoordinator::new(workspace_root, TransactionOwner::default())?;
    let mut changes = Vec::new();
    for (target, replacements) in file_replacements {
        let relative = target.strip_prefix(workspace_root).unwrap_or(target);
        let relative = relative.to_str().ok_or("replacement path is not UTF-8")?;
        let snapshot = coordinator.snapshot(relative)?;
        let updated = apply_byte_replacements(snapshot.text()?, replacements)?;
        changes.push(snapshot.change(Some(updated.into_bytes())));
    }
    let count = changes.len();
    let preview = coordinator.preview(changes)?;
    let applied = coordinator
        .apply(&preview.id, &|_| Ok(()), None)
        .map_err(|e| format!("{e}; transaction {}", preview.id))?;
    Ok(format!(
        "Applied replacements across {count} files successfully. Transaction: {}.",
        applied.id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_journal(root: &Path, entries: Vec<JournalEntry>) -> PathBuf {
        let path = root.join(JOURNAL_FILE_NAME);
        fs::write(
            &path,
            serde_json::to_vec(&PatchJournal {
                timestamp: 0,
                entries,
            })
            .unwrap(),
        )
        .unwrap();
        path
    }

    #[test]
    fn patch_never_recovers_unrequested_journal_targets() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("unrelated.txt"), "keep").unwrap();
        let journal = write_journal(
            dir.path(),
            vec![JournalEntry {
                relative_path: "unrelated.txt".into(),
                original_content: None,
            }],
        );
        let result = execute_apply_patch(
            dir.path(),
            "*** Begin Patch\n*** Add File: requested.txt\n+new\n*** End Patch",
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("unrelated.txt")).unwrap(),
            "keep"
        );
        assert!(!dir.path().join("requested.txt").exists());
        assert!(journal.exists());
    }

    #[test]
    fn patch_cannot_overwrite_reserved_journal() {
        let dir = tempdir().unwrap();
        let patch =
            format!("*** Begin Patch\n*** Add File: ./{JOURNAL_FILE_NAME}\n+forged\n*** End Patch");
        assert!(execute_apply_patch(dir.path(), &patch).is_err());
        assert!(!dir.path().join(JOURNAL_FILE_NAME).exists());
    }

    #[test]
    fn recovery_validates_all_targets_before_restoring_any() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        let journal = write_journal(
            dir.path(),
            vec![
                JournalEntry {
                    relative_path: "keep.txt".into(),
                    original_content: None,
                },
                JournalEntry {
                    relative_path: "../outside.txt".into(),
                    original_content: None,
                },
            ],
        );
        assert!(recover_incomplete_journal_if_any(dir.path()).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
            "keep"
        );
        assert!(journal.exists());
    }

    #[test]
    fn failed_recovery_preserves_journal() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("directory")).unwrap();
        let journal = write_journal(
            dir.path(),
            vec![JournalEntry {
                relative_path: "directory".into(),
                original_content: Some("original".into()),
            }],
        );
        assert!(recover_incomplete_journal_if_any(dir.path()).is_err());
        assert!(journal.exists());
        assert!(dir.path().join("directory").is_dir());
    }

    #[test]
    fn legacy_recovery_refuses_to_overwrite_unowned_changes() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("changed.txt"), "changed").unwrap();
        let journal = write_journal(
            dir.path(),
            vec![JournalEntry {
                relative_path: "changed.txt".into(),
                original_content: Some("original".into()),
            }],
        );
        assert!(recover_incomplete_journal_if_any(dir.path())
            .unwrap_err()
            .contains("owned postimage"));
        assert_eq!(
            fs::read_to_string(dir.path().join("changed.txt")).unwrap(),
            "changed"
        );
        assert!(journal.exists());
    }

    #[test]
    fn mutation_failure_restores_already_applied_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("blocker"), "not a directory").unwrap();
        let patch = "*** Begin Patch\n*** Add File: first.txt\n+new\n*** Add File: blocker/child.txt\n+cannot create\n*** End Patch";
        let error = execute_apply_patch(dir.path(), patch).unwrap_err();
        assert!(!error.is_empty());
        assert!(!dir.path().join("first.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("blocker")).unwrap(),
            "not a directory"
        );
        assert!(!dir.path().join(JOURNAL_FILE_NAME).exists());
    }

    #[test]
    fn parses_valid_multi_file_patch() {
        let patch = r#"*** Begin Patch
*** Add File: src/new_file.txt
+Hello World
+Second Line
*** Update File: src/main.rs
@@ fn main()
-    println!("old");
+    println!("new");
*** Delete File: obsolete.txt
*** End Patch"#;

        let parsed = parse_codex_patch(patch).unwrap();
        assert_eq!(parsed.actions.len(), 3);
        assert!(
            matches!(&parsed.actions[0], FileAction::Add { path, .. } if path == "src/new_file.txt")
        );
        assert!(
            matches!(&parsed.actions[1], FileAction::Update { path, .. } if path == "src/main.rs")
        );
        assert!(
            matches!(&parsed.actions[2], FileAction::Delete { path } if path == "obsolete.txt")
        );
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempdir().unwrap();
        assert!(sanitize_relative_path(dir.path(), "../secret.txt").is_err());
        assert!(sanitize_relative_path(dir.path(), "foo/../../secret.txt").is_err());
    }

    #[test]
    fn atomic_execution_and_rollback_on_context_mismatch() {
        let dir = tempdir().unwrap();
        let target_file = dir.path().join("code.txt");
        fs::write(&target_file, "line1\nline2\nline3\n").unwrap();

        let bad_patch = r#"*** Begin Patch
*** Update File: code.txt
@@ fn something()
-non_existent_line
+replacement_line
*** End Patch"#;

        let res = execute_apply_patch(dir.path(), bad_patch);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Context mismatch"));

        // Verify original file content was preserved!
        let current = fs::read_to_string(&target_file).unwrap();
        assert_eq!(current, "line1\nline2\nline3\n");
    }

    #[test]
    fn successful_patch_application() {
        let dir = tempdir().unwrap();
        let target_file = dir.path().join("hello.txt");
        fs::write(&target_file, "hello old world\n").unwrap();

        let patch = r#"*** Begin Patch
*** Update File: hello.txt
@@ main
-hello old world
+hello new world
*** Add File: created.txt
+First created line
*** End Patch"#;

        let res = execute_apply_patch(dir.path(), patch).unwrap();
        assert!(res.contains("1 modified, 1 added, 0 deleted"));

        let updated = fs::read_to_string(&target_file).unwrap();
        assert_eq!(updated, "hello new world\n");

        let created = fs::read_to_string(dir.path().join("created.txt")).unwrap();
        assert_eq!(created, "First created line\n");
    }

    #[test]
    fn rejects_absolute_paths() {
        let dir = tempdir().unwrap();
        assert!(sanitize_relative_path(dir.path(), "/etc/passwd").is_err());
        assert!(sanitize_relative_path(dir.path(), "\\Windows\\System32").is_err());
        if cfg!(windows) {
            assert!(sanitize_relative_path(dir.path(), "C:\\secret.txt").is_err());
        }
    }

    #[test]
    fn handles_patch_content_with_asterisks() {
        let patch = r#"*** Begin Patch
*** Add File: docs.md
+***Important Heading***
+Here is a divider:
+***
+And bold text: ***bold***
*** End Patch"#;

        let parsed = parse_codex_patch(patch).unwrap();
        assert_eq!(parsed.actions.len(), 1);
        if let FileAction::Add { content, .. } = &parsed.actions[0] {
            assert!(content.contains("***Important Heading***"));
            assert!(content.contains("And bold text: ***bold***"));
        } else {
            panic!("Expected Add File");
        }
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("windows.txt");
        fs::write(&target, "first line\r\nsecond line\r\nthird line\r\n").unwrap();

        let patch = r#"*** Begin Patch
*** Update File: windows.txt
@@ second line
-second line
+modified line
*** End Patch"#;

        execute_apply_patch(dir.path(), patch).unwrap();
        let updated = fs::read_to_string(&target).unwrap();
        assert!(updated.contains("\r\n"));
        assert_eq!(updated, "first line\r\nmodified line\r\nthird line\r\n");
    }

    #[test]
    fn applies_multiple_sequential_hunks() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("multi.txt");
        fs::write(
            &target,
            "fn a() {\n    return 1;\n}\n\nfn b() {\n    return 2;\n}\n",
        )
        .unwrap();

        let patch = r#"*** Begin Patch
*** Update File: multi.txt
@@ fn a()
-    return 1;
+    return 10;
@@ fn b()
-    return 2;
+    return 20;
*** End Patch"#;

        execute_apply_patch(dir.path(), patch).unwrap();
        let updated = fs::read_to_string(&target).unwrap();
        assert_eq!(
            updated,
            "fn a() {\n    return 10;\n}\n\nfn b() {\n    return 20;\n}\n"
        );
    }

    #[test]
    fn test_apply_byte_replacements_non_overlapping() {
        let text = "alpha beta gamma delta";
        let replacements = vec![(0, 5, "first".to_string()), (11, 16, "third".to_string())];
        let res = apply_byte_replacements(text, &replacements).unwrap();
        assert_eq!(res, "first beta third delta");
    }

    #[test]
    fn test_apply_transactional_replacements_roundtrip() {
        let dir = tempdir().unwrap();
        let file_a = dir.path().join("a.rs");
        let file_b = dir.path().join("b.rs");
        fs::write(&file_a, "let foo = 1;").unwrap();
        fs::write(&file_b, "let foo = foo + 1;").unwrap();

        let batch = vec![
            (file_a.clone(), vec![(4, 7, "bar".to_string())]),
            (
                file_b.clone(),
                vec![(4, 7, "bar".to_string()), (10, 13, "bar".to_string())],
            ),
        ];

        apply_transactional_replacements(dir.path(), &batch).unwrap();

        assert_eq!(fs::read_to_string(&file_a).unwrap(), "let bar = 1;");
        assert_eq!(fs::read_to_string(&file_b).unwrap(), "let bar = bar + 1;");
    }
}
