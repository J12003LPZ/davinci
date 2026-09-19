//! Per-run counters for the harness itself: how many times the model was
//! asked, how wide its tool batches were, how long tools and the provider
//! took, and how much context the run carried.
//!
//! No TypeScript counterpart. These are the numbers that separate a run
//! that finished in six turns from one that needed thirty: a UI can count
//! tool rows, but only the loop knows how many inference boundaries were
//! crossed and how much of the transcript was pruned before the provider
//! saw it. Exposed through `get_session_stats` (RPC), `/status` and
//! `--mode json`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The counters bumped from inside a tool call, where the agent is only
/// borrowed: batch operations, workers and evidence files. Folded into
/// `RunStats` by `Agent::run_stats`.
#[derive(Debug, Default)]
pub struct SharedCounters {
    pub batch_operations: AtomicU64,
    pub subagents: AtomicU64,
    pub evidence_files: AtomicU64,
    pub permission_prompts: AtomicU64,
    pub permission_denials: AtomicU64,
    pub files_changed_count: AtomicU64,
    pub verification_commands_run: AtomicU64,
    pub verification_failures: AtomicU64,
    pub process_startups: AtomicU64,
    pub process_reuses: AtomicU64,
    pub process_restarts: AtomicU64,
}

impl SharedCounters {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn add(counter: &AtomicU64, by: u64) {
        counter.fetch_add(by, Ordering::Relaxed);
    }

    pub fn fold_into(&self, stats: &mut RunStats) {
        stats.batch_operations = self.batch_operations.load(Ordering::Relaxed);
        stats.subagents = self.subagents.load(Ordering::Relaxed);
        stats.evidence_files = self.evidence_files.load(Ordering::Relaxed);
        stats.permission_prompts += self.permission_prompts.load(Ordering::Relaxed);
        stats.permission_denials += self.permission_denials.load(Ordering::Relaxed);
        stats.files_changed_count += self.files_changed_count.load(Ordering::Relaxed);
        stats.verification_commands_run += self.verification_commands_run.load(Ordering::Relaxed);
        stats.verification_failures += self.verification_failures.load(Ordering::Relaxed);
        stats.process_startups += self.process_startups.load(Ordering::Relaxed);
        stats.process_reuses += self.process_reuses.load(Ordering::Relaxed);
        stats.process_restarts += self.process_restarts.load(Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    #[serde(default)]
    pub process_startups: u64,
    #[serde(default)]
    pub process_reuses: u64,
    #[serde(default)]
    pub process_restarts: u64,
    /// Provider completions requested (one per model turn, retries excluded).
    pub model_turns: u64,
    /// Additional provider attempts actually started, excluding cancelled backoffs.
    #[serde(default)]
    pub provider_retries: u64,
    /// Assistant messages that carried at least one tool call.
    pub tool_batches: u64,
    /// Tool calls the model issued directly.
    pub tool_calls: u64,
    /// Widest single assistant message, in tool calls.
    pub max_batch_width: u64,
    /// Groups of two or more calls that ran concurrently.
    pub parallel_groups: u64,
    /// Operations run underneath `batch` calls (not model-visible boundaries).
    pub batch_operations: u64,
    /// Nested workers started by the `agent` tool.
    pub subagents: u64,
    /// Wall time spent inside tools, summed per batch (overlap counted once).
    pub tool_wall_ms: u64,
    /// Wall time spent waiting on the provider.
    pub model_wall_ms: u64,
    /// The largest estimated model-visible context, in tokens.
    pub peak_context_tokens: u64,
    /// Tool results whose bodies were pruned from the provider view.
    pub pruned_results: u64,
    /// Characters those prunings removed from the provider view.
    pub pruned_chars: u64,
    /// Files written to the evidence store for output that exceeded a cap.
    pub evidence_files: u64,
    /// Automatic compactions performed.
    pub compactions: u64,
    /// Permission prompts presented to the user/approver.
    #[serde(default)]
    pub permission_prompts: u64,
    /// Tool calls denied by permission rules or user rejection.
    #[serde(default)]
    pub permission_denials: u64,
    /// Distinct files modified during the run.
    #[serde(default)]
    pub files_changed_count: u64,
    /// Test or build verification commands executed.
    #[serde(default)]
    pub verification_commands_run: u64,
    /// Verification commands that resulted in test/build failures.
    #[serde(default)]
    pub verification_failures: u64,
    /// Capability completion attempts that reached the reminder ceiling with
    /// evidence still incomplete.
    #[serde(default)]
    pub capability_incomplete_evidence: u64,
    /// Mid-run steering inputs injected by the user.
    #[serde(default)]
    pub user_steers: u64,
    /// Total tokens charged by the root resource ledger across turns.
    #[serde(default)]
    pub budget_tokens_charged: u64,
    /// Whether any usage or pricing was unknown.
    #[serde(default)]
    pub has_unknown_cost: bool,
    /// Known cost in minor units, if pricing was available.
    #[serde(default)]
    pub cost_minor_units: Option<u64>,
}

impl RunStats {
    pub fn note_batch(&mut self, width: usize) {
        if width == 0 {
            return;
        }
        self.tool_batches += 1;
        self.tool_calls += width as u64;
        self.max_batch_width = self.max_batch_width.max(width as u64);
    }

    pub fn note_context(&mut self, tokens: u64) {
        self.peak_context_tokens = self.peak_context_tokens.max(tokens);
    }

    pub fn apply_budget_snapshot(&mut self, snapshot: &crate::runtime::BudgetSnapshot) {
        self.budget_tokens_charged = snapshot.tokens_charged;
        self.has_unknown_cost = snapshot.has_unknown_cost;
        self.cost_minor_units = snapshot.cost_minor_units;
    }

    pub fn mean_batch_width(&self) -> f64 {
        if self.tool_batches == 0 {
            0.0
        } else {
            self.tool_calls as f64 / self.tool_batches as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_accumulate_width() {
        let mut stats = RunStats::default();
        stats.note_batch(3);
        stats.note_batch(1);
        stats.note_batch(0);
        assert_eq!(stats.tool_batches, 2);
        assert_eq!(stats.tool_calls, 4);
        assert_eq!(stats.max_batch_width, 3);
        assert!((stats.mean_batch_width() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_value(RunStats::default()).unwrap();
        assert!(json.get("modelTurns").is_some());
        assert!(json.get("peakContextTokens").is_some());
        assert!(json.get("permissionPrompts").is_some());
        assert!(json.get("filesChangedCount").is_some());
    }

    #[test]
    fn older_stats_default_provider_retries_to_zero() {
        let mut json = serde_json::to_value(RunStats::default()).unwrap();
        json.as_object_mut().unwrap().remove("providerRetries");
        let restored: RunStats = serde_json::from_value(json).unwrap();
        assert_eq!(restored.provider_retries, 0);
        assert_eq!(restored.budget_tokens_charged, 0);
        assert!(!restored.has_unknown_cost);
        assert_eq!(restored.cost_minor_units, None);
    }

    #[test]
    fn test_stats_budget_snapshot_mapping() {
        let mut stats = RunStats::default();
        let snap = crate::runtime::BudgetSnapshot {
            root_run_id: crate::runtime::RunId::new(),
            revision: 1,
            token_ceiling: 10000,
            tokens_charged: 4500,
            tokens_reserved: 500,
            verification_reserve: 1500,
            handoff_reserve: 500,
            implementation_remaining: Some(3500),
            elapsed: std::time::Duration::from_secs(60),
            deadline: std::time::Duration::from_secs(900),
            active_workers: 2,
            max_concurrency: 4,
            retries_used: 1,
            retry_ceiling: 5,
            cache_read_tokens: 100,
            cache_write_tokens: 50,
            cost_minor_units: Some(120),
            has_unknown_cost: false,
        };

        stats.apply_budget_snapshot(&snap);
        assert_eq!(stats.budget_tokens_charged, 4500);
        assert_eq!(stats.cost_minor_units, Some(120));
        assert!(!stats.has_unknown_cost);
    }

    #[test]
    fn older_stats_default_behavioral_fields_to_zero() {
        let mut json = serde_json::to_value(RunStats::default()).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("permissionPrompts");
        obj.remove("permissionDenials");
        obj.remove("filesChangedCount");
        obj.remove("verificationCommandsRun");
        obj.remove("verificationFailures");
        obj.remove("capabilityIncompleteEvidence");
        obj.remove("userSteers");
        let restored: RunStats = serde_json::from_value(json).unwrap();
        assert_eq!(restored.permission_prompts, 0);
        assert_eq!(restored.permission_denials, 0);
        assert_eq!(restored.files_changed_count, 0);
        assert_eq!(restored.verification_commands_run, 0);
        assert_eq!(restored.verification_failures, 0);
        assert_eq!(restored.capability_incomplete_evidence, 0);
        assert_eq!(restored.user_steers, 0);
    }
}
