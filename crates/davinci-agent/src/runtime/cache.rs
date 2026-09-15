//! Universal cache identity and cache miss reason tracking for all agent types.
//!
//! Provides deterministic, provider-safe cache keys for agent and worker prompt-prefix
//! reuse, with reason-coded invalidation that survives retries but rejects transient state.
//!
//! Key invariant: the cache key MUST NOT include random process IDs, wall-clock time,
//! session IDs when compatible requests should share a prefix, or transient runtime state.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Inputs that constitute the stable identity of a prompt prefix, determining
/// whether two requests can share a provider prompt-cache slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheIdentity {
    /// AI provider name (e.g. "openai", "anthropic", "gemini").
    pub provider: String,
    /// Model identifier (e.g. "gpt-4o", "claude-opus-4-5").
    pub model_id: String,
    /// SHA-256 hash of the stable portion of the system prompt.
    pub system_prompt_hash: String,
    /// SHA-256 hash of the visible tool schemas (sorted, canonical).
    pub tool_schema_hash: String,
    /// SHA-256 hash of the permission-visible tool set (sorted names).
    pub permission_surface_hash: String,
    /// Optional SHA-256 hashes of stable context-packet items (e.g. memory, skills).
    pub context_item_hashes: Vec<String>,
    /// Optional agent profile name + version/hash for profile-backed workers.
    pub agent_profile_hash: Option<String>,
    /// Optional workflow/graph contract hash (e.g. artifact schema).
    pub contract_hash: Option<String>,
    /// Agent role label (e.g. "researcher", "writer", "reviewer") for prefix grouping.
    pub role: Option<String>,
}

impl CacheIdentity {
    /// Compute a deterministic, provider-safe prompt-cache key from this identity.
    /// The key is stable across process restarts and compatible runs.
    pub fn cache_key(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"cache_identity_v1\n");
        h.update(self.provider.as_bytes());
        h.update(b"\n");
        h.update(self.model_id.as_bytes());
        h.update(b"\n");
        h.update(self.system_prompt_hash.as_bytes());
        h.update(b"\n");
        h.update(self.tool_schema_hash.as_bytes());
        h.update(b"\n");
        h.update(self.permission_surface_hash.as_bytes());
        h.update(b"\n");
        // context items: sorted for stability
        let mut sorted_ctx = self.context_item_hashes.clone();
        sorted_ctx.sort();
        for item_hash in &sorted_ctx {
            h.update(item_hash.as_bytes());
            h.update(b"\n");
        }
        if let Some(profile) = &self.agent_profile_hash {
            h.update(b"--profile--\n");
            h.update(profile.as_bytes());
            h.update(b"\n");
        }
        if let Some(contract) = &self.contract_hash {
            h.update(b"--contract--\n");
            h.update(contract.as_bytes());
            h.update(b"\n");
        }
        if let Some(role) = &self.role {
            h.update(b"--role--\n");
            h.update(role.as_bytes());
            h.update(b"\n");
        }
        let hex = format!("{:x}", h.finalize());
        let short = &hex[..16];
        let prefix = self
            .role
            .as_deref()
            .map(|r| format!("{}-", r))
            .unwrap_or_default();
        format!("ci-{}{}", prefix, short)
    }

    /// Compute a reason-coded diff describing what changed compared to a previous identity.
    pub fn diff(&self, previous: &CacheIdentity) -> Vec<CacheMissReason> {
        let mut reasons = Vec::new();
        if self.provider != previous.provider {
            reasons.push(CacheMissReason::ProviderChanged);
        }
        if self.model_id != previous.model_id {
            reasons.push(CacheMissReason::ModelChanged);
        }
        if self.system_prompt_hash != previous.system_prompt_hash {
            reasons.push(CacheMissReason::SystemPromptChanged);
        }
        if self.tool_schema_hash != previous.tool_schema_hash {
            reasons.push(CacheMissReason::ToolSchemaChanged);
        }
        if self.permission_surface_hash != previous.permission_surface_hash {
            reasons.push(CacheMissReason::PermissionSurfaceChanged);
        }
        {
            let mut a = self.context_item_hashes.clone();
            let mut b = previous.context_item_hashes.clone();
            a.sort();
            b.sort();
            if a != b {
                reasons.push(CacheMissReason::ContextChanged);
            }
        }
        if self.agent_profile_hash != previous.agent_profile_hash {
            reasons.push(CacheMissReason::AgentProfileChanged);
        }
        if self.contract_hash != previous.contract_hash {
            reasons.push(CacheMissReason::ContractChanged);
        }
        reasons
    }
}

/// Reason-coded explanation for a prompt-cache miss between two CacheIdentity values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMissReason {
    ProviderChanged,
    ModelChanged,
    SystemPromptChanged,
    ToolSchemaChanged,
    PermissionSurfaceChanged,
    ContextChanged,
    AgentProfileChanged,
    ContractChanged,
    Unknown,
}

impl std::fmt::Display for CacheMissReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CacheMissReason::ProviderChanged => "provider_changed",
            CacheMissReason::ModelChanged => "model_changed",
            CacheMissReason::SystemPromptChanged => "system_prompt_changed",
            CacheMissReason::ToolSchemaChanged => "tool_schema_changed",
            CacheMissReason::PermissionSurfaceChanged => "permission_surface_changed",
            CacheMissReason::ContextChanged => "context_changed",
            CacheMissReason::AgentProfileChanged => "agent_profile_changed",
            CacheMissReason::ContractChanged => "contract_changed",
            CacheMissReason::Unknown => "unknown",
        };
        write!(f, "{}", s)
    }
}

/// Compute a SHA-256 hash of an ordered list of tool names (sorted for stability).
pub fn hash_tool_names(tools: &[&str]) -> String {
    let mut sorted: Vec<&str> = tools.to_vec();
    sorted.sort();
    let mut h = Sha256::new();
    for t in &sorted {
        h.update(t.as_bytes());
        h.update(b"\n");
    }
    format!("{:x}", h.finalize())
}

/// Compute a SHA-256 hash of a system prompt string.
pub fn hash_system_prompt(prompt: &str) -> String {
    let mut h = Sha256::new();
    h.update(prompt.as_bytes());
    format!("{:x}", h.finalize())
}

/// Compute a SHA-256 hash of a system prompt, preferring stable_sha256 from manifest if available.
pub fn hash_system_prompt_with_manifest(
    prompt: &str,
    manifest: Option<&crate::prompt::manifest::PromptManifest>,
) -> String {
    if let Some(m) = manifest {
        m.stable_sha256.clone()
    } else {
        hash_system_prompt(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_identity() -> CacheIdentity {
        CacheIdentity {
            provider: "openai".into(),
            model_id: "gpt-4o".into(),
            system_prompt_hash: hash_system_prompt("You are a helpful assistant."),
            tool_schema_hash: hash_tool_names(&["read", "write", "edit"]),
            permission_surface_hash: hash_tool_names(&["read"]),
            context_item_hashes: vec!["ctx-hash-a".into(), "ctx-hash-b".into()],
            agent_profile_hash: None,
            contract_hash: None,
            role: Some("writer".into()),
        }
    }

    #[test]
    fn test_cache_key_is_stable_across_retries() {
        let id = sample_identity();
        let k1 = id.cache_key();
        let k2 = id.cache_key();
        assert_eq!(k1, k2, "Cache key must be deterministic");
        assert!(
            k1.starts_with("ci-writer-"),
            "Key should include role prefix"
        );
    }

    #[test]
    fn test_cache_key_changes_on_intentional_invalidation() {
        let id1 = sample_identity();
        let mut id2 = sample_identity();
        id2.model_id = "claude-opus-4-5".into();
        assert_ne!(id1.cache_key(), id2.cache_key());

        let mut id3 = sample_identity();
        id3.system_prompt_hash = hash_system_prompt("Different system prompt");
        assert_ne!(id1.cache_key(), id3.cache_key());

        let mut id4 = sample_identity();
        id4.contract_hash = Some("some-contract-hash".into());
        assert_ne!(id1.cache_key(), id4.cache_key());
    }

    #[test]
    fn test_context_items_order_invariant() {
        let mut id1 = sample_identity();
        id1.context_item_hashes = vec!["ctx-b".into(), "ctx-a".into()];
        let mut id2 = sample_identity();
        id2.context_item_hashes = vec!["ctx-a".into(), "ctx-b".into()];
        // Same items in different order → same key (sorted internally)
        assert_eq!(id1.cache_key(), id2.cache_key());
    }

    #[test]
    fn test_diff_detects_all_change_reasons() {
        let prev = sample_identity();

        let mut changed_model = sample_identity();
        changed_model.model_id = "gpt-4-turbo".into();
        let reasons = changed_model.diff(&prev);
        assert!(reasons.contains(&CacheMissReason::ModelChanged));
        assert!(!reasons.contains(&CacheMissReason::SystemPromptChanged));

        let mut changed_prompt = sample_identity();
        changed_prompt.system_prompt_hash = hash_system_prompt("New prompt");
        let reasons2 = changed_prompt.diff(&prev);
        assert!(reasons2.contains(&CacheMissReason::SystemPromptChanged));

        let mut changed_ctx = sample_identity();
        changed_ctx.context_item_hashes = vec!["ctx-new".into()];
        let reasons3 = changed_ctx.diff(&prev);
        assert!(reasons3.contains(&CacheMissReason::ContextChanged));
    }

    #[test]
    fn test_hash_tool_names_is_order_independent() {
        let h1 = hash_tool_names(&["read", "write", "edit"]);
        let h2 = hash_tool_names(&["edit", "read", "write"]);
        assert_eq!(h1, h2);
    }
}
