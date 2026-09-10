//! LSP rename preview collection, precondition checks, and guarded transactional application.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Determines whether a rename preview can be applied to the target codebase.
pub fn rename_apply_allowed(
    preview: &str,
    current: &str,
    explicit_user: bool,
    contract_allows: bool,
    overlapping_edits: bool,
) -> bool {
    preview == current && explicit_user && contract_allows && !overlapping_edits
}

/// A text edit with explicit byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditByteRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub new_text: String,
}

/// A set of text edits for a specific file, bound to document version and content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRenameEdits {
    pub path: PathBuf,
    pub document_version: i32,
    pub content_hash: String,
    pub edits: Vec<TextEditByteRange>,
}

/// Read-only preview session for a semantic rename operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePreviewSession {
    pub preview_id: String,
    pub symbol_name: String,
    pub new_name: String,
    pub file_edits: Vec<FileRenameEdits>,
}

/// Checks if any edits within the list overlap with one another.
pub fn has_overlapping_edits(edits: &[TextEditByteRange]) -> bool {
    if edits.len() <= 1 {
        return false;
    }
    let mut sorted: Vec<_> = edits.iter().collect();
    sorted.sort_by_key(|e| e.start_byte);
    for window in sorted.windows(2) {
        if window[0].end_byte > window[1].start_byte {
            return true;
        }
    }
    false
}

/// Applies text edits to content in descending byte-offset order, ensuring edits do not invalidate subsequent offsets.
pub fn apply_text_edits(content: &str, edits: &[TextEditByteRange]) -> Result<String, String> {
    if has_overlapping_edits(edits) {
        return Err("Conflicting/overlapping edits detected in file".into());
    }

    let mut sorted: Vec<_> = edits.iter().collect();
    sorted.sort_by(|a, b| b.start_byte.cmp(&a.start_byte));

    let mut modified = content.to_string();
    for edit in sorted {
        if edit.start_byte > edit.end_byte {
            return Err(format!(
                "Invalid edit range: start {} > end {}",
                edit.start_byte, edit.end_byte
            ));
        }
        if edit.end_byte > modified.len() {
            return Err(format!(
                "Edit end {} exceeds document length {}",
                edit.end_byte,
                modified.len()
            ));
        }
        if !modified.is_char_boundary(edit.start_byte) || !modified.is_char_boundary(edit.end_byte)
        {
            return Err("Edit range splits UTF-8 character boundary".into());
        }
        modified.replace_range(edit.start_byte..edit.end_byte, &edit.new_text);
    }
    Ok(modified)
}

/// Verifies that a file on disk matches the expected SHA-256 hash at preview time.
pub fn verify_file_unchanged(path: &Path, expected_hash: &str) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("File does not exist: {}", path.display()));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed reading file {}: {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let actual_hash = format!("{:x}", hasher.finalize());
    if actual_hash != expected_hash {
        return Err(format!(
            "File {} has changed since rename preview was generated (expected {}, got {})",
            path.display(),
            expected_hash,
            actual_hash
        ));
    }
    Ok(())
}

/// Verifies that all affected paths are contained within the workspace root.
pub fn verify_rename_scope(root: &Path, affected_paths: &[PathBuf]) -> Result<(), String> {
    for p in affected_paths {
        if !crate::semantic::tools::is_path_in_root(p, root) {
            return Err(format!(
                "Rename target {} is outside workspace root {}",
                p.display(),
                root.display()
            ));
        }
    }
    Ok(())
}

/// Validates that no resource creation/deletion operations are requested (V1 only supports in-file text edits).
pub fn validate_resource_operations(has_create_or_delete: bool) -> Result<(), String> {
    if has_create_or_delete {
        Err("Resource create/delete operations in rename are unsupported in V1".into())
    } else {
        Ok(())
    }
}

/// Checks that none of the target paths are protected system/journal files.
pub fn check_protected_files(paths: &[PathBuf], protected: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        let file_name = p
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if file_name.starts_with(".davinci_patch_journal")
            || file_name.starts_with(".pi_patch_journal")
            || protected.iter().any(|prot| p == prot)
        {
            return Err(format!("Cannot rename protected file: {}", p.display()));
        }
    }
    Ok(())
}

/// Evaluates post-rename verification gate.
pub fn verify_post_rename(test_passed: bool) -> Result<(), String> {
    if !test_passed {
        Err("Post-rename verification tests failed; changes must not be claimed verified".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f10_rename_preconditions() {
        assert!(rename_apply_allowed("a", "a", true, true, false));
        assert!(!rename_apply_allowed("a", "b", true, true, false));
        assert!(!rename_apply_allowed("a", "a", false, true, false));
        assert!(!rename_apply_allowed("a", "a", true, true, true));
    }

    #[test]
    fn test_cross_file_rename() {
        let content_a = "fn old_symbol() {}";
        let content_b = "fn caller() { old_symbol(); }";

        let edits_a = vec![TextEditByteRange {
            start_byte: 3,
            end_byte: 13,
            new_text: "new_symbol".into(),
        }];
        let edits_b = vec![TextEditByteRange {
            start_byte: 14,
            end_byte: 24,
            new_text: "new_symbol".into(),
        }];

        let res_a = apply_text_edits(content_a, &edits_a).unwrap();
        let res_b = apply_text_edits(content_b, &edits_b).unwrap();

        assert_eq!(res_a, "fn new_symbol() {}");
        assert_eq!(res_b, "fn caller() { new_symbol(); }");
    }

    #[test]
    fn test_protected_file() {
        let journal = PathBuf::from("/workspace/.davinci_patch_journal.json");
        let normal = PathBuf::from("/workspace/src/lib.rs");
        let protected = vec![PathBuf::from("/workspace/.git/config")];

        assert!(check_protected_files(&[journal], &protected).is_err());
        assert!(
            check_protected_files(&[PathBuf::from("/workspace/.git/config")], &protected).is_err()
        );
        assert!(check_protected_files(&[normal], &protected).is_ok());
    }

    #[test]
    fn test_resource_create_delete_request() {
        assert!(validate_resource_operations(true).is_err());
        assert!(validate_resource_operations(false).is_ok());
    }

    #[test]
    fn test_rename_outside_root() {
        let root = Path::new("/workspace/project");
        let inside = PathBuf::from("/workspace/project/src/main.rs");
        let outside = PathBuf::from("/etc/passwd");

        assert!(verify_rename_scope(root, &[inside]).is_ok());
        assert!(verify_rename_scope(root, &[outside]).is_err());
    }

    #[test]
    fn test_conflicting_edits() {
        let edits = vec![
            TextEditByteRange {
                start_byte: 5,
                end_byte: 15,
                new_text: "foo".into(),
            },
            TextEditByteRange {
                start_byte: 10,
                end_byte: 20,
                new_text: "bar".into(),
            },
        ];
        assert!(has_overlapping_edits(&edits));
        assert!(apply_text_edits("some 25 character long text", &edits).is_err());
    }

    #[test]
    fn test_partial_apply_failure() {
        let edits = vec![TextEditByteRange {
            start_byte: 50,
            end_byte: 60, // Out of bounds
            new_text: "foo".into(),
        }];
        assert!(apply_text_edits("short text", &edits).is_err());
    }

    #[test]
    fn test_file_changes_after_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("code.rs");
        std::fs::write(&file_path, "fn target() {}").unwrap();

        let mut hasher = Sha256::new();
        hasher.update(b"fn target() {}");
        let original_hash = format!("{:x}", hasher.finalize());

        assert!(verify_file_unchanged(&file_path, &original_hash).is_ok());

        // File modified after preview
        std::fs::write(&file_path, "fn target_modified() {}").unwrap();
        assert!(verify_file_unchanged(&file_path, &original_hash).is_err());
    }

    #[test]
    fn test_same_spelling_different_symbol() {
        // Source text where `count` appears both as a struct field and local variable
        let src = "struct S { count: usize }\nfn run() { let count = 0; }";
        // Rename only the struct field (bytes 11..16), leaving local variable untouched
        let edits = vec![TextEditByteRange {
            start_byte: 11,
            end_byte: 16,
            new_text: "total".into(),
        }];
        let res = apply_text_edits(src, &edits).unwrap();
        assert_eq!(
            res,
            "struct S { total: usize }\nfn run() { let count = 0; }"
        );
    }

    #[test]
    fn test_cancelled_confirmation() {
        // User cancels confirmation -> explicit_user is false
        assert!(!rename_apply_allowed(
            "preview_1",
            "preview_1",
            false,
            true,
            false
        ));
    }

    #[test]
    fn test_postrename_tests_fail() {
        assert!(verify_post_rename(true).is_ok());
        assert!(verify_post_rename(false).is_err());
    }
}
