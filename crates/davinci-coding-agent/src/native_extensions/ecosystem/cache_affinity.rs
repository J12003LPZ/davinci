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

/// High-level helper to derive a worker cache key directly from worker execution inputs.
pub fn derive_worker_cache_key(
    repo_id: &str,
    graph_version: u32,
    role: Role,
    model: Option<&str>,
    tools: &[String],
    system_prompt: &str,
    expect: crate::native_extensions::graph::ArtifactKind,
) -> String {
    let mut sorted_tools = tools.to_vec();
    sorted_tools.sort();
    let mut tool_hasher = Sha256::new();
    for tool in &sorted_tools {
        tool_hasher.update(tool.as_bytes());
        tool_hasher.update(b"\n");
    }
    let toolset_hash = format!("{:x}", tool_hasher.finalize());

    let mut contract_hasher = Sha256::new();
    contract_hasher.update(system_prompt.as_bytes());
    contract_hasher.update(b"\n--contract--\n");
    let contract_str = format!(
        "{}",
        crate::native_extensions::graph::validate::artifact_contract(expect)
    );
    contract_hasher.update(contract_str.as_bytes());
    let system_contract_hash = format!("{:x}", contract_hasher.finalize());

    let identity = GraphCacheIdentity {
        repo_id,
        graph_version,
        role,
        model: model.unwrap_or("default"),
        toolset_hash: &toolset_hash,
        system_contract_hash: &system_contract_hash,
    };
    graph_worker_cache_key(&identity)
}

/// Adapter: convert a graph worker's execution inputs into the universal `CacheIdentity`
/// from `davinci_agent::runtime::cache`. This preserves the existing `derive_worker_cache_key`
/// output behavior through an adapter until the full cutover to universal keys.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn graph_worker_to_universal_cache_identity(
    provider: &str,
    model: Option<&str>,
    repo_id: &str,
    graph_version: u32,
    role: Role,
    tools: &[String],
    system_prompt: &str,
    expect: crate::native_extensions::graph::ArtifactKind,
    context_item_hashes: Vec<String>,
) -> davinci_agent::CacheIdentity {
    use davinci_agent::runtime::cache::{hash_system_prompt, hash_tool_names};
    use sha2::{Digest, Sha256};

    let tool_refs: Vec<&str> = tools.iter().map(|s| s.as_str()).collect();
    let tool_schema_hash = hash_tool_names(&tool_refs);

    let mut contract_hasher = Sha256::new();
    contract_hasher.update(system_prompt.as_bytes());
    contract_hasher.update(b"\n--contract--\n");
    let contract_str = format!(
        "{}",
        crate::native_extensions::graph::validate::artifact_contract(expect)
    );
    contract_hasher.update(contract_str.as_bytes());
    let contract_hash = format!("{:x}", contract_hasher.finalize());

    let mut repo_hasher = Sha256::new();
    repo_hasher.update(repo_id.as_bytes());
    repo_hasher.update(b"\n");
    repo_hasher.update(graph_version.to_string().as_bytes());
    let repo_hash = format!("{:x}", repo_hasher.finalize());

    davinci_agent::CacheIdentity {
        provider: provider.to_string(),
        model_id: model.unwrap_or("default").to_string(),
        system_prompt_hash: hash_system_prompt(system_prompt),
        tool_schema_hash,
        permission_surface_hash: repo_hash,
        context_item_hashes,
        agent_profile_hash: None,
        contract_hash: Some(contract_hash),
        role: Some(role.as_str().to_string()),
    }
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

    #[test]
    fn derive_worker_cache_key_is_deterministic_and_sensitive() {
        use crate::native_extensions::graph::ArtifactKind;
        let tools = vec!["read".into(), "grep".into(), "graph_submit".into()];
        let k1 = derive_worker_cache_key(
            "my-repo",
            1,
            Role::Researcher,
            Some("gpt-4o"),
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
        );
        let k2 = derive_worker_cache_key(
            "my-repo",
            1,
            Role::Researcher,
            Some("gpt-4o"),
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
        );
        assert_eq!(k1, k2);
        assert!(k1.starts_with("gw-researcher-"));

        // Different tool order still yields same key due to canonical sorting
        let reversed_tools = vec!["graph_submit".into(), "grep".into(), "read".into()];
        let k_sorted = derive_worker_cache_key(
            "my-repo",
            1,
            Role::Researcher,
            Some("gpt-4o"),
            &reversed_tools,
            "system prompt",
            ArtifactKind::Evidence,
        );
        assert_eq!(k1, k_sorted);

        // Different model yields different key
        let k_diff_model = derive_worker_cache_key(
            "my-repo",
            1,
            Role::Researcher,
            Some("claude-3-5-sonnet"),
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
        );
        assert_ne!(k1, k_diff_model);
    }

    #[test]
    fn universal_cache_identity_adapter_stable_on_retry_and_invalidates_on_change() {
        use crate::native_extensions::graph::ArtifactKind;
        let tools = vec!["read".into(), "grep".into()];
        let id1 = super::graph_worker_to_universal_cache_identity(
            "openai",
            Some("gpt-4o"),
            "my-repo",
            1,
            Role::Researcher,
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
            vec![],
        );
        let id2 = super::graph_worker_to_universal_cache_identity(
            "openai",
            Some("gpt-4o"),
            "my-repo",
            1,
            Role::Researcher,
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
            vec![],
        );
        // Same inputs → same key (retry stable)
        assert_eq!(id1.cache_key(), id2.cache_key());
        assert!(id1.cache_key().starts_with("ci-researcher-"));

        // Changed model → different key (intentional invalidation)
        let id_diff = super::graph_worker_to_universal_cache_identity(
            "openai",
            Some("claude-opus-4-5"),
            "my-repo",
            1,
            Role::Researcher,
            &tools,
            "system prompt",
            ArtifactKind::Evidence,
            vec![],
        );
        assert_ne!(id1.cache_key(), id_diff.cache_key());
        let diff_reasons = id_diff.diff(&id1);
        assert!(
            diff_reasons.contains(&davinci_agent::CacheMissReason::ModelChanged),
            "Diff must report ModelChanged"
        );
    }
}
