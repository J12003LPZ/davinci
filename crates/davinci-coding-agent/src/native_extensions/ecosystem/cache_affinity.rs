//! Cache affinity identity and provider cache key generation for graph workers.

use crate::native_extensions::graph::Role;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Provider-reported token counters for one Graph role. These values are kept
/// separate from local cache identity diagnostics so a stable hash is never
/// mistaken for a provider cache hit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleCacheStats {
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub turns: u64,
}

impl RoleCacheStats {
    pub fn from_usage(usage: &crate::native_extensions::graph::WorkerUsage) -> Self {
        Self {
            input_tokens: usage.input,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
            turns: usage.turns,
        }
    }

    pub fn add(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
        self.turns = self.turns.saturating_add(other.turns);
    }

    /// Provider cache-read ratio over all provider input buckets.
    ///
    /// Returns `None` when no provider input was reported so callers do not
    /// mislabel missing telemetry as a measured 0% cache-read ratio.
    #[allow(dead_code)]
    pub fn provider_cache_read_ratio(&self) -> Option<f64> {
        let total_input = self
            .input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens);
        (total_input > 0).then(|| self.cache_read_tokens as f64 / total_input as f64)
    }
}

/// One worker's provider usage paired with local cache identity diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheObservation {
    pub role: Role,
    pub provider_usage: RoleCacheStats,
    pub identity: davinci_agent::CacheIdentity,
    #[serde(default)]
    pub miss_reasons: Vec<davinci_agent::CacheMissReason>,
}

impl CacheObservation {
    #[allow(dead_code)]
    pub fn new(
        role: Role,
        usage: &crate::native_extensions::graph::WorkerUsage,
        identity: davinci_agent::CacheIdentity,
        previous_identity: Option<&davinci_agent::CacheIdentity>,
    ) -> Self {
        let miss_reasons = previous_identity
            .map(|previous| identity.diff(previous))
            .unwrap_or_default();
        Self {
            role,
            provider_usage: RoleCacheStats::from_usage(usage),
            identity,
            miss_reasons,
        }
    }
}

#[allow(dead_code)]
pub fn aggregate_role_cache_stats(
    observations: &[CacheObservation],
) -> BTreeMap<Role, RoleCacheStats> {
    let mut totals = BTreeMap::new();
    for observation in observations {
        totals
            .entry(observation.role)
            .or_insert_with(RoleCacheStats::default)
            .add(&observation.provider_usage);
    }
    totals
}

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
    let candidate = davinci_agent::runtime::cache::legacy_prompt_cache_key(
        "graph_worker_cache_v1",
        &[
            input.repo_id,
            &input.graph_version.to_string(),
            input.role.as_str(),
            input.model,
            input.toolset_hash,
            input.system_contract_hash,
        ],
        &format!("gw-{}-", input.role.as_str()),
    );
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
    let registry = davinci_agent::RuntimeCapabilityRegistry::with_builtins();
    let toolset_hash = registry.hash_tool_capabilities(tools);

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
    use davinci_agent::runtime::cache::hash_system_prompt;
    use sha2::{Digest, Sha256};

    let registry = davinci_agent::RuntimeCapabilityRegistry::with_builtins();
    let tool_schema_hash = registry.hash_tool_capabilities(tools);

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

    fn sample_universal_identity() -> davinci_agent::CacheIdentity {
        davinci_agent::CacheIdentity {
            provider: "fixture".into(),
            model_id: "fixture-model".into(),
            system_prompt_hash: "prompt".into(),
            tool_schema_hash: "tools".into(),
            permission_surface_hash: "permissions".into(),
            context_item_hashes: Vec::new(),
            agent_profile_hash: None,
            contract_hash: None,
            role: Some(Role::Researcher.as_str().into()),
        }
    }

    #[test]
    fn graph_cache_stats_aggregate_provider_usage_by_role() {
        use crate::native_extensions::graph::WorkerUsage;

        let identity = sample_universal_identity();
        let observations = [
            CacheObservation::new(
                Role::Researcher,
                &WorkerUsage {
                    input: 100,
                    cache_read: 40,
                    cache_write: 5,
                    turns: 1,
                    ..WorkerUsage::default()
                },
                identity.clone(),
                None,
            ),
            CacheObservation::new(
                Role::Researcher,
                &WorkerUsage {
                    input: 80,
                    cache_read: 20,
                    cache_write: 3,
                    turns: 2,
                    ..WorkerUsage::default()
                },
                identity.clone(),
                Some(&identity),
            ),
            CacheObservation::new(
                Role::Writer,
                &WorkerUsage {
                    input: 50,
                    cache_read: 0,
                    cache_write: 10,
                    turns: 1,
                    ..WorkerUsage::default()
                },
                identity,
                None,
            ),
        ];
        let totals = aggregate_role_cache_stats(&observations);
        assert_eq!(totals[&Role::Researcher].input_tokens, 180);
        assert_eq!(totals[&Role::Researcher].cache_read_tokens, 60);
        assert_eq!(totals[&Role::Researcher].cache_write_tokens, 8);
        assert_eq!(totals[&Role::Researcher].turns, 3);
        assert_eq!(totals[&Role::Writer].cache_write_tokens, 10);
        let ratio = totals[&Role::Researcher]
            .provider_cache_read_ratio()
            .expect("provider input was reported");
        assert!((ratio - (60.0 / 248.0)).abs() < 1e-9);
    }

    #[test]
    fn local_cache_identity_is_not_reported_as_provider_hit() {
        use crate::native_extensions::graph::WorkerUsage;

        let identity = sample_universal_identity();
        let observation = CacheObservation::new(
            Role::Researcher,
            &WorkerUsage::default(),
            identity.clone(),
            Some(&identity),
        );
        let encoded = serde_json::to_string(&observation).unwrap();
        assert!(observation.miss_reasons.is_empty());
        assert_eq!(observation.provider_usage.provider_cache_read_ratio(), None);
        assert!(!encoded.contains("providerCacheHit"));
        assert!(!encoded.contains("localCacheHit"));
    }
}
