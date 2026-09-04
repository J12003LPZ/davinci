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
        let total = self.uncached_input_tokens + self.cached_input_tokens;
        if total == 0 {
            0.0
        } else {
            self.cached_input_tokens as f64 / total as f64
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

/// Evaluates whether the paired run results meet §16.5 release gate criteria:
/// - Verified success rate of optimized >= generic.
/// - Duplicate side-effect execution is zero.
/// - At least two major efficiency dimensions improve (wall time, turns, tokens)
///   without another materially worsening (>10% increase in median).
pub fn evaluate_release_gate(comparisons: &[PairedTaskComparison]) -> PairedDeltaSummary {
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
    let mut total_duplicates = 0;

    for c in comparisons {
        if c.generic_metrics.success {
            generic_successes += 1;
        }
        if c.optimized_metrics.success {
            optimized_successes += 1;
        }
        total_duplicates += c.optimized_metrics.duplicate_side_effects;

        // Calculate deltas only among tasks where generic succeeded (or both)
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
        median_wall <= 10.0 && median_resp <= 10.0 && median_tokens <= 10.0;

    let meets_gate = success_ok && side_effects_ok && (improvements >= 2) && no_material_worsening;

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
            prompt: "Find all usages of `previous_response_id` across crates/pi-ai".into(),
            expected_files_changed: vec![],
            forbidden_files_changed: vec!["Cargo.toml".into()],
            verification_command: None,
            verification_substring: Some("previous_response_id".into()),
        },
        CodexBenchmarkTask {
            id: "bugfix_jsonl".into(),
            name: "Fix jsonl truncation boundary".into(),
            prompt: "Fix string truncation bug in session serialization".into(),
            expected_files_changed: vec!["crates/pi-session/src/jsonl.rs".into()],
            forbidden_files_changed: vec!["Cargo.lock".into()],
            verification_command: Some("cargo test -p pi-session".into()),
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

    #[test]
    fn median_odd_and_even() {
        let mut odd = vec![5.0, 1.0, 3.0];
        assert_eq!(median(&mut odd), 3.0);

        let mut even = vec![1.0, 2.0, 5.0, 10.0];
        assert_eq!(median(&mut even), 3.5);
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
                    wall_time_ms: 7_500, // -25%
                    model_responses: 7,  // -30%
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

        let summary = evaluate_release_gate(&comparisons);
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

        let summary = evaluate_release_gate(&comparisons);
        assert!(!summary.meets_release_gate);
    }
}
