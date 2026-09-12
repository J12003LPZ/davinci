//! Matched, fixture-identical DaVinci versus external-harness execution.

use super::command::{
    list_relative_files_with_content, ExternalHarness, ExternalRun, ExternalTask,
};
use crate::behavior::{BehaviorScenario, VerificationCommand, VerificationResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MatchedRunConfig {
    pub artifact_root: PathBuf,
    pub timeout: Duration,
    pub environment: BTreeMap<String, String>,
    pub ignore_paths: Vec<String>,
    pub verification_commands: Vec<VerificationCommand>,
    pub source_fixture: Option<PathBuf>,
}

impl Default for MatchedRunConfig {
    fn default() -> Self {
        Self {
            artifact_root: PathBuf::from("target/competitor-evals"),
            timeout: Duration::from_secs(120),
            environment: BTreeMap::new(),
            ignore_paths: Vec::new(),
            verification_commands: Vec::new(),
            source_fixture: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchedRunResult {
    pub scenario_id: String,
    pub davinci: ExternalRun,
    pub competitor: ExternalRun,
    pub davinci_verification: Vec<VerificationResult>,
    pub competitor_verification: Vec<VerificationResult>,
    pub snapshot_hash: String,
}

pub fn execute_matched_run(
    scenario: &BehaviorScenario,
    davinci: &dyn ExternalHarness,
    competitor: &dyn ExternalHarness,
    config: &MatchedRunConfig,
) -> Result<MatchedRunResult, String> {
    let fixture = config
        .source_fixture
        .clone()
        .unwrap_or_else(|| fixture_path(&scenario.repo_fixture));
    if !fixture.is_dir() {
        return Err(format!(
            "matched fixture does not exist: {}",
            fixture.display()
        ));
    }
    let snapshot_hash = hash_fixture(&fixture)?;
    let scenario_root = config.artifact_root.join(&scenario.id);
    if scenario_root.exists() {
        return Err(format!(
            "matched scenario artifact directory already exists: {}",
            scenario_root.display()
        ));
    }
    let baseline_task = ExternalTask {
        repo_path: fixture.clone(),
        request: scenario.request.clone(),
        timeout: config.timeout,
        artifact_dir: scenario_root.join("baseline"),
        environment: config.environment.clone(),
        ignore_paths: config.ignore_paths.clone(),
    };
    let candidate_task = ExternalTask {
        repo_path: fixture.clone(),
        request: scenario.request.clone(),
        timeout: config.timeout,
        artifact_dir: scenario_root.join("candidate"),
        environment: config.environment.clone(),
        ignore_paths: config.ignore_paths.clone(),
    };

    let (davinci, davinci_verification) =
        davinci.run_with_verification(&baseline_task, &config.verification_commands)?;
    let (competitor, competitor_verification) =
        competitor.run_with_verification(&candidate_task, &config.verification_commands)?;
    if hash_fixture(&fixture)? != snapshot_hash {
        return Err("source fixture changed during matched execution".into());
    }
    Ok(MatchedRunResult {
        scenario_id: scenario.id.clone(),
        davinci,
        competitor,
        davinci_verification,
        competitor_verification,
        snapshot_hash,
    })
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("repos")
        .join(name)
}

fn hash_fixture(root: &Path) -> Result<String, String> {
    let mut digest = Sha256::new();
    for (path, contents) in list_relative_files_with_content(root) {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(contents);
    }
    let bytes = digest.finalize();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::{BehaviorCategory, BehaviorLimits};
    use crate::competitor::CommandHarness;

    #[test]
    fn matched_run_uses_one_source_hash_and_separate_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("runner.py");
        std::fs::write(
            &script,
            "from pathlib import Path\nPath('matched.txt').write_text('same')\nprint('ok')\n",
        )
        .unwrap();
        let harness = CommandHarness::new(
            "fixture",
            "python",
            vec![script.to_string_lossy().to_string()],
        );
        let scenario = BehaviorScenario {
            id: "matched-fixture".into(),
            category: BehaviorCategory::Exploration,
            request: "perform matched work".into(),
            repo_fixture: "core-repo".into(),
            requirements: Vec::new(),
            limits: BehaviorLimits::default(),
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        };
        let result = execute_matched_run(
            &scenario,
            &harness,
            &harness,
            &MatchedRunConfig {
                artifact_root: root.path().join("artifacts"),
                ..MatchedRunConfig::default()
            },
        )
        .unwrap();
        assert_eq!(result.scenario_id, "matched-fixture");
        assert_eq!(result.davinci.exit_code, 0);
        assert_eq!(result.competitor.exit_code, 0);
        let baseline_transcript = root
            .path()
            .join("artifacts")
            .join("matched-fixture")
            .join("baseline")
            .join("transcript.log");
        assert_eq!(result.davinci.transcript_path, baseline_transcript);
        assert!(result.davinci.transcript_path.is_file());
        assert_ne!(
            result.davinci.transcript_path,
            result.competitor.transcript_path
        );
        assert_eq!(result.snapshot_hash.len(), 64);
    }
}
