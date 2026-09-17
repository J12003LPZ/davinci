use davinci_agent::runtime::transactions::{
    ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
};
use std::fs;

fn owner() -> TransactionOwner {
    TransactionOwner::default()
}

#[cfg(unix)]
#[test]
fn unix_transaction_preserves_owner_and_special_mode_during_edit_and_recovery() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source");
    fs::write(&path, b"before").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o6750)).unwrap();
    let initial = fs::metadata(&path).unwrap();
    assert_eq!(initial.mode() & 0o7777, 0o6750);
    let identity = owner();
    let manager = TransactionCoordinator::new(root.path(), identity.clone()).unwrap();
    for proposal in [
        ProposedChange::write("source", b"after".to_vec()),
        ProposedChange::delete("source"),
    ] {
        let preview = manager.preview(vec![proposal]).unwrap();
        manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
        if path.exists() {
            let applied = fs::metadata(&path).unwrap();
            assert_eq!(
                (applied.uid(), applied.gid(), applied.mode() & 0o7777),
                (initial.uid(), initial.gid(), 0o6750)
            );
        }
        TransactionCoordinator::new(root.path(), identity.clone())
            .unwrap()
            .rollback(&preview.id, &|_| Ok(()), None)
            .unwrap();
        let recovered = fs::metadata(&path).unwrap();
        assert_eq!(
            (recovered.uid(), recovered.gid(), recovered.mode() & 0o7777),
            (initial.uid(), initial.gid(), 0o6750)
        );
        assert_eq!(fs::read(&path).unwrap(), b"before");
    }
}

#[cfg(unix)]
#[test]
fn unix_special_mode_only_change_refuses_apply_and_rollback() {
    use std::os::unix::fs::PermissionsExt;
    for rollback in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source");
        fs::write(&path, b"before").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
        let preview = manager
            .preview(vec![ProposedChange::write("source", b"after".to_vec())])
            .unwrap();
        if rollback {
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o4750)).unwrap();
        let error = if rollback {
            manager.rollback(&preview.id, &|_| Ok(()), None)
        } else {
            manager.apply(&preview.id, &|_| Ok(()), None)
        }
        .unwrap_err();
        assert!(error.contains("conflict"), "{error}");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o4750
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            if rollback {
                b"after".as_slice()
            } else {
                b"before".as_slice()
            }
        );
    }
}

#[test]
fn derived_edits_refuse_changes_between_read_and_preview() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let snapshot = manager.snapshot("a.txt").unwrap();
    fs::write(root.path().join("a.txt"), b"other!").unwrap();
    assert!(manager
        .preview(vec![snapshot.change(Some(b"after".to_vec()))])
        .unwrap_err()
        .contains("conflict"));
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"other!");
}

#[test]
fn explicit_transaction_tools_preview_apply_status_and_rollback() {
    use davinci_agent::tools::{execute_tool_with, ToolContext};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "before\n").unwrap();
    let context = ToolContext::default();
    let result = execute_tool_with(
        root.path(),
        "patch_preview",
        &serde_json::json!({
            "input":"*** Begin Patch\n*** Update File: a.txt\n@@\n-before\n+after\n*** End Patch"
        }),
        &context,
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let summary = result.details.unwrap();
    assert_eq!(summary["transaction"]["state"], "previewed");
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before\n");
    let args = serde_json::json!({"id":summary["transaction"]["id"], "paths":["a.txt"]});
    for (tool, state) in [
        ("patch_apply", "applied"),
        ("patch_status", "applied"),
        ("patch_rollback", "rolled_back"),
    ] {
        let result = execute_tool_with(root.path(), tool, &args, &context).unwrap();
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.details.unwrap()["transaction"]["state"], state);
    }
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before\n");
}

#[test]
fn disabling_explicit_transactions_keeps_ordinary_edit_safety() {
    use davinci_agent::tools::{execute_tool_with, ToolContext};
    let root = tempfile::tempdir().unwrap();
    let context = ToolContext {
        transactions_disabled: true,
        ..ToolContext::default()
    };
    let preview =
        serde_json::json!({"input":"*** Begin Patch\n*** Add File: a.txt\n+new\n*** End Patch"});
    assert!(execute_tool_with(root.path(), "patch_preview", &preview, &context).is_err());
    assert!(!root.path().join("a.txt").exists());
    let result = execute_tool_with(
        root.path(),
        "write",
        &serde_json::json!({"path":"a.txt","content":"new"}),
        &context,
    )
    .unwrap();
    assert_eq!(result.details.unwrap()["transaction"]["state"], "applied");
}

#[test]
fn parent_session_recovery_retains_original_owner_and_records_current_actor() {
    use davinci_agent::tools::{execute_tool_with, ToolContext};
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let root = tempfile::tempdir().unwrap();
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session("resumable");
    let context = ToolContext {
        runtime: Some(runtime.clone()),
        ..ToolContext::default()
    };
    let result = execute_tool_with(
        root.path(),
        "write",
        &serde_json::json!({"path":"a.txt","content":"created"}),
        &context,
    )
    .unwrap();
    let summary = result.details.unwrap()["transaction"].clone();
    let args = serde_json::json!({"id":summary["id"],"paths":["a.txt"]});
    let mut worker = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session("resumable");
    worker.parent_agent_id = Some(runtime.agent_id);
    let worker_context = ToolContext {
        runtime: Some(worker),
        ..ToolContext::default()
    };
    assert!(execute_tool_with(root.path(), "patch_rollback", &args, &worker_context).is_err());
    let graph_context = ToolContext {
        runtime: Some(
            RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
                .with_session("resumable"),
        ),
        transaction_owner: TransactionOwner {
            graph_node: Some("worker-node".into()),
            ..owner()
        },
        ..ToolContext::default()
    };
    assert!(execute_tool_with(root.path(), "patch_rollback", &args, &graph_context).is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"created");
    for session in ["other", "resumable"] {
        let resumed = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_session(session);
        let context = ToolContext {
            runtime: Some(resumed.clone()),
            ..ToolContext::default()
        };
        let result = execute_tool_with(root.path(), "patch_rollback", &args, &context);
        if session == "other" {
            assert!(result.is_err());
            continue;
        }
        let result = result.unwrap();
        assert_eq!(
            result.details.unwrap()["transaction"]["owner"],
            summary["owner"]
        );
        let effects = resumed.effect_ledger.read().unwrap();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].actor, resumed.agent_id);
    }
    assert!(!root.path().join("a.txt").exists());
}

#[test]
fn parent_session_cannot_adopt_worker_or_graph_transactions() {
    use davinci_agent::tools::{execute_tool_with, ToolContext};
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    for graph in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_session("shared-session");
        if !graph {
            runtime.parent_agent_id = Some(AgentId::new());
        }
        let context = ToolContext {
            runtime: Some(runtime),
            transaction_owner: TransactionOwner {
                graph_node: graph.then(|| "node".into()),
                ..owner()
            },
            ..ToolContext::default()
        };
        let result = execute_tool_with(
            root.path(),
            "write",
            &serde_json::json!({"path":"a.txt","content":"worker"}),
            &context,
        )
        .unwrap();
        let args = serde_json::json!({"id":result.details.unwrap()["transaction"]["id"],"paths":["a.txt"]});
        let resumed = ToolContext {
            runtime: Some(
                RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
                    .with_session("shared-session"),
            ),
            ..ToolContext::default()
        };
        assert!(execute_tool_with(root.path(), "patch_rollback", &args, &resumed).is_err());
        assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"worker");
        execute_tool_with(root.path(), "patch_rollback", &args, &context).unwrap();
        assert!(!root.path().join("a.txt").exists());
    }
}

#[test]
fn normal_write_edit_and_patch_report_recoverable_transactions() {
    use davinci_agent::tools::{execute_tool_with, ToolContext};
    let root = tempfile::tempdir().unwrap();
    let context = ToolContext::default();
    let manager =
        TransactionCoordinator::new(root.path(), context.transaction_owner.clone()).unwrap();
    for (tool, args, before, after) in [
        (
            "write",
            serde_json::json!({"path":"a.txt","content":"one\n"}),
            None,
            Some("one\n"),
        ),
        (
            "edit",
            serde_json::json!({"path":"a.txt","oldText":"one","newText":"two"}),
            None,
            None,
        ),
    ] {
        if tool == "edit" {
            fs::write(root.path().join("a.txt"), "one\n").unwrap();
        }
        let result = execute_tool_with(root.path(), tool, &args, &context).unwrap();
        assert!(!result.is_error, "{}", result.content);
        let details = result.details.unwrap();
        let id = details["transaction"]["id"].as_str().unwrap();
        assert_eq!(manager.status(id).unwrap().state, TransactionState::Applied);
        if let Some(after) = after {
            assert_eq!(
                fs::read_to_string(root.path().join("a.txt")).unwrap(),
                after
            );
        }
        manager.rollback(id, &|_| Ok(()), None).unwrap();
        if tool == "write" {
            assert!(!root.path().join("a.txt").exists());
        } else {
            assert_eq!(
                fs::read_to_string(root.path().join("a.txt")).unwrap(),
                before.unwrap_or("one\n")
            );
        }
    }
    let patch = serde_json::json!({"input":"*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+three\n*** Add File: b.txt\n+new\n*** End Patch"});
    let result = execute_tool_with(root.path(), "apply_patch", &patch, &context).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let details = result.details.unwrap();
    manager
        .rollback(
            details["transaction"]["id"].as_str().unwrap(),
            &|_| Ok(()),
            None,
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!root.path().join("b.txt").exists());
}

#[cfg(windows)]
#[test]
fn windows_replacement_preserves_existing_named_streams() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    fs::write(root.path().join("a.txt:transaction-test"), b"metadata").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(
        fs::read(root.path().join("a.txt:transaction-test")).unwrap(),
        b"metadata"
    );
    manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(
        fs::read(root.path().join("a.txt:transaction-test")).unwrap(),
        b"metadata"
    );
}

#[cfg(windows)]
#[test]
fn windows_deleted_named_streams_survive_recovery() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    fs::write(root.path().join("a.txt:transaction-test"), b"metadata").unwrap();
    fs::write(root.path().join("a.txt:empty"), []).unwrap();
    fs::write(root.path().join("a.txt:métadonnées"), b"unicode name").unwrap();
    let identity = owner();
    let manager = TransactionCoordinator::new(root.path(), identity.clone()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::delete("a.txt")])
        .unwrap();
    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    assert!(!root.path().join("a.txt").exists());
    drop(manager);
    let recovered = TransactionCoordinator::new(root.path(), identity).unwrap();
    recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before");
    assert_eq!(
        fs::read(root.path().join("a.txt:transaction-test")).unwrap(),
        b"metadata"
    );
    assert!(fs::read(root.path().join("a.txt:empty"))
        .unwrap()
        .is_empty());
    assert_eq!(
        fs::read(root.path().join("a.txt:métadonnées")).unwrap(),
        b"unicode name"
    );
}

#[cfg(windows)]
#[test]
fn windows_changed_named_streams_block_apply_and_rollback() {
    for after_apply in [false, true] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), b"before").unwrap();
        fs::write(root.path().join("a.txt:transaction-test"), b"metadata").unwrap();
        let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
        let preview = manager
            .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
            .unwrap();
        if after_apply {
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
        }
        fs::write(root.path().join("a.txt:transaction-test"), b"user update").unwrap();
        let result = if after_apply {
            manager.rollback(&preview.id, &|_| Ok(()), None)
        } else {
            manager.apply(&preview.id, &|_| Ok(()), None)
        };
        assert!(
            result.is_err(),
            "stream edit must conflict; after_apply={after_apply}"
        );
        assert_eq!(
            fs::read(root.path().join("a.txt:transaction-test")).unwrap(),
            b"user update"
        );
        assert_eq!(
            fs::read(root.path().join("a.txt")).unwrap(),
            if after_apply {
                b"after".as_slice()
            } else {
                b"before".as_slice()
            }
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_named_stream_limits_leave_source_unchanged() {
    for many in [false, true] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), b"before").unwrap();
        if many {
            for index in 0..65 {
                fs::write(root.path().join(format!("a.txt:stream-{index}")), []).unwrap();
            }
        } else {
            fs::File::create(root.path().join("a.txt:large"))
                .unwrap()
                .set_len(1024 * 1024 + 1)
                .unwrap();
        }
        let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
        assert!(manager
            .preview(vec![ProposedChange::delete("a.txt")])
            .is_err());
        assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before");
        if !many {
            assert_eq!(
                fs::metadata(root.path().join("a.txt:large")).unwrap().len(),
                1024 * 1024 + 1
            );
        }
    }
}

#[test]
fn transaction_stale_preview_refuses_every_write() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"old a").unwrap();
    fs::write(root.path().join("b.txt"), b"old b").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::write("a.txt", b"new a".to_vec()),
            ProposedChange::write("b.txt", b"new b".to_vec()),
        ])
        .unwrap();
    fs::write(root.path().join("b.txt"), b"other").unwrap();
    assert!(manager
        .apply(&preview.id, &|_| Ok(()), None)
        .unwrap_err()
        .contains("conflict"));
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"old a");
    assert_eq!(fs::read(root.path().join("b.txt")).unwrap(), b"other");
    assert_eq!(
        manager.status(&preview.id).unwrap().state,
        TransactionState::Conflicted
    );
}

#[test]
fn transaction_rollback_preserves_later_user_changes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    fs::write(root.path().join("a.txt"), b"user!").unwrap();
    assert!(manager.rollback(&preview.id, &|_| Ok(()), None).is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"user!");
}

#[test]
fn transaction_roundtrip_preserves_binary_preimage_and_owner() {
    let root = tempfile::tempdir().unwrap();
    let original = [0, 255, 10, 128];
    fs::write(root.path().join("old.bin"), original).unwrap();
    let owner = owner();
    let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::delete("old.bin"),
            ProposedChange::write("new.txt", b"created".to_vec()),
        ])
        .unwrap();
    let applied = manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(applied.state, TransactionState::Applied);
    assert!(!root.path().join("old.bin").exists());
    assert!(manager.apply(&preview.id, &|_| Ok(()), None).is_err());
    let stranger = TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    assert!(stranger.status(&preview.id).is_err());
    assert!(stranger.rollback(&preview.id, &|_| Ok(()), None).is_err());
    let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
    assert_eq!(
        recovered
            .rollback(&preview.id, &|_| Ok(()), None)
            .unwrap()
            .state,
        TransactionState::RolledBack
    );
    assert_eq!(fs::read(root.path().join("old.bin")).unwrap(), original);
    assert!(!root.path().join("new.txt").exists());
}

#[test]
fn transaction_denial_and_cancellation_prevent_writes() {
    use std::sync::atomic::AtomicBool;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    assert!(manager
        .apply(&preview.id, &|_| Err("denied".into()), None)
        .is_err());
    assert!(manager
        .apply(&preview.id, &|_| Ok(()), Some(&AtomicBool::new(true)))
        .is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before");
    assert_eq!(
        manager.status(&preview.id).unwrap().state,
        TransactionState::Previewed
    );
}

#[test]
fn transaction_partial_failure_recovers_only_owned_changes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    for file in ["a.txt", "b.txt"] {
        fs::write(root.path().join(file), b"before").unwrap();
    }
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::write("a.txt", b"after".to_vec()),
            ProposedChange::write("b.txt", b"after".to_vec()),
        ])
        .unwrap();
    let calls = AtomicUsize::new(0);
    let result = manager.apply(
        &preview.id,
        &|_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                return Err("injected before second mutation".into());
            }
            Ok(())
        },
        None,
    );
    assert!(result.unwrap_err().contains("rolled back"));
    for file in ["a.txt", "b.txt"] {
        assert_eq!(fs::read(root.path().join(file)).unwrap(), b"before");
    }
    assert_eq!(
        manager.status(&preview.id).unwrap().state,
        TransactionState::RolledBack
    );
}

#[test]
fn transaction_identical_bytes_in_a_new_file_are_not_owned() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    // Keep the old file alive so an inode/file-id cannot be recycled by this fixture.
    fs::rename(root.path().join("a.txt"), root.path().join("retained.txt")).unwrap();
    fs::write(root.path().join("a.txt"), b"after").unwrap();
    assert!(manager.rollback(&preview.id, &|_| Ok(()), None).is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
}

#[test]
fn transaction_cancellation_during_authority_check_recovers_prior_writes() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    for path in ["a.txt", "b.txt"] {
        fs::write(root.path().join(path), b"before").unwrap();
    }
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::write("a.txt", b"after".to_vec()),
            ProposedChange::write("b.txt", b"after".to_vec()),
        ])
        .unwrap();
    let abort = AtomicBool::new(false);
    let calls = AtomicUsize::new(0);
    let result = manager.apply(
        &preview.id,
        &|_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
                abort.store(true, Ordering::SeqCst);
            }
            Ok(())
        },
        Some(&abort),
    );
    assert!(
        result.is_err(),
        "cancellation was ignored during the final authority check"
    );
    assert_eq!(
        manager.status(&preview.id).unwrap().state,
        TransactionState::RolledBack
    );
    for path in ["a.txt", "b.txt"] {
        assert_eq!(fs::read(root.path().join(path)).unwrap(), b"before");
    }
}

#[test]
fn transaction_rejects_a_tampered_stage_before_replacing_the_source() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    let preview = manager
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    let calls = AtomicUsize::new(0);
    let result = manager.apply(
        &preview.id,
        &|_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                let stage = fs::read_dir(root.path())
                    .unwrap()
                    .map(Result::unwrap)
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".davinci-txn-")
                    })
                    .unwrap();
                fs::write(stage.path(), b"tampered").unwrap();
            }
            Ok(())
        },
        None,
    );
    assert!(result.is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before");
}

#[test]
fn transaction_rollback_cancellation_retains_a_resumable_journal() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    for path in ["a.txt", "b.txt"] {
        fs::write(root.path().join(path), b"before").unwrap();
    }
    let owner = owner();
    let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::write("a.txt", b"after".to_vec()),
            ProposedChange::write("b.txt", b"after".to_vec()),
        ])
        .unwrap();
    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let abort = AtomicBool::new(false);
    let calls = AtomicUsize::new(0);
    assert!(manager
        .rollback(
            &preview.id,
            &|_| {
                if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                    abort.store(true, Ordering::SeqCst);
                }
                Ok(())
            },
            Some(&abort)
        )
        .is_err());
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
    assert_eq!(fs::read(root.path().join("b.txt")).unwrap(), b"before");
    let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
    assert_eq!(
        recovered.status(&preview.id).unwrap().state,
        TransactionState::Conflicted
    );
    recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
    for path in ["a.txt", "b.txt"] {
        assert_eq!(fs::read(root.path().join(path)).unwrap(), b"before");
    }
    assert_eq!(
        recovered.status(&preview.id).unwrap().state,
        TransactionState::RolledBack
    );
}

#[test]
fn transaction_rejects_links_reserved_paths_and_duplicate_aliases() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    fs::hard_link(root.path().join("a.txt"), root.path().join("alias.txt")).unwrap();
    let manager = TransactionCoordinator::new(root.path(), owner()).unwrap();
    for path in [
        "a.txt",
        "alias.txt",
        "../escape",
        ".git/config",
        ".davinci-transactions/active.json",
        ".davinci_patch_journal.json",
        "x:stream",
    ] {
        assert!(
            manager
                .preview(vec![ProposedChange::write(path, b"after".to_vec())])
                .is_err(),
            "{path}"
        );
    }
    assert!(manager
        .preview(vec![
            ProposedChange::write("new.txt", vec![]),
            ProposedChange::write("./new.txt", vec![])
        ])
        .is_err());
}

#[test]
fn transaction_crash_helper() {
    let Some(root) = std::env::var_os("DAVINCI_TRANSACTION_TEST_ROOT") else {
        return;
    };
    let owner: TransactionOwner =
        serde_json::from_str(&std::env::var("DAVINCI_TRANSACTION_TEST_OWNER").unwrap()).unwrap();
    let id = std::env::var("DAVINCI_TRANSACTION_TEST_ID").unwrap();
    let manager = TransactionCoordinator::new(std::path::Path::new(&root), owner).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    manager
        .apply(
            &id,
            &|_| {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 3 {
                    std::process::exit(77);
                }
                Ok(())
            },
            None,
        )
        .unwrap();
    panic!("crash injection did not run");
}

#[test]
fn transaction_real_host_exit_between_writes_is_recoverable() {
    let root = tempfile::tempdir().unwrap();
    for file in ["a.txt", "b.txt"] {
        fs::write(root.path().join(file), b"before").unwrap();
    }
    let owner = owner();
    let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
    let preview = manager
        .preview(vec![
            ProposedChange::write("a.txt", b"after".to_vec()),
            ProposedChange::write("b.txt", b"after".to_vec()),
        ])
        .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "transaction_crash_helper"])
        .env("DAVINCI_TRANSACTION_TEST_ROOT", root.path())
        .env(
            "DAVINCI_TRANSACTION_TEST_OWNER",
            serde_json::to_string(&owner).unwrap(),
        )
        .env("DAVINCI_TRANSACTION_TEST_ID", &preview.id)
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(77),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
    assert_eq!(fs::read(root.path().join("b.txt")).unwrap(), b"before");
    assert_eq!(
        manager.status(&preview.id).unwrap().state,
        TransactionState::Applying
    );
    let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
    recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
    for file in ["a.txt", "b.txt"] {
        assert_eq!(fs::read(root.path().join(file)).unwrap(), b"before");
    }
}
