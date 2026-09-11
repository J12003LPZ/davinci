//! Code review capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const CODE_REVIEW_POLICY: &str = "\
<code_review_policy>
When performing code reviews or auditing changes:
1. Focus primarily on correctness, security vulnerabilities, regression risks, concurrency safety, and performance pitfalls.
2. Inspect the requested change set and relevant surrounding context thoroughly.
3. Provide concrete, actionable feedback referencing specific file paths and line numbers.
4. Distinguish critical correctness or security blockers from optional suggestions.
5. Avoid pedantic style debates or speculative rewrites unless they violate established project conventions.
</code_review_policy>";

pub fn code_review_module() -> PromptModule {
    PromptModule {
        id: NativeBehaviorCapability::CodeReview.id().to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: CODE_REVIEW_POLICY.to_string(),
    }
}
