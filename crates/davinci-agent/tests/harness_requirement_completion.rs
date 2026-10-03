use davinci_agent::runtime::evidence_store::ExecutionReceipt;

#[test]
fn harness_zero_tests_and_unknown_discovery_are_not_test_success() {
    let mut receipt = ExecutionReceipt {
        started: true,
        exit_code: Some(0),
        argv: vec!["cargo test --workspace".into()],
        ..Default::default()
    };
    assert!(
        !receipt.is_verified_check(),
        "shell success cannot prove tests ran"
    );
    receipt.assertion_counts = Some(Default::default());
    assert!(!receipt.is_verified_check());
    receipt.assertion_counts = Some(davinci_agent::runtime::evidence::AssertionCounts {
        total: 2,
        passed: 2,
        failed: 0,
        skipped: 0,
    });
    assert!(receipt.is_verified_check());
    receipt.assertion_counts.as_mut().unwrap().total = 1;
    assert!(
        !receipt.is_verified_check(),
        "inconsistent counts cannot establish success"
    );
}

#[test]
fn tokenized_test_commands_also_require_discovery() {
    let receipt = ExecutionReceipt {
        started: true,
        exit_code: Some(0),
        argv: vec!["cargo".into(), "test".into(), "--workspace".into()],
        ..Default::default()
    };
    assert!(!receipt.is_verified_check());
}
