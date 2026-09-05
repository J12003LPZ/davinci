//! Codex `apply_patch` grammar parser, transactional commit, and rollback matching §10.1.
//! Preserves applicable MIT attribution from openai/codex.
//!
//! Copyright (c) OpenAI. All rights reserved.
//! Licensed under the MIT License.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const JOURNAL_FILE_NAME: &str = ".davinci_patch_journal.json";
const LEGACY_JOURNAL_FILE_NAME: &str = ".pi_patch_journal.json";

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
    let davinci_journal = workspace_root.join(JOURNAL_FILE_NAME);
    let legacy_journal = workspace_root.join(LEGACY_JOURNAL_FILE_NAME);
    let journal_path = if davinci_journal.exists() {
        davinci_journal
    } else if legacy_journal.exists() {
        legacy_journal
    } else {
        return Ok(());
    };
    match fs::symlink_metadata(&journal_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Failed inspecting journal: {error}")),
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("Journal must be a regular file".into())
        }
        Ok(_) => {}
    }

    let raw =
        fs::read_to_string(&journal_path).map_err(|e| format!("Failed reading journal: {e}"))?;
    let journal: PatchJournal =
        serde_json::from_str(&raw).map_err(|e| format!("Corrupt journal file: {e}"))?;

    let targets = journal
        .entries
        .iter()
        .map(|entry| sanitize_relative_path(workspace_root, &entry.relative_path))
        .collect::<Result<Vec<_>, _>>()?;
    for (entry, target) in journal.entries.iter().zip(targets) {
        restore_entry(&target, entry).map_err(|error| {
            format!(
                "Recovery failed for {}: {error}; journal retained",
                entry.relative_path
            )
        })?;
    }
    fs::remove_file(journal_path)
        .map_err(|error| format!("Recovery applied but journal cleanup failed: {error}"))
}

fn restore_entry(target: &Path, entry: &JournalEntry) -> std::io::Result<()> {
    match &entry.original_content {
        Some(original) => fs::write(target, original),
        None => match fs::remove_file(target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}

/// Executes a patch with rollback. Existing journals require explicit recovery.
pub fn execute_apply_patch(workspace_root: &Path, input: &str) -> Result<String, String> {
    let parsed = parse_codex_patch(input)?;
    let journal_path = workspace_root.join(JOURNAL_FILE_NAME);
    let legacy_journal = workspace_root.join(LEGACY_JOURNAL_FILE_NAME);
    if journal_path.exists() || legacy_journal.exists() {
        return Err(
            "An existing patch journal requires explicit, authorized recovery; no files changed"
                .into(),
        );
    }
    match fs::symlink_metadata(&journal_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Failed inspecting journal: {error}")),
        Ok(_) => return Err(
            "An existing patch journal requires explicit, authorized recovery; no files changed"
                .into(),
        ),
    }

    // Step 1: Pre-flight validation and prepare journal
    let mut journal_entries = Vec::new();
    let mut planned_mutations: Vec<(PathBuf, Option<String>)> = Vec::new();

    let mut modified_count = 0;
    let mut added_count = 0;
    let mut deleted_count = 0;

    for action in &parsed.actions {
        match action {
            FileAction::Add { path, content } => {
                let target = sanitize_relative_path(workspace_root, path)?;
                let original = if target.exists() {
                    Some(fs::read_to_string(&target).map_err(|e| e.to_string())?)
                } else {
                    None
                };
                journal_entries.push(JournalEntry {
                    relative_path: path.clone(),
                    original_content: original,
                });
                planned_mutations.push((target, Some(content.clone())));
                added_count += 1;
            }
            FileAction::Delete { path } => {
                let target = sanitize_relative_path(workspace_root, path)?;
                if !target.exists() {
                    return Err(format!("File to delete does not exist: {path}"));
                }
                let original = fs::read_to_string(&target).map_err(|e| e.to_string())?;
                journal_entries.push(JournalEntry {
                    relative_path: path.clone(),
                    original_content: Some(original),
                });
                planned_mutations.push((target, None));
                deleted_count += 1;
            }
            FileAction::Update { path, hunks } => {
                let target = sanitize_relative_path(workspace_root, path)?;
                if !target.exists() {
                    return Err(format!("File to update does not exist: {path}"));
                }
                let original = fs::read_to_string(&target).map_err(|e| e.to_string())?;
                let updated = apply_hunks_to_content(&original, hunks)?;
                journal_entries.push(JournalEntry {
                    relative_path: path.clone(),
                    original_content: Some(original),
                });
                planned_mutations.push((target, Some(updated)));
                modified_count += 1;
            }
        }
    }

    // Step 2: Write journal
    let journal = PatchJournal {
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        entries: journal_entries,
    };
    let journal_bytes = serde_json::to_string(&journal).map_err(|e| e.to_string())?;
    let mut journal_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&journal_path)
        .map_err(|e| format!("Failed to create exclusive journal; no files changed: {e}"))?;
    journal_file
        .write_all(journal_bytes.as_bytes())
        .and_then(|_| journal_file.sync_all())
        .map_err(|e| {
            format!("Failed to persist journal; no files changed, journal retained: {e}")
        })?;
    drop(journal_file);

    // Step 3: Apply mutations transactionally
    let mut applied_so_far: Vec<&JournalEntry> = Vec::new();
    for (target, new_content) in planned_mutations {
        let entry = journal.entries.iter().find(|e| {
            match sanitize_relative_path(workspace_root, &e.relative_path) {
                Ok(p) => p == target,
                Err(_) => false,
            }
        });

        let mutation_result = match new_content {
            Some(content) => {
                if let Some(parent) = target.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&target, content)
            }
            None => fs::remove_file(&target),
        };

        if let Err(err) = mutation_result {
            // Rollback everything from journal including the failed target
            if let Some(e) = entry {
                applied_so_far.push(e);
            }
            let mut rollback_errors = Vec::new();
            for entry in applied_so_far.into_iter().rev() {
                let restored = sanitize_relative_path(workspace_root, &entry.relative_path)
                    .and_then(|path| {
                        restore_entry(&path, entry).map_err(|error| error.to_string())
                    });
                if let Err(error) = restored {
                    rollback_errors.push(format!("{}: {error}", entry.relative_path));
                }
            }
            if !rollback_errors.is_empty() {
                return Err(format!(
                    "Mutation failed: {err}; rollback incomplete, journal retained: {}",
                    rollback_errors.join("; ")
                ));
            }
            fs::remove_file(&journal_path).map_err(|error| {
                format!(
                    "Mutation failed: {err}; rollback applied but journal cleanup failed: {error}"
                )
            })?;
            return Err(format!("Mutation failed, rolled back changes: {err}"));
        }

        if let Some(e) = entry {
            applied_so_far.push(e);
        }
    }

    // Step 4: Commit complete, remove journal
    fs::remove_file(journal_path).map_err(|error| {
        format!("Patch applied but journal cleanup failed; inspect state before retrying: {error}")
    })?;

    Ok(format!(
        "Applied patch (digest: {}): {} modified, {} added, {} deleted.",
        &parsed.raw_digest[..8],
        modified_count,
        added_count,
        deleted_count
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
    fn explicit_recovery_restores_and_removes_journal() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("changed.txt"), "changed").unwrap();
        let journal = write_journal(
            dir.path(),
            vec![JournalEntry {
                relative_path: "changed.txt".into(),
                original_content: Some("original".into()),
            }],
        );
        recover_incomplete_journal_if_any(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("changed.txt")).unwrap(),
            "original"
        );
        assert!(!journal.exists());
    }

    #[test]
    fn mutation_failure_restores_already_applied_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("blocker"), "not a directory").unwrap();
        let patch = "*** Begin Patch\n*** Add File: first.txt\n+new\n*** Add File: blocker/child.txt\n+cannot create\n*** End Patch";
        let error = execute_apply_patch(dir.path(), patch).unwrap_err();
        assert!(error.contains("rolled back"), "{error}");
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
}
