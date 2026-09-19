use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextVmMetrics {
    pub images_compiled: u64,
    pub folds: u64,
    pub rebuilds: u64,
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
        self.page_fault_hits as f64
            / (self
                .page_fault_hits
                .saturating_add(self.page_fault_misses)
                .max(1) as f64)
    }
}
