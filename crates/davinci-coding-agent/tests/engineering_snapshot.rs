use davinci_coding_agent::native_extensions::{
    engineering_snapshot::EngineeringSnapshots,
    repo_intelligence::{RepoIntelligence, RepoIntelligenceConfig},
    NativeExtensionHost,
};
use std::{fs, sync::Arc};

#[test]
fn engineering_snapshot_reuses_facts_and_rechecks_permissions_and_changes() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","scripts":{"test":"node --test"}}"#,
    )
    .unwrap();
    fs::write(root.path().join("a.js"), "export const a = 1;").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), RepoIntelligenceConfig::default());
    let snapshots = EngineeringSnapshots::default();
    let paths = vec!["a.js".into()];
    let allow = |_: &str| Ok(());
    let first = snapshots
        .get_with_usage(root.path(), &repo, &paths, false, &allow)
        .unwrap()
        .0;
    let second = snapshots
        .get_with_usage(root.path(), &repo, &paths, false, &allow)
        .unwrap()
        .0;
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(snapshots.counters(), (1, 1));
    assert!(Arc::ptr_eq(
        &first,
        &snapshots.peek_current(root.path()).unwrap()
    ));
    assert!(snapshots
        .get_with_usage(
            root.path(),
            &repo,
            &paths,
            false,
            &|_| Err("revoked".into())
        )
        .is_err());
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"changed-fixture","scripts":{"test":"vitest run"}}"#,
    )
    .unwrap();
    assert!(snapshots.peek_current(root.path()).is_none());
    let changed = snapshots
        .get_with_usage(root.path(), &repo, &paths, false, &allow)
        .unwrap()
        .0;
    assert_ne!(changed.identity, first.identity);
    fs::write(root.path().join("new.js"), "export const b = 2;").unwrap();
    let added = snapshots
        .get_with_usage(root.path(), &repo, &paths, false, &allow)
        .unwrap()
        .0;
    assert!(added.index.files.contains_key("new.js"));
    snapshots.invalidate();
    assert!(snapshots.peek().is_none());
    let fresh = snapshots
        .get_with_usage(root.path(), &repo, &paths, true, &allow)
        .unwrap()
        .0;
    assert!(fresh.generation > first.generation);
    assert_eq!(
        fresh.workspace_dirty,
        davinci_agent::decision::request::WorkspaceDirtyState::Unknown
    );
}

#[test]
fn native_test_impact_reads_the_shared_snapshot_and_mutations_invalidate_it() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(root.path().join("package.json"), r#"{"name":"fixture"}"#).unwrap();
    fs::write(root.path().join("a.js"), "export const a = 1;").unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("snapshot", root.path(), Some(cache.path()));
    let args = serde_json::json!({"path":"a.js"});
    let result = host.execute_tool(root.path(), "test_plan", &args).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let first = host.engineering.peek().unwrap();
    let result = host.execute_tool(root.path(), "test_plan", &args).unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(Arc::ptr_eq(&first, &host.engineering.peek().unwrap()));
    assert!(host.engineering.counters().1 >= 1);
    host.after_tool("write", &args, result);
    assert!(host.engineering.peek().is_none());
}

#[test]
fn verification_and_build_consumers_reuse_metadata_without_changing_results() {
    use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        cache.path().join("settings.json"),
        r#"{"verificationPlanner":{"enabled":true}}"#,
    )
    .unwrap();
    fs::write(root.path().join("package.json"),
        r#"{"name":"fixture","scripts":{"test":"node --test","lint":"eslint .","typecheck":"tsc"}}"#).unwrap();
    fs::write(root.path().join("a.ts"), "export const a = 1;").unwrap();
    let mut host = NativeExtensionHost::new_with_agent_dir(
        "snapshot-consumers",
        root.path(),
        Some(cache.path()),
    );
    host.verification_planner
        .set_permissions(Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))));
    let plan_args = serde_json::json!({"files":["a.ts"]});
    let cold_plan = host
        .execute_tool(root.path(), "verification_plan", &plan_args)
        .unwrap()
        .details
        .unwrap();
    let cold_packages = host
        .execute_tool(root.path(), "workspace_packages", &serde_json::json!({}))
        .unwrap()
        .details
        .unwrap();
    assert!(
        host.engineering.peek().is_none(),
        "passive consumers must not start a scan"
    );
    let indexed = host
        .execute_tool(
            root.path(),
            "test_plan",
            &serde_json::json!({"path":"a.ts"}),
        )
        .unwrap();
    assert!(!indexed.is_error, "{}", indexed.content);
    let (builds, hits) = host.engineering.counters();
    let warm_plan = host
        .execute_tool(root.path(), "verification_plan", &plan_args)
        .unwrap()
        .details
        .unwrap();
    let warm_packages = host
        .execute_tool(root.path(), "workspace_packages", &serde_json::json!({}))
        .unwrap()
        .details
        .unwrap();
    let without_telemetry = |mut value: serde_json::Value| {
        value.as_object_mut().unwrap().remove("telemetry");
        value
    };
    assert_eq!(
        without_telemetry(cold_plan),
        without_telemetry(warm_plan.clone())
    );
    assert_eq!(cold_packages, warm_packages);
    assert_eq!(host.engineering.counters().0, builds);
    assert!(host.engineering.counters().1 >= hits + 2);
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","scripts":{}}"#,
    )
    .unwrap();
    let changed = host
        .execute_tool(root.path(), "verification_plan", &plan_args)
        .unwrap()
        .details
        .unwrap();
    assert_ne!(
        changed["steps"], warm_plan["steps"],
        "external changes must bypass cached scripts"
    );
}
