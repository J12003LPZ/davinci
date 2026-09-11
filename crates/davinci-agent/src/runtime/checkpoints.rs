//! Content-addressed checkpoint store and task effect preimage/postimage capture.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::ids::TaskId;
use super::source_manifest::SourceManifest;

pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024; // 16 MiB per blob
pub const MAX_TASK_BLOBS_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB per task

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("Blob exceeds maximum 16 MiB limit: size {size} bytes")]
    BlobTooLarge { size: usize },
    #[error("Task storage exceeds maximum 256 MiB limit: current {current} bytes")]
    TaskCapacityExceeded { current: u64 },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Simulated disk full")]
    DiskFull,
    #[error("Corrupt blob hash: expected {expected}, calculated {calculated}")]
    CorruptBlob {
        expected: String,
        calculated: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub id: String,
    pub task_id: TaskId,
    pub attempt: u32,
    pub journal_sequence: u64,
    pub source_manifest: SourceManifest,
    pub task_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_leaf: Option<String>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCapture {
    pub path: String,
    pub exists: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub mode: Option<u32>,
    pub blob_hash: Option<String>,
    pub size: u64,
}

pub fn may_mutate_with_checkpoint(
    authorized: bool,
    durable_checkpoint: bool,
    explicit_nonrewindable_exception: bool,
) -> bool {
    authorized && (durable_checkpoint || explicit_nonrewindable_exception)
}

pub fn compute_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Default)]
pub struct BlobStore {
    blobs: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    task_usage: Arc<RwLock<HashMap<TaskId, u64>>>,
    disk_full_injection: Arc<RwLock<bool>>,
}

impl BlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_disk_full(&self, disk_full: bool) {
        if let Ok(mut df) = self.disk_full_injection.write() {
            *df = disk_full;
        }
    }

    pub fn store_blob_for_task(
        &self,
        task_id: TaskId,
        bytes: &[u8],
    ) -> Result<String, CheckpointError> {
        if let Ok(df) = self.disk_full_injection.read() {
            if *df {
                return Err(CheckpointError::DiskFull);
            }
        }
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(CheckpointError::BlobTooLarge { size: bytes.len() });
        }
        let hash = compute_sha256(bytes);
        let mut blobs = self.blobs.write().map_err(|_| {
            CheckpointError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Blob store lock poisoned",
            ))
        })?;

        if !blobs.contains_key(&hash) {
            let mut usage = self.task_usage.write().map_err(|_| {
                CheckpointError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Task usage lock poisoned",
                ))
            })?;
            let current = usage.entry(task_id).or_insert(0);
            let next = *current + bytes.len() as u64;
            if next > MAX_TASK_BLOBS_BYTES {
                return Err(CheckpointError::TaskCapacityExceeded { current: *current });
            }
            *current = next;
            blobs.insert(hash.clone(), bytes.to_vec());
        }
        Ok(hash)
    }

    pub fn get_blob(&self, hash: &str) -> Option<Vec<u8>> {
        self.blobs.read().ok()?.get(hash).cloned()
    }

    pub fn capture_file(
        &self,
        task_id: TaskId,
        base_dir: &Path,
        relative_path: &str,
    ) -> Result<FileCapture, CheckpointError> {
        let full_path = base_dir.join(relative_path);
        let symlink_meta = std::fs::symlink_metadata(&full_path);

        match symlink_meta {
            Ok(meta) => {
                let is_symlink = meta.file_type().is_symlink();
                let symlink_target = if is_symlink {
                    std::fs::read_link(&full_path)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                } else {
                    None
                };

                let mode = {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        Some(meta.permissions().mode())
                    }
                    #[cfg(not(unix))]
                    {
                        if meta.permissions().readonly() {
                            Some(0o444)
                        } else {
                            Some(0o644)
                        }
                    }
                };

                let (blob_hash, size) = if is_symlink {
                    let target_bytes = symlink_target
                        .as_ref()
                        .map(|s| s.as_bytes())
                        .unwrap_or_default();
                    let hash = self.store_blob_for_task(task_id, target_bytes)?;
                    (Some(hash), target_bytes.len() as u64)
                } else if meta.is_file() {
                    let bytes = std::fs::read(&full_path)?;
                    let size = bytes.len() as u64;
                    let hash = self.store_blob_for_task(task_id, &bytes)?;
                    (Some(hash), size)
                } else {
                    (None, 0)
                };

                Ok(FileCapture {
                    path: relative_path.replace('\\', "/"),
                    exists: true,
                    is_symlink,
                    symlink_target,
                    mode,
                    blob_hash,
                    size,
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(FileCapture {
                path: relative_path.replace('\\', "/"),
                exists: false,
                is_symlink: false,
                symlink_target: None,
                mode: None,
                blob_hash: None,
                size: 0,
            }),
            Err(err) => Err(CheckpointError::Io(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f04_checkpoint_required() {
        assert!(may_mutate_with_checkpoint(true, true, false));
        assert!(!may_mutate_with_checkpoint(true, false, false));
        assert!(may_mutate_with_checkpoint(true, false, true));
        assert!(!may_mutate_with_checkpoint(false, true, true));
    }

    #[test]
    fn test_crlf_preservation() {
        let store = BlobStore::new();
        let task_id = TaskId::new();
        let crlf_bytes = b"first line\r\nsecond line\r\n";
        let hash = store.store_blob_for_task(task_id, crlf_bytes).unwrap();
        let retrieved = store.get_blob(&hash).unwrap();
        assert_eq!(retrieved, crlf_bytes);
    }

    #[test]
    fn test_invalid_utf8_binary_preservation() {
        let store = BlobStore::new();
        let task_id = TaskId::new();
        let binary_bytes = vec![0xFF, 0xFE, 0x00, 0x01, 0xDE, 0xAD, 0xBE, 0xEF];
        let hash = store.store_blob_for_task(task_id, &binary_bytes).unwrap();
        let retrieved = store.get_blob(&hash).unwrap();
        assert_eq!(retrieved, binary_bytes);
    }

    #[test]
    fn test_empty_file_blob() {
        let store = BlobStore::new();
        let task_id = TaskId::new();
        let empty_bytes = b"";
        let hash = store.store_blob_for_task(task_id, empty_bytes).unwrap();
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let retrieved = store.get_blob(&hash).unwrap();
        assert_eq!(retrieved, empty_bytes);
    }

    #[test]
    fn test_blob_cap_enforcement() {
        let store = BlobStore::new();
        let task_id = TaskId::new();
        let oversized = vec![0u8; MAX_BLOB_BYTES + 1];
        let err = store.store_blob_for_task(task_id, &oversized).unwrap_err();
        assert!(matches!(err, CheckpointError::BlobTooLarge { .. }));
    }

    #[test]
    fn test_disk_full_injection() {
        let store = BlobStore::new();
        let task_id = TaskId::new();
        store.set_disk_full(true);
        let err = store.store_blob_for_task(task_id, b"hello").unwrap_err();
        assert!(matches!(err, CheckpointError::DiskFull));
    }

    #[test]
    fn test_capture_file_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new();
        let task_id = TaskId::new();

        // 1. Non-existent file capture
        let cap_missing = store
            .capture_file(task_id, dir.path(), "untracked.txt")
            .unwrap();
        assert!(!cap_missing.exists);
        assert_eq!(cap_missing.blob_hash, None);

        // 2. Pre-existing untracked file capture
        let file_path = dir.path().join("untracked.txt");
        std::fs::write(&file_path, "original dirty content").unwrap();
        let cap_existing = store
            .capture_file(task_id, dir.path(), "untracked.txt")
            .unwrap();
        assert!(cap_existing.exists);
        assert!(cap_existing.blob_hash.is_some());
        assert_eq!(
            store
                .get_blob(cap_existing.blob_hash.as_ref().unwrap())
                .unwrap(),
            b"original dirty content"
        );
    }
}
