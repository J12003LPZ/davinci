#!/usr/bin/env python3
from pathlib import Path
import argparse

ROOT = Path(__file__).resolve().parents[2]
VECTOR = ROOT / "crates/davinci-coding-agent/src/native_extensions/vector_memory.rs"
EVAL_LIB = ROOT / "crates/davinci-evals/src/lib.rs"
EVAL_MOD = ROOT / "crates/davinci-evals/src/harness_eval.rs"

TEST_MODULE = r'''

#[cfg(test)]
mod phase3_memory_retrieval_regressions {
    use super::*;

    fn record(memory: &VectorMemory, id: &str, text: String, importance: f32) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            repo_id: memory.repo_id.clone(),
            kind: MemoryKind::Discovery,
            text: text.clone(),
            source: "repository_fact".into(),
            content_hash: content_hash(&text),
            importance,
            created_at: 1000,
            embedding: None,
            confidence: Some(0.9),
            source_session_id: Some("session-phase3".into()),
            source_turn: Some(1),
            verification: Some("verified".into()),
            source_paths: Vec::new(),
            source_state_hash: None,
            verified_at_revision: None,
            use_count: 0,
            last_used_at: None,
            agent_profile_name: None,
            memory_scope: None,
        }
    }

    #[test]
    fn phase3_candidate_limit_is_a_hard_search_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let mut memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                candidate_limit: 2,
                minimum_score: 0.1,
                promotion: false,
                ..VectorMemoryConfig::default()
            },
        );
        memory.mark_dense_offline();
        for index in 0..8 {
            memory.records.push(record(
                &memory,
                &format!("candidate-{index}"),
                format!("shared authentication candidate {index}"),
                0.8,
            ));
        }

        let hits = memory.search("shared authentication", 20);
        assert!(hits.len() <= 2, "candidate_limit must cap final retrieval work");
    }

    #[test]
    fn phase3_oversized_first_hit_does_not_starve_smaller_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let mut memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                candidate_limit: 8,
                minimum_score: 0.1,
                promotion: false,
                ..VectorMemoryConfig::default()
            },
        );
        memory.mark_dense_offline();
        memory.records.push(record(
            &memory,
            "large",
            format!("authentication token {}", "x".repeat(2000)),
            1.0,
        ));
        memory.records.push(record(
            &memory,
            "small",
            "authentication token uses repository setting AUTH_V2".into(),
            0.8,
        ));

        let hits = memory.context_hits("authentication token", 4, 20);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "small");
    }

    #[test]
    fn phase3_bounded_indexing_makes_incremental_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                max_index_chunks: 2,
                promotion: false,
                ..VectorMemoryConfig::default()
            },
        );
        memory.mark_dense_offline();
        let messages = (0..6)
            .map(|index| MemoryMessage {
                role: "user".into(),
                content: format!("unique indexing fact number {index}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(memory.index_messages(&messages).unwrap(), 2);
        assert_eq!(memory.index_messages(&messages).unwrap(), 2);
        assert_eq!(memory.record_count(), 4);
    }

    #[test]
    fn phase3_exact_identifier_survives_small_candidate_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                candidate_limit: 1,
                minimum_score: 0.1,
                promotion: false,
                ..VectorMemoryConfig::default()
            },
        );
        memory.mark_dense_offline();
        memory.records.push(record(
            &memory,
            "noise",
            "BUG-1000 authentication authentication authentication".into(),
            1.0,
        ));
        memory.records.push(record(
            &memory,
            "target",
            "Regression BUG-8472 is fixed by rotating the cache key".into(),
            0.4,
        ));

        let hits = memory.search("BUG-8472", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.id, "target");
    }
}
'''

EVAL_TEST_ONLY = r'''//! Labeled harness retrieval evaluation contracts.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_metrics_expose_irrelevant_tokens_and_missed_evidence() {
        let case = LabeledRetrievalCase::new("auth", RetrievalCaseKind::ExactIdentifier, "snapshot-a")
            .with_label(RetrievalLabel::mandatory(
                "good",
                "repository_fact",
                vec!["AUTH_V2".into()],
            ));
        let run = RetrievalRun::new(
            "snapshot-a",
            vec![
                RetrievedMemory::new("good", "use AUTH_V2", "repository_fact", 8, true),
                RetrievedMemory::new("noise", "irrelevant prose", "assistant_inference", 20, false),
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
    fn old_and_new_retrieval_are_compared_from_same_snapshot() {
        let case = LabeledRetrievalCase::new("semantic", RetrievalCaseKind::Semantic, "snapshot-a")
            .with_label(RetrievalLabel::optional("m1", "repository_fact", vec![]));
        let baseline = RetrievalRun::new("snapshot-a", vec![]);
        let candidate = RetrievalRun::new(
            "snapshot-a",
            vec![RetrievedMemory::new("m1", "useful", "repository_fact", 5, false)],
        );
        let comparison = compare_retrieval(&case, &baseline, &candidate).unwrap();
        assert!(comparison.candidate.recall > comparison.baseline.recall);

        let wrong_snapshot = RetrievalRun::new("snapshot-b", vec![]);
        assert!(compare_retrieval(&case, &baseline, &wrong_snapshot).is_err());
    }
}
'''

EVAL_IMPLEMENTATION = r'''//! Labeled memory-retrieval evaluation used by harness optimization gates.
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
        if relevant_total == 0 { 1.0 } else { 0.0 }
    } else {
        useful_retrievals as f64 / retrieved as f64
    };
    let recall = if relevant_total == 0 {
        if useful_retrievals == 0 { 1.0 } else { 0.0 }
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
        let case = LabeledRetrievalCase::new("auth", RetrievalCaseKind::ExactIdentifier, "snapshot-a")
            .with_label(RetrievalLabel::mandatory(
                "good",
                "repository_fact",
                vec!["AUTH_V2".into()],
            ));
        let run = RetrievalRun::new(
            "snapshot-a",
            vec![
                RetrievedMemory::new("good", "use AUTH_V2", "repository_fact", 8, true),
                RetrievedMemory::new("noise", "irrelevant prose", "assistant_inference", 20, false),
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
            vec![RetrievedMemory::new("nice", "secondary", "repository_fact", 4, false)],
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
            vec![RetrievedMemory::new("m1", "useful", "repository_fact", 5, false)],
        );
        let comparison = compare_retrieval(&case, &baseline, &candidate).unwrap();
        assert!(comparison.candidate.recall > comparison.baseline.recall);

        let wrong_snapshot = RetrievalRun::new("snapshot-b", vec![]);
        assert!(compare_retrieval(&case, &baseline, &wrong_snapshot).is_err());
    }
}
'''


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return text.replace(old, new, 1)


def add_tests() -> None:
    lib = EVAL_LIB.read_text()
    if "pub mod harness_eval;" not in lib:
        lib = replace_once(lib, "pub mod harness_table;\n", "pub mod harness_table;\npub mod harness_eval;\n", "eval module export")
        EVAL_LIB.write_text(lib)

    EVAL_MOD.write_text(EVAL_TEST_ONLY)

    vector = VECTOR.read_text()
    if "mod phase3_memory_retrieval_regressions" not in vector:
        VECTOR.write_text(vector + TEST_MODULE)


def implement_eval() -> None:
    EVAL_MOD.write_text(EVAL_IMPLEMENTATION)


def implement_vector() -> None:
    text = VECTOR.read_text()

    # Add a bounded indexing setting. It caps durable/embedding work per settled
    # pass without losing progress: known chunks are skipped on the next pass.
    old = '''    #[serde(default = "default_candidate_limit")]\n    pub candidate_limit: usize,\n    #[serde(default = "default_max_injected_tokens")]\n'''
    new = '''    #[serde(default = "default_candidate_limit")]\n    pub candidate_limit: usize,\n    #[serde(default = "default_max_index_chunks")]\n    pub max_index_chunks: usize,\n    #[serde(default = "default_max_injected_tokens")]\n'''
    text = replace_once(text, old, new, "config max_index_chunks field")

    old = '''fn default_candidate_limit() -> usize {\n    30\n}\nfn default_max_injected_tokens() -> usize {\n'''
    new = '''fn default_candidate_limit() -> usize {\n    30\n}\nfn default_max_index_chunks() -> usize {\n    64\n}\nfn default_max_injected_tokens() -> usize {\n'''
    text = replace_once(text, old, new, "default max index chunks")

    old = '''            result_limit: default_result_limit(),\n            candidate_limit: default_candidate_limit(),\n            max_injected_tokens: default_max_injected_tokens(),\n'''
    new = '''            result_limit: default_result_limit(),\n            candidate_limit: default_candidate_limit(),\n            max_index_chunks: default_max_index_chunks(),\n            max_injected_tokens: default_max_injected_tokens(),\n'''
    text = replace_once(text, old, new, "config default max index chunks")

    old = '''        if let Some(value) = env_usize("PI_MEMORY_CANDIDATE_LIMIT") {\n            config.candidate_limit = value.clamp(1, 100);\n        }\n        if let Some(value) = env_usize("PI_MEMORY_MAX_INJECTED_TOKENS") {\n'''
    new = '''        if let Some(value) = env_usize("PI_MEMORY_CANDIDATE_LIMIT") {\n            config.candidate_limit = value.clamp(1, 100);\n        }\n        if let Some(value) = env_usize("PI_MEMORY_MAX_INDEX_CHUNKS") {\n            config.max_index_chunks = value.clamp(1, 512);\n        }\n        if let Some(value) = env_usize("PI_MEMORY_MAX_INJECTED_TOKENS") {\n'''
    text = replace_once(text, old, new, "config env max index chunks")

    old = '''            "resultLimit": self.config.result_limit,\n            "candidateLimit": self.config.candidate_limit,\n            "maxInjectedTokens": self.config.max_injected_tokens,\n'''
    new = '''            "resultLimit": self.config.result_limit,\n            "candidateLimit": self.config.candidate_limit,\n            "maxIndexChunks": self.config.max_index_chunks,\n            "maxInjectedTokens": self.config.max_injected_tokens,\n'''
    text = replace_once(text, old, new, "status max index chunks")

    old = '''        for chunk in chunks {\n            let hash = content_hash(&chunk.text);\n            let tag = known_key_scoped(chunk.kind, &hash, agent_profile_name);\n            if !self.known.insert(tag) {\n                continue;\n            }\n            let id_seed = format!(\n'''
    new = '''        for chunk in chunks {\n            let hash = content_hash(&chunk.text);\n            let tag = known_key_scoped(chunk.kind, &hash, agent_profile_name);\n            if self.known.contains(&tag) {\n                continue;\n            }\n            if inserted >= self.config.max_index_chunks.max(1) {\n                break;\n            }\n            self.known.insert(tag);\n            let id_seed = format!(\n'''
    text = replace_once(text, old, new, "bounded new chunk insertion")

    # Replace the unbounded scoring path with two bounded rank lists. Records
    # stay borrowed until final selection, avoiding full record/vector clones.
    old = '''        let candidates: Vec<&MemoryRecord> = self\n            .records\n            .iter()\n            .filter(|record| {\n                if self.tombstones.contains(&record.id)\n                    || self.supersessions.contains_key(&record.id)\n                {\n                    return false;\n                }\n                match memory_scope {\n                    Some("none") => false,\n                    Some("agent_global") => {\n                        record.agent_profile_name.as_deref() == agent_profile_name\n                    }\n                    Some("agent_project") => {\n                        (record.repo_id == self.repo_id || record.repo_id == "*")\n                            && record.agent_profile_name.as_deref() == agent_profile_name\n                    }\n                    Some("project") => {\n                        record.repo_id == self.repo_id\n                            && (record.agent_profile_name.is_none()\n                                || record.agent_profile_name.as_deref() == agent_profile_name)\n                    }\n                    _ => {\n                        if let Some(profile) = agent_profile_name {\n                            (record.repo_id == self.repo_id || record.repo_id == "*")\n                                && (record.agent_profile_name.is_none()\n                                    || record.agent_profile_name.as_deref() == Some(profile))\n                        } else {\n                            // Main agent behavior remains unchanged: only project-wide memory where agent_profile_name is None\n                            record.repo_id == self.repo_id && record.agent_profile_name.is_none()\n                        }\n                    }\n                }\n            })\n            .collect();\n\n        if candidates.is_empty() {\n            return Vec::new();\n        }\n\n        let query_embedding = (self.dense_available()\n            && candidates.iter().any(|record| record.embedding.is_some()))\n        .then(|| match self.embed_query(query) {\n            Ok(vector) => Some(vector),\n            Err(_) => {\n                self.mark_dense_offline();\n                None\n            }\n        })\n        .flatten();\n\n        let lexical_query = LexicalQuery::new(query);\n        let mut hits = candidates\n            .into_iter()\n            .filter_map(|record| {\n                let lexical = lexical_query.score(&record.text);\n                let dense = query_embedding\n                    .as_ref()\n                    .zip(record.embedding.as_ref())\n                    .map(|(query, embedding)| cosine_similarity(query, embedding))\n                    .unwrap_or(lexical);\n                let base = (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);\n                let score = (base * memory_freshness(record, &self.cwd)).clamp(0.0, 1.0);\n                (score >= self.config.minimum_score).then(|| MemoryHit {\n                    record: (*record).clone(),\n                    score,\n                    dense_score: dense,\n                    lexical_score: lexical,\n                })\n            })\n            .collect::<Vec<_>>();\n        fuse_hits(std::mem::take(&mut hits), limit.max(1))\n'''
    new = '''        let candidates: Vec<&MemoryRecord> = self\n            .records\n            .iter()\n            .filter(|record| {\n                if self.tombstones.contains(&record.id)\n                    || self.supersessions.contains_key(&record.id)\n                {\n                    return false;\n                }\n                match memory_scope {\n                    Some("none") => false,\n                    Some("agent_global") => {\n                        record.agent_profile_name.as_deref() == agent_profile_name\n                    }\n                    Some("agent_project") => {\n                        (record.repo_id == self.repo_id || record.repo_id == "*")\n                            && record.agent_profile_name.as_deref() == agent_profile_name\n                    }\n                    Some("project") => {\n                        record.repo_id == self.repo_id\n                            && (record.agent_profile_name.is_none()\n                                || record.agent_profile_name.as_deref() == agent_profile_name)\n                    }\n                    _ => {\n                        if let Some(profile) = agent_profile_name {\n                            (record.repo_id == self.repo_id || record.repo_id == "*")\n                                && (record.agent_profile_name.is_none()\n                                    || record.agent_profile_name.as_deref() == Some(profile))\n                        } else {\n                            record.repo_id == self.repo_id && record.agent_profile_name.is_none()\n                        }\n                    }\n                }\n            })\n            .collect();\n\n        if candidates.is_empty() {\n            return Vec::new();\n        }\n\n        let candidate_limit = self.config.candidate_limit.max(1);\n        let lexical_query = LexicalQuery::new(query);\n        let mut lexical_ranked = candidates\n            .iter()\n            .enumerate()\n            .filter_map(|(index, record)| {\n                let lexical = lexical_query.score(&record.text);\n                let exact = exact_identifier_match(query, record);\n                (exact || lexical > 0.0).then_some((index, lexical, exact))\n            })\n            .collect::<Vec<_>>();\n        lexical_ranked.sort_by(|left, right| {\n            right\n                .2\n                .cmp(&left.2)\n                .then_with(|| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal))\n                .then_with(|| candidates[left.0].id.cmp(&candidates[right.0].id))\n        });\n        lexical_ranked.truncate(candidate_limit);\n\n        let query_embedding = (self.dense_available()\n            && candidates.iter().any(|record| record.embedding.is_some()))\n        .then(|| match self.embed_query(query) {\n            Ok(vector) => Some(vector),\n            Err(_) => {\n                self.mark_dense_offline();\n                None\n            }\n        })\n        .flatten();\n\n        let mut dense_ranked = query_embedding\n            .as_ref()\n            .map(|query_vector| {\n                let mut ranked = candidates\n                    .iter()\n                    .enumerate()\n                    .filter_map(|(index, record)| {\n                        let embedding = record.embedding.as_ref()?;\n                        if embedding.len() != query_vector.len() {\n                            return None;\n                        }\n                        let score = cosine_similarity(query_vector, embedding);\n                        (score > 0.0).then_some((index, score))\n                    })\n                    .collect::<Vec<_>>();\n                ranked.sort_by(|left, right| {\n                    right\n                        .1\n                        .partial_cmp(&left.1)\n                        .unwrap_or(Ordering::Equal)\n                        .then_with(|| candidates[left.0].id.cmp(&candidates[right.0].id))\n                });\n                ranked.truncate(candidate_limit);\n                ranked\n            })\n            .unwrap_or_default();\n\n        #[derive(Default, Clone, Copy)]\n        struct RankState {\n            lexical_rank: Option<usize>,\n            dense_rank: Option<usize>,\n            lexical_score: f32,\n            dense_score: f32,\n            exact: bool,\n        }\n\n        let mut selected = BTreeMap::<usize, RankState>::new();\n        for (rank, (index, score, exact)) in lexical_ranked.into_iter().enumerate() {\n            let state = selected.entry(index).or_default();\n            state.lexical_rank = Some(rank);\n            state.lexical_score = score;\n            state.exact = exact;\n        }\n        for (rank, (index, score)) in dense_ranked.drain(..).enumerate() {\n            let state = selected.entry(index).or_default();\n            state.dense_rank = Some(rank);\n            state.dense_score = score;\n        }\n\n        let rank_value = |rank: usize| -> f32 {\n            1.0 - (rank.min(candidate_limit - 1) as f32 / candidate_limit as f32)\n        };\n        let mut hits = selected\n            .into_iter()\n            .map(|(index, state)| {\n                let record = candidates[index];\n                let lexical = state.lexical_score;\n                let dense = if query_embedding.is_some() {\n                    state.dense_score\n                } else {\n                    lexical\n                };\n                let mut rank_sum = 0.0f32;\n                let mut rank_count = 0.0f32;\n                if let Some(rank) = state.lexical_rank {\n                    rank_sum += rank_value(rank);\n                    rank_count += 1.0;\n                }\n                if let Some(rank) = state.dense_rank {\n                    rank_sum += rank_value(rank);\n                    rank_count += 1.0;\n                }\n                let rank_fusion = if rank_count > 0.0 {\n                    rank_sum / rank_count\n                } else {\n                    0.0\n                };\n                let mut base = if query_embedding.is_some() {\n                    dense * 0.50 + lexical * 0.25 + rank_fusion * 0.15 + record.importance * 0.10\n                } else {\n                    lexical * 0.65 + rank_fusion * 0.25 + record.importance * 0.10\n                };\n                if state.exact {\n                    base = base.max(0.95);\n                }\n                let score = (base.clamp(0.0, 1.0) * memory_freshness(record, &self.cwd))\n                    .clamp(0.0, 1.0);\n                MemoryHit {\n                    record: (*record).clone(),\n                    score,\n                    dense_score: dense,\n                    lexical_score: lexical,\n                }\n            })\n            .filter(|hit| hit.score >= self.config.minimum_score)\n            .collect::<Vec<_>>();\n        hits.sort_by(|left, right| {\n            right\n                .score\n                .partial_cmp(&left.score)\n                .unwrap_or(Ordering::Equal)\n                .then_with(|| left.record.id.cmp(&right.record.id))\n        });\n        hits.truncate(limit.max(1).min(candidate_limit));\n        hits\n'''
    text = replace_once(text, old, new, "bounded hybrid search")

    # Skip an individually oversized hit instead of terminating packing, and
    # avoid injecting duplicate content under different record identities.
    old = '''        let mut results = Vec::new();\n        let mut accumulated_tokens = 0;\n        for hit in hits {\n            let text = redact_secrets(&hit.record.text);\n            let estimated_tokens = (text.chars().count() + 3) / 4;\n            if accumulated_tokens + estimated_tokens > token_cap {\n                break;\n            }\n            accumulated_tokens += estimated_tokens;\n            results.push(MemoryContextHit {\n'''
    new = '''        let mut results = Vec::new();\n        let mut accumulated_tokens = 0;\n        let mut seen_content = HashSet::new();\n        for hit in hits {\n            if !seen_content.insert(hit.record.content_hash.clone()) {\n                continue;\n            }\n            let text = redact_secrets(&hit.record.text);\n            let estimated_tokens = (text.chars().count() + 3) / 4;\n            if estimated_tokens > token_cap.saturating_sub(accumulated_tokens) {\n                continue;\n            }\n            accumulated_tokens += estimated_tokens;\n            results.push(MemoryContextHit {\n'''
    text = replace_once(text, old, new, "context packing skip oversized")

    # Add exact-identifier matching helper before similarity scoring.
    anchor = '''pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {\n'''
    helper = '''fn exact_identifier_match(query: &str, record: &MemoryRecord) -> bool {\n    let query = query.trim();\n    if query.is_empty() {\n        return false;\n    }\n    if record.id.eq_ignore_ascii_case(query) || record.content_hash.eq_ignore_ascii_case(query) {\n        return true;\n    }\n    let text = record.text.to_ascii_lowercase();\n    query\n        .split_whitespace()\n        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '-' | '_' | ':' | '/' | '.')))\n        .filter(|token| token.len() >= 5)\n        .filter(|token| {\n            token.chars().any(|ch| ch.is_ascii_digit())\n                || token.contains(['-', '_', ':', '/', '.'])\n        })\n        .any(|token| text.contains(&token.to_ascii_lowercase()))\n}\n\n'''
    if helper.strip() not in text:
        text = replace_once(text, anchor, helper + anchor, "exact identifier helper")

    VECTOR.write_text(text)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["tests", "impl"])
    args = parser.parse_args()
    if args.mode == "tests":
        add_tests()
    else:
        implement_eval()
        implement_vector()


if __name__ == "__main__":
    main()
