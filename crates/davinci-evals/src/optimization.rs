//! Deterministic measurements and offline gates for harness optimizations.
//!
//! Provider usage is deliberately represented as supplied data. This module
//! never estimates billed tokens or invokes a provider or competitor binary.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The complete set of optimization experiments covered by the offline gate.
pub const OPTIMIZATION_ABLATION_NAMES: [&str; 8] = [
    "governor-content-routing-vnext",
    "root-context-budgeting",
    "capability-toolbox-node-query",
    "graph-deferred-schemas",
    "graph-failure-aware-retry",
    "memory-freshness",
    "security-incremental",
    "semantic-navigation",
];

/// Provider-reported usage copied into an evaluation record without conversion.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: Option<f64>,
    pub model_turns: u64,
    pub tool_calls: u64,
}

/// Common verified-success measurement record for optimization A/B runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedRunMetrics {
    pub verified_success: bool,
    pub first_attempt_verified_success: bool,
    pub provider_input_tokens: u64,
    pub provider_output_tokens: u64,
    pub provider_cache_read_tokens: u64,
    pub provider_cache_write_tokens: u64,
    pub provider_cost_usd: Option<f64>,
    pub model_turns: u64,
    pub tool_calls: u64,
    pub retries: u64,
    pub workers: u64,
    pub wall_ms: u64,
    pub scope_violations: u64,
    pub permission_failures: u64,
    pub false_success_claims: u64,
}

impl VerifiedRunMetrics {
    /// Attach exactly the usage values supplied by the provider adapter.
    pub fn with_provider_usage(mut self, usage: &ProviderUsageSnapshot) -> Self {
        self.provider_input_tokens = usage.input_tokens;
        self.provider_output_tokens = usage.output_tokens;
        self.provider_cache_read_tokens = usage.cache_read_tokens;
        self.provider_cache_write_tokens = usage.cache_write_tokens;
        self.provider_cost_usd = usage.cost_usd;
        self.model_turns = usage.model_turns;
        self.tool_calls = usage.tool_calls;
        self
    }
}

/// Cost per verified success, including the cost of failed runs.
///
/// A missing cost makes the aggregate unknown. Returning `None` is important:
/// zero is a valid reported cost, but it is not a valid stand-in for missing
/// provider data.
pub fn cost_per_verified_success(runs: &[VerifiedRunMetrics]) -> Option<f64> {
    let successes = runs.iter().filter(|run| run.verified_success).count();
    if successes == 0 || runs.is_empty() {
        return None;
    }
    if runs
        .iter()
        .any(|run| run.provider_cost_usd.is_none_or(|cost| !cost.is_finite()))
    {
        return None;
    }
    let total_cost = runs
        .iter()
        .map(|run| run.provider_cost_usd.expect("cost checked above"))
        .sum::<f64>();
    Some(total_cost / successes as f64)
}

/// One deterministic baseline/candidate comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationAblationResult {
    pub name: String,
    pub baseline_correct: bool,
    pub candidate_correct: bool,
    pub baseline_units: u64,
    pub candidate_units: u64,
    pub unit_name: String,
}

impl OptimizationAblationResult {
    pub fn new(
        name: impl Into<String>,
        baseline_correct: bool,
        candidate_correct: bool,
        baseline_units: u64,
        candidate_units: u64,
        unit_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            baseline_correct,
            candidate_correct,
            baseline_units,
            candidate_units,
            unit_name: unit_name.into(),
        }
    }

    pub fn correctness_preserved(&self) -> bool {
        self.baseline_correct && self.candidate_correct
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationGateReport {
    pub offline: bool,
    pub passed: bool,
    pub results: Vec<OptimizationAblationResult>,
    pub failures: Vec<String>,
}

/// Execute the named offline registry against production implementations. The
/// units are deterministic structural measurements, not provider-token claims.
pub fn offline_ablation_registry() -> Result<Vec<OptimizationAblationResult>, String> {
    davinci_coding_agent::optimization::deterministic_ablation_measurements().map(|measurements| {
        measurements
            .into_iter()
            .map(|measurement| {
                OptimizationAblationResult::new(
                    measurement.name,
                    measurement.baseline_correct,
                    measurement.candidate_correct,
                    measurement.baseline_units,
                    measurement.candidate_units,
                    measurement.unit_name,
                )
            })
            .collect()
    })
}

/// Check that every named experiment is present and that correctness was not
/// traded for a lower deterministic unit count.
pub fn evaluate_optimization_gate(
    results: &[OptimizationAblationResult],
) -> OptimizationGateReport {
    let mut failures = Vec::new();
    for name in OPTIMIZATION_ABLATION_NAMES {
        match results.iter().filter(|result| result.name == name).count() {
            0 => failures.push(format!("missing offline ablation: {name}")),
            1 => {}
            count => failures.push(format!("offline ablation {name} appeared {count} times")),
        }
    }
    for result in results {
        if !OPTIMIZATION_ABLATION_NAMES.contains(&result.name.as_str()) {
            failures.push(format!("unknown offline ablation: {}", result.name));
        }
    }
    for result in results {
        if !result.baseline_correct {
            failures.push(format!(
                "baseline is incorrect for ablation {}",
                result.name
            ));
        } else if !result.candidate_correct {
            failures.push(format!(
                "candidate lost correctness for ablation {}",
                result.name
            ));
        } else if result.baseline_correct
            && result.candidate_correct
            && result.candidate_units >= result.baseline_units
        {
            failures.push(format!(
                "candidate did not improve {} for ablation {}: baseline={}, candidate={}",
                result.unit_name, result.name, result.baseline_units, result.candidate_units
            ));
        }
    }
    OptimizationGateReport {
        offline: true,
        passed: failures.is_empty(),
        results: results.to_vec(),
        failures,
    }
}

/// Run the complete deterministic gate without touching external runners.
pub fn run_offline_optimization_gate() -> OptimizationGateReport {
    match offline_ablation_registry() {
        Ok(results) => evaluate_optimization_gate(&results),
        Err(error) => OptimizationGateReport {
            offline: true,
            passed: false,
            results: Vec::new(),
            failures: vec![format!("offline ablation execution failed: {error}")],
        },
    }
}

pub fn write_optimization_gate_report(
    report: &OptimizationGateReport,
    path: &Path,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create optimization report directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("failed to encode optimization report: {error}"))?;
    std::fs::write(path, bytes)
        .map_err(|error| format!("failed to write optimization report: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::semantic::{
        text_fallback_definition, text_fallback_diagnostics, text_fallback_outline,
        text_fallback_references,
    };
    use std::fs;
    use tempfile::tempdir;

    fn run(success: bool, cost: Option<f64>) -> VerifiedRunMetrics {
        VerifiedRunMetrics {
            verified_success: success,
            provider_cost_usd: cost,
            ..VerifiedRunMetrics::default()
        }
    }

    #[test]
    fn verified_metrics_cost_per_success_is_failures_inclusive() {
        let runs = [
            run(true, Some(1.0)),
            run(true, Some(2.0)),
            run(false, Some(3.0)),
        ];
        assert_eq!(cost_per_verified_success(&runs), Some(3.0));
    }

    #[test]
    fn verified_metrics_cost_per_success_is_unknown_without_successes_or_cost() {
        assert_eq!(cost_per_verified_success(&[run(false, Some(1.0))]), None);
        assert_eq!(cost_per_verified_success(&[run(true, None)]), None);
    }

    #[test]
    fn provider_usage_is_persisted_without_invention() {
        let usage = ProviderUsageSnapshot {
            input_tokens: 11,
            output_tokens: 13,
            cache_read_tokens: 17,
            cache_write_tokens: 19,
            cost_usd: Some(0.37),
            model_turns: 23,
            tool_calls: 29,
        };
        let metrics = VerifiedRunMetrics::default().with_provider_usage(&usage);
        assert_eq!(metrics.provider_input_tokens, 11);
        assert_eq!(metrics.provider_output_tokens, 13);
        assert_eq!(metrics.provider_cache_read_tokens, 17);
        assert_eq!(metrics.provider_cache_write_tokens, 19);
        assert_eq!(metrics.provider_cost_usd, Some(0.37));
        assert_eq!(metrics.model_turns, 23);
        assert_eq!(metrics.tool_calls, 29);
    }

    #[test]
    fn optimization_ablation_registry_covers_harness_program() {
        let names = offline_ablation_registry()
            .unwrap()
            .into_iter()
            .map(|result| result.name)
            .collect::<Vec<_>>();
        for name in OPTIMIZATION_ABLATION_NAMES {
            assert!(names.iter().any(|candidate| candidate == name), "{name}");
        }
    }

    #[test]
    fn optimization_gate_never_trades_correctness_for_savings() {
        let result = OptimizationAblationResult::new("test", true, false, 100, 1, "bytes");
        let report = evaluate_optimization_gate(&[result]);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("lost correctness")));
    }

    #[test]
    fn optimization_gate_rejects_incorrect_baseline() {
        let result = OptimizationAblationResult::new("test", false, true, 100, 1, "bytes");
        let report = evaluate_optimization_gate(&[result]);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("baseline is incorrect")));
    }

    #[test]
    fn optimization_gate_rejects_duplicate_and_unknown_ablations() {
        let mut results = OPTIMIZATION_ABLATION_NAMES
            .iter()
            .map(|name| OptimizationAblationResult::new(*name, true, true, 2, 1, "units"))
            .collect::<Vec<_>>();
        results.push(results[0].clone());
        results.push(OptimizationAblationResult::new(
            "unregistered",
            true,
            true,
            2,
            1,
            "units",
        ));

        let report = evaluate_optimization_gate(&results);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("appeared 2 times")));
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("unknown offline ablation")));
    }

    #[test]
    fn optimization_gate_rejects_no_op_candidates() {
        let result = OptimizationAblationResult::new("test", true, true, 100, 100, "bytes");
        let report = evaluate_optimization_gate(&[result]);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("did not improve")));
    }

    #[test]
    fn optimization_gate_offline_never_invokes_external_runner() {
        let report = run_offline_optimization_gate();
        assert!(report.offline);
        assert!(
            report.passed,
            "offline gate failures: {:?}",
            report.failures
        );
        assert_eq!(report.results.len(), OPTIMIZATION_ABLATION_NAMES.len());
    }

    #[test]
    fn semantic_navigation_fixture_covers_rust_and_typescript() {
        let dir = tempdir().unwrap();
        let fixtures = [
            (
                "rust_fixture.rs",
                "rust_symbol",
                "fn rust_symbol() {}\nfn caller() {\n    rust_symbol();\n}\n",
                "function",
            ),
            (
                "typescript_fixture.ts",
                "ts_symbol",
                "function ts_symbol() {}\nfunction caller() {\n  ts_symbol();\n}\n",
                "function",
            ),
        ];

        for (file_name, symbol, source, expected_kind) in fixtures {
            let file = dir.path().join(file_name);
            fs::write(&file, source).unwrap();

            let definition =
                text_fallback_definition(dir.path(), symbol, Some(&file), None).unwrap();
            assert_eq!(definition.locations.len(), 1, "definition for {file_name}");
            assert_eq!(definition.locations[0].range.start.line, 0);
            assert!(definition.locations[0].is_textual_fallback);

            let references =
                text_fallback_references(dir.path(), symbol, Some(&file), None).unwrap();
            assert_eq!(
                references
                    .locations
                    .iter()
                    .map(|location| location.range.start.line)
                    .collect::<Vec<_>>(),
                vec![0, 2],
                "references for {file_name}"
            );

            let outline = text_fallback_outline(dir.path(), file_name).unwrap();
            assert_eq!(outline.symbols.len(), 2, "outline for {file_name}");
            assert_eq!(outline.symbols[0].name, symbol);
            assert_eq!(outline.symbols[0].kind, expected_kind);
            assert!(!outline.partial);

            let diagnostics = text_fallback_diagnostics(dir.path(), file_name).unwrap();
            assert!(diagnostics.partial);
            assert!(diagnostics.diagnostics.is_empty());
        }
    }
}
