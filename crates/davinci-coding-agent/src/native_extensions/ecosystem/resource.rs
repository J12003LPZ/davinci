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
}
