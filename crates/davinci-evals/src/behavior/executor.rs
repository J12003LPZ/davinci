//! End-to-end execution of one behavior scenario.

use super::artifacts::ArtifactRoot;
use super::process::{run_davinci_process, DavinciProcessConfig};
use super::runner::{classify_failure_signal, classify_process_result, RunDisposition};
use super::scenario::{BehaviorRequirement, BehaviorScenario, VerificationCommand};
use super::scorer::{score_trace, ScoreCard};
use super::trace::{classify_shell_command, BehaviorTrace, FileDiff, VerificationEvent};
use super::verification::{run_verification_commands, VerificationResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDiff {
    pub files_changed: Vec<String>,
    pub file_diffs: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MultiTurnSoakResult {
    pub scenario_id: String,
    pub turns_completed: usize,
    pub stable_prefix_hashes: Vec<String>,
    pub full_prompt_hashes: Vec<String>,
    pub active_capabilities: Vec<Vec<String>>,
    pub resume_hash_transition_detected: bool,
}

pub fn execute_multiturn_synthetic_scenario(
    scenario: &super::scenario::MultiTurnBehaviorScenario,
    profile: davinci_agent::prompt::PromptProfile,
) -> Result<MultiTurnSoakResult, String> {
    if scenario.turns.is_empty() {
        return Err(format!("scenario has no turns: {}", scenario.id));
    }

    let clean_cwd =
        tempfile::tempdir().map_err(|error| format!("failed to create cwd: {error}"))?;
    let mut agent = davinci_agent::Agent::new_builtin(profile);
    agent.cwd = clean_cwd.path().to_path_buf();

    let mut stable_prefix_hashes = Vec::with_capacity(scenario.turns.len());
    let mut full_prompt_hashes = Vec::with_capacity(scenario.turns.len());
    let mut active_capabilities = Vec::with_capacity(scenario.turns.len());
    for turn in &scenario.turns {
        agent.prompt_user_with(&turn.user_request, &[]);
        let manifest = agent
            .prompt_manifest
            .as_ref()
            .ok_or_else(|| format!("missing prompt manifest after turn: {}", scenario.id))?;
        stable_prefix_hashes.push(manifest.stable_sha256.clone());
        full_prompt_hashes.push(manifest.full_sha256.clone());
        active_capabilities.push(
            manifest
                .modules
                .iter()
                .filter_map(|module| module.id.strip_prefix("capability."))
                .map(str::to_string)
                .collect(),
        );

        // Build the provider view after every turn so a long run exercises
        // message projection without contacting a provider or changing files.
        let _ = agent.messages_for_provider();
    }

    let stale_record = davinci_agent::PromptSessionRecord::new(
        profile.id(),
        profile.version(),
        "stale-prompt-hash",
        None,
    );
    let (_, diagnostic) =
        davinci_agent::prompt::resolve_resume_prompt_session(Some(&stale_record), Some(profile));

    Ok(MultiTurnSoakResult {
        scenario_id: scenario.id.clone(),
        turns_completed: scenario.turns.len(),
        stable_prefix_hashes,
        full_prompt_hashes,
        active_capabilities,
        resume_hash_transition_detected: diagnostic.is_some(),
    })
}

impl WorkspaceDiff {
    fn empty() -> Self {
        Self {
            files_changed: Vec::new(),
            file_diffs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioRunResult {
    pub scenario: BehaviorScenario,
    pub trace: BehaviorTrace,
    pub score: ScoreCard,
    pub verification: Vec<VerificationResult>,
    pub workspace_diff: WorkspaceDiff,
    pub infrastructure_error: Option<String>,
    pub disposition: RunDisposition,
}

/// Execute the complete isolated scenario pipeline. Product launch, malformed
/// output, fixture setup, and artifact failures are infrastructure errors;
/// unmet scenario requirements remain ordinary score failures.
pub fn execute_scenario(
    scenario: &BehaviorScenario,
    config: &DavinciProcessConfig,
    artifacts: &ArtifactRoot,
) -> ScenarioRunResult {
    let mut trace = BehaviorTrace::new(&scenario.id, None);
    let mut verification = Vec::new();
    let mut workspace_diff = WorkspaceDiff::empty();
    let mut infrastructure_error = None;

    let result = (|| -> Result<(), String> {
        super::verification::validate_scenario_verification(scenario)?;
        let fixture = fixture_path(&scenario.repo_fixture);
        let sandbox = tempfile::Builder::new()
            .prefix("davinci-eval-")
            .tempdir()
            .map_err(|error| format!("failed to create scenario sandbox: {error}"))?;
        let scenario_root = sandbox.path();
        let workspace = scenario_root.join("workspace");
        copy_directory(&fixture, &workspace)?;

        run_setup_commands(&scenario.setup_commands, &workspace)?;
        let reproducer_commands = original_reproducer_commands(scenario);
        let baseline_reproducers = run_verification_commands(&reproducer_commands, &workspace)?;
        if let Some(result) = baseline_reproducers.iter().find(|result| result.passed) {
            return Err(format!(
                "configuration failure: original reproducer unexpectedly passed before the agent ran: {}",
                result.command
            ));
        }
        if scenario
            .verification_commands
            .iter()
            .any(|command| command.command.contains("git diff"))
        {
            initialize_git_workspace(&workspace)?;
        }
        let before = snapshot(&workspace)?;

        let mut process_config = config.clone();
        process_config.clean_agent_dir = scenario_root.join("agent");
        let previous_dir = std::env::current_dir()
            .map_err(|error| format!("failed to read evaluator cwd: {error}"))?;
        if process_config.binary.is_relative() {
            process_config.binary = previous_dir.join(&process_config.binary);
        }
        std::env::set_current_dir(&workspace)
            .map_err(|error| format!("failed to enter scenario workspace: {error}"))?;
        let process_result = run_davinci_process(&process_config, &scenario.request);
        let restore_result = std::env::set_current_dir(previous_dir);
        restore_result.map_err(|error| format!("failed to restore evaluator cwd: {error}"))?;
        let process_result = process_result?;
        if process_result.timed_out {
            return Err("DaVinci process timed out".into());
        }
        if let Some(disposition) = classify_process_result(
            process_result.exit_code,
            process_result.timed_out,
            &process_result.stdout_lines.join("\n"),
            &process_result.stderr,
        ) {
            let label = match disposition {
                RunDisposition::BehavioralResult => "behavioral result",
                RunDisposition::InfrastructureFailure => "infrastructure failure",
                RunDisposition::ConfigurationFailure => "configuration failure",
            };
            let diagnostic = process_result.bounded_diagnostic();
            return Err(format!(
                "{label}: DaVinci exited with code {}{}",
                process_result.exit_code,
                if diagnostic.is_empty() {
                    String::new()
                } else {
                    format!("; {diagnostic}")
                }
            ));
        }
        trace =
            super::json_events::trace_from_json_lines(&scenario.id, &process_result.stdout_lines)?;
        trace.stats.wall_ms = process_result.wall_ms;
        let mut observed_verification = baseline_reproducers
            .iter()
            .map(verification_event)
            .collect::<Vec<_>>();
        observed_verification.append(&mut trace.verification);
        trace.verification = observed_verification;

        let after = snapshot(&workspace)?;
        workspace_diff = diff_snapshots(&before, &after);
        augment_trace_with_workspace_diff(&mut trace, &workspace_diff);

        let baseline_count = baseline_reproducers.len();
        verification = baseline_reproducers;
        verification.extend(run_verification_commands(
            &scenario.verification_commands,
            &workspace,
        )?);
        verification.extend(run_verification_commands(&reproducer_commands, &workspace)?);
        for result in verification.iter().skip(baseline_count) {
            trace.verification.push(verification_event(result));
        }

        artifacts.write_json(
            Path::new("scenarios").join(&scenario.id).join("trace.json"),
            &trace,
        )?;
        artifacts.write_json(
            Path::new("scenarios")
                .join(&scenario.id)
                .join("verification.json"),
            &verification,
        )?;
        artifacts.write_json(
            Path::new("scenarios")
                .join(&scenario.id)
                .join("workspace-diff.json"),
            &workspace_diff,
        )?;
        Ok(())
    })();

    if let Err(error) = result {
        let disposition = classify_failure_signal(&error);
        infrastructure_error = Some(error);
        let score = score_trace(scenario, &trace);
        return ScenarioRunResult {
            scenario: scenario.clone(),
            trace,
            score,
            verification,
            workspace_diff,
            infrastructure_error,
            disposition,
        };
    }
    let score = score_trace(scenario, &trace);
    ScenarioRunResult {
        scenario: scenario.clone(),
        trace,
        score,
        verification,
        workspace_diff,
        infrastructure_error,
        disposition: RunDisposition::BehavioralResult,
    }
}

fn original_reproducer_commands(scenario: &BehaviorScenario) -> Vec<VerificationCommand> {
    let mut commands = Vec::new();
    for requirement in &scenario.requirements {
        let BehaviorRequirement::OriginalReproducerPassed { command, .. } = requirement else {
            continue;
        };
        if commands
            .iter()
            .any(|existing: &VerificationCommand| existing.command == *command)
        {
            continue;
        }
        commands.push(VerificationCommand {
            command: command.clone(),
            timeout_seconds: 60,
            expected_exit: 0,
        });
    }
    commands
}

fn verification_event(result: &VerificationResult) -> VerificationEvent {
    VerificationEvent {
        kind: classify_shell_command(&result.command).into(),
        command: result.command.clone(),
        passed: result.passed,
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("repos")
        .join(name)
}

fn run_setup_commands(commands: &[String], workspace: &Path) -> Result<(), String> {
    for command in commands {
        let setup = VerificationCommand {
            command: command.clone(),
            timeout_seconds: 60,
            expected_exit: 0,
        };
        let result = super::verification::run_verification_command(&setup, workspace)?;
        if !result.passed {
            return Err(format!("setup command failed: {command}"));
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!(
            "fixture directory does not exist: {}",
            source.display()
        ));
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create scenario workspace: {error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("failed to read fixture: {error}"))? {
        let entry = entry.map_err(|error| format!("failed to read fixture entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)
                .map_err(|error| format!("failed to copy fixture file: {error}"))?;
        }
    }
    Ok(())
}

fn initialize_git_workspace(workspace: &Path) -> Result<(), String> {
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.email", "davinci-eval@example.invalid"],
        vec!["config", "user.name", "DaVinci Eval"],
        vec!["add", "--all"],
        vec![
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "--allow-empty",
            "-m",
            "baseline",
        ],
    ] {
        let output = Command::new("git")
            .args(["-c", "core.hooksPath=NUL"])
            .args(args)
            .current_dir(workspace)
            .output()
            .map_err(|error| format!("failed to initialize verification repository: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to initialize verification repository: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    Ok(())
}

fn snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    snapshot_into(root, root, &mut files)?;
    Ok(files)
}

fn snapshot_into(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(current).map_err(|error| format!("failed to snapshot workspace: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to read workspace entry: {error}"))?;
        let path = entry.path();
        if generated_workspace_entry(&entry) {
            continue;
        }
        if path.is_dir() {
            snapshot_into(root, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("failed to relativize workspace path: {error}"))?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(
                relative,
                fs::read(&path)
                    .map_err(|error| format!("failed to read workspace file: {error}"))?,
            );
        }
    }
    Ok(())
}

fn generated_workspace_entry(entry: &fs::DirEntry) -> bool {
    if !entry.path().is_dir() {
        return false;
    }
    matches!(
        entry.file_name().to_string_lossy().as_ref(),
        ".git" | ".davinci" | ".pi" | "target" | "node_modules" | "__pycache__"
    )
}

fn diff_snapshots(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> WorkspaceDiff {
    let paths: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let mut diff = WorkspaceDiff::empty();
    for path in paths {
        if before.get(path) == after.get(path) {
            continue;
        }
        diff.files_changed.push(path.clone());
        diff.file_diffs.push(FileDiff {
            path: path.clone(),
            diff: format_diff(before.get(path), after.get(path)),
        });
    }
    diff
}

fn format_diff(before: Option<&Vec<u8>>, after: Option<&Vec<u8>>) -> String {
    let old = before
        .map(|bytes| String::from_utf8_lossy(bytes))
        .unwrap_or_default();
    let new = after
        .map(|bytes| String::from_utf8_lossy(bytes))
        .unwrap_or_default();
    let mut diff = String::new();
    for line in old.lines() {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in new.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

fn augment_trace_with_workspace_diff(trace: &mut BehaviorTrace, diff: &WorkspaceDiff) {
    for path in &diff.files_changed {
        if !trace.files_changed.contains(path) {
            trace.files_changed.push(path.clone());
        }
    }
    trace.file_diffs.extend(diff.file_diffs.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::artifacts::ArtifactRoot;
    use crate::behavior::scenario::{
        BehaviorCategory, BehaviorLimits, BehaviorTurn, MultiTurnBehaviorScenario,
    };
    use davinci_agent::PromptProfile;
    use std::collections::BTreeMap;
    use std::time::Duration;

    #[test]
    fn derives_unique_original_reproducer_commands() {
        let scenario = BehaviorScenario {
            id: "reproducer".into(),
            category: BehaviorCategory::VerificationIntegrity,
            request: "fix it".into(),
            repo_fixture: "fixture".into(),
            requirements: vec![
                BehaviorRequirement::OriginalReproducerPassed {
                    kind: "test".into(),
                    command: "cargo test --quiet".into(),
                },
                BehaviorRequirement::OriginalReproducerPassed {
                    kind: "test".into(),
                    command: "cargo test --quiet".into(),
                },
            ],
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        };

        assert_eq!(
            original_reproducer_commands(&scenario),
            vec![VerificationCommand {
                command: "cargo test --quiet".into(),
                timeout_seconds: 60,
                expected_exit: 0,
            }]
        );
    }

    #[test]
    fn snapshot_excludes_generated_directories_but_keeps_lockfiles() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("target/debug")).unwrap();
        fs::create_dir_all(directory.path().join(".davinci/vector-memory")).unwrap();
        fs::write(directory.path().join("target/debug/output"), b"generated").unwrap();
        fs::write(
            directory
                .path()
                .join(".davinci/vector-memory/records.jsonl"),
            b"runtime",
        )
        .unwrap();
        fs::write(directory.path().join("Cargo.lock"), b"tracked").unwrap();

        let files = snapshot(directory.path()).unwrap();

        assert_eq!(
            files.keys().cloned().collect::<Vec<_>>(),
            vec!["Cargo.lock"]
        );
    }

    fn config(binary: &str) -> DavinciProcessConfig {
        DavinciProcessConfig {
            binary: binary.into(),
            launcher_args: Vec::new(),
            provider: "fixture".into(),
            model: "fixture-model".into(),
            prompt_profile: PromptProfile::Stable,
            permission_mode: "read-only".into(),
            timeout: Duration::from_secs(5),
            clean_agent_dir: std::env::temp_dir().join("unused-agent-dir"),
            auth_source: None,
            allowed_env: BTreeMap::new(),
        }
    }

    #[test]
    fn separates_product_launch_failure_from_behavior_score() {
        let dir = tempfile::tempdir().unwrap();
        let scenario = BehaviorScenario {
            id: "missing-product".into(),
            category: BehaviorCategory::Exploration,
            request: "inspect".into(),
            repo_fixture: "core-repo".into(),
            requirements: Vec::new(),
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        };
        let result = execute_scenario(
            &scenario,
            &config("definitely-missing-davinci-binary"),
            &ArtifactRoot::new(dir.path()),
        );
        assert!(result.infrastructure_error.is_some());
        assert_eq!(result.disposition, RunDisposition::InfrastructureFailure);
        assert!(result.verification.is_empty());
        assert_eq!(result.trace.scenario_id, "missing-product");
    }

    #[test]
    fn synthetic_twenty_turn_soak_preserves_stable_prefix_and_resets_capabilities() {
        let scenario = MultiTurnBehaviorScenario {
            id: "twenty-turn-soak".into(),
            turns: (0..20)
                .map(|index| BehaviorTurn {
                    user_request: match index % 4 {
                        0 => "Inspect the repository before changing it.".into(),
                        1 => "Redesign this dashboard with premium visual hierarchy.".into(),
                        2 => "Diagnose the root cause of this failure and reproduce it.".into(),
                        _ => "Check the dependency version without editing files.".into(),
                    },
                })
                .collect(),
            verification_commands: Vec::new(),
        };

        let result = execute_multiturn_synthetic_scenario(&scenario, PromptProfile::Stable)
            .expect("synthetic soak should complete");

        assert_eq!(result.turns_completed, 20);
        assert!(result
            .stable_prefix_hashes
            .windows(2)
            .all(|hashes| hashes[0] == hashes[1]));
        assert!(result.active_capabilities[1].contains(&"frontend-design".to_string()));
        assert!(result.active_capabilities[2].contains(&"debugging".to_string()));
        assert!(result.active_capabilities[3].is_empty());
        assert!(result.resume_hash_transition_detected);
    }

    #[test]
    fn git_diff_verification_has_a_local_fixture_baseline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fixture.txt"), "baseline\n").unwrap();
        initialize_git_workspace(dir.path()).unwrap();
        std::fs::write(dir.path().join("fixture.txt"), "changed\n").unwrap();
        let result = super::super::verification::run_verification_command(
            &VerificationCommand {
                command: "git diff --check".into(),
                timeout_seconds: 5,
                expected_exit: 0,
            },
            dir.path(),
        )
        .unwrap();
        assert!(result.passed);
    }

    #[cfg(windows)]
    #[test]
    fn deeply_nested_artifact_root_does_not_host_the_scenario_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut artifact_path = dir.path().to_path_buf();
        while artifact_path.as_os_str().len() < 245 {
            artifact_path.push("content-addressed-evaluation-artifacts");
        }
        let scenario = BehaviorScenario {
            id: "windows-path-budget".into(),
            category: BehaviorCategory::Exploration,
            request: "inspect".into(),
            repo_fixture: "core-repo".into(),
            requirements: Vec::new(),
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: vec![VerificationCommand {
                command: "git diff --check".into(),
                timeout_seconds: 5,
                expected_exit: 0,
            }],
        };

        let result = execute_scenario(
            &scenario,
            &config("definitely-missing-davinci-binary"),
            &ArtifactRoot::new(artifact_path),
        );
        let error = result.infrastructure_error.expect("launch must fail");
        assert!(
            error.contains("failed to spawn"),
            "the short workspace must reach product launch, got: {error}"
        );
    }
}
