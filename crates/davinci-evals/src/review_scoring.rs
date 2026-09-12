//! Deterministic scoring and consolidation for native review findings.

use davinci_agent::prompt::capabilities::{ReviewFinding, ReviewSeverity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewMetrics {
    pub accepted_findings: usize,
    pub duplicate_findings: usize,
    pub blocker_or_major_findings: usize,
    pub critical_recall: f64,
    pub precision: f64,
    pub false_positive_rate: f64,
}

fn severity_rank(severity: ReviewSeverity) -> u8 {
    match severity {
        ReviewSeverity::Blocker => 3,
        ReviewSeverity::Major => 2,
        ReviewSeverity::Minor => 1,
    }
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_path(path: &str) -> String {
    normalized(&path.replace('\\', "/"))
}

fn line_regions_overlap(left: Option<u32>, right: Option<u32>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn duplicate_of(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    normalized_path(&left.path) == normalized_path(&right.path)
        && line_regions_overlap(left.line, right.line)
        && normalized(&left.title) == normalized(&right.title)
}

fn stronger(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    (severity_rank(left.severity), left.confidence)
        > (severity_rank(right.severity), right.confidence)
}

/// Keep only findings that satisfy the confidence threshold, then consolidate
/// semantically identical findings from multiple review lenses.
pub fn consolidate_review_findings(findings: &[ReviewFinding]) -> Vec<ReviewFinding> {
    let mut accepted: Vec<ReviewFinding> = Vec::new();
    for finding in findings.iter().filter(|finding| {
        finding.confidence <= 100
            && match finding.severity {
                ReviewSeverity::Blocker | ReviewSeverity::Major => finding.confidence >= 75,
                ReviewSeverity::Minor => finding.confidence >= 90,
            }
    }) {
        if let Some(existing) = accepted
            .iter_mut()
            .find(|existing| duplicate_of(existing, finding))
        {
            if stronger(finding, existing) {
                *existing = finding.clone();
            }
        } else {
            accepted.push(finding.clone());
        }
    }
    accepted
}

/// Score findings against planted critical titles and harmless decoys.
pub fn score_review_findings(
    findings: &[ReviewFinding],
    planted_critical_titles: &[String],
    harmless_decoy_titles: &[String],
) -> ReviewMetrics {
    let accepted = consolidate_review_findings(findings);
    let accepted_titles: BTreeSet<String> = accepted.iter().map(|f| normalized(&f.title)).collect();
    let planted: BTreeSet<String> = planted_critical_titles
        .iter()
        .map(|title| normalized(title))
        .collect();
    let decoys: BTreeSet<String> = harmless_decoy_titles
        .iter()
        .map(|title| normalized(title))
        .collect();
    let found_critical = planted.intersection(&accepted_titles).count();
    let false_positives = accepted_titles.intersection(&decoys).count();
    let accepted_count = accepted.len();
    let critical_count = planted.len();

    ReviewMetrics {
        accepted_findings: accepted_count,
        duplicate_findings: findings.len().saturating_sub(accepted_count),
        blocker_or_major_findings: accepted
            .iter()
            .filter(|finding| {
                matches!(
                    finding.severity,
                    ReviewSeverity::Blocker | ReviewSeverity::Major
                )
            })
            .count(),
        critical_recall: if critical_count == 0 {
            1.0
        } else {
            found_critical as f64 / critical_count as f64
        },
        precision: if accepted_count == 0 {
            1.0
        } else {
            (accepted_count.saturating_sub(false_positives)) as f64 / accepted_count as f64
        },
        false_positive_rate: if decoys.is_empty() {
            0.0
        } else {
            false_positives as f64 / decoys.len() as f64
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::prompt::capabilities::ReviewLens;

    fn finding(
        lens: ReviewLens,
        severity: ReviewSeverity,
        confidence: u8,
        path: &str,
        line: Option<u32>,
        title: &str,
    ) -> ReviewFinding {
        ReviewFinding {
            lens,
            severity,
            confidence,
            path: path.into(),
            line,
            title: title.into(),
            evidence: "evidence".into(),
            recommendation: "recommendation".into(),
        }
    }

    #[test]
    fn thresholds_and_deduplication_keep_strongest_finding() {
        let findings = vec![
            finding(
                ReviewLens::Correctness,
                ReviewSeverity::Major,
                80,
                "src\\lib.rs",
                Some(10),
                "Unchecked result",
            ),
            finding(
                ReviewLens::Security,
                ReviewSeverity::Blocker,
                76,
                "src/lib.rs",
                Some(10),
                "unchecked-result!",
            ),
            finding(
                ReviewLens::Tests,
                ReviewSeverity::Minor,
                89,
                "src/lib.rs",
                Some(10),
                "too weak",
            ),
        ];
        let accepted = consolidate_review_findings(&findings);
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].severity, ReviewSeverity::Blocker);
        assert_eq!(accepted[0].lens, ReviewLens::Security);
    }

    #[test]
    fn different_lines_are_not_deduplicated() {
        let findings = vec![
            finding(
                ReviewLens::Correctness,
                ReviewSeverity::Major,
                80,
                "src/lib.rs",
                Some(10),
                "Unchecked result",
            ),
            finding(
                ReviewLens::Correctness,
                ReviewSeverity::Major,
                80,
                "src/lib.rs",
                Some(11),
                "Unchecked result",
            ),
        ];
        assert_eq!(consolidate_review_findings(&findings).len(), 2);
    }

    #[test]
    fn planted_defect_metrics_include_decoy_rate() {
        let findings = vec![
            finding(
                ReviewLens::Correctness,
                ReviewSeverity::Major,
                90,
                "src/lib.rs",
                Some(10),
                "Unchecked result",
            ),
            finding(
                ReviewLens::CommentsAndDocs,
                ReviewSeverity::Minor,
                95,
                "README.md",
                None,
                "Style preference",
            ),
        ];
        let metrics = score_review_findings(
            &findings,
            &["unchecked result".into()],
            &["style preference".into()],
        );
        assert_eq!(metrics.critical_recall, 1.0);
        assert_eq!(metrics.precision, 0.5);
        assert_eq!(metrics.false_positive_rate, 1.0);
    }

    #[test]
    fn review_finding_schema_roundtrips() {
        let original = finding(
            ReviewLens::Security,
            ReviewSeverity::Blocker,
            99,
            "src/lib.rs",
            Some(4),
            "Unsafe input",
        );
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: ReviewFinding = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn committed_planted_fixture_exercises_recall_and_decoys() {
        #[derive(Deserialize)]
        struct Fixture {
            findings: Vec<ReviewFinding>,
            planted_critical_titles: Vec<String>,
            harmless_decoy_titles: Vec<String>,
        }

        let fixture: Fixture =
            serde_json::from_str(include_str!("../fixtures/review/planted-findings.json")).unwrap();
        let metrics = score_review_findings(
            &fixture.findings,
            &fixture.planted_critical_titles,
            &fixture.harmless_decoy_titles,
        );
        assert_eq!(metrics.duplicate_findings, 1);
        assert_eq!(metrics.critical_recall, 1.0);
        assert_eq!(metrics.false_positive_rate, 1.0);
    }
}
