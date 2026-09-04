//! Request-shape and stable-prefix hashing matching §8 and §12.
//! Detects when model, tool schema, instructions, or permissions invalidate cached provider state.

use sha2::{Digest, Sha256};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct RequestShapeParams<'a> {
    pub model_id: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub instructions: &'a str,
    pub tools: &'a [Value],
    pub permissions_summary: &'a str,
    pub cache_mode: &'a str,
    pub feature_flags: &'a [&'a str],
    pub backend_family: &'a str,
    pub compaction_lineage: Option<&'a str>,
}

/// Computes the SHA-256 hash identifying the request shape.
/// Any change triggers a new lineage boundary or forces full replay.
pub fn compute_request_shape_hash(params: &RequestShapeParams<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"model:");
    hasher.update(params.model_id.as_bytes());
    hasher.update(b"\neffort:");
    hasher.update(params.reasoning_effort.unwrap_or("none").as_bytes());
    hasher.update(b"\ninstructions:");
    hasher.update(params.instructions.as_bytes());
    hasher.update(b"\ntools:");
    let mut sorted_tools: Vec<&Value> = params.tools.iter().collect();
    sorted_tools.sort_by_key(|t| {
        t.get("name")
            .or_else(|| t.pointer("/function/name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    });
    for tool in sorted_tools {
        hasher.update(tool.to_string().as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"permissions:");
    hasher.update(params.permissions_summary.as_bytes());
    hasher.update(b"\ncache_mode:");
    hasher.update(params.cache_mode.as_bytes());
    hasher.update(b"\nflags:");
    for flag in params.feature_flags {
        hasher.update(flag.as_bytes());
        hasher.update(b",");
    }
    hasher.update(b"\nbackend:");
    hasher.update(params.backend_family.as_bytes());
    hasher.update(b"\ncompaction:");
    hasher.update(params.compaction_lineage.unwrap_or("none").as_bytes());

    format!("{:x}", hasher.finalize())
}

/// Computes the stable-prefix hash for prompt caching across turns.
pub fn compute_stable_prefix_hash(
    base_instructions: &str,
    repository_instructions: &str,
    tool_definitions: &[Value],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"base:");
    hasher.update(base_instructions.as_bytes());
    hasher.update(b"\nrepo:");
    hasher.update(repository_instructions.as_bytes());
    hasher.update(b"\ntools:");
    let mut sorted_tools: Vec<&Value> = tool_definitions.iter().collect();
    sorted_tools.sort_by_key(|t| {
        t.get("name")
            .or_else(|| t.pointer("/function/name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    });
    for tool in sorted_tools {
        hasher.update(tool.to_string().as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deterministic_and_sensitive_to_changes() {
        let tools = vec![json!({"name": "read", "type": "function"})];
        let params1 = RequestShapeParams {
            model_id: "gpt-5-codex",
            reasoning_effort: Some("high"),
            instructions: "You are an assistant.",
            tools: &tools,
            permissions_summary: "read_only",
            cache_mode: "auto",
            feature_flags: &["websocket", "lossless"],
            backend_family: "chatgpt_oauth",
            compaction_lineage: None,
        };
        let hash1 = compute_request_shape_hash(&params1);
        let hash1_again = compute_request_shape_hash(&params1);
        assert_eq!(hash1, hash1_again);

        // Change tool
        let tools2 = vec![
            json!({"name": "read", "type": "function"}),
            json!({"name": "apply_patch", "type": "custom"}),
        ];
        let params2 = RequestShapeParams {
            tools: &tools2,
            ..params1
        };
        let hash2 = compute_request_shape_hash(&params2);
        assert_ne!(hash1, hash2);

        // Change effort
        let params3 = RequestShapeParams {
            reasoning_effort: Some("low"),
            ..params1
        };
        let hash3 = compute_request_shape_hash(&params3);
        assert_ne!(hash1, hash3);
    }
}
