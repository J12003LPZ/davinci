//! Host-enforced closure invariants for native report publication and recovery.
use serde_json::Value;
use std::collections::BTreeSet;

fn rows<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("report requires {key} array"))
}

fn identity(value: &Value, key: &str) -> Result<String, String> {
    let text = value[key]
        .as_str()
        .ok_or_else(|| format!("record requires {key}"))?;
    let id = uuid::Uuid::parse_str(text).map_err(|_| format!("invalid {key}"))?;
    if id.is_nil() || id.to_string() != text {
        return Err(format!("noncanonical {key}"));
    }
    Ok(text.to_string())
}

pub fn validate_snapshot(
    value: &Value,
    snapshot: &super::snapshot::Snapshot,
) -> Result<(), String> {
    if value["snapshotId"].as_str() != Some(snapshot.id.as_str()) {
        return Err("report snapshot identity mismatch".into());
    }
    let source = &value["source"];
    if source["revisions"] != serde_json::json!(snapshot.revisions)
        || source["currentSide"] != snapshot.current_side()
    {
        return Err("report source revisions differ from snapshot".into());
    }
    if value["partial"] != true
        && (source["indexConflicts"] != serde_json::json!(snapshot.conflicts)
            || source["supportingFiles"] != snapshot.supporting_files.len()
            || source["supportingBaseFiles"] != snapshot.supporting_base_files.len()
            || source["supportingSkipped"] != serde_json::json!(snapshot.supporting_skipped))
    {
        return Err("report supporting source differs from snapshot".into());
    }
    for candidate in rows(value, "candidates")? {
        if let Some(scope) = candidate.get("scope") {
            let claim: super::validation::Claim =
                serde_json::from_value(candidate["claim"].clone())
                    .map_err(|_| "saved claim schema is invalid")?;
            if scope != super::supporting::claim_scope(snapshot, &claim) {
                return Err("finding scope differs from captured control evidence".into());
            }
        }
    }
    let coverage = &value["coverage"];
    let audits = rows(coverage, "audits")?;
    if audits.len() > 2 {
        return Err("report exceeds independent audit limit".into());
    }
    for (index, audit) in audits.iter().enumerate() {
        if audit["audit"].as_u64() != Some(index as u64 + 1) {
            return Err("saved audit identity is out of order".into());
        }
        super::review::AuditCoverage::validate_saved(audit, snapshot)?;
    }
    if coverage["skipped"]
        != serde_json::to_value(&snapshot.skipped).map_err(|_| "cannot encode skipped coverage")?
    {
        return Err("report skipped coverage differs from snapshot".into());
    }
    for (count, paths, files) in [
        ("eligibleFiles", "reviewedPaths", &snapshot.files),
        ("baselineFiles", "reviewedBasePaths", &snapshot.base_files),
    ] {
        if coverage[count].as_u64() != Some(files.len() as u64) {
            return Err("report inventory differs from snapshot".into());
        }
        let paths = rows(coverage, paths)?;
        let mut seen = BTreeSet::new();
        for path in paths {
            let path = path.as_str().ok_or("invalid reviewed path")?;
            if !files.contains_key(path) || !seen.insert(path) {
                return Err("report reviewed path is unknown or repeated".into());
            }
        }
        if value["coverageComplete"] == true && seen.len() != files.len() {
            return Err("complete report omits captured source".into());
        }
        let mut aggregate = BTreeSet::new();
        let deferred_key = if count == "eligibleFiles" {
            "deferredPaths"
        } else {
            "deferredBasePaths"
        };
        let reviewed_key = if count == "eligibleFiles" {
            "reviewedPaths"
        } else {
            "reviewedBasePaths"
        };
        for audit in rows(coverage, "audits")? {
            let mut partition = BTreeSet::new();
            for key in [reviewed_key, deferred_key] {
                let entries = rows(audit, key)?;
                if key == deferred_key && audit["complete"] == true && !entries.is_empty() {
                    return Err("complete audit contains deferred source".into());
                }
                for path in entries {
                    let path = path.as_str().ok_or("invalid audit coverage path")?;
                    if !files.contains_key(path) || !partition.insert(path) {
                        return Err(
                            "audit path is unknown, repeated or both reviewed and deferred".into(),
                        );
                    }
                    if key == reviewed_key {
                        aggregate.insert(path);
                    }
                }
            }
            if partition.len() != files.len() {
                return Err("audit coverage omits captured source".into());
            }
        }
        if seen != aggregate {
            return Err("aggregate coverage disagrees with audit evidence".into());
        }
    }
    Ok(())
}

pub fn validate_mode(value: &Value, mode: super::command::ScanMode) -> Result<(), String> {
    let expected = if mode == super::command::ScanMode::Deep {
        2
    } else {
        1
    };
    let count = rows(&value["coverage"], "audits")?.len();
    if count > expected || (value["coverageComplete"] == true && count != expected) {
        return Err("report audit count does not match requested mode".into());
    }
    Ok(())
}

pub fn validate(value: &Value) -> Result<(), String> {
    super::report_envelope::validate(value)?;
    if value["runtimeTestsExecuted"] != false {
        return Err("native static report requires runtimeTestsExecuted=false".into());
    }
    let complete = value["coverageComplete"]
        .as_bool()
        .ok_or("report requires coverage verdict")?;
    let candidates = rows(value, "candidates")?;
    let findings = rows(value, "findings")?;
    let leads = rows(value, "unresolvedLeads")?;
    let limitations = rows(value, "limitations")?;
    let mut ids = BTreeSet::new();
    let mut canonical = std::collections::BTreeMap::<String, &Value>::new();
    let mut finding_ids = std::collections::BTreeMap::new();
    let mut occurrence_ids = BTreeSet::new();
    let mut deferred = Vec::new();
    for candidate in candidates {
        let disposition = candidate["assessment"]["disposition"]
            .as_str()
            .ok_or("candidate lacks disposition")?;
        let id = identity(candidate, "candidateId")?;
        if !ids.insert(id.clone()) {
            return Err("candidate identity is invalid or repeated".into());
        }
        let claim = serde_json::from_value::<super::validation::Claim>(candidate["claim"].clone())
            .map_err(|_| "saved claim schema is invalid")?;
        super::validation::validate_claim_shape(&claim)?;
        if candidate.get("validation").is_some() {
            validate_static_provenance(candidate)?;
        }
        if candidate["assessment"].get("gates").is_some() {
            validate_static_provenance(candidate)?;
            let assessment = serde_json::from_value::<super::validation::Assessment>(
                candidate["assessment"].clone(),
            )
            .map_err(|_| "saved assessment schema is invalid")?;
            super::validation::validate_assessment_semantics(&claim, &assessment)?;
            if let Some(prior) = candidate.get("priorAssessment") {
                let prior = serde_json::from_value(prior.clone())
                    .map_err(|_| "invalid prior assessment")?;
                super::validation::validate_assessment_semantics(&claim, &prior)?;
            }
        }
        match disposition {
            "reportable" => {
                let finding_id = identity(candidate, "findingId")?;
                let group = (candidate["scope"].clone(), candidate["fingerprint"].clone());
                if finding_ids
                    .insert(finding_id, group.clone())
                    .is_some_and(|prior| prior != group)
                    || !occurrence_ids.insert(identity(candidate, "occurrenceId")?)
                {
                    return Err(
                        "finding identity crosses root groups or occurrence identity is repeated"
                            .into(),
                    );
                }
                let fingerprint = &candidate["fingerprint"];
                let root: super::root_cause::RootCause =
                    serde_json::from_value(candidate["assessment"]["rootCause"].clone())
                        .map_err(|_| "finding lacks canonical root identity")?;
                if *fingerprint != root.fingerprint() {
                    return Err("finding fingerprint differs from root identity".into());
                }
                if fingerprint["version"] != 2
                    || !fingerprint["value"].as_str().is_some_and(|hash| {
                        hash.len() == 64
                            && hash
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                {
                    return Err("invalid finding fingerprint".into());
                }
            }
            "deferred" => deferred.push(candidate),
            "suppressed" | "not_applicable" => (),
            "duplicate" => {
                let original = candidate["assessment"]["duplicateOf"]
                    .as_str()
                    .ok_or("duplicate lacks original")?;
                if original == id || !ids.contains(original) {
                    return Err("duplicate references unknown or future candidate".into());
                }
                let original = canonical
                    .get(original)
                    .ok_or("duplicate must reference a canonical candidate")?;
                if original["claim"] != candidate["claim"] {
                    return Err("exact duplicate changes the claim or affected occurrence".into());
                }
            }
            _ => return Err("unknown candidate disposition".into()),
        }
        if disposition != "duplicate" {
            canonical.insert(id, candidate);
        }
    }
    if super::grouping::project(candidates)? != *findings
        || deferred != leads.iter().collect::<Vec<_>>()
    {
        return Err("report projections disagree with candidate history".into());
    }
    let conflicts = super::reconciliation::conflicts(candidates)?;
    if !conflicts.is_empty()
        && (complete
            || conflicts
                .iter()
                .any(|conflict| !limitations.contains(&Value::String(conflict.clone()))))
    {
        return Err("report hides unresolved contradictory assessments".into());
    }
    if complete {
        if value["partial"] == true || !deferred.is_empty() || !limitations.is_empty() {
            return Err("complete report contains unresolved work".into());
        }
        let coverage = &value["coverage"];
        let audits = rows(coverage, "audits")?;
        if audits.is_empty()
            || audits.iter().any(|audit| audit["complete"] != true)
            || !rows(coverage, "skipped")?.is_empty()
        {
            return Err("complete report lacks complete audit coverage".into());
        }
        for (count, paths) in [
            ("eligibleFiles", "reviewedPaths"),
            ("baselineFiles", "reviewedBasePaths"),
        ] {
            let paths = rows(coverage, paths)?;
            let unique: BTreeSet<_> = paths.iter().filter_map(Value::as_str).collect();
            if unique.len() != paths.len() || coverage[count].as_u64() != Some(paths.len() as u64) {
                return Err("complete report coverage count mismatch".into());
            }
        }
    }
    Ok(())
}

fn validate_static_provenance(candidate: &Value) -> Result<(), String> {
    let review_type = if candidate.get("priorAssessment").is_some() {
        "independent-reconciliation"
    } else {
        "independent-counterreview"
    };
    let expected = serde_json::json!({
        "method":"static",
        "runtimeReproduction":"not_attempted",
        "reviewType":review_type
    });
    if candidate["validation"] != expected {
        return Err("saved validation provenance differs from native static review".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn finding_fixture(id: &str) -> Value {
    let location = serde_json::json!({"path":"fixture.rs","startLine":1,"endLine":1,
        "snapshotSide":"worktree","contentHash":super::sha256_hex(b"fn fixture() {}\n"),"role":"control"});
    let mut record = serde_json::json!({"candidateId":uuid::Uuid::new_v4().to_string(),"findingId":id,
        "scope":"target",
        "validation":{"method":"static","runtimeReproduction":"not_attempted","reviewType":"independent-counterreview"},
        "occurrenceId":uuid::Uuid::new_v4().to_string(),"fingerprint":{"version":1,"value":"0".repeat(64)},
        "claim":{"title":"Stored finding","actor":"fixture actor","entrypoint":"fixture entry",
            "control":"fixture control","sink":"fixture sink","impact":"fixture impact",
            "prerequisites":[],"locations":[location.clone()],"proofGaps":[]},
        "assessment":{"disposition":"reportable","classification":"confirmed","severity":"high",
            "rootCause":{"rootControl":"fixture","violatedInvariant":"fixture invariant",
                "sinkOrDecision":"fixture decision","attackPathClass":"fixture path",
                "control":location.clone(),"decision":location.clone()},
            "severityRationale":"fixture","confidence":"high","confidenceRationale":"fixture",
            "gates":(0..5).map(|_| serde_json::json!({"satisfied":true,"rationale":"fixture","locations":[location.clone()]})).collect::<Vec<_>>(),
            "counterevidence":[],"proofGaps":[],"reason":"fixture","remediation":"fixture"}});
    let root: super::root_cause::RootCause =
        serde_json::from_value(record["assessment"]["rootCause"].clone()).unwrap();
    record["fingerprint"] = root.fingerprint();
    record
}

#[cfg(test)]
pub fn coverage_fixture(snapshot: &super::snapshot::Snapshot, complete: bool) -> Value {
    use serde_json::json;
    let sources: Vec<_> = snapshot.sources().map(|(side,path,file)| json!({
        "location":{"path":path,"startLine":1,"endLine":1,"contentHash":file.hash,"snapshotSide":side,"role":"source"},
        "surface":"fixture","rationale":"Inert fixture","language":"Rust fixture","buildContext":"Offline fixture",
        "unitIds":[],"noUnitReason":"Inert test source"})).collect();
    let audit = super::worker::AuditResult {
        repository_map: serde_json::from_value(
            json!({"sources":sources,"units":[],"environmentAssumptions":[]}),
        )
        .unwrap(),
        reviewed_paths: if complete {
            snapshot.files.keys().cloned().collect()
        } else {
            Vec::new()
        },
        reviewed_base_paths: if complete {
            snapshot.base_files.keys().cloned().collect()
        } else {
            Vec::new()
        },
        candidates: Vec::new(),
        limitations: if complete {
            Vec::new()
        } else {
            vec!["Fixture review incomplete".into()]
        },
    };
    json!({"eligibleFiles":snapshot.files.len(),"reviewedPaths":audit.reviewed_paths,
        "baselineFiles":snapshot.base_files.len(),"reviewedBasePaths":audit.reviewed_base_paths,"skipped":snapshot.skipped,
        "audits":[super::review::AuditCoverage::new(1, &audit, snapshot)]})
}

#[cfg(test)]
pub fn project_fixture(report: &Value) -> Value {
    serde_json::json!(super::grouping::project(report["candidates"].as_array().unwrap()).unwrap())
}

#[cfg(test)]
pub fn fixture(id: &str, generation: u64, complete: bool) -> Value {
    serde_json::json!({"schemaVersion":2,"scanId":id,"generation":generation,"coverageComplete":complete,"snapshotId":"","runtimeTestsExecuted":false,
        "experimental":true,
        "source":{"revisions":null,"currentSide":"worktree","indexConflicts":[],"supportingFiles":0,"supportingBaseFiles":0,"supportingSkipped":[]},
        "failOn":super::config::FailOn::default(),"methodology":super::skills::manifest(),
        "provenance":{"runner":"offline-fixture","qualityEvaluation":"synthetic-only"},
        "budgets":{"maxTokens":24000,"maxTurns":20,"validationReserveRatio":0.5,
            "discoveryPerAudit":{"mappingTurns":0,"mappingTokens":0,"auditTurns":10,"auditTokens":12000},
            "cumulative":{"accountedTokens":0,"requests":0,"remainingTokens":24000,"remainingRequests":20,
                "unsettledRequests":0,"discoveryRequests":0,"discoveryTokens":0,"maxDiscoveryTokens":12000,
                "remainingDiscoveryTokens":12000,"remainingDiscoveryRequests":10,"persistent":false}},
        "usage":[],"hardening":[],"checks":[{"name":"runtime-reproduction","status":"not_attempted"},{"name":"external-analyzers","status":"disabled"}],
        "findings":[],"candidates":[],"unresolvedLeads":[],"limitations":[],
        "coverage":coverage_fixture(&Default::default(), complete)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn security_conflict_report_cannot_claim_complete_coverage_or_invalid_identity() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["source"]["indexConflicts"] = json!([{
            "path":"source.rs","stage":2,"objectId":"a".repeat(40),"mode":"100644"
        }]);
        assert!(
            validate(&report).is_err(),
            "unresolved index conflict is not complete coverage"
        );
        report["coverageComplete"] = json!(false);
        assert!(validate(&report).is_ok());
        report["source"]["indexConflicts"][0]["stage"] = json!(0);
        assert!(validate(&report).is_err());
        report["source"]["indexConflicts"][0]["stage"] = json!(2);
        let duplicate = report["source"]["indexConflicts"][0].clone();
        report["source"]["indexConflicts"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(validate(&report).is_err());
    }

    #[test]
    fn security_duplicate_preserves_distinct_occurrences() {
        let original = finding_fixture(&uuid::Uuid::new_v4().to_string());
        let duplicate = json!({"candidateId":uuid::Uuid::new_v4().to_string(),
            "claim":original["claim"],"assessment":{"disposition":"duplicate",
                "duplicateOf":original["candidateId"],"reason":"Exact identical claim and immutable evidence"}});
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["candidates"] = json!([original.clone(), duplicate.clone()]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_ok());
        for key in ["entrypoint", "actor", "control", "sink", "impact"] {
            report["candidates"][1] = duplicate.clone();
            report["candidates"][1]["claim"][key] = json!("Different affected instance");
            assert!(validate(&report).is_err(), "lost distinct {key}");
        }
        report["candidates"] = json!([original, duplicate.clone(), duplicate.clone()]);
        report["candidates"][2]["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        report["candidates"][2]["assessment"]["duplicateOf"] = duplicate["candidateId"].clone();
        assert!(
            validate(&report).is_err(),
            "accepted noncanonical duplicate chain"
        );
    }

    #[test]
    fn security_completion_rejects_contradictory_assessments() {
        let finding = finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut suppressed = finding_fixture(&uuid::Uuid::new_v4().to_string());
        suppressed["claim"]["title"] = json!("Alternative wording of same path");
        suppressed["assessment"]["disposition"] = json!("suppressed");
        suppressed["assessment"]["classification"] = Value::Null;
        suppressed["assessment"]["counterevidence"] = suppressed["claim"]["locations"].clone();
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["candidates"] = json!([finding.clone(), suppressed]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_err());
        report["coverageComplete"] = json!(false);
        report["limitations"] = json!(super::super::reconciliation::conflicts(
            report["candidates"].as_array().unwrap()
        )
        .unwrap());
        assert!(validate(&report).is_ok());
    }

    #[test]
    fn security_final_report_requires_strict_envelope() {
        let report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        assert!(validate(&report).is_ok());
        for key in report.as_object().unwrap().keys() {
            let mut missing = report.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(validate(&missing).is_err(), "accepted missing {key}");
        }
        for pointer in [
            "",
            "/source",
            "/coverage",
            "/budgets",
            "/provenance",
            "/methodology",
            "/failOn",
        ] {
            let mut extra = report.clone();
            extra
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), json!(true));
            assert!(
                validate(&extra).is_err(),
                "accepted unknown field at {pointer}"
            );
        }
    }

    #[test]
    fn security_report_rejects_inconsistent_budget_and_source_metadata() {
        let report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        for (pointer, replacement) in [
            ("/budgets/cumulative/remainingTokens", json!(24001)),
            ("/budgets/cumulative/unsettledRequests", json!(1)),
            ("/budgets/validationReserveRatio", json!(2)),
            (
                "/provenance/qualityEvaluation",
                json!("production-qualified"),
            ),
        ] {
            let mut altered = report.clone();
            *altered.pointer_mut(pointer).unwrap() = replacement;
            assert!(validate(&altered).is_err(), "accepted {pointer}");
        }
        let snapshot = super::super::snapshot::Snapshot::default();
        for key in [
            "currentSide",
            "revisions",
            "supportingFiles",
            "supportingSkipped",
            "indexConflicts",
        ] {
            let mut altered = report.clone();
            altered["source"][key] = json!("invented");
            assert!(
                validate_snapshot(&altered, &snapshot).is_err(),
                "accepted {key}"
            );
        }
    }

    #[test]
    fn security_report_rejects_untyped_candidate_fields() {
        let candidate = finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, false);
        report["candidates"] = json!([candidate.clone()]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_ok());
        for pointer in ["/unexpected", "/fingerprint/unexpected", "/scope"] {
            let mut changed = candidate.clone();
            if pointer == "/fingerprint/unexpected" {
                changed["fingerprint"]["unexpected"] = json!(true);
            } else {
                changed[pointer.trim_start_matches('/')] = json!("invented");
            }
            report["candidates"] = json!([changed]);
            report["findings"] = project_fixture(&report);
            assert!(validate(&report).is_err(), "accepted {pointer}");
        }
    }

    #[test]
    fn security_report_recomputes_finding_scope() {
        let snapshot = super::super::snapshot::Snapshot::default();
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, false);
        let mut candidate = finding_fixture(&uuid::Uuid::new_v4().to_string());
        // A location absent from the target inventory cannot claim target scope.
        report["candidates"] = json!([candidate.clone()]);
        assert!(validate_snapshot(&report, &snapshot).is_err());
        candidate["scope"] = json!("supporting");
        report["candidates"] = json!([candidate]);
        assert!(validate_snapshot(&report, &snapshot).is_ok());
        // Full evidence validation separately rejects an absent source anchor.
    }

    #[test]
    fn security_report_rejects_invented_runtime_provenance() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["runtimeTestsExecuted"] = json!(false);
        let mut candidate = finding_fixture(&uuid::Uuid::new_v4().to_string());
        candidate["validation"] = json!({"method":"static","runtimeReproduction":"not_attempted","reviewType":"independent-counterreview"});
        report["candidates"] = json!([candidate.clone()]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_ok());
        let mut executed = report.clone();
        executed["runtimeTestsExecuted"] = json!(true);
        assert!(validate(&executed).is_err());
        for provenance in [
            Value::Null,
            json!({"method":"dynamic","runtimeReproduction":"passed","reviewType":"independent-counterreview"}),
            json!({"method":"static","runtimeReproduction":"not_attempted","reviewType":"independent-counterreview","executed":true}),
        ] {
            let mut altered = report.clone();
            altered["candidates"][0]["validation"] = provenance.clone();
            altered["findings"][0]["validation"] = provenance;
            assert!(validate(&altered).is_err());
        }
    }

    #[test]
    fn security_report_coverage_cannot_omit_snapshot_files_without_findings() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("unreviewed.rs"), "fn fixture() {}\n").unwrap();
        let snapshot = super::super::snapshot::Snapshot::capture(
            root.path(),
            &super::super::command::ScanCommand::parse("").unwrap(),
            &Default::default(),
        )
        .unwrap();
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["snapshotId"] = json!(snapshot.id);
        assert!(validate_snapshot(&report, &snapshot).is_err());
        report["coverage"] = coverage_fixture(&snapshot, true);
        assert!(validate_snapshot(&report, &snapshot).is_ok());
        report["coverage"]["audits"][0]["reviewedPaths"] = json!([]);
        assert!(validate_snapshot(&report, &snapshot).is_err());
        report["coverage"]["reviewedPaths"] = json!(["invented.rs"]);
        assert!(validate_snapshot(&report, &snapshot).is_err());
    }

    #[test]
    fn security_archive_reconstructs_unresolved_mapping_assumptions() {
        let snapshot = super::super::snapshot::Snapshot::default();
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        assert!(validate_snapshot(&report, &snapshot).is_ok());
        report["coverage"]["audits"][0]["repositoryMap"]["environmentAssumptions"] =
            json!(["Deployment identity unknown"]);
        assert!(validate_snapshot(&report, &snapshot).is_err());
    }

    #[test]
    fn security_deep_archive_cannot_drop_an_independent_audit() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        assert!(validate_mode(&report, super::super::command::ScanMode::Deep).is_err());
        let mut second = report["coverage"]["audits"][0].clone();
        second["audit"] = json!(2);
        report["coverage"]["audits"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(validate_mode(&report, super::super::command::ScanMode::Deep).is_ok());
        assert!(validate_mode(&report, super::super::command::ScanMode::Quick).is_err());
    }

    #[test]
    fn security_report_contract_rejects_ambiguous_finding_identity() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, false);
        let first = finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut second = first.clone();
        second["candidateId"] = json!(uuid::Uuid::new_v4().to_string());
        report["candidates"] = json!([first, second]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_err());
        let valid = finding_fixture(&uuid::Uuid::new_v4().to_string());
        for (key, replacement) in [
            ("findingId", Value::Null),
            ("occurrenceId", json!(uuid::Uuid::nil().to_string())),
            (
                "candidateId",
                json!(uuid::Uuid::new_v4().simple().to_string()),
            ),
            ("fingerprint", json!({"version":3,"value":"0".repeat(64)})),
            ("fingerprint", json!({"version":2,"value":"0".repeat(64)})),
            ("fingerprint", json!({"version":1,"value":"not-a-hash"})),
        ] {
            let mut changed = valid.clone();
            changed[key] = replacement;
            report["candidates"] = json!([changed]);
            report["findings"] = project_fixture(&report);
            assert!(validate(&report).is_err(), "accepted invalid {key}");
        }
        let mut changed = valid;
        changed["assessment"]["rootCause"]["violatedInvariant"] = json!("Changed invariant");
        report["candidates"] = json!([changed]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report)
            .unwrap_err()
            .contains("fingerprint differs"));
    }

    #[test]
    fn security_report_contract_rejects_confirmation_without_evidence_gates() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, false);
        let finding = json!({"candidateId":uuid::Uuid::new_v4().to_string(),
            "claim":{"title":"Unsupported confirmation"},
            "assessment":{"disposition":"reportable","classification":"confirmed","severity":"high"}});
        report["candidates"] = json!([finding.clone()]);
        report["findings"] = json!([finding]);
        assert!(validate(&report).is_err());
        let valid = finding_fixture(&uuid::Uuid::new_v4().to_string());
        report["candidates"] = json!([valid.clone()]);
        report["findings"] = project_fixture(&report);
        assert!(validate(&report).is_ok());
        for (key, replacement) in [
            ("proofGaps", json!(["Unresolved reachability"])),
            ("gates", json!([])),
            ("classification", json!("verified")),
            ("remediation", json!("")),
        ] {
            let mut changed = valid.clone();
            changed["assessment"][key] = replacement;
            report["candidates"] = json!([changed]);
            report["findings"] = project_fixture(&report);
            assert!(validate(&report).is_err(), "accepted invalid {key}");
        }
    }

    #[test]
    fn security_report_contract_rejects_false_completion_and_lost_candidates() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["coverage"]["eligibleFiles"] = json!(1);
        report["coverage"]["reviewedPaths"] = json!(["a.rs"]);
        assert!(validate(&report).is_ok());
        report["coverage"]["reviewedPaths"] = json!(["a.rs", "a.rs"]);
        assert!(validate(&report).is_err());
        report["coverageComplete"] = json!(false);
        let candidate = json!({"candidateId":uuid::Uuid::new_v4().to_string(),
            "claim":finding_fixture(&uuid::Uuid::new_v4().to_string())["claim"],
            "assessment":{"disposition":"deferred","reason":"Validation did not complete"}});
        report["candidates"] = json!([candidate.clone()]);
        assert!(validate(&report).is_err());
        report["unresolvedLeads"] = json!([candidate]);
        assert!(validate(&report).is_ok());
        report["coverageComplete"] = json!(true);
        assert!(validate(&report).is_err());
    }

    #[test]
    fn security_completion_rejects_unclosed_or_stale_candidates() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, true);
        report["coverage"]["eligibleFiles"] = json!(1);
        report["coverage"]["reviewedPaths"] = json!(["a.rs"]);
        let candidate = json!({"candidateId":uuid::Uuid::new_v4().to_string(),
            "claim":finding_fixture(&uuid::Uuid::new_v4().to_string())["claim"],
            "assessment":{"disposition":"deferred","reason":"Validation did not complete"}});
        report["candidates"] = json!([candidate.clone()]);
        report["unresolvedLeads"] = json!([]);
        report["coverageComplete"] = json!(true);
        assert!(validate(&report).is_err());
        report["coverageComplete"] = json!(false);
        report["unresolvedLeads"] = json!([candidate]);
        assert!(validate(&report).is_ok());
    }

    #[test]
    fn security_report_contract_rejects_duplicate_identity_and_broken_projection() {
        let mut report = fixture(&uuid::Uuid::new_v4().to_string(), 1, false);
        let id = uuid::Uuid::new_v4().to_string();
        let mut candidate = finding_fixture(&uuid::Uuid::new_v4().to_string());
        candidate["candidateId"] = json!(id);
        candidate["assessment"]["disposition"] = json!("suppressed");
        candidate["assessment"]["classification"] = Value::Null;
        candidate["assessment"]["counterevidence"] = candidate["claim"]["locations"].clone();
        report["candidates"] = json!([candidate.clone(), candidate.clone()]);
        assert!(validate(&report).is_err());
        report["candidates"] = json!([candidate]);
        assert!(validate(&report).is_ok());
        report["findings"] = report["candidates"].clone();
        assert!(validate(&report).is_err());
        report["findings"] = json!([]);
        report["candidates"][0]["assessment"] = json!({"disposition":"duplicate","duplicateOf":id});
        assert!(validate(&report).is_err());
        report["candidates"][0]["assessment"]["duplicateOf"] =
            json!(uuid::Uuid::new_v4().to_string());
        assert!(validate(&report).is_err());
    }
}
