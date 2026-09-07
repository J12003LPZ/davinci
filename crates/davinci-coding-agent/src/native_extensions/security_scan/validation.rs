//! Native evidence gates. Schema validity does not replace causal review.

use super::snapshot::Snapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Location {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content_hash: String,
    pub role: String,
    #[serde(default = "default_side")]
    pub snapshot_side: String,
}

pub(super) fn default_side() -> String {
    "worktree".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Claim {
    pub title: String,
    pub actor: String,
    pub entrypoint: String,
    pub control: String,
    pub sink: String,
    pub impact: String,
    pub prerequisites: Vec<String>,
    pub locations: Vec<Location>,
    pub proof_gaps: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Reportable,
    Suppressed,
    NotApplicable,
    Duplicate,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Confirmed,
    Likely,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Gate {
    pub satisfied: bool,
    pub rationale: String,
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assessment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_cause: Option<super::root_cause::RootCause>,
    pub disposition: Disposition,
    pub classification: Option<Classification>,
    pub severity: super::FindingSeverity,
    pub severity_rationale: String,
    pub confidence: String,
    pub confidence_rationale: String,
    /// Identity, reachability, control semantics, impact, adversarial acceptance.
    pub gates: [Gate; 5],
    pub counterevidence: Vec<Location>,
    pub proof_gaps: Vec<String>,
    pub reason: String,
    pub remediation: String,
}

pub fn validate_location(location: &Location, snapshot: &Snapshot) -> Result<(), String> {
    if ![
        "source",
        "entrypoint",
        "control",
        "sink",
        "configuration",
        "test",
    ]
    .contains(&location.role.as_str())
    {
        return Err("unknown evidence role".into());
    }
    let file = snapshot.file(&location.path, &location.snapshot_side)?;
    if file.hash != location.content_hash {
        return Err("evidence source hash mismatch".into());
    }
    snapshot.read_side(
        &location.path,
        location.start_line,
        location.end_line,
        &location.snapshot_side,
    )?;
    Ok(())
}

pub fn validate_claim(claim: &Claim, snapshot: &Snapshot) -> Result<(), String> {
    validate_claim_shape(claim)?;
    for location in &claim.locations {
        validate_location(location, snapshot)?;
    }
    Ok(())
}

pub fn validate_claim_shape(claim: &Claim) -> Result<(), String> {
    for value in [
        &claim.title,
        &claim.actor,
        &claim.entrypoint,
        &claim.control,
        &claim.sink,
        &claim.impact,
    ] {
        if value.trim().is_empty() || value.len() > 8192 {
            return Err("claim lacks bounded causal evidence".into());
        }
    }
    if claim.locations.is_empty()
        || claim.locations.len() > 32
        || claim.prerequisites.len() > 32
        || claim.proof_gaps.len() > 32
    {
        return Err("invalid claim evidence count".into());
    }
    Ok(())
}

pub fn validate_assessment(
    claim: &Claim,
    assessment: &Assessment,
    snapshot: &Snapshot,
) -> Result<(), String> {
    validate_claim(claim, snapshot)?;
    if let Some(root) = &assessment.root_cause {
        validate_location(&root.control, snapshot)?;
        validate_location(&root.decision, snapshot)?;
    }
    for location in assessment
        .gates
        .iter()
        .flat_map(|gate| &gate.locations)
        .chain(&assessment.counterevidence)
    {
        validate_location(location, snapshot)?;
    }
    validate_assessment_semantics(claim, assessment)
}

pub fn validate_assessment_semantics(claim: &Claim, assessment: &Assessment) -> Result<(), String> {
    validate_claim_shape(claim)?;
    if let Some(root) = &assessment.root_cause {
        root.validate(&assessment.gates)?;
    }
    if assessment.reason.trim().is_empty()
        || !["high", "medium", "low"].contains(&assessment.confidence.as_str())
    {
        return Err("assessment requires a reason and calibrated confidence".into());
    }
    for gate in &assessment.gates {
        if gate.rationale.trim().is_empty() || (gate.satisfied && gate.locations.is_empty()) {
            return Err("evidence gate requires source-backed rationale".into());
        }
        if gate.satisfied && claims_observed_runtime(&gate.rationale) {
            return Err("runtime claim requires observed result".into());
        }
    }
    match assessment.disposition {
        Disposition::Reportable => {
            if assessment.root_cause.is_none() {
                return Err("reportable assessment requires source-bound root identity".into());
            }
            if assessment.classification.is_none()
                || assessment.severity_rationale.trim().is_empty()
                || assessment.confidence_rationale.trim().is_empty()
                || assessment.remediation.trim().is_empty()
            {
                return Err("reportable assessment lacks classification or rationale".into());
            }
            if assessment.classification == Some(Classification::Confirmed)
                && (!assessment.gates.iter().all(|gate| gate.satisfied)
                    || !assessment.proof_gaps.is_empty()
                    || !claim.proof_gaps.is_empty())
            {
                return Err(
                    "confirmation cannot bypass evidence gates or material proof gaps".into(),
                );
            }
            if assessment.classification == Some(Classification::Likely)
                && assessment.proof_gaps.is_empty()
            {
                return Err("likely classification requires a named material gap".into());
            }
        }
        Disposition::Suppressed | Disposition::NotApplicable => {
            if assessment.counterevidence.is_empty() {
                return Err("suppression requires concrete counterevidence".into());
            }
            if assessment
                .counterevidence
                .iter()
                .all(|location| location.role == "test")
            {
                return Err("missing runtime is a gap, not counterevidence".into());
            }
            if !assessment
                .counterevidence
                .iter()
                .any(|location| location.role == "control")
            {
                return Err("suppression requires control-role counterevidence".into());
            }
            if assessment.classification.is_some() {
                return Err("non-reportable claim cannot have finding classification".into());
            }
        }
        Disposition::Duplicate => {
            return Err("only coordinator deduplication can assign duplicate disposition".into())
        }
        Disposition::Deferred => {
            if assessment.classification.is_some() || assessment.proof_gaps.is_empty() {
                return Err("deferred claim requires gaps and no classification".into());
            }
        }
    }
    Ok(())
}

fn claims_observed_runtime(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("runtime reproduction")
        || lower.contains("exploit executed")
        || lower.contains("test executed")
}

#[cfg(test)]
mod tests {
    use super::super::{command::ScanCommand, config::ScanConfig};
    use super::*;

    #[test]
    fn security_reportable_requires_evidenced_root_identity() {
        let record =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let claim: Claim = serde_json::from_value(record["claim"].clone()).unwrap();
        let mut assessment = record["assessment"].clone();
        assessment.as_object_mut().unwrap().remove("rootCause");
        let assessment: Assessment = serde_json::from_value(assessment).unwrap();
        assert!(validate_assessment_semantics(&claim, &assessment).is_err());
    }

    #[test]
    fn security_dangerous_api_only_is_not_reportable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sample.py"), "eval(input)\n").unwrap();
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        let candidate: Claim = serde_json::from_value(serde_json::json!({
            "title":"eval is dangerous", "actor":"", "entrypoint":"", "control":"",
            "sink":"eval", "impact":"", "prerequisites":[], "locations":[], "proofGaps":[]
        }))
        .unwrap();
        assert!(validate_claim(&candidate, &snapshot).is_err());
    }

    #[test]
    fn security_completion_rejects_stale_evidence() {
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: Default::default(),
            skipped: vec![],
            bytes: 0,
            ..Default::default()
        };
        let location = Location {
            path: "made-up.rs".into(),
            start_line: 1,
            end_line: 2,
            content_hash: "fake".into(),
            role: "control".into(),
            snapshot_side: "worktree".into(),
        };
        assert!(validate_location(&location, &snapshot).is_err());
    }

    #[test]
    fn security_confirmed_requires_every_gate_and_no_material_gaps() {
        let source = "fn fixture() {}\n";
        let location = Location {
            path: "source.rs".into(),
            start_line: 1,
            end_line: 1,
            content_hash: super::super::sha256_hex(source.as_bytes()),
            role: "control".into(),
            snapshot_side: "worktree".into(),
        };
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: std::collections::BTreeMap::from([(
                "source.rs".into(),
                super::super::snapshot::SourceFile {
                    hash: location.content_hash.clone(),
                    text: source.into(),
                },
            )]),
            skipped: vec![],
            bytes: source.len() as u64,
            ..Default::default()
        };
        let claim = Claim {
            title: "fixture claim".into(),
            actor: "fixture actor".into(),
            entrypoint: "fixture entry".into(),
            control: "fixture semantics".into(),
            sink: "fixture sink".into(),
            impact: "fixture boundary".into(),
            prerequisites: vec![],
            locations: vec![location.clone()],
            proof_gaps: vec![],
        };
        let gate = Gate {
            satisfied: true,
            rationale: "synthetic gate for mechanical validation only".into(),
            locations: vec![location.clone()],
        };
        let assessment = Assessment {
            root_cause: Some(super::super::root_cause::RootCause {
                root_control: "fixture".into(),
                violated_invariant: "fixture invariant".into(),
                sink_or_decision: "fixture decision".into(),
                attack_path_class: "fixture path".into(),
                control: location.clone(),
                decision: location.clone(),
            }),
            disposition: Disposition::Reportable,
            classification: Some(Classification::Confirmed),
            severity: super::super::FindingSeverity::High,
            severity_rationale: "fixture impact".into(),
            confidence: "high".into(),
            confidence_rationale: "fixture evidence".into(),
            gates: std::array::from_fn(|_| gate.clone()),
            counterevidence: vec![location],
            proof_gaps: vec![],
            reason: "fixture review".into(),
            remediation: "fixture remediation".into(),
        };
        assert!(validate_assessment(&claim, &assessment, &snapshot).is_ok());
        for index in 0..5 {
            let mut invalid = assessment.clone();
            invalid.gates[index].satisfied = false;
            assert!(validate_assessment(&claim, &invalid, &snapshot).is_err());
        }
        let mut uncertain = assessment.clone();
        uncertain.proof_gaps.push("unverified deployment".into());
        assert!(validate_assessment(&claim, &uncertain, &snapshot).is_err());
        uncertain.classification = Some(Classification::Likely);
        assert!(validate_assessment(&claim, &uncertain, &snapshot).is_ok());
        let mut suppressed = assessment;
        suppressed.disposition = Disposition::Suppressed;
        suppressed.classification = None;
        suppressed.counterevidence.clear();
        assert!(validate_assessment(&claim, &suppressed, &snapshot).is_err());
    }

    #[test]
    fn security_missing_authz_requires_real_actor_and_reachability() {
        let mut claim = Claim {
            title: "missing authorization".into(),
            actor: "".into(),
            entrypoint: "http get".into(),
            control: "owner check".into(),
            sink: "document read".into(),
            impact: "cross-account".into(),
            prerequisites: vec![],
            locations: vec![],
            proof_gaps: vec![],
        };
        assert!(validate_claim_shape(&claim).is_err());
        claim.actor = "authenticated stranger".into();
        assert!(validate_claim_shape(&claim).is_err());
        claim.locations.push(Location {
            path: "handler.rs".into(),
            start_line: 1,
            end_line: 1,
            content_hash: "x".into(),
            role: "sink".into(),
            snapshot_side: "worktree".into(),
        });
        assert!(validate_claim_shape(&claim).is_ok());
    }

    #[test]
    fn security_sanitizer_name_without_semantics_does_not_suppress() {
        let location = Location {
            path: "source.rs".into(),
            start_line: 1,
            end_line: 1,
            content_hash: "x".into(),
            role: "sink".into(),
            snapshot_side: "worktree".into(),
        };
        let claim = Claim {
            title: "injection".into(),
            actor: "user".into(),
            entrypoint: "form".into(),
            control: "escape".into(),
            sink: "html".into(),
            impact: "xss".into(),
            prerequisites: vec![],
            locations: vec![location.clone()],
            proof_gaps: vec![],
        };
        let gate = Gate {
            satisfied: false,
            rationale: "function named sanitize is present".into(),
            locations: vec![],
        };
        let assessment = Assessment {
            root_cause: None,
            disposition: Disposition::Suppressed,
            classification: None,
            severity: super::super::FindingSeverity::Low,
            severity_rationale: "n".into(),
            confidence: "low".into(),
            confidence_rationale: "name match".into(),
            gates: std::array::from_fn(|_| gate.clone()),
            counterevidence: vec![location],
            proof_gaps: vec![],
            reason: "sanitize() exists".into(),
            remediation: "none".into(),
        };
        assert_eq!(
            validate_assessment_semantics(&claim, &assessment).unwrap_err(),
            "suppression requires control-role counterevidence"
        );
    }

    #[test]
    fn security_missing_runtime_is_a_gap_not_counterevidence() {
        let location = Location {
            path: "source.rs".into(),
            start_line: 1,
            end_line: 1,
            content_hash: "x".into(),
            role: "test".into(),
            snapshot_side: "worktree".into(),
        };
        let claim = Claim {
            title: "racy debit".into(),
            actor: "two requests".into(),
            entrypoint: "withdraw".into(),
            control: "atomic debit".into(),
            sink: "balance write".into(),
            impact: "double spend".into(),
            prerequisites: vec![],
            locations: vec![location.clone()],
            proof_gaps: vec![],
        };
        let gate = Gate {
            satisfied: false,
            rationale: "no runtime reproduction".into(),
            locations: vec![],
        };
        let assessment = Assessment {
            root_cause: None,
            disposition: Disposition::Suppressed,
            classification: None,
            severity: super::super::FindingSeverity::Low,
            severity_rationale: "n".into(),
            confidence: "low".into(),
            confidence_rationale: "unrun test".into(),
            gates: std::array::from_fn(|_| gate.clone()),
            counterevidence: vec![location],
            proof_gaps: vec![],
            reason: "unit test file exists".into(),
            remediation: "none".into(),
        };
        assert_eq!(
            validate_assessment_semantics(&claim, &assessment).unwrap_err(),
            "missing runtime is a gap, not counterevidence"
        );
    }

    #[test]
    fn security_runtime_claim_requires_observed_result() {
        let location = Location {
            path: "source.rs".into(),
            start_line: 1,
            end_line: 1,
            content_hash: "x".into(),
            role: "sink".into(),
            snapshot_side: "worktree".into(),
        };
        let claim = Claim {
            title: "claim".into(),
            actor: "actor".into(),
            entrypoint: "entry".into(),
            control: "control".into(),
            sink: "sink".into(),
            impact: "impact".into(),
            prerequisites: vec![],
            locations: vec![location.clone()],
            proof_gaps: vec![],
        };
        let mut gate = Gate {
            satisfied: true,
            rationale: "runtime reproduction succeeded".into(),
            locations: vec![location],
        };
        let assessment = Assessment {
            root_cause: None,
            disposition: Disposition::Deferred,
            classification: None,
            severity: super::super::FindingSeverity::Low,
            severity_rationale: "n".into(),
            confidence: "low".into(),
            confidence_rationale: "n".into(),
            gates: std::array::from_fn(|_| gate.clone()),
            counterevidence: vec![],
            proof_gaps: vec!["need review".into()],
            reason: "deferred".into(),
            remediation: "none".into(),
        };
        assert_eq!(
            validate_assessment_semantics(&claim, &assessment).unwrap_err(),
            "runtime claim requires observed result"
        );
        gate.satisfied = false;
        gate.rationale = "runtime not attempted".into();
        let mut deferred = assessment;
        deferred.gates = std::array::from_fn(|_| gate.clone());
        assert!(validate_assessment_semantics(&claim, &deferred).is_ok());
    }
}
