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
    pub(crate) digest_retrieval: ElapsedWork,
    pub(crate) integration: ElapsedWork,
    pub(crate) read_diagnostics: crate::tool_diagnostics::ReadDiagnostics,
    /// Leaf tool dispatches, including failures; excludes batch wrappers and journal replay.
    pub executed_leaf_operations: AtomicU64,
    pub batch_operations: AtomicU64,
    pub subagents: AtomicU64,
    pub evidence_files: AtomicU64,
    pub permission_prompts: AtomicU64,
    pub permission_denials: AtomicU64,
    pub files_changed_count: AtomicU64,
    pub verification_commands_run: AtomicU64,
    pub verification_failures: AtomicU64,
    pub verification_work_ms: AtomicU64,
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
        stats.digest_retrieval_ms = self.digest_retrieval.millis();
        stats.integration_ms = self.integration.millis();
        stats.batch_operations = self.batch_operations.load(Ordering::Relaxed);
        stats.subagents = self.subagents.load(Ordering::Relaxed);
        stats.evidence_files = self.evidence_files.load(Ordering::Relaxed);
        stats.permission_prompts += self.permission_prompts.load(Ordering::Relaxed);
        stats.permission_denials += self.permission_denials.load(Ordering::Relaxed);
        stats.files_changed_count += self.files_changed_count.load(Ordering::Relaxed);
        stats.verification_commands_run += self.verification_commands_run.load(Ordering::Relaxed);
        stats.verification_failures += self.verification_failures.load(Ordering::Relaxed);
        if self.verification_commands_run.load(Ordering::Relaxed) > 0 {
            stats.verification_work_ms = Some(self.verification_work_ms.load(Ordering::Relaxed));
        }
        stats.process_startups += self.process_startups.load(Ordering::Relaxed);
        stats.process_reuses += self.process_reuses.load(Ordering::Relaxed);
        stats.process_restarts += self.process_restarts.load(Ordering::Relaxed);
    }
}

/// Observed process work, including failed operations. Callers must avoid
/// nesting guards for the same phase; sums may overlap other phases.
#[derive(Debug, Default)]
pub(crate) struct ElapsedWork {
    observed: AtomicU64,
    nanos: AtomicU64,
}

impl ElapsedWork {
    pub(crate) fn start(&self) -> ElapsedWorkGuard<'_> {
        ElapsedWorkGuard {
            work: self,
            started: std::time::Instant::now(),
        }
    }

    pub(crate) fn millis(&self) -> Option<u64> {
        (self.observed.load(Ordering::Relaxed) > 0)
            .then(|| self.nanos.load(Ordering::Relaxed) / 1_000_000)
    }
}

pub(crate) struct ElapsedWorkGuard<'a> {
    work: &'a ElapsedWork,
    started: std::time::Instant,
}

impl Drop for ElapsedWorkGuard<'_> {
    fn drop(&mut self) {
        let elapsed = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let _ = self
            .work
            .nanos
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |total| {
                Some(total.saturating_add(elapsed))
            });
        self.work.observed.store(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    /// Elapsed host run time; includes overlapping work once. Never sum worker durations here.
    #[serde(default)]
    pub wall_ms: Option<u64>,
    /// Preparation before a foreground completion, including nested compaction when required.
    #[serde(default)]
    pub preparation_ms: Option<u64>,
    #[serde(default)]
    pub queue_ms: Option<u64>,
    /// Time in the completion adapter, excluding the host's queue and retry backoff.
    #[serde(default)]
    pub provider_ms: Option<u64>,
    #[serde(default)]
    pub retry_wait_ms: Option<u64>,
    /// Sum of observed verification process durations, possibly overlapping tool/worker time.
    #[serde(default)]
    pub verification_work_ms: Option<u64>,
    /// These phases are absent unless the owning execution path measures them.
    #[serde(default)]
    pub integration_ms: Option<u64>,
    #[serde(default)]
    pub digest_retrieval_ms: Option<u64>,
    /// Bounded in-process root diagnostics. Absent until at least one operation is observed.
    #[serde(default)]
    pub diagnostic_comparable_operations: Option<u64>,
    #[serde(default)]
    pub diagnostic_unknown_operations: Option<u64>,
    #[serde(default)]
    pub repeated_reads: Option<u64>,
    #[serde(default)]
    pub repeated_searches: Option<u64>,
    /// Matching successful operations by different actors; not proof they were avoidable.
    #[serde(default)]
    pub worker_duplicate_operations: Option<u64>,
    #[serde(default)]
    pub diagnostics_ms: Option<u64>,
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
    /// Actual transport sends. Absent in legacy stats and unobserved adapters.
    #[serde(default)]
    pub provider_attempts: Option<u64>,
    #[serde(default)]
    pub usage_complete_attempts: Option<u64>,
    #[serde(default)]
    pub usage_unknown_attempts: Option<u64>,
    #[serde(default)]
    pub provider_observation_overflow: bool,
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
    /// Legacy completion wall time, including admission and retry backoff.
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
    /// `completion.requirements` continuations, at most one per real prompt.
    #[serde(default)]
    pub completion_requirement_reminders: u64,
    /// `completion.hook_block` continuations.
    #[serde(default)]
    pub completion_hook_blocks: u64,
    /// `completion.hook_limit` notices after three consecutive blocks.
    #[serde(default)]
    pub completion_hook_limit_hits: u64,
    /// Mid-run steering inputs injected by the user.
    #[serde(default)]
    pub user_steers: u64,
    /// Extra turns `--output-schema` spent asking the model to fix a final
    /// answer that did not match the schema. Also counted in `model_turns`.
    #[serde(default)]
    pub output_schema_repair_turns: u64,
    /// Observed complete attempt tokens across turns; excludes reservations
    /// and attempts whose token provenance is incomplete.
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
    pub(crate) fn add_elapsed(counter: &mut Option<u64>, elapsed: std::time::Duration) {
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        *counter = Some(counter.unwrap_or(0).saturating_add(millis));
    }

    pub fn note_provider_observation(
        &mut self,
        event: &davinci_ai::provider_observation::ProviderAttemptObservation,
    ) {
        if self.provider_observation_overflow {
            return;
        }
        if event.kind == "attempt_start" {
            self.provider_attempts = Some(self.provider_attempts.unwrap_or(0).saturating_add(1));
            self.usage_complete_attempts.get_or_insert(0);
            self.usage_unknown_attempts =
                Some(self.usage_unknown_attempts.unwrap_or(0).saturating_add(1));
        } else if event.kind == "attempt_end" {
            if let Some(usage) = event
                .usage
                .as_ref()
                .filter(|_| event.usage_complete == Some(true))
            {
                self.usage_complete_attempts =
                    Some(self.usage_complete_attempts.unwrap_or(0).saturating_add(1));
                self.usage_unknown_attempts =
                    self.usage_unknown_attempts.map(|n| n.saturating_sub(1));
                self.budget_tokens_charged = self
                    .budget_tokens_charged
                    .saturating_add(usage.total_tokens);
                if usage.cost.total.is_finite() && usage.cost.total > 0.0 {
                    let minor = (usage.cost.total * 10_000.0).ceil() as u64;
                    self.cost_minor_units =
                        Some(self.cost_minor_units.unwrap_or(0).saturating_add(minor));
                } else {
                    self.has_unknown_cost = true;
                }
            } else {
                self.has_unknown_cost = true;
            }
        } else if event.kind == "telemetry_overflow" {
            self.provider_observation_overflow = true;
            self.has_unknown_cost = true;
            self.provider_attempts = None;
            self.usage_complete_attempts = None;
            self.usage_unknown_attempts = None;
        }
    }

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
    fn harness_attempt_usage_is_distinct_from_message_summaries() {
        use davinci_ai::provider_observation::{
            begin_request, record_openai_usage, Attempt, ObservationScope,
        };
        let scope = ObservationScope::capture();
        begin_request("coding", "fixture", None, "schema");
        Attempt::start("http").finish("failed", Some(500), None);
        let attempt = Attempt::start("http");
        record_openai_usage(&serde_json::json!({"input_tokens":10,"output_tokens":5,
            "input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}));
        attempt.finish(
            "completed",
            Some(200),
            Some(davinci_protocol::Usage {
                input: 10,
                output: 5,
                total_tokens: 15,
                ..Default::default()
            }),
        );
        let mut stats = RunStats::default();
        for event in scope.finish("completed") {
            stats.note_provider_observation(&event);
        }
        assert_eq!(stats.provider_attempts, Some(2));
        assert_eq!(stats.usage_complete_attempts, Some(1));
        assert_eq!(stats.usage_unknown_attempts, Some(1));
        assert_eq!(stats.budget_tokens_charged, 15);
        assert!(stats.has_unknown_cost);
    }

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
    fn harness_observation_overflow_stays_unknown_after_later_requests() {
        use davinci_ai::provider_observation::{begin_request, Attempt, ObservationScope};
        let scope = ObservationScope::capture();
        begin_request("coding", "fixture", None, "schema");
        Attempt::start("http").finish("failed", Some(500), None);
        let events = scope.finish("failed");
        let mut overflow = events[0].clone();
        overflow.kind = "telemetry_overflow".into();
        let mut stats = RunStats::default();
        stats.note_provider_observation(&overflow);
        for event in events {
            stats.note_provider_observation(&event);
        }
        assert!(stats.provider_observation_overflow);
        assert_eq!(stats.provider_attempts, None);
        assert_eq!(stats.usage_complete_attempts, None);
        assert_eq!(stats.usage_unknown_attempts, None);
        assert!(stats.has_unknown_cost);
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
    fn older_stats_default_behavioral_fields_to_zero() {
        let mut json = serde_json::to_value(RunStats::default()).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("permissionPrompts");
        obj.remove("permissionDenials");
        obj.remove("filesChangedCount");
        obj.remove("verificationCommandsRun");
        obj.remove("verificationFailures");
        obj.remove("capabilityIncompleteEvidence");
        obj.remove("completionRequirementReminders");
        obj.remove("completionHookBlocks");
        obj.remove("completionHookLimitHits");
        obj.remove("userSteers");
        obj.remove("outputSchemaRepairTurns");
        let restored: RunStats = serde_json::from_value(json).unwrap();
        assert_eq!(restored.permission_prompts, 0);
        assert_eq!(restored.permission_denials, 0);
        assert_eq!(restored.files_changed_count, 0);
        assert_eq!(restored.verification_commands_run, 0);
        assert_eq!(restored.verification_failures, 0);
        assert_eq!(restored.capability_incomplete_evidence, 0);
        assert_eq!(restored.completion_requirement_reminders, 0);
        assert_eq!(restored.completion_hook_blocks, 0);
        assert_eq!(restored.completion_hook_limit_hits, 0);
        assert_eq!(restored.user_steers, 0);
        assert_eq!(restored.output_schema_repair_turns, 0);
    }
}
