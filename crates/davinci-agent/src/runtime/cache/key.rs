use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheNamespace {
    Prompt,
    Context,
    File,
    Ast,
    Repo,
    Query,
    Lsp,
    Package,
    Git,
    Build,
    Test,
}

/// Exact dependency versions, supplied by the consumer after validating current state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheDependency {
    ContentHash(String),
    WorkspaceGeneration {
        workspace: String,
        generation: u64,
    },
    DocumentVersion {
        workspace: String,
        uri: String,
        version: i64,
    },
    ConfigHash(String),
    PackageManifestHash(String),
    LockfileHash(String),
    GitObject(String),
    ServerGeneration {
        workspace: String,
        generation: String,
    },
    PermissionHash(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheKey {
    namespace: CacheNamespace,
    logical: String,
    schema: u32,
    algorithm: String,
    dependencies: Vec<CacheDependency>,
}

impl CacheKey {
    pub fn new(
        namespace: CacheNamespace,
        logical: impl Into<String>,
        schema: u32,
        algorithm: impl Into<String>,
        mut dependencies: Vec<CacheDependency>,
    ) -> Self {
        dependencies.sort();
        dependencies.dedup();
        Self {
            namespace,
            logical: logical.into(),
            schema,
            algorithm: algorithm.into(),
            dependencies,
        }
    }
    pub fn namespace(&self) -> CacheNamespace {
        self.namespace
    }
    pub fn dependencies(&self) -> &[CacheDependency] {
        &self.dependencies
    }
    pub fn digest(&self) -> String {
        // This closed type has only infallibly serializable fields.
        digest(&serde_json::to_vec(self).expect("cache key serialization"))
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CachePolicy {
    NoCache,
    MemoryOnly,
    TtlBound { ttl_ms: u64 },
    Negative { ttl_ms: u64 },
    PersistentImmutable,
    PersistentWorkspaceBound,
}
impl CachePolicy {
    pub(crate) fn persistent(&self) -> bool {
        matches!(
            self,
            Self::PersistentImmutable | Self::PersistentWorkspaceBound
        )
    }
    pub(crate) fn ttl(&self) -> Option<std::time::Duration> {
        match self {
            Self::TtlBound { ttl_ms } => {
                Some(std::time::Duration::from_millis((*ttl_ms).min(86_400_000)))
            }
            Self::Negative { ttl_ms } => {
                Some(std::time::Duration::from_millis((*ttl_ms).min(5_000)))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheRequest {
    pub key: CacheKey,
    pub policy: CachePolicy,
}
impl CacheRequest {
    pub fn new(key: CacheKey, policy: CachePolicy) -> Self {
        Self { key, policy }
    }
    pub(crate) fn cacheable(&self) -> bool {
        self.policy != CachePolicy::NoCache
            && self.key.logical.len() <= 4096
            && self.key.algorithm.len() <= 256
            && self.key.dependencies.len() <= 1024
            && self
                .key
                .dependencies
                .iter()
                .all(|dep| serde_json::to_vec(dep).is_ok_and(|bytes| bytes.len() <= 8192))
            && (self.policy != CachePolicy::PersistentWorkspaceBound
                || self
                    .key
                    .dependencies
                    .iter()
                    .any(|dep| matches!(dep, CacheDependency::WorkspaceGeneration { .. })))
    }
    pub(crate) fn identity<T: 'static>(&self) -> String {
        digest(
            &serde_json::to_vec(&(&self.key, &self.policy, std::any::type_name::<T>()))
                .expect("cache request serialization"),
        )
    }
}
