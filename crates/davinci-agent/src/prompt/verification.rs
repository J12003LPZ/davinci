//! Verification prompt module: evidence-before-completion and stopping criteria.

use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub fn verification_completion_module() -> PromptModule {
    PromptModule {
        id: "verification.completion".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
After changing code, run the smallest meaningful verification first, then broader checks \
when risk warrants them. If a check fails, investigate the failure. Do not explain it away. \
Do not claim a check passed unless you ran it. Do not claim a test, build, lint, or behavior \
passed unless fresh evidence from this run supports it. Stop when the requested outcome is \
complete and verified. Do not continue polishing unrelated areas merely because tools and \
context remain available."
            .to_string(),
    }
}

/// Candidate-only requirement classifier and no-suite verification guidance.
pub fn verification_requirement_checks_module() -> PromptModule {
    PromptModule {
        id: "verification.requirement-checks".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
Before finishing, compare every explicit requirement with a concrete input or observable \
condition. If the project has no test suite, verify with a short `python -c` or heredoc script \
that imports the changed module and asserts on normal, boundary, and invalid inputs. Use the \
same normal/boundary/invalid classifier for every route. Report the actual verification briefly \
and leave any unverified condition explicit."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_policy_forbids_unverified_success_claims() {
        let text = verification_completion_module().body;
        assert!(text.contains("Do not claim a check passed unless you ran it"));
    }

    #[test]
    fn candidate_checks_define_the_no_suite_classifier() {
        let text = verification_requirement_checks_module().body;
        assert!(text.contains("python -c"));
        assert!(text.contains("heredoc script"));
        assert!(text.contains("normal, boundary, and invalid inputs"));
        assert!(text.contains("same normal/boundary/invalid classifier for every route"));
    }
}
