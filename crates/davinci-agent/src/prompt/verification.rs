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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_policy_forbids_unverified_success_claims() {
        let text = verification_completion_module().body;
        assert!(text.contains("Do not claim a check passed unless you ran it"));
    }
}
