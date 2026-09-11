//! Complete prepared context manifest representing the final provider-view context.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ids::RunId;

/// Categorization of provenance for an item in the context manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    UserDecision,
    RepositoryFact,
    AgentInference,
    ToolEvidence,
    MandatoryPolicy,
}

impl ProvenanceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProvenanceKind::UserDecision => "user_decision",
            ProvenanceKind::RepositoryFact => "repository_fact",
            ProvenanceKind::AgentInference => "agent_inference",
            ProvenanceKind::ToolEvidence => "tool_evidence",
            ProvenanceKind::MandatoryPolicy => "mandatory_policy",
        }
    }
}

/// A discrete entry within the prepared context manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextManifestEntry {
    pub id: String,
    pub category: String,
    pub provenance_kind: ProvenanceKind,
    pub source_ref: String,
    pub content_hash: String,
    pub token_estimate: u64,
    pub selected: bool,
    pub selection_reason: Option<String>,
    pub mandatory: bool,
    pub freshness: String,
    pub inspect_ref: Option<String>,
}

impl ContextManifestEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        category: impl Into<String>,
        provenance_kind: ProvenanceKind,
        source_ref: impl Into<String>,
        content_hash: impl Into<String>,
        token_estimate: u64,
        selected: bool,
        selection_reason: Option<String>,
        mandatory: bool,
        freshness: impl Into<String>,
        inspect_ref: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            provenance_kind,
            source_ref: source_ref.into(),
            content_hash: content_hash.into(),
            token_estimate,
            selected,
            selection_reason,
            mandatory,
            freshness: freshness.into(),
            inspect_ref,
        }
    }

    /// Computes a stable content hash for string content.
    pub fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Complete prepared context manifest sent to the provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedContextManifest {
    pub request_id: String,
    pub root_run_id: RunId,
    pub source_revision: u64,
    pub overlay_revision: u64,
    pub entries: Vec<ContextManifestEntry>,
    pub estimated_total_tokens: u64,
    pub estimated_overhead_tokens: u64,
    pub manifest_digest: String,
}

impl PreparedContextManifest {
    pub fn new(
        request_id: impl Into<String>,
        root_run_id: RunId,
        source_revision: u64,
        overlay_revision: u64,
        entries: Vec<ContextManifestEntry>,
        estimated_overhead_tokens: u64,
    ) -> Self {
        let request_id = request_id.into();
        let estimated_total_tokens = entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.token_estimate)
            .sum::<u64>()
            + estimated_overhead_tokens;

        let digest = Self::compute_digest(
            &request_id,
            root_run_id,
            source_revision,
            overlay_revision,
            &entries,
            estimated_overhead_tokens,
        );

        Self {
            request_id,
            root_run_id,
            source_revision,
            overlay_revision,
            entries,
            estimated_total_tokens,
            estimated_overhead_tokens,
            manifest_digest: digest,
        }
    }

    pub fn compute_digest(
        request_id: &str,
        root_run_id: RunId,
        source_revision: u64,
        overlay_revision: u64,
        entries: &[ContextManifestEntry],
        overhead: u64,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(request_id.as_bytes());
        hasher.update(b":");
        hasher.update(root_run_id.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(source_revision.to_le_bytes());
        hasher.update(b":");
        hasher.update(overlay_revision.to_le_bytes());
        hasher.update(b":");
        hasher.update(overhead.to_le_bytes());
        for entry in entries {
            hasher.update(b"|");
            hasher.update(entry.id.as_bytes());
            hasher.update(entry.content_hash.as_bytes());
            hasher.update(if entry.selected { b"1" } else { b"0" });
            hasher.update(entry.token_estimate.to_le_bytes());
            hasher.update(if entry.mandatory { b"M" } else { b"O" });
        }
        format!("{:x}", hasher.finalize())
    }

    pub fn selected_entries(&self) -> impl Iterator<Item = &ContextManifestEntry> {
        self.entries.iter().filter(|e| e.selected)
    }

    pub fn mandatory_entries(&self) -> impl Iterator<Item = &ContextManifestEntry> {
        self.entries.iter().filter(|e| e.mandatory)
    }

    /// Checks if any mandatory policy item is unselected / excluded.
    pub fn is_mandatory_violated(&self) -> bool {
        self.entries.iter().any(|e| e.mandatory && !e.selected)
    }
}

/// Determines the overlay action: "unchanged" if revisions match; "next_request" if request is in flight; "reprepare" if idle.
pub fn overlay_application(
    in_flight: bool,
    captured_revision: u64,
    current_revision: u64,
) -> &'static str {
    if captured_revision == current_revision {
        "unchanged"
    } else if in_flight {
        "next_request"
    } else {
        "reprepare"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f08_overlay_next_request() {
        assert_eq!(overlay_application(true, 3, 4), "next_request");
        assert_eq!(overlay_application(false, 3, 4), "reprepare");
        assert_eq!(overlay_application(false, 4, 4), "unchanged");
    }

    #[test]
    fn test_stream_starts_during_overlay_edit() {
        // While streaming, overlay change is pending for next request boundary
        let action = overlay_application(true, 10, 11);
        assert_eq!(action, "next_request");
    }

    #[test]
    fn test_mandatory_content_unavailable_blocks_request() {
        let run_id = RunId::new();
        let mandatory_entry = ContextManifestEntry::new(
            "security_policy",
            "system",
            ProvenanceKind::MandatoryPolicy,
            "policy::mandatory",
            "hash_pol",
            100,
            false, // unselected / unavailable!
            Some("unavailable".into()),
            true,
            "stale",
            None,
        );
        let manifest =
            PreparedContextManifest::new("req-test", run_id, 1, 1, vec![mandatory_entry], 0);
        assert!(manifest.is_mandatory_violated());
    }

    #[test]
    fn test_repeated_inspector_opening() {
        let run_id = RunId::new();
        let entry = ContextManifestEntry::new(
            "doc1",
            "memory",
            ProvenanceKind::RepositoryFact,
            "file::doc1",
            "hash_doc1",
            50,
            true,
            Some("selected".into()),
            false,
            "fresh",
            None,
        );
        let m1 = PreparedContextManifest::new("req-1", run_id, 1, 1, vec![entry.clone()], 0);
        let m2 = PreparedContextManifest::new("req-1", run_id, 1, 1, vec![entry], 0);
        assert_eq!(m1.manifest_digest, m2.manifest_digest);
    }
}
