//! Native capability intent detection and router.

use super::{CapabilityDecision, NativeBehaviorCapability};

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
