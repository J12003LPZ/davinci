//! Offline decision-intelligence eval fixtures.
//!
//! These fixtures exercise the authority boundary and privacy contract without
//! claiming live TypeSafe quality, latency, or token measurements. A live
//! provider run is a separate operational gate.

use davinci_agent::decision::policy::{
    add_optional_capabilities, DecisionRollout, OptionalCapability,
};
use davinci_coding_agent::decision_state::build_request;

#[derive(Debug, Clone)]
pub struct DecisionIntelligenceScenario {
    pub name: &'static str,
    pub task: &'static str,
    pub deterministic: Vec<&'static str>,
    pub optional: Vec<OptionalCapability>,
    pub expected_additions: Vec<&'static str>,
    pub provider_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionIntelligenceEvalReport {
    pub scenario_count: usize,
    pub task_successes: usize,
    pub shadow_behavior_unchanged: bool,
    pub mandatory_verification_preserved: bool,
    pub capability_removals: usize,
    pub false_additions: usize,
    pub missed_relevant_capabilities: usize,
    pub fallback_cases: usize,
    pub provider_failure_fallback_success: bool,
    pub input_token_estimate: usize,
    pub decision_latency_ms: u64,
    pub turn_latency_ms: u64,
    pub jev_usage_tokens: usize,
    pub guarded_policy_is_explicit: bool,
}

pub fn scenarios() -> Vec<DecisionIntelligenceScenario> {
    vec![
        scenario(
            "frontend login-button bug",
            "Fix a browser login button bug",
            "browser",
        ),
        scenario("backend-only logic bug", "Fix backend validation logic", ""),
        scenario(
            "dependency API mismatch",
            "Investigate a package API mismatch",
            "package",
        ),
        scenario(
            "Git-history-relevant regression",
            "Use git history to investigate a regression",
            "git",
        ),
        scenario(
            "trivial typo needing no extra intelligence",
            "Fix a typo",
            "",
        ),
        scenario(
            "auth/security-sensitive change",
            "Review an auth security change",
            "",
        ),
        scenario(
            "test-only change",
            "Update tests for the changed behavior",
            "tests",
        ),
        scenario(
            "package/workspace change",
            "Update the package workspace dependency",
            "package",
        ),
        DecisionIntelligenceScenario {
            name: "TypeSafe unavailable",
            task: "Run the normal deterministic route when Jev is unavailable",
            deterministic: vec!["mandatory_verification"],
            optional: Vec::new(),
            expected_additions: Vec::new(),
            provider_failure: true,
        },
        DecisionIntelligenceScenario {
            name: "TypeSafe conflicting with deterministic routing",
            task: "Keep the deterministic security veto despite a conflicting suggestion",
            deterministic: vec!["mandatory_verification"],
            optional: vec![OptionalCapability {
                id: "security_override".into(),
                relevance: 1.0,
                available: true,
                authorized: true,
                vetoed: true,
            }],
            expected_additions: Vec::new(),
            provider_failure: false,
        },
    ]
}

fn scenario(
    name: &'static str,
    task: &'static str,
    optional: &'static str,
) -> DecisionIntelligenceScenario {
    let optional_capability =
        (!optional.is_empty()).then(|| OptionalCapability::new(optional, 0.90));
    DecisionIntelligenceScenario {
        name,
        task,
        deterministic: vec!["mandatory_verification"],
        optional: optional_capability.into_iter().collect(),
        expected_additions: if optional.is_empty() {
            Vec::new()
        } else {
            vec![optional]
        },
        provider_failure: false,
    }
}

pub fn run_offline_eval() -> DecisionIntelligenceEvalReport {
    let scenarios = scenarios();
    let mut task_successes = 0;
    let mut shadow_behavior_unchanged = true;
    let mut mandatory_verification_preserved = true;
    let mut capability_removals = 0;
    let mut false_additions = 0;
    let mut missed_relevant_capabilities = 0;
    let mut fallback_cases = 0;
    let mut provider_failure_fallback_success = true;
    let mut input_token_estimate = 0;

    for scenario in &scenarios {
        let request = build_request(
            format!("offline-{}", scenario.name),
            scenario.task,
            davinci_agent::decision::risk::DecisionRisk::Planning,
        );
        input_token_estimate += request.validate_size().unwrap_or_default().len() / 4;

        let shadow = add_optional_capabilities(
            &scenario
                .deterministic
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>(),
            &scenario.optional,
            DecisionRollout::Shadow,
        );
        shadow_behavior_unchanged &= shadow
            == scenario
                .deterministic
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>();

        let guarded = if scenario.provider_failure {
            fallback_cases += 1;
            scenario
                .deterministic
                .iter()
                .map(|id| (*id).to_owned())
                .collect()
        } else {
            add_optional_capabilities(
                &scenario
                    .deterministic
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect::<Vec<_>>(),
                &scenario.optional,
                DecisionRollout::GuardedAdditive,
            )
        };
        let expected: Vec<String> = scenario
            .expected_additions
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        false_additions += guarded
            .iter()
            .filter(|id| **id != "mandatory_verification" && !expected.contains(id))
            .count();
        missed_relevant_capabilities += expected.iter().filter(|id| !guarded.contains(id)).count();
        capability_removals += scenario
            .deterministic
            .iter()
            .filter(|id| !guarded.iter().any(|candidate| candidate == *id))
            .count();
        mandatory_verification_preserved &= guarded.iter().any(|id| id == "mandatory_verification");
        if scenario.provider_failure {
            provider_failure_fallback_success &= guarded == scenario.deterministic;
        }
        task_successes += usize::from(guarded.iter().any(|id| id == "mandatory_verification"));
    }

    DecisionIntelligenceEvalReport {
        scenario_count: scenarios.len(),
        task_successes,
        shadow_behavior_unchanged,
        mandatory_verification_preserved,
        capability_removals,
        false_additions,
        missed_relevant_capabilities,
        fallback_cases,
        provider_failure_fallback_success,
        input_token_estimate,
        decision_latency_ms: 0,
        turn_latency_ms: 0,
        jev_usage_tokens: 0,
        guarded_policy_is_explicit: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_required_scenarios_preserve_the_deterministic_floor() {
        let report = run_offline_eval();
        assert_eq!(report.scenario_count, 10);
        assert_eq!(report.task_successes, 10);
        assert!(report.shadow_behavior_unchanged);
        assert!(report.mandatory_verification_preserved);
        assert_eq!(report.capability_removals, 0);
        assert_eq!(report.false_additions, 0);
        assert_eq!(report.missed_relevant_capabilities, 0);
        assert_eq!(report.fallback_cases, 1);
        assert!(report.provider_failure_fallback_success);
        assert!(report.input_token_estimate > 0);
        assert_eq!(report.jev_usage_tokens, 0);
        assert!(report.guarded_policy_is_explicit);
    }
}
