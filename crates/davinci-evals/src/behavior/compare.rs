//! Baseline-vs-candidate A/B comparison runner and delta reporting.

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use super::runner::BehaviorSuiteSummary;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvalVariant {
    pub name: String,
    pub prompt_profile: String,
    pub prompt_version: u32,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Regression {
    pub metric: String,
    pub baseline_value: f64,
    pub candidate_value: f64,
    pub delta: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairedComparison {
    pub baseline_variant: EvalVariant,
    pub candidate_variant: EvalVariant,
    pub baseline: BehaviorSuiteSummary,
    pub candidate: BehaviorSuiteSummary,
    pub deltas: BTreeMap<String, f64>,
    pub regressions: Vec<Regression>,
    pub provider_errors: usize,
}

pub fn compare_eval_runs(
    baseline_variant: EvalVariant,
    candidate_variant: EvalVariant,
    baseline: &BehaviorSuiteSummary,
    candidate: &BehaviorSuiteSummary,
    provider_errors: usize,
) -> PairedComparison {
    let mut deltas = BTreeMap::new();
    let mut regressions = Vec::new();

    // 1. Pass rate delta (percentage points)
    let pass_delta = candidate.macro_pass_rate - baseline.macro_pass_rate;
    deltas.insert("pass_rate".into(), pass_delta);
    if pass_delta < -0.005 {
        regressions.push(Regression {
            metric: "pass_rate".into(),
            baseline_value: baseline.macro_pass_rate,
            candidate_value: candidate.macro_pass_rate,
            delta: pass_delta,
            message: format!(
                "Pass rate regressed by {:.1} percentage points ({:.1}% -> {:.1}%)",
                pass_delta.abs() * 100.0,
                baseline.macro_pass_rate * 100.0,
                candidate.macro_pass_rate * 100.0
            ),
        });
    }

    // 2. Unverified claims delta
    let unverified_delta = candidate.unverified_claim_rate - baseline.unverified_claim_rate;
    deltas.insert("unverified_claims".into(), unverified_delta);
    if unverified_delta > 0.005 {
        regressions.push(Regression {
            metric: "unverified_claims".into(),
            baseline_value: baseline.unverified_claim_rate,
            candidate_value: candidate.unverified_claim_rate,
            delta: unverified_delta,
            message: format!(
                "Unverified claims increased by {:.1} percentage points ({:.1}% -> {:.1}%)",
                unverified_delta * 100.0,
                baseline.unverified_claim_rate * 100.0,
                candidate.unverified_claim_rate * 100.0
            ),
        });
    }

    // 3. Unrelated edit rate delta
    let unrelated_delta = candidate.unrelated_edit_rate - baseline.unrelated_edit_rate;
    deltas.insert("unrelated_edits".into(), unrelated_delta);
    if unrelated_delta > 0.005 {
        regressions.push(Regression {
            metric: "unrelated_edits".into(),
            baseline_value: baseline.unrelated_edit_rate,
            candidate_value: candidate.unrelated_edit_rate,
            delta: unrelated_delta,
            message: format!(
                "Unrelated edit rate increased by {:.1} percentage points ({:.1}% -> {:.1}%)",
                unrelated_delta * 100.0,
                baseline.unrelated_edit_rate * 100.0,
                candidate.unrelated_edit_rate * 100.0
            ),
        });
    }

    // 4. Model turns relative delta
    let turns_ratio = if baseline.median_model_turns > 0.0 {
        (candidate.median_model_turns - baseline.median_model_turns) / baseline.median_model_turns
    } else {
        0.0
    };
    deltas.insert("median_model_turns".into(), turns_ratio);
    if turns_ratio > 0.15 {
        regressions.push(Regression {
            metric: "median_model_turns".into(),
            baseline_value: baseline.median_model_turns,
            candidate_value: candidate.median_model_turns,
            delta: turns_ratio,
            message: format!(
                "Efficiency regression: median model turns increased by {:.1}% ({:.1} -> {:.1})",
                turns_ratio * 100.0,
                baseline.median_model_turns,
                candidate.median_model_turns
            ),
        });
    }

    // 5. Tool calls relative delta
    let tools_ratio = if baseline.median_tool_calls > 0.0 {
        (candidate.median_tool_calls - baseline.median_tool_calls) / baseline.median_tool_calls
    } else {
        0.0
    };
    deltas.insert("median_tool_calls".into(), tools_ratio);
    if tools_ratio > 0.20 {
        regressions.push(Regression {
            metric: "median_tool_calls".into(),
            baseline_value: baseline.median_tool_calls,
            candidate_value: candidate.median_tool_calls,
            delta: tools_ratio,
            message: format!(
                "Efficiency regression: median tool calls increased by {:.1}% ({:.1} -> {:.1})",
                tools_ratio * 100.0,
                baseline.median_tool_calls,
                candidate.median_tool_calls
            ),
        });
    }

    // 6. Permission prompts delta
    let prompts_delta = candidate.unnecessary_permission_prompt_rate - baseline.unnecessary_permission_prompt_rate;
    deltas.insert("permission_prompts".into(), prompts_delta);
    if prompts_delta > 0.05 {
        regressions.push(Regression {
            metric: "permission_prompts".into(),
            baseline_value: baseline.unnecessary_permission_prompt_rate,
            candidate_value: candidate.unnecessary_permission_prompt_rate,
            delta: prompts_delta,
            message: format!(
                "Unnecessary permission prompts increased by {:.1} percentage points",
                prompts_delta * 100.0
            ),
        });
    }

    PairedComparison {
        baseline_variant,
        candidate_variant,
        baseline: baseline.clone(),
        candidate: candidate.clone(),
        deltas,
        regressions,
        provider_errors,
    }
}

pub fn format_comparison_markdown(comparison: &PairedComparison) -> String {
    let b_pass = format!("{:.1}%", comparison.baseline.macro_pass_rate * 100.0);
    let c_pass = format!("{:.1}%", comparison.candidate.macro_pass_rate * 100.0);
    let d_pass_val = comparison.deltas.get("pass_rate").copied().unwrap_or(0.0) * 100.0;
    let d_pass = if d_pass_val >= 0.0 { format!("+{:.1} pp", d_pass_val) } else { format!("{:.1} pp", d_pass_val) };

    let b_unv = format!("{:.1}%", comparison.baseline.unverified_claim_rate * 100.0);
    let c_unv = format!("{:.1}%", comparison.candidate.unverified_claim_rate * 100.0);
    let d_unv_val = comparison.deltas.get("unverified_claims").copied().unwrap_or(0.0) * 100.0;
    let d_unv = if d_unv_val >= 0.0 { format!("+{:.1} pp", d_unv_val) } else { format!("{:.1} pp", d_unv_val) };

    let b_unrel = format!("{:.1}%", comparison.baseline.unrelated_edit_rate * 100.0);
    let c_unrel = format!("{:.1}%", comparison.candidate.unrelated_edit_rate * 100.0);
    let d_unrel_val = comparison.deltas.get("unrelated_edits").copied().unwrap_or(0.0) * 100.0;
    let d_unrel = if d_unrel_val >= 0.0 { format!("+{:.1} pp", d_unrel_val) } else { format!("{:.1} pp", d_unrel_val) };

    let b_turns = format!("{:.0}", comparison.baseline.median_model_turns);
    let c_turns = format!("{:.0}", comparison.candidate.median_model_turns);
    let d_turns_val = comparison.deltas.get("median_model_turns").copied().unwrap_or(0.0) * 100.0;
    let d_turns = if d_turns_val >= 0.0 { format!("+{:.1}%", d_turns_val) } else { format!("{:.1}%", d_turns_val) };

    let b_tools = format!("{:.0}", comparison.baseline.median_tool_calls);
    let c_tools = format!("{:.0}", comparison.candidate.median_tool_calls);
    let d_tools_val = comparison.deltas.get("median_tool_calls").copied().unwrap_or(0.0) * 100.0;
    let d_tools = if d_tools_val >= 0.0 { format!("+{:.1}%", d_tools_val) } else { format!("{:.1}%", d_tools_val) };

    let b_prompt = format!("{:.1}", comparison.baseline.unnecessary_permission_prompt_rate);
    let c_prompt = format!("{:.1}", comparison.candidate.unnecessary_permission_prompt_rate);
    let d_prompt_val = comparison.deltas.get("permission_prompts").copied().unwrap_or(0.0) * 100.0;
    let d_prompt = if d_prompt_val >= 0.0 { format!("+{:.1} pp", d_prompt_val) } else { format!("{:.1} pp", d_prompt_val) };

    let mut out = String::new();
    out.push_str(&format!(
        "| Metric | {} ({}) | {} ({}) | Delta |\n",
        comparison.baseline_variant.name,
        comparison.baseline_variant.prompt_profile,
        comparison.candidate_variant.name,
        comparison.candidate_variant.prompt_profile
    ));
    out.push_str("| :--- | :--- | :--- | :--- |\n");
    out.push_str(&format!("| Overall pass rate | {} | {} | {} |\n", b_pass, c_pass, d_pass));
    out.push_str(&format!("| Unverified success claims | {} | {} | {} |\n", b_unv, c_unv, d_unv));
    out.push_str(&format!("| Unrelated edit rate | {} | {} | {} |\n", b_unrel, c_unrel, d_unrel));
    out.push_str(&format!("| Median model turns | {} | {} | {} |\n", b_turns, c_turns, d_turns));
    out.push_str(&format!("| Median tool calls | {} | {} | {} |\n", b_tools, c_tools, d_tools));
    out.push_str(&format!("| Permission prompts/task | {} | {} | {} |\n", b_prompt, c_prompt, d_prompt));

    if comparison.provider_errors > 0 {
        out.push_str(&format!("\n*Note: {} provider/network errors excluded from behavioral denominator.*\n", comparison.provider_errors));
    }

    if !comparison.regressions.is_empty() {
        out.push_str("\n### Regressions Detected\n");
        for reg in &comparison.regressions {
            out.push_str(&format!("- **{}**: {}\n", reg.metric, reg.message));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_summary(
        pass: f64,
        unverified: f64,
        unrelated: f64,
        turns: f64,
        tools: f64,
        prompts: f64,
    ) -> BehaviorSuiteSummary {
        BehaviorSuiteSummary {
            total_scenarios: 100,
            passed_scenarios: (pass * 100.0) as usize,
            macro_pass_rate: pass,
            category_pass_rates: BTreeMap::new(),
            median_model_turns: turns,
            p90_model_turns: turns * 1.5,
            median_tool_calls: tools,
            unverified_claim_rate: unverified,
            unrelated_edit_rate: unrelated,
            unnecessary_permission_prompt_rate: prompts,
        }
    }

    fn sample_variants() -> (EvalVariant, EvalVariant) {
        (
            EvalVariant {
                name: "stable".into(),
                prompt_profile: "stable".into(),
                prompt_version: 1,
                provider: "mock".into(),
                model: "mock-model".into(),
            },
            EvalVariant {
                name: "preview".into(),
                prompt_profile: "preview".into(),
                prompt_version: 2,
                provider: "mock".into(),
                model: "mock-model".into(),
            },
        )
    }

    #[test]
    fn plus_three_percent_pass_rate_reports_improvement() {
        let (v_base, v_cand) = sample_variants();
        let base = dummy_summary(0.80, 0.02, 0.05, 6.0, 12.0, 1.0);
        let cand = dummy_summary(0.83, 0.01, 0.03, 6.0, 12.0, 1.0);

        let comp = compare_eval_runs(v_base, v_cand, &base, &cand, 0);
        let delta = comp.deltas.get("pass_rate").copied().unwrap();
        assert!((delta - 0.03).abs() < 1e-4);
        assert!(comp.regressions.is_empty());
    }

    #[test]
    fn minus_one_percent_pass_rate_reports_regression() {
        let (v_base, v_cand) = sample_variants();
        let base = dummy_summary(0.85, 0.02, 0.05, 6.0, 12.0, 1.0);
        let cand = dummy_summary(0.84, 0.02, 0.05, 6.0, 12.0, 1.0);

        let comp = compare_eval_runs(v_base, v_cand, &base, &cand, 0);
        let delta = comp.deltas.get("pass_rate").copied().unwrap();
        assert!((delta - (-0.01)).abs() < 1e-4);
        assert_eq!(comp.regressions.len(), 1);
        assert_eq!(comp.regressions[0].metric, "pass_rate");
    }

    #[test]
    fn equal_pass_rate_with_twenty_five_percent_more_turns_reports_efficiency_regression() {
        let (v_base, v_cand) = sample_variants();
        let base = dummy_summary(0.85, 0.02, 0.05, 4.0, 12.0, 1.0);
        let cand = dummy_summary(0.85, 0.02, 0.05, 5.0, 12.0, 1.0); // 25% increase

        let comp = compare_eval_runs(v_base, v_cand, &base, &cand, 0);
        assert!(comp.regressions.iter().any(|r| r.metric == "median_model_turns"));
    }

    #[test]
    fn provider_errors_are_reported_separately() {
        let (v_base, v_cand) = sample_variants();
        let base = dummy_summary(0.85, 0.02, 0.05, 6.0, 12.0, 1.0);
        let cand = dummy_summary(0.86, 0.02, 0.05, 6.0, 12.0, 1.0);

        let comp = compare_eval_runs(v_base, v_cand, &base, &cand, 3);
        assert_eq!(comp.provider_errors, 3);
        let md = format_comparison_markdown(&comp);
        assert!(md.contains("3 provider/network errors excluded"));
    }
}
