use davinci_evals::run_offline_optimization_gate;

#[test]
fn security_scan_incremental() {
    let report = run_offline_optimization_gate();
    assert!(report.passed, "offline gate failures: {:?}", report.failures);
    let result = report
        .results
        .iter()
        .find(|result| result.name == "security-incremental")
        .expect("security-incremental ablation must be registered");
    assert!(result.baseline_correct);
    assert!(result.candidate_correct);
    assert!(result.candidate_units < result.baseline_units);
    assert_eq!(result.unit_name, "files_rescanned");
}
