//! Context packet contracts and deterministic bounds for graph workers.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[allow(unused_imports)]
pub use crate::native_extensions::learning::{
    format_skill_context, select_graph_skill_candidates, SkillContextCandidate,
};
#[allow(unused_imports)]
pub use crate::native_extensions::vector_memory::{format_memory_context, MemoryContextHit};

pub const DEFAULT_GRAPH_CONTEXT_TOKENS: usize = 2_500;
pub const DEFAULT_GRAPH_MEMORY_TOKENS: usize = 1_200;
pub const DEFAULT_GRAPH_MEMORY_HITS: usize = 4;
pub const DEFAULT_GRAPH_SKILL_TOKENS: usize = 1_000;
pub const DEFAULT_GRAPH_SKILL_COUNT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPacketRequest<'a> {
    pub prompt: &'a str,
    pub role: Option<crate::native_extensions::graph::Role>,
    pub token_cap: usize,
    pub include_skills: bool,
}

#[allow(dead_code)]
impl<'a> ContextPacketRequest<'a> {
    pub fn new(prompt: &'a str) -> Self {
        Self {
            prompt,
            role: None,
            token_cap: DEFAULT_GRAPH_CONTEXT_TOKENS,
            include_skills: true,
        }
    }

    pub fn with_role(mut self, role: crate::native_extensions::graph::Role) -> Self {
        self.role = Some(role);
        self
    }

    pub fn with_token_cap(mut self, cap: usize) -> Self {
        self.token_cap = cap;
        self
    }

    pub fn with_skills(mut self, include: bool) -> Self {
        self.include_skills = include;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillContextRef {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextPacket {
    pub text: String,
    pub memory_refs: Vec<String>,
    pub skill_refs: Vec<SkillContextRef>,
    pub estimated_tokens: usize,
    pub fingerprint: String,
}

#[allow(dead_code)]
impl ContextPacket {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.memory_refs.is_empty() && self.skill_refs.is_empty()
    }
}

/// Compute a deterministic fingerprint for context content (canonical text + memory refs + skill refs).
/// Excludes run IDs and timestamps so identical retrieved context yields identical fingerprints.
pub fn compute_context_fingerprint(
    text: &str,
    memory_refs: &[String],
    skill_refs: &[SkillContextRef],
) -> String {
    if text.is_empty() && memory_refs.is_empty() && skill_refs.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"context_packet_v1\n");
    hasher.update(text.as_bytes());
    hasher.update(b"\n--memory-refs--\n");
    for r in memory_refs {
        hasher.update(r.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"--skill-refs--\n");
    for s in skill_refs {
        hasher.update(s.name.as_bytes());
        hasher.update(b":");
        hasher.update(s.version.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(s.content_hash.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_fingerprint_is_deterministic_and_excludes_run_metadata() {
        let text = "<context>sample</context>";
        let mem = vec!["mem-1".into(), "mem-2".into()];
        let skills = vec![SkillContextRef {
            name: "test-skill".into(),
            version: 1,
            content_hash: "abc123hash".into(),
        }];

        let fp1 = compute_context_fingerprint(text, &mem, &skills);
        let fp2 = compute_context_fingerprint(text, &mem, &skills);
        assert_eq!(fp1, fp2);
        assert!(!fp1.is_empty());

        // Any change alters fingerprint
        let fp3 = compute_context_fingerprint("altered text", &mem, &skills);
        assert_ne!(fp1, fp3);

        // Empty returns empty
        assert_eq!(compute_context_fingerprint("", &[], &[]), "");
    }
}
