//! Scenario validity: what a scenario can honestly score.
//!
//! A scenario is only `Verified` when it has a fixture that exists, every file
//! its requirements name, and at least one meaningful verification command.
//! `StructuralOnly` scenarios may still be scored for behavior (tool use, scope,
//! claims), but their result must never be counted as coding-task success.
//! `Invalid` scenarios are excluded from every success-rate denominator.
//!
//! A command such as `git diff --check` proves a patch is well formed, not that
//! the patch is correct, so it is classified as structural rather than as an
//! oracle.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::scenario::{BehaviorRequirement, BehaviorScenario};
use super::verification::validate_fixture_command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioValidity {
    /// The scenario has a real correctness oracle and may be scored as a task.
    Verified,
    /// The scenario is runnable but only behavior/structural properties can be
    /// scored. It must never be reported as verified task success.
    StructuralOnly,
    /// The scenario cannot be honestly executed or scored at all.
    Invalid,
}

impl ScenarioValidity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::StructuralOnly => "structural_only",
            Self::Invalid => "invalid",
        }
    }

    pub fn is_success_scored(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityFindingCode {
    EmptyScenarioId,
    DuplicateScenarioId,
    EmptyRequest,
    MissingRequirements,
    MissingFixture,
    MissingReferencedFile,
    UnsafeRelativePath,
    InvalidSetupCommand,
    InvalidVerificationCommand,
    ZeroVerificationTimeout,
    MissingVerificationCommand,
    StructuralVerificationOnly,
    VerificationKindWithoutOracle,
}

impl ValidityFindingCode {
    /// Findings that make a scenario invalid. A missing oracle is different:
    /// the scenario is still runnable, so it is structural-only.
    pub fn is_blocking(&self) -> bool {
        !matches!(
            self,
            Self::MissingVerificationCommand
                | Self::StructuralVerificationOnly
                | Self::VerificationKindWithoutOracle
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyScenarioId => "empty_scenario_id",
            Self::DuplicateScenarioId => "duplicate_scenario_id",
            Self::EmptyRequest => "empty_request",
            Self::MissingRequirements => "missing_requirements",
            Self::MissingFixture => "missing_fixture",
            Self::MissingReferencedFile => "missing_referenced_file",
            Self::UnsafeRelativePath => "unsafe_relative_path",
            Self::InvalidSetupCommand => "invalid_setup_command",
            Self::InvalidVerificationCommand => "invalid_verification_command",
            Self::ZeroVerificationTimeout => "zero_verification_timeout",
            Self::MissingVerificationCommand => "missing_verification_command",
            Self::StructuralVerificationOnly => "structural_verification_only",
            Self::VerificationKindWithoutOracle => "verification_kind_without_oracle",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidityFinding {
    pub code: ValidityFindingCode,
    pub detail: String,
    pub blocking: bool,
}

impl ValidityFinding {
    fn new(code: ValidityFindingCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            blocking: code.is_blocking(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioValidityReport {
    pub scenario_id: String,
    pub validity: ScenarioValidity,
    pub verification_commands: usize,
    pub meaningful_verification_commands: usize,
    pub findings: Vec<ValidityFinding>,
}

impl ScenarioValidityReport {
    pub fn is_success_scored(&self) -> bool {
        self.validity.is_success_scored()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusValidityReport {
    pub corpus: String,
    pub total_scenarios: usize,
    pub verified_scenarios: usize,
    pub structural_only_scenarios: usize,
    pub invalid_scenarios: usize,
    /// Only `verified_scenarios` may appear in a verified task-success rate.
    pub success_scored_scenarios: usize,
    pub scenarios: Vec<ScenarioValidityReport>,
}

impl CorpusValidityReport {
    pub fn build(
        corpus: impl Into<String>,
        scenarios: &[BehaviorScenario],
        fixture_root: &Path,
    ) -> Self {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for scenario in scenarios {
            if !scenario.id.trim().is_empty() {
                *counts.entry(scenario.id.as_str()).or_default() += 1;
            }
        }

        let mut verified = 0;
        let mut structural = 0;
        let mut invalid = 0;
        let mut reports = Vec::with_capacity(scenarios.len());
        for scenario in scenarios {
            let mut report = classify_scenario(scenario, fixture_root);
            if counts.get(scenario.id.as_str()).copied().unwrap_or(0) > 1 {
                report.findings.push(ValidityFinding::new(
                    ValidityFindingCode::DuplicateScenarioId,
                    format!("scenario id '{}' appears more than once", scenario.id),
                ));
                report.validity = ScenarioValidity::Invalid;
            }
            match report.validity {
                ScenarioValidity::Verified => verified += 1,
                ScenarioValidity::StructuralOnly => structural += 1,
                ScenarioValidity::Invalid => invalid += 1,
            }
            reports.push(report);
        }

        Self {
            corpus: corpus.into(),
            total_scenarios: scenarios.len(),
            verified_scenarios: verified,
            structural_only_scenarios: structural,
            invalid_scenarios: invalid,
            success_scored_scenarios: verified,
            scenarios: reports,
        }
    }

    pub fn issues(&self) -> impl Iterator<Item = &ScenarioValidityReport> {
        self.scenarios
            .iter()
            .filter(|scenario| !scenario.findings.is_empty())
    }

    pub fn invalid_ids(&self) -> Vec<&str> {
        self.scenarios
            .iter()
            .filter(|scenario| scenario.validity == ScenarioValidity::Invalid)
            .map(|scenario| scenario.scenario_id.as_str())
            .collect()
    }

    pub fn structural_only_ids(&self) -> Vec<&str> {
        self.scenarios
            .iter()
            .filter(|scenario| scenario.validity == ScenarioValidity::StructuralOnly)
            .map(|scenario| scenario.scenario_id.as_str())
            .collect()
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| format!("failed to encode corpus validity report: {error}"))
    }

    /// A success-scored corpus is only usable when every scenario is verified.
    pub fn require_all_verified(&self) -> Result<(), String> {
        if self.total_scenarios == 0 {
            return Err(format!("corpus '{}' contains no scenarios", self.corpus));
        }
        let rejected = self
            .scenarios
            .iter()
            .filter(|scenario| !scenario.is_success_scored())
            .map(|scenario| {
                let codes = scenario
                    .findings
                    .iter()
                    .map(|finding| finding.code.as_str())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(",");
                if codes.is_empty() {
                    scenario.scenario_id.clone()
                } else {
                    format!("{} ({codes})", scenario.scenario_id)
                }
            })
            .collect::<Vec<_>>();
        if rejected.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "corpus '{}' has {} of {} scenarios that cannot be scored as verified task success: {}",
                self.corpus,
                rejected.len(),
                self.total_scenarios,
                rejected.join("; ")
            ))
        }
    }
}

pub fn default_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("repos")
}

/// Commands that inspect or format a patch instead of checking behavior.
pub fn is_meaningful_oracle(command: &str) -> bool {
    let normalized = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    const STRUCTURAL: &[&str] = &[
        "git diff",
        "git status",
        "git log",
        "git show",
        "git ls-files",
        "git rev-parse",
        "true",
        "ls",
        "dir",
        "cat",
        "echo",
        "pwd",
        "test -e",
        "test -f",
        "test -d",
    ];
    !STRUCTURAL
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(&format!("{prefix} ")))
}

fn referenced_path(requirement: &BehaviorRequirement) -> Option<&str> {
    match requirement {
        BehaviorRequirement::ReadBeforeEdit { target }
        | BehaviorRequirement::FileChanged { path: target } => Some(target),
        _ => None,
    }
}

fn verification_kinds(scenario: &BehaviorScenario) -> BTreeSet<&str> {
    scenario
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            BehaviorRequirement::VerificationPassed { kind } => Some(kind.as_str()),
            _ => None,
        })
        .collect()
}

fn is_safe_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

/// Classify one scenario without executing anything.
pub fn classify_scenario(
    scenario: &BehaviorScenario,
    fixture_root: &Path,
) -> ScenarioValidityReport {
    let mut findings = Vec::new();

    if scenario.id.trim().is_empty() {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::EmptyScenarioId,
            "scenario id is empty",
        ));
    }
    if scenario.request.trim().is_empty() {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::EmptyRequest,
            "scenario request is empty",
        ));
    }
    if scenario.requirements.is_empty() {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::MissingRequirements,
            "scenario declares no requirements",
        ));
    }

    let fixture_relative = Path::new(&scenario.repo_fixture);
    let fixture = fixture_root.join(fixture_relative);
    if scenario.repo_fixture.trim().is_empty() || !is_safe_relative(fixture_relative) {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::UnsafeRelativePath,
            format!(
                "fixture '{}' must be a relative path",
                scenario.repo_fixture
            ),
        ));
    } else if !fixture.is_dir() {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::MissingFixture,
            format!("fixture directory does not exist: {}", fixture.display()),
        ));
    } else {
        for requirement in &scenario.requirements {
            let Some(referenced) = referenced_path(requirement) else {
                continue;
            };
            let relative = Path::new(referenced);
            if !is_safe_relative(relative) {
                findings.push(ValidityFinding::new(
                    ValidityFindingCode::UnsafeRelativePath,
                    format!("requirement path '{referenced}' must stay inside the fixture"),
                ));
            } else if !fixture.join(relative).exists() {
                findings.push(ValidityFinding::new(
                    ValidityFindingCode::MissingReferencedFile,
                    format!(
                        "fixture '{}' does not contain '{referenced}'",
                        scenario.repo_fixture
                    ),
                ));
            }
        }
    }

    for command in &scenario.setup_commands {
        if let Err(error) = validate_fixture_command(command) {
            findings.push(ValidityFinding::new(
                ValidityFindingCode::InvalidSetupCommand,
                format!("setup command '{command}': {error}"),
            ));
        }
    }

    let mut meaningful = 0;
    for command in &scenario.verification_commands {
        if let Err(error) = validate_fixture_command(&command.command) {
            findings.push(ValidityFinding::new(
                ValidityFindingCode::InvalidVerificationCommand,
                format!("verification command '{}': {error}", command.command),
            ));
        }
        if command.timeout_seconds == 0 {
            findings.push(ValidityFinding::new(
                ValidityFindingCode::ZeroVerificationTimeout,
                format!("verification command '{}' has no timeout", command.command),
            ));
        }
        if is_meaningful_oracle(&command.command) {
            meaningful += 1;
        }
    }

    if scenario.verification_commands.is_empty() {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::MissingVerificationCommand,
            "scenario has no verification commands",
        ));
    } else if meaningful == 0 {
        findings.push(ValidityFinding::new(
            ValidityFindingCode::StructuralVerificationOnly,
            "every verification command checks structure, not task behavior",
        ));
    }

    for kind in verification_kinds(scenario) {
        let covered = scenario.verification_commands.iter().any(|command| {
            is_meaningful_oracle(&command.command)
                && super::trace::classify_shell_command(&command.command) == kind
        });
        if !covered {
            findings.push(ValidityFinding::new(
                ValidityFindingCode::VerificationKindWithoutOracle,
                format!(
                    "requirement expects '{kind}' verification but no meaningful command provides it"
                ),
            ));
        }
    }

    let validity = if findings.iter().any(|finding| finding.blocking) {
        ScenarioValidity::Invalid
    } else if meaningful == 0 {
        ScenarioValidity::StructuralOnly
    } else {
        ScenarioValidity::Verified
    };

    ScenarioValidityReport {
        scenario_id: scenario.id.clone(),
        validity,
        verification_commands: scenario.verification_commands.len(),
        meaningful_verification_commands: meaningful,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{BehaviorCategory, BehaviorLimits};

    fn scenario(id: &str, repo_fixture: &str) -> BehaviorScenario {
        BehaviorScenario {
            id: id.into(),
            category: BehaviorCategory::Exploration,
            request: "repair the fixture".into(),
            repo_fixture: repo_fixture.into(),
            requirements: vec![BehaviorRequirement::FileChanged {
                path: "src/lib.rs".into(),
            }],
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        }
    }

    #[test]
    fn structural_commands_are_not_oracles() {
        assert!(!is_meaningful_oracle("git diff --check"));
        assert!(!is_meaningful_oracle("git status --short"));
        assert!(!is_meaningful_oracle("echo done"));
        assert!(is_meaningful_oracle("cargo test --quiet"));
        assert!(is_meaningful_oracle("pytest -q"));
    }

    #[test]
    fn missing_fixture_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let report = classify_scenario(&scenario("a", "absent"), dir.path());
        assert_eq!(report.validity, ScenarioValidity::Invalid);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == ValidityFindingCode::MissingFixture));
    }

    #[test]
    fn missing_referenced_file_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        let report = classify_scenario(&scenario("a", "repo"), dir.path());
        assert_eq!(report.validity, ScenarioValidity::Invalid);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == ValidityFindingCode::MissingReferencedFile));
    }

    #[test]
    fn no_verification_is_structural_only_not_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let report = classify_scenario(&scenario("a", "repo"), dir.path());
        assert_eq!(report.validity, ScenarioValidity::StructuralOnly);
        assert!(!report.is_success_scored());
    }

    #[test]
    fn patch_formatting_is_structural_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut scenario = scenario("a", "repo");
        scenario
            .verification_commands
            .push(crate::behavior::scenario::VerificationCommand {
                command: "git diff --check".into(),
                timeout_seconds: 30,
                expected_exit: 0,
            });
        let report = classify_scenario(&scenario, dir.path());
        assert_eq!(report.validity, ScenarioValidity::StructuralOnly);
        assert_eq!(report.meaningful_verification_commands, 0);
    }

    #[test]
    fn duplicate_ids_invalidate_every_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut first = scenario("same", "repo");
        first
            .verification_commands
            .push(crate::behavior::scenario::VerificationCommand {
                command: "cargo test --quiet".into(),
                timeout_seconds: 30,
                expected_exit: 0,
            });
        let second = first.clone();
        let report = CorpusValidityReport::build("x", &[first, second], dir.path());
        assert_eq!(report.invalid_scenarios, 2);
        assert_eq!(report.success_scored_scenarios, 0);
        assert!(report.require_all_verified().is_err());
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut scenario = scenario("a", "../outside");
        scenario
            .verification_commands
            .push(crate::behavior::scenario::VerificationCommand {
                command: "cargo test --quiet".into(),
                timeout_seconds: 30,
                expected_exit: 0,
            });
        let report = classify_scenario(&scenario, dir.path());
        assert_eq!(report.validity, ScenarioValidity::Invalid);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == ValidityFindingCode::UnsafeRelativePath));
    }
}
