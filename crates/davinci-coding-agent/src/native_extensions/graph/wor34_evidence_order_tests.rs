//! WOR-34: the research digest must not depend on worker completion order.

use super::*;
use crate::native_extensions::graph::types::*;

fn evidence(kind: ResearchKind, claim: &str, baseline: &str) -> EvidenceArtifact {
    EvidenceArtifact {
        kind,
        findings: vec![EvidenceFinding {
            claim: claim.into(),
            refs: vec![format!("{claim}.rs:1")],
            confidence: Confidence::High,
        }],
        risks: vec![format!("risk from {claim}")],
        gaps: vec![],
        test_baseline: Some(TestBaseline {
            command: baseline.into(),
            exit_code: 0,
            summary: format!("summary {baseline}"),
        }),
    }
}

fn digest_for_completion_order(order: &[usize]) -> String {
    let requests = [
        evidence(ResearchKind::CodeSearch, "shared claim", "cargo test a"),
        evidence(ResearchKind::History, "shared claim", "cargo test b"),
        evidence(ResearchKind::CodeSearch, "third claim", "cargo test c"),
    ];
    let completed: Vec<(usize, EvidenceArtifact)> = order
        .iter()
        .map(|&index| (index, requests[index].clone()))
        .collect();
    build_evidence_digest(&evidence_in_request_order(completed), &[], 10_000)
}

#[test]
fn digest_is_identical_for_every_completion_order() {
    let expected = digest_for_completion_order(&[0, 1, 2]);
    for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        assert_eq!(
            digest_for_completion_order(&order),
            expected,
            "completion order {order:?} changed the digest"
        );
    }
    // First request wins the baseline, and the digest is not empty.
    assert!(expected.contains("cargo test a"));
}
