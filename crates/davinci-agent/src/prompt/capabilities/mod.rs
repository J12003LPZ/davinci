//! Native conditional behavioral capabilities: FrontendDesign, Debugging, CodeReview.

pub mod debugging;
pub mod frontend;
pub mod gate;
pub mod review;
pub mod router;
pub mod state;

use crate::prompt::composer::PromptModule;
use serde::{Deserialize, Serialize};

pub use debugging::{debugging_module, DEBUGGING_POLICY};
pub use frontend::{frontend_design_module, FRONTEND_DESIGN_POLICY};
pub use review::{code_review_module, CODE_REVIEW_POLICY};
pub use router::{
    detect_native_capabilities, route_capabilities, CapabilityEvidence, CapabilityEvidenceKind,
    CapabilityRouterInput,
};
pub use state::{CapabilityRunState, DebuggingState, FrontendDesignState, ReviewState};

pub const CAPABILITY_POLICY_MAX_TOKENS: usize = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NativeBehaviorCapability {
    FrontendDesign,
    Debugging,
    CodeReview,
}

impl NativeBehaviorCapability {
    pub fn id(self) -> &'static str {
        match self {
            Self::FrontendDesign => "capability.frontend-design",
            Self::Debugging => "capability.debugging",
            Self::CodeReview => "capability.code-review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDecision {
    pub capabilities: Vec<NativeBehaviorCapability>,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<CapabilityEvidence>,
}

impl CapabilityDecision {
    pub fn is_active(&self, cap: NativeBehaviorCapability) -> bool {
        self.capabilities.contains(&cap)
    }

    pub fn rule_ids(&self) -> Vec<&str> {
        self.evidence.iter().map(|e| e.rule_id.as_str()).collect()
    }
}

pub fn capability_module(cap: NativeBehaviorCapability) -> PromptModule {
    match cap {
        NativeBehaviorCapability::FrontendDesign => frontend_design_module(),
        NativeBehaviorCapability::Debugging => debugging_module(),
        NativeBehaviorCapability::CodeReview => code_review_module(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::manifest::estimate_tokens_from_str;

    #[test]
    fn frontend_design_triggers_on_visual_redesign() {
        let d = detect_native_capabilities(
            "Redesign this dashboard so it feels premium and intentional.",
        );
        assert!(d
            .capabilities
            .contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn frontend_design_does_not_trigger_for_react_bugfix() {
        let d = detect_native_capabilities("Fix the hydration error in Dashboard.tsx.");
        assert!(!d
            .capabilities
            .contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn frontend_design_does_not_trigger_for_component_rename_or_css_bug() {
        let d1 = detect_native_capabilities(
            "Rename the component Button to ActionButton in Button.tsx.",
        );
        assert!(!d1
            .capabilities
            .contains(&NativeBehaviorCapability::FrontendDesign));

        let d2 = detect_native_capabilities(
            "Fix the css bug causing text overflow in Sidebar.module.css.",
        );
        assert!(!d2
            .capabilities
            .contains(&NativeBehaviorCapability::FrontendDesign));

        let d3 = detect_native_capabilities("Backend endpoint returning HTML for health check.");
        assert!(!d3
            .capabilities
            .contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn debugging_triggers_on_diagnostic_intent() {
        let d = detect_native_capabilities("Diagnose and find the root cause of the memory spike.");
        assert!(d
            .capabilities
            .contains(&NativeBehaviorCapability::Debugging));
    }

    #[test]
    fn code_review_triggers_on_audit_intent() {
        let d = detect_native_capabilities(
            "Perform a code review of this PR focusing on security risks.",
        );
        assert!(d
            .capabilities
            .contains(&NativeBehaviorCapability::CodeReview));
    }

    #[test]
    fn capability_policies_stay_within_budget() {
        let caps = [
            NativeBehaviorCapability::FrontendDesign,
            NativeBehaviorCapability::Debugging,
            NativeBehaviorCapability::CodeReview,
        ];
        for cap in caps {
            let m = capability_module(cap);
            let tokens = estimate_tokens_from_str(&m.body);
            assert!(
                tokens <= CAPABILITY_POLICY_MAX_TOKENS,
                "Capability {:?} exceeds token budget: {}",
                cap,
                tokens
            );
        }
    }

    #[test]
    fn routing_fixtures_meet_accuracy_threshold() {
        let fe_json =
            include_str!("../../../../davinci-evals/fixtures/behavior/frontend/routing.json");
        let fe_cases: Vec<serde_json::Value> = serde_json::from_str(fe_json).unwrap();
        assert!(fe_cases.len() >= 30);

        let mut correct = 0;
        let total = fe_cases.len();

        for case in &fe_cases {
            let req = case["request"].as_str().unwrap();
            let prev = case.get("previous_request").and_then(|v| v.as_str());
            let expect_fe = case["expect_frontend"].as_bool().unwrap();
            let input = CapabilityRouterInput::new(req).with_previous_request(prev);
            let decision = route_capabilities(&input);
            let has_fe = decision
                .capabilities
                .contains(&NativeBehaviorCapability::FrontendDesign);
            if has_fe == expect_fe {
                correct += 1;
            }
        }

        let accuracy = (correct as f64) / (total as f64);
        assert!(
            accuracy >= 0.95,
            "Frontend routing accuracy {} below 95% threshold ({}/{})",
            accuracy,
            correct,
            total
        );
    }

    #[test]
    fn capability_router_frontend_precision_and_recall() {
        let json_str =
            include_str!("../../../../davinci-evals/fixtures/behavior/frontend/routing.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            cases.len(),
            80,
            "Expected exactly 80 frontend routing cases"
        );

        let mut tp = 0;
        let mut fp = 0;
        let mut tn = 0;
        let mut fn_count = 0;

        for case in &cases {
            let id = case["id"].as_str().unwrap();
            let req = case["request"].as_str().unwrap();
            let prev = case.get("previous_request").and_then(|v| v.as_str());
            let expect = case["expect_frontend"].as_bool().unwrap();

            let input = CapabilityRouterInput::new(req).with_previous_request(prev);
            let decision = route_capabilities(&input);
            let actual = decision.is_active(NativeBehaviorCapability::FrontendDesign);

            match (expect, actual) {
                (true, true) => tp += 1,
                (false, true) => {
                    fp += 1;
                    eprintln!("Frontend FP on case {}: {:?}", id, req);
                }
                (false, false) => tn += 1,
                (true, false) => {
                    fn_count += 1;
                    eprintln!("Frontend FN on case {}: {:?}", id, req);
                }
            }
        }

        let precision = tp as f64 / (tp + fp) as f64;
        let recall = tp as f64 / (tp + fn_count) as f64;

        eprintln!(
            "Frontend routing metrics: TP={}, FP={}, TN={}, FN={}, Precision={:.3}, Recall={:.3}",
            tp, fp, tn, fn_count, precision, recall
        );

        assert!(
            precision >= 0.97,
            "Frontend precision {:.3} below 97% requirement",
            precision
        );
        assert!(
            recall >= 0.94,
            "Frontend recall {:.3} below 94% requirement",
            recall
        );
    }

    #[test]
    fn capability_router_debugging_precision_and_recall() {
        let json_str =
            include_str!("../../../../davinci-evals/fixtures/behavior/debugging/routing.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            cases.len(),
            80,
            "Expected exactly 80 debugging routing cases"
        );

        let mut tp = 0;
        let mut fp = 0;
        let mut tn = 0;
        let mut fn_count = 0;

        for case in &cases {
            let id = case["id"].as_str().unwrap();
            let req = case["request"].as_str().unwrap();
            let prev = case.get("previous_request").and_then(|v| v.as_str());
            let tools_vec: Vec<String> = case
                .get("recent_tools")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let expect = case["expect_debugging"].as_bool().unwrap();

            let input = CapabilityRouterInput::new(req)
                .with_previous_request(prev)
                .with_recent_tools(&tools_vec);
            let decision = route_capabilities(&input);
            let actual = decision.is_active(NativeBehaviorCapability::Debugging);

            match (expect, actual) {
                (true, true) => tp += 1,
                (false, true) => {
                    fp += 1;
                    eprintln!("Debugging FP on case {}: {:?}", id, req);
                }
                (false, false) => tn += 1,
                (true, false) => {
                    fn_count += 1;
                    eprintln!("Debugging FN on case {}: {:?}", id, req);
                }
            }
        }

        let precision = tp as f64 / (tp + fp) as f64;
        let recall = tp as f64 / (tp + fn_count) as f64;

        eprintln!(
            "Debugging routing metrics: TP={}, FP={}, TN={}, FN={}, Precision={:.3}, Recall={:.3}",
            tp, fp, tn, fn_count, precision, recall
        );

        assert!(
            precision >= 0.97,
            "Debugging precision {:.3} below 97% requirement",
            precision
        );
        assert!(
            recall >= 0.94,
            "Debugging recall {:.3} below 94% requirement",
            recall
        );
    }

    #[test]
    fn capability_router_review_precision_and_recall() {
        let json_str =
            include_str!("../../../../davinci-evals/fixtures/behavior/review/routing.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(cases.len(), 80, "Expected exactly 80 review routing cases");

        let mut tp = 0;
        let mut fp = 0;
        let mut tn = 0;
        let mut fn_count = 0;

        for case in &cases {
            let id = case["id"].as_str().unwrap();
            let req = case["request"].as_str().unwrap();
            let prev = case.get("previous_request").and_then(|v| v.as_str());
            let uncommitted = case
                .get("has_uncommitted_changes")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let expect = case["expect_review"].as_bool().unwrap();

            let input = CapabilityRouterInput::new(req)
                .with_previous_request(prev)
                .with_uncommitted_changes(uncommitted);
            let decision = route_capabilities(&input);
            let actual = decision.is_active(NativeBehaviorCapability::CodeReview);

            match (expect, actual) {
                (true, true) => tp += 1,
                (false, true) => {
                    fp += 1;
                    eprintln!("Review FP on case {}: {:?}", id, req);
                }
                (false, false) => tn += 1,
                (true, false) => {
                    fn_count += 1;
                    eprintln!("Review FN on case {}: {:?}", id, req);
                }
            }
        }

        let precision = tp as f64 / (tp + fp) as f64;
        let recall = tp as f64 / (tp + fn_count) as f64;

        eprintln!(
            "Review routing metrics: TP={}, FP={}, TN={}, FN={}, Precision={:.3}, Recall={:.3}",
            tp, fp, tn, fn_count, precision, recall
        );

        assert!(
            precision >= 0.97,
            "Review precision {:.3} below 97% requirement",
            precision
        );
        assert!(
            recall >= 0.94,
            "Review recall {:.3} below 94% requirement",
            recall
        );
    }

    #[test]
    fn capability_router_evidence_and_rule_ids_recorded() {
        let decision = detect_native_capabilities("Diagnose and find root cause of crash");
        assert!(decision.is_active(NativeBehaviorCapability::Debugging));
        assert!(!decision.evidence.is_empty());
        let rule_ids = decision.rule_ids();
        assert!(
            rule_ids.contains(&"dbg.explicit.diagnose")
                || rule_ids.contains(&"dbg.explicit.root_cause"),
            "Expected diagnostic rule IDs, got: {:?}",
            rule_ids
        );
        for reason in &decision.reasons {
            assert!(
                reason.starts_with("capability.debugging: [dbg."),
                "Reason should log capability and rule ID without chain-of-thought: {}",
                reason
            );
        }
    }

    #[test]
    fn capability_router_negative_signals_veto_activation() {
        // Adversarial debug request: asks to add a CLI flag to print debug logs
        let d1 = detect_native_capabilities("Add a new CLI flag --verbose to print debug logs.");
        assert!(
            !d1.is_active(NativeBehaviorCapability::Debugging),
            "Feature request with 'debug' word should not activate Debugging capability"
        );

        // Adversarial review request: asks to fix review comments
        let d2 = detect_native_capabilities("Fix the review comments from the latest PR.");
        assert!(
            !d2.is_active(NativeBehaviorCapability::CodeReview),
            "Fixing review comments should not activate CodeReview capability"
        );

        // Adversarial frontend request: asks to fix CSS overflow bug
        let d3 = detect_native_capabilities(
            "Fix the css bug causing text overflow in Sidebar.module.css.",
        );
        assert!(
            !d3.is_active(NativeBehaviorCapability::FrontendDesign),
            "Fixing CSS bug should not activate FrontendDesign capability"
        );
    }
}
