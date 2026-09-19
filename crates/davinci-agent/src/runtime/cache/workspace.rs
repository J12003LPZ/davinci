use super::{digest, CacheDependency, CacheError};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A mutable workspace view, isolated by canonical root and exact input versions.
/// Callers supply the complete set of inputs their aggregate actually depends on.
/// Per-file immutable artifacts should use ContentHash without this workspace token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    identity: String,
    inputs_hash: String,
}

impl WorkspaceSnapshot {
    pub fn new(root: &Path, mut inputs: Vec<(String, String)>) -> Result<Self, CacheError> {
        let root = root
            .canonicalize()
            .map_err(|e| CacheError::Compute(e.to_string()))?;
        let root = root
            .to_str()
            .ok_or_else(|| CacheError::Compute("workspace path is not UTF-8".into()))?;
        inputs.sort();
        inputs.dedup();
        let encoded =
            serde_json::to_vec(&inputs).map_err(|e| CacheError::Compute(e.to_string()))?;
        Ok(Self {
            identity: digest(root.as_bytes()),
            inputs_hash: digest(&encoded),
        })
    }

    pub fn dependencies(&self) -> Vec<CacheDependency> {
        // Preserve the full digest as well as the numeric generation. Neither
        // timestamps nor a process-local counter can make old state look current.
        let generation = u64::from_str_radix(&self.inputs_hash[..16], 16).expect("SHA-256 hex");
        vec![
            CacheDependency::WorkspaceGeneration {
                workspace: self.identity.clone(),
                generation,
            },
            CacheDependency::ContentHash(self.inputs_hash.clone()),
        ]
    }
}
