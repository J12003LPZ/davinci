use davinci_agent::runtime::operations::{
    inspect, CallerType, EffectClass, EffectProfile, ExecutionOwner, ExecutionOwnerId,
    IdempotencyScope, InspectionTarget, InspectorStatus, JournalId, JournalIdentity,
    OperationAttempt, OperationContext, OperationJournal, OperationKind, OperationSpec,
    RootNamespaceId, ScopedIdempotencyKey, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId};
use davinci_coding_agent::runtime_inspect::{parse_command, InspectTarget, MaintenanceCommand};
use serde_json::json;
use std::collections::BTreeSet;
use std::process::Command;
use tempfile::{tempdir, TempDir};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

struct Fixture {
    temp: TempDir,
    root: std::path::PathBuf,
    operation_id: String,
    run_id: String,
    session_id: String,
}

fn fixture() -> Fixture {
    let temp = tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let journal_dir = root.join(".davinci").join("operations");
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let root_namespace_id = RootNamespaceId::new();
    let run_id = RunId::new();
    let session_id = "runtime-inspect-session".to_owned();
    let spec = OperationSpec::new(
        OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id,
            session_id: session_id.clone(),
            runtime_run_id: run_id,
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: CallerType::HostControl,
            wire_tool_call_id: Some("fixture-call".into()),
        },
        ScopedIdempotencyKey::new(IdempotencyScope::HostControl, "fixture-key").unwrap(),
        OperationKind::CustomExternalAction,
        EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: true,
            supports_postcondition_probe: true,
            supports_compensation: false,
            requires_live_owner: true,
        },
        json!({"redacted": true}),
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
    let snapshot_dir = root.join("snapshot");
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
        temp,
        root,
        operation_id,
        run_id: run_id.to_string(),
        session_id,
    }
}

#[test]
fn command_grammar_supports_operation_run_session_and_doctor() {
    assert!(matches!(
        parse_command(&args(&[
            "inspect",
            "operation",
            "00000000-0000-0000-0000-000000000001",
            "--json"
        ])),
        Ok(Some(MaintenanceCommand::Inspect {
            target: InspectTarget::Operation(_),
            json: true
        }))
    ));
    assert!(matches!(
        parse_command(&args(&[
            "inspect",
            "run",
            "00000000-0000-0000-0000-000000000001"
        ])),
        Ok(Some(MaintenanceCommand::Inspect {
            target: InspectTarget::Run(_),
            ..
        }))
    ));
    assert!(matches!(
        parse_command(&args(&["inspect", "session", "session-a"])),
        Ok(Some(MaintenanceCommand::Inspect {
            target: InspectTarget::Session(_),
            ..
        }))
    ));
    assert!(matches!(
        parse_command(&args(&["doctor", "runtime"])),
        Ok(Some(MaintenanceCommand::DoctorRuntime { json: false }))
    ));
}

#[test]
fn invalid_ids_and_literal_prompts_are_safe() {
    assert!(parse_command(&args(&["inspect", "operation", "bad-id"])).is_err());
    assert_eq!(
        parse_command(&args(&["--", "inspect", "operation", "bad-id"])),
        Ok(None)
    );
}

#[test]
fn missing_root_returns_diagnostic_exit_without_creating_files() {
    let temp = tempdir().unwrap();
    let output = inspect(temp.path(), InspectionTarget::DoctorRuntime);
    assert_eq!(output.status, InspectorStatus::Unavailable);
    assert_eq!(output.exit_code, 3);
    assert!(!temp.path().join(".davinci").exists());
}

#[test]
fn operation_run_and_session_views_are_read_only_and_bounded() {
    let fixture = fixture();
    let journal_path = fixture.root.as_path().join(".davinci").join("operations");
    let before = std::fs::read_dir(&journal_path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    let operation = inspect(
        fixture.root.as_path(),
        InspectionTarget::Operation(fixture.operation_id.clone()),
    );
    let run = inspect(
        fixture.root.as_path(),
        InspectionTarget::Run(fixture.run_id.clone()),
    );
    let session = inspect(
        fixture.root.as_path(),
        InspectionTarget::Session(fixture.session_id.clone()),
    );
    let after = std::fs::read_dir(&journal_path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(before, after);
    assert_eq!(operation.exit_code, 0);
    assert_eq!(run.exit_code, 0);
    assert_eq!(session.exit_code, 0);
    assert!(
        operation.json_string().len()
            <= davinci_agent::runtime::operations::INSPECTOR_MAX_OUTPUT_BYTES
    );
    assert!(operation
        .report
        .pointer("/operations/0/identity/operation_id")
        .is_some());
    assert_eq!(
        operation
            .report
            .pointer("/operations/0/requester/session_id")
            .and_then(|value| value.as_str()),
        Some(fixture.session_id.as_str())
    );
}

#[test]
fn corrupt_artifact_returns_unavailable_without_sidecars() {
    let fixture = fixture();
    let journal_dir = fixture.root.as_path().join(".davinci").join("operations");
    let database = journal_dir.join("operations.sqlite3");
    let before = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    std::fs::write(&database, b"not a sqlite database").unwrap();
    let output = inspect(
        fixture.root.as_path(),
        InspectionTarget::Operation(fixture.operation_id),
    );
    let after = std::fs::read_dir(&journal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(output.status, InspectorStatus::Unavailable);
    assert_eq!(output.exit_code, 3);
    assert_eq!(before, after);
}

#[test]
fn unknown_schema_returns_unavailable_without_repairing_database() {
    let fixture = fixture();
    let database = fixture
        .temp
        .path()
        .join(".davinci")
        .join("operations")
        .join("operations.sqlite3");
    let mut bytes = std::fs::read(&database).unwrap();
    bytes[60..64].copy_from_slice(&99_i32.to_be_bytes());
    std::fs::write(&database, bytes).unwrap();
    let output = inspect(
        fixture.root.as_path(),
        InspectionTarget::Operation(fixture.operation_id),
    );
    assert_eq!(output.status, InspectorStatus::Unavailable);
    assert_eq!(output.exit_code, 3);
    assert!(output
        .findings
        .iter()
        .any(|finding| finding.contains("unsupported journal schema")));
}

#[test]
fn generated_fixture_is_available_through_the_cli_without_provider_startup() {
    let fixture = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .current_dir(fixture.root.as_path())
        .args(["inspect", "operation", &fixture.operation_id, "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["exit_code"], 0);
    assert_eq!(report["command"], "inspect_operation");
    assert!(output.stderr.is_empty());
}

#[test]
fn doctor_reports_incomplete_state_without_repair() {
    let fixture = fixture();
    let output = inspect(fixture.root.as_path(), InspectionTarget::DoctorRuntime);
    assert!(matches!(
        output.status,
        InspectorStatus::Healthy | InspectorStatus::Findings
    ));
    assert!(output.report.pointer("/checks/repair").is_some());
    assert!(output
        .report
        .pointer("/checks/incomplete_operations")
        .is_some());
}
