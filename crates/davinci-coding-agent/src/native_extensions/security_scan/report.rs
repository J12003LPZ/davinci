//! Export projections from canonical review data, never from model Markdown.
use super::command::ReportFormat;
use serde_json::{json, Value};

/// Read-only view of a schema-1 artifact. Never a v2 confirmation or resume source.
pub fn project_legacy_v1(bytes: &[u8]) -> Result<Value, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "invalid legacy security report")?;
    if value["schemaVersion"] == 2 {
        return Err("v2 report is not a legacy v1 consumer document".into());
    }
    let validated = value["validated"] == true;
    Ok(json!({
        "schemaVersion": 1,
        "readOnly": true,
        "status": "legacy-readonly",
        "coverageComplete": false,
        "legacyValidatedFlag": validated,
        "findings": value.get("findings").cloned().unwrap_or_else(|| json!([])),
        "limitations": ["legacy v1 artifacts are read-only and are not v2 confirmation"],
        "qualityEvaluation": "legacy-unconfirmed",
    }))
}

pub fn select_finding(mut report: Value, finding_id: Option<&str>) -> Result<Value, String> {
    let Some(id) = finding_id else {
        return Ok(report);
    };
    if report["schemaVersion"] != 2 {
        return Err("finding selection requires a native v2 report".into());
    }
    let matches: Vec<_> = report["findings"]
        .as_array()
        .ok_or("findings are not available yet")?
        .iter()
        .filter(|finding| finding["findingId"].as_str() == Some(id))
        .cloned()
        .collect();
    if matches.len() != 1 {
        return Err("finding identity is missing or ambiguous in this scan".into());
    }
    // Keep source identity, coverage, limitations and the full candidate history;
    // this is a read-only projection, never a new sealed canonical report.
    report["selectedFinding"] = matches[0].clone();
    report["selectedFindingId"] = json!(id);
    Ok(report)
}

pub fn exit_code(value: &Value) -> i32 {
    if value["status"] == "cancelled" {
        return 130;
    }
    if value["coverageComplete"] != true
        || value["status"]
            .as_str()
            .is_some_and(|status| status != "completed")
    {
        return 2;
    }
    let policy = if value.get("failOn").is_none() {
        super::config::FailOn::default()
    } else {
        let Ok(policy) = serde_json::from_value::<super::config::FailOn>(value["failOn"].clone())
        else {
            return 2;
        };
        if policy.validate().is_err() {
            return 2;
        }
        policy
    };
    if value["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| {
            finding
                .get("occurrences")
                .is_some_and(|rows| rows.as_array().is_none_or(|rows| rows.is_empty()))
        })
    }) {
        return 2;
    }
    if value["findings"].as_array().is_some_and(|findings| {
        findings.iter().flat_map(occurrences).any(|finding| {
            if finding["scope"] == "supporting" {
                return false;
            }
            let assessment = &finding["assessment"];
            assessment["classification"].as_str().is_some_and(|class| {
                policy
                    .classifications
                    .iter()
                    .any(|allowed| allowed == class)
            }) && super::config::severity_rank(
                assessment["severity"].as_str().unwrap_or("informational"),
            ) >= super::config::severity_rank(&policy.minimum_severity)
        })
    }) {
        1
    } else {
        0
    }
}

fn occurrences(finding: &Value) -> Vec<&Value> {
    finding["occurrences"]
        .as_array()
        .map(|rows| rows.iter().collect())
        .unwrap_or_else(|| vec![finding])
}

pub fn sanitize(value: &mut Value) {
    match value {
        Value::String(text) => {
            *text = super::redaction::text(text)
                .chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
                .collect()
        }
        Value::Array(values) => values.iter_mut().for_each(sanitize),
        Value::Object(values) => values.values_mut().for_each(sanitize),
        _ => (),
    }
}

pub fn render(value: &Value, format: ReportFormat) -> Result<String, String> {
    let mut value = value.clone();
    sanitize(&mut value);
    if format == ReportFormat::Json {
        return serde_json::to_string_pretty(&value).map_err(|e| e.to_string());
    }
    let empty = Vec::new();
    let selected;
    let findings = if let Some(finding) = value.get("selectedFinding") {
        selected = vec![finding.clone()];
        &selected
    } else {
        value["findings"].as_array().unwrap_or(&empty)
    };
    if format == ReportFormat::Sarif {
        let results: Vec<_> = findings.iter().filter(|finding| finding["assessment"]["disposition"] == "reportable").map(|finding| {
            let assessment = &finding["assessment"];
            let members = occurrences(finding);
            let locations: Vec<_> = members.iter().flat_map(|member| member["claim"]["locations"].as_array().unwrap_or(&empty).iter().map(move |location| (*member, location))).map(|(member, location)| {
                let path = location["path"].as_str().unwrap_or("");
                let uri = if super::snapshot::relative_scope(path).is_ok() {
                    path.bytes().map(|b| {
                        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }
                    }).collect::<String>()
                } else {
                    "omitted-invalid-path".into()
                };
                json!({"physicalLocation":{"artifactLocation":{"uri":uri},"region":{"startLine":location["startLine"],"endLine":location["endLine"]}},
                    "properties":{"davinci.snapshotSide":location["snapshotSide"],"davinci.contentHash":location["contentHash"],"davinci.role":location["role"],"davinci.occurrenceId":member["occurrenceId"]}})
            }).collect();
            let mut result = json!({"ruleId":"native-security-review", "level": if assessment["classification"] == "confirmed" {
                match assessment["severity"].as_str() { Some("critical" | "high") => "error", Some("medium") => "warning", _ => "note" }
            } else { "note" }, "message":{"text":finding["claim"]["title"]}, "locations":locations.iter().take(1).collect::<Vec<_>>(),
                "relatedLocations":locations.iter().skip(1).enumerate().map(|(index, location)| {
                    let mut location = location.clone();
                    location["id"] = json!(index + 1);
                    location
                }).collect::<Vec<_>>(),
                "properties":{"davinci.classification":assessment["classification"],"davinci.confidence":assessment["confidence"],"davinci.proofGaps":assessment["proofGaps"]}});
            if let (Some(version), Some(fingerprint)) = (finding["fingerprint"]["version"].as_u64(), finding["fingerprint"]["value"].as_str()) {
                let scheme = if version == 2 { "root" } else { "claim" };
                result["partialFingerprints"] = json!({format!("davinci.{scheme}/v{version}"):fingerprint});
            }
            if finding.get("occurrences").is_some() {
                result["properties"]["davinci.representativeCandidateId"] = finding["representativeCandidateId"].clone();
                result["properties"]["davinci.candidateIds"] = finding["candidateIds"].clone();
                result["properties"]["davinci.occurrences"] = json!(members.iter().map(|member| json!({
                    "occurrenceId":member["occurrenceId"],"candidateId":member["candidateId"],
                    "entrypoint":member["claim"]["entrypoint"],"classification":member["assessment"]["classification"],
                    "severity":member["assessment"]["severity"],"confidence":member["assessment"]["confidence"],
                    "proofGaps":member["assessment"]["proofGaps"],"validation":member["validation"]})).collect::<Vec<_>>());
            }
            result
        }).collect();
        return serde_json::to_string_pretty(&json!({"version":"2.1.0","$schema":"https://json.schemastore.org/sarif-2.1.0.json",
            "runs":[{"tool":{"driver":{"name":"davinci-native-security","rules":[{"id":"native-security-review"}]}},
                "invocations":[{"executionSuccessful":value["coverageComplete"] == true}],"results":results,
                "properties":{"experimental":true,"coverage":value["coverage"],"limitations":value["limitations"],"usage":value["usage"]}}]})).map_err(|e| e.to_string());
    }
    let mut output = format!(
        "Experimental security review {}\nCoverage: {}\n",
        value["scanId"].as_str().unwrap_or("unknown"),
        if value["coverageComplete"] == true {
            "complete for captured scope"
        } else {
            "incomplete"
        }
    );
    if let Some(records) = value.get("usage") {
        output.push_str(&render_usage(records)?);
    }
    for audit in value["coverage"]["audits"].as_array().unwrap_or(&empty) {
        output.push_str(&format!(
            "Audit {}: {} ({} current, {} baseline files reviewed)\n",
            audit["audit"],
            if audit["complete"] == true {
                "complete"
            } else {
                "incomplete"
            },
            audit["reviewedPaths"].as_array().map_or(0, Vec::len),
            audit["reviewedBasePaths"].as_array().map_or(0, Vec::len),
        ));
        for (key, label) in [
            ("deferredPaths", "current"),
            ("deferredBasePaths", "baseline"),
            ("unmappedPaths", "current map source"),
            ("unmappedBasePaths", "baseline map source"),
            ("unreviewedMappedUnits", "assigned review unit"),
            ("unresolvedMappingAssumptions", "mapping assumption"),
        ] {
            let paths = audit[key].as_array().unwrap_or(&empty);
            for path in paths.iter().take(50) {
                output.push_str(&format!(
                    "  Unreviewed {label}: {}\n",
                    path.as_str().unwrap_or("unknown")
                ));
            }
            if paths.len() > 50 {
                output.push_str(&format!(
                    "  {} additional unreviewed {label} entries are listed in the JSON report.\n",
                    paths.len() - 50
                ));
            }
        }
        if let Some(map) = audit.get("repositoryMap") {
            let units = map["units"].as_array().unwrap_or(&empty);
            let incomplete_units = units
                .iter()
                .filter(|unit| {
                    unit["disposition"] != "reviewed"
                        || unit["unknowns"]
                            .as_array()
                            .is_some_and(|unknowns| !unknowns.is_empty())
                })
                .count();
            let unknown_surfaces = map["sources"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter(|source| source["surface"] == "unknown")
                .count();
            let assumptions = map["environmentAssumptions"].as_array().unwrap_or(&empty);
            output.push_str(&format!("  Map: {} units, {incomplete_units} unresolved units, {unknown_surfaces} unknown product surfaces, {} environment assumptions\n",
                units.len(), assumptions.len()));
        }
        for limitation in audit["limitations"].as_array().unwrap_or(&empty) {
            output.push_str(&format!(
                "  Audit limitation: {}\n",
                limitation.as_str().unwrap_or("unspecified")
            ));
        }
    }
    for group in findings {
        output.push_str(&render_finding_details(group));
    }
    for item in value["hardening"].as_array().unwrap_or(&empty) {
        output.push_str(&format!(
            "\nHardening recommendation: {}\n",
            item["title"]
                .as_str()
                .or_else(|| item["recommendation"].as_str())
                .unwrap_or("unspecified")
        ));
    }
    for lead in value["unresolvedLeads"].as_array().unwrap_or(&empty) {
        output.push_str(&format!(
            "\nUnresolved lead: {}\n{}\n",
            lead["claim"]["title"].as_str().unwrap_or("untitled"),
            lead["assessment"]["reason"]
                .as_str()
                .unwrap_or("not validated")
        ));
        for gap in lead["assessment"]["proofGaps"].as_array().unwrap_or(&empty) {
            output.push_str(&format!(
                "Proof gap: {}\n",
                gap.as_str().unwrap_or("unspecified")
            ));
        }
    }
    for limitation in value["limitations"].as_array().unwrap_or(&empty) {
        output.push_str(&format!(
            "Limitation: {}\n",
            limitation.as_str().unwrap_or("")
        ));
    }
    if findings.is_empty() {
        output.push_str("No validated findings reported. This is not a guarantee of security.\n");
    }
    Ok(output)
}

/// Shared terminal and TUI details preserve all occurrence evidence and caveats.
pub fn render_finding_details(group: &Value) -> String {
    let mut group = group.clone();
    sanitize(&mut group);
    let mut output = String::new();
    let empty = Vec::new();
    if let Some(rows) = group["occurrences"].as_array() {
        output.push_str(&format!(
            "\nFinding {}: {} affected occurrences\n",
            group["findingId"].as_str().unwrap_or("unknown"),
            rows.len()
        ));
    }
    for finding in occurrences(&group) {
        if let Some(id) = finding["occurrenceId"].as_str() {
            output.push_str(&format!("Occurrence {id}\n"));
        }
        let assessment = &finding["assessment"];
        if finding["scope"] == "supporting" {
            output.push_str("\nSupporting-source finding: outside the requested target scope\n");
        }
        output.push_str(&format!(
            "\n{} / {}: {}\n{}\n",
            assessment["classification"]
                .as_str()
                .unwrap_or_else(|| assessment["disposition"].as_str().unwrap_or("deferred")),
            assessment["severity"].as_str().unwrap_or("unassessed"),
            finding["claim"]["title"].as_str().unwrap_or(""),
            assessment["reason"].as_str().unwrap_or("")
        ));
        for (key, label) in [
            ("actor", "Actor"),
            ("entrypoint", "Entry point"),
            ("control", "Control"),
            ("sink", "Sink"),
            ("impact", "Impact"),
        ] {
            if let Some(text) = finding["claim"][key].as_str() {
                output.push_str(&format!("{label}: {text}\n"));
            }
        }
        for location in finding["claim"]["locations"].as_array().unwrap_or(&empty) {
            output.push_str(&format!(
                "Evidence: {}:{}-{} [{}; {}]\n",
                location["path"].as_str().unwrap_or("unknown"),
                location["startLine"],
                location["endLine"],
                location["snapshotSide"].as_str().unwrap_or("unknown"),
                location["role"].as_str().unwrap_or("source")
            ));
        }
        for (key, label) in [
            ("prerequisites", "Prerequisite"),
            ("proofGaps", "Claim gap"),
        ] {
            for entry in finding["claim"][key].as_array().unwrap_or(&empty) {
                output.push_str(&format!(
                    "{label}: {}\n",
                    entry.as_str().unwrap_or("unspecified")
                ));
            }
        }
        for gap in assessment["proofGaps"].as_array().unwrap_or(&empty) {
            output.push_str(&format!(
                "Proof gap: {}\n",
                gap.as_str().unwrap_or("unspecified")
            ));
        }
        if let Some(remediation) = assessment["remediation"].as_str() {
            output.push_str(&format!("Remediation: {remediation}\n"));
        }
    }
    output
}

fn render_usage(value: &Value) -> Result<String, String> {
    let records: Vec<super::usage::RequestUsage> =
        serde_json::from_value(value.clone()).map_err(|_| "invalid security usage records")?;
    let accounted = records.iter().fold(0u64, |sum, record| {
        sum.saturating_add(record.accounted_tokens)
    });
    let wall = records
        .iter()
        .fold(0u64, |sum, record| sum.saturating_add(record.wall_time_ms));
    let measured = if records.is_empty() {
        None
    } else {
        records.iter().try_fold(0u64, |sum, record| {
            sum.checked_add(record.measured.as_ref()?.total)
        })
    };
    let cost = if records.is_empty() {
        None
    } else {
        records.iter().try_fold(0.0f64, |sum, record| {
            let cost = record.estimated_cost_usd?;
            let total = sum + cost;
            (cost >= 0.0 && total.is_finite()).then_some(total)
        })
    };
    Ok(format!("Provider requests: {}\nAccounted tokens: {} (includes reservations for unknown usage)\nMeasured tokens: {}\nProvider wall time: {} ms\nEstimated cost (USD): {}\n",
        records.len(), accounted, measured.map_or_else(|| "unknown".into(), |total| total.to_string()), wall,
        cost.map_or_else(|| "unknown".into(), |cost| format!("{cost:.6}"))))
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires explicitly provisioned DAVINCI_SARIF_PYTHON test environment"]
    fn security_sarif_official_schema() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut packets = Vec::new();
        for status in ["completed", "failed", "cancelled", "interrupted"] {
            for classification in ["confirmed", "likely"] {
                let mut finding = json!({"findingId":"fixture", "fingerprint":{"version":2,"value":"fixture-hash"},
                    "claim":{"title":"Fixture finding", "locations":[
                        {"path":"src/日本語 file.rs","startLine":1,"endLine":2,"snapshotSide":"base","role":"source","contentHash":"base-hash"},
                        {"path":"src/new.rs","startLine":2,"endLine":3,"snapshotSide":"head","role":"sink","contentHash":"head-hash"}]},
                    "assessment":{"disposition":"reportable","classification":classification,"severity":"high",
                        "confidence":"high","proofGaps":[]}});
                finding["scope"] = json!("target");
                finding["candidateId"] = json!("first-candidate");
                finding["occurrenceId"] = json!("first-occurrence");
                let mut second = finding.clone();
                second["candidateId"] = json!("second-candidate");
                second["occurrenceId"] = json!("second-occurrence");
                second["claim"]["entrypoint"] = json!("Second affected caller");
                let finding = super::super::grouping::project(&[finding, second])
                    .unwrap()
                    .remove(0);
                let mut report = json!({"schemaVersion":2,"status":status,"coverageComplete":status == "completed",
                    "findings":[finding.clone()],"coverage":{},"limitations":[],"usage":[]});
                for selected in [false, true] {
                    if selected {
                        report["selectedFinding"] = finding.clone();
                    }
                    packets.push(
                        serde_json::from_str::<Value>(
                            &render(&report, ReportFormat::Sarif).unwrap(),
                        )
                        .unwrap(),
                    );
                }
                report.as_object_mut().unwrap().remove("selectedFinding");
                report["findings"] = json!([]);
                packets.push(
                    serde_json::from_str::<Value>(&render(&report, ReportFormat::Sarif).unwrap())
                        .unwrap(),
                );
            }
        }
        let python = std::env::var_os("DAVINCI_SARIF_PYTHON")
            .expect("set DAVINCI_SARIF_PYTHON to the provisioned validator");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/validate-security-sarif.py");
        let mut child = Command::new(python)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&packets).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("Validated 24 native SARIF exports offline"));
    }

    #[test]
    fn security_sarif_preserves_snapshot_evidence_and_fingerprints() {
        let report = json!({"coverageComplete":false,"findings":[{
            "fingerprint":{"version":2,"value":"causal-fixture"},
            "claim":{"title":"Boundary failure","locations":[
                {"path":"src/a b.rs","startLine":2,"endLine":3,"snapshotSide":"base","role":"control","contentHash":"old-hash"},
                {"path":"src/a b.rs","startLine":5,"endLine":6,"snapshotSide":"head","role":"sink","contentHash":"new-hash"}]},
            "assessment":{"disposition":"reportable","classification":"likely","severity":"high"}}]});
        let rendered: Value =
            serde_json::from_str(&render(&report, ReportFormat::Sarif).unwrap()).unwrap();
        let result = &rendered["runs"][0]["results"][0];
        assert_eq!(result["locations"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["relatedLocations"][0]["properties"]["davinci.snapshotSide"],
            "head"
        );
        assert_eq!(
            result["locations"][0]["properties"]["davinci.contentHash"],
            "old-hash"
        );
        assert_eq!(
            result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "src/a%20b.rs"
        );
        assert_eq!(
            result["partialFingerprints"]["davinci.root/v2"],
            "causal-fixture"
        );
        assert_eq!(result["level"], "note");
    }

    #[test]
    fn security_supporting_findings_do_not_expand_target_failure_policy() {
        let mut value = json!({"coverageComplete":true,"findings":[{"scope":"supporting",
            "claim":{"title":"Outside target"},"assessment":{"disposition":"reportable","classification":"confirmed","severity":"high"}}]});
        assert_eq!(exit_code(&value), 0);
        let terminal = render(&value, ReportFormat::Terminal).unwrap();
        assert!(terminal.contains("outside the requested target scope"));
        value["findings"][0]["scope"] = json!("target");
        assert_eq!(exit_code(&value), 1);
    }

    use super::*;

    #[test]
    fn security_sarif_uses_valid_relative_locations() {
        let report = json!({"coverageComplete":false,"findings":[{
            "claim":{"title":"Boundary failure","locations":[
                {"path":"src/ok.rs","startLine":1,"endLine":1,"snapshotSide":"worktree","role":"source","contentHash":"hash"},
                {"path":"../etc/passwd","startLine":1,"endLine":1,"snapshotSide":"worktree","role":"sink","contentHash":"hash"},
                {"path":"C:\\\\Windows\\\\win.ini","startLine":1,"endLine":1,"snapshotSide":"worktree","role":"sink","contentHash":"hash"}]},
            "assessment":{"disposition":"reportable","classification":"likely","severity":"high"}}]});
        let rendered: Value =
            serde_json::from_str(&render(&report, ReportFormat::Sarif).unwrap()).unwrap();
        let result = &rendered["runs"][0]["results"][0];
        let uri = result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
            .as_str()
            .unwrap();
        assert_eq!(uri, "src/ok.rs");
        assert!(!uri.contains(".."));
        assert!(!uri.contains(':'));
        let related = result["relatedLocations"].as_array().unwrap();
        for location in related {
            let uri = location["physicalLocation"]["artifactLocation"]["uri"]
                .as_str()
                .unwrap();
            assert_eq!(uri, "omitted-invalid-path");
            assert!(!uri.contains("etc/passwd"));
            assert!(!uri.contains("Windows"));
        }
    }

    #[test]
    fn security_report_redacts_secrets_and_terminal_controls() {
        let mut value = json!({"findings":[{"claim":{"title":"api_key = 'fixture-sensitive'\u{1b}]0;stolen"}}]});
        sanitize(&mut value);
        let text = value.to_string();
        assert!(!text.contains("fixture-sensitive"));
        assert!(!text.contains('\u{1b}'));
        let rendered = render(&value, ReportFormat::Json).unwrap();
        assert!(!rendered.contains("fixture-sensitive"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn security_finding_drilldown_preserves_scan_verdict_and_identity() {
        let id = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let report = json!({"schemaVersion":2,"scanId":"fixture","generation":3,"coverageComplete":true,
            "findings":[{"findingId":id,"claim":{"title":"selected low"},"assessment":{"classification":"confirmed","severity":"low"}},
                {"findingId":other,"claim":{"title":"other high"},"assessment":{"classification":"confirmed","severity":"high"}}]});
        let selected = select_finding(report.clone(), Some(&id)).unwrap();
        assert_eq!(selected["scanId"], report["scanId"]);
        assert_eq!(selected["generation"], 3);
        assert_eq!(selected["findings"], report["findings"]);
        assert_eq!(exit_code(&selected), 1);
        let text = render(&selected, ReportFormat::Terminal).unwrap();
        assert!(text.contains("selected low"));
        assert!(!text.contains("other high"));
        assert!(select_finding(report.clone(), Some("missing")).is_err());
        let mut ambiguous = report;
        ambiguous["findings"][1]["findingId"] = json!(id);
        assert!(select_finding(ambiguous, Some(&id)).is_err());
    }
    #[test]
    fn security_terminal_report_explains_independent_audit_coverage() {
        let report = json!({"scanId":"fixture","coverageComplete":false,"findings":[],
            "coverage":{"audits":[{"audit":1,"complete":false,"reviewedPaths":["new.rs"],
                "reviewedBasePaths":[],"deferredPaths":["other.rs"],"deferredBasePaths":["removed.rs"],
                "unmappedPaths":["unmapped.rs"],"unreviewedMappedUnits":["authorize-request"],
                "repositoryMap":{"units":[{"disposition":"deferred","unknowns":[]}],
                    "sources":[{"surface":"unknown"}],"environmentAssumptions":["deployment"]},
                "limitations":["Caller not resolved"]}]}});
        let text = render(&report, ReportFormat::Terminal).unwrap();
        for required in [
            "Audit 1: incomplete",
            "other.rs",
            "removed.rs",
            "Caller not resolved",
            "Unreviewed current map source: unmapped.rs",
            "Unreviewed assigned review unit: authorize-request",
            "Map: 1 units, 1 unresolved units, 1 unknown product surfaces, 1 environment assumptions",
        ] {
            assert!(text.contains(required), "missing {required}: {text}");
        }
    }

    #[test]
    fn security_terminal_usage_distinguishes_reservations_from_measurements() {
        let usage = super::super::usage::RequestUsage::new(None, 1000, 25, false);
        let report =
            json!({"scanId":"fixture", "coverageComplete":false, "findings":[], "usage":[usage]});
        let text = render(&report, ReportFormat::Terminal).unwrap();
        for required in [
            "Provider requests: 1",
            "Accounted tokens: 1000",
            "Measured tokens: unknown",
            "Estimated cost (USD): unknown",
        ] {
            assert!(text.contains(required), "missing {required}: {text}");
        }
    }
    #[test]
    fn security_terminal_report_exposes_causal_path_remediation_and_leads() {
        let report = json!({"scanId":"fixture","coverageComplete":false,
            "findings":[{"claim":{"title":"Boundary failure","actor":"tenant user","entrypoint":"POST /item","control":"ownership check","sink":"storage read","impact":"cross-tenant disclosure","locations":[{"path":"src/auth.rs","startLine":4,"endLine":8,"snapshotSide":"base","role":"control"}]},
            "assessment":{"classification":"likely","severity":"high","reason":"Static path identified","remediation":"Check final resource ownership","proofGaps":["Deployment setting unknown"]}}],
            "unresolvedLeads":[{"claim":{"title":"Unresolved caller"},"assessment":{"reason":"Caller authority unknown","proofGaps":["Call graph incomplete"]}}]});
        let text = render(&report, ReportFormat::Terminal).unwrap();
        for required in [
            "tenant user",
            "POST /item",
            "ownership check",
            "storage read",
            "cross-tenant disclosure",
            "src/auth.rs:4-8",
            "base",
            "Check final resource ownership",
            "Deployment setting unknown",
            "Unresolved caller",
            "Call graph incomplete",
        ] {
            assert!(text.contains(required), "missing {required}: {text}");
        }
    }
    #[test]
    fn security_exit_policy_honors_likely_and_failure_precedence() {
        let mut value = json!({"coverageComplete":true,"findings":[{"assessment":{"classification":"likely","severity":"high","disposition":"reportable"}}],"failOn":{"classifications":["confirmed","likely"],"minimumSeverity":"high"}});
        assert_eq!(exit_code(&value), 1);
        value["failOn"]["classifications"] = json!(["confirmed"]);
        assert_eq!(exit_code(&value), 0);
        value["findings"][0]["assessment"]["classification"] = json!("confirmed");
        assert_eq!(exit_code(&value), 1);
        value["coverageComplete"] = json!(false);
        assert_eq!(exit_code(&value), 2);
        value["status"] = json!("cancelled");
        assert_eq!(exit_code(&value), 130);
        value["coverageComplete"] = json!(true);
        value["status"] = json!("failed");
        assert_eq!(exit_code(&value), 2);
    }
    #[test]
    fn security_report_separates_confirmed_likely_hardening_and_leads() {
        let value = json!({
            "scanId":"fixture","coverageComplete":true,
            "findings":[
                {"findingId":"a","assessment":{"classification":"confirmed","severity":"high","disposition":"reportable"},"claim":{"title":"Confirmed owner miss"}},
                {"findingId":"b","assessment":{"classification":"likely","severity":"medium","disposition":"reportable","proofGaps":["deploy unknown"]},"claim":{"title":"Likely race"}}
            ],
            "hardening":[{"title":"Add an allowlist at the process boundary"}],
            "unresolvedLeads":[{"claim":{"title":"Unresolved caller"},"assessment":{"reason":"Need more evidence"}}]
        });
        let text = render(&value, ReportFormat::Terminal).unwrap();
        assert!(text.contains("confirmed"));
        assert!(text.contains("Confirmed owner miss"));
        assert!(text.contains("likely"));
        assert!(text.contains("Likely race"));
        assert!(text.contains("Hardening recommendation"));
        assert!(text.contains("Add an allowlist at the process boundary"));
        assert!(text.contains("Unresolved lead"));
        assert!(text.contains("Unresolved caller"));
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(encoded["findings"].as_array().unwrap().len(), 2);
        assert_eq!(encoded["hardening"].as_array().unwrap().len(), 1);
        assert_eq!(encoded["unresolvedLeads"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn security_report_counts_match_canonical_findings() {
        let value = json!({
            "coverageComplete":true,
            "findings":[
                {"findingId":"a","assessment":{"classification":"confirmed","severity":"high","disposition":"reportable"},
                 "claim":{"title":"one","locations":[{"path":"a.rs","startLine":1,"endLine":1}]}},
                {"findingId":"b","assessment":{"classification":"likely","severity":"low","disposition":"reportable"},
                 "claim":{"title":"two","locations":[{"path":"b.rs","startLine":1,"endLine":1}]}}
            ]
        });
        assert_eq!(value["findings"].as_array().unwrap().len(), 2);
        let sarif: Value =
            serde_json::from_str(&render(&value, ReportFormat::Sarif).unwrap()).unwrap();
        assert_eq!(sarif["runs"][0]["results"].as_array().unwrap().len(), 2);
        let json_text = render(&value, ReportFormat::Json).unwrap();
        let parsed: Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(parsed["findings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn security_json_stdout_has_no_terminal_prefix() {
        let value = json!({"schemaVersion":2,"status":"completed","coverageComplete":true,"findings":[],"limitations":[]});
        let text = render(&value, ReportFormat::Json).unwrap();
        assert!(text.trim_start().starts_with('{'));
        assert!(!text.contains("Experimental security review"));
        assert!(!text.contains('\u{1b}'));
        assert!(serde_json::from_str::<Value>(&text).is_ok());
    }

    #[test]
    fn security_legacy_validated_flag_is_not_v2_confirmation() {
        let legacy = json!({
            "validated": true,
            "coverageComplete": true,
            "status": "completed",
            "findings": [{"severity":"critical","title":"legacy"}]
        });
        assert_eq!(exit_code(&legacy), 0);
        assert!(select_finding(legacy.clone(), Some("legacy")).is_err());
        let view = project_legacy_v1(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(view["schemaVersion"], 1);
        assert_eq!(view["readOnly"], true);
        assert_eq!(view["coverageComplete"], false);
        assert_eq!(view["legacyValidatedFlag"], true);
        assert_eq!(exit_code(&view), 2);
        assert!(project_legacy_v1(br#"{"schemaVersion":2,"findings":[]}"#).is_err());
    }

    #[test]
    fn security_report_formats_preserve_incomplete_status() {
        let value = json!({"scanId":"fixture", "coverageComplete":false,"findings":[],"limitations":["budget exhausted"]});
        assert_eq!(exit_code(&value), 2);
        assert!(
            serde_json::from_str::<Value>(&render(&value, ReportFormat::Json).unwrap()).is_ok()
        );
        let sarif: Value =
            serde_json::from_str(&render(&value, ReportFormat::Sarif).unwrap()).unwrap();
        assert_eq!(
            sarif["runs"][0]["invocations"][0]["executionSuccessful"],
            false
        );
        assert!(render(&value, ReportFormat::Terminal)
            .unwrap()
            .contains("incomplete"));
    }
}
