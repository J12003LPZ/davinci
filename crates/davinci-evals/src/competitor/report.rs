//! Claim classification and fail-closed competitor suite reporting.

use serde::{Deserialize, Serialize};

pub const MIN_SHARED_SCENARIOS: usize = 150;
pub const MIN_REPETITIONS: u32 = 3;
pub const MIN_PASS_RATE_DELTA: f64 = 0.03;
pub const MAX_WALL_TIME_REGRESSION: f64 = 1.15;

/// Whether a comparison controls the harness around a matched model or
/// measures each product with its configured/default model stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonMode {
    Harness,
    Product,
}

impl Default for ComparisonMode {
    fn default() -> Self {
        Self::Product
    }
}

impl ComparisonMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Product => "product",
        }
    }

    pub fn disclosure(self) -> &'static str {
        match self {
            Self::Harness => {
                "Harness comparison: same-model and model-parity controls are required."
            }
            Self::Product => {
                "Product comparison: each system uses its configured/default model stack."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonClass {
    HarnessControlled,
    ProductSystem,
    ArchitectureOnly,
}

impl ComparisonClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HarnessControlled => "harness-controlled",
            Self::ProductSystem => "product-system",
            Self::ArchitectureOnly => "architecture-only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompetitorSuiteSummary {
    #[serde(default)]
    pub comparison_mode: ComparisonMode,
    pub comparison_class: ComparisonClass,
    pub shared_scenarios: usize,
    pub davinci_pass_rate: f64,
    pub competitor_pass_rate: f64,
    pub pass_rate_delta_ci95: (f64, f64),
    pub davinci_unrelated_edit_rate: f64,
    pub competitor_unrelated_edit_rate: f64,
    pub davinci_wall_median_ms: f64,
    pub competitor_wall_median_ms: f64,
}

/// Refuse to combine harness and product observations in one ranking.
pub fn reject_mixed_comparison_modes(modes: &[ComparisonMode]) -> Result<ComparisonMode, String> {
    let Some(first) = modes.first().copied() else {
        return Err("comparison report contains no modes".into());
    };
    if modes.iter().any(|mode| *mode != first) {
        return Err("cannot aggregate harness and product comparison modes".into());
    }
    Ok(first)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompetitorClaimInputs {
    pub repetitions: u32,
    pub runtime_boundary_failures: u32,
    pub approved_non_inferiority: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimGateFailure {
    pub reason: String,
}

pub fn classify_comparison(models_controlled: bool, matched_run_executed: bool) -> ComparisonClass {
    if !matched_run_executed {
        ComparisonClass::ArchitectureOnly
    } else if models_controlled {
        ComparisonClass::HarnessControlled
    } else {
        ComparisonClass::ProductSystem
    }
}

pub fn evaluate_competitor_claim(
    summary: &CompetitorSuiteSummary,
    inputs: CompetitorClaimInputs,
) -> Result<(), Vec<ClaimGateFailure>> {
    let mut failures = Vec::new();
    if summary.comparison_class == ComparisonClass::ArchitectureOnly {
        failures.push(failure(
            "architecture-only comparisons cannot support a suite claim",
        ));
    }
    if summary.shared_scenarios < MIN_SHARED_SCENARIOS {
        failures.push(failure(format!(
            "shared scenario count {} is below {}",
            summary.shared_scenarios, MIN_SHARED_SCENARIOS
        )));
    }
    if inputs.repetitions < MIN_REPETITIONS {
        failures.push(failure(format!(
            "repetition count {} is below {}",
            inputs.repetitions, MIN_REPETITIONS
        )));
    }
    if !valid_rate(summary.davinci_pass_rate) || !valid_rate(summary.competitor_pass_rate) {
        failures.push(failure("pass rates must be finite values in [0, 1]"));
    } else if summary.davinci_pass_rate < summary.competitor_pass_rate + MIN_PASS_RATE_DELTA {
        failures.push(failure(format!(
            "DaVinci pass-rate delta {:.3} is below {:.3}",
            summary.davinci_pass_rate - summary.competitor_pass_rate,
            MIN_PASS_RATE_DELTA
        )));
    }
    let (ci_lower, ci_upper) = summary.pass_rate_delta_ci95;
    if !ci_lower.is_finite() || !ci_upper.is_finite() || ci_lower > ci_upper {
        failures.push(failure("pass-rate confidence interval is invalid"));
    } else if ci_lower <= 0.0 && !inputs.approved_non_inferiority {
        failures.push(failure(
            "pass-rate confidence interval lower bound is not above zero",
        ));
    }
    if !valid_rate(summary.davinci_unrelated_edit_rate)
        || !valid_rate(summary.competitor_unrelated_edit_rate)
    {
        failures.push(failure(
            "unrelated-edit rates must be finite values in [0, 1]",
        ));
    } else if summary.davinci_unrelated_edit_rate > summary.competitor_unrelated_edit_rate {
        failures.push(failure(
            "DaVinci unrelated-edit rate exceeds the competitor rate",
        ));
    }
    if !summary.davinci_wall_median_ms.is_finite()
        || !summary.competitor_wall_median_ms.is_finite()
        || summary.davinci_wall_median_ms < 0.0
        || summary.competitor_wall_median_ms < 0.0
        || summary.davinci_wall_median_ms
            > summary.competitor_wall_median_ms * MAX_WALL_TIME_REGRESSION
    {
        failures.push(failure(format!(
            "DaVinci wall median {:.1} ms exceeds the {:.0}% competitor allowance",
            summary.davinci_wall_median_ms,
            (MAX_WALL_TIME_REGRESSION - 1.0) * 100.0
        )));
    }
    if inputs.runtime_boundary_failures != 0 {
        failures.push(failure(format!(
            "{} runtime-boundary failures were recorded",
            inputs.runtime_boundary_failures
        )));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

pub fn claim_is_eligible(summary: &CompetitorSuiteSummary, inputs: CompetitorClaimInputs) -> bool {
    evaluate_competitor_claim(summary, inputs).is_ok()
}

pub fn format_competitor_suite_summary(summary: &CompetitorSuiteSummary) -> String {
    format!(
        "comparison_mode={}\ndisclosure={}\ncomparison_class={}\nshared_scenarios={}\ndavinci_pass_rate={:.4}\ncompetitor_pass_rate={:.4}\npass_rate_delta_ci95=({:.4}, {:.4})\ndavinci_unrelated_edit_rate={:.4}\ncompetitor_unrelated_edit_rate={:.4}\ndavinci_wall_median_ms={:.1}\ncompetitor_wall_median_ms={:.1}\n",
        summary.comparison_mode.as_str(),
        summary.comparison_mode.disclosure(),
        summary.comparison_class.as_str(),
        summary.shared_scenarios,
        summary.davinci_pass_rate,
        summary.competitor_pass_rate,
        summary.pass_rate_delta_ci95.0,
        summary.pass_rate_delta_ci95.1,
        summary.davinci_unrelated_edit_rate,
        summary.competitor_unrelated_edit_rate,
        summary.davinci_wall_median_ms,
        summary.competitor_wall_median_ms,
    )
}

fn valid_rate(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn failure(reason: impl Into<String>) -> ClaimGateFailure {
    ClaimGateFailure {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_summary(comparison_class: ComparisonClass) -> CompetitorSuiteSummary {
        CompetitorSuiteSummary {
            comparison_mode: ComparisonMode::Product,
            comparison_class,
            shared_scenarios: 150,
            davinci_pass_rate: 0.93,
            competitor_pass_rate: 0.90,
            pass_rate_delta_ci95: (0.01, 0.05),
            davinci_unrelated_edit_rate: 0.01,
            competitor_unrelated_edit_rate: 0.02,
            davinci_wall_median_ms: 100.0,
            competitor_wall_median_ms: 100.0,
        }
    }

    fn passing_inputs() -> CompetitorClaimInputs {
        CompetitorClaimInputs {
            repetitions: 3,
            runtime_boundary_failures: 0,
            approved_non_inferiority: false,
        }
    }

    #[test]
    fn classification_distinguishes_controlled_product_and_architecture_claims() {
        assert_eq!(
            classify_comparison(true, true),
            ComparisonClass::HarnessControlled
        );
        assert_eq!(
            classify_comparison(false, true),
            ComparisonClass::ProductSystem
        );
        assert_eq!(
            classify_comparison(true, false),
            ComparisonClass::ArchitectureOnly
        );
    }

    #[test]
    fn claim_gate_accepts_only_predeclared_winning_evidence() {
        let summary = passing_summary(ComparisonClass::ProductSystem);
        assert!(claim_is_eligible(&summary, passing_inputs()));
        assert!(format_competitor_suite_summary(&summary).contains("product-system"));
    }

    #[test]
    fn competitor_report_rejects_mixed_comparison_modes() {
        assert_eq!(
            reject_mixed_comparison_modes(&[ComparisonMode::Product, ComparisonMode::Product]),
            Ok(ComparisonMode::Product)
        );
        assert!(reject_mixed_comparison_modes(
            &[ComparisonMode::Harness, ComparisonMode::Product,]
        )
        .is_err());
    }

    #[test]
    fn competitor_report_discloses_comparison_mode() {
        let mut summary = passing_summary(ComparisonClass::HarnessControlled);
        summary.comparison_mode = ComparisonMode::Harness;
        let formatted = format_competitor_suite_summary(&summary);
        assert!(formatted.contains("comparison_mode=harness"));
        assert!(formatted.contains("same-model"));
    }

    #[test]
    fn claim_gate_rejects_insufficient_or_unsafe_evidence() {
        let mut summary = passing_summary(ComparisonClass::HarnessControlled);
        summary.shared_scenarios = 149;
        let mut inputs = passing_inputs();
        inputs.repetitions = 2;
        inputs.runtime_boundary_failures = 1;
        let failures = evaluate_competitor_claim(&summary, inputs).unwrap_err();
        assert!(failures.len() >= 3);

        summary.shared_scenarios = 150;
        inputs.repetitions = 3;
        inputs.runtime_boundary_failures = 0;
        summary.pass_rate_delta_ci95 = (-0.01, 0.05);
        assert!(!claim_is_eligible(&summary, inputs));
        inputs.approved_non_inferiority = true;
        assert!(claim_is_eligible(&summary, inputs));
    }

    #[test]
    fn architecture_only_comparison_is_never_claim_eligible() {
        assert!(!claim_is_eligible(
            &passing_summary(ComparisonClass::ArchitectureOnly),
            passing_inputs()
        ));
    }
}
