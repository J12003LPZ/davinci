use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, EvidenceId, RunId, TaskId};
use serde_json::{json, Value};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
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
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        Self {
            directory: temp.path().join("operations"),
            _temp: temp,
            identity,
            root: RootNamespaceId::new(),
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
        payload: Value,
        preconditions: Vec<Precondition>,
    ) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: root,
                session_id: format!("property-{key}"),
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
                supports_idempotency_key: matches!(
                    classification,
                    EffectClass::IdempotentMutation | EffectClass::ExternalMutation
                ),
                supports_postcondition_probe: true,
                supports_compensation: false,
                requires_live_owner: true,
            },
            payload,
            preconditions,
        )
        .unwrap()
    }
}

fn owner(generation: u64) -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), generation).unwrap()
}

fn next_seed(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *seed
}

fn record_failure(seed: u64, sequence: &[String]) -> PathBuf {
    let path = std::env::temp_dir().join("davinci-operation-properties-failing-seed.txt");
    let body = format!("seed={seed}\nsequence={}\n", sequence.join("\n"));
    let _ = fs::write(&path, body);
    path
}

fn assert_property(seed: u64, sequence: &[String], condition: bool) {
    if !condition {
        let path = record_failure(seed, sequence);
        panic!(
            "property failed for seed {seed}; minimized sequence at {}",
            path.display()
        );
    }
}

fn initial(spec: &OperationSpec, owner: ExecutionOwner) -> OperationAttempt {
    OperationAttempt::new(spec.operation_id(), 1, owner).unwrap()
}

fn queue_attempt(
    journal: &OperationJournal,
    spec: &OperationSpec,
    owner: ExecutionOwner,
) -> OperationAttempt {
    let admitted = match journal.admit(spec, &initial(spec, owner)).unwrap() {
        OperationAdmission::New(admitted) => admitted.attempt,
        outcome => panic!("expected new operation, got {outcome:?}"),
    };
    let authorized = journal
        .transition(
            owner,
            admitted.attempt_id(),
            admitted.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "property-policy-v1".to_owned(),
                Timestamp::from_unix_millis(1),
            )),
            None,
            vec![],
        )
        .unwrap();
    journal
        .transition(
            owner,
            authorized.attempt_id(),
            authorized.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap()
}

fn effect_started(
    journal: &OperationJournal,
    spec: &OperationSpec,
    owner: ExecutionOwner,
) -> OperationAttempt {
    let queued = queue_attempt(journal, spec, owner);
    let claim = journal
        .claim_dispatch(owner, queued.attempt_id(), queued.revision())
        .unwrap();
    let permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(2))
        .unwrap();
    permit
        .dispatch(journal, || ())
        .expect("effect permit must dispatch exactly once");
    journal.load_attempt(queued.attempt_id()).unwrap()
}

fn resource(path: &str) -> Precondition {
    Precondition {
        kind: PreconditionKind::ResourceVersion,
        resource: path.to_owned(),
        expected_digest: None,
    }
}

fn recovery_input(
    spec: OperationSpec,
    attempts: Vec<OperationAttempt>,
    phase: RecoveryPhase,
    observations: RecoveryObservations,
) -> RecoveryInput {
    RecoveryInput {
        spec,
        attempts,
        phase,
        observations,
        policy_version: "recovery-policy-v1".to_owned(),
    }
}

#[test]
fn deterministic_same_key_sequences_never_admit_a_conflicting_intent() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let shared_owner = owner(1);

    for mut seed in [3_u64, 17, 101, 4_096, 65_535] {
        for step in 0..8_u64 {
            let value = next_seed(&mut seed);
            let key = format!("collision-{step}-{value}");
            let first = fixture.spec(
                fixture.root,
                &key,
                OperationKind::ShellCommand,
                EffectClass::ExternalMutation,
                json!({"command": "echo", "value": value}),
                vec![],
            );
            let conflicting = fixture.spec(
                fixture.root,
                &key,
                OperationKind::ShellCommand,
                EffectClass::ExternalMutation,
                json!({"command": "echo", "value": value.wrapping_add(1)}),
                vec![],
            );
            let sequence = vec![format!("admit {key}"), format!("collide {key}")];
            assert_property(
                value,
                &sequence,
                matches!(
                    journal.admit(&first, &initial(&first, shared_owner)),
                    Ok(OperationAdmission::New(_))
                ),
            );
            assert_property(
                value,
                &sequence,
                matches!(
                    journal.admit(&conflicting, &initial(&conflicting, shared_owner)),
                    Ok(OperationAdmission::Collision)
                ),
            );
        }
    }
}

#[test]
fn duplicate_delivery_has_one_effect_per_operation() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let spec = fixture.spec(
        fixture.root,
        "duplicate-delivery",
        OperationKind::FileWrite,
        EffectClass::IdempotentMutation,
        json!({"path": "out.txt", "content": "once"}),
        vec![resource("workspace/out.txt")],
    );
    let owner = owner(1);
    let admitted = match journal.admit(&spec, &initial(&spec, owner)).unwrap() {
        OperationAdmission::New(record) => record.attempt,
        outcome => panic!("expected first admission, got {outcome:?}"),
    };
    let duplicate = journal.admit(&spec, &initial(&spec, owner)).unwrap();
    assert!(matches!(duplicate, OperationAdmission::ExistingInFlight(_)));

    let authorized = journal
        .transition(
            owner,
            admitted.attempt_id(),
            admitted.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "property-policy-v1".to_owned(),
                Timestamp::from_unix_millis(1),
            )),
            None,
            vec![],
        )
        .unwrap();
    let queued = journal
        .transition(
            owner,
            authorized.attempt_id(),
            authorized.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();
    let claim = journal
        .claim_dispatch(owner, queued.attempt_id(), queued.revision())
        .unwrap();
    let permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(2))
        .unwrap();
    let mut effects = 0;
    permit
        .dispatch(&journal, || {
            effects += 1;
        })
        .unwrap();
    assert_eq!(effects, 1);
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
}

#[test]
fn successful_siblings_survive_a_terminal_result_and_restart() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let owner = owner(1);
    let first = fixture.spec(
        fixture.root,
        "sibling-a",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        json!({"path": "a.txt"}),
        vec![resource("workspace/a.txt")],
    );
    let second = fixture.spec(
        fixture.root,
        "sibling-b",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        json!({"path": "b.txt"}),
        vec![resource("workspace/b.txt")],
    );
    let first_attempt = effect_started(&journal, &first, owner);
    let second_attempt = queue_attempt(&journal, &second, owner);
    let payload = json!({"written": "a"});
    journal
        .transition(
            owner,
            first_attempt.attempt_id(),
            first_attempt.revision(),
            OperationEvent::CompleteSuccess {
                result: ResultRef::new(PayloadDigest::of_json(&payload).unwrap()),
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(3),
            },
            Some(payload.clone()),
            vec![],
        )
        .unwrap();
    drop(journal);

    let restarted = fixture.journal(fixture.root);
    let snapshot = restarted.snapshot().unwrap();
    assert_eq!(snapshot.operations.len(), 2);
    assert_eq!(
        snapshot
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id() == second_attempt.attempt_id())
            .map(OperationAttempt::state),
        Some(OperationState::Queued)
    );
    match restarted.admit(&first, &initial(&first, owner)).unwrap() {
        OperationAdmission::ExistingResult(record) => {
            assert_eq!(record.result.unwrap().payload, payload)
        }
        outcome => panic!("expected full result replay, got {outcome:?}"),
    }
}

#[test]
fn recovery_never_promotes_unknown_effects_or_termination_to_success() {
    let fixture = Fixture::new();
    let spec = fixture.spec(
        fixture.root,
        "unknown-outcome",
        OperationKind::McpCall,
        EffectClass::ExternalMutation,
        json!({"remote": "fixture"}),
        vec![],
    );
    let unknown = RecoveryEngine::reduce(&recovery_input(
        spec.clone(),
        vec![],
        RecoveryPhase::AfterExternalRequestBeforeResponse,
        RecoveryObservations {
            remote: RemoteObservation::Unknown,
            ..RecoveryObservations::default()
        },
    ));
    assert_eq!(
        unknown.classification,
        RecoveryClassification::NeedsReconciliation
    );
    assert_eq!(unknown.action, RecoveryAction::ReconcileDomain);
    assert_ne!(unknown.action, RecoveryAction::RecordObservedOutcome);

    let termination = RecoveryEngine::reduce(&recovery_input(
        spec,
        vec![],
        RecoveryPhase::DuringCancellationOrTermination,
        RecoveryObservations {
            owner: OwnerObservation::Unknown,
            process: ProcessObservation::Live,
            ..RecoveryObservations::default()
        },
    ));
    assert_eq!(
        termination.classification,
        RecoveryClassification::NeedsHumanDecision
    );
    assert_eq!(termination.action, RecoveryAction::ReconcileTermination);
    assert_ne!(termination.action, RecoveryAction::RecordObservedOutcome);
}

#[test]
fn fenced_retry_preserves_operation_lineage_and_requires_a_new_owner_generation() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let old_owner = owner(1);
    let replacement = owner(2);
    let spec = fixture.spec(
        fixture.root,
        "retry-lineage",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        json!({"path": "retry.txt"}),
        vec![resource("workspace/retry.txt")],
    );
    let queued = queue_attempt(&journal, &spec, old_owner);
    drop(
        journal
            .claim_dispatch(old_owner, queued.attempt_id(), queued.revision())
            .unwrap(),
    );
    let result = RecoveryEngine::run_authorized(
        &journal,
        old_owner,
        recovery_input(
            spec.clone(),
            vec![queued.clone()],
            RecoveryPhase::AfterClaimBeforeUnsafeBoundary,
            RecoveryObservations {
                owner: OwnerObservation::Quiescent,
                authorization: AuthorizationObservation::Current,
                dispatch: DispatchObservation::ClaimedUnstarted,
                ..RecoveryObservations::default()
            },
        ),
        Some(replacement),
        EvidenceId::new(),
        |_| Ok::<(), String>(()),
        |_| Ok::<(), String>(()),
        |_, retry_attempt| Ok::<_, String>(retry_attempt.map(OperationAttempt::attempt_number)),
    )
    .unwrap();
    let retry = result
        .retry_attempt
        .expect("safe retry must create an attempt");
    assert_eq!(retry.operation_id(), spec.operation_id());
    assert_eq!(retry.attempt_number(), 2);
    assert_eq!(retry.owner(), replacement);
    assert_eq!(result.result, Some(Some(2)));
    assert!(journal
        .snapshot()
        .unwrap()
        .attempts
        .iter()
        .any(|attempt| attempt.attempt_id() == queued.attempt_id()));
}

#[test]
fn a_fresh_key_cannot_bypass_an_unresolved_effect_claim() {
    let fixture = Fixture::new();
    let first_root = fixture.root;
    let second_root = RootNamespaceId::new();
    let first_journal = fixture.journal(first_root);
    let second_journal = fixture.journal(second_root);
    let first = fixture.spec(
        first_root,
        "unresolved-first",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        json!({"path": "shared.txt"}),
        vec![resource("workspace/shared.txt")],
    );
    let _ = effect_started(&first_journal, &first, owner(1));

    for mut seed in [7_u64, 31, 127, 9_001] {
        for step in 0..5_u64 {
            let value = next_seed(&mut seed);
            let candidate = fixture.spec(
                second_root,
                &format!("fresh-key-{seed}-{step}-{value}"),
                OperationKind::FileWrite,
                EffectClass::ReversibleMutation,
                json!({"path": "shared.txt", "value": value}),
                vec![resource("workspace/shared.txt")],
            );
            assert!(matches!(
                second_journal.admit(&candidate, &initial(&candidate, owner(1))),
                Err(JournalError::UnresolvedEffectConflict { .. })
            ));
        }
    }
}

#[test]
fn corruption_and_unknown_schema_fail_closed_without_repair() {
    let fixture = Fixture::new();
    let database = fixture.directory.join(JOURNAL_DATABASE_FILE_NAME);
    drop(fixture.journal(fixture.root));
    fs::write(&database, b"not a sqlite database").unwrap();
    assert!(
        OperationJournal::open(&fixture.directory, fixture.identity.clone(), fixture.root).is_err()
    );

    let fresh = Fixture::new();
    drop(fresh.journal(fresh.root));
    rusqlite::Connection::open(fresh.directory.join(JOURNAL_DATABASE_FILE_NAME))
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert!(matches!(
        OperationJournal::open(&fresh.directory, fresh.identity.clone(), fresh.root),
        Err(JournalError::UnsupportedSchema(99))
    ));
}

#[test]
fn bounded_payloads_are_rejected_before_any_intent_is_written() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let large = fixture.spec(
        fixture.root,
        "bounded-payload",
        OperationKind::ToolInvocation,
        EffectClass::ReadOnly,
        json!({"data": "x".repeat(1_100_000)}),
        vec![],
    );
    assert!(matches!(
        journal.persist_intent(&large, &initial(&large, owner(1))),
        Err(JournalError::OversizedRecord { .. })
    ));
    assert!(journal.snapshot().unwrap().operations.is_empty());
}

#[test]
fn outbox_delivery_and_evidence_are_deduplicated_and_current() {
    let fixture = Fixture::new();
    let journal = fixture.journal(fixture.root);
    let execution_owner = owner(1);
    let spec = fixture.spec(
        fixture.root,
        "outbox-currentness",
        OperationKind::FileWrite,
        EffectClass::IdempotentMutation,
        json!({"path": "published.txt"}),
        vec![resource("workspace/published.txt")],
    );
    let attempt = effect_started(&journal, &spec, execution_owner);
    let payload = json!({"published": true});
    journal
        .transition(
            execution_owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::CompleteSuccess {
                result: ResultRef::new(PayloadDigest::of_json(&payload).unwrap()),
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(3),
            },
            Some(payload.clone()),
            vec![OutboxDraft::new("property-sink", payload.clone()).unwrap()],
        )
        .unwrap();
    let first = journal.pending_outbox(8).unwrap();
    let second = journal.pending_outbox(8).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 1);
    journal
        .acknowledge_outbox(first[0].id, Timestamp::from_unix_millis(4))
        .unwrap();
    assert!(journal.pending_outbox(8).unwrap().is_empty());

    let unknown = RecoveryEngine::reduce(&recovery_input(
        spec.clone(),
        vec![],
        RecoveryPhase::AfterExternalRequestBeforeResponse,
        RecoveryObservations {
            remote: RemoteObservation::Unknown,
            ..RecoveryObservations::default()
        },
    ));
    let applied = RecoveryEngine::reduce(&recovery_input(
        spec,
        vec![],
        RecoveryPhase::AfterExternalRequestBeforeResponse,
        RecoveryObservations {
            remote: RemoteObservation::Applied,
            ..RecoveryObservations::default()
        },
    ));
    assert_ne!(
        unknown.input_evidence_digests,
        applied.input_evidence_digests
    );
}

#[test]
fn bounded_mutation_fuzz_never_panics_parser_or_reducer() {
    let fixture = Fixture::new();
    let spec = fixture.spec(
        fixture.root,
        "bounded-fuzz",
        OperationKind::CustomExternalAction,
        EffectClass::ExternalMutation,
        json!({"value": "fixture"}),
        vec![],
    );
    let input = recovery_input(
        spec,
        vec![],
        RecoveryPhase::AfterExternalRequestBeforeResponse,
        RecoveryObservations {
            remote: RemoteObservation::Unknown,
            ..RecoveryObservations::default()
        },
    );
    let baseline = serde_json::to_vec(&input).unwrap();
    for mut seed in [11_u64, 29, 47, 65_537] {
        for round in 0..64_u64 {
            let mut bytes = baseline.clone();
            let index = (next_seed(&mut seed) as usize) % bytes.len();
            bytes[index] ^= 1_u8 << (round as u8 % 8);
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                if let Ok(parsed) = serde_json::from_slice::<RecoveryInput>(&bytes) {
                    let _ = RecoveryEngine::reduce(&parsed);
                }
            }));
            assert_property(
                seed,
                &[format!("mutate round={round} index={index}")],
                outcome.is_ok(),
            );
        }
    }
}
