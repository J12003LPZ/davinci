//! Code review capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewLens {
    Correctness,
    Tests,
    ErrorHandling,
    TypesAndInvariants,
    Security,
    CommentsAndDocs,
    Simplification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewSeverity {
    Blocker,
    Major,
    Minor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub lens: ReviewLens,
    pub severity: ReviewSeverity,
    pub confidence: u8,
    pub path: String,
    pub line: Option<u32>,
    pub title: String,
    pub evidence: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewChangeSignals {
    pub changed_paths: Vec<String>,
    pub changed_test_files: bool,
    pub changed_error_paths: bool,
    pub changed_public_types: bool,
    pub changed_security_sensitive_area: bool,
    pub changed_comments_docs: bool,
    pub diff_line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewExecutionPlan {
    Inline {
        lenses: Vec<ReviewLens>,
    },
    Parallel {
        lenses: Vec<ReviewLens>,
        max_workers: usize,
    },
}

/// Select the review execution shape without creating workers. Worker
/// creation remains the caller's responsibility and must use the existing
/// scoped `SubagentRunner` when parallel execution is selected.
pub fn plan_review_execution(lenses: &[ReviewLens], changed_lines: usize) -> ReviewExecutionPlan {
    let lenses = lenses.to_vec();
    if changed_lines <= 250 || lenses.len() <= 2 {
        ReviewExecutionPlan::Inline { lenses }
    } else {
        ReviewExecutionPlan::Parallel {
            lenses,
            max_workers: 4,
        }
    }
}

/// Select the smallest deterministic set of review lenses justified by a diff.
/// Correctness is always retained, and comprehensive reviews are capped at five
/// lenses so a noisy diff cannot create unbounded review work.
pub fn select_review_lenses(signals: &ReviewChangeSignals) -> Vec<ReviewLens> {
    let behavior_changed = signals.changed_paths.iter().any(|path| {
        let lower = path.to_ascii_lowercase();
        !lower.ends_with(".md")
            && !lower.ends_with(".txt")
            && !lower.ends_with(".toml")
            && !lower.ends_with(".lock")
            && !lower.ends_with(".yml")
            && !lower.ends_with(".yaml")
            && !lower.ends_with(".rst")
    });

    let mut lenses = vec![ReviewLens::Correctness];
    if behavior_changed || signals.changed_test_files {
        lenses.push(ReviewLens::Tests);
    }
    if signals.changed_error_paths {
        lenses.push(ReviewLens::ErrorHandling);
    }
    if signals.changed_public_types {
        lenses.push(ReviewLens::TypesAndInvariants);
    }
    if signals.changed_security_sensitive_area {
        lenses.push(ReviewLens::Security);
    }
    if signals.changed_comments_docs {
        lenses.push(ReviewLens::CommentsAndDocs);
    }
    if signals.diff_line_count > 250 {
        lenses.push(ReviewLens::Simplification);
    }
    lenses.truncate(5);
    lenses
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correctness_is_always_selected() {
        assert_eq!(
            select_review_lenses(&ReviewChangeSignals::default()),
            vec![ReviewLens::Correctness]
        );
    }

    #[test]
    fn behavior_and_test_changes_select_tests() {
        let behavior = ReviewChangeSignals {
            changed_paths: vec!["src/service.rs".into()],
            ..Default::default()
        };
        assert_eq!(
            select_review_lenses(&behavior),
            vec![ReviewLens::Correctness, ReviewLens::Tests]
        );

        let tests = ReviewChangeSignals {
            changed_paths: vec!["tests/service.rs".into()],
            changed_test_files: true,
            ..Default::default()
        };
        assert_eq!(
            select_review_lenses(&tests),
            vec![ReviewLens::Correctness, ReviewLens::Tests]
        );
    }

    #[test]
    fn selects_specialized_lenses_in_stable_order_and_caps_at_five() {
        let signals = ReviewChangeSignals {
            changed_paths: vec!["src/auth.rs".into()],
            changed_test_files: true,
            changed_error_paths: true,
            changed_public_types: true,
            changed_security_sensitive_area: true,
            changed_comments_docs: true,
            diff_line_count: 400,
        };
        assert_eq!(
            select_review_lenses(&signals),
            vec![
                ReviewLens::Correctness,
                ReviewLens::Tests,
                ReviewLens::ErrorHandling,
                ReviewLens::TypesAndInvariants,
                ReviewLens::Security,
            ]
        );
    }

    #[test]
    fn documentation_only_changes_do_not_trigger_tests() {
        let signals = ReviewChangeSignals {
            changed_paths: vec!["docs/review.md".into()],
            changed_comments_docs: true,
            ..Default::default()
        };
        assert_eq!(
            select_review_lenses(&signals),
            vec![ReviewLens::Correctness, ReviewLens::CommentsAndDocs]
        );
    }

    #[test]
    fn structured_data_changes_are_behavior_changes() {
        let signals = ReviewChangeSignals {
            changed_paths: vec!["fixtures/routing.json".into()],
            ..Default::default()
        };
        assert_eq!(
            select_review_lenses(&signals),
            vec![ReviewLens::Correctness, ReviewLens::Tests]
        );
    }

    #[test]
    fn simplification_requires_a_large_diff() {
        let signals = ReviewChangeSignals {
            changed_paths: vec!["src/service.rs".into()],
            diff_line_count: 251,
            ..Default::default()
        };
        assert_eq!(
            select_review_lenses(&signals),
            vec![
                ReviewLens::Correctness,
                ReviewLens::Tests,
                ReviewLens::Simplification
            ]
        );
    }

    #[test]
    fn small_reviews_run_inline() {
        let lenses = vec![
            ReviewLens::Correctness,
            ReviewLens::Tests,
            ReviewLens::Security,
        ];
        assert_eq!(
            plan_review_execution(&lenses, 250),
            ReviewExecutionPlan::Inline { lenses }
        );
    }

    #[test]
    fn large_multi_lens_reviews_are_capped_at_four_workers() {
        let lenses = vec![
            ReviewLens::Correctness,
            ReviewLens::Tests,
            ReviewLens::ErrorHandling,
        ];
        assert_eq!(
            plan_review_execution(&lenses, 251),
            ReviewExecutionPlan::Parallel {
                lenses,
                max_workers: 4,
            }
        );
    }

    #[test]
    fn two_lenses_remain_inline_even_for_large_diffs() {
        let lenses = vec![ReviewLens::Correctness, ReviewLens::Security];
        assert_eq!(
            plan_review_execution(&lenses, 10_000),
            ReviewExecutionPlan::Inline { lenses }
        );
    }
}
