#[path = "support/operation_harness.rs"]
mod operation_harness;

use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::EvidenceId;
use operation_harness::{
    commit_projection_once, increment_endpoint, probe_endpoint, projection_count, wait_for_marker,
    ChildConfig, ChildCrashInjector, SerializedIdentity, CHILD_CONFIG_ENV,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::Read;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const RECOVERY_CONFIG_ENV: &str = "DAVINCI_OPERATION_RECOVERY_CONFIG";

fn child_process() {
    let Some(config_path) = std::env::var_os(CHILD_CONFIG_ENV) else {
        return;
    };
    let config: ChildConfig = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    let mut faults = ChildCrashInjector {
        selected: config.fault,
        marker: config.marker_path.clone(),
    };
    let identity = config.identity.clone().into_journal_identity();
    let journal = OperationJournal::open(&config.journal_directory, identity, config.root).unwrap();
    let initial = OperationAttempt::new(config.spec.operation_id(), 1, config.owner).unwrap();
    faults.checkpoint(FaultPoint::BeforeIntentCommit);
    let mut attempt = match journal.admit(&config.spec, &initial).unwrap() {
        OperationAdmission::New(admitted) => admitted.attempt,
        outcome => panic!("child expected new intent, got {outcome:?}"),
    };
    faults.checkpoint(FaultPoint::AfterIntentCommit);
    attempt = journal
        .transition(
            config.owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                config.spec.intent_digest().unwrap(),
                "crash-harness-policy".to_owned(),
                Timestamp::from_unix_millis(1),
            )),
            None,
            vec![],
        )
        .unwrap();
    attempt = journal
        .transition(
            config.owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();
    let claim = journal
        .claim_dispatch(config.owner, attempt.attempt_id(), attempt.revision())
        .unwrap();
    faults.checkpoint(FaultPoint::AfterAttemptClaim);
    let permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(2))
        .unwrap();
    faults.checkpoint(FaultPoint::AfterEffectLatch);
    attempt = journal.load_attempt(attempt.attempt_id()).unwrap();
    permit
        .dispatch(&journal, || {
            increment_endpoint(&config.endpoint_path, config.spec.operation_id()).unwrap();
            faults.checkpoint(FaultPoint::AfterSideEffect);
        })
        .unwrap();
    let payload = json!({"mutations": 1, "operation_id": config.spec.operation_id()});
    let event = OperationEvent::CompleteSuccess {
        result: ResultRef::new(PayloadDigest::of_json(&payload).unwrap()),
        effect_status: EffectStatus::EffectsObserved,
        finished_at: Timestamp::from_unix_millis(3),
    };
    let outbox = vec![OutboxDraft::new("crash-test-sink", payload.clone()).unwrap()];
    faults.checkpoint(FaultPoint::BeforeResultCommit);
    journal
        .transition(
            config.owner,
            attempt.attempt_id(),
            attempt.revision(),
            event,
            Some(payload),
            outbox,
        )
        .unwrap();
    faults.checkpoint(FaultPoint::AfterResultCommit);
    faults.checkpoint(FaultPoint::BeforePublication);
    for item in journal.pending_outbox(1).unwrap() {
        commit_projection_once(&config.sink_path, &item.id.to_string(), &item.payload).unwrap();
        faults.checkpoint(FaultPoint::AfterSinkCommit);
        faults.checkpoint(FaultPoint::BeforeOutboxAcknowledgement);
        journal
            .acknowledge_outbox(item.id, Timestamp::from_unix_millis(4))
            .unwrap();
    }
}

#[test]
fn operation_crash_child() {
    let Some(config_path) = std::env::var_os(CHILD_CONFIG_ENV) else {
        return;
    };
    let config: ChildConfig = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    let panic_path = config.marker_path.with_extension("panic");
    if let Err(payload) = catch_unwind(AssertUnwindSafe(child_process)) {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_owned())
            })
            .unwrap_or_else(|| "child panicked with a non-string payload".to_owned());
        let _ = fs::write(panic_path, message);
        resume_unwind(payload);
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct CrashCase {
    point: FaultPoint,
    phase: RecoveryPhase,
    effect_happened: bool,
    result_committed: bool,
    sink_committed: bool,
    expected_action: RecoveryAction,
}

#[derive(Serialize, Deserialize)]
struct RecoveryConfig {
    child: ChildConfig,
    case: CrashCase,
}

const CASES: [CrashCase; 10] = [
    CrashCase {
        point: FaultPoint::BeforeIntentCommit,
        phase: RecoveryPhase::BeforeIntentCommit,
        effect_happened: false,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::AdmitIntent,
    },
    CrashCase {
        point: FaultPoint::AfterIntentCommit,
        phase: RecoveryPhase::AfterIntentBeforeAuthorization,
        effect_happened: false,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::Authorize,
    },
    CrashCase {
        point: FaultPoint::AfterAttemptClaim,
        phase: RecoveryPhase::AfterClaimBeforeUnsafeBoundary,
        effect_happened: false,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::StartSafeRetry,
    },
    CrashCase {
        point: FaultPoint::AfterEffectLatch,
        phase: RecoveryPhase::AfterEffectLatchBeforeSyscall,
        effect_happened: false,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::ProbePostcondition,
    },
    CrashCase {
        point: FaultPoint::AfterSideEffect,
        phase: RecoveryPhase::AfterExternalRequestBeforeResponse,
        effect_happened: true,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::ReconcileDomain,
    },
    CrashCase {
        point: FaultPoint::BeforeResultCommit,
        phase: RecoveryPhase::AfterExternalRequestBeforeResponse,
        effect_happened: true,
        result_committed: false,
        sink_committed: false,
        expected_action: RecoveryAction::ReconcileDomain,
    },
    CrashCase {
        point: FaultPoint::AfterResultCommit,
        phase: RecoveryPhase::AfterResultBeforeNotification,
        effect_happened: true,
        result_committed: true,
        sink_committed: false,
        expected_action: RecoveryAction::ReplayPublication,
    },
    CrashCase {
        point: FaultPoint::BeforePublication,
        phase: RecoveryPhase::AfterResultBeforeNotification,
        effect_happened: true,
        result_committed: true,
        sink_committed: false,
        expected_action: RecoveryAction::ReplayPublication,
    },
    CrashCase {
        point: FaultPoint::AfterSinkCommit,
        phase: RecoveryPhase::AfterResultBeforeNotification,
        effect_happened: true,
        result_committed: true,
        sink_committed: true,
        expected_action: RecoveryAction::ReplayPublication,
    },
    CrashCase {
        point: FaultPoint::BeforeOutboxAcknowledgement,
        phase: RecoveryPhase::AfterResultBeforeNotification,
        effect_happened: true,
        result_committed: true,
        sink_committed: true,
        expected_action: RecoveryAction::ReplayPublication,
    },
];

fn stop_child_at(temp: &TempDir, config: &ChildConfig) -> Child {
    let config_path = temp.path().join("child-config.json");
    fs::write(&config_path, serde_json::to_vec(config).unwrap()).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new(executable)
        .args(["--exact", "operation_crash_child", "--show-output"])
        .env(CHILD_CONFIG_ENV, config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_marker(&mut child, &config.marker_path, Duration::from_secs(20));
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "fault-injected child unexpectedly exited normally"
    );
    child
}

fn observations(case: CrashCase) -> RecoveryObservations {
    match case.phase {
        RecoveryPhase::BeforeIntentCommit => RecoveryObservations::default(),
        RecoveryPhase::AfterIntentBeforeAuthorization => RecoveryObservations {
            authorization: AuthorizationObservation::Current,
            ..RecoveryObservations::default()
        },
        RecoveryPhase::AfterClaimBeforeUnsafeBoundary => RecoveryObservations {
            owner: OwnerObservation::Quiescent,
            authorization: AuthorizationObservation::Current,
            dispatch: DispatchObservation::ClaimedUnstarted,
            ..RecoveryObservations::default()
        },
        RecoveryPhase::AfterEffectLatchBeforeSyscall
        | RecoveryPhase::AfterExternalRequestBeforeResponse => RecoveryObservations {
            owner: OwnerObservation::Quiescent,
            authorization: AuthorizationObservation::Current,
            remote: RemoteObservation::Unknown,
            dispatch: DispatchObservation::EffectLatched,
            ..RecoveryObservations::default()
        },
        RecoveryPhase::AfterResultBeforeNotification => RecoveryObservations {
            publication: PublicationObservation::Pending,
            ..RecoveryObservations::default()
        },
        _ => unreachable!(),
    }
}

fn recovery_process(config: &RecoveryConfig) {
    let child = &config.child;
    let case = config.case;
    let identity = child.identity.clone().into_journal_identity();
    let journal = OperationJournal::open(&child.journal_directory, identity, child.root).unwrap();
    let before_replay = journal.snapshot().unwrap();
    assert_eq!(
        before_replay.operations.len(),
        usize::from(case.point != FaultPoint::BeforeIntentCommit)
    );
    assert_eq!(
        probe_endpoint(&child.endpoint_path, child.spec.operation_id()).unwrap(),
        u64::from(case.effect_happened),
        "{}",
        case.point as u8
    );
    assert_eq!(
        before_replay.results.len(),
        usize::from(case.result_committed)
    );
    assert_eq!(
        projection_count(&child.sink_path).unwrap(),
        u64::from(case.sink_committed)
    );

    if case.point == FaultPoint::BeforeIntentCommit {
        let result = RecoveryEngine::run_authorized::<()>(
            &journal,
            child.owner,
            RecoveryInput {
                spec: child.spec.clone(),
                attempts: vec![],
                phase: case.phase,
                observations: observations(case),
                policy_version: "recovery-policy-v1".to_owned(),
            },
            None,
            EvidenceId::new(),
            |_| Ok::<(), String>(()),
            |decision| {
                assert_eq!(decision.action, case.expected_action);
                assert!(journal
                    .latest_recovery_decision(child.spec.operation_id())
                    .unwrap()
                    .is_some());
                Ok::<(), String>(())
            },
            |decision, retry_attempt| {
                assert_eq!(decision.action, case.expected_action);
                assert!(retry_attempt.is_none());
                Ok::<(), String>(())
            },
        )
        .unwrap();
        assert_eq!(result.decision.action, case.expected_action);
        assert_eq!(journal.snapshot().unwrap().attempts.len(), 1);
        return;
    }

    let replay_attempt = OperationAttempt::new(child.spec.operation_id(), 1, child.owner).unwrap();
    let admission = journal.admit(&child.spec, &replay_attempt).unwrap();
    if case.result_committed {
        assert!(matches!(admission, OperationAdmission::ExistingResult(_)));
    } else {
        assert!(matches!(admission, OperationAdmission::ExistingInFlight(_)));
    }
    assert_eq!(journal.snapshot().unwrap().attempts.len(), 1);

    let input = RecoveryInput {
        spec: child.spec.clone(),
        attempts: journal.snapshot().unwrap().attempts,
        phase: case.phase,
        observations: observations(case),
        policy_version: "recovery-policy-v1".to_owned(),
    };
    let replacement_owner = (case.expected_action == RecoveryAction::StartSafeRetry)
        .then(|| ExecutionOwner::new(ExecutionOwnerId::new(), 2).unwrap());
    let result = RecoveryEngine::run_authorized(
        &journal,
        child.owner,
        input,
        replacement_owner,
        EvidenceId::new(),
        |old_owner| {
            assert_eq!(old_owner, child.owner);
            Ok::<(), String>(())
        },
        |_| Ok::<(), String>(()),
        |decision, retry_attempt| {
            assert_eq!(decision.action, case.expected_action);
            if decision.action == RecoveryAction::StartSafeRetry {
                assert_eq!(retry_attempt.map(OperationAttempt::attempt_number), Some(2));
            }
            if decision.action == RecoveryAction::ReplayPublication {
                for item in journal
                    .pending_outbox(1)
                    .map_err(|error| error.to_string())?
                {
                    commit_projection_once(&child.sink_path, &item.id.to_string(), &item.payload)
                        .map_err(|error| error.to_string())?;
                    journal
                        .acknowledge_outbox(item.id, Timestamp::from_unix_millis(5))
                        .map_err(|error| error.to_string())?;
                }
            }
            Ok::<(), String>(())
        },
    )
    .unwrap();
    assert_eq!(result.decision.action, case.expected_action);
    assert_eq!(
        journal
            .latest_recovery_decision(child.spec.operation_id())
            .unwrap(),
        Some(result.decision)
    );
    assert_eq!(
        probe_endpoint(&child.endpoint_path, child.spec.operation_id()).unwrap(),
        u64::from(case.effect_happened),
        "recovery replayed the side effect at {:?}",
        case.point
    );
    assert_eq!(
        projection_count(&child.sink_path).unwrap(),
        u64::from(case.result_committed)
    );
    if case.result_committed {
        assert_eq!(journal.pending_outbox(1).unwrap().len(), 0);
    }
}

#[test]
fn operation_crash_recovery_child() {
    let Some(config_path) = std::env::var_os(RECOVERY_CONFIG_ENV) else {
        return;
    };
    let config: RecoveryConfig = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    let panic_path = config.child.marker_path.with_extension("recovery-panic");
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| recovery_process(&config))) {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_owned())
            })
            .unwrap_or_else(|| "recovery child panicked with a non-string payload".to_owned());
        let _ = fs::write(panic_path, message);
        resume_unwind(payload);
    }
}

fn child_output(child: &mut Child, status: std::process::ExitStatus) -> Output {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(output) = child.stdout.as_mut() {
        output.read_to_end(&mut stdout).unwrap();
    }
    if let Some(output) = child.stderr.as_mut() {
        output.read_to_end(&mut stderr).unwrap();
    }
    Output {
        status,
        stdout,
        stderr,
    }
}

fn run_recovery_child(temp: &TempDir, config: &RecoveryConfig) {
    let config_path = temp.path().join("recovery-config.json");
    fs::write(&config_path, serde_json::to_vec(config).unwrap()).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new(executable)
        .args(["--exact", "operation_crash_recovery_child", "--show-output"])
        .env(RECOVERY_CONFIG_ENV, config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child_output(&mut child, status);
            assert!(
                output.status.success(),
                "fresh recovery process failed; panic: {}; stdout: {}; stderr: {}",
                fs::read_to_string(config.child.marker_path.with_extension("recovery-panic"))
                    .unwrap_or_default(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let status = child.wait().unwrap();
            let output = child_output(&mut child, status);
            panic!(
                "fresh recovery process timed out; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn child_process_crashes_recover_without_duplicate_mutations_or_projections() {
    for (index, case) in CASES.into_iter().enumerate() {
        let temp = tempfile::tempdir().unwrap();
        // macOS exposes the default temporary root through /var -> /private/var.
        // The production journal intentionally rejects symlinked ancestors, so
        // make the test fixture use the canonical temporary root instead of
        // weakening the journal's no-follow policy.
        let temp_root = fs::canonicalize(temp.path()).unwrap();
        let identity = SerializedIdentity {
            journal_id: JournalId::new(),
            workspace: WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        };
        let root = RootNamespaceId::new();
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
        let spec = operation_harness::standard_spec(&identity, root, &format!("crash-{index}"));
        let config = ChildConfig {
            journal_directory: temp_root.join("journal"),
            endpoint_path: temp_root.join("endpoint.sqlite3"),
            sink_path: temp_root.join("sink.sqlite3"),
            marker_path: temp_root.join("fault-reached"),
            identity,
            root,
            owner,
            spec,
            fault: case.point,
        };
        let _terminated_child = stop_child_at(&temp, &config);
        run_recovery_child(
            &temp,
            &RecoveryConfig {
                child: config.clone(),
                case,
            },
        );
        assert_eq!(
            probe_endpoint(&config.endpoint_path, config.spec.operation_id()).unwrap(),
            u64::from(case.effect_happened),
            "fresh recovery process repeated the side effect at {:?}",
            case.point
        );
        assert_eq!(
            projection_count(&config.sink_path).unwrap(),
            u64::from(case.result_committed)
        );
    }
}
