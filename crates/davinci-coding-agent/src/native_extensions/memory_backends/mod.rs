//! The optional remote vector projection.
//!
//! The local JSONL store is the authority. A projection is a derived,
//! rebuildable copy that can answer nearest-neighbour queries faster on a large
//! store. It never holds text, never decides what a memory is, and its absence
//! or failure never loses a record: the caller falls back to local search and
//! says so.
//!
//! [`MemoryProjectionBackend::Local`] is the default. In that mode nothing in
//! this module is reached: no socket is opened and no `docker` is run.

#[cfg(any(test, feature = "test-fixtures"))]
pub mod fake_ollama;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fake_qdrant;
mod pending_seal;
pub mod qdrant;
pub mod runtime;

pub use runtime::{
    ProjectionHealth, ProjectionRuntime, ProjectionState, QDRANT_OFFLINE, QDRANT_REFUSED,
};

use serde::{Deserialize, Serialize};
use std::fmt;

/// Which projection backs recall. `Local` is the bounded in-process index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryProjectionBackend {
    /// Unknown spellings also read as `Local`: a typo must never turn a remote
    /// store on. (`other` must be the last variant.)
    Qdrant,
    #[default]
    #[serde(other)]
    Local,
}

impl MemoryProjectionBackend {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "qdrant" => Some(Self::Qdrant),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Qdrant => "qdrant",
        }
    }
}

/// A credential that cannot reach a log line: `Debug` and `Display` print a
/// fixed marker, and it is never serialized.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let value = value.trim();
        (!value.is_empty()).then(|| Self(value.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Everything the remote projection reads from config. Lives under
/// `projection` in `vector-memory.json`; every field has a safe default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionConfig {
    #[serde(default)]
    pub backend: MemoryProjectionBackend,
    /// Sent as the `api-key` header. Config or `PI_MEMORY_QDRANT_API_KEY`;
    /// never written back to disk by DaVinci and never logged.
    #[serde(default, skip_serializing)]
    pub api_key: Option<Secret>,
    /// A Qdrant that is not on this machine is refused unless this is true.
    #[serde(default)]
    pub allow_remote: bool,
    /// Prefix for the namespaced collection name.
    #[serde(default = "default_prefix")]
    pub collection_prefix: String,
    /// Records sent per request, and per reconcile pass.
    #[serde(default = "default_batch")]
    pub batch_size: usize,
    /// After a failure the remote is skipped for this long.
    #[serde(default = "default_backoff")]
    pub offline_backoff_seconds: u64,
}

fn default_prefix() -> String {
    "davinci".into()
}
fn default_batch() -> usize {
    64
}
fn default_backoff() -> u64 {
    60
}

impl Default for ProjectionConfig {
    fn default() -> Self {
        Self {
            backend: MemoryProjectionBackend::Local,
            api_key: None,
            allow_remote: false,
            collection_prefix: default_prefix(),
            batch_size: default_batch(),
            offline_backoff_seconds: default_backoff(),
        }
    }
}

/// One vector to mirror. No text: the remote learns an id, a namespace and
/// numbers, and the id is resolved against the local records on the way back.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub repo_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionHit {
    pub id: String,
    pub score: f32,
}

/// Why a projection call did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// Unreachable, not ready, or timed out. Retry later; fall back now.
    Offline(String),
    /// Reachable but wrong: foreign collection, bad dimension, auth refused.
    /// Retrying does not help until the user changes something.
    Refused(String),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline(detail) | Self::Refused(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// What one reconcile pass did. Local records are never touched by it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionRepairReceipt {
    /// Records sent to the remote in this pass.
    pub indexed: usize,
    /// Retired records (tombstoned, superseded, gone) removed from the remote.
    pub removed: usize,
    /// Records still not mirrored (or still to remove) after the pass. Run
    /// again while above zero.
    pub lag: usize,
    /// Live records with no current local vector; they cannot be mirrored
    /// until the local repair embeds them.
    pub skipped: usize,
    pub failures: usize,
    /// `QDRANT_OFFLINE` or `QDRANT_REFUSED` when the pass could not finish.
    pub code: Option<&'static str>,
    pub error: Option<String>,
}

impl ProjectionRepairReceipt {
    pub fn is_complete(&self) -> bool {
        self.failures == 0 && self.lag == 0
    }
}

/// A remote vector index. Implemented for Qdrant; a fake in tests.
pub trait VectorProjection: Send + Sync {
    /// Readiness plus collection validation. Creates the namespaced collection
    /// when it is absent; never touches any other collection.
    fn ensure_ready(&self) -> Result<(), ProjectionError>;
    /// Idempotent: the same point twice is one point.
    fn upsert(&self, points: &[ProjectionPoint]) -> Result<(), ProjectionError>;
    /// Nearest ids inside `repo_id` only.
    fn query(
        &self,
        vector: &[f32],
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<ProjectionHit>, ProjectionError>;
    /// Remove these ids from this namespace.
    fn delete(&self, ids: &[String], repo_id: &str) -> Result<(), ProjectionError>;
    /// Remove every point of this namespace. Other namespaces in the same
    /// collection, and other collections, are never touched.
    fn clear_namespace(&self, repo_id: &str) -> Result<(), ProjectionError>;
    /// Remove these ids of `repo_id` from a named collection, which may not be
    /// the one this projection uses (the name follows the embedding model and
    /// the repo id, and a clear can outlive a change of either). A collection
    /// that is gone, or that DaVinci did not make, holds nothing to remove and
    /// is left alone; it is never created.
    fn delete_from(
        &self,
        collection: &str,
        ids: &[String],
        repo_id: &str,
    ) -> Result<(), ProjectionError>;
    /// How many of exactly these ids the remote holds for `repo_id`. Other
    /// points of the same `repo_id` (another clone of the repository writes to
    /// it too) do not count. A collection that is gone counts as zero, so a
    /// deleted and recreated remote reads as empty.
    fn count_held(&self, ids: &[String], repo_id: &str) -> Result<usize, ProjectionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_defaults_to_local_and_unknown_words_read_as_local() {
        assert_eq!(
            MemoryProjectionBackend::default(),
            MemoryProjectionBackend::Local
        );
        let parsed: MemoryProjectionBackend = serde_json::from_str("\"qdrnat\"").unwrap();
        assert_eq!(parsed, MemoryProjectionBackend::Local);
        let parsed: MemoryProjectionBackend = serde_json::from_str("\"qdrant\"").unwrap();
        assert_eq!(parsed, MemoryProjectionBackend::Qdrant);
        assert_eq!(
            MemoryProjectionBackend::parse("Qdrant "),
            Some(MemoryProjectionBackend::Qdrant)
        );
        assert_eq!(MemoryProjectionBackend::parse("pinecone"), None);
    }

    #[test]
    fn secret_never_prints_or_serializes() {
        let config = ProjectionConfig {
            api_key: Secret::new("sk-very-secret"),
            ..ProjectionConfig::default()
        };
        assert!(!format!("{config:?}").contains("sk-very-secret"));
        assert!(!serde_json::to_string(&config)
            .unwrap()
            .contains("sk-very-secret"));
        assert_eq!(Secret::new("   "), None);
        let read: ProjectionConfig = serde_json::from_str(r#"{"apiKey":"abc"}"#).unwrap();
        assert_eq!(read.api_key.unwrap().expose(), "abc");
    }
}
