//! Optional Claude Code differential adapter and fair comparison reporter.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use super::command::{CommandHarness, ExternalHarness, ExternalRun, ExternalTask};

pub const CLAUDE_CODE_BIN_ENV: &str = "DAVINCI_CLAUDE_CODE_BIN";
pub const PI_CLAUDE_CODE_BIN_ENV: &str = "PI_CLAUDE_CODE_BIN";

#[derive(Debug, Clone)]
pub struct ClaudeCodeHarness {
    inner: CommandHarness,
}

impl ClaudeCodeHarness {
    pub fn new() -> Self {
        let bin = std::env::var(CLAUDE_CODE_BIN_ENV)
            .or_else(|_| std::env::var(PI_CLAUDE_CODE_BIN_ENV))
            .unwrap_or_else(|_| "claude".into());

        Self {
            inner: CommandHarness::new("claude-code", PathBuf::from(bin), vec!["-p".into()]),
        }
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            inner: CommandHarness::new("claude-code", binary, vec!["-p".into()]),
        }
    }
}

impl Default for ClaudeCodeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalHarness for ClaudeCodeHarness {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn available(&self) -> Result<bool, String> {
        self.inner.available()
    }

    fn run(&self, task: &ExternalTask) -> Result<ExternalRun, String> {
        self.inner.run(task)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FairComparisonMetadata {
    pub davinci_version: String,
    pub competitor_name: String,
    pub competitor_version: Option<String>,
    pub davinci_model: String,
    pub competitor_model: String,
    pub models_controlled: bool,
    pub permission_mode: String,
    pub timeout_seconds: u64,
    pub repo_snapshot_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompetitorComparisonReport {
    pub metadata: FairComparisonMetadata,
    pub task_id: String,
    pub request: String,
    pub davinci_passed: bool,
    pub competitor_passed: bool,
    pub davinci_wall_ms: u64,
    pub competitor_wall_ms: u64,
    pub davinci_files_changed: Vec<String>,
    pub competitor_files_changed: Vec<String>,
}

pub fn format_competitor_report_markdown(report: &CompetitorComparisonReport) -> String {
    let mut md = String::new();
    md.push_str("## External Harness Differential Report\n\n");

    md.push_str("### Fair Comparison Disclosure\n");
    md.push_str(&format!("- **DaVinci version**: {}\n", report.metadata.davinci_version));
    md.push_str(&format!("- **Competitor**: {} ({})\n",
        report.metadata.competitor_name,
        report.metadata.competitor_version.as_deref().unwrap_or("unknown")
    ));
    md.push_str(&format!("- **DaVinci Model**: {}\n", report.metadata.davinci_model));
    md.push_str(&format!("- **Competitor Model**: {}\n", report.metadata.competitor_model));
    md.push_str(&format!("- **Models Controlled (Identical)**: {}\n",
        if report.metadata.models_controlled { "YES (Identical underlying model)" } else { "NO (Different models: harness-only comparison not claimed)" }
    ));
    md.push_str(&format!("- **Permission Mode**: {}\n", report.metadata.permission_mode));
    md.push_str(&format!("- **Timeout**: {}s\n", report.metadata.timeout_seconds));
    md.push_str(&format!("- **Repo Snapshot**: {}\n\n", report.metadata.repo_snapshot_hash));

    md.push_str("### Observable Results\n");
    md.push_str("| Metric | DaVinci | Competitor |\n");
    md.push_str("| :--- | :--- | :--- |\n");
    md.push_str(&format!("| Task Correctness | {} | {} |\n",
        if report.davinci_passed { "PASS" } else { "FAIL" },
        if report.competitor_passed { "PASS" } else { "FAIL" }
    ));
    md.push_str(&format!("| Wall Time | {} ms | {} ms |\n", report.davinci_wall_ms, report.competitor_wall_ms));
    md.push_str(&format!("| Files Changed Count | {} | {} |\n",
        report.davinci_files_changed.len(),
        report.competitor_files_changed.len()
    ));

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn competitor_report_markdown_includes_fair_comparison_disclosures() {
        let meta = FairComparisonMetadata {
            davinci_version: "1.0.0".into(),
            competitor_name: "claude-code".into(),
            competitor_version: Some("0.2.29".into()),
            davinci_model: "claude-3-7-sonnet".into(),
            competitor_model: "claude-3-7-sonnet".into(),
            models_controlled: true,
            permission_mode: "auto".into(),
            timeout_seconds: 120,
            repo_snapshot_hash: "a1b2c3d4".into(),
        };

        let report = CompetitorComparisonReport {
            metadata: meta,
            task_id: "scen-diff-01".into(),
            request: "Fix bug in parser".into(),
            davinci_passed: true,
            competitor_passed: true,
            davinci_wall_ms: 3200,
            competitor_wall_ms: 4800,
            davinci_files_changed: vec!["src/parser.rs".into()],
            competitor_files_changed: vec!["src/parser.rs".into(), "package.json".into()],
        };

        let md = format_competitor_report_markdown(&report);
        assert!(md.contains("Fair Comparison Disclosure"));
        assert!(md.contains("YES (Identical underlying model)"));
        assert!(md.contains("3200 ms"));
        assert!(md.contains("4800 ms"));
    }

    #[test]
    fn competitor_report_warns_when_models_are_uncontrolled() {
        let meta = FairComparisonMetadata {
            davinci_version: "1.0.0".into(),
            competitor_name: "claude-code".into(),
            competitor_version: None,
            davinci_model: "gpt-4o".into(),
            competitor_model: "claude-3-7-sonnet".into(),
            models_controlled: false,
            permission_mode: "auto".into(),
            timeout_seconds: 60,
            repo_snapshot_hash: "hash123".into(),
        };

        let report = CompetitorComparisonReport {
            metadata: meta,
            task_id: "scen-diff-02".into(),
            request: "Refactor error handling".into(),
            davinci_passed: true,
            competitor_passed: false,
            davinci_wall_ms: 2500,
            competitor_wall_ms: 3100,
            davinci_files_changed: vec!["src/err.rs".into()],
            competitor_files_changed: vec![],
        };

        let md = format_competitor_report_markdown(&report);
        assert!(md.contains("NO (Different models: harness-only comparison not claimed)"));
    }
}
