//! Coding behavior prompt modules: exploration, scope discipline, change quality.

use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub fn coding_exploration_module() -> PromptModule {
    PromptModule {
        id: "coding.exploration".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
Understand before editing. Read and understand relevant code, tests, project instructions, \
and nearby conventions before changing behavior. Search for definitions and call sites rather \
than guessing from names. Do not invent behavior for files you have not inspected. \
Read and understand relevant code before editing it."
            .to_string(),
    }
}

pub fn coding_scope_discipline_module() -> PromptModule {
    PromptModule {
        id: "coding.scope-discipline".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
Make the minimum coherent change that fully satisfies the request. Do not add speculative \
features, future-proofing, compatibility shims, helper layers, or unrelated refactors. \
Do not refactor unrelated code. Reuse existing abstractions when they already fit; create a \
new abstraction only when the current task genuinely needs one."
            .to_string(),
    }
}

pub fn coding_change_quality_module() -> PromptModule {
    PromptModule {
        id: "coding.change-quality".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
Preserve user-authored changes. Treat unexpected modifications as potentially intentional. \
Do not revert, overwrite, or \"clean up\" unrelated work to make your patch easier. \
Match the prevailing style, error-handling conventions, and architecture of the codebase."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploration_policy_requires_reading_relevant_code_before_editing() {
        let text = coding_exploration_module().body;
        assert!(text.contains("Read and understand relevant code before editing it."));
        assert!(text.contains("Do not invent behavior for files you have not inspected."));
    }

    #[test]
    fn scope_policy_rejects_unrequested_cleanup() {
        let text = coding_scope_discipline_module().body;
        assert!(text.contains("Do not refactor unrelated code"));
        assert!(text.contains("minimum coherent change"));
    }
}
