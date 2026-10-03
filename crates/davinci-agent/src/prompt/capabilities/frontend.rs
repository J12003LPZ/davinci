//! Frontend design capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const FRONTEND_DESIGN_POLICY: &str = "\
<frontend_design_policy>
When crafting a frontend interface or visual experience, follow this two-pass process:
1. Ground the work in the product's subject, audience, and primary job; infer context from the repository before asking a design question.
2. Inspect existing components, tokens, styles, typography, color, spacing, and responsive conventions before adding visual language.
3. Before coding, write a compact direction covering color intent, typography without assuming a specific font, composition and hierarchy, and one signature element.
4. Critique the first pass: could this belong to any generic SaaS? Avoid default card-and-grid composition, placeholder copy, undifferentiated panels, and decorative clutter. Keep one memorable element with supporting restraint.
5. Implement the direction, then when a browser or screenshot backend exists, visually inspect the rendered result for hierarchy, spacing, contrast, alignment, responsive behavior, and interaction states.
6. Revise the highest-impact weakness found by inspection. Preserve accessibility through semantic or ARIA structure, keyboard access, visible focus, and contrast; support mobile usability and restrained motion with reduced-motion behavior.
7. Do not activate visual redesign for simple bug fixes, renames, or behavior-only changes.
Non-goals: do not choose a specific font by default; do not force a dark or light theme; do not forbid cards universally; do not force animation; do not ask a design question when product context can be inferred from the repository.
</frontend_design_policy>";

pub fn frontend_design_module() -> PromptModule {
    PromptModule {
        id: NativeBehaviorCapability::FrontendDesign.id().to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: FRONTEND_DESIGN_POLICY.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{frontend_design_module, FRONTEND_DESIGN_POLICY};

    #[test]
    fn frontend_design_module_contains_semantic_invariants() {
        let policy = FRONTEND_DESIGN_POLICY.to_ascii_lowercase();
        let required_concepts = [
            "two-pass",
            "product",
            "subject",
            "audience",
            "primary job",
            "before coding",
            "color",
            "typography",
            "composition",
            "signature element",
            "generic saas",
            "visual",
            "inspect",
            "revise",
            "mobile",
            "restrained motion",
            "reduced-motion",
            "accessibility",
            "card-and-grid",
        ];

        for concept in required_concepts {
            assert!(
                policy.contains(concept),
                "frontend design policy is missing semantic concept: {concept}"
            );
        }

        assert!(
            frontend_design_module().body.len().div_ceil(4) <= 900,
            "frontend design policy exceeds the estimated 900-token budget"
        );
    }

    #[test]
    fn frontend_design_module_states_non_goals_without_overconstraining_design() {
        let policy = FRONTEND_DESIGN_POLICY.to_ascii_lowercase();

        for non_goal in [
            "specific font",
            "dark or light theme",
            "do not forbid cards universally",
            "do not force animation",
            "do not ask a design question",
        ] {
            assert!(
                policy.contains(non_goal),
                "frontend design policy is missing non-goal: {non_goal}"
            );
        }

        for overconstraint in [
            "always use dark",
            "always use light",
            "never use cards",
            "must animate",
        ] {
            assert!(
                !policy.contains(overconstraint),
                "frontend design policy contains overconstraint: {overconstraint}"
            );
        }
    }
}
