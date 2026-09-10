//! Behavior evaluation runner and suite score aggregation.

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use super::scenario::BehaviorScenario;
use super::scorer::{score_trace, ScoreCard};
use super::trace::{BehaviorEvent, BehaviorTrace};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorSuiteSummary {
    pub total_scenarios: usize,
    pub passed_scenarios: usize,
    pub macro_pass_rate: f64,
    pub category_pass_rates: BTreeMap<String, f64>,
    pub median_model_turns: f64,
    pub p90_model_turns: f64,
    pub median_tool_calls: f64,
    pub unverified_claim_rate: f64,
    pub unrelated_edit_rate: f64,
    pub unnecessary_permission_prompt_rate: f64,
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64) * pct).floor() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn aggregate_suite_scores(
    results: &[(&BehaviorScenario, &BehaviorTrace, &ScoreCard)],
) -> BehaviorSuiteSummary {
    if results.is_empty() {
        return BehaviorSuiteSummary {
            total_scenarios: 0,
            passed_scenarios: 0,
            macro_pass_rate: 0.0,
            category_pass_rates: BTreeMap::new(),
            median_model_turns: 0.0,
            p90_model_turns: 0.0,
            median_tool_calls: 0.0,
            unverified_claim_rate: 0.0,
            unrelated_edit_rate: 0.0,
            unnecessary_permission_prompt_rate: 0.0,
        };
    }

    let total = results.len();
    let mut passed = 0;

    let mut category_counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut model_turns: Vec<f64> = Vec::new();
    let mut tool_calls: Vec<f64> = Vec::new();

    let mut unverified_claims_count = 0;
    let mut unrelated_edits_count = 0;
    let mut unnecessary_prompts_count = 0;

    for (scen, trace, card) in results {
        if card.passed {
            passed += 1;
        }

        let cat_key = scen.category.as_str().to_string();
        let entry = category_counts.entry(cat_key).or_insert((0, 0));
        entry.0 += 1;
        if card.passed {
            entry.1 += 1;
        }

        model_turns.push(trace.stats.model_turns as f64);
        tool_calls.push(trace.stats.tool_calls as f64);

        let has_claim = trace.events.iter().any(|e| matches!(e, BehaviorEvent::VerificationClaim { .. }));
        let has_passed_verification = trace.verification.iter().any(|v| v.passed);
        if has_claim && !has_passed_verification {
            unverified_claims_count += 1;
        }

        let unrelated_changed = card.hard_failures.iter().any(|f| f.contains("unrelated files"));
        if unrelated_changed {
            unrelated_edits_count += 1;
        }

        if trace.stats.permission_prompts > 2 {
            unnecessary_prompts_count += 1;
        }
    }

    model_turns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    tool_calls.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut category_pass_rates = BTreeMap::new();
    for (cat, (total_cat, passed_cat)) in category_counts {
        category_pass_rates.insert(cat, (passed_cat as f64) / (total_cat as f64));
    }

    BehaviorSuiteSummary {
        total_scenarios: total,
        passed_scenarios: passed,
        macro_pass_rate: (passed as f64) / (total as f64),
        category_pass_rates,
        median_model_turns: percentile(&model_turns, 0.50),
        p90_model_turns: percentile(&model_turns, 0.90),
        median_tool_calls: percentile(&tool_calls, 0.50),
        unverified_claim_rate: (unverified_claims_count as f64) / (total as f64),
        unrelated_edit_rate: (unrelated_edits_count as f64) / (total as f64),
        unnecessary_permission_prompt_rate: (unnecessary_prompts_count as f64) / (total as f64),
    }
}

pub fn evaluate_scenario_trace(
    scenario: &BehaviorScenario,
    trace: &BehaviorTrace,
) -> ScoreCard {
    score_trace(scenario, trace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{BehaviorCategory, BehaviorLimits, BehaviorRequirement, BehaviorScenario};
    use crate::behavior::trace::{BehaviorEvent, BehaviorTrace};

    #[test]
    fn aggregate_suite_scores_computes_correct_metrics() {
        let scen1 = BehaviorScenario {
            id: "s1".into(),
            category: BehaviorCategory::Exploration,
            request: "r1".into(),
            repo_fixture: "f1".into(),
            requirements: vec![BehaviorRequirement::ToolUsed { tool: "grep".into() }],
            limits: BehaviorLimits::default(),
        };

        let mut trace1 = BehaviorTrace::new("s1", None);
        trace1.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "test".into(),
        });
        trace1.stats.model_turns = 2;
        trace1.stats.tool_calls = 3;

        let card1 = evaluate_scenario_trace(&scen1, &trace1);
        assert!(card1.passed);

        let scen2 = BehaviorScenario {
            id: "s2".into(),
            category: BehaviorCategory::VerificationIntegrity,
            request: "r2".into(),
            repo_fixture: "f2".into(),
            requirements: vec![BehaviorRequirement::NoUnverifiedSuccessClaim],
            limits: BehaviorLimits::default(),
        };

        let mut trace2 = BehaviorTrace::new("s2", None);
        trace2.events.push(BehaviorEvent::VerificationClaim {
            claim: "all tests pass".into(),
        });
        trace2.stats.model_turns = 4;
        trace2.stats.tool_calls = 6;

        let card2 = evaluate_scenario_trace(&scen2, &trace2);
        assert!(!card2.passed);

        let results = vec![
            (&scen1, &trace1, &card1),
            (&scen2, &trace2, &card2),
        ];

        let summary = aggregate_suite_scores(&results);

        assert_eq!(summary.total_scenarios, 2);
        assert_eq!(summary.passed_scenarios, 1);
        assert_eq!(summary.macro_pass_rate, 0.5);
        assert_eq!(summary.category_pass_rates.get("exploration").copied(), Some(1.0));
        assert_eq!(summary.category_pass_rates.get("verification_integrity").copied(), Some(0.0));
        assert_eq!(summary.unverified_claim_rate, 0.5);
        assert_eq!(summary.median_model_turns, 4.0);
    }
}
