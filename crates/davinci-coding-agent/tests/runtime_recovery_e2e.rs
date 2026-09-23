use davinci_agent::runtime::operations::{
    CallerType, EffectClass, EffectProfile, ExecutionOwner, ExecutionOwnerId, IdempotencyScope,
    JournalId, JournalIdentity, OperationAttempt, OperationContext, OperationJournal,
    OperationKind, OperationSpec, RootNamespaceId, ScopedIdempotencyKey, WorkspaceId,
    WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId};
use serde_json::json;
use std::collections::BTreeSet;
use std::process::Command;
use tempfile::{tempdir, TempDir};

struct Fixture {
    _temp: TempDir,
    root: std::path::PathBuf,
    operation_id: String,
}

fn fixture() -> Fixture {
    let temp = tempdir().unwrap();
    let root_path = std::fs::canonicalize(temp.path()).unwrap();
    let journal_dir = root_path.join(".davinci").join("operations");
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let root_namespace_id = RootNamespaceId::new();
    let spec = OperationSpec::new(
        OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id,
            session_id: "runtime-recovery-e2e".to_owned(),
            runtime_run_id: RunId::new(),
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: CallerType::HostControl,
            wire_tool_call_id: Some("e2e-call".into()),
        },
        ScopedIdempotencyKey::new(IdempotencyScope::HostControl, "runtime-recovery-e2e").unwrap(),
        OperationKind::CustomExternalAction,
        EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: true,
            supports_postcondition_probe: true,
            supports_compensation: false,
            requires_live_owner: true,
        },
        json!({"fixture": true}),
        vec![],
    )
    .unwrap();
    let operation_id = spec.operation_id().to_string();
    let journal = OperationJournal::open(&journal_dir, identity, root_namespace_id).unwrap();
    let attempt = OperationAttempt::new(
        spec.operation_id(),
        1,
        ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
    )
    .unwrap();
    journal.persist_intent(&spec, &attempt).unwrap();
    let snapshot_dir = root_path.join("snapshot");
    journal.backup_to(&snapshot_dir).unwrap();
    drop(journal);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(journal_dir.join(format!("operations.sqlite3{suffix}")));
    }
    std::fs::copy(
        snapshot_dir.join("operations.sqlite3"),
        journal_dir.join("operations.sqlite3"),
    )
    .unwrap();
    Fixture {
        _temp: temp,
        root: root_path,
        operation_id,
    }
}

fn cli(fixture: &Fixture, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_davinci"))
        .current_dir(fixture.root.as_path())
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn inspector_reopens_a_real_fixture_without_starting_a_provider() {
    let fixture = fixture();
    let output = cli(
        &fixture,
        &["inspect", "operation", &fixture.operation_id, "--json"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["command"], "inspect_operation");
    assert_eq!(report["exit_code"], 0);
    assert_eq!(report["status"], "healthy");
    assert_eq!(
        report
            .pointer("/report/operations/0/identity/operation_id")
            .and_then(|value| value.as_str()),
        Some(fixture.operation_id.as_str())
    );
}

#[test]
fn inspector_is_restart_safe_and_does_not_mutate_journal_artifacts() {
    let fixture = fixture();
    let journal_dir = fixture.root.as_path().join(".davinci").join("operations");
    let before = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    let first = cli(
        &fixture,
        &["inspect", "operation", &fixture.operation_id, "--json"],
    );
    let second = cli(
        &fixture,
        &["inspect", "operation", &fixture.operation_id, "--json"],
    );
    let after = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(before, after);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn corrupt_journal_returns_diagnostic_exit_without_repairing_files() {
    let fixture = fixture();
    let journal_dir = fixture.root.as_path().join(".davinci").join("operations");
    let database = journal_dir.join("operations.sqlite3");
    let before = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    std::fs::write(&database, b"corrupt operation journal").unwrap();
    let output = cli(
        &fixture,
        &["inspect", "operation", &fixture.operation_id, "--json"],
    );
    let after = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(before, after);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["exit_code"], 3);
    assert_eq!(report["status"], "unavailable");
}

#[test]
fn empty_workspace_doctor_is_read_only_and_sessionless_mode_is_explicitly_ephemeral() {
    let temp = tempdir().unwrap();
    let doctor = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .current_dir(temp.path())
        .args(["doctor", "runtime", "--json"])
        .output()
        .unwrap();
    assert_eq!(doctor.status.code(), Some(3));
    assert!(!temp.path().join(".davinci").exists());
    let help = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .current_dir(temp.path())
        .arg("--help")
        .output()
        .unwrap();
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help.status.success());
    assert!(help_text.contains("--no-session"));
    assert!(help_text.contains("ephemeral"));
}
