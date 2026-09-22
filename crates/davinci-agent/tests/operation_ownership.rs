use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
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
        let directory = temp.path().join("operations");
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

    fn spec(&self) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root,
                session_id: "session-owner-test".to_owned(),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: self.identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: Some("owner-test-call".to_owned()),
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, "owner-test-key").unwrap(),
            OperationKind::CustomExternalAction,
            EffectProfile {
                classification: EffectClass::ExternalMutation,
                supports_idempotency_key: true,
                supports_postcondition_probe: false,
                supports_compensation: false,
                requires_live_owner: true,
            },
            json!({"action": "inspect"}),
            vec![],
        )
        .unwrap()
    }
}

fn owner(generation: u64) -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), generation).unwrap()
}

fn authorized_queued_attempt(
    journal: &OperationJournal,
    spec: &OperationSpec,
    attempt: OperationAttempt,
    owner: ExecutionOwner,
) -> OperationAttempt {
    let attempt = journal
        .transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "owner-policy-v1".to_owned(),
                Timestamp::from_unix_millis(10),
            )),
            None,
            vec![],
        )
        .unwrap();
    let attempt = journal
        .transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();
    attempt
}

fn queued_attempt(
    journal: &OperationJournal,
    spec: &OperationSpec,
    attempt: OperationAttempt,
    owner: ExecutionOwner,
) -> (OperationAttempt, DispatchPermit) {
    let attempt = authorized_queued_attempt(journal, spec, attempt, owner);
    let claim = journal
        .claim_dispatch(owner, attempt.attempt_id(), attempt.revision())
        .unwrap();
    (attempt, claim)
}

#[test]
fn stale_revision_is_rejected_even_for_the_current_owner() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let owner = owner(1);
    let attempt = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    let admitted = match journal.admit(&spec, &attempt).unwrap() {
        OperationAdmission::New(record) => record,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };

    assert!(matches!(
        journal.transition(
            owner,
            admitted.attempt.attempt_id(),
            0,
            OperationEvent::CancelBeforeStart {
                finished_at: Timestamp::from_unix_millis(20),
            },
            None,
            vec![],
        ),
        Err(JournalError::Transition(TransitionError::StaleRevision {
            expected: 0,
            actual: 1,
        }))
    ));
}

#[test]
fn replacement_generation_fences_old_owner_and_does_not_retry() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let admitted = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    let authorized = journal
        .transition(
            old_owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "owner-policy-v1".to_owned(),
                Timestamp::from_unix_millis(10),
            )),
            None,
            vec![],
        )
        .unwrap();
    let queued = journal
        .transition(
            old_owner,
            authorized.attempt_id(),
            authorized.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();
    let claim = journal
        .claim_dispatch(old_owner, queued.attempt_id(), queued.revision())
        .unwrap();
    let effect_permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(20))
        .unwrap();

    journal
        .fence_owner_after_quiescence(spec.operation_id(), old_owner, replacement, |owner| {
            if owner == old_owner {
                Ok(())
            } else {
                Err(())
            }
        })
        .unwrap();

    assert!(matches!(
        journal.transition(
            old_owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::CancelBeforeStart {
                finished_at: Timestamp::from_unix_millis(20),
            },
            None,
            vec![],
        ),
        Err(JournalError::OwnerFenced { .. })
    ));
    assert!(matches!(
        journal.claim_dispatch(
            replacement,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision()
        ),
        Err(JournalError::OwnerFenced { .. })
    ));
    let mut dispatch_called = false;
    assert!(matches!(
        effect_permit.dispatch(&journal, || dispatch_called = true),
        Err(JournalError::OwnerFenced { .. })
    ));
    assert!(!dispatch_called);

    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.attempts.len(), 1);
    assert_eq!(snapshot.attempts[0].attempt_number(), 1);
    assert_eq!(snapshot.attempts[0].state(), OperationState::EffectPossible);
}

#[test]
fn dispatch_claim_is_single_use_and_effect_latch_is_committed_before_dispatch() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let owner = owner(1);
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    let mut attempt = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    attempt = journal
        .transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "owner-policy-v1".to_owned(),
                Timestamp::from_unix_millis(10),
            )),
            None,
            vec![],
        )
        .unwrap();
    attempt = journal
        .transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();

    let mut calls = 0;
    assert!(matches!(
        journal.transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Start {
                at: Timestamp::from_unix_millis(20),
            },
            None,
            vec![],
        ),
        Err(JournalError::DispatchPermitRequired)
    ));
    assert!(matches!(
        journal.transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::MarkEffectPossible,
            None,
            vec![],
        ),
        Err(JournalError::DispatchPermitRequired)
    ));
    let claim = journal
        .claim_dispatch(owner, attempt.attempt_id(), attempt.revision())
        .unwrap();
    assert_eq!(calls, 0);
    assert!(matches!(
        journal.claim_dispatch(owner, attempt.attempt_id(), attempt.revision()),
        Err(JournalError::AttemptAlreadyClaimed)
    ));
    let permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(20))
        .unwrap();
    assert_eq!(
        journal.load_attempt(attempt.attempt_id()).unwrap().state(),
        OperationState::EffectPossible
    );
    assert_eq!(calls, 0);

    permit.dispatch(&journal, || calls += 1).unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn unclaimed_queued_attempt_cannot_be_recovered_or_retried() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let first = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    let queued = authorized_queued_attempt(&journal, &spec, first, old_owner);

    assert!(matches!(
        journal.transition(
            old_owner,
            queued.attempt_id(),
            queued.revision(),
            OperationEvent::RecoverUnstartedDispatch {
                evidence: davinci_agent::runtime::EvidenceId::new(),
            },
            None,
            vec![],
        ),
        Err(JournalError::CoordinatorTransitionRequired)
    ));
    journal
        .fence_owner_after_quiescence(spec.operation_id(), old_owner, replacement, |_| {
            Ok::<(), ()>(())
        })
        .unwrap();

    assert!(matches!(
        journal.recover_unstarted_dispatch_after_owner_fence(
            queued.attempt_id(),
            queued.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        ),
        Err(JournalError::DispatchNotReady)
    ));
    assert!(matches!(
        journal.admit_retry_after_owner_fence(
            queued.attempt_id(),
            queued.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        ),
        Err(JournalError::Transition(TransitionError::RetryNotReady))
    ));
}

#[test]
fn failed_quiescence_does_not_fence_or_enable_recovery() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let attempt = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };

    assert!(matches!(
        journal.fence_owner_after_quiescence(
            spec.operation_id(),
            old_owner,
            replacement,
            |_| Err::<(), _>(()),
        ),
        Err(JournalError::OwnerQuiescenceNotConfirmed)
    ));
    assert!(matches!(
        journal.admit_retry_after_owner_fence(
            attempt.attempt_id(),
            attempt.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        ),
        Err(JournalError::OwnerFenced {
            expected,
            actual
        }) if expected == replacement && actual == old_owner
    ));
}

#[test]
fn retry_gets_a_monotonic_attempt_only_after_quiescence_and_known_no_effect() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let first = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    let (queued, claim) = queued_attempt(&journal, &spec, first, old_owner);
    assert_eq!(queued.effect_status(), EffectStatus::NotStarted);
    journal
        .fence_owner_after_quiescence(spec.operation_id(), old_owner, replacement, |owner| {
            if owner == old_owner {
                Ok(())
            } else {
                Err(())
            }
        })
        .unwrap();

    assert!(matches!(
        journal.latch_effect_start(claim, Timestamp::from_unix_millis(20)),
        Err(JournalError::OwnerFenced { .. })
    ));

    let recoverable = journal
        .recover_unstarted_dispatch_after_owner_fence(
            queued.attempt_id(),
            queued.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        )
        .unwrap();
    assert_eq!(recoverable.state(), OperationState::RecoveryRequired);
    assert_eq!(recoverable.effect_status(), EffectStatus::NotStarted);
    assert!(recoverable.recovery_evidence().is_some());

    let retry = journal
        .admit_retry_after_owner_fence(
            recoverable.attempt_id(),
            recoverable.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        )
        .unwrap();
    assert_eq!(retry.operation_id(), spec.operation_id());
    assert_eq!(retry.attempt_number(), 2);
    assert_eq!(retry.retries_attempt_id(), Some(recoverable.attempt_id()));
    assert_eq!(retry.owner(), replacement);
    assert_eq!(retry.state(), OperationState::Persisted);
}

#[test]
fn retry_rechecks_the_durable_effect_latch_before_admitting_a_new_attempt() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let first = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    let (queued, _claim) = queued_attempt(&journal, &spec, first, old_owner);
    journal
        .fence_owner_after_quiescence(spec.operation_id(), old_owner, replacement, |_| {
            Ok::<(), ()>(())
        })
        .unwrap();
    let recovered = journal
        .recover_unstarted_dispatch_after_owner_fence(
            queued.attempt_id(),
            queued.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        )
        .unwrap();

    Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME))
        .unwrap()
        .execute(
            "UPDATE dispatch_claims SET effect_started = 1 WHERE attempt_id = ?1",
            [recovered.attempt_id().to_string()],
        )
        .unwrap();

    assert!(matches!(
        journal.admit_retry_after_owner_fence(
            recovered.attempt_id(),
            recovered.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        ),
        Err(JournalError::RetryEffectUncertain)
    ));
}

#[test]
fn retry_is_blocked_when_the_prior_effect_may_have_started() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec();
    let old_owner = owner(1);
    let replacement = ExecutionOwner::new(old_owner.id, 2).unwrap();
    let initial = OperationAttempt::new(spec.operation_id(), 1, old_owner).unwrap();
    let first = match journal.admit(&spec, &initial).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    let (queued, claim) = queued_attempt(&journal, &spec, first, old_owner);
    let effect_permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(20))
        .unwrap();
    drop(effect_permit);
    let effect_possible = journal.load_attempt(queued.attempt_id()).unwrap();
    let interrupted = journal
        .transition(
            old_owner,
            effect_possible.attempt_id(),
            effect_possible.revision(),
            OperationEvent::Interrupt {
                at: Timestamp::from_unix_millis(30),
            },
            None,
            vec![],
        )
        .unwrap();
    let recoverable = journal
        .transition(
            old_owner,
            interrupted.attempt_id(),
            interrupted.revision(),
            OperationEvent::RequireRecovery,
            None,
            vec![],
        )
        .unwrap();
    assert_eq!(recoverable.effect_status(), EffectStatus::Possible);
    journal
        .fence_owner_after_quiescence(spec.operation_id(), old_owner, replacement, |_| {
            Ok::<(), ()>(())
        })
        .unwrap();

    assert!(matches!(
        journal.admit_retry_after_owner_fence(
            recoverable.attempt_id(),
            recoverable.revision(),
            replacement,
            davinci_agent::runtime::EvidenceId::new(),
        ),
        Err(JournalError::RetryEffectUncertain)
    ));
}
