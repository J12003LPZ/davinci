use super::super::{AgentId, TaskId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_FILES: usize = 64;
pub const MAX_FILE_BYTES: usize = super::super::checkpoints::MAX_BLOB_BYTES;
pub const MAX_TRANSACTION_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RECORD_BYTES: u64 = 192 * 1024 * 1024;
pub const MAX_RECORDS: usize = 128;
pub const STORE_NAME: &str = ".davinci-transactions";
// Windows v2 requires creation time and attributes for every captured file.
// These values cannot be reconstructed safely from a legacy rollback journal.
pub(super) const RECORD_SCHEMA: u32 = if cfg!(windows) { 2 } else { 1 };

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionOwner {
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub session_id: Option<String>,
    pub task_id: Option<TaskId>,
    pub graph_node: Option<String>,
}
impl Default for TransactionOwner {
    fn default() -> Self {
        Self {
            agent_id: AgentId::new(),
            parent_agent_id: None,
            session_id: None,
            task_id: None,
            graph_node: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Draft,
    Previewed,
    Applying,
    Applied,
    Verified,
    Committed,
    RollingBack,
    RolledBack,
    Conflicted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionSummary {
    pub id: String,
    pub owner: TransactionOwner,
    pub workspace: String,
    pub workspace_identity: String,
    pub base_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_revision: Option<String>,
    pub affected_files: Vec<String>,
    pub before_hashes: BTreeMap<String, Option<String>>,
    pub proposed_hashes: BTreeMap<String, Option<String>>,
    pub applied_hashes: BTreeMap<String, Option<String>>,
    pub state: TransactionState,
    pub verification_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<super::verification::TransactionVerification>,
    pub timestamp_ms: u64,
    pub sequence: u64,
    pub conflict: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProposedChange {
    pub path: String,
    pub bytes: Option<Vec<u8>>,
    pub(super) expected: Option<Image>,
}
impl ProposedChange {
    pub fn write(path: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            bytes: Some(bytes),
            expected: None,
        }
    }
    pub fn delete(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            bytes: None,
            expected: None,
        }
    }
}

/// An opaque, bounded source observation. Derive edits from these bytes so a
/// concurrent change between reading and previewing cannot be silently adopted.
#[derive(Debug)]
pub struct SourceSnapshot {
    pub(super) path: String,
    pub(super) image: Image,
    pub(super) bytes: Option<Vec<u8>>,
}
impl SourceSnapshot {
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
    pub fn text(&self) -> Result<&str, String> {
        std::str::from_utf8(self.bytes().ok_or("source file does not exist")?)
            .map_err(|e| e.to_string())
    }
    pub fn change(self, bytes: Option<Vec<u8>>) -> ProposedChange {
        ProposedChange {
            path: self.path,
            bytes,
            expected: Some(self.image),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WindowsMetadata {
    pub created: u64,
    pub attributes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    pub hash: Option<String>,
    pub identity: Option<String>,
    pub mode: Option<u32>,
    pub access: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_metadata: Option<WindowsMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_owner: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_acl: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub xattrs: BTreeMap<String, Vec<u8>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub streams: BTreeMap<String, Vec<u8>>,
}
impl Image {
    pub fn missing() -> Self {
        Self {
            hash: None,
            identity: None,
            mode: None,
            access: None,
            windows_metadata: None,
            unix_owner: None,
            macos_acl: None,
            xattrs: BTreeMap::new(),
            streams: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Change {
    pub path: String,
    pub before: Image,
    pub before_bytes: Option<Vec<u8>>,
    pub proposed: Image,
    pub proposed_bytes: Option<Vec<u8>>,
    pub staged_name: Option<String>,
    pub restored: Option<Image>,
    pub restore_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub schema: u32,
    pub summary: TransactionSummary,
    pub changes: Vec<Change>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_image_round_trips_unix_ownership() {
        let value = serde_json::json!({
            "hash":"image", "identity":"1:2", "mode":0o6750,
            "access":null, "unix_owner":[1000,100]
        });
        let image: Image = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(image).unwrap(), value);
    }
}
