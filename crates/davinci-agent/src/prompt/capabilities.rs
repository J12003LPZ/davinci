//! Native conditional behavioral capabilities: FrontendDesign, Debugging, CodeReview.

use crate::prompt::composer::{PromptCacheClass, PromptModule};
use serde::{Deserialize, Serialize};

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

const FRONTEND_DESIGN_POLICY: &str = "\
<frontend_design_policy>
When crafting frontend user interfaces and visual experiences:
1. Ground visual choices in the product's identity, target audience, and intended emotional tone.
2. Inspect existing design systems, typography, color palettes, spacing conventions, and CSS tokens before introducing new styles.
3. Establish a clear visual hierarchy with intentional contrast, harmonious proportions, and deliberate typography scales.
4. Avoid generic placeholder UI patterns, monotonous gray-on-gray palettes, or artificial clutter. Strive for polished, distinctive interfaces with character.
5. Ensure responsive design across viewport sizes, robust accessibility (proper ARIA semantics, keyboard navigation, visible focus indicators, color contrast), and fluid transitions.
6. When browser or screenshot tools are available, visually inspect rendered output to verify spacing, contrast, and alignment before completing the task.
7. Do not activate visual redesigns for simple bug fixes, component renames, or behavioral logic changes.
</frontend_design_policy>";

const DEBUGGING_POLICY: &str = "\
<debugging_policy>
When investigating and resolving bugs or unexpected behavior:
1. Establish reproducible evidence: review error logs, failing tests, or stack traces first.
2. Read and inspect the causal code paths before proposing modifications. Trace state, inputs, and boundaries.
3. Isolate the root cause rather than patching over symptoms or suppressing errors with workarounds.
4. Make the minimal targeted correction to the root cause path, avoiding speculative changes to unaffected logic.
5. Verify the fix by running the reproduction check or relevant test suite. Ensure the original error is resolved and no regressions were introduced.
</debugging_policy>";

const CODE_REVIEW_POLICY: &str = "\
<code_review_policy>
When performing code reviews or auditing changes:
1. Focus primarily on correctness, security vulnerabilities, regression risks, concurrency safety, and performance pitfalls.
2. Inspect the requested change set and relevant surrounding context thoroughly.
3. Provide concrete, actionable feedback referencing specific file paths and line numbers.
4. Distinguish critical correctness or security blockers from optional suggestions.
5. Avoid pedantic style debates or speculative rewrites unless they violate established project conventions.
</code_review_policy>";

pub fn detect_native_capabilities(request: &str) -> CapabilityDecision {
    let lower = request.to_lowercase();
    let mut capabilities = Vec::new();
    let mut reasons = Vec::new();

    // 1. Frontend design detection:
    // Requires clear visual design, redesign, look-and-feel, or aesthetic styling intent.
    let is_visual_design = lower.contains("redesign")
        || lower.contains("look and feel")
        || lower.contains("look-and-feel")
        || lower.contains("aesthetic")
        || lower.contains("visual design")
        || lower.contains("make it look")
        || lower.contains("polish the ui")
        || lower.contains("modernize the design")
        || lower.contains("create a design system")
        || lower.contains("overhaul the styling")
        || lower.contains("ui overhaul")
        || lower.contains("theme overhaul")
        || lower.contains("landing page design")
        || lower.contains("hero section design")
        || lower.contains("feels premium and intentional");

    let is_simple_bugfix = lower.contains("fix hydration")
        || lower.contains("hydration error")
        || lower.contains("fix the css bug")
        || lower.contains("fix the overflow bug")
        || lower.contains("rename component")
        || lower.contains("rename the component")
        || lower.contains("unit test for")
        || lower.contains("endpoint returning html")
        || lower.contains("api returning html");

    if is_visual_design && !is_simple_bugfix {
        capabilities.push(NativeBehaviorCapability::FrontendDesign);
        reasons.push("frontend-design: matched visual redesign or aesthetic intent".to_string());
    }

    // 2. Debugging detection:
    let is_debugging = lower.contains("debug")
        || lower.contains("diagnose")
        || lower.contains("root cause")
        || lower.contains("investigate the failure")
        || lower.contains("trace the bug")
        || lower.contains("why does it crash")
        || lower.contains("fix the failing test")
        || lower.contains("troubleshoot");

    if is_debugging {
        capabilities.push(NativeBehaviorCapability::Debugging);
        reasons.push("debugging: matched diagnostic or bug investigation intent".to_string());
    }

    // 3. Code review detection:
    let is_review = lower.contains("code review")
        || lower.contains("review this code")
        || lower.contains("review the pr")
        || lower.contains("review my pr")
        || lower.contains("audit this change")
        || lower.contains("review the diff")
        || lower.contains("inspect recent changes for issues");

    if is_review {
        capabilities.push(NativeBehaviorCapability::CodeReview);
        reasons.push("code-review: matched code review or audit intent".to_string());
    }

    CapabilityDecision {
        capabilities,
        reasons,
    }
}

pub fn capability_module(cap: NativeBehaviorCapability) -> PromptModule {
    match cap {
        NativeBehaviorCapability::FrontendDesign => PromptModule {
            id: cap.id().to_string(),
            version: 1,
            cache_class: PromptCacheClass::Dynamic,
            body: FRONTEND_DESIGN_POLICY.to_string(),
        },
        NativeBehaviorCapability::Debugging => PromptModule {
            id: cap.id().to_string(),
            version: 1,
            cache_class: PromptCacheClass::Dynamic,
            body: DEBUGGING_POLICY.to_string(),
        },
        NativeBehaviorCapability::CodeReview => PromptModule {
            id: cap.id().to_string(),
            version: 1,
            cache_class: PromptCacheClass::Dynamic,
            body: CODE_REVIEW_POLICY.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::manifest::estimate_tokens_from_str;

    #[test]
    fn frontend_design_triggers_on_visual_redesign() {
        let d = detect_native_capabilities("Redesign this dashboard so it feels premium and intentional.");
        assert!(d.capabilities.contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn frontend_design_does_not_trigger_for_react_bugfix() {
        let d = detect_native_capabilities("Fix the hydration error in Dashboard.tsx.");
        assert!(!d.capabilities.contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn frontend_design_does_not_trigger_for_component_rename_or_css_bug() {
        let d1 = detect_native_capabilities("Rename the component Button to ActionButton in Button.tsx.");
        assert!(!d1.capabilities.contains(&NativeBehaviorCapability::FrontendDesign));

        let d2 = detect_native_capabilities("Fix the css bug causing text overflow in Sidebar.module.css.");
        assert!(!d2.capabilities.contains(&NativeBehaviorCapability::FrontendDesign));

        let d3 = detect_native_capabilities("Backend endpoint returning HTML for health check.");
        assert!(!d3.capabilities.contains(&NativeBehaviorCapability::FrontendDesign));
    }

    #[test]
    fn debugging_triggers_on_diagnostic_intent() {
        let d = detect_native_capabilities("Diagnose and find the root cause of the memory spike.");
        assert!(d.capabilities.contains(&NativeBehaviorCapability::Debugging));
    }

    #[test]
    fn code_review_triggers_on_audit_intent() {
        let d = detect_native_capabilities("Perform a code review of this PR focusing on security risks.");
        assert!(d.capabilities.contains(&NativeBehaviorCapability::CodeReview));
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
        let fe_json = include_str!("../../../davinci-evals/fixtures/behavior/frontend/routing.json");
        let fe_cases: Vec<serde_json::Value> = serde_json::from_str(fe_json).unwrap();
        assert!(fe_cases.len() >= 30);

        let mut correct = 0;
        let total = fe_cases.len();

        for case in &fe_cases {
            let req = case["request"].as_str().unwrap();
            let expect_fe = case["expect_frontend"].as_bool().unwrap();
            let decision = detect_native_capabilities(req);
            let has_fe = decision.capabilities.contains(&NativeBehaviorCapability::FrontendDesign);
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
