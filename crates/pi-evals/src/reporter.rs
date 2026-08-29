//! TS `vendor/pi/packages/evals/src/vitest-evals/reporter.ts` without Vitest.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::artifacts::{
    persist_eval_artifact_references, EvalArtifact, PI_SESSION_SNAPSHOT_ARTIFACT,
};
use crate::harness_table::{
    parse_eval_harness_iteration_artifact, EVAL_HARNESS_ITERATION_ARTIFACT,
};
use crate::summary::{
    format_harness_comparison_report, summarize_harness_comparisons, HarnessObservation,
    HarnessObservationOutcome,
};

pub const EVAL_COMPARISONS_INTERRUPTED: &str =
    "Eval comparisons unavailable: test run interrupted.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<f64>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl Default for HarnessUsage {
    fn default() -> Self {
        Self {
            total_tokens: None,
            metadata: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTimings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRun {
    #[serde(default)]
    pub artifacts: Map<String, Value>,
    #[serde(default)]
    pub usage: HarnessUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timings: Option<HarnessTimings>,
    #[serde(default)]
    pub errors: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTestCase {
    pub id: String,
    pub file: String,
    pub name: String,
    pub full_name: String,
    pub status: String,
    pub harness_name: String,
    pub run: HarnessRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTestModule {
    pub file: String,
    pub tests: Vec<HarnessTestCase>,
}

pub fn is_harness_run(run: &HarnessRun) -> bool {
    run.artifacts.contains_key(EVAL_HARNESS_ITERATION_ARTIFACT)
        || run.artifacts.contains_key("runId")
        || !run.errors.is_empty()
        || run.usage.total_tokens.is_some()
        || run.timings.is_some()
}

fn read_finite_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
}

fn outcome_from_state(state: &str) -> HarnessObservationOutcome {
    match state {
        "passed" => HarnessObservationOutcome::Unscored,
        "failed" => HarnessObservationOutcome::Errored,
        "skipped" => HarnessObservationOutcome::Skipped,
        "pending" => HarnessObservationOutcome::Pending,
        "errored" => HarnessObservationOutcome::Errored,
        _ => HarnessObservationOutcome::Pending,
    }
}

/// TS `collectHarnessObservations`.
pub fn collect_harness_observations(modules: &[HarnessTestModule]) -> Vec<HarnessObservation> {
    let mut observations = Vec::new();
    for module in modules {
        for test in &module.tests {
            if !is_harness_run(&test.run) {
                continue;
            }
            let iteration = parse_eval_harness_iteration_artifact(
                test.run.artifacts.get(EVAL_HARNESS_ITERATION_ARTIFACT),
            );
            let Some(iteration) = iteration else {
                continue;
            };
            let estimated_cost_usd =
                read_finite_number(test.run.usage.metadata.get("estimatedCostUsd"));
            let mut observation = HarnessObservation {
                eval_set: iteration.eval_set,
                group_key: iteration.group_key,
                test_name: test.name.clone(),
                file: module.file.clone(),
                harness: iteration.harness,
                baseline: iteration.baseline,
                candidates: iteration.candidates,
                repetition: iteration.repetition,
                total_tokens: test.run.usage.total_tokens,
                total_ms: test
                    .run
                    .timings
                    .as_ref()
                    .and_then(|timings| timings.total_ms),
                estimated_cost_usd,
                outcome: HarnessObservationOutcome::Unscored,
                score: None,
            };
            if !test.run.errors.is_empty() {
                observation.outcome = HarnessObservationOutcome::Errored;
            } else if let Some(score) = test.avg_score.filter(|value| value.is_finite()) {
                observation.outcome = HarnessObservationOutcome::Scored;
                observation.score = Some(score);
            } else {
                observation.outcome = outcome_from_state(&test.status);
            }
            observations.push(observation);
        }
    }
    observations
}

/// TS `onTestRunEnd` log payload (without the leading newline Vitest adds).
pub fn format_test_run_end(reason: &str, modules: &[HarnessTestModule]) -> Option<String> {
    if reason == "interrupted" {
        return Some(EVAL_COMPARISONS_INTERRUPTED.to_string());
    }
    let observations = collect_harness_observations(modules);
    let formatted = format_harness_comparison_report(&summarize_harness_comparisons(&observations));
    if formatted.is_empty() {
        None
    } else {
        Some(formatted)
    }
}

fn fallback_run_id(test: &HarnessTestCase) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{}:{}:{}", test.id, test.file, test.full_name));
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// TS `appendHarnessRunReport`.
pub fn append_harness_run_report(
    test: &HarnessTestCase,
    artifact_directory: Option<&Path>,
    extra_artifacts: &[EvalArtifact],
) -> Result<Option<PathBuf>, String> {
    let directory = match artifact_directory {
        Some(path) => path.to_path_buf(),
        None => {
            let env = std::env::var("PI_EVAL_ARTIFACT_DIR").unwrap_or_default();
            let trimmed = env.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            PathBuf::from(trimmed)
        }
    };
    if !is_harness_run(&test.run) {
        return Ok(None);
    }
    let run_id = test
        .run
        .artifacts
        .get("runId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| fallback_run_id(test));
    let mut metadata = Map::new();
    for (name, value) in &test.run.artifacts {
        if name != "runId" && name != PI_SESSION_SNAPSHOT_ARTIFACT {
            metadata.insert(name.clone(), value.clone());
        }
    }
    let references = persist_eval_artifact_references(extra_artifacts, &run_id, &directory)?;
    let artifacts = references
        .iter()
        .map(|item| serde_json::json!({ "name": item.name, "path": item.path }))
        .collect::<Vec<_>>();
    let mut record = serde_json::json!({
        "schemaVersion": 1,
        "runId": run_id,
        "test": {
            "id": test.id,
            "file": test.file,
            "name": test.name,
            "fullName": test.full_name,
            "status": test.status,
        },
        "harness": test.harness_name,
        "usage": test.run.usage,
        "artifacts": artifacts,
    });
    if let Some(timings) = &test.run.timings {
        record["timings"] = serde_json::to_value(timings).unwrap_or(Value::Null);
    }
    if !test.run.errors.is_empty() {
        record["errors"] = Value::Array(test.run.errors.clone());
    }
    if !metadata.is_empty() {
        record["metadata"] = Value::Object(metadata);
    }
    std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
    }
    let path = directory.join("runs.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(format!("{}\n", record).as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::HarnessObservationOutcome;

    fn iteration_artifact() -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "evalSet": "tool access",
            "groupKey": "[\"read file\",1]",
            "harness": "baseline",
            "baseline": "baseline",
            "candidates": ["candidate"],
            "repetition": 1
        })
    }

    fn scored_test(status: &str, score: Option<f64>, errors: Vec<Value>) -> HarnessTestCase {
        let mut artifacts = Map::new();
        artifacts.insert(EVAL_HARNESS_ITERATION_ARTIFACT.into(), iteration_artifact());
        artifacts.insert("runId".into(), Value::String("run-1".into()));
        let mut usage = HarnessUsage {
            total_tokens: Some(12.0),
            metadata: Map::new(),
        };
        usage
            .metadata
            .insert("estimatedCostUsd".into(), serde_json::json!(0.25));
        HarnessTestCase {
            id: "test-1".into(),
            file: "src/tool-access.eval.ts".into(),
            name: "read file".into(),
            full_name: "tool access > read file".into(),
            status: status.into(),
            harness_name: "baseline".into(),
            run: HarnessRun {
                artifacts,
                usage,
                timings: Some(HarnessTimings {
                    total_ms: Some(40.0),
                }),
                errors,
            },
            avg_score: score,
        }
    }

    #[test]
    fn collect_and_interrupted_run_match_ts() {
        assert_eq!(
            format_test_run_end("interrupted", &[]).as_deref(),
            Some(EVAL_COMPARISONS_INTERRUPTED)
        );
        let scored = scored_test("passed", Some(1.0), Vec::new());
        let errored = {
            let mut test =
                scored_test("failed", None, vec![serde_json::json!({"message": "boom"})]);
            test.id = "test-2".into();
            test
        };
        let unscored = {
            let mut test = scored_test("passed", None, Vec::new());
            test.id = "test-3".into();
            test
        };
        let modules = [HarnessTestModule {
            file: "src/tool-access.eval.ts".into(),
            tests: vec![scored, errored, unscored],
        }];
        let observations = collect_harness_observations(&modules);
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].outcome, HarnessObservationOutcome::Scored);
        assert_eq!(observations[0].score, Some(1.0));
        assert_eq!(observations[0].total_tokens, Some(12.0));
        assert_eq!(observations[0].total_ms, Some(40.0));
        assert_eq!(observations[0].estimated_cost_usd, Some(0.25));
        assert_eq!(observations[1].outcome, HarnessObservationOutcome::Errored);
        assert_eq!(observations[2].outcome, HarnessObservationOutcome::Unscored);
        assert!(format_test_run_end("passed", &modules).is_some());
    }

    #[test]
    fn append_harness_run_report_writes_runs_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let test = scored_test("passed", Some(1.0), Vec::new());
        let path = append_harness_run_report(&test, Some(dir.path()), &[])
            .unwrap()
            .expect("runs.jsonl");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("runs.jsonl")
        );
        let line = std::fs::read_to_string(&path).unwrap();
        let record: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(record["schemaVersion"], 1);
        assert_eq!(record["runId"], "run-1");
        assert_eq!(record["test"]["fullName"], "tool access > read file");
        assert_eq!(record["test"]["status"], "passed");
        assert_eq!(record["harness"], "baseline");
        assert_eq!(record["usage"]["totalTokens"], 12.0);
        assert!(record.get("metadata").is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(append_harness_run_report(&test, None, &[])
            .unwrap()
            .is_none());
    }
}
