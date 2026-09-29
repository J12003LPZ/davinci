//! Offline after measurement paired with the frozen P4 before artifact.
use davinci_agent::{
    runtime::transactions::TransactionState,
    tools::{execute_tool_with, ToolContext},
};
use serde_json::json;
use std::{fs, time::Instant};

#[test]
fn root_alias_boundary_security_matrix() {
    use davinci_agent::{
        FilesystemBoundaryPolicy, PermissionMode, PermissionPolicy, PermissionRule,
        PermissionVerdict,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Workspace");
    let extra = temp.path().join("Shared");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&extra).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&extra, root.join("link")).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("link"))
        .arg(&extra)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(extra.join("existing.rs"), "fixture").unwrap();
    let mut cases = 0;
    for mode in PermissionMode::ALL {
        let mut policy = PermissionPolicy::new(mode);
        policy.filesystem_boundary = FilesystemBoundaryPolicy {
            root: Some(root.clone()),
            extra_roots: vec![extra.clone()],
            ..Default::default()
        };
        policy.allow.push(PermissionRule::bare("*"));
        for suffix in ["existing.rs", "new.rs", "missing/deep/new.rs"] {
            let args = json!({"path":root.join("link").join(suffix), "content":"bad"});
            assert!(
                matches!(
                    policy.decide("eval", "write", &args, &root),
                    PermissionVerdict::Deny { .. }
                ),
                "{mode:?} {suffix}"
            );
            cases += 1;
        }
    }
    assert_eq!(fs::read(extra.join("existing.rs")).unwrap(), b"fixture");
    println!(
        "{}",
        json!({"evaluation":"root-alias-boundary", "escape_cases_denied":cases, "destination_bytes_preserved":true})
    );
}

#[test]
fn transaction_after_measurement() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "before\n").unwrap();
    fs::write(root.path().join("b.txt"), "before\n").unwrap();
    let context = ToolContext::default();
    let began = Instant::now();
    let mut ids = Vec::new();
    for (tool, args) in [
        ("write", json!({"path":"a.txt","content":"written\n"})),
        (
            "edit",
            json!({"path":"a.txt","oldText":"written","newText":"edited"}),
        ),
        (
            "apply_patch",
            json!({"input":"*** Begin Patch\n*** Update File: a.txt\n@@\n-edited\n+patched\n*** Update File: b.txt\n@@\n-before\n+patched\n*** End Patch"}),
        ),
    ] {
        let result = execute_tool_with(root.path(), tool, &args, &context).unwrap();
        assert!(!result.is_error, "{}", result.content);
        let summary = result.details.unwrap()["transaction"].clone();
        assert_eq!(summary["state"], "applied");
        ids.push(summary["id"].as_str().unwrap().to_owned());
    }
    let mutation_ms = began.elapsed().as_secs_f64() * 1000.0;
    let manager = davinci_agent::runtime::transactions::TransactionCoordinator::new(
        root.path(),
        context.transaction_owner.clone(),
    )
    .unwrap();
    let began = Instant::now();
    manager.rollback(&ids[2], &|_| Ok(()), None).unwrap();
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"edited\n");
    assert_eq!(fs::read(root.path().join("b.txt")).unwrap(), b"before\n");
    let rollback_ms = began.elapsed().as_secs_f64() * 1000.0;
    // A later actor changes the exact source owned by an earlier transaction.
    fs::write(root.path().join("a.txt"), b"later user bytes\n").unwrap();
    assert!(manager.rollback(&ids[1], &|_| Ok(()), None).is_err());
    assert_eq!(
        manager.status(&ids[1]).unwrap().state,
        TransactionState::Conflicted
    );
    let preserved = fs::read(root.path().join("a.txt")).unwrap() == b"later user bytes\n";
    assert!(preserved);
    let artifact = json!({
        "schema":1,"platform":std::env::consts::OS,
        "measurement":"P4 after: public write/edit/apply_patch, owned rollback and stale recovery refusal in temporary files",
        "operation_count":3,"affected_files":2,"mutation_ms":mutation_ms,
        "operations_with_transaction_id":ids.len(),"owned_rollback_restored_files":2,
        "rollback_ms":rollback_ms,"later_user_bytes_preserved":preserved,
        "stale_rollback_state":"conflicted",
        "notes":["Single debug-profile sample; timings are not a statistical speed claim.",
            "Before used a constructed legacy journal; after uses genuine completed transaction records.",
            "This evaluates rollback correctness, not power-loss durability or host-crash latency."]
    });
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
    if let Some(path) = std::env::var_os("DAVINCI_TRANSACTION_EVAL_ARTIFACT") {
        fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}
