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
pub use router::detect_native_capabilities;

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
            let expect_fe = case["expect_frontend"].as_bool().unwrap();
            let decision = detect_native_capabilities(req);
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
}
