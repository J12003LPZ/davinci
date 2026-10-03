//! Debugging capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const DEBUGGING_POLICY: &str = "\
<debugging_policy>
When investigating or resolving a bug, follow this evidence chain:
1. Establish the failure signal.
   Start with logs, tests, traces, or a clear report.
2. Reproduce or obtain concrete evidence.
   Do this before changing code.
3. Locate the narrow causal path.
   Trace relevant state, inputs, and boundaries.
4. Form a testable hypothesis.
   It must explain the same failing signal.
5. Change the minimal causal code.
   Avoid speculative edits and symptom suppression.
6. Re-run the original reproducer.
   Verify the same failing signal is gone.
7. Run adjacent regression checks based on risk.
   Report exactly what passed.
If a missing dependency or environment prevents reproduction, record the blocker and label the result \"not reproduced\"; never claim fixed. State that the fix is unverified until the original reproducer runs.
</debugging_policy>";

pub fn debugging_module() -> PromptModule {
    PromptModule {
        id: NativeBehaviorCapability::Debugging.id().to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: DEBUGGING_POLICY.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugging_policy_keeps_reproduce_first_and_minimal_edit_order() {
        let stages = [
            "1. Establish the failure signal.",
            "2. Reproduce or obtain concrete evidence.",
            "3. Locate the narrow causal path.",
            "4. Form a testable hypothesis.",
            "5. Change the minimal causal code.",
            "6. Re-run the original reproducer.",
            "7. Run adjacent regression checks based on risk.",
        ];
        let positions = stages
            .iter()
            .map(|stage| {
                DEBUGGING_POLICY
                    .find(stage)
                    .expect("missing debugging stage")
            })
            .collect::<Vec<_>>();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn debugging_policy_requires_same_signal_verification() {
        for concept in [
            "same failing signal",
            "original reproducer",
            "minimal causal code",
        ] {
            assert!(
                DEBUGGING_POLICY.contains(concept),
                "debugging policy is missing invariant: {concept}"
            );
        }
    }

    #[test]
    fn debugging_policy_distinguishes_missing_dependency_from_a_confirmed_fix() {
        for concept in [
            "missing dependency",
            "not reproduced",
            "never claim fixed",
            "fix is unverified",
        ] {
            assert!(
                DEBUGGING_POLICY.contains(concept),
                "debugging policy is missing exception rule: {concept}"
            );
        }
    }

    #[test]
    fn debugging_policy_stays_within_budget() {
        assert!(
            DEBUGGING_POLICY.len().div_ceil(4) <= 900,
            "debugging policy exceeds the estimated 900-token budget"
        );
    }
}
