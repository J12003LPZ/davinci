use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use serde_json::json;
use std::path::PathBuf;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    directory: PathBuf,
    identity: JournalIdentity,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        Self {
            directory: temp_root.join("journal"),
            _temp: temp,
            identity,
        }
    }

    fn journal(&self, root: RootNamespaceId) -> OperationJournal {
        OperationJournal::open(&self.directory, self.identity.clone(), root).unwrap()
    }

    fn spec(
        &self,
        root: RootNamespaceId,
        key: &str,
        kind: OperationKind,
        classification: EffectClass,
        preconditions: Vec<Precondition>,
    ) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: root,
                session_id: format!("recovery-{key}"),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: self.identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: Some(format!("wire-{key}")),
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, key).unwrap(),
            kind,
            EffectProfile {
                classification,
                supports_idempotency_key: classification == EffectClass::IdempotentMutation,
                supports_postcondition_probe: true,
                supports_compensation: false,
                requires_live_owner: true,
            },
            json!({"action": key}),
            preconditions,
        )
        .unwrap()
    }
}

fn owner(generation: u64) -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), generation).unwrap()
}

fn resource(path: &str) -> Precondition {
    Precondition {
        kind: PreconditionKind::ResourceVersion,
        resource: path.to_owned(),
        expected_digest: None,
    }
}

fn begin_effect(
    journal: &OperationJournal,
    spec: &OperationSpec,
    owner: ExecutionOwner,
) -> OperationAttempt {
    let attempt = queue_attempt(journal, spec, owner);
    let claim = journal
        .claim_dispatch(owner, attempt.attempt_id(), attempt.revision())
        .unwrap();
    drop(
        journal
            .latch_effect_start(claim, Timestamp::from_unix_millis(2))
            .unwrap(),
    );
    journal.load_attempt(attempt.attempt_id()).unwrap()
}

fn queue_attempt(
    journal: &OperationJournal,
    spec: &OperationSpec,
    owner: ExecutionOwner,
) -> OperationAttempt {
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    let mut attempt = match journal.admit(spec, &initial).unwrap() {
        OperationAdmission::New(admitted) => admitted.attempt,
        outcome => panic!("unexpected admission: {outcome:?}"),
    };
    attempt = journal
        .transition(
            owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "recovery-test-policy".to_owned(),
                Timestamp::from_unix_millis(1),
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
    attempt
}

fn recovery_attempt(
    spec: &OperationSpec,
    owner: ExecutionOwner,
    phase: RecoveryPhase,
) -> Vec<OperationAttempt> {
    if phase == RecoveryPhase::BeforeIntentCommit {
        return Vec::new();
    }
    let mut attempt = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    attempt = transition_attempt(&attempt, 0, OperationEvent::Persist).unwrap();
    if matches!(
        phase,
        RecoveryPhase::AfterAuthorizationBeforeClaim
            | RecoveryPhase::AfterClaimBeforeUnsafeBoundary
            | RecoveryPhase::AfterEffectLatchBeforeSyscall
            | RecoveryPhase::AfterProcessSpawnBeforeReceipt
            | RecoveryPhase::AfterFileEffectBeforeResult
            | RecoveryPhase::DuringMultiFileApplyOrRollback
            | RecoveryPhase::AfterExternalRequestBeforeResponse
            | RecoveryPhase::AfterResultBeforeNotification
            | RecoveryPhase::DuringSessionAppend
            | RecoveryPhase::DuringVerification
            | RecoveryPhase::DuringGraphWorkerCompletion
            | RecoveryPhase::DuringCancellationOrTermination
    ) {
        attempt = transition_attempt(
            &attempt,
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "recovery-test-policy".to_owned(),
                Timestamp::from_unix_millis(1),
            )),
        )
        .unwrap();
    }
    if matches!(
        phase,
        RecoveryPhase::AfterClaimBeforeUnsafeBoundary
            | RecoveryPhase::AfterEffectLatchBeforeSyscall
            | RecoveryPhase::AfterProcessSpawnBeforeReceipt
            | RecoveryPhase::AfterFileEffectBeforeResult
            | RecoveryPhase::DuringMultiFileApplyOrRollback
            | RecoveryPhase::AfterExternalRequestBeforeResponse
            | RecoveryPhase::AfterResultBeforeNotification
            | RecoveryPhase::DuringSessionAppend
            | RecoveryPhase::DuringVerification
            | RecoveryPhase::DuringGraphWorkerCompletion
            | RecoveryPhase::DuringCancellationOrTermination
    ) {
        attempt = transition_attempt(&attempt, attempt.revision(), OperationEvent::Queue).unwrap();
    }
    if matches!(
        phase,
        RecoveryPhase::AfterEffectLatchBeforeSyscall
            | RecoveryPhase::AfterProcessSpawnBeforeReceipt
            | RecoveryPhase::AfterFileEffectBeforeResult
            | RecoveryPhase::DuringMultiFileApplyOrRollback
            | RecoveryPhase::AfterExternalRequestBeforeResponse
            | RecoveryPhase::AfterResultBeforeNotification
            | RecoveryPhase::DuringSessionAppend
            | RecoveryPhase::DuringVerification
            | RecoveryPhase::DuringGraphWorkerCompletion
            | RecoveryPhase::DuringCancellationOrTermination
    ) {
        attempt = transition_attempt(
            &attempt,
            attempt.revision(),
            OperationEvent::Start {
                at: Timestamp::from_unix_millis(2),
            },
        )
        .unwrap();
        attempt = transition_attempt(
            &attempt,
            attempt.revision(),
            OperationEvent::MarkEffectPossible,
        )
        .unwrap();
    }
    if matches!(
        phase,
        RecoveryPhase::AfterResultBeforeNotification
            | RecoveryPhase::DuringSessionAppend
            | RecoveryPhase::DuringVerification
            | RecoveryPhase::DuringGraphWorkerCompletion
    ) {
        attempt = transition_attempt(
            &attempt,
            attempt.revision(),
            OperationEvent::CompleteSuccess {
                result: ResultRef::new(PayloadDigest::of_bytes(b"durable-result")),
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(3),
            },
        )
        .unwrap();
    }
    vec![attempt]
}

#[path = "operation_recovery/cases.rs"]
mod cases;
