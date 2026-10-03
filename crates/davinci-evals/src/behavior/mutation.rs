//! Prompt mutation and ablation testing harness.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::runner::BehaviorSuiteSummary;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AblationResult {
    pub ablated_module_id: String,
    pub baseline_pass_rate: f64,
    pub ablated_pass_rate: f64,
    pub pass_rate_delta: f64,
    pub baseline_unverified_claims: f64,
    pub ablated_unverified_claims: f64,
    pub unverified_claims_delta: f64,
    pub baseline_unrelated_edits: f64,
    pub ablated_unrelated_edits: f64,
    pub unrelated_edits_delta: f64,
    pub degradation_observed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeadPromptAudit {
    pub flagged_modules: Vec<String>,
    pub reviewed_modules: Vec<String>,
    pub details: BTreeMap<String, String>,
}

pub fn evaluate_ablation(
    ablated_module_id: &str,
    baseline: &BehaviorSuiteSummary,
    ablated: &BehaviorSuiteSummary,
) -> AblationResult {
    let pass_rate_delta = ablated.macro_pass_rate - baseline.macro_pass_rate;
    let unverified_claims_delta = ablated.unverified_claim_rate - baseline.unverified_claim_rate;
    let unrelated_edits_delta = ablated.unrelated_edit_rate - baseline.unrelated_edit_rate;

    // Degradation occurs if pass rate drops or bad behaviors increase
    let degradation_observed =
        pass_rate_delta < -0.01 || unverified_claims_delta > 0.01 || unrelated_edits_delta > 0.01;

    AblationResult {
        ablated_module_id: ablated_module_id.to_string(),
        baseline_pass_rate: baseline.macro_pass_rate,
        ablated_pass_rate: ablated.macro_pass_rate,
        pass_rate_delta,
        baseline_unverified_claims: baseline.unverified_claim_rate,
        ablated_unverified_claims: ablated.unverified_claim_rate,
        unverified_claims_delta,
        baseline_unrelated_edits: baseline.unrelated_edit_rate,
        ablated_unrelated_edits: ablated.unrelated_edit_rate,
        unrelated_edits_delta,
        degradation_observed,
    }
}

pub fn audit_dead_prompt_modules(
    results: &[AblationResult],
    protected_safety_modules: &[&str],
) -> DeadPromptAudit {
    let mut flagged = Vec::new();
    let mut reviewed = Vec::new();
    let mut details = BTreeMap::new();

    for r in results {
        reviewed.push(r.ablated_module_id.clone());
        if protected_safety_modules.contains(&r.ablated_module_id.as_str()) {
            details.insert(
                r.ablated_module_id.clone(),
                "Retained: protected safety/compatibility requirement".into(),
            );
            continue;
        }

        if !r.degradation_observed {
            flagged.push(r.ablated_module_id.clone());
            details.insert(
                r.ablated_module_id.clone(),
                format!(
                    "Review recommended: removal produced no degradation (delta pass={:.1}%, unverified={:.1}%, unrelated={:.1}%)",
                    r.pass_rate_delta * 100.0,
                    r.unverified_claims_delta * 100.0,
                    r.unrelated_edits_delta * 100.0
                ),
            );
        } else {
            details.insert(
                r.ablated_module_id.clone(),
                "Active: removing module causes measurable behavioral degradation".into(),
            );
        }
    }

    DeadPromptAudit {
        flagged_modules: flagged,
        reviewed_modules: reviewed,
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::prompt::{
        compose_default_prompt, compose_with_mutations, PromptContext, PromptMutation,
    };
    use davinci_agent::PermissionMode;

    fn test_context() -> PromptContext<'static> {
        PromptContext {
            provider: "anthropic",
            model_id: "claude-3-7-sonnet",
            permission_mode: PermissionMode::Ask,
            plan_active: false,
        }
    }

    #[test]
    fn removing_module_removes_exactly_that_module_and_changes_hash() {
        let ctx = test_context();
        let baseline = compose_default_prompt(&ctx);

        assert!(baseline
            .manifest
            .modules
            .iter()
            .any(|m| m.id == "verification.completion"));

        let ablated = compose_with_mutations(
            &ctx,
            &[PromptMutation::RemoveModule {
                id: "verification.completion".into(),
            }],
        );

        assert!(!ablated
            .manifest
            .modules
            .iter()
            .any(|m| m.id == "verification.completion"));

        // Stable hash changes
        assert_ne!(
            baseline.manifest.stable_sha256,
            ablated.manifest.stable_sha256
        );

        // Other modules remain present and unchanged
        let other_modules = [
            "core.identity",
            "core.autonomy",
            "coding.exploration",
            "coding.scope-discipline",
            "coding.change-quality",
            "collaboration.user-intent",
        ];
        for mod_id in &other_modules {
            assert!(ablated.manifest.modules.iter().any(|m| m.id == *mod_id));
        }

        // Production default_system_prompt is unaffected
        let prod_prompt = davinci_agent::default_system_prompt();
        assert!(!prod_prompt.is_empty());
    }

    #[test]
    fn replaces_and_downgrades_module_in_mutations() {
        let ctx = test_context();
        let mutated = compose_with_mutations(
            &ctx,
            &[
                PromptMutation::ReplaceModuleBody {
                    id: "coding.exploration".into(),
                    body: "custom exploration body".into(),
                },
                PromptMutation::DowngradeModuleVersion {
                    id: "coding.scope-discipline".into(),
                    version: 0,
                },
            ],
        );

        let exp = mutated
            .manifest
            .modules
            .iter()
            .find(|m| m.id == "coding.exploration")
            .unwrap();
        assert_eq!(
            exp.sha256,
            davinci_agent::prompt::manifest::hash_text("custom exploration body")
        );

        let scope = mutated
            .manifest
            .modules
            .iter()
            .find(|m| m.id == "coding.scope-discipline")
            .unwrap();
        assert_eq!(scope.version, 0);
    }

    #[test]
    fn dead_prompt_audit_flags_ineffective_modules_except_protected() {
        let baseline = BehaviorSuiteSummary {
            total_scenarios: 50,
            passed_scenarios: 45,
            macro_pass_rate: 0.90,
            category_pass_rates: BTreeMap::new(),
            median_model_turns: 4.0,
            p90_model_turns: 6.0,
            median_tool_calls: 8.0,
            unverified_claim_rate: 0.02,
            unrelated_edit_rate: 0.02,
            unnecessary_permission_prompt_rate: 0.0,
        };

        // Case 1: Useful module (ablation causes pass rate drop)
        let mut ablated_useful = baseline.clone();
        ablated_useful.macro_pass_rate = 0.70;
        let res_useful = evaluate_ablation("coding.exploration", &baseline, &ablated_useful);
        assert!(res_useful.degradation_observed);

        // Case 2: Ineffective module (no degradation)
        let ablated_dead = baseline.clone();
        let res_dead = evaluate_ablation("fluff.module", &baseline, &ablated_dead);
        assert!(!res_dead.degradation_observed);

        // Case 3: Ineffective module that is protected for safety
        let res_protected = evaluate_ablation("core.identity", &baseline, &ablated_dead);

        let audit = audit_dead_prompt_modules(
            &[res_useful, res_dead, res_protected],
            &["core.identity", "core.autonomy"],
        );

        assert_eq!(audit.flagged_modules, vec!["fluff.module"]);
        assert!(audit
            .details
            .get("core.identity")
            .unwrap()
            .contains("protected"));
        assert!(audit
            .details
            .get("fluff.module")
            .unwrap()
            .contains("Review recommended"));
    }
}
