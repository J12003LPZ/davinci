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
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
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
    }

    #[test]
    fn older_stats_default_provider_retries_to_zero() {
        let mut json = serde_json::to_value(RunStats::default()).unwrap();
        json.as_object_mut().unwrap().remove("providerRetries");
        let restored: RunStats = serde_json::from_value(json).unwrap();
        assert_eq!(restored.provider_retries, 0);
    }
}
