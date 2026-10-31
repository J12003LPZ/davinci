//! Offline oracle evaluation for success-scored scenarios.
//!
//! The oracle proves a scenario can distinguish a correct repair from an
//! incorrect one without calling a model:
//!
//! 1. classify the scenario and refuse anything that is not success-scored;
//! 2. copy the fixture into a disposable workspace;
//! 3. run the scenario's verification commands on the untouched fixture and
//!    require at least one to fail (the scenario must reproduce a defect);
//! 4. apply a solution variant from `fixtures/solutions/<id>/`;
//! 5. digest the resulting workspace and re-run the same verification.
//!
//! Setup failures, timeouts and unavailable commands are reported separately
//! from task failures so a broken environment can never be counted as a wrong
//! solution or as a correct one.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::scenario::{BehaviorScenario, VerificationCommand};
use super::validity::{classify_scenario, default_fixture_root, ScenarioValidity};
use super::verification::{
    run_verification_command, run_verification_commands, VerificationResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolutionVariant {
    Correct,
    Unchanged,
    Incorrect,
    SyntaxOnly,
}

impl SolutionVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::Unchanged => "unchanged",
            Self::Incorrect => "incorrect",
            Self::SyntaxOnly => "syntax_only",
        }
    }

    fn solution_directory(&self) -> Option<&'static str> {
        match self {
            Self::Correct => Some("correct"),
            Self::Unchanged => None,
            Self::Incorrect => Some("incorrect"),
            Self::SyntaxOnly => Some("syntax_only"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleVerdict {
    /// The solution satisfied every verification command.
    Verified,
    /// The untouched fixture still fails: no repair was made.
    Unchanged,
    /// A change was applied and the verification failed.
    Incorrect,
    /// A verification command exceeded its timeout.
    TimedOut,
    /// A setup command failed before any task behavior was exercised.
    SetupFailure,
    /// A verification command could not be executed.
    VerificationUnavailable,
    /// The scenario is structural-only or invalid, so it cannot score success.
    NotSuccessScored,
    /// The fixture passed before a repair, or does not compile: the scenario
    /// cannot demonstrate a failing-before state.
    ScenarioInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OraclePhase {
    Validity,
    Setup,
    Baseline,
    Verification,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OracleReport {
    pub scenario_id: String,
    pub variant: SolutionVariant,
    pub validity: ScenarioValidity,
    pub verdict: OracleVerdict,
    pub phase: OraclePhase,
    pub detail: Option<String>,
    pub baseline_failed: bool,
    pub solution_applied: bool,
    pub workspace_digest: Option<String>,
    pub solution_digest: Option<String>,
    pub verification: Vec<VerificationResult>,
}

impl OracleReport {
    pub fn is_verified(&self) -> bool {
        self.verdict == OracleVerdict::Verified
    }

    fn at(
        scenario: &BehaviorScenario,
        variant: SolutionVariant,
        validity: ScenarioValidity,
        phase: OraclePhase,
        verdict: OracleVerdict,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            scenario_id: scenario.id.clone(),
            variant,
            validity,
            verdict,
            phase,
            detail: Some(detail.into()),
            baseline_failed: false,
            solution_applied: false,
            workspace_digest: None,
            solution_digest: None,
            verification: Vec::new(),
        }
    }
}

pub fn default_solution_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("solutions")
}

pub fn evaluate_scenario(
    scenario: &BehaviorScenario,
    variant: SolutionVariant,
) -> Result<OracleReport, String> {
    evaluate_scenario_in(
        scenario,
        variant,
        &default_fixture_root(),
        &default_solution_root(),
    )
}

pub fn evaluate_scenario_in(
    scenario: &BehaviorScenario,
    variant: SolutionVariant,
    fixture_root: &Path,
    solution_root: &Path,
) -> Result<OracleReport, String> {
    let validity = classify_scenario(scenario, fixture_root);
    if !validity.is_success_scored() {
        let codes = validity
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        return Ok(OracleReport::at(
            scenario,
            variant,
            validity.validity,
            OraclePhase::Validity,
            OracleVerdict::NotSuccessScored,
            if codes.is_empty() {
                "scenario is not success-scored".to_string()
            } else {
                format!("scenario is not success-scored: {codes}")
            },
        ));
    }

    let fixture = fixture_root.join(&scenario.repo_fixture);
    let sandbox = tempfile::Builder::new()
        .prefix("davinci-oracle-")
        .tempdir()
        .map_err(|error| format!("failed to create oracle sandbox: {error}"))?;
    let workspace = sandbox.path().join("workspace");
    copy_tree(&fixture, &workspace)?;

    let mut report = OracleReport::at(
        scenario,
        variant,
        validity.validity,
        OraclePhase::Baseline,
        OracleVerdict::Incorrect,
        "baseline has not run",
    );

    for command in &scenario.setup_commands {
        let setup = VerificationCommand {
            command: command.clone(),
            timeout_seconds: 60,
            expected_exit: 0,
        };
        match run_verification_command(&setup, &workspace) {
            Ok(result) if result.passed => {}
            Ok(result) => {
                report.verification.push(result);
                return Ok(OracleReport {
                    phase: OraclePhase::Setup,
                    verdict: OracleVerdict::SetupFailure,
                    detail: Some(format!("setup command failed: {command}")),
                    ..report
                });
            }
            Err(error) => {
                return Ok(OracleReport {
                    phase: OraclePhase::Setup,
                    verdict: OracleVerdict::VerificationUnavailable,
                    detail: Some(format!("setup command could not run: {error}")),
                    ..report
                });
            }
        }
    }

    let baseline = run_verification_commands(&scenario.verification_commands, &workspace)?;
    let baseline_timed_out = baseline.iter().any(|result| result.timed_out);
    let baseline_failed = baseline.iter().any(|result| !result.passed);
    report.verification.extend(baseline);
    if baseline_timed_out {
        return Ok(OracleReport {
            phase: OraclePhase::Baseline,
            verdict: OracleVerdict::TimedOut,
            baseline_failed,
            detail: Some("verification timed out on the untouched fixture".into()),
            ..report
        });
    }
    if !baseline_failed {
        return Ok(OracleReport {
            phase: OraclePhase::Baseline,
            verdict: OracleVerdict::ScenarioInvalid,
            detail: Some("verification passed before any repair was applied".into()),
            ..report
        });
    }
    if report
        .verification
        .iter()
        .any(|result| looks_like_compile_failure(&result.stderr))
    {
        return Ok(OracleReport {
            phase: OraclePhase::Baseline,
            verdict: OracleVerdict::ScenarioInvalid,
            baseline_failed,
            detail: Some("fixture does not compile before the task".into()),
            ..report
        });
    }
    report.baseline_failed = true;

    if let Some(directory) = variant.solution_directory() {
        let solution = solution_root.join(&scenario.id).join(directory);
        if !solution.is_dir() {
            return Err(format!(
                "missing solution for {} ({}): {}",
                scenario.id,
                variant.as_str(),
                solution.display()
            ));
        }
        report.solution_digest = Some(directory_digest(&solution)?);
        copy_tree_overlay(&solution, &workspace)?;
        report.solution_applied = true;
        // `fs::copy` preserves the source timestamp on Windows, which can be
        // older than the baseline build; drop build artifacts so the re-run
        // cannot execute the pre-repair binary.
        match std::fs::remove_dir_all(workspace.join("target")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to clear build artifacts for {}: {error}",
                    scenario.id
                ))
            }
        }
    }

    // Digest before verification so build artifacts cannot affect identity.
    report.workspace_digest = Some(directory_digest(&workspace)?);

    let after = run_verification_commands(&scenario.verification_commands, &workspace)?;
    let after_timed_out = after.iter().any(|result| result.timed_out);
    let after_passed = after.iter().all(|result| result.passed);
    report.verification.extend(after);

    report.phase = OraclePhase::Verification;
    report.verdict = if after_timed_out {
        OracleVerdict::TimedOut
    } else if after_passed {
        OracleVerdict::Verified
    } else if variant == SolutionVariant::Unchanged {
        OracleVerdict::Unchanged
    } else {
        OracleVerdict::Incorrect
    };
    Ok(report)
}

fn looks_like_compile_failure(stderr: &str) -> bool {
    [
        "error[E",
        "could not compile",
        "error: expected",
        "error: cannot find",
    ]
    .iter()
    .any(|marker| stderr.contains(marker))
}

pub fn directory_digest(directory: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hash_directory(&mut hasher, directory, "")?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn hash_directory(hasher: &mut Sha256, directory: &Path, prefix: &str) -> Result<(), String> {
    if !directory.is_dir() {
        return Err(format!(
            "cannot digest missing directory: {}",
            directory.display()
        ));
    }
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), ".git" | "target" | "node_modules") {
            continue;
        }
        let relative = format!("{prefix}{name}");
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            hasher.update(b"dir\0");
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            hash_directory(hasher, &path, &format!("{relative}/"))?;
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(&path)
                .map_err(|error| format!("cannot read link {}: {error}", path.display()))?;
            hasher.update(b"link\0");
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            hasher.update(target.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update([0]);
        } else {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let normalized = bytes
                .into_iter()
                .filter(|byte| *byte != b'\r')
                .collect::<Vec<_>>();
            hasher.update(b"file\0");
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            hasher.update((normalized.len() as u64).to_le_bytes());
            hasher.update(&normalized);
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!(
            "fixture directory does not exist: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("cannot list {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", source.display()))?;
        let target = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot stat {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn copy_tree_overlay(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!(
            "solution directory does not exist: {}",
            source.display()
        ));
    }
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("cannot list {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", source.display()))?;
        let target = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot stat {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
            copy_tree_overlay(&entry.path(), &target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
            std::fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{
        load_verified_core_corpus, BehaviorCategory, BehaviorLimits, BehaviorRequirement,
    };

    fn scenario(id: &str, repo_fixture: &str) -> BehaviorScenario {
        BehaviorScenario {
            id: id.into(),
            category: BehaviorCategory::VerificationIntegrity,
            request: "repair the fixture".into(),
            repo_fixture: repo_fixture.into(),
            requirements: vec![
                BehaviorRequirement::FileChanged {
                    path: "src/lib.rs".into(),
                },
                BehaviorRequirement::VerificationPassed {
                    kind: "test".into(),
                },
            ],
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: vec![VerificationCommand {
                command: "cargo test --quiet".into(),
                timeout_seconds: 120,
                expected_exit: 0,
            }],
        }
    }

    #[test]
    fn correct_solution_passes_every_verified_scenario() {
        let scenarios = load_verified_core_corpus().expect("verified corpus");
        for scenario in &scenarios {
            let report = evaluate_scenario(scenario, SolutionVariant::Correct).unwrap();
            assert!(
                report.is_verified(),
                "{} should verify: {report:?}",
                scenario.id
            );
            assert!(report.baseline_failed);
            assert!(report.workspace_digest.is_some());
        }
    }

    #[test]
    fn unchanged_repository_fails() {
        let scenarios = load_verified_core_corpus().unwrap();
        for scenario in &scenarios {
            let report = evaluate_scenario(scenario, SolutionVariant::Unchanged).unwrap();
            assert_eq!(
                report.verdict,
                OracleVerdict::Unchanged,
                "{} must not verify without a repair: {report:?}",
                scenario.id
            );
            assert!(!report.is_verified());
        }
    }

    #[test]
    fn syntax_only_change_does_not_verify() {
        let scenarios = load_verified_core_corpus().unwrap();
        for scenario in &scenarios {
            let report = evaluate_scenario(scenario, SolutionVariant::SyntaxOnly).unwrap();
            assert_eq!(
                report.verdict,
                OracleVerdict::Incorrect,
                "{} must reject a formatting-only change: {report:?}",
                scenario.id
            );
        }
    }

    #[test]
    fn incorrect_solution_does_not_verify() {
        let scenarios = load_verified_core_corpus().unwrap();
        for scenario in &scenarios {
            let report = evaluate_scenario(scenario, SolutionVariant::Incorrect).unwrap();
            assert_eq!(
                report.verdict,
                OracleVerdict::Incorrect,
                "{} must reject an incorrect repair: {report:?}",
                scenario.id
            );
        }
    }

    #[test]
    fn repeated_runs_are_deterministic() {
        let scenarios = load_verified_core_corpus().unwrap();
        let scenario = &scenarios[0];
        let first = evaluate_scenario(scenario, SolutionVariant::Correct).unwrap();
        let second = evaluate_scenario(scenario, SolutionVariant::Correct).unwrap();
        assert_eq!(first.verdict, second.verdict);
        assert_eq!(first.workspace_digest, second.workspace_digest);
        assert_eq!(first.solution_digest, second.solution_digest);
    }

    #[test]
    fn missing_oracle_cannot_be_scored_as_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut scenario = scenario("no-oracle", "repo");
        scenario.verification_commands.clear();
        let report =
            evaluate_scenario_in(&scenario, SolutionVariant::Correct, dir.path(), dir.path())
                .unwrap();
        assert_eq!(report.verdict, OracleVerdict::NotSuccessScored);
        assert_eq!(report.validity, ScenarioValidity::StructuralOnly);
    }

    #[test]
    fn baseline_that_already_passes_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut scenario = scenario("already-green", "repo");
        scenario.verification_commands = vec![VerificationCommand {
            command: "cargo --version".into(),
            timeout_seconds: 30,
            expected_exit: 0,
        }];
        scenario.requirements = vec![BehaviorRequirement::VerificationPassed {
            kind: "build".into(),
        }];
        let report =
            evaluate_scenario_in(&scenario, SolutionVariant::Correct, dir.path(), dir.path())
                .unwrap();
        assert_eq!(report.verdict, OracleVerdict::ScenarioInvalid);
        assert_eq!(report.phase, OraclePhase::Baseline);
    }

    #[test]
    fn setup_failure_is_not_a_task_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut scenario = scenario("setup", "repo");
        scenario.setup_commands = vec!["cargo not-a-real-cargo-command".into()];
        let report =
            evaluate_scenario_in(&scenario, SolutionVariant::Correct, dir.path(), dir.path())
                .unwrap();
        assert_eq!(report.verdict, OracleVerdict::SetupFailure);
        assert_eq!(report.phase, OraclePhase::Setup);
    }

    #[test]
    fn verification_timeout_is_not_a_test_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        std::fs::write(
            dir.path().join("repo/spin.py"),
            "import time\ntime.sleep(30)\n",
        )
        .unwrap();
        let mut scenario = scenario("slow", "repo");
        scenario.verification_commands = vec![VerificationCommand {
            command: "python spin.py".into(),
            timeout_seconds: 1,
            expected_exit: 0,
        }];
        let report =
            evaluate_scenario_in(&scenario, SolutionVariant::Correct, dir.path(), dir.path())
                .unwrap();
        assert_eq!(report.verdict, OracleVerdict::TimedOut);
        assert!(report.verification.iter().any(|result| result.timed_out));
    }

    #[test]
    fn failing_verification_command_is_never_verified() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo/src")).unwrap();
        std::fs::write(dir.path().join("repo/src/lib.rs"), "pub fn a() {}").unwrap();
        let mut scenario = scenario("bad-command", "repo");
        scenario.verification_commands = vec![VerificationCommand {
            command: "cargo --unavailable-check".into(),
            timeout_seconds: 30,
            expected_exit: 0,
        }];
        let report = evaluate_scenario_in(
            &scenario,
            SolutionVariant::Unchanged,
            dir.path(),
            dir.path(),
        )
        .unwrap();
        // cargo exists, so the command runs and fails: the oracle must call
        // this an unchanged repository, not a verified success.
        assert_eq!(report.verdict, OracleVerdict::Unchanged);
        assert!(!report.is_verified());
    }
}
