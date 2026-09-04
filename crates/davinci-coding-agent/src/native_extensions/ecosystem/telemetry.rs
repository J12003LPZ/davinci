//! Structured ecosystem integration telemetry across Memory, Skills, Governor, Cache, Graph, Security, and Learning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EcosystemStats {
    #[serde(alias = "memory_hits")]
    pub memory_hits: u64,
    #[serde(alias = "memory_injected_tokens")]
    pub memory_injected_tokens: u64,
    #[serde(alias = "skill_candidates_considered")]
    pub skill_candidates_considered: u64,
    #[serde(alias = "skills_injected")]
    pub skills_injected: u64,
    #[serde(alias = "skill_injected_tokens")]
    pub skill_injected_tokens: u64,
    #[serde(alias = "context_packet_tokens")]
    pub context_packet_tokens: u64,
    #[serde(alias = "context_fingerprint", skip_serializing_if = "Option::is_none")]
    pub context_fingerprint: Option<String>,
    #[serde(alias = "governor_bytes_omitted")]
    pub governor_bytes_omitted: u64,
    #[serde(alias = "governor_retrievals")]
    pub governor_retrievals: u64,
    #[serde(alias = "prunings")]
    pub prunings: u64,
    #[serde(alias = "cache_read_tokens")]
    pub cache_read_tokens: u64,
    #[serde(alias = "cache_write_tokens")]
    pub cache_write_tokens: u64,
    #[serde(alias = "graph_workers")]
    pub graph_workers: u64,
    #[serde(alias = "graph_cost_usd")]
    pub graph_cost_usd: f64,
    #[serde(alias = "security_gate_triggered")]
    pub security_gate_triggered: bool,
    #[serde(alias = "security_result", skip_serializing_if = "Option::is_none")]
    pub security_result: Option<String>,
    #[serde(alias = "learning_reviews_dispatched")]
    pub learning_reviews_dispatched: u64,
    #[serde(alias = "learning_reviews_skipped")]
    pub learning_reviews_skipped: u64,
    #[serde(alias = "learned_artifacts_applied")]
    pub learned_artifacts_applied: u64,
}

impl EcosystemStats {
    /// Returns true if all counters are zero and no optional fields or triggers are populated.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.memory_hits == 0
            && self.memory_injected_tokens == 0
            && self.skill_candidates_considered == 0
            && self.skills_injected == 0
            && self.skill_injected_tokens == 0
            && self.context_packet_tokens == 0
            && self.context_fingerprint.is_none()
            && self.governor_bytes_omitted == 0
            && self.governor_retrievals == 0
            && self.prunings == 0
            && self.cache_read_tokens == 0
            && self.cache_write_tokens == 0
            && self.graph_workers == 0
            && self.graph_cost_usd == 0.0
            && !self.security_gate_triggered
            && self.security_result.is_none()
            && self.learning_reviews_dispatched == 0
            && self.learning_reviews_skipped == 0
            && self.learned_artifacts_applied == 0
    }

    #[allow(dead_code)]
    pub fn record_context_packet(
        &mut self,
        packet: &crate::native_extensions::ecosystem::ContextPacket,
    ) {
        if !packet.is_empty() {
            self.memory_hits += packet.memory_refs.len() as u64;
            self.memory_injected_tokens += packet.memory_tokens as u64;
            self.skill_candidates_considered += packet.skill_candidates_considered as u64;
            self.skills_injected += packet.skill_refs.len() as u64;
            self.skill_injected_tokens += packet.skill_tokens as u64;
            self.context_packet_tokens += packet.estimated_tokens as u64;
            self.context_fingerprint = Some(packet.fingerprint.clone());
        }
    }

    #[allow(dead_code)]
    pub fn record_governor(&mut self, stats: &crate::native_extensions::GovernorStats) {
        self.governor_bytes_omitted = stats.bytes_withheld;
        self.governor_retrievals = stats.retrievals;
        self.prunings = stats.prunings;
    }

    #[allow(dead_code)]
    pub fn record_worker_usage(
        &mut self,
        usage: &crate::native_extensions::graph::types::WorkerUsage,
    ) {
        self.cache_read_tokens += usage.cache_read;
        self.cache_write_tokens += usage.cache_write;
        self.graph_cost_usd += usage.cost_usd;
    }

    #[allow(dead_code)]
    pub fn record_security_gate(&mut self, triggered: bool, result: Option<String>) {
        self.security_gate_triggered = triggered;
        self.security_result = result;
    }

    #[allow(dead_code)]
    pub fn record_learning_stats(
        &mut self,
        stats: &crate::native_extensions::learning::LearningStats,
    ) {
        self.learning_reviews_dispatched = stats.reviews_dispatched;
        self.learning_reviews_skipped = stats.reviews_skipped;
        self.learned_artifacts_applied = stats.candidates_approved;
    }

    /// Formats compact operator lines for status sheets and run summaries.
    /// Only non-zero participation lines are returned; zero-state rows are omitted.
    pub fn render_compact_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        // 1. Context: memory / skills · tokens
        if self.memory_hits > 0 || self.skills_injected > 0 || self.context_packet_tokens > 0 {
            let mut parts = Vec::new();
            if self.memory_hits > 0 {
                parts.push(format!("{} memory", self.memory_hits));
            }
            if self.skills_injected > 0 {
                parts.push(format!(
                    "{} skill{}",
                    self.skills_injected,
                    if self.skills_injected == 1 { "" } else { "s" }
                ));
            }
            let base = if parts.is_empty() {
                String::new()
            } else {
                parts.join(" / ")
            };
            let tok = if self.context_packet_tokens > 0 {
                if base.is_empty() {
                    format!("{} tok", self.context_packet_tokens)
                } else {
                    format!("{base} · {} tok", self.context_packet_tokens)
                }
            } else {
                base
            };
            if !tok.is_empty() {
                lines.push(format!("{:<8} {}", "context", tok));
            }
        }

        // 2. Cache: read / write tokens
        if self.cache_read_tokens > 0 || self.cache_write_tokens > 0 {
            let read = format_compact_k(self.cache_read_tokens);
            let write = format_compact_k(self.cache_write_tokens);
            lines.push(format!("{:<8} {} read / {} write", "cache", read, write));
        }

        // 3. Compact: governor omitted · retrievals · prunings
        if self.governor_bytes_omitted > 0 || self.governor_retrievals > 0 || self.prunings > 0 {
            let mut parts = Vec::new();
            if self.governor_bytes_omitted > 0 {
                let kb = (self.governor_bytes_omitted as f64 / 1024.0).round() as u64;
                if kb > 0 {
                    parts.push(format!("{kb} KB governed"));
                } else {
                    parts.push(format!("{} B governed", self.governor_bytes_omitted));
                }
            }
            if self.governor_retrievals > 0 {
                parts.push(format!("{} recovered", self.governor_retrievals));
            }
            if self.prunings > 0 {
                parts.push(format!(
                    "{} pruning{}",
                    self.prunings,
                    if self.prunings == 1 { "" } else { "s" }
                ));
            }
            if !parts.is_empty() {
                lines.push(format!("{:<8} {}", "compact", parts.join(" · ")));
            }
        }

        // 4. Verify: tests / security
        if self.security_gate_triggered || self.security_result.is_some() {
            let sec_str = match self.security_result.as_deref() {
                Some("passed") => "security pass",
                Some("failed") => "security fail",
                Some(other) => other,
                None => "security not triggered",
            };
            lines.push(format!("{:<8} tests pass · {}", "verify", sec_str));
        }

        // 5. Learn: reviews · artifacts applied
        if self.learning_reviews_dispatched > 0
            || self.learned_artifacts_applied > 0
            || self.learning_reviews_skipped > 0
        {
            let mut parts = Vec::new();
            if self.learning_reviews_dispatched > 0 {
                parts.push(format!(
                    "{} review{}",
                    self.learning_reviews_dispatched,
                    if self.learning_reviews_dispatched == 1 { "" } else { "s" }
                ));
            } else if self.learning_reviews_skipped > 0 {
                parts.push(format!(
                    "{} review{} skipped",
                    self.learning_reviews_skipped,
                    if self.learning_reviews_skipped == 1 { "" } else { "s" }
                ));
            }
            if self.learned_artifacts_applied > 0 {
                parts.push(format!("{} applied", self.learned_artifacts_applied));
            }
            if !parts.is_empty() {
                lines.push(format!("{:<8} {}", "learn", parts.join(" · ")));
            }
        }

        lines
    }
}

fn format_compact_k(tokens: u64) -> String {
    format!("{:.1}k", tokens as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::GraphRun;

    #[test]
    fn ecosystem_stats_default_and_roundtrip_are_stable() {
        let stats = EcosystemStats::default();
        let json = serde_json::to_string(&stats).unwrap();
        let decoded: EcosystemStats = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.memory_hits, 0);
        assert_eq!(decoded.graph_cost_usd, 0.0);
        assert!(decoded.is_empty());
    }

    #[test]
    fn ecosystem_stats_roundtrip_with_values() {
        let stats = EcosystemStats {
            memory_hits: 3,
            memory_injected_tokens: 700,
            skill_candidates_considered: 2,
            skills_injected: 1,
            skill_injected_tokens: 320,
            context_packet_tokens: 1020,
            context_fingerprint: Some("abc123hash".into()),
            governor_bytes_omitted: 18432,
            governor_retrievals: 1,
            prunings: 1,
            cache_read_tokens: 4000,
            cache_write_tokens: 400,
            graph_workers: 5,
            graph_cost_usd: 0.042,
            security_gate_triggered: true,
            security_result: Some("passed".into()),
            learning_reviews_dispatched: 1,
            learning_reviews_skipped: 0,
            learned_artifacts_applied: 1,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let decoded: EcosystemStats = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, stats);
        assert!(!decoded.is_empty());
    }

    #[test]
    fn ecosystem_stats_supports_snake_case_deserialization() {
        let json = r#"{
            "memory_hits": 5,
            "graph_cost_usd": 0.12,
            "security_gate_triggered": true
        }"#;
        let decoded: EcosystemStats = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.memory_hits, 5);
        assert_eq!(decoded.graph_cost_usd, 0.12);
        assert!(decoded.security_gate_triggered);
        assert_eq!(decoded.skills_injected, 0);
    }

    #[test]
    fn graph_run_backward_compatibility_without_ecosystem_stats() {
        use crate::native_extensions::graph::types::GraphBudgets;

        let run_val = serde_json::json!({
            "version": 1,
            "runId": "run-legacy-1",
            "goal": "test goal",
            "cwd": "/test",
            "phase": "implement",
            "dryRun": false,
            "tasks": [],
            "budgets": GraphBudgets::default(),
            "counters": {
                "workersSpawned": 1,
                "revisionCycles": 0,
                "replans": 0,
                "costUsd": 0.01,
                "startedAt": 1000
            },
            "updatedAt": 1000
        });

        assert!(run_val.get("ecosystemStats").is_none());
        let run: GraphRun = serde_json::from_value(run_val).unwrap();
        assert_eq!(run.ecosystem_stats, EcosystemStats::default());
        assert_eq!(run.ecosystem_stats.memory_hits, 0);
        assert!(run.ecosystem_stats.is_empty());
    }

    #[test]
    fn ecosystem_telemetry_deterministic_aggregation_fixture() {
        let mut stats = EcosystemStats::default();

        // 1. memory packet: 3 hits / 700 tokens, skills: 2 candidates / 1 injected / 320 tokens
        let packet = crate::native_extensions::ecosystem::ContextPacket {
            text: "mock context text".into(),
            memory_refs: vec!["m1".into(), "m2".into(), "m3".into()],
            skill_refs: vec![crate::native_extensions::ecosystem::SkillContextRef {
                name: "test-skill".into(),
                version: 1,
                content_hash: "hash123".into(),
            }],
            estimated_tokens: 1020,
            fingerprint: "ctx-fp-xyz".into(),
            memory_tokens: 700,
            skill_tokens: 320,
            skill_candidates_considered: 2,
        };
        stats.record_context_packet(&packet);

        // 2. governor: 18 KB omitted / 1 retrieve_output, pruning: 1
        let gov_stats = crate::native_extensions::GovernorStats {
            bytes_withheld: 18 * 1024,
            retrievals: 1,
            compressed_outputs: 1,
            deduplicated_reads: 0,
            blocked_calls: 0,
            prunings: 1,
        };
        stats.record_governor(&gov_stats);

        // 3. provider: 4,000 cache-read / 400 cache-write tokens
        stats.graph_workers += 1;
        let worker_usage = crate::native_extensions::graph::types::WorkerUsage {
            input: 5000,
            output: 200,
            cache_read: 4000,
            cache_write: 400,
            cost_usd: 0.05,
            turns: 2,
        };
        stats.record_worker_usage(&worker_usage);

        // 4. security: triggered + passed
        stats.record_security_gate(true, Some("passed".into()));

        // 5. learning: 1 dispatched / 1 artifact applied
        let learn_stats = crate::native_extensions::learning::LearningStats {
            reviews_dispatched: 1,
            reviews_skipped: 0,
            candidates_approved: 1,
            ..Default::default()
        };
        stats.record_learning_stats(&learn_stats);

        // Assert every EcosystemStats field exactly
        assert_eq!(stats.memory_hits, 3);
        assert_eq!(stats.memory_injected_tokens, 700);
        assert_eq!(stats.skill_candidates_considered, 2);
        assert_eq!(stats.skills_injected, 1);
        assert_eq!(stats.skill_injected_tokens, 320);
        assert_eq!(stats.context_packet_tokens, 1020);
        assert_eq!(stats.context_fingerprint, Some("ctx-fp-xyz".into()));
        assert_eq!(stats.governor_bytes_omitted, 18432);
        assert_eq!(stats.governor_retrievals, 1);
        assert_eq!(stats.prunings, 1);
        assert_eq!(stats.cache_read_tokens, 4000);
        assert_eq!(stats.cache_write_tokens, 400);
        assert_eq!(stats.graph_workers, 1);
        assert!((stats.graph_cost_usd - 0.05).abs() < 1e-6);
        assert!(stats.security_gate_triggered);
        assert_eq!(stats.security_result, Some("passed".into()));
        assert_eq!(stats.learning_reviews_dispatched, 1);
        assert_eq!(stats.learning_reviews_skipped, 0);
        assert_eq!(stats.learned_artifacts_applied, 1);
    }

    #[test]
    fn ecosystem_telemetry_resumed_nodes_do_not_double_count_workers_or_cost() {
        use crate::native_extensions::graph::types::*;
        let mut stats = EcosystemStats::default();

        // Historical reused node: contributes to task status but current-run worker counters are NOT incremented
        let _reused_task = GraphTaskState {
            id: "classify".into(),
            role: Role::Classifier,
            expect: ArtifactKind::Classification,
            depends_on: vec![],
            focus: None,
            status: TaskStatus::Succeeded,
            attempts: 1,
            artifact_file: Some("artifacts/classify.json".into()),
            error: None,
            usage: WorkerUsage {
                input: 2000,
                output: 100,
                cache_read: 1500,
                cache_write: 100,
                cost_usd: 0.02,
                turns: 1,
            },
            started_at: Some(100),
            ended_at: Some(200),
            last_activity: None,
            fingerprint: None,
            mutation: None,
            context_fingerprint: None,
            context_tokens: 0,
            memory_refs: vec![],
            skill_refs: vec![],
        };

        // Live newly executed worker
        stats.graph_workers += 1;
        let live_usage = WorkerUsage {
            input: 1000,
            output: 50,
            cache_read: 800,
            cache_write: 50,
            cost_usd: 0.01,
            turns: 1,
        };
        stats.record_worker_usage(&live_usage);

        // Reused task didn't increment workers or cost
        assert_eq!(stats.graph_workers, 1);
        assert_eq!(stats.cache_read_tokens, 800);
        assert_eq!(stats.cache_write_tokens, 50);
        assert!((stats.graph_cost_usd - 0.01).abs() < 1e-6);
    }

    #[test]
    fn ecosystem_telemetry_render_non_zero_participation() {
        let stats = EcosystemStats {
            memory_hits: 3,
            memory_injected_tokens: 700,
            skills_injected: 1,
            skill_injected_tokens: 320,
            context_packet_tokens: 1020,
            cache_read_tokens: 18400,
            cache_write_tokens: 700,
            governor_bytes_omitted: 18432,
            governor_retrievals: 1,
            prunings: 1,
            security_gate_triggered: true,
            security_result: Some("passed".into()),
            learning_reviews_dispatched: 1,
            learned_artifacts_applied: 1,
            ..Default::default()
        };

        let lines = stats.render_compact_lines();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "context  3 memory / 1 skill · 1020 tok");
        assert_eq!(lines[1], "cache    18.4k read / 0.7k write");
        assert_eq!(lines[2], "compact  18 KB governed · 1 recovered · 1 pruning");
        assert_eq!(lines[3], "verify   tests pass · security pass");
        assert_eq!(lines[4], "learn    1 review · 1 applied");
    }

    #[test]
    fn ecosystem_telemetry_zero_state_omits_meaningless_zero_rows() {
        let stats = EcosystemStats::default();
        let lines = stats.render_compact_lines();
        assert!(
            lines.is_empty(),
            "zero state must omit meaningless rows, got: {lines:?}"
        );
    }
}
