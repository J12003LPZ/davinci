//! Behavior evaluation runner and suite score aggregation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::scenario::BehaviorScenario;
use super::scorer::{score_trace, ScoreCard};
use super::trace::{BehaviorEvent, BehaviorTrace};

pub const MAX_SCHEDULED_INFRASTRUCTURE_FAILURE_RATE: f64 = 0.10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunDisposition {
    #[serde(alias = "behavioral_result")]
    Completed,
    VerificationFailed,
    InfrastructureFailure,
    ConfigurationFailure,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunDispositionSummary {
    pub total_runs: usize,
    pub behavioral_runs: usize,
    pub infrastructure_failures: usize,
    pub configuration_failures: usize,
    #[serde(default)]
    pub verification_failures: usize,
    #[serde(default)]
    pub timed_out_runs: usize,
}

impl RunDispositionSummary {
    pub fn infrastructure_failure_rate(&self) -> f64 {
        rate(self.infrastructure_failures, self.total_runs)
    }

    pub fn configuration_failure_rate(&self) -> f64 {
        rate(self.configuration_failures, self.total_runs)
    }

    pub fn minimum_behavioral_runs_met(&self, minimum: usize) -> bool {
        self.behavioral_runs >= minimum
    }
}

pub fn scheduled_infrastructure_gate(summary: &RunDispositionSummary) -> Result<(), String> {
    let rate = summary.infrastructure_failure_rate();
    if rate > MAX_SCHEDULED_INFRASTRUCTURE_FAILURE_RATE {
        Err(format!(
            "infrastructure failure rate {:.2}% exceeds scheduled-eval limit of {:.2}%",
            rate * 100.0,
            MAX_SCHEDULED_INFRASTRUCTURE_FAILURE_RATE * 100.0
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DispositionedSuiteSummary {
    pub dispositions: RunDispositionSummary,
    pub behavioral: BehaviorSuiteSummary,
}

pub fn classify_failure_signal(signal: &str) -> RunDisposition {
    let lower = signal.to_ascii_lowercase();
    if ["timed out", "timeout"]
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        RunDisposition::TimedOut
    } else if ["verification failed", "verification command failed"]
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        RunDisposition::VerificationFailed
    } else if [
        "missing auth",
        "no credentials",
        "missing credentials",
        "api key",
        "authentication required",
        "credential",
        "configuration failure",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        RunDisposition::ConfigurationFailure
    } else {
        RunDisposition::InfrastructureFailure
    }
}

pub fn classify_process_result(
    exit_code: i32,
    timed_out: bool,
    stdout: &str,
    stderr: &str,
) -> Option<RunDisposition> {
    if timed_out {
        return Some(RunDisposition::TimedOut);
    }
    // A successful product process may contain failed tool results or provider-
    // shaped text in its JSON transcript. Those are behavioral evidence, not a
    // harness outage. Only inspect failure signals when the product itself failed.
    if exit_code == 0 {
        return None;
    }
    let signal = format!("{stdout}\n{stderr}");
    let lower = signal.to_ascii_lowercase();
    if ["verification failed", "verification command failed"]
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return Some(RunDisposition::VerificationFailed);
    }
    if [
        "missing auth",
        "no credentials",
        "missing credentials",
        "api key",
        "authentication required",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return Some(RunDisposition::ConfigurationFailure);
    }
    if [
        "provider error",
        "status 429",
        "status 503",
        "rate limit",
        "connection refused",
        "panic",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return Some(RunDisposition::InfrastructureFailure);
    }
    (exit_code != 0 && stdout.trim().is_empty()).then_some(RunDisposition::InfrastructureFailure)
}

pub fn summarize_dispositions(dispositions: &[RunDisposition]) -> RunDispositionSummary {
    let mut summary = RunDispositionSummary {
        total_runs: dispositions.len(),
        behavioral_runs: 0,
        infrastructure_failures: 0,
        configuration_failures: 0,
        verification_failures: 0,
        timed_out_runs: 0,
    };
    for disposition in dispositions {
        match disposition {
            RunDisposition::Completed => summary.behavioral_runs += 1,
            RunDisposition::VerificationFailed => {
                summary.behavioral_runs += 1;
                summary.verification_failures += 1;
            }
            RunDisposition::InfrastructureFailure => summary.infrastructure_failures += 1,
            RunDisposition::ConfigurationFailure => summary.configuration_failures += 1,
            RunDisposition::TimedOut => summary.timed_out_runs += 1,
        }
    }
    summary
}

pub fn aggregate_scenario_results(
    results: &[super::executor::ScenarioRunResult],
) -> DispositionedSuiteSummary {
    let dispositions = summarize_dispositions(
        &results
            .iter()
            .map(|result| result.disposition)
            .collect::<Vec<_>>(),
    );
    let behavioral_results = results
        .iter()
        .filter(|result| {
            matches!(
                result.disposition,
                RunDisposition::Completed | RunDisposition::VerificationFailed
            )
        })
        .map(|result| (&result.scenario, &result.trace, &result.score))
        .collect::<Vec<_>>();
    DispositionedSuiteSummary {
        dispositions,
        behavioral: aggregate_suite_scores(&behavioral_results),
    }
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

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

        let has_claim = trace
            .events
            .iter()
            .any(|e| matches!(e, BehaviorEvent::VerificationClaim { .. }));
        let has_passed_verification = trace.verification.iter().any(|v| v.passed);
        if has_claim && !has_passed_verification {
            unverified_claims_count += 1;
        }

        let unrelated_changed = card
            .hard_failures
            .iter()
            .any(|f| f.contains("unrelated files"));
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

pub fn evaluate_scenario_trace(scenario: &BehaviorScenario, trace: &BehaviorTrace) -> ScoreCard {
    score_trace(scenario, trace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{
        BehaviorCategory, BehaviorLimits, BehaviorRequirement, BehaviorScenario,
    };
    use crate::behavior::trace::{BehaviorEvent, BehaviorTrace};

    #[test]
    fn aggregate_suite_scores_computes_correct_metrics() {
        let scen1 = BehaviorScenario {
            id: "s1".into(),
            category: BehaviorCategory::Exploration,
            request: "r1".into(),
            repo_fixture: "f1".into(),
            requirements: vec![BehaviorRequirement::ToolUsed {
                tool: "grep".into(),
            }],
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
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
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        };

        let mut trace2 = BehaviorTrace::new("s2", None);
        trace2.events.push(BehaviorEvent::VerificationClaim {
            claim: "all tests pass".into(),
        });
        trace2.stats.model_turns = 4;
        trace2.stats.tool_calls = 6;

        let card2 = evaluate_scenario_trace(&scen2, &trace2);
        assert!(!card2.passed);

        let results = vec![(&scen1, &trace1, &card1), (&scen2, &trace2, &card2)];

        let summary = aggregate_suite_scores(&results);

        assert_eq!(summary.total_scenarios, 2);
        assert_eq!(summary.passed_scenarios, 1);
        assert_eq!(summary.macro_pass_rate, 0.5);
        assert_eq!(
            summary.category_pass_rates.get("exploration").copied(),
            Some(1.0)
        );
        assert_eq!(
            summary
                .category_pass_rates
                .get("verification_integrity")
                .copied(),
            Some(0.0)
        );
        assert_eq!(summary.unverified_claim_rate, 0.5);
        assert_eq!(summary.median_model_turns, 4.0);
    }

    #[test]
    fn disposition_summary_counts_verification_failures_as_behavioral_runs() {
        let summary = summarize_dispositions(&[
            RunDisposition::Completed,
            RunDisposition::InfrastructureFailure,
            RunDisposition::ConfigurationFailure,
            RunDisposition::VerificationFailed,
            RunDisposition::TimedOut,
        ]);

        assert_eq!(summary.total_runs, 5);
        assert_eq!(summary.behavioral_runs, 2);
        assert_eq!(summary.infrastructure_failures, 1);
        assert_eq!(summary.configuration_failures, 1);
        assert_eq!(summary.verification_failures, 1);
        assert_eq!(summary.timed_out_runs, 1);
        assert_eq!(summary.infrastructure_failure_rate(), 1.0 / 5.0);
        assert!(summary.minimum_behavioral_runs_met(1));
        assert!(summary.minimum_behavioral_runs_met(2));
    }

    #[test]
    fn classifies_auth_as_configuration_and_other_harness_errors_as_infrastructure() {
        assert_eq!(
            classify_failure_signal("provider returned no credentials"),
            RunDisposition::ConfigurationFailure
        );
        assert_eq!(
            classify_failure_signal("DaVinci process timed out"),
            RunDisposition::TimedOut
        );
        assert_eq!(
            classify_failure_signal("verification failed: cargo test"),
            RunDisposition::VerificationFailed
        );
        assert_eq!(
            classify_process_result(1, false, "", "no credentials configured"),
            Some(RunDisposition::ConfigurationFailure)
        );
        assert_eq!(
            classify_process_result(1, false, "", "provider error: status 503"),
            Some(RunDisposition::InfrastructureFailure)
        );
        assert_eq!(
            classify_process_result(1, true, "", ""),
            Some(RunDisposition::TimedOut)
        );
        assert_eq!(
            classify_process_result(
                0,
                false,
                r#"{"type":"tool_result","isError":true,"text":"provider error"}"#,
                ""
            ),
            None
        );
    }

    #[test]
    fn scheduled_infrastructure_gate_rejects_rates_above_ten_percent() {
        let passing = summarize_dispositions(&[
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::InfrastructureFailure,
        ]);
        assert!(scheduled_infrastructure_gate(&passing).is_ok());

        let failing = summarize_dispositions(&[
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::Completed,
            RunDisposition::InfrastructureFailure,
            RunDisposition::InfrastructureFailure,
        ]);
        assert!(scheduled_infrastructure_gate(&failing).is_err());
    }

    #[test]
    fn provider_configuration_failure_is_not_scored_as_task_failure() {
        let summary = summarize_dispositions(&[
            RunDisposition::ConfigurationFailure,
            RunDisposition::Completed,
        ]);

        assert_eq!(summary.configuration_failures, 1);
        assert_eq!(summary.behavioral_runs, 1);
        assert_eq!(summary.verification_failures, 0);
    }
}
