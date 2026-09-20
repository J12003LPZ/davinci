use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ContextVmMetrics {
    pub images_compiled: u64,
    pub folds: u64,
    pub rebuilds: u64,
    pub page_lookup_hits: u64,
    pub page_lookup_misses: u64,
    pub rebuild_attempts: u64,
    pub rebuild_successes: u64,
    pub rebuild_failures: u64,
    /// Explicit attempts to recover pageable context, excluding invalid requests.
    pub semantic_page_faults: u64,
    pub retrieval_hits: u64,
    pub retrieval_misses: u64,
    /// Compatibility counters for semantic retrieval, never cache rebuilds.
    pub page_faults: u64,
    pub page_fault_hits: u64,
    pub page_fault_misses: u64,
    pub shadow_missing_user_refs: u64,
    pub shadow_missing_tool_refs: u64,
    pub prefix_churn: u64,
    pub tokens_before_fold: u64,
    pub tokens_after_fold: u64,
}

impl ContextVmMetrics {
    pub fn context_recovery_rate(&self) -> f64 {
        self.retrieval_hits as f64 / self.semantic_page_faults.max(1) as f64
    }
}
