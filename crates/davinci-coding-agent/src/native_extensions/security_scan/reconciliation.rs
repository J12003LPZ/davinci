//! Conservative conflict detection before semantic reconciliation; native-only.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Compare identical causal packets, ignoring only presentation and named gaps.
/// This does not infer semantic equivalence between differently worded claims.
pub(super) fn conflicts(candidates: &[Value]) -> Result<Vec<String>, String> {
    groups(candidates)?.into_iter().map(|group| {
        let ids = group.iter().map(|index| candidates[*index]["candidateId"].as_str()
            .ok_or("reconciliation candidate lacks identity")).collect::<Result<Vec<_>, _>>()?;
        Ok(format!("Unresolved contradictory assessments for candidates {}; source-grounded reconciliation is required", ids.join(", ")))
    }).collect()
}

fn groups(candidates: &[Value]) -> Result<Vec<Vec<usize>>, String> {
    let mut claims = BTreeMap::<String, (BTreeSet<&str>, Vec<usize>)>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let disposition = candidate["assessment"]["disposition"]
            .as_str()
            .unwrap_or("");
        let verdict = match disposition {
            "reportable" => "reportable",
            "suppressed" | "not_applicable" => "rejected",
            _ => continue,
        };
        let claim = &candidate["claim"];
        let mut locations = claim["locations"]
            .as_array()
            .cloned()
            .ok_or("reconciliation claim lacks locations")?;
        locations.sort_by_key(Value::to_string);
        locations.dedup();
        let mut prerequisites = claim["prerequisites"]
            .as_array()
            .cloned()
            .ok_or("reconciliation claim lacks prerequisites")?;
        prerequisites.sort_by_key(Value::to_string);
        prerequisites.dedup();
        let packet = json!({"actor":claim["actor"],"entrypoint":claim["entrypoint"],
            "control":claim["control"],"sink":claim["sink"],"impact":claim["impact"],
            "prerequisites":prerequisites,"locations":locations});
        let key = if candidate["fingerprint"]["version"] == 2 {
            serde_json::to_string(&json!({
                "fingerprint": candidate["fingerprint"]["value"],
                "entrypoint": claim["entrypoint"],
                "locations": locations,
            }))
            .map_err(|_| "cannot identify reconciliation claim")?
        } else {
            serde_json::to_string(&packet).map_err(|_| "cannot identify reconciliation claim")?
        };
        let (verdicts, ids) = claims.entry(key).or_default();
        verdicts.insert(verdict);
        ids.push(index);
    }
    Ok(claims
        .into_values()
        .filter_map(|(verdicts, ids)| (verdicts.len() > 1).then_some(ids))
        .collect())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    assessments: Vec<Decision>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    candidate_index: usize,
    assessment: super::validation::Assessment,
}

fn validate_reply(
    value: &Value,
    group: &[usize],
    candidates: &[Value],
    snapshot: &super::snapshot::Snapshot,
) -> Result<Reply, String> {
    let reply: Reply =
        serde_json::from_value(value.clone()).map_err(|_| "invalid reconciliation schema")?;
    let mut seen = BTreeSet::new();
    for decision in &reply.assessments {
        if !group.contains(&decision.candidate_index) || !seen.insert(decision.candidate_index) {
            return Err("reconciliation contains foreign or repeated candidate".into());
        }
        let claim = serde_json::from_value(candidates[decision.candidate_index]["claim"].clone())
            .map_err(|_| "invalid reconciliation claim")?;
        super::validation::validate_assessment(&claim, &decision.assessment, snapshot)?;
    }
    if seen.len() != group.len() {
        return Err("reconciliation omitted a conflicting candidate".into());
    }
    Ok(reply)
}

pub(super) struct Resolution {
    pub candidates: Vec<Value>,
    pub limitations: Vec<String>,
}

pub(super) fn resolve(
    candidates: &[Value],
    snapshot: &super::snapshot::Snapshot,
    run: &super::controller::RunHandle,
    runner: &super::worker::SecurityWorkerRunner,
    store: Option<&super::store::Store>,
    turns: usize,
    tokens: u64,
) -> Result<Resolution, String> {
    let groups = groups(candidates)?;
    let count = groups.len().max(1);
    let mut result = Resolution {
        candidates: candidates.to_vec(),
        limitations: Vec::new(),
    };
    for (index, group) in groups.iter().enumerate() {
        let packet: Vec<_> = group
            .iter()
            .map(|i| {
                json!({"candidateIndex":i,
            "claim":candidates[*i]["claim"],"assessment":candidates[*i]["assessment"]})
            })
            .collect();
        let objective = json!({"role":"independent-reconciliation","candidates":packet,
            "task":"Reconcile contradictory assessments of these exact claims. Reread decisive immutable source using the snapshot tools. Reassess every candidate independently with all five evidence gates; do not vote or substitute a different claim. Retain unknown conditions as proof gaps and defer when evidence cannot resolve them. No runtime tests have run.",
            "resultSchema":{"assessments":[{"candidateIndex":"one of the supplied indexes, exactly once each",
                "assessment":"Complete native Assessment with disposition reportable|suppressed|not_applicable|deferred, classification confirmed|likely|null, severity and confidence with rationales, exactly five gates with freshly retrieved locations, counterevidence, proofGaps, reason and remediation; same field schema as each input assessment."}]}});
        if serde_json::to_vec(&objective)
            .map_err(|_| "cannot encode reconciliation packet")?
            .len()
            > 256 * 1024
        {
            result
                .limitations
                .push("Reconciliation packet exceeds evidence limit".into());
            continue;
        }
        let allowance = turns / count + usize::from(index < turns % count);
        match runner
            .run_recoverable(
                &objective,
                snapshot,
                run,
                allowance,
                tokens / count as u64,
                store,
                |value| validate_reply(value, group, candidates, snapshot).map(|_| ()),
            )
            .and_then(|value| validate_reply(&value, group, candidates, snapshot))
        {
            Ok(reply) => {
                for decision in reply.assessments {
                    let record = &mut result.candidates[decision.candidate_index];
                    record["fingerprint"] = if let Some(root) = &decision.assessment.root_cause {
                        root.fingerprint()
                    } else {
                        let claim: super::validation::Claim =
                            serde_json::from_value(record["claim"].clone())
                                .map_err(|_| "cannot identify reconciled claim")?;
                        json!({"version":1,"value":super::sha256_hex(
                            &serde_json::to_vec(&claim).map_err(|_| "cannot encode reconciled claim")?)})
                    };
                    record["priorAssessment"] = record["assessment"].clone();
                    record["findingId"] = json!(super::grouping::finding_id(
                        &run.status().scan_id,
                        &record["fingerprint"],
                        record["scope"]
                            .as_str()
                            .ok_or("reconciled claim lacks scope")?
                    )?);
                    record["assessment"] = serde_json::to_value(decision.assessment)
                        .map_err(|_| "cannot encode reconciled assessment")?;
                    record["validation"]["reviewType"] = json!("independent-reconciliation");
                }
            }
            Err(error) => result
                .limitations
                .push(format!("Reconciliation did not complete: {error}")),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_reconciliation_requires_complete_bound_assessments() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("fixture.rs"), "fn fixture() {}\n").unwrap();
        let snapshot = super::super::snapshot::Snapshot::capture(
            root.path(),
            &super::super::command::ScanCommand::parse("").unwrap(),
            &Default::default(),
        )
        .unwrap();
        let candidate =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let candidates = vec![candidate.clone(), candidate.clone()];
        let valid = json!({"assessments":[
            {"candidateIndex":0,"assessment":candidate["assessment"]},
            {"candidateIndex":1,"assessment":candidate["assessment"]}]});
        assert!(validate_reply(&valid, &[0, 1], &candidates, &snapshot).is_ok());
        let mut missing = valid.clone();
        missing["assessments"].as_array_mut().unwrap().pop();
        assert!(validate_reply(&missing, &[0, 1], &candidates, &snapshot).is_err());
        for index in [0, 2] {
            let mut changed = valid.clone();
            changed["assessments"][1]["candidateIndex"] = json!(index);
            assert!(validate_reply(&changed, &[0, 1], &candidates, &snapshot).is_err());
        }
        let mut changed = valid;
        changed["assessments"][0]["assessment"]["gates"][0]["locations"][0]["contentHash"] =
            json!("0".repeat(64));
        assert!(validate_reply(&changed, &[0, 1], &candidates, &snapshot).is_err());
    }

    #[test]
    fn security_reconciliation_preserves_opposing_evidence_without_voting() {
        let mut first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        first["claim"]["prerequisites"] = json!(["Condition one", "Condition two"]);
        let mut rejected = first.clone();
        rejected["claim"]["prerequisites"] = json!(["Condition two", "Condition one"]);
        rejected["claim"]["locations"] = json!([
            first["claim"]["locations"][0],
            first["claim"]["locations"][0]
        ]);
        rejected["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        rejected["claim"]["title"] = json!("Alternate presentation");
        rejected["assessment"]["disposition"] = json!("suppressed");
        assert_eq!(
            conflicts(&[first.clone(), first.clone(), rejected.clone()])
                .unwrap()
                .len(),
            1
        );
        rejected["claim"]["entrypoint"] = json!("A different caller");
        assert!(conflicts(&[first, rejected]).unwrap().is_empty());
    }

    #[test]
    fn security_reconciliation_groups_differently_worded_same_root() {
        let first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut other = first.clone();
        other["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        other["claim"]["title"] = json!("Missing owner predicate at fetch");
        other["claim"]["control"] = json!("authorization predicate at document fetch");
        other["assessment"]["disposition"] = json!("suppressed");
        other["assessment"]["classification"] = Value::Null;
        other["assessment"]["counterevidence"] = other["claim"]["locations"].clone();
        assert_eq!(first["fingerprint"], other["fingerprint"]);
        assert_eq!(conflicts(&[first.clone(), other.clone()]).unwrap().len(), 1);
        other["claim"]["entrypoint"] = json!("A different caller");
        assert!(conflicts(&[first, other]).unwrap().is_empty());
    }

    #[test]
    fn security_reconciliation_keeps_source_bound_wording_as_one_finding() {
        let first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut restated = first.clone();
        restated["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        restated["occurrenceId"] = json!(uuid::Uuid::new_v4().to_string());
        restated["claim"]["title"] = json!("Owner check omitted on the read path");
        restated["claim"]["control"] = json!("the same fixture control under a new sentence");
        restated["claim"]["impact"] = json!("tenant data disclosed through the same sink");
        assert_eq!(first["fingerprint"], restated["fingerprint"]);
        let findings = super::super::grouping::project(&[first, restated]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["occurrences"].as_array().unwrap().len(), 2);
    }
}
