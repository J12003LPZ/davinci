use super::*;

fn observations_for(
    phase: RecoveryPhase,
) -> (RecoveryObservations, RecoveryClassification, RecoveryAction) {
    use RecoveryAction as A;
    use RecoveryClassification as C;
    use RecoveryObservations as O;
    use RecoveryPhase as P;

    match phase {
        P::BeforeIntentCommit => (O::default(), C::SafeToRetry, A::AdmitIntent),
        P::AfterIntentBeforeAuthorization => (
            O {
                authorization: AuthorizationObservation::Current,
                ..O::default()
            },
            C::SafeToRetry,
            A::Authorize,
        ),
        P::AfterAuthorizationBeforeClaim => (
            O {
                authorization: AuthorizationObservation::Current,
                ..O::default()
            },
            C::SafeToRetry,
            A::ClaimDispatch,
        ),
        P::AfterClaimBeforeUnsafeBoundary => (
            O {
                owner: OwnerObservation::Quiescent,
                authorization: AuthorizationObservation::Current,
                dispatch: DispatchObservation::ClaimedUnstarted,
                ..O::default()
            },
            C::SafeToRetry,
            A::StartSafeRetry,
        ),
        P::AfterEffectLatchBeforeSyscall => (
            O {
                owner: OwnerObservation::Quiescent,
                authorization: AuthorizationObservation::Current,
                dispatch: DispatchObservation::EffectLatched,
                ..O::default()
            },
            C::NeedsVerification,
            A::ProbePostcondition,
        ),
        P::AfterProcessSpawnBeforeReceipt => (
            O {
                owner: OwnerObservation::Unknown,
                process: ProcessObservation::MissingReceipt,
                ..O::default()
            },
            C::NeedsReconciliation,
            A::ReconcileDomain,
        ),
        P::AfterFileEffectBeforeResult => (
            O {
                postcondition: PostconditionObservation::Satisfied,
                ..O::default()
            },
            C::AlreadyCompleted,
            A::RecordObservedOutcome,
        ),
        P::DuringMultiFileApplyOrRollback => (
            O {
                transaction: TransactionObservation::InProgress,
                ..O::default()
            },
            C::NeedsReconciliation,
            A::ReconcileDomain,
        ),
        P::AfterExternalRequestBeforeResponse => (
            O {
                remote: RemoteObservation::Unknown,
                ..O::default()
            },
            C::NeedsReconciliation,
            A::ReconcileDomain,
        ),
        P::AfterResultBeforeNotification => (
            O {
                publication: PublicationObservation::Pending,
                ..O::default()
            },
            C::AlreadyCompleted,
            A::ReplayPublication,
        ),
        P::DuringSessionAppend => (
            O {
                session: SessionProjectionObservation::TornTail,
                ..O::default()
            },
            C::NeedsReconciliation,
            A::RepairProjection,
        ),
        P::DuringVerification => (
            O {
                verification: VerificationObservation::Incomplete,
                probe_authorized: true,
                ..O::default()
            },
            C::NeedsVerification,
            A::RunVerification,
        ),
        P::DuringGraphWorkerCompletion => (
            O {
                worker: WorkerObservation::ValidBoundResult,
                ..O::default()
            },
            C::AlreadyCompleted,
            A::ReplayPublication,
        ),
        P::DuringCancellationOrTermination => (
            O {
                owner: OwnerObservation::Unknown,
                process: ProcessObservation::Live,
                ..O::default()
            },
            C::NeedsHumanDecision,
            A::ReconcileTermination,
        ),
        P::DuringSchemaMigration => (
            O {
                migration: MigrationObservation::SourceValid,
                ..O::default()
            },
            C::SafeToRetry,
            A::ResumeMigration,
        ),
        P::DuringJournalWriteOrCorruption => (
            O {
                journal: JournalIntegrityObservation::Corrupt,
                ..O::default()
            },
            C::TerminalFailure,
            A::Stop,
        ),
    }
}

#[test]
fn reducer_classifies_every_documented_crash_boundary_from_typed_facts() {
    let fixture = Fixture::new();
    let root = RootNamespaceId::new();
    let spec = fixture.spec(
        root,
        "matrix",
        OperationKind::CustomExternalAction,
        EffectClass::ExternalMutation,
        vec![],
    );
    let owner = owner(1);
    let phases = [
        RecoveryPhase::BeforeIntentCommit,
        RecoveryPhase::AfterIntentBeforeAuthorization,
        RecoveryPhase::AfterAuthorizationBeforeClaim,
        RecoveryPhase::AfterClaimBeforeUnsafeBoundary,
        RecoveryPhase::AfterEffectLatchBeforeSyscall,
        RecoveryPhase::AfterProcessSpawnBeforeReceipt,
        RecoveryPhase::AfterFileEffectBeforeResult,
        RecoveryPhase::DuringMultiFileApplyOrRollback,
        RecoveryPhase::AfterExternalRequestBeforeResponse,
        RecoveryPhase::AfterResultBeforeNotification,
        RecoveryPhase::DuringSessionAppend,
        RecoveryPhase::DuringVerification,
        RecoveryPhase::DuringGraphWorkerCompletion,
        RecoveryPhase::DuringCancellationOrTermination,
        RecoveryPhase::DuringSchemaMigration,
        RecoveryPhase::DuringJournalWriteOrCorruption,
    ];

    for phase in phases {
        let (observations, expected_classification, expected_action) = observations_for(phase);
        let input = RecoveryInput {
            spec: spec.clone(),
            attempts: recovery_attempt(&spec, owner, phase),
            phase,
            observations,
            policy_version: "recovery-policy-v1".to_owned(),
        };
        let decision = RecoveryEngine::reduce(&input);
        assert_eq!(
            decision.classification, expected_classification,
            "{phase:?}"
        );
        assert_eq!(decision.action, expected_action, "{phase:?}");
        assert_eq!(decision.operation_id, spec.operation_id());
        assert_eq!(decision.policy_version, "recovery-policy-v1");
        assert!(!decision.input_evidence_digests.is_empty(), "{phase:?}");
    }
}

#[test]
fn ambiguous_external_effect_never_becomes_a_retry() {
    let fixture = Fixture::new();
    let spec = fixture.spec(
        RootNamespaceId::new(),
        "ambiguous-external",
        OperationKind::McpCall,
        EffectClass::ExternalMutation,
        vec![],
    );
    let input = RecoveryInput {
        attempts: recovery_attempt(
            &spec,
            owner(1),
            RecoveryPhase::AfterExternalRequestBeforeResponse,
        ),
        spec,
        phase: RecoveryPhase::AfterExternalRequestBeforeResponse,
        observations: RecoveryObservations {
            remote: RemoteObservation::Unknown,
            owner: OwnerObservation::Quiescent,
            authorization: AuthorizationObservation::Current,
            ..RecoveryObservations::default()
        },
        policy_version: "recovery-policy-v1".to_owned(),
    };

    let decision = RecoveryEngine::reduce(&input);
    assert_ne!(decision.classification, RecoveryClassification::SafeToRetry);
    assert_ne!(decision.action, RecoveryAction::StartSafeRetry);
}

#[test]
fn effect_claims_block_conflicting_mutations_across_roots_but_allow_disjoint_and_read_only_work() {
    let fixture = Fixture::new();
    let first_root = RootNamespaceId::new();
    let second_root = RootNamespaceId::new();
    let first_journal = fixture.journal(first_root);
    let second_journal = fixture.journal(second_root);
    let first = fixture.spec(
        first_root,
        "write-a",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/lib.rs")],
    );
    let first_owner = owner(1);
    let first_queued = queue_attempt(&first_journal, &first, first_owner);

    let conflicting = fixture.spec(
        second_root,
        "edit-a",
        OperationKind::FileEdit,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/lib.rs")],
    );
    let conflicting_queued = queue_attempt(&second_journal, &conflicting, owner(1));
    let first_claim = first_journal
        .claim_dispatch(
            first_owner,
            first_queued.attempt_id(),
            first_queued.revision(),
        )
        .unwrap();
    drop(
        first_journal
            .latch_effect_start(first_claim, Timestamp::from_unix_millis(2))
            .unwrap(),
    );
    assert!(matches!(
        second_journal.claim_dispatch(
            conflicting_queued.owner(),
            conflicting_queued.attempt_id(),
            conflicting_queued.revision()
        ),
        Err(JournalError::UnresolvedEffectConflict { .. })
    ));

    let another_conflicting = fixture.spec(
        second_root,
        "edit-b",
        OperationKind::FileEdit,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/lib.rs")],
    );
    let attempt = OperationAttempt::new(another_conflicting.operation_id(), 1, owner(1)).unwrap();
    assert!(matches!(
        second_journal.admit(&another_conflicting, &attempt),
        Err(JournalError::UnresolvedEffectConflict { .. })
    ));

    let disjoint = fixture.spec(
        second_root,
        "write-b",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/other.rs")],
    );
    assert!(matches!(
        second_journal
            .admit(
                &disjoint,
                &OperationAttempt::new(disjoint.operation_id(), 1, owner(1)).unwrap()
            )
            .unwrap(),
        OperationAdmission::New(_)
    ));

    let read_only = fixture.spec(
        second_root,
        "read-a",
        OperationKind::ToolInvocation,
        EffectClass::ReadOnly,
        vec![resource("workspace/src/lib.rs")],
    );
    assert!(matches!(
        second_journal
            .admit(
                &read_only,
                &OperationAttempt::new(read_only.operation_id(), 1, owner(1)).unwrap()
            )
            .unwrap(),
        OperationAdmission::New(_)
    ));

    let completed = first_journal
        .load_attempt(first_queued.attempt_id())
        .unwrap();
    let payload = json!({"done": true});
    first_journal
        .transition(
            first_owner,
            completed.attempt_id(),
            completed.revision(),
            OperationEvent::CompleteSuccess {
                result: ResultRef::new(PayloadDigest::of_json(&payload).unwrap()),
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(3),
            },
            Some(payload),
            vec![],
        )
        .unwrap();
    let after_resolution = fixture.spec(
        second_root,
        "edit-after-resolution",
        OperationKind::FileEdit,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/lib.rs")],
    );
    assert!(matches!(
        second_journal
            .admit(
                &after_resolution,
                &OperationAttempt::new(after_resolution.operation_id(), 1, owner(1)).unwrap()
            )
            .unwrap(),
        OperationAdmission::New(_)
    ));
}

#[test]
fn cancelling_before_start_releases_the_claim_and_invalidates_the_dispatch_permit() {
    let fixture = Fixture::new();
    let root = RootNamespaceId::new();
    let journal = fixture.journal(root);
    let first = fixture.spec(
        root,
        "cancel-before-start",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/cancelled.rs")],
    );
    let owner = owner(1);
    let queued = queue_attempt(&journal, &first, owner);
    let permit = journal
        .claim_dispatch(owner, queued.attempt_id(), queued.revision())
        .unwrap();
    let cancelled = journal
        .transition(
            owner,
            queued.attempt_id(),
            queued.revision(),
            OperationEvent::CancelBeforeStart {
                finished_at: Timestamp::from_unix_millis(2),
            },
            None,
            vec![],
        )
        .unwrap();

    assert_eq!(cancelled.state(), OperationState::Cancelled);
    assert_eq!(cancelled.effect_status(), EffectStatus::NotStarted);
    assert!(matches!(
        journal.latch_effect_start(permit, Timestamp::from_unix_millis(3)),
        Err(JournalError::InvalidDispatchPermit)
    ));

    let replacement = fixture.spec(
        root,
        "replacement-after-cancel",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/src/cancelled.rs")],
    );
    assert!(matches!(
        journal
            .admit(
                &replacement,
                &OperationAttempt::new(replacement.operation_id(), 1, owner).unwrap()
            )
            .unwrap(),
        OperationAdmission::New(_)
    ));
}

#[test]
fn before_intent_recovery_admits_before_persisting_or_scheduling_its_decision() {
    let fixture = Fixture::new();
    let root = RootNamespaceId::new();
    let journal = fixture.journal(root);
    let spec = fixture.spec(
        root,
        "before-intent-runner",
        OperationKind::CustomExternalAction,
        EffectClass::ExternalMutation,
        vec![],
    );
    let owner = owner(1);
    let input = RecoveryInput {
        spec: spec.clone(),
        attempts: vec![],
        phase: RecoveryPhase::BeforeIntentCommit,
        observations: RecoveryObservations::default(),
        policy_version: "recovery-policy-v1".to_owned(),
    };
    let mut scheduled = false;
    let result = RecoveryEngine::run_authorized::<()>(
        &journal,
        owner,
        input,
        None,
        davinci_agent::runtime::EvidenceId::new(),
        |_| Ok::<(), String>(()),
        |decision| {
            assert_eq!(decision.action, RecoveryAction::AdmitIntent);
            assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
            assert_eq!(
                journal
                    .latest_recovery_decision(spec.operation_id())
                    .unwrap(),
                Some(decision.clone())
            );
            Ok::<(), String>(())
        },
        |decision, _| {
            assert_eq!(decision.action, RecoveryAction::AdmitIntent);
            assert_eq!(
                journal
                    .latest_recovery_decision(spec.operation_id())
                    .unwrap(),
                Some(decision.clone())
            );
            scheduled = true;
            Ok(())
        },
    )
    .unwrap();

    assert!(scheduled);
    assert!(result.scheduled);
    assert_eq!(result.decision.action, RecoveryAction::AdmitIntent);
    assert_eq!(journal.snapshot().unwrap().attempts.len(), 1);
}

#[test]
fn recovery_decision_is_durable_before_authorization_and_scheduling() {
    let fixture = Fixture::new();
    let root = RootNamespaceId::new();
    let journal = fixture.journal(root);
    let spec = fixture.spec(
        root,
        "decision-order",
        OperationKind::CustomExternalAction,
        EffectClass::ExternalMutation,
        vec![],
    );
    let owner = owner(1);
    let attempt = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    journal.admit(&spec, &attempt).unwrap();
    let input = RecoveryInput {
        spec: spec.clone(),
        attempts: vec![journal.load_attempt(attempt.attempt_id()).unwrap()],
        phase: RecoveryPhase::AfterIntentBeforeAuthorization,
        observations: RecoveryObservations {
            authorization: AuthorizationObservation::Current,
            ..RecoveryObservations::default()
        },
        policy_version: "recovery-policy-v1".to_owned(),
    };
    let authorized = std::cell::Cell::new(false);
    let scheduled = std::cell::Cell::new(false);
    let result = RecoveryEngine::run_authorized(
        &journal,
        owner,
        input,
        None,
        davinci_agent::runtime::EvidenceId::new(),
        |_| Ok::<(), String>(()),
        |decision| {
            assert_eq!(
                journal
                    .latest_recovery_decision(spec.operation_id())
                    .unwrap(),
                Some(decision.clone())
            );
            authorized.set(true);
            Ok::<(), String>(())
        },
        |decision, _retry_attempt| {
            assert!(authorized.get());
            assert_eq!(decision.action, RecoveryAction::Authorize);
            assert_eq!(
                journal
                    .latest_recovery_decision(spec.operation_id())
                    .unwrap(),
                Some(decision.clone())
            );
            scheduled.set(true);
            Ok::<(), String>(())
        },
    )
    .unwrap();

    assert!(authorized.get());
    assert!(scheduled.get());
    assert_eq!(result.decision.action, RecoveryAction::Authorize);
}

#[test]
fn denied_recovery_action_keeps_decision_but_never_schedules() {
    let fixture = Fixture::new();
    let root = RootNamespaceId::new();
    let journal = fixture.journal(root);
    let spec = fixture.spec(
        root,
        "denied-recovery",
        OperationKind::CustomExternalAction,
        EffectClass::ExternalMutation,
        vec![],
    );
    let owner = owner(1);
    let attempt = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
    journal.admit(&spec, &attempt).unwrap();
    let input = RecoveryInput {
        spec: spec.clone(),
        attempts: vec![journal.load_attempt(attempt.attempt_id()).unwrap()],
        phase: RecoveryPhase::AfterIntentBeforeAuthorization,
        observations: RecoveryObservations {
            authorization: AuthorizationObservation::Current,
            ..RecoveryObservations::default()
        },
        policy_version: "recovery-policy-v1".to_owned(),
    };
    let mut scheduled = false;
    let result = RecoveryEngine::run_authorized::<()>(
        &journal,
        owner,
        input,
        None,
        davinci_agent::runtime::EvidenceId::new(),
        |_| Ok::<(), String>(()),
        |_| Err("permission revoked during recovery".to_owned()),
        |_, _| {
            scheduled = true;
            Ok(())
        },
    );

    assert!(matches!(
        result,
        Err(RecoveryRunError::AuthorizationDenied(_))
    ));
    assert!(!scheduled);
    assert!(journal
        .latest_recovery_decision(spec.operation_id())
        .unwrap()
        .is_some());
}

#[test]
fn opaque_unresolved_effect_claim_blocks_other_roots_in_the_workspace() {
    let fixture = Fixture::new();
    let first_root = RootNamespaceId::new();
    let second_root = RootNamespaceId::new();
    let first_journal = fixture.journal(first_root);
    let second_journal = fixture.journal(second_root);
    let opaque = fixture.spec(
        first_root,
        "opaque-effect",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![
            resource("workspace/src/opaque.rs"),
            Precondition {
                kind: PreconditionKind::Custom,
                resource: "unknown-scope".to_owned(),
                expected_digest: None,
            },
        ],
    );
    begin_effect(&first_journal, &opaque, owner(1));

    let unrelated = fixture.spec(
        second_root,
        "different-wrapper-and-key",
        OperationKind::FileWrite,
        EffectClass::ReversibleMutation,
        vec![resource("workspace/docs/readme.md")],
    );
    assert!(matches!(
        second_journal.admit(
            &unrelated,
            &OperationAttempt::new(unrelated.operation_id(), 1, owner(1)).unwrap()
        ),
        Err(JournalError::UnresolvedEffectConflict { .. })
    ));
}
