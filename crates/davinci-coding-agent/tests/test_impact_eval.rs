//! Repeatable, explicit offline evaluation; no LLM or dependency downloads.
#[path = "support/test_impact_fixture.rs"]
mod fixture;
use davinci_coding_agent::native_extensions::{
    repo_intelligence::{RepoIntelligence, RepoIntelligenceConfig},
    NativeExtensionHost,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path, time::Instant};

fn plan(host: &mut NativeExtensionHost, root: &Path, args: Value) -> Value {
    let start = Instant::now();
    let result = host.execute_tool(root, "test_plan", &args).unwrap();
    let mut value = result.details.unwrap();
    value["measured_latency_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    value["returned_bytes"] = json!(result.content.len());
    value
}

#[test]
#[ignore = "explicit offline evaluation requires Node.js; use --ignored --nocapture"]
fn test_impact_monorepo_after() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let all = fixture::populate(root.path());
    let correct_full = fixture::run_tests(root.path(), &all);
    assert_eq!(correct_full["failed"], 0);
    let start = Instant::now();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("impact-eval", root.path(), Some(state.path()));
    let construction_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        host.repo_intelligence.status()["observation"]["started"],
        false
    );
    let args = json!({"path":fixture::CHANGED});
    let cold = plan(&mut host, root.path(), args.clone());
    let warm = plan(&mut host, root.path(), args.clone());
    assert_eq!(cold["total"], 3);
    assert_eq!(warm["telemetry"]["mapping_cache_hit"], true);
    assert!(
        warm["telemetry"]["source_bytes_read"].as_u64().unwrap()
            < cold["telemetry"]["source_bytes_read"].as_u64().unwrap()
    );
    let selected: Vec<String> = serde_json::from_value(json!(warm["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].clone())
        .collect::<Vec<_>>()))
    .unwrap();
    let correct_selected = fixture::run_tests(root.path(), &selected);
    assert_eq!(correct_selected["passed"], 3);
    fixture::plant_failure(root.path());
    let edited = plan(&mut host, root.path(), args);
    assert_eq!(edited["telemetry"]["reparsed"], 1);
    assert_ne!(edited["source_identity"], cold["source_identity"]);
    let full = fixture::run_tests(root.path(), &all);
    let targeted = fixture::run_tests(root.path(), &selected);
    let failures: BTreeSet<String> = serde_json::from_value(full["failure_paths"].clone()).unwrap();
    let selected_failures: BTreeSet<String> =
        serde_json::from_value(targeted["failure_paths"].clone()).unwrap();
    let expected: BTreeSet<String> = fixture::IMPACTED.iter().map(|p| p.to_string()).collect();
    assert_eq!(failures, expected);
    assert_eq!(selected_failures, failures);
    let false_positives: Vec<_> = selected.iter().filter(|p| !expected.contains(*p)).collect();
    assert!(false_positives.is_empty());
    assert!(!edited["broader_verification"]
        .as_array()
        .unwrap()
        .is_empty());
    let forced = plan(
        &mut host,
        root.path(),
        json!({"path":fixture::CHANGED,"refresh":true}),
    );
    assert_eq!(forced["source_identity"], edited["source_identity"]);
    let fallback = RepoIntelligence::new(
        root.path(),
        state.path(),
        RepoIntelligenceConfig {
            observe_changes: false,
            ..Default::default()
        },
    );
    fallback.refresh().unwrap();
    let start = Instant::now();
    let fallback_warm = fallback
        .refresh_observed_authorized(&[], false, &|_| Ok(()))
        .unwrap();
    let fallback_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(fallback_warm.files_read, 46);
    fixture::write(
        root.path(),
        "packages/auth/src/untested.mjs",
        "export const untested = 1;",
    );
    let no_tests = plan(
        &mut host,
        root.path(),
        json!({"path":"packages/auth/src/untested.mjs"}),
    );
    assert_eq!(no_tests["total"], 0);
    assert!(!no_tests["broader_verification"]
        .as_array()
        .unwrap()
        .is_empty());
    let result = json!({"schema":1,"fixture":"test-impact-monorepo-v1",
        "measurement":"single debug-profile sample; fixture-only accuracy, not a general miss-rate guarantee",
        "platform":std::env::consts::OS,
        "construction_ms":construction_ms,"correct_full_suite":correct_full,"correct_selected_suite":correct_selected,
        "cold":cold,"warm":warm,"one_file_edit":edited,"forced":forced,
        "fallback_warm":{"source_bytes_read":fallback_warm.bytes_read,"source_files_read":fallback_warm.files_read,"latency_ms":fallback_ms,"mode":fallback_warm.refresh_mode},
        "mutated_full_suite":full,"mutated_selected_suite":targeted,"false_positive_paths":false_positives,"missed_failure_paths":[],
        "no_test_case":{"selected":0,"execution":"not run; zero tests are not verification","broader_verification":no_tests["broader_verification"]},
        "notes":["Node commands execute real generated fixtures, without package downloads or LLM calls.","Counts of files not selected are planning facts, not proof those tests may be omitted at completion.","Warm source I/O excludes metadata and directory enumeration; total test_plan latency includes authorization and cache lookup."]});
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
