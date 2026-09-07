//! Codex live evaluation harness and paired A/B benchmark tooling matching §16.
//! Evaluates Pi generic vs Pi optimized Codex vs external Codex CLI.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexEvalProfile {
    PiGeneric,
    PiCodexOptimized,
    ExternalCodexCli,
}

impl CodexEvalProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PiGeneric => "pi_generic",
            Self::PiCodexOptimized => "pi_codex_optimized",
            Self::ExternalCodexCli => "external_codex_cli",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexBenchmarkTask {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub expected_files_changed: Vec<String>,
    pub forbidden_files_changed: Vec<String>,
    pub verification_command: Option<String>,
    pub verification_substring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexBenchmarkRunMetrics {
    pub success: bool,
    pub wall_time_ms: u64,
    pub model_responses: u32,
    pub tool_calls: u32,
    pub uncached_input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub duplicate_side_effects: u32,
}

impl CodexBenchmarkRunMetrics {
    pub fn cached_ratio(&self) -> f64 {
        let total = self.uncached_input_tokens as f64 + self.cached_input_tokens as f64;
        if total == 0.0 {
            0.0
        } else {
            self.cached_input_tokens as f64 / total
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedTaskComparison {
    pub task_id: String,
    pub generic_metrics: CodexBenchmarkRunMetrics,
    pub optimized_metrics: CodexBenchmarkRunMetrics,
    pub external_cli_metrics: Option<CodexBenchmarkRunMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    VerifiedSuccess,
    VerifiedFailure,
    Blocked,
    Aborted,
    BudgetExhausted,
    InfrastructureFailure,
    UnverifiedCompletion,
}

/// Observations collected by the runner, independently of assistant output.
/// This is intentionally not deserializable as a model-produced artifact.
#[derive(Debug, Clone, Default)]
pub struct OracleObservation {
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
    pub changed_files: Vec<String>,
    pub file_inventory_complete: bool,
}

impl OracleObservation {
    pub fn outcome(&self, task: &CodexBenchmarkTask) -> RunOutcome {
        if !self.file_inventory_complete {
            return RunOutcome::UnverifiedCompletion;
        }
        if self.changed_files.iter().any(|file| {
            task.forbidden_files_changed.contains(file)
                || !task.expected_files_changed.contains(file)
        }) {
            return RunOutcome::VerifiedFailure;
        }
        // Substrings in assistant text are never an independent oracle.
        if task
            .verification_command
            .as_deref()
            .is_none_or(str::is_empty)
            || self.command != task.verification_command
            || self.exit_code.is_none()
        {
            return RunOutcome::UnverifiedCompletion;
        }
        if self.exit_code != Some(0)
            || task
                .expected_files_changed
                .iter()
                .any(|file| !self.changed_files.contains(file))
            || task
                .verification_substring
                .as_ref()
                .is_some_and(|expected| !self.output.contains(expected))
        {
            RunOutcome::VerifiedFailure
        } else {
            RunOutcome::VerifiedSuccess
        }
    }
}

/// Only runner observations can construct a verified comparison. Serializing
/// the legacy boolean remains supported; it is always derived from the oracle.
#[derive(Debug, Clone, Serialize)]
pub struct VerifiedTaskComparison {
    manifest_fingerprint: String,
    generic_outcome: RunOutcome,
    optimized_outcome: RunOutcome,
    comparison: PairedTaskComparison,
}

impl VerifiedTaskComparison {
    pub fn from_observations(
        task: &CodexBenchmarkTask,
        mut generic_metrics: CodexBenchmarkRunMetrics,
        generic: &OracleObservation,
        mut optimized_metrics: CodexBenchmarkRunMetrics,
        optimized: &OracleObservation,
    ) -> Self {
        let generic_outcome = generic.outcome(task);
        let optimized_outcome = optimized.outcome(task);
        generic_metrics.success = generic_outcome == RunOutcome::VerifiedSuccess;
        optimized_metrics.success = optimized_outcome == RunOutcome::VerifiedSuccess;
        Self {
            manifest_fingerprint: task_fingerprint(task),
            generic_outcome,
            optimized_outcome,
            comparison: PairedTaskComparison {
                task_id: task.id.clone(),
                generic_metrics,
                optimized_metrics,
                external_cli_metrics: None,
            },
        }
    }
}

fn task_fingerprint(task: &CodexBenchmarkTask) -> String {
    use sha2::{Digest, Sha256};
    // All fields are strings/arrays/options, so serialization is infallible.
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(task).expect("task serialization"))
    )
}

/// Require the complete caller-owned manifest and matching oracle definitions.
/// Imported legacy reports remain readable but cannot authorize a release.
pub fn evaluate_verified_report(
    schema_version: u32,
    expected: &[CodexBenchmarkTask],
    comparisons: &[VerifiedTaskComparison],
) -> PairedDeltaSummary {
    let raw: Vec<_> = comparisons
        .iter()
        .map(|pair| pair.comparison.clone())
        .collect();
    let mut summary = evaluate_efficiency(&raw);
    let ids: std::collections::HashSet<_> = expected.iter().map(|task| &task.id).collect();
    let coverage = !expected.is_empty()
        && ids.len() == expected.len()
        && expected.len() == comparisons.len()
        && comparisons.iter().all(|pair| {
            expected.iter().any(|task| {
                task.id == pair.comparison.task_id
                    && task_fingerprint(task) == pair.manifest_fingerprint
            })
        });
    summary.meets_release_gate &= schema_version == 1 && coverage;
    summary
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDeltaSummary {
    pub median_wall_time_delta_pct: f64,
    pub median_responses_delta_pct: f64,
    pub median_uncached_tokens_delta_pct: f64,
    pub median_tool_calls_delta_pct: f64,
    pub generic_success_rate: f64,
    pub optimized_success_rate: f64,
    pub duplicate_side_effects: u32,
    pub meets_release_gate: bool,
}

/// Calculate median of a slice of f64.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Computes a paired bootstrap 95% confidence interval for median delta (in percent).
pub fn paired_bootstrap_median_ci(deltas: &[f64], iterations: usize) -> (f64, f64, f64) {
    if deltas.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted_deltas = deltas.to_vec();
    let point_median = median(&mut sorted_deltas);
    if deltas.len() < 3 || iterations == 0 {
        return (point_median, point_median, point_median);
    }

    let mut resamples = Vec::with_capacity(iterations);
    let n = deltas.len();
    // Simple deterministic LCG for reproducible bootstrap across test runs
    let mut rng_state = 123456789u64;
    for _ in 0..iterations {
        let mut sample = Vec::with_capacity(n);
        for _ in 0..n {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let idx = (rng_state as usize) % n;
            sample.push(deltas[idx]);
        }
        resamples.push(median(&mut sample));
    }
    resamples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p025_idx = ((iterations as f64) * 0.025) as usize;
    let p975_idx = (((iterations as f64) * 0.975) as usize).min(iterations - 1);
    (point_median, resamples[p025_idx], resamples[p975_idx])
}

/// Summarize legacy metrics without authorizing promotion. Use
/// `evaluate_verified_report` for oracle-checked reports with exact coverage.
pub fn evaluate_release_gate(comparisons: &[PairedTaskComparison]) -> PairedDeltaSummary {
    let mut summary = evaluate_efficiency(comparisons);
    // Legacy input has neither independent oracle evidence nor a manifest.
    // Preserve its measurements, but never use a boolean claim for promotion.
    summary.meets_release_gate = false;
    summary
}

fn evaluate_efficiency(comparisons: &[PairedTaskComparison]) -> PairedDeltaSummary {
    if comparisons.is_empty() {
        return PairedDeltaSummary {
            median_wall_time_delta_pct: 0.0,
            median_responses_delta_pct: 0.0,
            median_uncached_tokens_delta_pct: 0.0,
            median_tool_calls_delta_pct: 0.0,
            generic_success_rate: 0.0,
            optimized_success_rate: 0.0,
            duplicate_side_effects: 0,
            meets_release_gate: false,
        };
    }

    let mut wall_time_deltas = Vec::new();
    let mut response_deltas = Vec::new();
    let mut token_deltas = Vec::new();
    let mut tool_deltas = Vec::new();
    let mut generic_successes = 0;
    let mut optimized_successes = 0;
    let mut total_duplicates = 0_u32;
    let mut case_ids = std::collections::HashSet::new();
    let mut complete_pairs = true;

    for c in comparisons {
        complete_pairs &= !c.task_id.trim().is_empty() && case_ids.insert(&c.task_id);
        if c.generic_metrics.success {
            generic_successes += 1;
        }
        if c.optimized_metrics.success {
            optimized_successes += 1;
        }
        total_duplicates =
            total_duplicates.saturating_add(c.optimized_metrics.duplicate_side_effects);

        // Failed runs remain in the outcome denominator, never in efficiency pairs.
        if !c.generic_metrics.success || !c.optimized_metrics.success {
            continue;
        }
        // Legacy counters cannot distinguish missing usage from zero. Such
        // records may be reported, but cannot justify an efficiency promotion.
        complete_pairs &= c.generic_metrics.wall_time_ms > 0
            && c.optimized_metrics.wall_time_ms > 0
            && c.generic_metrics.model_responses > 0
            && c.optimized_metrics.model_responses > 0
            && c.generic_metrics.uncached_input_tokens > 0
            && c.optimized_metrics.uncached_input_tokens > 0
            && (c.generic_metrics.tool_calls > 0 || c.optimized_metrics.tool_calls == 0);
        if c.generic_metrics.wall_time_ms > 0 {
            let delta = ((c.optimized_metrics.wall_time_ms as f64
                - c.generic_metrics.wall_time_ms as f64)
                / c.generic_metrics.wall_time_ms as f64)
                * 100.0;
            wall_time_deltas.push(delta);
        }
        if c.generic_metrics.model_responses > 0 {
            let delta = ((c.optimized_metrics.model_responses as f64
                - c.generic_metrics.model_responses as f64)
                / c.generic_metrics.model_responses as f64)
                * 100.0;
            response_deltas.push(delta);
        }
        if c.generic_metrics.uncached_input_tokens > 0 {
            let delta = ((c.optimized_metrics.uncached_input_tokens as f64
                - c.generic_metrics.uncached_input_tokens as f64)
                / c.generic_metrics.uncached_input_tokens as f64)
                * 100.0;
            token_deltas.push(delta);
        }
        if c.generic_metrics.tool_calls > 0 {
            let delta = ((c.optimized_metrics.tool_calls as f64
                - c.generic_metrics.tool_calls as f64)
                / c.generic_metrics.tool_calls as f64)
                * 100.0;
            tool_deltas.push(delta);
        }
    }

    let median_wall = median(&mut wall_time_deltas);
    let median_resp = median(&mut response_deltas);
    let median_tokens = median(&mut token_deltas);
    let median_tools = median(&mut tool_deltas);

    let n = comparisons.len() as f64;
    let gen_rate = generic_successes as f64 / n;
    let opt_rate = optimized_successes as f64 / n;

    // Release gate logic:
    // 1. Success rate must not regress
    let success_ok = opt_rate >= gen_rate;
    // 2. No duplicate side effects
    let side_effects_ok = total_duplicates == 0;
    // 3. At least 2 efficiency metrics improve (delta < 0%)
    let mut improvements = 0;
    if median_wall < 0.0 {
        improvements += 1;
    }
    if median_resp < 0.0 {
        improvements += 1;
    }
    if median_tokens < 0.0 {
        improvements += 1;
    }
    // 4. No material worsening (no metric > +10%)
    let no_material_worsening =
        median_wall <= 10.0 && median_resp <= 10.0 && median_tokens <= 10.0 && median_tools <= 10.0;

    let meets_gate = complete_pairs
        && !wall_time_deltas.is_empty()
        && !response_deltas.is_empty()
        && !token_deltas.is_empty()
        && success_ok
        && side_effects_ok
        && (improvements >= 2)
        && no_material_worsening;

    PairedDeltaSummary {
        median_wall_time_delta_pct: median_wall,
        median_responses_delta_pct: median_resp,
        median_uncached_tokens_delta_pct: median_tokens,
        median_tool_calls_delta_pct: median_tools,
        generic_success_rate: gen_rate,
        optimized_success_rate: opt_rate,
        duplicate_side_effects: total_duplicates,
        meets_release_gate: meets_gate,
    }
}

/// Built-in corpus of representative tasks for regression detection.
pub fn codex_benchmark_corpus() -> Vec<CodexBenchmarkTask> {
    vec![
        CodexBenchmarkTask {
            id: "discovery_symbols".into(),
            name: "Codebase discovery of symbols".into(),
            prompt: "Find all usages of `previous_response_id` across crates/davinci-ai".into(),
            expected_files_changed: vec![],
            forbidden_files_changed: vec!["Cargo.toml".into()],
            verification_command: None,
            verification_substring: Some("previous_response_id".into()),
        },
        CodexBenchmarkTask {
            id: "bugfix_jsonl".into(),
            name: "Fix jsonl truncation boundary".into(),
            prompt: "Fix string truncation bug in session serialization".into(),
            expected_files_changed: vec!["crates/davinci-session/src/jsonl_repo.rs".into()],
            forbidden_files_changed: vec!["Cargo.lock".into()],
            verification_command: Some("cargo test -p davinci-session".into()),
            verification_substring: None,
        },
        CodexBenchmarkTask {
            id: "patch_hunk_replacement".into(),
            name: "Apply multi-hunk patch to markdown and rust".into(),
            prompt: "Update documentation references and adjust test constant".into(),
            expected_files_changed: vec!["docs/ui/design.md".into()],
            forbidden_files_changed: vec![],
            verification_command: None,
            verification_substring: Some("Applied patch".into()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle_task() -> CodexBenchmarkTask {
        CodexBenchmarkTask {
            id: "oracle".into(),
            name: "oracle".into(),
            prompt: "fix".into(),
            expected_files_changed: vec!["src/lib.rs".into()],
            forbidden_files_changed: vec!["Cargo.lock".into()],
            verification_command: Some("fixture-test".into()),
            verification_substring: Some("passed".into()),
        }
    }

    #[test]
    fn independent_oracle_rejects_claims_and_forbidden_changes() {
        let task = oracle_task();
        let evidence = OracleObservation {
            command: Some("fixture-test".into()),
            exit_code: Some(0),
            output: "passed".into(),
            changed_files: vec!["src/lib.rs".into()],
            file_inventory_complete: true,
        };
        assert_eq!(evidence.outcome(&task), RunOutcome::VerifiedSuccess);
        let mut failed = evidence.clone();
        failed.exit_code = Some(1);
        assert_eq!(failed.outcome(&task), RunOutcome::VerifiedFailure);
        failed.exit_code = None;
        assert_eq!(failed.outcome(&task), RunOutcome::UnverifiedCompletion);
        let mut forbidden = evidence.clone();
        forbidden.changed_files.push("Cargo.lock".into());
        assert_eq!(forbidden.outcome(&task), RunOutcome::VerifiedFailure);
        let mut absent = evidence;
        absent.file_inventory_complete = false;
        assert_eq!(absent.outcome(&task), RunOutcome::UnverifiedCompletion);
    }

    #[test]
    fn report_requires_exact_manifest_and_schema() {
        let task = oracle_task();
        let evidence = OracleObservation {
            command: Some("fixture-test".into()),
            exit_code: Some(0),
            output: "passed".into(),
            changed_files: vec!["src/lib.rs".into()],
            file_inventory_complete: true,
        };
        let run = |wall_time_ms, model_responses, uncached_input_tokens| CodexBenchmarkRunMetrics {
            wall_time_ms,
            model_responses,
            uncached_input_tokens,
            ..Default::default()
        };
        let pair = VerifiedTaskComparison::from_observations(
            &task,
            run(100, 10, 100),
            &evidence,
            run(50, 5, 50),
            &evidence,
        );
        let tasks = vec![task.clone()];
        assert!(evaluate_verified_report(1, &tasks, &[pair.clone()]).meets_release_gate);
        assert!(!evaluate_verified_report(0, &tasks, &[pair.clone()]).meets_release_gate);
        assert!(!evaluate_verified_report(1, &tasks, &[]).meets_release_gate);
        assert!(
            !evaluate_verified_report(1, &tasks, &[pair.clone(), pair.clone()]).meets_release_gate
        );
        let mut changed_manifest = tasks.clone();
        changed_manifest[0].verification_command = Some("different-oracle".into());
        assert!(
            !evaluate_verified_report(1, &changed_manifest, &[pair.clone()]).meets_release_gate
        );
        let mut failed_oracle = evidence.clone();
        failed_oracle.exit_code = Some(1);
        let claimed_success = CodexBenchmarkRunMetrics {
            success: true,
            ..run(50, 5, 50)
        };
        let failed_pair = VerifiedTaskComparison::from_observations(
            &task,
            run(100, 10, 100),
            &evidence,
            claimed_success,
            &failed_oracle,
        );
        assert!(!evaluate_verified_report(1, &tasks, &[failed_pair]).meets_release_gate);
        let mut extra = task;
        extra.id = "missing".into();
        assert!(
            !evaluate_verified_report(1, &[tasks[0].clone(), extra], &[pair]).meets_release_gate
        );
    }

    #[test]
    fn legacy_boolean_cannot_authorize_release() {
        let pair = PairedTaskComparison {
            task_id: "legacy".into(),
            generic_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 100,
                model_responses: 10,
                uncached_input_tokens: 100,
                ..Default::default()
            },
            optimized_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 50,
                model_responses: 5,
                uncached_input_tokens: 50,
                ..Default::default()
            },
            external_cli_metrics: None,
        };
        assert!(!evaluate_release_gate(&[pair]).meets_release_gate);
    }

    #[test]
    fn failed_fast_runs_cannot_pass_efficiency_gate() {
        for (generic_success, optimized_success) in [(false, false), (false, true), (true, false)] {
            let run = |success, wall_time_ms, model_responses, uncached_input_tokens| {
                CodexBenchmarkRunMetrics {
                    success,
                    wall_time_ms,
                    model_responses,
                    tool_calls: model_responses,
                    uncached_input_tokens,
                    ..Default::default()
                }
            };
            let pairs = vec![PairedTaskComparison {
                task_id: "failed-fast".into(),
                generic_metrics: run(generic_success, 1_000, 10, 1_000),
                optimized_metrics: run(optimized_success, 100, 1, 100),
                external_cli_metrics: None,
            }];
            assert!(!evaluate_efficiency(&pairs).meets_release_gate);
        }
    }

    #[test]
    fn incomplete_and_duplicate_pairs_cannot_pass() {
        let baseline = CodexBenchmarkRunMetrics {
            success: true,
            wall_time_ms: 100,
            model_responses: 10,
            tool_calls: 10,
            uncached_input_tokens: 100,
            ..Default::default()
        };
        let candidate = CodexBenchmarkRunMetrics {
            success: true,
            wall_time_ms: 50,
            model_responses: 5,
            tool_calls: 5,
            uncached_input_tokens: 50,
            ..Default::default()
        };
        let pair = PairedTaskComparison {
            task_id: "case".into(),
            generic_metrics: baseline,
            optimized_metrics: candidate,
            external_cli_metrics: None,
        };
        assert!(!evaluate_efficiency(&[pair.clone(), pair.clone()]).meets_release_gate);
        let mut missing = pair.clone();
        missing.optimized_metrics.uncached_input_tokens = 0;
        assert!(!evaluate_efficiency(&[missing]).meets_release_gate);
        let mut zero = pair;
        zero.generic_metrics.tool_calls = 0;
        assert!(!evaluate_efficiency(&[zero]).meets_release_gate);
        assert!(!evaluate_efficiency(&[]).meets_release_gate);
    }

    #[test]
    fn median_odd_and_even() {
        let mut odd = vec![5.0, 1.0, 3.0];
        assert_eq!(median(&mut odd), 3.0);

        let mut even = vec![1.0, 2.0, 5.0, 10.0];
        assert_eq!(median(&mut even), 3.5);
    }

    #[test]
    fn cache_ratio_remains_finite_at_counter_limits() {
        let run = CodexBenchmarkRunMetrics {
            uncached_input_tokens: u64::MAX,
            cached_input_tokens: u64::MAX,
            ..Default::default()
        };
        assert_eq!(run.cached_ratio(), 0.5);
    }

    #[test]
    fn bootstrap_confidence_interval() {
        let deltas = vec![-20.0, -25.0, -30.0, -15.0, -22.0];
        let (pt, low, high) = paired_bootstrap_median_ci(&deltas, 200);
        assert!(pt < 0.0);
        assert!(low <= pt);
        assert!(high >= pt);
    }

    #[test]
    fn release_gate_passes_on_target_improvements() {
        let comparisons = vec![
            PairedTaskComparison {
                task_id: "t1".into(),
                generic_metrics: CodexBenchmarkRunMetrics {
                    success: true,
                    wall_time_ms: 10_000,
                    model_responses: 10,
                    uncached_input_tokens: 5_000,
                    ..Default::default()
                },
                optimized_metrics: CodexBenchmarkRunMetrics {
                    success: true,
                    wall_time_ms: 7_500,          // -25%
                    model_responses: 7,           // -30%
                    uncached_input_tokens: 3_750, // -25%
                    duplicate_side_effects: 0,
                    ..Default::default()
                },
                external_cli_metrics: None,
            },
            PairedTaskComparison {
                task_id: "t2".into(),
                generic_metrics: CodexBenchmarkRunMetrics {
                    success: true,
                    wall_time_ms: 8_000,
                    model_responses: 8,
                    uncached_input_tokens: 4_000,
                    ..Default::default()
                },
                optimized_metrics: CodexBenchmarkRunMetrics {
                    success: true,
                    wall_time_ms: 6_000,
                    model_responses: 5,
                    uncached_input_tokens: 3_000,
                    duplicate_side_effects: 0,
                    ..Default::default()
                },
                external_cli_metrics: None,
            },
        ];

        let summary = evaluate_efficiency(&comparisons);
        assert!(summary.meets_release_gate);
        assert!(summary.median_wall_time_delta_pct <= -20.0);
        assert!(summary.median_responses_delta_pct <= -25.0);
        assert_eq!(summary.duplicate_side_effects, 0);
    }

    #[test]
    fn release_gate_rejects_duplicate_side_effects() {
        let comparisons = vec![PairedTaskComparison {
            task_id: "t1".into(),
            generic_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 10_000,
                model_responses: 10,
                uncached_input_tokens: 5_000,
                ..Default::default()
            },
            optimized_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 5_000,
                model_responses: 5,
                uncached_input_tokens: 2_500,
                duplicate_side_effects: 1, // Violates zero duplicate side-effect gate
                ..Default::default()
            },
            external_cli_metrics: None,
        }];

        let summary = evaluate_efficiency(&comparisons);
        assert!(!summary.meets_release_gate);
    }

    #[test]
    fn release_gate_rejects_tool_call_explosion_despite_other_improvements() {
        let comparisons = vec![PairedTaskComparison {
            task_id: "tool-regression".into(),
            generic_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 100,
                model_responses: 10,
                uncached_input_tokens: 100,
                tool_calls: 10,
                ..Default::default()
            },
            optimized_metrics: CodexBenchmarkRunMetrics {
                success: true,
                wall_time_ms: 50,
                model_responses: 5,
                uncached_input_tokens: 50,
                tool_calls: 100,
                ..Default::default()
            },
            external_cli_metrics: None,
        }];
        assert!(!evaluate_efficiency(&comparisons).meets_release_gate);
    }
}
