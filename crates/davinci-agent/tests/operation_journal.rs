use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    directory: PathBuf,
    identity: JournalIdentity,
    root: RootNamespaceId,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("operation-journal");
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        Self {
            _temp: temp,
            directory,
            identity,
            root: RootNamespaceId::new(),
        }
    }

    fn open(&self) -> Result<OperationJournal, JournalError> {
        OperationJournal::open(&self.directory, self.identity.clone(), self.root)
    }

    fn spec(&self, key: &str, payload: Value) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root,
                session_id: "session-journal-test".to_owned(),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: self.identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: None,
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, key).unwrap(),
            OperationKind::CustomExternalAction,
            EffectProfile {
                classification: EffectClass::ExternalMutation,
                supports_idempotency_key: true,
                supports_postcondition_probe: false,
                supports_compensation: false,
                requires_live_owner: true,
            },
            payload,
            vec![],
        )
        .unwrap()
    }
}

fn owner() -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap()
}

fn apply_event(
    journal: &OperationJournal,
    attempt: &OperationAttempt,
    event: OperationEvent,
    result_payload: Option<Value>,
    outbox: Vec<OutboxDraft>,
) -> Result<OperationAttempt, JournalError> {
    journal.transition(
        attempt.attempt_id(),
        attempt.revision(),
        event,
        result_payload,
        outbox,
    )
}

fn running_with_possible_effect(
    journal: &OperationJournal,
    attempt: OperationAttempt,
) -> OperationAttempt {
    let attempt = apply_event(
        journal,
        &attempt,
        OperationEvent::Authorize(AuthorizationReceipt::new(
            PayloadDigest::of_bytes(b"approved operation"),
            "test-policy-v1".to_owned(),
            Timestamp::from_unix_millis(10),
        )),
        None,
        vec![],
    )
    .unwrap();
    let attempt = apply_event(journal, &attempt, OperationEvent::Queue, None, vec![]).unwrap();
    let attempt = apply_event(
        journal,
        &attempt,
        OperationEvent::Start {
            at: Timestamp::from_unix_millis(20),
        },
        None,
        vec![],
    )
    .unwrap();
    apply_event(
        journal,
        &attempt,
        OperationEvent::MarkEffectPossible,
        None,
        vec![],
    )
    .unwrap()
}

const CHILD_DIRECTORY: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_DIRECTORY";
const CHILD_JOURNAL_ID: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_JOURNAL_ID";
const CHILD_WORKSPACE_ID: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_WORKSPACE_ID";
const CHILD_BINDING_VERSION: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_BINDING_VERSION";
const CHILD_ROOT_ID: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_ROOT_ID";
const CHILD_OPERATION_ID: &str = "DAVINCI_OPERATION_JOURNAL_CHILD_OPERATION_ID";

#[test]
fn child_process_reopens_committed_intent() {
    let Ok(directory) = std::env::var(CHILD_DIRECTORY) else {
        return;
    };
    let identity = JournalIdentity::new(
        std::env::var(CHILD_JOURNAL_ID)
            .unwrap()
            .parse::<JournalId>()
            .unwrap(),
        WorkspaceIdentity {
            id: std::env::var(CHILD_WORKSPACE_ID)
                .unwrap()
                .parse::<WorkspaceId>()
                .unwrap(),
            binding_version: std::env::var(CHILD_BINDING_VERSION)
                .unwrap()
                .parse()
                .unwrap(),
        },
    )
    .unwrap();
    let root = std::env::var(CHILD_ROOT_ID)
        .unwrap()
        .parse::<RootNamespaceId>()
        .unwrap();
    let operation_id = std::env::var(CHILD_OPERATION_ID)
        .unwrap()
        .parse::<OperationId>()
        .unwrap();
    let journal = OperationJournal::open(Path::new(&directory), identity, root).unwrap();
    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot
        .operations
        .iter()
        .any(|spec| spec.operation_id() == operation_id));
    assert!(snapshot.attempts.iter().any(|attempt| {
        attempt.operation_id() == operation_id && attempt.state() == OperationState::Persisted
    }));
}

#[test]
fn intent_commit_is_durable_before_returning_and_survives_process_reopen() {
    let fixture = Fixture::new();
    let spec = fixture.spec("call-1", json!({"command": "inspect"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let operation_id = spec.operation_id();
    let attempt = fixture
        .open()
        .unwrap()
        .persist_intent(&spec, &initial)
        .unwrap();
    assert_eq!(attempt.state(), OperationState::Persisted);
    drop(attempt);

    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("child_process_reopens_committed_intent")
        .arg("--nocapture")
        .env(CHILD_DIRECTORY, &fixture.directory)
        .env(CHILD_JOURNAL_ID, fixture.identity.journal_id.to_string())
        .env(
            CHILD_WORKSPACE_ID,
            fixture.identity.workspace.id.to_string(),
        )
        .env(
            CHILD_BINDING_VERSION,
            fixture.identity.workspace.binding_version.to_string(),
        )
        .env(CHILD_ROOT_ID, fixture.root.to_string())
        .env(CHILD_OPERATION_ID, operation_id.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child reopen failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn transition_result_event_and_outbox_commit_together() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let spec = fixture.spec("call-atomic", json!({"action": "publish"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt);

    let payload = json!({"receipt": "remote-42", "accepted": true});
    let reference = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    let consumer = "session_projection";
    let completed = apply_event(
        &journal,
        &attempt,
        OperationEvent::CompleteSuccess {
            result: reference.clone(),
            effect_status: EffectStatus::EffectsObserved,
            finished_at: Timestamp::from_unix_millis(50),
        },
        Some(payload.clone()),
        vec![OutboxDraft::new(consumer, json!({"operation": spec.operation_id()})).unwrap()],
    )
    .unwrap();

    assert_eq!(completed.state(), OperationState::Succeeded);
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(
        snapshot.events.last().unwrap().attempt_id,
        completed.attempt_id()
    );
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].reference, reference);
    assert_eq!(snapshot.results[0].payload, payload);
    assert_eq!(snapshot.outbox.len(), 1);
    assert_eq!(snapshot.outbox[0].consumer, consumer);
    assert_eq!(journal.pending_outbox(1).unwrap().len(), 1);
}

#[test]
fn failed_durable_write_rolls_back_and_poisons_until_reopen() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let spec = fixture.spec("call-failure", json!({"action": "publish"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt);
    Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_outbox_insert BEFORE INSERT ON operation_outbox
             BEGIN SELECT RAISE(ABORT, 'injected durable-write failure'); END;",
        )
        .unwrap();

    let payload = json!({"receipt": "remote-43"});
    let reference = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    let failed = apply_event(
        &journal,
        &attempt,
        OperationEvent::CompleteSuccess {
            result: reference,
            effect_status: EffectStatus::EffectsObserved,
            finished_at: Timestamp::from_unix_millis(60),
        },
        Some(payload),
        vec![OutboxDraft::new("projection", json!({"publish": true})).unwrap()],
    );
    assert!(matches!(failed, Err(JournalError::Sqlite(_))));
    assert!(matches!(
        apply_event(
            &journal,
            &attempt,
            OperationEvent::RequestCancellation,
            None,
            vec![]
        ),
        Err(JournalError::Poisoned)
    ));
    drop(journal);

    let reopened = fixture.open().unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.events.len(), 5);
    assert_eq!(snapshot.attempts[0].revision(), attempt.revision());
    assert_eq!(snapshot.attempts[0].state(), OperationState::EffectPossible);
    assert!(snapshot.results.is_empty());
    assert!(snapshot.outbox.is_empty());
}

#[test]
fn a_second_coordinator_cannot_claim_the_same_root_but_other_roots_can() {
    let fixture = Fixture::new();
    let first = fixture.open().unwrap();
    assert!(matches!(
        fixture.open(),
        Err(JournalError::CoordinatorAlreadyOwned)
    ));
    let other_root = RootNamespaceId::new();
    assert!(
        OperationJournal::open(&fixture.directory, fixture.identity.clone(), other_root).is_ok()
    );
    drop(first);
    assert!(fixture.open().is_ok());
}

#[test]
fn opening_rejects_workspace_schema_and_operation_root_mismatches() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let mut wrong_workspace = fixture.identity.clone();
    wrong_workspace.workspace.id = WorkspaceId::new();
    assert!(matches!(
        OperationJournal::open(&fixture.directory, wrong_workspace, fixture.root),
        Err(JournalError::WorkspaceMismatch { .. })
    ));

    assert!(
        Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME))
            .unwrap()
            .execute("UPDATE journal_metadata SET workspace_id = 'rewritten'", [])
            .is_err()
    );

    let wrong_root = RootNamespaceId::new();
    let mut spec = fixture.spec("call-wrong-root", json!({"command": "inspect"}));
    let mut encoded = serde_json::to_value(&spec).unwrap();
    encoded["context"]["root_namespace_id"] = json!(wrong_root);
    spec = serde_json::from_value(encoded).unwrap();
    let attempt = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    assert!(matches!(
        journal.persist_intent(&spec, &attempt),
        Err(JournalError::RootMismatch { .. })
    ));
    drop(journal);

    Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME))
        .unwrap()
        .pragma_update(None, "user_version", 77)
        .unwrap();
    assert!(matches!(
        fixture.open(),
        Err(JournalError::UnsupportedSchema(77))
    ));
}

#[test]
fn corrupt_records_fail_integrity_checks_and_poison_admission() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let spec = fixture.spec("call-corrupt", json!({"command": "inspect"}));
    let attempt = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &attempt).unwrap();

    let connection = Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME)).unwrap();
    connection
        .execute_batch("DROP TRIGGER attempts_revision_guard")
        .unwrap();
    connection
        .execute(
            "UPDATE attempts SET revision = revision + 1, attempt_json = '{' WHERE attempt_id = ?1",
            [attempt.attempt_id().to_string()],
        )
        .unwrap();

    assert!(matches!(
        journal.snapshot(),
        Err(JournalError::Integrity(_))
    ));
    let second_spec = fixture.spec("call-after-corruption", json!({"command": "inspect"}));
    let second_attempt = OperationAttempt::new(second_spec.operation_id(), 1, owner()).unwrap();
    assert!(matches!(
        journal.persist_intent(&second_spec, &second_attempt),
        Err(JournalError::Poisoned)
    ));
}

#[test]
fn oversized_records_and_outbox_batches_are_rejected_without_partial_writes() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let oversized = fixture.spec("call-large", json!({"payload": "x".repeat(1_100_000)}));
    let initial = OperationAttempt::new(oversized.operation_id(), 1, owner()).unwrap();
    assert!(matches!(
        journal.persist_intent(&oversized, &initial),
        Err(JournalError::OversizedRecord { .. })
    ));
    assert!(journal.snapshot().unwrap().operations.is_empty());

    let spec = fixture.spec("call-bounded-outbox", json!({"action": "publish"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt);
    let payload = json!({"receipt": "remote-44"});
    let reference = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    let batch = vec![
        OutboxDraft::new("projection", json!({"publish": true})).unwrap();
        MAX_OUTBOX_BATCH_SIZE + 1
    ];
    assert!(matches!(
        apply_event(
            &journal,
            &attempt,
            OperationEvent::CompleteSuccess {
                result: reference,
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(70),
            },
            Some(payload),
            batch,
        ),
        Err(JournalError::OutboxBatchTooLarge { .. })
    ));
    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot.results.is_empty());
    assert!(snapshot.outbox.is_empty());
    assert_eq!(snapshot.attempts[0].state(), OperationState::EffectPossible);
}

#[test]
fn online_backup_is_a_standalone_consistent_database() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let spec = fixture.spec("call-backup", json!({"command": "inspect"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    journal.persist_intent(&spec, &initial).unwrap();

    let backup_directory = fixture._temp.path().join("backup");
    journal.backup_to(&backup_directory).unwrap();
    drop(journal);

    assert!(backup_directory.join(JOURNAL_DATABASE_FILE_NAME).is_file());
    assert!(!Path::new(&format!(
        "{}-wal",
        backup_directory.join(JOURNAL_DATABASE_FILE_NAME).display()
    ))
    .exists());

    let backup =
        OperationJournal::open(&backup_directory, fixture.identity.clone(), fixture.root).unwrap();
    let snapshot = backup.snapshot().unwrap();
    assert_eq!(snapshot.operations.len(), 1);
    assert_eq!(snapshot.operations[0].operation_id(), spec.operation_id());
    assert_eq!(snapshot.attempts[0].state(), OperationState::Persisted);
    drop(backup);
}

#[test]
fn pending_outbox_reads_enforce_the_batch_bound_and_acknowledgements_are_durable() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    assert!(matches!(
        journal.pending_outbox(MAX_OUTBOX_BATCH_SIZE + 1),
        Err(JournalError::OutboxBatchTooLarge { .. })
    ));

    let spec = fixture.spec("call-ack", json!({"action": "publish"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt);
    let payload = json!({"receipt": "remote-45"});
    let reference = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    apply_event(
        &journal,
        &attempt,
        OperationEvent::CompleteSuccess {
            result: reference,
            effect_status: EffectStatus::EffectsObserved,
            finished_at: Timestamp::from_unix_millis(80),
        },
        Some(payload),
        vec![OutboxDraft::new("projection", json!({"done": true})).unwrap()],
    )
    .unwrap();
    let pending = journal.pending_outbox(1).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(journal
        .acknowledge_outbox(pending[0].id, Timestamp::from_unix_millis(90))
        .unwrap());
    assert!(journal.pending_outbox(1).unwrap().is_empty());
}

#[test]
fn same_scoped_key_with_a_different_payload_is_a_hard_collision() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let first = fixture.spec("same-key", json!({"command": "inspect"}));
    let first_attempt = OperationAttempt::new(first.operation_id(), 1, owner()).unwrap();
    journal.persist_intent(&first, &first_attempt).unwrap();

    let changed = fixture.spec("same-key", json!({"command": "delete"}));
    let changed_attempt = OperationAttempt::new(changed.operation_id(), 1, owner()).unwrap();
    assert!(matches!(
        journal.persist_intent(&changed, &changed_attempt),
        Err(JournalError::IdempotencyCollision)
    ));
}

#[test]
fn backup_target_must_be_a_new_private_directory() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    assert!(matches!(
        journal.backup_to(&fixture.directory),
        Err(JournalError::InvalidBackupPath)
    ));
    let unrelated = fixture._temp.path().join("unrelated.sqlite3");
    std::fs::write(&unrelated, b"not a directory").unwrap();
    assert!(journal.backup_to(&unrelated).is_err());
}
