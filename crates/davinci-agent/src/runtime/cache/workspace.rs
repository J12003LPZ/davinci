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
        // Reserve zero for malformed snapshots and keep valid generations nonzero.
        let generation = self
            .inputs_hash
            .get(..16)
            .filter(|_| {
                self.inputs_hash.len() == 64
                    && self
                        .inputs_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
            })
            .and_then(|prefix| u64::from_str_radix(prefix, 16).ok())
            .map(|generation| generation.max(1))
            .unwrap_or(0);
        vec![
            CacheDependency::WorkspaceGeneration {
                workspace: self.identity.clone(),
                generation,
            },
            CacheDependency::ContentHash(self.inputs_hash.clone()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_workspace_state() -> WorkspaceSnapshot {
        let root = tempfile::tempdir().unwrap();
        WorkspaceSnapshot::new(root.path(), Vec::new()).unwrap()
    }

    #[test]
    fn short_or_non_hex_hash_does_not_panic() {
        let mut snapshot = sample_workspace_state();
        for invalid_hash in [
            "zz".to_string(),
            "0123456789abcde".to_string(),
            "0123456789abcdeg".to_string(),
            format!("0123456789abcdef{}", "g".repeat(48)),
        ] {
            snapshot.inputs_hash = invalid_hash;
            let dependencies = snapshot.dependencies();
            assert!(dependencies.iter().any(|dependency| matches!(
                dependency,
                CacheDependency::WorkspaceGeneration { generation: 0, .. }
            )));
        }
    }

    #[test]
    fn valid_zero_hash_prefix_reserves_generation_zero() {
        let mut snapshot = sample_workspace_state();
        snapshot.inputs_hash = "0".repeat(64);
        let dependencies = snapshot.dependencies();
        assert!(dependencies.iter().any(|dependency| matches!(
            dependency,
            CacheDependency::WorkspaceGeneration { generation: 1, .. }
        )));
    }
}
