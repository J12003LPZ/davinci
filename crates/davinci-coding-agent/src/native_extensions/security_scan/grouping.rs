//! Canonical native finding projection without discarding candidate evidence.
use serde_json::{json, Value};

pub(super) fn project(candidates: &[Value]) -> Result<Vec<Value>, String> {
    let mut indexes = std::collections::BTreeMap::new();
    let mut memberships = std::collections::BTreeMap::new();
    let mut groups: Vec<Vec<&Value>> = Vec::new();
    for record in candidates
        .iter()
        .filter(|record| record["assessment"]["disposition"] == "reportable")
    {
        let key = (
            record["scope"].as_str().ok_or("finding lacks scope")?,
            record["fingerprint"]["version"]
                .as_u64()
                .ok_or("finding lacks fingerprint version")?,
            record["fingerprint"]["value"]
                .as_str()
                .ok_or("finding lacks fingerprint")?,
        );
        let index = *indexes.entry(key).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        memberships.insert(
            record["candidateId"]
                .as_str()
                .ok_or("finding lacks candidate identity")?,
            index,
        );
        groups[index].push(record);
    }
    let mut candidate_ids = vec![Vec::new(); groups.len()];
    for candidate in candidates {
        let member = if candidate["assessment"]["disposition"] == "duplicate" {
            candidate["assessment"]["duplicateOf"].as_str()
        } else {
            candidate["candidateId"].as_str()
        };
        if let Some(index) = member.and_then(|id| memberships.get(id)) {
            candidate_ids[*index].push(candidate["candidateId"].clone());
        }
    }
    groups
        .iter()
        .enumerate()
        .map(|(index, occurrences)| {
            let first = occurrences[0];
            if occurrences
                .iter()
                .any(|record| record["findingId"] != first["findingId"])
            {
                return Err("root group has inconsistent finding identity".into());
            }
            // A summary is one real assessment, not a vote or a synthesized severity.
            // Prefer confirmed evidence, then severity; ties retain discovery order.
            let representative = occurrences.iter().copied().fold(first, |best, candidate| {
                if rank(candidate) > rank(best) {
                    candidate
                } else {
                    best
                }
            });
            Ok(
                json!({"findingId":first["findingId"],"candidateIds":candidate_ids[index],
            "fingerprint":first["fingerprint"],"scope":first["scope"],
            "representativeCandidateId":representative["candidateId"],
            "claim":representative["claim"],"assessment":representative["assessment"],
            "validation":representative["validation"],"occurrences":occurrences}),
            )
        })
        .collect()
}

fn rank(candidate: &Value) -> (bool, usize) {
    (
        candidate["assessment"]["classification"] == "confirmed",
        super::config::severity_rank(
            candidate["assessment"]["severity"]
                .as_str()
                .unwrap_or("informational"),
        ) as usize,
    )
}

pub(super) fn finding_id(scan: &str, fingerprint: &Value, scope: &str) -> Result<String, String> {
    let version = fingerprint["version"]
        .as_u64()
        .ok_or("missing fingerprint version")?;
    let digest = fingerprint["value"]
        .as_str()
        .ok_or("missing root fingerprint")?;
    Ok(super::identity::record(
        scan,
        0,
        digest,
        &format!("finding/v{version}/{scope}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_root_grouping_preserves_every_candidate_and_occurrence() {
        let first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut second = first.clone();
        second["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        second["occurrenceId"] = json!(uuid::Uuid::new_v4().to_string());
        second["claim"]["entrypoint"] = json!("A distinct affected caller");
        second["assessment"]["classification"] = json!("likely");
        second["assessment"]["proofGaps"] = json!(["Deployment condition unknown"]);
        second["assessment"]["severity"] = json!("critical");
        let duplicate = json!({"candidateId":uuid::Uuid::new_v4().to_string(),"claim":first["claim"],
            "assessment":{"disposition":"duplicate","duplicateOf":first["candidateId"],"reason":"Exact repeated claim"}});
        let findings = project(&[first.clone(), second.clone(), duplicate.clone()]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0]["candidateIds"],
            json!([
                first["candidateId"],
                second["candidateId"],
                duplicate["candidateId"]
            ])
        );
        assert_eq!(findings[0]["occurrences"], json!([first, second]));
        assert_eq!(findings[0]["assessment"]["classification"], "confirmed");
        assert_eq!(findings[0]["assessment"]["severity"], "high");
        let mut report =
            super::super::report_contract::fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["candidates"] = json!([first, second, duplicate]);
        report["findings"] = json!(findings);
        assert!(super::super::report_contract::validate(&report).is_ok());
        let selected = super::super::report::select_finding(
            report.clone(),
            report["findings"][0]["findingId"].as_str(),
        )
        .unwrap();
        assert_eq!(
            selected["selectedFinding"]["occurrences"],
            report["findings"][0]["occurrences"]
        );
        report["failOn"] = json!({"classifications":["likely"],"minimumSeverity":"critical"});
        assert_eq!(
            super::super::report::exit_code(&report),
            1,
            "representative must not hide a likely blocker"
        );
        let text =
            super::super::report::render(&report, super::super::command::ReportFormat::Terminal)
                .unwrap();
        assert!(text.contains("A distinct affected caller"));
        assert!(text.contains("Deployment condition unknown"));
        let sarif: Value = serde_json::from_str(
            &super::super::report::render(&report, super::super::command::ReportFormat::Sarif)
                .unwrap(),
        )
        .unwrap();
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0]["properties"]["davinci.occurrences"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            results[0]["relatedLocations"][0]["properties"]["davinci.occurrenceId"],
            report["candidates"][1]["occurrenceId"]
        );
        for key in ["occurrences", "candidateIds"] {
            let mut lost = report.clone();
            lost["findings"][0][key].as_array_mut().unwrap().pop();
            assert!(
                super::super::report_contract::validate(&lost).is_err(),
                "accepted lost {key}"
            );
        }
        let mut changed = report.clone();
        changed["findings"][0]["occurrences"][1]["assessment"]["severity"] = json!("low");
        assert!(super::super::report_contract::validate(&changed).is_err());
        changed = report;
        changed["findings"][0]["occurrences"] = json!([]);
        assert_eq!(super::super::report::exit_code(&changed), 2);
    }

    #[test]
    fn security_root_grouping_keeps_distinct_wording_as_one_finding() {
        let first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut second = first.clone();
        second["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        second["occurrenceId"] = json!(uuid::Uuid::new_v4().to_string());
        second["claim"]["title"] = json!("Alternate wording of the same root");
        second["claim"]["control"] = json!("owner predicate expressed differently");
        let findings = project(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["occurrences"].as_array().unwrap().len(), 2);
        assert_eq!(findings[0]["fingerprint"], first["fingerprint"]);
        let mut spaced = first.clone();
        spaced["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        spaced["occurrenceId"] = json!(uuid::Uuid::new_v4().to_string());
        spaced["claim"]["title"] = json!("Same root restated with spacing");
        spaced["assessment"]["rootCause"]["rootControl"] = json!(format!(
            "  {}  ",
            first["assessment"]["rootCause"]["rootControl"]
                .as_str()
                .unwrap()
        ));
        let root: super::super::root_cause::RootCause =
            serde_json::from_value(spaced["assessment"]["rootCause"].clone()).unwrap();
        spaced["fingerprint"] = root.fingerprint();
        assert_eq!(first["fingerprint"], spaced["fingerprint"]);
        let findings = project(&[first, spaced]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["occurrences"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn security_root_grouping_keeps_distinct_roots_and_scopes_separate() {
        let first =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut second =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        second["scope"] = json!("supporting");
        assert_eq!(project(&[first.clone(), second.clone()]).unwrap().len(), 2);
        second["scope"] = json!("target");
        second["assessment"]["rootCause"]["violatedInvariant"] = json!("A different boundary");
        let root: super::super::root_cause::RootCause =
            serde_json::from_value(second["assessment"]["rootCause"].clone()).unwrap();
        second["fingerprint"] = root.fingerprint();
        assert_eq!(project(&[first.clone(), second]).unwrap().len(), 2);
        let mut repeated = first.clone();
        repeated["findingId"] = json!(uuid::Uuid::new_v4().to_string());
        assert!(project(&[first, repeated]).is_err());
    }
}
