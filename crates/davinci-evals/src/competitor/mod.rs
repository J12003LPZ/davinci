//! External competitor harness integration and differential comparison.

pub mod challenges;
pub mod claude_code;
pub mod codex;
pub mod command;
pub mod hermes;
pub mod matched;
pub mod opencode;
pub mod probe;
pub mod report;

pub use challenges::{
    load_claude_public_challenges, ClaudeChallengeCategory, ClaudePublicChallenge,
    MIN_CLAUDE_PUBLIC_CHALLENGES,
};
pub use claude_code::{
    comparison_class_for_metadata, format_competitor_report_markdown, ClaudeCodeHarness,
    CompetitorComparisonReport, FairComparisonMetadata, CLAUDE_CODE_BIN_ENV,
    PI_CLAUDE_CODE_BIN_ENV,
};
pub use codex::{CodexRunner, CODEX_BIN_ENV};
pub use command::{
    copy_dir_all, copy_dir_all_ignoring, list_relative_files_with_content,
    list_relative_files_with_content_ignoring, CommandHarness, CompetitorRunner, ExternalHarness,
    ExternalRun, ExternalTask,
};
pub use hermes::{HermesRunner, HERMES_BIN_ENV};
pub use matched::{execute_matched_run, MatchedRunConfig, MatchedRunResult};
pub use opencode::{OpenCodeRunner, OPENCODE_BIN_ENV};
pub use probe::{probe_harness, HarnessCapabilities};
pub use report::{
    claim_is_eligible, classify_comparison, evaluate_competitor_claim,
    format_competitor_suite_summary, reject_mixed_comparison_modes, ClaimGateFailure,
    ComparisonClass, ComparisonMode, CompetitorClaimInputs, CompetitorSuiteSummary,
};
