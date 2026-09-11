//! Debugging capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const DEBUGGING_POLICY: &str = "\
<debugging_policy>
When investigating and resolving bugs or unexpected behavior:
1. Establish reproducible evidence: review error logs, failing tests, or stack traces first.
2. Read and inspect the causal code paths before proposing modifications. Trace state, inputs, and boundaries.
3. Isolate the root cause rather than patching over symptoms or suppressing errors with workarounds.
4. Make the minimal targeted correction to the root cause path, avoiding speculative changes to unaffected logic.
5. Verify the fix by running the reproduction check or relevant test suite. Ensure the original error is resolved and no regressions were introduced.
</debugging_policy>";

pub fn debugging_module() -> PromptModule {
    PromptModule {
        id: NativeBehaviorCapability::Debugging.id().to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: DEBUGGING_POLICY.to_string(),
    }
}
