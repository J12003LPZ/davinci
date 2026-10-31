//! Trusted verification receipts for delegated work (G-02).
//!
//! A worker's final text is a claim. The parent must not turn that claim into
//! "verified" on its own. A receipt is derived by the host after the worker
//! turn ends, from two independent sources:
//!
//! 1. the worker agent's runtime completion state (`completion_status`, the
//!    mutation generation, and the generation a passing check covered); and
//! 2. a before/after snapshot of the worker's workspace, so a change the
//!    worker's own accounting missed is still observed by the host.
//!
//! No model-authored text is parsed to decide the status. When the host has no
//! observation at all, the receipt is `Unavailable`, never `Verified`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{CompletionStatus, MutationVerificationState};

pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

/// Host-owned slot a worker fills after its turn. It is created by the parent
/// agent, never by model tool JSON.
pub type SubagentReceiptSlot = Arc<Mutex<Option<VerificationReceipt>>>;

pub fn receipt_slot() -> SubagentReceiptSlot {
    Arc::new(Mutex::new(None))
}

/// How much of the worker's result the host could actually confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    /// A passing check covers the latest observed mutation, and the workspace
    /// did change during the turn.
    Verified,
    /// Files changed, but verification is missing, stale, partial, or not
    /// attributable to the worker's own runtime state.
    Unverified,
    /// The worker ran a check and it failed.
    Failed,
    /// The host could not observe enough state to make any claim (no changes,
    /// no snapshot, or no receipt was produced).
    Unavailable,
}

impl ReceiptStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Deterministic snapshot of a worker workspace.
///
/// Build output, dependency caches and VCS metadata are excluded: they change
/// for reasons unrelated to the task and would both slow the walk down and make
/// receipts noisy. A snapshot that exceeds the bounded limits fails instead of
/// silently covering only part of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub root: String,
    pub digest: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

const MAX_SNAPSHOT_FILES: usize = 50_000;
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;

const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    ".davinci",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".cargo",
    ".gradle",
    ".idea",
];

const EXCLUDED_FILES: &[&str] = &[".DS_Store", "Thumbs.db"];

impl WorkspaceSnapshot {
    pub fn capture(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve workspace {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("workspace is not a directory: {}", root.display()));
        }
        let mut files = BTreeMap::new();
        let mut bytes = 0u64;
        walk(&root, &root, &mut files, &mut bytes)?;
        Ok(Self {
            root: root.to_string_lossy().into_owned(),
            digest: digest_files(&files),
            files,
        })
    }

    /// Paths that differ between two snapshots, ordered lexicographically.
    /// Added, removed and modified files all count.
    pub fn changed_files(&self, other: &Self) -> Vec<String> {
        let mut changed = Vec::new();
        for (path, digest) in &self.files {
            if other.files.get(path) != Some(digest) {
                changed.push(path.clone());
            }
        }
        for path in other.files.keys() {
            if !self.files.contains_key(path) {
                changed.push(path.clone());
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }
}

fn walk(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, String>,
    bytes: &mut u64,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        if metadata.is_dir() {
            if EXCLUDED_DIRECTORIES.contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, files, bytes)?;
            continue;
        }
        if EXCLUDED_FILES.contains(&name.as_str()) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| format!("path escaped workspace: {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let file_digest = if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .map_err(|error| format!("cannot read link {}: {error}", path.display()))?;
            let mut hasher = Sha256::new();
            hasher.update(b"link\0");
            hasher.update(target.to_string_lossy().replace('\\', "/").as_bytes());
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        } else {
            *bytes = bytes.saturating_add(metadata.len());
            if *bytes > MAX_SNAPSHOT_BYTES {
                return Err(format!(
                    "workspace {} exceeds the {} byte snapshot limit",
                    root.display(),
                    MAX_SNAPSHOT_BYTES
                ));
            }
            let content = std::fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let mut hasher = Sha256::new();
            hasher.update(b"file\0");
            hasher.update((content.len() as u64).to_le_bytes());
            hasher.update(&content);
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        files.insert(relative, file_digest);
        if files.len() > MAX_SNAPSHOT_FILES {
            return Err(format!(
                "workspace {} exceeds the {} file snapshot limit",
                root.display(),
                MAX_SNAPSHOT_FILES
            ));
        }
    }
    Ok(())
}

fn digest_files(files: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (path, digest) in files {
        hasher.update(b"path\0");
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(digest.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A host-derived statement about one worker turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub schema_version: u32,
    pub status: ReceiptStatus,
    /// Host-side reason for the status. Never copied from model output.
    pub detail: String,
    pub workspace: Option<String>,
    pub workspace_digest: Option<String>,
    /// Files the host observed as added, removed or modified.
    pub changed_files: Vec<String>,
    /// False when the before/after snapshots could not both be taken.
    pub changed_files_known: bool,
    pub mutation_generation: u64,
    pub verified_generation: Option<u64>,
    pub completion_status: Option<CompletionStatus>,
    pub observed_at_ms: u64,
}

impl VerificationReceipt {
    /// A receipt for a worker the host never observed. Absence of evidence is
    /// `Unavailable`, not success.
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            schema_version: RECEIPT_SCHEMA_VERSION,
            status: ReceiptStatus::Unavailable,
            detail: detail.into(),
            workspace: None,
            workspace_digest: None,
            changed_files: Vec::new(),
            changed_files_known: false,
            mutation_generation: 0,
            verified_generation: None,
            completion_status: None,
            observed_at_ms: davinci_session::now_ms(),
        }
    }

    /// Derive the receipt from trusted runtime state and workspace snapshots.
    pub fn from_worker(
        completion: CompletionStatus,
        state: &MutationVerificationState,
        before: Option<&WorkspaceSnapshot>,
        after: Option<&WorkspaceSnapshot>,
        workspace: Option<&Path>,
    ) -> Self {
        let changed_files_known = matches!((before, after), (Some(_), Some(_)));
        let changed_files = match (before, after) {
            (Some(before), Some(after)) => before.changed_files(after),
            _ => Vec::new(),
        };
        let workspace_changed = changed_files_known && !changed_files.is_empty();

        let (status, detail) = if after.is_none() {
            (
                ReceiptStatus::Unavailable,
                "the host could not snapshot the worker workspace; no verification can be confirmed",
            )
        } else {
            match completion {
                CompletionStatus::Verified
                    if changed_files_known
                        && changed_files.is_empty()
                        && state.mutation_generation > 0 =>
                {
                    (
                        ReceiptStatus::Unavailable,
                        "the worker reported a verified generation but the workspace did not change",
                    )
                }
                CompletionStatus::Verified
                    if changed_files_known
                        && workspace_changed
                        && state.mutation_generation == 0 =>
                {
                    (
                        ReceiptStatus::Unverified,
                        "the host observed workspace changes the worker runtime did not record",
                    )
                }
                CompletionStatus::Verified
                    if state.mutation_generation > 0
                        && state.verified_generation == Some(state.mutation_generation) =>
                {
                    (
                        ReceiptStatus::Verified,
                        "a passing check covers the latest mutation generation",
                    )
                }
                CompletionStatus::Verified => (
                    ReceiptStatus::Unverified,
                    "a passing check does not cover the latest mutation generation",
                ),
                CompletionStatus::Partial => (
                    ReceiptStatus::Unverified,
                    "passing checks cover only part of the worker's changes",
                ),
                CompletionStatus::VerificationFailed => (
                    ReceiptStatus::Failed,
                    "the worker's latest verification failed",
                ),
                CompletionStatus::Unverified => (
                    ReceiptStatus::Unverified,
                    "the worker's verification is stale or incomplete",
                ),
                CompletionStatus::VerificationUnavailable => (
                    ReceiptStatus::Unverified,
                    "the worker changed files but no verification command was observed",
                ),
                CompletionStatus::NoChanges if workspace_changed => (
                    ReceiptStatus::Unverified,
                    "the host observed workspace changes the worker did not account for",
                ),
                CompletionStatus::NoChanges => (
                    ReceiptStatus::Unavailable,
                    "the worker made no repository changes; there is nothing to verify",
                ),
            }
        };

        Self {
            schema_version: RECEIPT_SCHEMA_VERSION,
            status,
            detail: detail.into(),
            workspace: workspace.map(|path| path.to_string_lossy().into_owned()),
            workspace_digest: after.map(|snapshot| snapshot.digest.clone()),
            changed_files,
            changed_files_known,
            mutation_generation: state.mutation_generation,
            verified_generation: state.verified_generation,
            completion_status: Some(completion),
            observed_at_ms: davinci_session::now_ms(),
        }
    }

    /// Compact, model-visible block appended to the worker result by the
    /// parent. It is host text, not worker text.
    pub fn render(&self) -> String {
        let mut lines = vec![format!(
            "[verification receipt] status: {}",
            self.status.as_str()
        )];
        lines.push(format!("detail: {}", self.detail));
        if !self.changed_files_known {
            lines.push("changed files: unknown".into());
        } else if self.changed_files.is_empty() {
            lines.push("changed files: none".into());
        } else {
            lines.push(format!(
                "changed files ({}): {}",
                self.changed_files.len(),
                self.changed_files.join(", ")
            ));
        }
        if let Some(generation) = self.verified_generation {
            lines.push(format!(
                "mutation generation: {} (verified {})",
                self.mutation_generation, generation
            ));
        } else {
            lines.push(format!(
                "mutation generation: {} (unverified)",
                self.mutation_generation
            ));
        }
        lines.join("\n")
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Read the slot, falling back to an honest `Unavailable` receipt when the
    /// worker or host produced nothing.
    pub fn take(slot: Option<&SubagentReceiptSlot>) -> Self {
        slot.and_then(|slot| {
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        })
        .unwrap_or_else(|| {
            Self::unavailable("the host did not observe the worker's completion state")
        })
    }
}

/// Convenience wrapper for a host that has an agent and two snapshots.
pub fn receipt_for_agent(
    agent: &crate::Agent,
    before: Option<&WorkspaceSnapshot>,
    after: Option<&WorkspaceSnapshot>,
    workspace: Option<&Path>,
) -> VerificationReceipt {
    VerificationReceipt::from_worker(
        agent.completion_status(),
        &agent.mutation_verification_state(),
        before,
        after,
        workspace,
    )
}

/// Write a receipt into a host-owned slot. A poisoned lock must not hide the
/// receipt: the value is still written.
pub fn publish(slot: &SubagentReceiptSlot, receipt: VerificationReceipt) {
    *slot.lock().unwrap_or_else(|error| error.into_inner()) = Some(receipt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    fn verified_state() -> MutationVerificationState {
        MutationVerificationState {
            mutation_generation: 1,
            verified_generation: Some(1),
            last_verification_succeeded: true,
            ..Default::default()
        }
    }

    fn snapshot_pair(
        root: &Path,
        change: Option<(&str, &str)>,
    ) -> (WorkspaceSnapshot, WorkspaceSnapshot) {
        let before = WorkspaceSnapshot::capture(root).unwrap();
        if let Some((relative, content)) = change {
            write(root, relative, content);
        }
        let after = WorkspaceSnapshot::capture(root).unwrap();
        (before, after)
    }

    #[test]
    fn capture_and_diff_detects_added_removed_and_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        write(dir.path(), "src/removed.rs", "gone\n");
        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();
        write(dir.path(), "src/lib.rs", "new\n");
        std::fs::remove_file(dir.path().join("src/removed.rs")).unwrap();
        write(dir.path(), "src/added.rs", "added\n");
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();
        assert_eq!(
            before.changed_files(&after),
            vec!["src/added.rs", "src/lib.rs", "src/removed.rs"]
        );
    }

    #[test]
    fn capture_ignores_build_and_vcs_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "code\n");
        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();
        write(dir.path(), "target/debug/artifact.bin", "noise\n");
        write(dir.path(), ".git/index", "noise\n");
        write(dir.path(), "node_modules/pkg/index.js", "noise\n");
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();
        assert_eq!(before.digest, after.digest);
        assert!(before.changed_files(&after).is_empty());
    }

    #[test]
    fn verified_runtime_state_produces_verified_receipt() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "fixed\n")));
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::Verified,
            &verified_state(),
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Verified);
        assert_eq!(receipt.changed_files, vec!["src/lib.rs"]);
        assert_eq!(receipt.schema_version, RECEIPT_SCHEMA_VERSION);
    }

    #[test]
    fn stale_generation_is_unverified() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "fixed\n")));
        let mut state = verified_state();
        state.mutation_generation = 2;
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::Verified,
            &state,
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Unverified);
        assert!(!receipt.status.is_verified());
    }

    #[test]
    fn failed_verification_is_failed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "broken\n")));
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::VerificationFailed,
            &MutationVerificationState {
                mutation_generation: 1,
                last_verification_succeeded: false,
                ..Default::default()
            },
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Failed);
    }

    #[test]
    fn changed_files_without_verification_are_unverified() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "changed\n")));
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::VerificationUnavailable,
            &MutationVerificationState {
                mutation_generation: 1,
                ..Default::default()
            },
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Unverified);
    }

    #[test]
    fn no_changes_is_unavailable_not_verified() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "stable\n");
        let (before, after) = snapshot_pair(dir.path(), None);
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::NoChanges,
            &MutationVerificationState::default(),
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Unavailable);
    }

    #[test]
    fn reported_verified_without_a_workspace_change_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "stable\n");
        let (before, after) = snapshot_pair(dir.path(), None);
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::Verified,
            &verified_state(),
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Unavailable);
        assert!(!receipt.status.is_verified());
    }

    #[test]
    fn missing_snapshot_is_unavailable() {
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::Verified,
            &verified_state(),
            None,
            None,
            None,
        );
        assert_eq!(receipt.status, ReceiptStatus::Unavailable);
        assert!(!receipt.changed_files_known);
    }

    #[test]
    fn host_snapshot_catches_changes_the_runtime_missed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "changed\n")));
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::NoChanges,
            &MutationVerificationState::default(),
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        assert_eq!(receipt.status, ReceiptStatus::Unverified);
        assert_eq!(receipt.changed_files, vec!["src/lib.rs"]);
    }

    #[test]
    fn claimed_text_alone_can_never_verify() {
        // No slot observation: the host refuses to infer success from text.
        let receipt = VerificationReceipt::take(None);
        assert_eq!(receipt.status, ReceiptStatus::Unavailable);
        assert!(!receipt.status.is_verified());
    }

    #[test]
    fn receipt_slot_is_shared_by_clone() {
        let slot = receipt_slot();
        let clone = slot.clone();
        assert!(VerificationReceipt::take(Some(&slot))
            .detail
            .contains("did not observe"));
        publish(
            &clone,
            VerificationReceipt::unavailable("worker wrote this"),
        );
        let receipt = VerificationReceipt::take(Some(&slot));
        assert_eq!(receipt.detail, "worker wrote this");
    }

    #[test]
    fn receipt_serializes_with_schema_version() {
        let receipt = VerificationReceipt::unavailable("nothing observed");
        let json = serde_json::to_value(&receipt).unwrap();
        assert_eq!(json["schema_version"], RECEIPT_SCHEMA_VERSION);
        assert_eq!(json["status"], "unavailable");
        let decoded: VerificationReceipt = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn render_lists_status_files_and_generation() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "old\n");
        let (before, after) = snapshot_pair(dir.path(), Some(("src/lib.rs", "fixed\n")));
        let receipt = VerificationReceipt::from_worker(
            CompletionStatus::Verified,
            &verified_state(),
            Some(&before),
            Some(&after),
            Some(dir.path()),
        );
        let rendered = receipt.render();
        assert!(rendered.contains("status: verified"));
        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains("mutation generation: 1 (verified 1)"));
    }
}
