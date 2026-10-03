//! Deterministic benchmark assessing learning review gating token reduction and artifact retention.

use serde::{Deserialize, Serialize};

use crate::native_extensions::learning::evidence::should_review_evidence;
use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningArtifact, LearningCandidate, LearningEvidence, LearningScope,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningBenchmarkResult {
    pub median_input_tokens: u64,
    pub dispatched_reviews: usize,
    pub accepted_high_confidence_artifacts: usize,
}

#[allow(dead_code)]
fn estimate_turn_tokens(evidence: &LearningEvidence) -> u64 {
    // Standard heuristic: 4 chars per token + fixed reviewer prompt overhead (~400 tokens)
    let raw_len = evidence.serialized_len() as u64;
    400 + (raw_len / 4)
}

/// Simulated canned reviewer output for high-signal turns without model calls.
#[allow(dead_code)]
fn mock_review_turn(evidence: &LearningEvidence) -> Vec<LearningCandidate> {
    if evidence.verification.commands_ran > 0 && evidence.verification.passed {
        vec![LearningCandidate {
            id: format!("cand-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::SkillCreate {
                name: format!("skill-turn-{}", evidence.turn),
                description: "Verified procedure".into(),
                body: "Run commands cleanly".into(),
            },
            confidence: 0.90,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "Deterministic verification passed".into(),
        }]
    } else if evidence.verification.graph_run_id.is_some() {
        vec![LearningCandidate {
            id: format!("cand-graph-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::SkillCreate {
                name: format!("skill-graph-{}", evidence.turn),
                description: "Graph delivered procedure".into(),
                body: "Multi-role graph execution".into(),
            },
            confidence: 0.92,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "Graph outcome recorded".into(),
        }]
    } else if evidence.verification.user_corrected {
        vec![LearningCandidate {
            id: format!("cand-correction-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::FailureLesson {
                text: "Avoid erroneous tool invocation".into(),
                importance: 0.95,
            },
            confidence: 0.95,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "User corrected behavior".into(),
        }]
    } else if evidence
        .messages
        .iter()
        .any(|m| m.content.contains("/learn"))
    {
        vec![LearningCandidate {
            id: format!("cand-learn-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::SkillCreate {
                name: "user-directed-skill".into(),
                description: "Explicitly requested pattern".into(),
                body: "Pattern body".into(),
            },
            confidence: 0.98,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "Explicit /learn instruction".into(),
        }]
    } else if evidence.tools.iter().filter(|t| t.is_error).count() >= 2 {
        vec![LearningCandidate {
            id: format!("cand-failure-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::FailureLesson {
                text: "Repeated tool failure lesson".into(),
                importance: 0.88,
            },
            confidence: 0.88,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "Repeated tool failure".into(),
        }]
    } else if evidence
        .tools
        .iter()
        .any(|t| !t.is_error && t.name.contains("write"))
    {
        vec![LearningCandidate {
            id: format!("cand-write-{}", evidence.turn),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::SkillCreate {
                name: "file-edit-skill".into(),
                description: "Edited code".into(),
                body: "Code modification pattern".into(),
            },
            confidence: 0.87,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: 1000,
            evidence: evidence.verification.clone(),
            rationale: "Files mutated".into(),
        }]
    } else {
        vec![]
    }
}

#[allow(dead_code)]
pub fn run_learning_benchmark(turns: &[LearningEvidence], gated: bool) -> LearningBenchmarkResult {
    let mut token_counts = Vec::with_capacity(turns.len());
    let mut dispatched = 0;
    let mut high_confidence_count = 0;

    for turn in turns {
        if !gated || should_review_evidence(turn) {
            dispatched += 1;
            let tokens = estimate_turn_tokens(turn);
            token_counts.push(tokens);
            let candidates = mock_review_turn(turn);
            for c in candidates {
                if c.confidence >= 0.85 {
                    high_confidence_count += 1;
                }
            }
        } else {
            token_counts.push(0);
        }
    }

    token_counts.sort_unstable();
    let median_tokens = if token_counts.is_empty() {
        0
    } else {
        let mid = token_counts.len() / 2;
        if token_counts.len() % 2 == 0 {
            (token_counts[mid - 1] + token_counts[mid]) / 2
        } else {
            token_counts[mid]
        }
    };

    LearningBenchmarkResult {
        median_input_tokens: median_tokens,
        dispatched_reviews: dispatched,
        accepted_high_confidence_artifacts: high_confidence_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::{ToolEvidence, VerificationEvidence};
    use crate::native_extensions::vector_memory::MemoryMessage;

    pub fn mixed_settled_turn_corpus() -> Vec<LearningEvidence> {
        let read_only = |turn: u64, topic: &str| LearningEvidence {
            session_id: format!("sess-{turn}"),
            repo_id: "repo-benchmark".into(),
            turn,
            messages: vec![
                MemoryMessage {
                    role: "user".into(),
                    content: format!("how does {topic} work?"),
                },
                MemoryMessage {
                    role: "assistant".into(),
                    content: format!("explanation of {topic} in detail"),
                },
            ],
            tools: vec![ToolEvidence {
                name: "view_file".into(),
                is_error: false,
                args_summary: format!("{topic}.rs"),
                result_summary: "fn example() {}".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        };

        let mut corpus = Vec::new();

        // 11 read-only explanatory turns (55% of corpus)
        let topics = [
            "routing",
            "storage",
            "crypto",
            "cache",
            "types",
            "cli",
            "auth",
            "parser",
            "validator",
            "runtime",
            "logging",
        ];
        for (i, topic) in topics.iter().enumerate() {
            corpus.push(read_only(i as u64 + 1, topic));
        }

        // 9 durable-signal turns (45% of corpus):
        // 1. Successful code edit + test pass
        corpus.push(LearningEvidence {
            session_id: "sess-edit-1".into(),
            repo_id: "repo-benchmark".into(),
            turn: 12,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "fix the timeout".into(),
            }],
            tools: vec![ToolEvidence {
                name: "write_to_file".into(),
                is_error: false,
                args_summary: "timeout.rs".into(),
                result_summary: "saved".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        });

        // 2. Successful code edit 2
        corpus.push(LearningEvidence {
            session_id: "sess-edit-2".into(),
            repo_id: "repo-benchmark".into(),
            turn: 13,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "patch memory leak".into(),
            }],
            tools: vec![ToolEvidence {
                name: "replace_file_content".into(),
                is_error: false,
                args_summary: "leak.rs".into(),
                result_summary: "patched".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        });

        // 3. Failed edit + verification fail
        corpus.push(LearningEvidence {
            session_id: "sess-fail-edit".into(),
            repo_id: "repo-benchmark".into(),
            turn: 14,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "tweak parser".into(),
            }],
            tools: vec![ToolEvidence {
                name: "write_to_file".into(),
                is_error: false,
                args_summary: "parser.rs".into(),
                result_summary: "written".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: false,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        });

        // 4. Graph run success
        corpus.push(LearningEvidence {
            session_id: "sess-graph-success".into(),
            repo_id: "repo-benchmark".into(),
            turn: 15,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "run graph for migration".into(),
            }],
            tools: vec![ToolEvidence {
                name: "graph_run".into(),
                is_error: false,
                args_summary: "goal".into(),
                result_summary: "done".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: Some("run-success".into()),
                commands_ran: 2,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        });

        // 5. Graph run failure
        corpus.push(LearningEvidence {
            session_id: "sess-graph-fail".into(),
            repo_id: "repo-benchmark".into(),
            turn: 16,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "run graph for refactor".into(),
            }],
            tools: vec![ToolEvidence {
                name: "graph_run".into(),
                is_error: true,
                args_summary: "goal".into(),
                result_summary: "blocked".into(),
                permission_denied: false,
            }],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: Some("run-failed".into()),
                commands_ran: 1,
                passed: false,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        });

        // 6. User correction
        corpus.push(LearningEvidence {
            session_id: "sess-corr-1".into(),
            repo_id: "repo-benchmark".into(),
            turn: 17,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "that's wrong, don't use unwrap here".into(),
            }],
            tools: vec![],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 0,
                passed: false,
                user_accepted: false,
                user_corrected: true,
                permission_denied: false,
            },
        });

        // 7. User correction message
        corpus.push(LearningEvidence {
            session_id: "sess-corr-2".into(),
            repo_id: "repo-benchmark".into(),
            turn: 18,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "correction: check null before indexing".into(),
            }],
            tools: vec![],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        });

        // 8. Explicit learn request
        corpus.push(LearningEvidence {
            session_id: "sess-learn".into(),
            repo_id: "repo-benchmark".into(),
            turn: 19,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "/learn persist this deployment step".into(),
            }],
            tools: vec![],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        });

        // 9. Repeated tool failures
        corpus.push(LearningEvidence {
            session_id: "sess-repeated-fail".into(),
            repo_id: "repo-benchmark".into(),
            turn: 20,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "try database connection".into(),
            }],
            tools: vec![
                ToolEvidence {
                    name: "bash".into(),
                    is_error: true,
                    args_summary: "connect".into(),
                    result_summary: "connection refused".into(),
                    permission_denied: false,
                },
                ToolEvidence {
                    name: "bash".into(),
                    is_error: true,
                    args_summary: "retry".into(),
                    result_summary: "connection refused".into(),
                    permission_denied: false,
                },
            ],
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        });

        corpus
    }

    #[test]
    fn benchmark_review_gating_efficiency_and_artifact_preservation() {
        let corpus = mixed_settled_turn_corpus();
        assert_eq!(corpus.len(), 20);

        let baseline = run_learning_benchmark(&corpus, false);
        let gated = run_learning_benchmark(&corpus, true);

        assert_eq!(baseline.dispatched_reviews, 20);
        assert_eq!(gated.dispatched_reviews, 9);

        // Assert at least 40% lower median learning-review input tokens
        assert!(
            gated.median_input_tokens * 100 <= baseline.median_input_tokens * 60,
            "gated median ({}) must be <= 60% of baseline median ({})",
            gated.median_input_tokens,
            baseline.median_input_tokens
        );

        // Assert at most 5% relative reduction in accepted high-confidence artifacts
        let allowed_loss =
            ((baseline.accepted_high_confidence_artifacts as f64) * 0.05).ceil() as usize;
        let actual_loss = baseline
            .accepted_high_confidence_artifacts
            .saturating_sub(gated.accepted_high_confidence_artifacts);
        assert!(
            actual_loss <= allowed_loss,
            "artifact loss ({actual_loss}) exceeded allowed loss ({allowed_loss})"
        );
        assert_eq!(
            gated.accepted_high_confidence_artifacts,
            baseline.accepted_high_confidence_artifacts
        );
    }
}
