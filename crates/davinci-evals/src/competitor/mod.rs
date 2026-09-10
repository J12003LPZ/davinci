//! External competitor harness integration and differential comparison.

pub mod claude_code;
pub mod command;

pub use claude_code::{
    format_competitor_report_markdown, ClaudeCodeHarness, CompetitorComparisonReport,
    FairComparisonMetadata, CLAUDE_CODE_BIN_ENV, PI_CLAUDE_CODE_BIN_ENV,
};
pub use command::{
    copy_dir_all, list_relative_files_with_content, CommandHarness, ExternalHarness, ExternalRun,
    ExternalTask,
};
