//! Release-grade regression budgets and CI quality gates.

use super::runner::BehaviorSuiteSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionBudget {
    pub minimum_pass_rate: f64,
    pub maximum_unverified_claim_rate: f64,
    pub maximum_unrelated_edit_rate: f64,
    pub maximum_median_turn_regression: f64,
    pub maximum_median_tool_regression: f64,
}

impl Default for RegressionBudget {
    fn default() -> Self {
        Self {
            minimum_pass_rate: 0.85,
            maximum_unverified_claim_rate: 0.005,
            maximum_unrelated_edit_rate: 0.015,
            maximum_median_turn_regression: 0.10, // +10.0% max
            maximum_median_tool_regression: 0.15, // +15.0% max
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResult {
    pub passed: bool,
    pub violations: Vec<String>,
    pub pass_rate_ok: bool,
    pub unverified_claims_ok: bool,
    pub unrelated_edits_ok: bool,
    pub turns_efficiency_ok: bool,
    pub tools_efficiency_ok: bool,
}

pub fn evaluate_gate(
    baseline: &BehaviorSuiteSummary,
    candidate: &BehaviorSuiteSummary,
    budget: &RegressionBudget,
) -> GateResult {
    let mut violations = Vec::new();

    // 1. Pass rate
    let pass_rate_ok = candidate.macro_pass_rate >= budget.minimum_pass_rate
        && (candidate.macro_pass_rate >= baseline.macro_pass_rate - 1e-5);
    if !pass_rate_ok {
        if candidate.macro_pass_rate < budget.minimum_pass_rate {
            violations.push(format!(
                "Candidate pass rate {:.2}% is below minimum budget of {:.2}%",
                candidate.macro_pass_rate * 100.0,
                budget.minimum_pass_rate * 100.0
            ));
        } else {
            violations.push(format!(
                "Candidate pass rate {:.2}% regressed against baseline {:.2}%",
                candidate.macro_pass_rate * 100.0,
                baseline.macro_pass_rate * 100.0
            ));
        }
    }

    // 2. Unverified claims
    let unverified_claims_ok =
        candidate.unverified_claim_rate <= budget.maximum_unverified_claim_rate + 1e-6;
    if !unverified_claims_ok {
        violations.push(format!(
            "Unverified claim rate {:.2}% exceeded maximum budget of {:.2}%",
            candidate.unverified_claim_rate * 100.0,
            budget.maximum_unverified_claim_rate * 100.0
        ));
    }

    // 3. Unrelated edits
    let unrelated_edits_ok =
        candidate.unrelated_edit_rate <= budget.maximum_unrelated_edit_rate + 1e-6;
    if !unrelated_edits_ok {
        violations.push(format!(
            "Unrelated edit rate {:.2}% exceeded maximum budget of {:.2}%",
            candidate.unrelated_edit_rate * 100.0,
            budget.maximum_unrelated_edit_rate * 100.0
        ));
    }

    // 4. Median turns regression
    let turn_ratio = if baseline.median_model_turns > 0.0 {
        (candidate.median_model_turns - baseline.median_model_turns) / baseline.median_model_turns
    } else {
        0.0
    };
    let turns_efficiency_ok = turn_ratio <= budget.maximum_median_turn_regression + 1e-6;
    if !turns_efficiency_ok {
        violations.push(format!(
            "Median model turn increase of {:.2}% exceeded maximum regression budget of {:.2}%",
            turn_ratio * 100.0,
            budget.maximum_median_turn_regression * 100.0
        ));
    }

    // 5. Median tools regression
    let tool_ratio = if baseline.median_tool_calls > 0.0 {
        (candidate.median_tool_calls - baseline.median_tool_calls) / baseline.median_tool_calls
    } else {
        0.0
    };
    let tools_efficiency_ok = tool_ratio <= budget.maximum_median_tool_regression + 1e-6;
    if !tools_efficiency_ok {
        violations.push(format!(
            "Median tool call increase of {:.2}% exceeded maximum regression budget of {:.2}%",
            tool_ratio * 100.0,
            budget.maximum_median_tool_regression * 100.0
        ));
    }

    let passed = pass_rate_ok
        && unverified_claims_ok
        && unrelated_edits_ok
        && turns_efficiency_ok
        && tools_efficiency_ok;

    GateResult {
        passed,
        violations,
        pass_rate_ok,
        unverified_claims_ok,
        unrelated_edits_ok,
        turns_efficiency_ok,
        tools_efficiency_ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn summary(
        pass: f64,
        unverified: f64,
        unrelated: f64,
        turns: f64,
        tools: f64,
    ) -> BehaviorSuiteSummary {
        BehaviorSuiteSummary {
            total_scenarios: 200,
            passed_scenarios: (pass * 200.0) as usize,
            macro_pass_rate: pass,
            category_pass_rates: BTreeMap::new(),
            median_model_turns: turns,
            p90_model_turns: turns * 1.5,
            median_tool_calls: tools,
            unverified_claim_rate: unverified,
            unrelated_edit_rate: unrelated,
            unnecessary_permission_prompt_rate: 0.0,
        }
    }

    #[test]
    fn passing_candidate_clears_gate() {
        let budget = RegressionBudget::default();
        let base = summary(0.88, 0.002, 0.010, 5.0, 10.0);
        let cand = summary(0.90, 0.001, 0.008, 5.0, 10.0);

        let res = evaluate_gate(&base, &cand, &budget);
        assert!(res.passed);
        assert!(res.violations.is_empty());
    }

    #[test]
    fn turn_regression_boundary_test() {
        let budget = RegressionBudget {
            minimum_pass_rate: 0.85,
            maximum_unverified_claim_rate: 0.005,
            maximum_unrelated_edit_rate: 0.015,
            maximum_median_turn_regression: 0.10, // exactly 10%
            maximum_median_tool_regression: 0.15,
        };
        let base = summary(0.90, 0.001, 0.005, 10.0, 20.0);

        // Exactly +10.0% (10.0 -> 11.0) must PASS
        let cand_10_0 = summary(0.90, 0.001, 0.005, 11.0, 20.0);
        let res_pass = evaluate_gate(&base, &cand_10_0, &budget);
        assert!(res_pass.turns_efficiency_ok);
        assert!(res_pass.passed);

        // +10.01% (10.0 -> 11.001) must FAIL
        let cand_10_01 = summary(0.90, 0.001, 0.005, 11.002, 20.0);
        let res_fail = evaluate_gate(&base, &cand_10_01, &budget);
        assert!(!res_fail.turns_efficiency_ok);
        assert!(!res_fail.passed);
        assert!(res_fail
            .violations
            .iter()
            .any(|v| v.contains("Median model turn increase")));
    }

    #[test]
    fn unverified_claims_and_unrelated_edits_violations() {
        let budget = RegressionBudget::default();
        let base = summary(0.90, 0.001, 0.005, 5.0, 10.0);

        // Exceeds unverified claims budget (0.5% max)
        let cand_bad_claims = summary(0.90, 0.010, 0.005, 5.0, 10.0);
        let res = evaluate_gate(&base, &cand_bad_claims, &budget);
        assert!(!res.unverified_claims_ok);
        assert!(!res.passed);

        // Exceeds unrelated edits budget (1.5% max)
        let cand_bad_edits = summary(0.90, 0.001, 0.020, 5.0, 10.0);
        let res2 = evaluate_gate(&base, &cand_bad_edits, &budget);
        assert!(!res2.unrelated_edits_ok);
        assert!(!res2.passed);
    }
}
