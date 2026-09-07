//! Supporting evidence capture within the repository; no TypeScript equivalent.

use super::{
    command::{ScanCommand, Selection},
    config::ScanConfig,
    snapshot::Snapshot,
};
use std::path::Path;

pub(super) fn claim_scope(snapshot: &Snapshot, claim: &super::validation::Claim) -> &'static str {
    let controls: Vec<_> = claim
        .locations
        .iter()
        .filter(|location| location.role == "control")
        .collect();
    let anchors: Vec<_> = if controls.is_empty() {
        claim.locations.iter().collect()
    } else {
        controls
    };
    if !anchors.is_empty()
        && anchors
            .iter()
            .all(|location| snapshot.is_target(&location.path, &location.snapshot_side))
    {
        "target"
    } else {
        "supporting"
    }
}

pub(super) fn capture(
    root: &Path,
    command: &ScanCommand,
    config: &ScanConfig,
    mut target: Snapshot,
    cancelled: &dyn Fn() -> bool,
) -> Result<Snapshot, String> {
    if !config.supporting_reads
        || command.selection == Selection::Worktree && command.scopes.is_empty()
    {
        return Ok(target);
    }
    let remaining = config.max_snapshot_bytes.saturating_sub(target.bytes);
    let remaining_entries = config
        .max_inventory_entries
        .saturating_sub(target.source_count());
    if remaining == 0 || remaining_entries == 0 {
        target
            .supporting_skipped
            .push(super::snapshot::SkippedFile {
                path: ".".into(),
                reason: "supporting evidence unavailable: snapshot byte or inventory limit".into(),
            });
    } else {
        let supporting_request = ScanCommand {
            scopes: Vec::new(),
            ..command.clone()
        };
        let supporting_config = ScanConfig {
            supporting_reads: false,
            max_snapshot_bytes: remaining,
            max_file_bytes: config.max_file_bytes.min(remaining),
            max_policy_bytes: config.max_policy_bytes.min(remaining),
            max_inventory_entries: remaining_entries,
            ..config.clone()
        };
        let support = if command.selection == Selection::Worktree {
            Snapshot::capture_targets_interruptible(
                root,
                &supporting_request,
                &supporting_config,
                cancelled,
            )?
        } else {
            super::git::capture_revision_sources(
                root,
                &supporting_request,
                &supporting_config,
                cancelled,
                true,
            )?
        };
        if support.revisions != target.revisions {
            return Err("revision identity changed while capturing supporting evidence".into());
        }
        merge_conflicts(&mut target, &support)?;
        target.supporting_files = support
            .files
            .into_iter()
            .filter(|(path, _)| !target.files.contains_key(path))
            .collect();
        target.supporting_base_files = support
            .base_files
            .into_iter()
            .filter(|(path, _)| !target.base_files.contains_key(path))
            .collect();
        target.supporting_skipped = support
            .skipped
            .into_iter()
            .filter(|skip| {
                !target.files.contains_key(&skip.path)
                    && !target.base_files.contains_key(&skip.path)
            })
            .collect();
        for file in target
            .supporting_files
            .values()
            .chain(target.supporting_base_files.values())
        {
            target.bytes = target
                .bytes
                .checked_add(file.text.len() as u64)
                .ok_or("supporting byte count overflow")?;
        }
    }
    target.id = super::sha256_hex(
        &serde_json::to_vec(&target).map_err(|_| "cannot identify supporting snapshot")?,
    );
    Ok(target)
}

fn merge_conflicts(target: &mut Snapshot, support: &Snapshot) -> Result<(), String> {
    let expected: std::collections::BTreeMap<_, _> = support
        .conflicts
        .iter()
        .map(|stage| ((&stage.path, stage.stage), stage))
        .collect();
    if target
        .conflicts
        .iter()
        .any(|stage| expected.get(&(&stage.path, stage.stage)).copied() != Some(stage))
    {
        return Err("index conflicts changed while capturing supporting evidence".into());
    }
    let targets: std::collections::BTreeSet<_> = target
        .conflicts
        .iter()
        .map(|stage| (&stage.path, stage.stage))
        .collect();
    let captured: std::collections::BTreeSet<_> = target
        .conflict_sources
        .iter()
        .map(|source| (source.identity.path.clone(), source.identity.stage))
        .collect();
    for source in &support.conflict_sources {
        if captured.contains(&(source.identity.path.clone(), source.identity.stage)) {
            continue;
        }
        target.bytes = target
            .bytes
            .checked_add(source.source.text.len() as u64)
            .ok_or("supporting conflict byte count overflow")?;
        target
            .conflict_sources
            .push(super::snapshot::ConflictSource {
                supporting: !targets.contains(&(&source.identity.path, source.identity.stage)),
                ..source.clone()
            });
    }
    target.conflicts = support.conflicts.clone();
    target.conflict_sources.sort_by(|a, b| {
        (&a.identity.path, a.identity.stage).cmp(&(&b.identity.path, b.identity.stage))
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn security_cross_file_audit_reads_support_but_covers_only_target_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("entry.rs"), "helper::check();\n").unwrap();
        std::fs::write(root.path().join("helper.rs"), "fn check() {}\n").unwrap();
        let location = |path: &str, text: &str, role: &str| {
            json!({"path":path,"startLine":1,"endLine":1,
            "contentHash":super::super::sha256_hex(text.as_bytes()),"snapshotSide":"worktree","role":role})
        };
        let entry = location("entry.rs", "helper::check();\n", "entrypoint");
        let helper = location("helper.rs", "fn check() {}\n", "control");
        let source = |anchor| {
            json!({"location":anchor,"surface":"fixture","rationale":"Offline fixture",
            "language":"Rust","buildContext":"Fixture only","unitIds":["entry-check"],"noUnitReason":null})
        };
        let map = json!({"sources":[source(entry.clone()),source(helper.clone())],"units":[{
            "id":"entry-check","entrypoint":"entry calls helper check","actorCapabilities":"fixture invoker",
            "assets":"no protected assets in fixture","trustBoundary":"no crossing in fixture",
            "closestControl":"helper body is empty","sensitiveOperation":"fixture check invocation",
            "dataFlow":"entry calls helper directly","relatedConfiguration":"none used by fixture",
            "locations":[entry,helper],"disposition":"reviewed","rationale":"Both complete fixture bodies inspected","unknowns":[]}],
            "environmentAssumptions":[]});
        let runner = super::super::worker::SecurityWorkerRunner::new(move |request| {
            use davinci_ai::ContentBlock;
            let content = if request.messages.len() == 1 {
                ["entry.rs", "helper.rs"]
                    .iter()
                    .map(|path| ContentBlock::ToolCall {
                        id: (*path).into(),
                        name: "sec_source_read".into(),
                        arguments: json!({"path":path,"startLine":1,"endLine":1}),
                    })
                    .collect()
            } else {
                let output = if request.run.status().status
                    == super::super::types::RunStatus::Mapping
                {
                    map.clone()
                } else {
                    json!({"repositoryMap":map,"reviewedPaths":["entry.rs"],"candidates":[],"limitations":[]})
                };
                vec![ContentBlock::Text {
                    text: output.to_string(),
                }]
            };
            Ok(davinci_ai::AssistantMessage {
                id: "fixture".into(),
                role: "assistant".into(),
                content,
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            })
        });
        let mut controller = super::super::SecurityScanController::new(root.path().to_path_buf());
        controller.configure_review(runner, ScanConfig::default());
        controller
            .command("security-scan", "--scope entry.rs")
            .unwrap();
        controller.wait_for_review();
        let report = controller.command("sec-report", "").unwrap().unwrap();
        assert_eq!(report["coverageComplete"], true, "{report}");
        assert_eq!(report["coverage"]["eligibleFiles"], 1);
        assert_eq!(report["source"]["supportingFiles"], 1);
        assert_eq!(report["coverage"]["reviewedPaths"], json!(["entry.rs"]));
    }

    #[test]
    fn security_cross_file_finding_requires_both_source_and_control_reads() {
        let root = tempfile::tempdir().unwrap();
        let entry = "helper::allow(user);\n";
        let helper = "fn allow(_user: u64) {}\n";
        std::fs::write(root.path().join("entry.rs"), entry).unwrap();
        std::fs::write(root.path().join("helper.rs"), helper).unwrap();
        let loc = |path: &str, text: &str, role: &str| {
            json!({"path":path,"startLine":1,"endLine":1,
            "contentHash":super::super::sha256_hex(text.as_bytes()),"snapshotSide":"worktree","role":role})
        };
        let entry_loc = loc("entry.rs", entry, "entrypoint");
        let helper_loc = loc("helper.rs", helper, "control");
        let source = |anchor| {
            json!({"location":anchor,"surface":"fixture","rationale":"Offline fixture",
            "language":"Rust","buildContext":"Fixture only","unitIds":["entry-allow"],"noUnitReason":null})
        };
        let map = json!({"sources":[source(entry_loc.clone()),source(helper_loc.clone())],"units":[{
            "id":"entry-allow","entrypoint":"entry calls helper allow","actorCapabilities":"fixture invoker",
            "assets":"caller identity","trustBoundary":"entry to helper","closestControl":"helper allow",
            "sensitiveOperation":"authorization decision","dataFlow":"entry passes user to helper",
            "relatedConfiguration":"none","locations":[entry_loc.clone(),helper_loc.clone()],
            "disposition":"reviewed","rationale":"Both fixture bodies inspected","unknowns":[]}],
            "environmentAssumptions":[]});
        let record =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let mut claim = record["claim"].clone();
        claim["locations"] = json!([entry_loc, helper_loc]);
        claim["entrypoint"] = json!("entry.rs helper::allow");
        claim["control"] = json!("helper.rs allow");
        let mut assessment = record["assessment"].clone();
        assessment["rootCause"]["control"] = claim["locations"][1].clone();
        assessment["rootCause"]["decision"] = claim["locations"][0].clone();
        assessment["gates"] = json!((0..5)
            .map(|_| json!({"satisfied":true,"rationale":"fixture","locations":claim["locations"]}))
            .collect::<Vec<_>>());
        let reads = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let seen = reads.clone();
        let runner = super::super::worker::SecurityWorkerRunner::new(move |request| {
            use davinci_ai::ContentBlock;
            let content = if request.messages.len() == 1 {
                for path in ["entry.rs", "helper.rs"] {
                    seen.lock().unwrap().insert(path.to_string());
                }
                ["entry.rs", "helper.rs"]
                    .iter()
                    .map(|path| ContentBlock::ToolCall {
                        id: (*path).into(),
                        name: "sec_source_read".into(),
                        arguments: json!({"path":path,"startLine":1,"endLine":1}),
                    })
                    .collect()
            } else {
                let output = if request.run.status().status
                    == super::super::types::RunStatus::Mapping
                {
                    map.clone()
                } else if request.run.status().status
                    == super::super::types::RunStatus::Investigating
                {
                    json!({"repositoryMap":map,"reviewedPaths":["entry.rs"],"candidates":[claim],"limitations":[]})
                } else {
                    assessment.clone()
                };
                vec![ContentBlock::Text {
                    text: output.to_string(),
                }]
            };
            Ok(davinci_ai::AssistantMessage {
                id: "fixture".into(),
                role: "assistant".into(),
                content,
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            })
        });
        let mut controller = super::super::SecurityScanController::new(root.path().to_path_buf());
        controller.configure_review(runner, ScanConfig::default());
        controller
            .command("security-scan", "--scope entry.rs")
            .unwrap();
        controller.wait_for_review();
        let report = controller.command("sec-report", "").unwrap().unwrap();
        assert!(
            reads.lock().unwrap().contains("entry.rs")
                && reads.lock().unwrap().contains("helper.rs")
        );
        let locations = report["candidates"][0]["claim"]["locations"]
            .as_array()
            .unwrap();
        let paths: Vec<_> = locations
            .iter()
            .filter_map(|row| row["path"].as_str())
            .collect();
        assert!(
            paths.contains(&"entry.rs") && paths.contains(&"helper.rs"),
            "{report}"
        );
        assert_eq!(report["source"]["supportingFiles"], 1, "{report}");
        assert_eq!(report["coverage"]["reviewedPaths"], json!(["entry.rs"]));
    }

    #[test]
    fn security_supporting_tools_policy_and_checkpoint_preserve_scope_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("entry.rs"), "helper::check();\n").unwrap();
        std::fs::write(root.path().join("helper.rs"), "fn check() {}\n").unwrap();
        let command = ScanCommand::parse("--scope entry.rs").unwrap();
        let snapshot = Snapshot::capture(root.path(), &command, &ScanConfig::default()).unwrap();
        let list = super::super::tools::execute(&snapshot, "sec_source_list", json!({"offset":0}))
            .unwrap();
        assert_eq!(list["entries"][0]["scope"], "target");
        assert_eq!(list["entries"][1]["scope"], "supporting");
        let search = super::super::tools::execute(
            &snapshot,
            "sec_source_search",
            json!({"query":"fn check","offset":0}),
        )
        .unwrap();
        assert_eq!(search["matches"][0]["scope"], "supporting");
        let id = uuid::Uuid::new_v4().to_string();
        let store =
            super::super::store::Store::open(storage.path(), root.path(), &id, true).unwrap();
        store.checkpoint(&command, &snapshot).unwrap();
        std::fs::write(root.path().join("helper.rs"), "new bytes\n").unwrap();
        let saved = store.load(1024 * 1024).unwrap();
        assert_eq!(saved.snapshot.id, snapshot.id);
        assert_eq!(
            saved.snapshot.read("helper.rs", 1, 1).unwrap(),
            "fn check() {}"
        );
        assert!(!saved.snapshot.is_target("helper.rs", "worktree"));
        let mut overlapping = snapshot.clone();
        overlapping
            .supporting_files
            .insert("entry.rs".into(), snapshot.files["entry.rs"].clone());
        overlapping.bytes += snapshot.files["entry.rs"].text.len() as u64;
        let second_id = uuid::Uuid::new_v4().to_string();
        let duplicate =
            super::super::store::Store::open(storage.path(), root.path(), &second_id, true)
                .unwrap();
        duplicate.checkpoint(&command, &overlapping).unwrap();
        assert!(duplicate
            .load(1024 * 1024)
            .unwrap_err()
            .contains("duplicate target/supporting"));
        let parent = ScanConfig {
            supporting_reads: false,
            ..ScanConfig::default()
        };
        assert!(
            !parent
                .narrow_with(&ScanConfig::default())
                .unwrap()
                .supporting_reads
        );
        assert!(
            !ScanConfig::default()
                .narrow_with(&parent)
                .unwrap()
                .supporting_reads
        );
    }

    #[test]
    fn security_supporting_reads_respect_explicit_read_boundary() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("entry.rs"), "helper::check();\n").unwrap();
        std::fs::write(root.path().join("helper.rs"), "fn check() {}\n").unwrap();
        let command = ScanCommand::parse("--scope entry.rs").unwrap();
        let confined = ScanConfig {
            supporting_reads: false,
            ..ScanConfig::default()
        };
        let snapshot = Snapshot::capture(root.path(), &command, &confined).unwrap();
        assert_eq!(
            snapshot.files.keys().cloned().collect::<Vec<_>>(),
            ["entry.rs"]
        );
        assert!(snapshot.supporting_files.is_empty());
        assert!(snapshot.read("helper.rs", 1, 1).is_err());
        assert!(super::super::tools::execute(
            &snapshot,
            "sec_source_read",
            json!({"path":"helper.rs","startLine":1,"endLine":1})
        )
        .is_err());
        let id = uuid::Uuid::new_v4().to_string();
        let store =
            super::super::store::Store::open(storage.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint_bound(&command, &snapshot, &confined)
            .unwrap();
        let loaded = store.load(1024 * 1024).unwrap();
        assert!(loaded.validate_resume(&ScanConfig::default()).is_err());
        loaded.validate_resume(&confined).unwrap();
    }

    #[test]
    fn security_supporting_capture_respects_shared_byte_and_inventory_caps() {
        let root = tempfile::tempdir().unwrap();
        for path in ["target.rs", "support.rs", "another.rs"] {
            std::fs::write(root.path().join(path), "12345678\n").unwrap();
        }
        let config = ScanConfig {
            max_file_bytes: 10,
            max_policy_bytes: 10,
            max_snapshot_bytes: 18,
            max_inventory_entries: 3,
            ..ScanConfig::default()
        };
        let snapshot = Snapshot::capture(
            root.path(),
            &ScanCommand::parse("--scope target.rs").unwrap(),
            &config,
        )
        .unwrap();
        assert_eq!(snapshot.files["target.rs"].text, "12345678\n");
        assert!(snapshot.bytes <= 18);
        assert!(snapshot.source_count() <= 3);
        assert!(!snapshot.supporting_skipped.is_empty());
    }
}
