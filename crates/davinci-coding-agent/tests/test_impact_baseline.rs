//! Explicit pre-implementation baseline. No model calls, downloads, or production edits.
#[path = "support/test_impact_fixture.rs"]
mod fixture;

use davinci_coding_agent::native_extensions::repo_intelligence::RepoIntelligence;
use serde_json::json;
use std::{collections::BTreeSet, time::Instant};

#[test]
#[ignore = "explicit offline baseline requires installed Node.js; use --ignored --nocapture"]
fn test_impact_monorepo_baseline() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let all_tests = fixture::populate(root.path());
    let correct = fixture::run_tests(root.path(), &all_tests);
    assert_eq!(correct["failed"], 0);
    let manager = RepoIntelligence::new(root.path(), cache.path(), Default::default());

    let started = Instant::now();
    let cold = manager.refresh().unwrap();
    let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let warm = manager.refresh().unwrap();
    let warm_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(warm.reparsed, 0);

    let started = Instant::now();
    let related = manager
        .query(
            "related_files",
            &json!({"path":fixture::CHANGED,"limit":100}),
        )
        .unwrap();
    let related_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(related["truncated"], false);
    let selected: Vec<String> = related["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["path"].as_str())
        .filter(|path| all_tests.iter().any(|test| test == path))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    fixture::plant_failure(root.path());
    let started = Instant::now();
    let edited = manager.refresh().unwrap();
    let edited_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(edited.reparsed, 1);
    let full = fixture::run_tests(root.path(), &all_tests);
    let heuristic = fixture::run_tests(root.path(), &selected);
    let expected: BTreeSet<String> = fixture::IMPACTED.iter().map(|s| s.to_string()).collect();
    let full_failures: BTreeSet<String> =
        serde_json::from_value(full["failure_paths"].clone()).unwrap();
    assert_eq!(
        full_failures, expected,
        "planted failure must reach all three consumers"
    );
    let selected_failures: BTreeSet<String> =
        serde_json::from_value(heuristic["failure_paths"].clone()).unwrap();
    let missed: Vec<_> = full_failures
        .difference(&selected_failures)
        .cloned()
        .collect();
    // This records the baseline honestly. Future selection must catch every
    // planted failure; related_files is a heuristic, not a test-completion gate.
    let result = json!({
        "schema":1,"fixture":"test-impact-monorepo-v1",
        "scope":"three JS packages; 23 Node tests; one planted cross-package regression",
        "selection":"existing related_files results intersected with fixture test paths",
        "correct_full_suite":correct,"mutated_full_suite":full,
        "mutated_heuristic_selection":heuristic,"missed_failure_paths":missed,
        "cold":{"source_files":cold.files.len(),"source_bytes_read":cold.bytes_read,
            "reparsed":cold.reparsed,"latency_ms":cold_ms},
        "warm":{"source_bytes_read":warm.bytes_read,"reparsed":warm.reparsed,"latency_ms":warm_ms},
        "one_file_edit":{"source_bytes_read":edited.bytes_read,"reparsed":edited.reparsed,"latency_ms":edited_ms},
        "related_query":{"latency_ms":related_ms,"returned_bytes":serde_json::to_vec(&related).unwrap().len(),
            "results":related},
        "notes":[
            "Single debug-profile sample; timing is descriptive, not a performance threshold.",
            "A heuristic subset is not sufficient final verification. Full suite establishes all planted failures.",
            "No provider token counts, LSP, browser, or new test-impact implementation are measured."
        ]
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
