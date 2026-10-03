//! Labeled memory-retrieval evaluation used by harness optimization gates.
//!
//! The evaluator is intentionally independent of the production retriever. A
//! baseline and candidate can be scored from the same immutable snapshot
//! without trusting model claims about which memory was useful.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalCaseKind {
    ExactIdentifier,
    Semantic,
    Stale,
    Contradictory,
    Adversarial,
    CrossScope,
    NoAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievalLabel {
    pub memory_id: String,
    pub required_provenance: String,
    pub useful_spans: Vec<String>,
    pub mandatory: bool,
}

impl RetrievalLabel {
    pub fn mandatory(
        memory_id: impl Into<String>,
        required_provenance: impl Into<String>,
        useful_spans: Vec<String>,
    ) -> Self {
        Self {
            memory_id: memory_id.into(),
            required_provenance: required_provenance.into(),
            useful_spans,
            mandatory: true,
        }
    }

    pub fn optional(
        memory_id: impl Into<String>,
        required_provenance: impl Into<String>,
        useful_spans: Vec<String>,
    ) -> Self {
        Self {
            memory_id: memory_id.into(),
            required_provenance: required_provenance.into(),
            useful_spans,
            mandatory: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabeledRetrievalCase {
    pub name: String,
    pub kind: RetrievalCaseKind,
    pub snapshot_id: String,
    pub labels: Vec<RetrievalLabel>,
}

impl LabeledRetrievalCase {
    pub fn new(
        name: impl Into<String>,
        kind: RetrievalCaseKind,
        snapshot_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            snapshot_id: snapshot_id.into(),
            labels: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: RetrievalLabel) -> Self {
        self.labels.push(label);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievedMemory {
    pub memory_id: String,
    pub text: String,
    pub provenance: String,
    pub rendered_tokens: u64,
    pub cited: bool,
}

impl RetrievedMemory {
    pub fn new(
        memory_id: impl Into<String>,
        text: impl Into<String>,
        provenance: impl Into<String>,
        rendered_tokens: u64,
        cited: bool,
    ) -> Self {
        Self {
            memory_id: memory_id.into(),
            text: text.into(),
            provenance: provenance.into(),
            rendered_tokens,
            cited,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievalRun {
    pub snapshot_id: String,
    pub items: Vec<RetrievedMemory>,
}

impl RetrievalRun {
    pub fn new(snapshot_id: impl Into<String>, items: Vec<RetrievedMemory>) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            items,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalMetrics {
    pub relevant_retrieved: usize,
    pub retrieved: usize,
    pub relevant_total: usize,
    pub precision: f64,
    pub recall: f64,
    pub useful_tokens: u64,
    pub irrelevant_tokens: u64,
    pub missed_mandatory_evidence: usize,
    pub false_memory_citations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalComparison {
    pub snapshot_id: String,
    pub baseline: RetrievalMetrics,
    pub candidate: RetrievalMetrics,
}

fn item_satisfies_label(item: &RetrievedMemory, label: &RetrievalLabel) -> bool {
    if item.memory_id != label.memory_id || item.provenance != label.required_provenance {
        return false;
    }
    label.useful_spans.is_empty()
        || label
            .useful_spans
            .iter()
            .any(|span| !span.is_empty() && item.text.contains(span))
}

pub fn evaluate_retrieval(
    case: &LabeledRetrievalCase,
    run: &RetrievalRun,
) -> Result<RetrievalMetrics, String> {
    if case.snapshot_id != run.snapshot_id {
        return Err(format!(
            "retrieval snapshot mismatch: case={} run={}",
            case.snapshot_id, run.snapshot_id
        ));
    }

    let labels = case
        .labels
        .iter()
        .map(|label| (label.memory_id.as_str(), label))
        .collect::<HashMap<_, _>>();
    let mut matched_labels = HashSet::new();
    let mut useful_tokens = 0u64;
    let mut irrelevant_tokens = 0u64;
    let mut false_memory_citations = 0usize;
    let mut useful_retrievals = 0usize;

    for item in &run.items {
        let useful = labels
            .get(item.memory_id.as_str())
            .is_some_and(|label| item_satisfies_label(item, label));
        if useful {
            useful_retrievals += 1;
            useful_tokens = useful_tokens.saturating_add(item.rendered_tokens);
            matched_labels.insert(item.memory_id.as_str());
        } else {
            irrelevant_tokens = irrelevant_tokens.saturating_add(item.rendered_tokens);
            if item.cited {
                false_memory_citations += 1;
            }
        }
    }

    let missed_mandatory_evidence = case
        .labels
        .iter()
        .filter(|label| label.mandatory && !matched_labels.contains(label.memory_id.as_str()))
        .count();
    let retrieved = run.items.len();
    let relevant_total = case.labels.len();
    let precision = if retrieved == 0 {
        if relevant_total == 0 {
            1.0
        } else {
            0.0
        }
    } else {
        useful_retrievals as f64 / retrieved as f64
    };
    let recall = if relevant_total == 0 {
        if useful_retrievals == 0 {
            1.0
        } else {
            0.0
        }
    } else {
        matched_labels.len() as f64 / relevant_total as f64
    };

    Ok(RetrievalMetrics {
        relevant_retrieved: matched_labels.len(),
        retrieved,
        relevant_total,
        precision,
        recall,
        useful_tokens,
        irrelevant_tokens,
        missed_mandatory_evidence,
        false_memory_citations,
    })
}

pub fn compare_retrieval(
    case: &LabeledRetrievalCase,
    baseline: &RetrievalRun,
    candidate: &RetrievalRun,
) -> Result<RetrievalComparison, String> {
    let baseline_metrics = evaluate_retrieval(case, baseline)?;
    let candidate_metrics = evaluate_retrieval(case, candidate)?;
    Ok(RetrievalComparison {
        snapshot_id: case.snapshot_id.clone(),
        baseline: baseline_metrics,
        candidate: candidate_metrics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_metrics_expose_irrelevant_tokens_and_missed_evidence() {
        let case =
            LabeledRetrievalCase::new("auth", RetrievalCaseKind::ExactIdentifier, "snapshot-a")
                .with_label(RetrievalLabel::mandatory(
                    "good",
                    "repository_fact",
                    vec!["AUTH_V2".into()],
                ));
        let run = RetrievalRun::new(
            "snapshot-a",
            vec![
                RetrievedMemory::new("good", "use AUTH_V2", "repository_fact", 8, true),
                RetrievedMemory::new(
                    "noise",
                    "irrelevant prose",
                    "assistant_inference",
                    20,
                    false,
                ),
            ],
        );
        let metrics = evaluate_retrieval(&case, &run).unwrap();
        assert_eq!(metrics.relevant_retrieved, 1);
        assert_eq!(metrics.irrelevant_tokens, 20);
        assert_eq!(metrics.missed_mandatory_evidence, 0);
        assert!((metrics.precision - 0.5).abs() < f64::EPSILON);
        assert!((metrics.recall - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn repeated_cited_false_memory_never_counts_as_useful() {
        let case = LabeledRetrievalCase::new("none", RetrievalCaseKind::NoAnswer, "snapshot-a");
        let run = RetrievalRun::new(
            "snapshot-a",
            vec![
                RetrievedMemory::new("false", "invented fact", "assistant_inference", 7, true),
                RetrievedMemory::new("false", "invented fact", "assistant_inference", 7, true),
            ],
        );
        let metrics = evaluate_retrieval(&case, &run).unwrap();
        assert_eq!(metrics.relevant_retrieved, 0);
        assert_eq!(metrics.false_memory_citations, 2);
        assert_eq!(metrics.irrelevant_tokens, 14);
        assert_eq!(metrics.precision, 0.0);
    }

    #[test]
    fn mandatory_miss_is_reported_even_when_other_memory_is_relevant() {
        let case = LabeledRetrievalCase::new("stale", RetrievalCaseKind::Stale, "snapshot-a")
            .with_label(RetrievalLabel::mandatory("must", "repository_fact", vec![]))
            .with_label(RetrievalLabel::optional("nice", "repository_fact", vec![]));
        let run = RetrievalRun::new(
            "snapshot-a",
            vec![RetrievedMemory::new(
                "nice",
                "secondary",
                "repository_fact",
                4,
                false,
            )],
        );
        let metrics = evaluate_retrieval(&case, &run).unwrap();
        assert_eq!(metrics.missed_mandatory_evidence, 1);
        assert_eq!(metrics.relevant_retrieved, 1);
    }

    #[test]
    fn old_and_new_retrieval_are_compared_from_same_snapshot() {
        let case = LabeledRetrievalCase::new("semantic", RetrievalCaseKind::Semantic, "snapshot-a")
            .with_label(RetrievalLabel::optional("m1", "repository_fact", vec![]));
        let baseline = RetrievalRun::new("snapshot-a", vec![]);
        let candidate = RetrievalRun::new(
            "snapshot-a",
            vec![RetrievedMemory::new(
                "m1",
                "useful",
                "repository_fact",
                5,
                false,
            )],
        );
        let comparison = compare_retrieval(&case, &baseline, &candidate).unwrap();
        assert!(comparison.candidate.recall > comparison.baseline.recall);

        let wrong_snapshot = RetrievalRun::new("snapshot-b", vec![]);
        assert!(compare_retrieval(&case, &baseline, &wrong_snapshot).is_err());
    }
}
