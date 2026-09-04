//! Cache affinity identity and provider cache key generation for graph workers.

use crate::native_extensions::graph::Role;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCacheIdentity<'a> {
    pub repo_id: &'a str,
    pub graph_version: u32,
    pub role: Role,
    pub model: &'a str,
    pub toolset_hash: &'a str,
    pub system_contract_hash: &'a str,
}

/// Generate a stable, provider-safe prompt cache key for an ephemeral graph worker.
/// The key is derived purely from (repo_id, graph_version, role, model, toolset, system_contract)
/// and specifically excludes run IDs or timestamps so compatible runs and retries reuse cache slots.
pub fn graph_worker_cache_key(input: &GraphCacheIdentity<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"graph_worker_cache_v1\n");
    hasher.update(input.repo_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(input.graph_version.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(input.role.as_str().as_bytes());
    hasher.update(b"\n");
    hasher.update(input.model.as_bytes());
    hasher.update(b"\n");
    hasher.update(input.toolset_hash.as_bytes());
    hasher.update(b"\n");
    hasher.update(input.system_contract_hash.as_bytes());

    let hash_hex = format!("{:x}", hasher.finalize());
    let short_hash = &hash_hex[..16];
    let candidate = format!("gw-{}-{}", input.role.as_str(), short_hash);
    davinci_ai::cache::clamp_openai_prompt_cache_key(&candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_across_runs_and_sensitive_to_contract_inputs() {
        let id1 = GraphCacheIdentity {
            repo_id: "repo-123",
            graph_version: 1,
            role: Role::Researcher,
            model: "gpt-4o",
            toolset_hash: "tools_hash_abc",
            system_contract_hash: "contract_hash_xyz",
        };
        let key1 = graph_worker_cache_key(&id1);
        let key2 = graph_worker_cache_key(&id1);
        assert_eq!(key1, key2);
        assert!(key1.starts_with("gw-researcher-"));
        assert!(key1.len() <= davinci_ai::cache::OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH);

        // Model change changes key
        let id_model = GraphCacheIdentity {
            model: "claude-3-5-sonnet",
            ..id1.clone()
        };
        assert_ne!(graph_worker_cache_key(&id_model), key1);

        // Role change changes key
        let id_role = GraphCacheIdentity {
            role: Role::Writer,
            ..id1.clone()
        };
        assert_ne!(graph_worker_cache_key(&id_role), key1);

        // Toolset hash change changes key
        let id_tools = GraphCacheIdentity {
            toolset_hash: "tools_hash_different",
            ..id1.clone()
        };
        assert_ne!(graph_worker_cache_key(&id_tools), key1);

        // Graph version change changes key
        let id_ver = GraphCacheIdentity {
            graph_version: 2,
            ..id1.clone()
        };
        assert_ne!(graph_worker_cache_key(&id_ver), key1);

        // System contract change changes key
        let id_contract = GraphCacheIdentity {
            system_contract_hash: "contract_hash_different",
            ..id1.clone()
        };
        assert_ne!(graph_worker_cache_key(&id_contract), key1);
    }
}
