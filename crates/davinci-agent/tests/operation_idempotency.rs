use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
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
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let directory = temp_root.join("operations");
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

    fn open(&self) -> OperationJournal {
        OperationJournal::open(&self.directory, self.identity.clone(), self.root).unwrap()
    }

    fn spec(&self, key: &str, kind: OperationKind, payload: Value) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root,
                session_id: "session-idempotency-test".to_owned(),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: self.identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: Some("wire-call-42".to_owned()),
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, key).unwrap(),
            kind,
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

fn owner(generation: u64) -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), generation).unwrap()
}

fn initial(spec: &OperationSpec, owner: ExecutionOwner) -> OperationAttempt {
    OperationAttempt::new(spec.operation_id(), 1, owner).unwrap()
}

fn admit_after_bounded_contention(
    journal: &OperationJournal,
    spec: &OperationSpec,
    attempt: &OperationAttempt,
) -> OperationAdmission {
    for _ in 0..100 {
        match journal.admit(spec, attempt) {
            Ok(admission) => return admission,
            Err(JournalError::WriterBusy) => thread::yield_now(),
            Err(error) => panic!("unexpected admission error: {error}"),
        }
    }
    panic!("journal remained busy past the bounded retry window");
}

#[test]
fn concurrent_same_key_delivery_admits_one_operation_and_one_call_mapping() {
    let fixture = Fixture::new();
    let journal = Arc::new(fixture.open());
    let gate = Arc::new(Barrier::new(2));
    let shared_owner = owner(1);
    let spec_a = fixture.spec(
        "assistant-message/tool-42",
        OperationKind::ShellCommand,
        json!({"args": ["pwd"]}),
    );
    let spec_b = fixture.spec(
        "assistant-message/tool-42",
        OperationKind::ShellCommand,
        json!({"args": ["pwd"]}),
    );

    let handles = [spec_a, spec_b].map(|spec| {
        let journal = Arc::clone(&journal);
        let gate = Arc::clone(&gate);
        thread::spawn(move || {
            let attempt = initial(&spec, shared_owner);
            gate.wait();
            admit_after_bounded_contention(&journal, &spec, &attempt)
        })
    });
    let outcomes = handles.map(|handle| handle.join().unwrap());

    let admitted = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            OperationAdmission::New(record) => Some(record),
            _ => None,
        })
        .collect::<Vec<_>>();
    let duplicate = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            OperationAdmission::ExistingInFlight(record) => Some(record),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(admitted.len(), 1);
    assert_eq!(duplicate.len(), 1);
    assert_eq!(
        admitted[0].attempt.operation_id(),
        duplicate[0].attempt.operation_id()
    );

    let connection =
        rusqlite::Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME)).unwrap();
    let mappings: i64 = connection
        .query_row("SELECT count(*) FROM operation_call_mappings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(mappings, 1);
}

#[test]
fn same_key_with_different_arguments_is_a_collision_without_an_admission() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let owner = owner(1);
    let first = fixture.spec(
        "assistant-message/tool-43",
        OperationKind::ShellCommand,
        json!({"args": ["pwd"]}),
    );
    let conflicting = fixture.spec(
        "assistant-message/tool-43",
        OperationKind::ShellCommand,
        json!({"args": ["rm", "-rf", "x"]}),
    );

    assert!(matches!(
        journal.admit(&first, &initial(&first, owner)).unwrap(),
        OperationAdmission::New(_)
    ));
    assert!(matches!(
        journal
            .admit(&conflicting, &initial(&conflicting, owner))
            .unwrap(),
        OperationAdmission::Collision
    ));
}

#[test]
fn equal_arguments_under_different_commands_do_not_share_an_idempotency_identity() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let owner = owner(1);
    let args = json!({"path": "report.txt", "content": "approved"});
    let write = fixture.spec(
        "assistant-message/action",
        OperationKind::FileWrite,
        args.clone(),
    );
    let delete = fixture.spec("assistant-message/action", OperationKind::FileDelete, args);

    assert_eq!(write.payload_digest(), delete.payload_digest());
    assert_ne!(
        write.intent_digest().unwrap(),
        delete.intent_digest().unwrap()
    );
    assert!(matches!(
        journal.admit(&write, &initial(&write, owner)).unwrap(),
        OperationAdmission::New(_)
    ));
    assert!(matches!(
        journal.admit(&delete, &initial(&delete, owner)).unwrap(),
        OperationAdmission::Collision
    ));
}

#[test]
fn duplicate_terminal_admission_returns_the_full_durable_result() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let owner = owner(1);
    let spec = fixture.spec(
        "assistant-message/tool-44",
        OperationKind::FileWrite,
        json!({"path": "out.txt"}),
    );
    let mut admitted = match journal.admit(&spec, &initial(&spec, owner)).unwrap() {
        OperationAdmission::New(record) => record,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    admitted.attempt = journal
        .transition(
            owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "host-policy-v1".to_owned(),
                Timestamp::from_unix_millis(10),
            )),
            None,
            vec![],
        )
        .unwrap();
    admitted.attempt = journal
        .transition(
            owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();

    let claim = journal
        .claim_dispatch(
            owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
        )
        .unwrap();
    let effect_permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(20))
        .unwrap();
    assert_eq!(
        journal
            .load_attempt(admitted.attempt.attempt_id())
            .unwrap()
            .state(),
        OperationState::EffectPossible
    );
    assert_eq!(effect_permit.dispatch(&journal, || 1).unwrap(), 1);

    let payload = json!({"written": true, "bytes": 8});
    let result = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    let completed = journal
        .transition(
            owner,
            admitted.attempt.attempt_id(),
            journal
                .load_attempt(admitted.attempt.attempt_id())
                .unwrap()
                .revision(),
            OperationEvent::CompleteSuccess {
                result,
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(30),
            },
            Some(payload.clone()),
            vec![],
        )
        .unwrap();
    assert_eq!(completed.state(), OperationState::Succeeded);

    match journal.admit(&spec, &initial(&spec, owner)).unwrap() {
        OperationAdmission::ExistingResult(record) => {
            assert_eq!(record.attempt.state(), OperationState::Succeeded);
            assert_eq!(record.result.unwrap().payload, payload);
        }
        outcome => panic!("expected durable result replay, got {outcome:?}"),
    }
}
