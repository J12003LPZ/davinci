//! Deterministic resource envelopes and accounting snapshots for graph executions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEnvelope {
    pub max_cost_usd: Option<f64>,
    pub run_deadline_ms: Option<u64>,
    pub max_parallel_workers: usize,
    pub context_soft_limit_tokens: Option<u64>,
}

impl Default for ResourceEnvelope {
    fn default() -> Self {
        Self {
            max_cost_usd: None,
            run_deadline_ms: None,
            max_parallel_workers: 4,
            context_soft_limit_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSnapshot {
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub governor_bytes_omitted: u64,
    pub governor_retrievals: u64,
    pub prunings: u64,
}

impl ResourceSnapshot {
    pub fn collect(
        tasks: &[crate::native_extensions::graph::GraphTaskState],
        governor_stats: Option<&crate::native_extensions::GovernorStats>,
    ) -> Self {
        let mut cost_usd = 0.0;
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut cache_read_tokens = 0;
        let mut cache_write_tokens = 0;

        for task in tasks {
            cost_usd += task.usage.cost_usd;
            input_tokens += task.usage.input;
            output_tokens += task.usage.output;
            cache_read_tokens += task.usage.cache_read;
            cache_write_tokens += task.usage.cache_write;
        }

        let (governor_bytes_omitted, governor_retrievals) = governor_stats
            .map(|g| (g.bytes_withheld, g.retrievals))
            .unwrap_or((0, 0));

        Self {
            cost_usd,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            governor_bytes_omitted,
            governor_retrievals,
            prunings: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_snapshot_default_is_zeroed() {
        let snap = ResourceSnapshot::default();
        assert_eq!(snap.cost_usd, 0.0);
        assert_eq!(snap.input_tokens, 0);
        assert_eq!(snap.output_tokens, 0);
        assert_eq!(snap.cache_read_tokens, 0);
        assert_eq!(snap.cache_write_tokens, 0);
        assert_eq!(snap.governor_bytes_omitted, 0);
        assert_eq!(snap.governor_retrievals, 0);
        assert_eq!(snap.prunings, 0);
    }

    #[test]
    fn test_resource_snapshot_collect_aggregates_usage_and_governor() {
        use crate::native_extensions::graph::{ArtifactKind, GraphTaskState, Role, WorkerUsage};
        use crate::native_extensions::GovernorStats;

        let mut t1 =
            GraphTaskState::new("t1", Role::Researcher, ArtifactKind::Evidence, vec![], None);
        t1.usage = WorkerUsage {
            input: 1000,
            output: 250,
            cache_read: 400,
            cache_write: 50,
            cost_usd: 0.05,
            turns: 2,
        };

        let mut t2 =
            GraphTaskState::new("t2", Role::Writer, ArtifactKind::PatchReport, vec![], None);
        t2.usage = WorkerUsage {
            input: 2000,
            output: 500,
            cache_read: 800,
            cache_write: 150,
            cost_usd: 0.10,
            turns: 3,
        };

        let gov_stats = GovernorStats {
            bytes_withheld: 12_500,
            retrievals: 2,
            compressed_outputs: 3,
            deduplicated_reads: 1,
            blocked_calls: 0,
        };

        let snapshot = ResourceSnapshot::collect(&[t1, t2], Some(&gov_stats));
        assert_eq!(snapshot.input_tokens, 3000);
        assert_eq!(snapshot.output_tokens, 750);
        assert_eq!(snapshot.cache_read_tokens, 1200);
        assert_eq!(snapshot.cache_write_tokens, 200);
        assert!((snapshot.cost_usd - 0.15).abs() < 1e-6);
        assert_eq!(snapshot.governor_bytes_omitted, 12_500);
        assert_eq!(snapshot.governor_retrievals, 2);
        assert_eq!(snapshot.prunings, 0);
    }
}
