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
}
