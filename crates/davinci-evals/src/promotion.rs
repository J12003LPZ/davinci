//! Machine-checkable evidence for prompt promotion.

use crate::behavior::ArtifactRoot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::path::Path;

pub const MIN_CORE_200_RUNS: usize = 3;
pub const MIN_CROSS_FAMILY_RUNS: usize = 1;
pub const MAX_UNVERIFIED_CLAIM_RATE: f64 = 0.005;
pub const MAX_UNRELATED_EDIT_RATE: f64 = 0.015;
pub const MAX_MEDIAN_TURN_DELTA: f64 = 0.10;
pub const MIN_PASS_RATE_DELTA: f64 = 0.02;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptPromotionEvidence {
    pub candidate_id: String,
    pub candidate_prompt_hash: String,
    pub base_stable_hash: String,
    pub davinci_commit: String,
    pub core_200_run_ids: Vec<String>,
    pub cross_family_run_ids: Vec<String>,
    pub competitor_run_ids: Vec<String>,
    pub pass_rate_delta: f64,
    pub unverified_claim_rate: f64,
    pub unrelated_edit_rate: f64,
    pub median_turn_delta: f64,
    pub stable_token_delta: i64,
    /// Entries use `run-id=sha256(manifest.json)`.
    pub artifact_manifest_hashes: Vec<String>,
}

pub fn validate_promotion_evidence(
    evidence: &PromptPromotionEvidence,
    artifact_root: &Path,
) -> Result<(), Vec<String>> {
    validate_promotion_evidence_against_hashes(evidence, artifact_root, None, None)
}

pub fn validate_promotion_evidence_against_hashes(
    evidence: &PromptPromotionEvidence,
    artifact_root: &Path,
    expected_candidate_hash: Option<&str>,
    expected_stable_hash: Option<&str>,
) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    for (name, value) in [
        ("candidate_id", evidence.candidate_id.as_str()),
        (
            "candidate_prompt_hash",
            evidence.candidate_prompt_hash.as_str(),
        ),
        ("base_stable_hash", evidence.base_stable_hash.as_str()),
        ("davinci_commit", evidence.davinci_commit.as_str()),
    ] {
        if value.trim().is_empty() {
            failures.push(format!("{name} is empty"));
        }
    }
    if evidence.candidate_prompt_hash == evidence.base_stable_hash {
        failures.push("candidate and stable prompt hashes must differ".into());
    }
    if let Some(expected) = expected_candidate_hash {
        if evidence.candidate_prompt_hash != expected {
            failures.push("candidate prompt hash does not match the expected hash".into());
        }
    }
    if let Some(expected) = expected_stable_hash {
        if evidence.base_stable_hash != expected {
            failures.push("stable prompt hash does not match the expected hash".into());
        }
    }
    validate_run_ids(
        "core-200",
        &evidence.core_200_run_ids,
        MIN_CORE_200_RUNS,
        &mut failures,
    );
    validate_run_ids(
        "cross-family",
        &evidence.cross_family_run_ids,
        MIN_CROSS_FAMILY_RUNS,
        &mut failures,
    );
    validate_run_ids("competitor", &evidence.competitor_run_ids, 0, &mut failures);

    if !evidence.pass_rate_delta.is_finite() || evidence.pass_rate_delta < MIN_PASS_RATE_DELTA {
        failures.push(format!(
            "pass-rate delta must be finite and at least {:.3}",
            MIN_PASS_RATE_DELTA
        ));
    }
    if !bounded_rate(evidence.unverified_claim_rate, MAX_UNVERIFIED_CLAIM_RATE) {
        failures.push(format!(
            "unverified claim rate must be in [0, {:.3}]",
            MAX_UNVERIFIED_CLAIM_RATE
        ));
    }
    if !bounded_rate(evidence.unrelated_edit_rate, MAX_UNRELATED_EDIT_RATE) {
        failures.push(format!(
            "unrelated edit rate must be in [0, {:.3}]",
            MAX_UNRELATED_EDIT_RATE
        ));
    }
    if !evidence.median_turn_delta.is_finite() || evidence.median_turn_delta > MAX_MEDIAN_TURN_DELTA
    {
        failures.push(format!(
            "median turn delta must be at most {:.3}",
            MAX_MEDIAN_TURN_DELTA
        ));
    }
    if evidence.artifact_manifest_hashes.is_empty() {
        failures.push("at least one artifact manifest hash is required".into());
    } else {
        validate_manifest_hashes(evidence, artifact_root, &mut failures);
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

fn validate_run_ids(label: &str, ids: &[String], minimum: usize, failures: &mut Vec<String>) {
    if ids.len() < minimum {
        failures.push(format!(
            "{label} evidence has {} runs; at least {minimum} are required",
            ids.len()
        ));
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() || id.contains('/') || id.contains('\\') || !unique.insert(id) {
            failures.push(format!("invalid or duplicate {label} run id: {id}"));
        }
    }
}

fn validate_manifest_hashes(
    evidence: &PromptPromotionEvidence,
    artifact_root: &Path,
    failures: &mut Vec<String>,
) {
    let referenced_ids: BTreeSet<&str> = evidence
        .core_200_run_ids
        .iter()
        .chain(&evidence.cross_family_run_ids)
        .chain(&evidence.competitor_run_ids)
        .map(String::as_str)
        .collect();
    let mut seen = BTreeSet::new();
    for entry in &evidence.artifact_manifest_hashes {
        let Some((run_id, expected_hash)) = entry.split_once('=') else {
            failures.push(format!("invalid artifact manifest hash entry: {entry}"));
            continue;
        };
        if !referenced_ids.contains(run_id)
            || !seen.insert(run_id)
            || expected_hash.len() != 64
            || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            failures.push(format!("invalid artifact manifest hash entry: {entry}"));
            continue;
        }
        let run_root = artifact_root.join(run_id);
        let artifact = ArtifactRoot::new(&run_root);
        let Ok(manifest) = artifact.validate_complete() else {
            failures.push(format!("artifact run is incomplete or tampered: {run_id}"));
            continue;
        };
        if manifest.candidate_prompt_hash != evidence.candidate_prompt_hash
            || manifest.baseline_prompt_hash != evidence.base_stable_hash
        {
            failures.push(format!(
                "artifact manifest prompt hashes do not match promotion evidence: {run_id}"
            ));
            continue;
        }
        let manifest_path = run_root.join("manifest.json");
        match fs::read(&manifest_path) {
            Ok(bytes) if sha256_hex(&bytes) == expected_hash => {}
            Ok(_) => failures.push(format!("artifact manifest hash mismatch: {run_id}")),
            Err(error) => failures.push(format!(
                "failed to read artifact manifest {run_id}: {error}"
            )),
        }
    }
}

fn bounded_rate(value: f64, maximum: f64) -> bool {
    value.is_finite() && (0.0..=maximum).contains(&value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::{persist_eval_run, EvalRunManifest};
    use serde_json::json;

    fn manifest(run_id: &str) -> EvalRunManifest {
        EvalRunManifest {
            run_id: run_id.into(),
            davinci_commit: "commit".into(),
            runner_version: "runner".into(),
            suite_hash: "suite".into(),
            provider: "provider".into(),
            model: "model".into(),
            baseline_prompt_hash: "stable-hash".into(),
            candidate_prompt_hash: "candidate-hash".into(),
            permission_mode: "default".into(),
            tool_surface_hash: "tools".into(),
            repeats: 3,
        }
    }

    fn evidence(run_id: &str, manifest_hash: &str) -> PromptPromotionEvidence {
        PromptPromotionEvidence {
            candidate_id: "candidate-1".into(),
            candidate_prompt_hash: "candidate-hash".into(),
            base_stable_hash: "stable-hash".into(),
            davinci_commit: "commit".into(),
            core_200_run_ids: vec![run_id.into(), "core-2".into(), "core-3".into()],
            cross_family_run_ids: vec![run_id.into()],
            competitor_run_ids: Vec::new(),
            pass_rate_delta: 0.02,
            unverified_claim_rate: 0.0,
            unrelated_edit_rate: 0.0,
            median_turn_delta: 0.0,
            stable_token_delta: 0,
            artifact_manifest_hashes: vec![format!("{run_id}={manifest_hash}")],
        }
    }

    #[test]
    fn promotion_validation_accepts_complete_matching_artifact() {
        let root = tempfile::tempdir().unwrap();
        let (run_root, _) = persist_eval_run(
            root.path(),
            &manifest("core-1"),
            "# Summary\n",
            &json!({"passed": true}),
        )
        .unwrap();
        let manifest_bytes = fs::read(run_root.join("manifest.json")).unwrap();
        let hash = sha256_hex(&manifest_bytes);
        assert!(validate_promotion_evidence(&evidence("core-1", &hash), root.path()).is_ok());
    }

    #[test]
    fn promotion_validation_rejects_wrong_candidate_or_stable_hash() {
        let root = tempfile::tempdir().unwrap();
        let (run_root, _) = persist_eval_run(
            root.path(),
            &manifest("core-1"),
            "# Summary\n",
            &json!({"passed": true}),
        )
        .unwrap();
        let hash = sha256_hex(&fs::read(run_root.join("manifest.json")).unwrap());
        let evidence = evidence("core-1", &hash);
        assert!(validate_promotion_evidence_against_hashes(
            &evidence,
            root.path(),
            Some("wrong-candidate"),
            Some("stable-hash"),
        )
        .is_err());
        assert!(validate_promotion_evidence_against_hashes(
            &evidence,
            root.path(),
            Some("candidate-hash"),
            Some("wrong-stable"),
        )
        .is_err());
    }

    #[test]
    fn promotion_validation_rejects_insufficient_runs_and_bad_metrics() {
        let mut evidence = PromptPromotionEvidence {
            candidate_id: "candidate-1".into(),
            candidate_prompt_hash: "candidate-hash".into(),
            base_stable_hash: "stable-hash".into(),
            davinci_commit: "commit".into(),
            core_200_run_ids: vec!["one".into()],
            cross_family_run_ids: Vec::new(),
            competitor_run_ids: Vec::new(),
            pass_rate_delta: 0.01,
            unverified_claim_rate: 0.01,
            unrelated_edit_rate: 0.02,
            median_turn_delta: 0.11,
            stable_token_delta: 0,
            artifact_manifest_hashes: Vec::new(),
        };
        let failures = validate_promotion_evidence(&evidence, Path::new("artifacts")).unwrap_err();
        assert!(failures.len() >= 6);
        evidence.pass_rate_delta = 0.02;
        assert!(validate_promotion_evidence(&evidence, Path::new("artifacts")).is_err());
    }
}
