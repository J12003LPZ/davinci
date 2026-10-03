use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};

fn limits() -> BudgetLimits {
    BudgetLimits {
        max_requests: 100,
        max_output_tokens: Some(10_000),
        max_cost_microusd: None,
        codex_subscription: None,
        deadline_unix_ms: u64::MAX,
    }
}

#[test]
fn design_operation_counts_child_attempts_in_the_existing_root_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("root.json");
    let root = RootBudget::open(path.clone(), "root", limits()).unwrap();
    root.begin_operation("design-one", 12, u64::MAX).unwrap();
    for index in 0..12 {
        let child = RootBudget::reopen(path.clone(), "root", limits()).unwrap();
        let id = format!("child-{index}");
        child.reserve("child", &id, 100, None).unwrap();
        child.reconcile(&id, Some(10), None).unwrap();
    }
    assert!(root.reserve("parent", "thirteenth", 100, None).is_err());
    assert_eq!(root.snapshot().unwrap().requests, 12);
    assert!(root.begin_operation("reset", 12, u64::MAX).is_err());
    root.finish_operation("design-one").unwrap();
    assert!(root.begin_operation("design-one", 12, u64::MAX).is_err());
    root.begin_operation("design-two", 12, u64::MAX).unwrap();
    root.reserve("parent", "next", 100, None).unwrap();
    assert_eq!(root.snapshot().unwrap().requests, 13);
}

#[test]
fn design_deadlines_unknown_outcomes_and_stricter_parent_are_not_reset() {
    let dir = tempfile::tempdir().unwrap();
    let root = RootBudget::open(dir.path().join("expired.json"), "r", limits()).unwrap();
    root.begin_operation("design", 12, 1).unwrap();
    assert!(root.reserve("p", "expired", 1, None).is_err());
    assert!(root.begin_operation("design", 12, u64::MAX).is_err());
    let mut small = limits();
    small.max_requests = 1;
    let root = RootBudget::open(dir.path().join("strict.json"), "r", small).unwrap();
    root.begin_operation("design", 12, u64::MAX).unwrap();
    root.reserve("p", "one", 1, None).unwrap();
    root.reconcile("one", None, None).unwrap();
    root.finish_operation("design").unwrap();
    root.begin_operation("next-design", 12, u64::MAX).unwrap();
    assert!(root.reserve("p", "two", 1, None).is_err());
    root.reconcile("one", Some(1), None).unwrap();
    assert!(root.reserve("p", "two", 1, None).is_err());
}
