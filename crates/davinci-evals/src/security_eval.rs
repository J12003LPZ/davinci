//! Security evaluation metrics over independently adjudicated causal identities.
//! Fixtures exercise scoring only; model detection quality requires held-out runs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod behavior;
pub mod corpus;
pub mod held_out;
pub mod runner;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CausalIdentity {
    pub control: String,
    pub trust_boundary: String,
    pub sink: String,
    pub occurrence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdjudicatedFinding {
    /// Assigned by an independent evaluator, never inferred from a title match.
    pub identity: CausalIdentity,
    pub confirmed: bool,
    pub evidence_valid: bool,
    #[serde(default)]
    pub severity: Option<corpus::Severity>,
    pub gates_satisfied: bool,
}

/// Confirmed-only metrics. This helper has no live-run, corpus or release evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScore {
    pub confirmed_findings: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub duplicates: usize,
    pub invalid_evidence: usize,
    pub precision: Option<f64>,
    /// Confirmed-only recall; unresolved and likely results do not increase it.
    pub recall: Option<f64>,
    pub duplicate_rate: Option<f64>,
    pub coverage_complete: bool,
    /// Descriptive thresholds only; never a production or model-quality gate.
    pub confirmed_thresholds_met: bool,
}

pub fn score(
    expected: &[CausalIdentity],
    findings: &[AdjudicatedFinding],
    coverage_complete: bool,
) -> SecurityScore {
    let expected: BTreeSet<_> = expected.iter().cloned().collect();
    let mut observed = BTreeSet::new();
    let mut matched = BTreeSet::new();
    let mut false_positives = 0;
    let mut duplicates = 0;
    let mut invalid_evidence = 0;
    for finding in findings.iter().filter(|finding| finding.confirmed) {
        if !observed.insert(finding.identity.clone()) {
            duplicates += 1;
        }
        if !finding.evidence_valid || !finding.gates_satisfied {
            invalid_evidence += 1;
            false_positives += 1;
        } else if !expected.contains(&finding.identity) || !matched.insert(finding.identity.clone())
        {
            // Each redundant report remains in the precision denominator. A valid
            // occurrence can still be detected after an invalid duplicate; order
            // does not erase either its detection or the invalid evidence.
            false_positives += 1;
        }
    }
    let true_positives = matched.len();
    let false_negatives = expected.len() - true_positives;
    let precision = (true_positives + false_positives > 0)
        .then(|| true_positives as f64 / (true_positives + false_positives) as f64);
    let recall = (!expected.is_empty()).then(|| true_positives as f64 / expected.len() as f64);
    let confirmed_findings = true_positives + false_positives;
    let duplicate_rate =
        (confirmed_findings > 0).then(|| duplicates as f64 / confirmed_findings as f64);
    SecurityScore {
        confirmed_findings,
        true_positives,
        false_positives,
        false_negatives,
        duplicates,
        invalid_evidence,
        precision,
        recall,
        duplicate_rate,
        coverage_complete,
        confirmed_thresholds_met: coverage_complete
            && invalid_evidence == 0
            && duplicate_rate.is_some_and(|value| value <= 0.05)
            && precision.is_some_and(|value| value >= 0.9)
            && recall.is_some_and(|value| value >= 0.8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(occurrence: &str) -> CausalIdentity {
        CausalIdentity {
            control: "owner predicate".into(),
            trust_boundary: "cross-account".into(),
            sink: "document lookup".into(),
            occurrence: occurrence.into(),
        }
    }
    #[test]
    fn security_scoring_preserves_occurrences_and_counts_secure_false_positives() {
        let expected = vec![identity("download"), identity("preview")];
        let finding = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: true,
            evidence_valid: true,
            severity: None,
            gates_satisfied: true,
        };
        let result = score(&expected, &[finding.clone(), finding.clone()], true);
        assert_eq!(
            (
                result.true_positives,
                result.false_negatives,
                result.duplicates
            ),
            (1, 1, 1)
        );
        assert!(!result.confirmed_thresholds_met);
        assert_eq!(score(&[], &[finding], true).false_positives, 1);
    }
    #[test]
    fn security_scoring_missing_evidence_and_empty_suite_cannot_pass() {
        assert!(!score(&[], &[], true).confirmed_thresholds_met);
        let finding = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: true,
            evidence_valid: false,
            severity: None,
            gates_satisfied: true,
        };
        let result = score(&[identity("download")], &[finding], true);
        assert_eq!(result.invalid_evidence, 1);
        assert_eq!(result.false_negatives, 1);
        assert!(!result.confirmed_thresholds_met);
    }

    #[test]
    fn security_eval_fixture_metrics_cannot_claim_release_readiness() {
        let finding = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: true,
            evidence_valid: true,
            severity: None,
            gates_satisfied: true,
        };
        let result =
            serde_json::to_value(score(&[identity("download")], &[finding], true)).unwrap();
        assert!(
            result.get("releaseGatePassed").is_none(),
            "a metric-only helper has no provenance, corpus or platform release evidence"
        );
        assert_eq!(result["confirmedThresholdsMet"], true);
    }

    #[test]
    fn security_eval_invalid_duplicates_cannot_hide_evidence_or_depend_on_order() {
        let valid = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: true,
            evidence_valid: true,
            severity: None,
            gates_satisfied: true,
        };
        let invalid = AdjudicatedFinding {
            evidence_valid: false,
            ..valid.clone()
        };
        let forward = score(
            &[identity("download")],
            &[valid.clone(), invalid.clone()],
            true,
        );
        let reverse = score(&[identity("download")], &[invalid, valid], true);
        assert_eq!(
            (
                forward.true_positives,
                forward.false_positives,
                forward.invalid_evidence,
                forward.duplicates
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            serde_json::to_value(forward).unwrap(),
            serde_json::to_value(reverse).unwrap()
        );
    }

    #[test]
    fn security_eval_duplicate_threshold_uses_all_reported_confirmed_findings() {
        for (count, passes) in [(18, false), (19, true)] {
            let expected: Vec<_> = (0..count)
                .map(|index| identity(&format!("entry-{index}")))
                .collect();
            let mut findings: Vec<_> = expected
                .iter()
                .map(|identity| AdjudicatedFinding {
                    identity: identity.clone(),
                    confirmed: true,
                    evidence_valid: true,
                    severity: None,
                    gates_satisfied: true,
                })
                .collect();
            findings.push(findings[0].clone());
            let result = score(&expected, &findings, true);
            assert_eq!(result.confirmed_findings, count + 1);
            assert_eq!(result.false_positives, 1);
            assert_eq!(result.duplicate_rate, Some(1.0 / (count + 1) as f64));
            assert_eq!(result.confirmed_thresholds_met, passes);
        }
    }

    #[test]
    fn security_eval_unresolved_positive_is_missed_and_zero_denominators_are_undefined() {
        let unresolved = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: false,
            evidence_valid: true,
            severity: None,
            gates_satisfied: false,
        };
        let result = score(&[identity("download")], &[unresolved], false);
        assert_eq!(result.false_negatives, 1);
        assert_eq!(result.recall, Some(0.0));
        assert_eq!(result.precision, None);
        assert_eq!(result.duplicate_rate, None);
        assert!(!result.confirmed_thresholds_met);
        let empty = score(&[], &[], true);
        assert_eq!(
            (empty.precision, empty.recall, empty.duplicate_rate),
            (None, None, None)
        );
    }

    #[test]
    fn security_eval_reports_zero_denominator_as_undefined() {
        let empty = score(&[], &[], true);
        assert_eq!(empty.precision, None);
        assert_eq!(empty.recall, None);
        assert_eq!(empty.duplicate_rate, None);
        assert!(!empty.confirmed_thresholds_met);
        let unresolved = AdjudicatedFinding {
            identity: identity("download"),
            confirmed: false,
            evidence_valid: true,
            severity: None,
            gates_satisfied: false,
        };
        let missed = score(&[identity("download")], &[unresolved], false);
        assert_eq!(missed.false_negatives, 1);
        assert_eq!(missed.recall, Some(0.0));
        assert_eq!(missed.precision, None);
    }
}
