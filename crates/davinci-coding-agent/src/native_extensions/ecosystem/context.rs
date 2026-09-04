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

pub use crate::native_extensions::learning::types::SkillVersionRef as SkillContextRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextPacket {
    pub text: String,
    pub memory_refs: Vec<String>,
    pub skill_refs: Vec<SkillContextRef>,
    pub estimated_tokens: usize,
    pub fingerprint: String,
    #[serde(default)]
    pub memory_tokens: usize,
    #[serde(default)]
    pub skill_tokens: usize,
    #[serde(default)]
    pub skill_candidates_considered: usize,
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

pub fn assemble_packet_text(
    memory_hits: &[MemoryContextHit],
    skill_candidates: &[SkillContextCandidate],
) -> String {
    if memory_hits.is_empty() && skill_candidates.is_empty() {
        return String::new();
    }
    let mut out = String::from("<context source=\"davinci\" untrusted=\"true\">\n");
    if !memory_hits.is_empty() {
        out.push_str(&format_memory_context(memory_hits));
        out.push('\n');
    }
    if !skill_candidates.is_empty() {
        out.push_str(&format_skill_context(skill_candidates));
        out.push('\n');
    }
    out.push_str("</context>");
    out
}

pub fn build_context_packet(
    memory: &crate::native_extensions::VectorMemory,
    learning: &crate::native_extensions::LearningController,
    request: ContextPacketRequest<'_>,
) -> ContextPacket {
    if request.token_cap == 0 || request.prompt.trim().is_empty() {
        return ContextPacket::empty();
    }

    let memory_cap = DEFAULT_GRAPH_MEMORY_TOKENS.min(request.token_cap);
    let skill_cap = DEFAULT_GRAPH_SKILL_TOKENS.min(request.token_cap);

    let mut memory_hits =
        memory.context_hits(request.prompt, DEFAULT_GRAPH_MEMORY_HITS, memory_cap);
    let mut skill_candidates = if request.include_skills {
        let role = request
            .role
            .unwrap_or(crate::native_extensions::graph::Role::Writer);
        learning.graph_skill_candidates(request.prompt, role, DEFAULT_GRAPH_SKILL_COUNT, skill_cap)
    } else {
        Vec::new()
    };
    let skill_candidates_considered = skill_candidates.len();

    if memory_hits.is_empty() && skill_candidates.is_empty() {
        return ContextPacket::empty();
    }

    let mut text = assemble_packet_text(&memory_hits, &skill_candidates);
    let mut est_tokens = (text.chars().count() + 3) / 4;

    // Enforce aggregate token_cap: trimming order is lowest-ranked context first
    while est_tokens > request.token_cap
        && (!memory_hits.is_empty() || !skill_candidates.is_empty())
    {
        match (memory_hits.last(), skill_candidates.last()) {
            (Some(m), Some(s)) => {
                if m.score <= s.score {
                    memory_hits.pop();
                } else {
                    skill_candidates.pop();
                }
            }
            (Some(_), None) => {
                if memory_hits.len() > 1 {
                    memory_hits.pop();
                } else {
                    break;
                }
            }
            (None, Some(_)) => {
                if skill_candidates.len() > 1 {
                    skill_candidates.pop();
                } else {
                    break;
                }
            }
            (None, None) => break,
        }
        text = assemble_packet_text(&memory_hits, &skill_candidates);
        est_tokens = (text.chars().count() + 3) / 4;
    }

    // If still over cap with 1 item remaining, truncate the remaining item
    if est_tokens > request.token_cap {
        let char_budget = request.token_cap.saturating_mul(4);
        if char_budget < 60 {
            return ContextPacket::empty();
        }
        if let Some(s) = skill_candidates.last_mut() {
            let inner_cap = char_budget.saturating_sub(60);
            let truncated: String = s.body.chars().take(inner_cap).collect();
            s.body = truncated;
            s.estimated_tokens = (s.body.chars().count() + 3) / 4;
            text = assemble_packet_text(&memory_hits, &skill_candidates);
            est_tokens = (text.chars().count() + 3) / 4;
        } else if let Some(m) = memory_hits.last_mut() {
            let inner_cap = char_budget.saturating_sub(60);
            let truncated: String = m.text.chars().take(inner_cap).collect();
            m.text = truncated;
            m.estimated_tokens = (m.text.chars().count() + 3) / 4;
            text = assemble_packet_text(&memory_hits, &skill_candidates);
            est_tokens = (text.chars().count() + 3) / 4;
        }
    }

    if memory_hits.is_empty() && skill_candidates.is_empty() {
        return ContextPacket::empty();
    }

    let memory_tokens: usize = memory_hits.iter().map(|m| m.estimated_tokens).sum();
    let skill_tokens: usize = skill_candidates.iter().map(|s| s.estimated_tokens).sum();
    let memory_refs = memory_hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
    let skill_refs = skill_candidates
        .into_iter()
        .map(|s| SkillContextRef {
            name: s.name,
            version: s.version,
            content_hash: s.content_hash,
        })
        .collect::<Vec<_>>();
    let fingerprint = compute_context_fingerprint(&text, &memory_refs, &skill_refs);

    ContextPacket {
        text,
        memory_refs,
        skill_refs,
        estimated_tokens: est_tokens,
        fingerprint,
        memory_tokens,
        skill_tokens,
        skill_candidates_considered,
    }
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

    #[test]
    fn test_assemble_packet_text_format() {
        let mem = vec![MemoryContextHit {
            id: "m1".into(),
            text: "indexed memory fact".into(),
            score: 0.8,
            estimated_tokens: 5,
        }];
        let skills = vec![SkillContextCandidate {
            name: "test-skill".into(),
            version: 1,
            content_hash: "hash".into(),
            body: "# Test\nAction instructions.".into(),
            score: 0.9,
            estimated_tokens: 6,
        }];

        let text = assemble_packet_text(&mem, &skills);
        assert!(text.starts_with("<context source=\"davinci\" untrusted=\"true\">"));
        assert!(text.ends_with("</context>"));
        assert!(text.contains("<memory>"));
        assert!(text.contains("<skill name=\"test-skill\" version=\"1\">"));

        // Empty returns empty string
        assert_eq!(assemble_packet_text(&[], &[]), "");
    }

    #[test]
    fn test_empty_packet_has_zero_tokens_and_empty_text() {
        let packet = ContextPacket::empty();
        assert!(packet.is_empty());
        assert_eq!(packet.estimated_tokens, 0);
        assert_eq!(packet.text, "");
        assert_eq!(packet.fingerprint, "");
    }

    #[test]
    fn test_build_context_packet_enforces_aggregate_cap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let memory = crate::native_extensions::VectorMemory::new(temp_dir.path().to_path_buf());
        let learning =
            crate::native_extensions::LearningController::new(temp_dir.path(), None, None);

        // Blank query produces empty packet
        let req_blank = ContextPacketRequest::new("   ");
        let empty_pkt = build_context_packet(&memory, &learning, req_blank);
        assert!(empty_pkt.is_empty());
        assert_eq!(empty_pkt.estimated_tokens, 0);

        // Cap of 0 produces empty packet
        let req_zero = ContextPacketRequest::new("test prompt").with_token_cap(0);
        let zero_pkt = build_context_packet(&memory, &learning, req_zero);
        assert!(zero_pkt.is_empty());
    }
}
