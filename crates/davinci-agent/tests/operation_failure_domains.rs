use davinci_agent::runtime::operations::{
    AllowedRecoveryAction, CausalFailureReport, DurableResultStatus, EffectCertainty,
    FailureReasonCode, FailureSubsystem, OperationFailure, PublicationStatus, VerificationStatus,
};
use davinci_agent::runtime::{
    AgentId, RunId, RuntimeBus, RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope, RuntimeHandle,
    RuntimeSubscriber,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

struct Collector(Arc<Mutex<Vec<RuntimeEventEnvelope>>>);

impl RuntimeSubscriber for Collector {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        self.0.lock().unwrap().push(event.clone());
        RuntimeDecision::Continue
    }
}

struct SlowDenyingObserver {
    calls: Arc<AtomicUsize>,
}

impl RuntimeSubscriber for SlowDenyingObserver {
    fn on_event(&self, _event: &RuntimeEventEnvelope) -> RuntimeDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(250));
        RuntimeDecision::Deny {
            reason: "observer must never own execution authority".into(),
        }
    }
}

struct PanickingObserver;

impl RuntimeSubscriber for PanickingObserver {
    fn on_event(&self, _event: &RuntimeEventEnvelope) -> RuntimeDecision {
        panic!("observer disconnected/panicked");
    }
}

fn envelope(payload: RuntimeEvent) -> RuntimeEventEnvelope {
    RuntimeEventEnvelope::new(1, RunId::new(), None, Some(AgentId::new()), None, payload)
}

#[test]
fn causal_report_keeps_domains_and_recovery_action_distinct() {
    let report = CausalFailureReport::new(
        FailureSubsystem::Verification,
        FailureReasonCode::VerificationFailed,
        "verifier rejected token=sk-secret9999 after write succeeded",
        123,
    )
    .with_identity(
        Some("op-1".into()),
        Some("attempt-1".into()),
        Some("root-1".into()),
        Some("run-1".into()),
        Some("session-1".into()),
        Some("agent-1".into()),
        Some("worker-1".into()),
    )
    .with_transition(
        Some("parent-op".into()),
        Some(2),
        Some(9),
        Some("effect_possible".into()),
        Some("recovery_required".into()),
    )
    .with_artifact_refs(vec!["receipt-1".into(), "log://safe".into()])
    .with_outcome(
        EffectCertainty::KnownEffect,
        DurableResultStatus::Committed,
        VerificationStatus::Failed,
        PublicationStatus::Published,
        AllowedRecoveryAction::AuthenticatedHumanDecision,
    );
    assert_eq!(report.failed_subsystem(), FailureSubsystem::Verification);
    assert!(!report.message.contains("sk-secret9999"));
    assert_eq!(report.operation_id.as_deref(), Some("op-1"));
    assert_eq!(
        report.allowed_recovery,
        AllowedRecoveryAction::AuthenticatedHumanDecision
    );
    assert!(!report.is_recoverable_without_replay());
    let wire = serde_json::to_string(&report).unwrap();
    assert!(!wire.contains("sk-secret9999"));
    assert!(wire.contains("recovery_required"));
}

#[test]
fn failure_chain_preserves_provider_and_observation_cause() {
    let provider = OperationFailure::new(FailureReasonCode::Timeout, FailureSubsystem::Execution);
    let observer = OperationFailure::new(
        FailureReasonCode::PublicationFailed,
        FailureSubsystem::Publication,
    )
    .caused_by(provider.clone());
    assert_eq!(observer.cause.as_deref(), Some(&provider));
    let wire = serde_json::to_value(observer).unwrap();
    assert_eq!(wire["subsystem"], json!("publication"));
    assert_eq!(wire["cause"]["subsystem"], json!("execution"));
}

#[test]
fn noncritical_observer_is_bounded_and_cannot_block_or_deny_execution() {
    let bus = RuntimeBus::new();
    let calls = Arc::new(AtomicUsize::new(0));
    bus.subscribe_noncritical(Arc::new(SlowDenyingObserver {
        calls: calls.clone(),
    }));
    let started = Instant::now();
    for sequence in 1..=80 {
        bus.emit_decision(RuntimeEventEnvelope::new(
            sequence,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::PreToolUse {
                call_id: format!("call-{sequence}"),
                tool: "remote_write".into(),
                args: json!({"mutation": true}),
            },
        ))
        .unwrap();
    }
    assert!(started.elapsed() < Duration::from_millis(150));
    assert!(calls.load(Ordering::SeqCst) <= 80);
}

#[test]
fn observer_panic_does_not_erase_durable_runtime_facts() {
    let bus = RuntimeBus::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    bus.subscribe_noncritical(Arc::new(PanickingObserver));
    bus.subscribe(Arc::new(Collector(events.clone())));
    bus.emit_observe(envelope(RuntimeEvent::RuntimeWarning {
        code: "provider_failure".into(),
        message: "provider failed after tool success".into(),
    }));
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[test]
fn runtime_handle_emits_redacted_causal_projection() {
    let bus = RuntimeBus::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    bus.subscribe(Arc::new(Collector(events.clone())));
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), bus);
    runtime.emit_causal_failure(&CausalFailureReport::new(
        FailureSubsystem::Persistence,
        FailureReasonCode::Persistence,
        "result write failed secret=sk-protected",
        99,
    ));
    let recorded = events.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    match &recorded[0].payload {
        RuntimeEvent::RuntimeWarning { code, message } => {
            assert_eq!(code, "causal_failure");
            assert!(!message.contains("sk-protected"));
            assert!(message.contains("persistence"));
        }
        other => panic!("expected causal warning, got {other:?}"),
    }
}

#[test]
fn failed_child_report_does_not_claim_successful_sibling_failed() {
    let failed = CausalFailureReport::new(
        FailureSubsystem::Execution,
        FailureReasonCode::UnknownEffect,
        "child A response was lost",
        10,
    )
    .with_identity(
        Some("child-a".into()),
        None,
        Some("root".into()),
        None,
        None,
        None,
        None,
    )
    .with_outcome(
        EffectCertainty::Unknown,
        DurableResultStatus::Unknown,
        VerificationStatus::Unknown,
        PublicationStatus::Failed,
        AllowedRecoveryAction::ReconcileBeforeReplay,
    );
    let sibling = CausalFailureReport::new(
        FailureSubsystem::Verification,
        FailureReasonCode::Other,
        "child B completed",
        11,
    )
    .with_identity(
        Some("child-b".into()),
        None,
        Some("root".into()),
        None,
        None,
        None,
        None,
    )
    .with_outcome(
        EffectCertainty::KnownEffect,
        DurableResultStatus::Committed,
        VerificationStatus::Verified,
        PublicationStatus::Published,
        AllowedRecoveryAction::None,
    );
    assert_eq!(
        failed.allowed_recovery,
        AllowedRecoveryAction::ReconcileBeforeReplay
    );
    assert_eq!(sibling.durable_result, DurableResultStatus::Committed);
    assert_ne!(failed.operation_id, sibling.operation_id);
}
