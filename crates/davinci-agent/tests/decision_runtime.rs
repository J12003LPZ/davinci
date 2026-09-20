use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use davinci_agent::decision::policy::{
    add_optional_capabilities, DecisionRollout, OptionalCapability,
};
use davinci_agent::decision::provider::{DecisionError, DecisionProvider};
use davinci_agent::decision::request::{DecisionQuestion, DecisionRequest};
use davinci_agent::decision::response::{parse_and_validate_response, DecisionResponse};
use davinci_agent::decision::risk::DecisionRisk;
use davinci_agent::decision::DecisionRuntime;
use serde_json::json;

struct FixtureProvider {
    calls: AtomicUsize,
    entered: Option<Arc<Barrier>>,
    release: Option<Arc<Barrier>>,
    result: Result<Vec<u8>, DecisionError>,
}

impl DecisionProvider for FixtureProvider {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn model(&self) -> &'static str {
        "fixture-model"
    }

    fn evaluate(
        &self,
        request: &DecisionRequest,
        _budget: Duration,
    ) -> Result<DecisionResponse, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = &self.entered {
            entered.wait();
        }
        if let Some(release) = &self.release {
            release.wait();
        }
        parse_and_validate_response(&self.result.clone()?, request)
    }
}

fn request() -> DecisionRequest {
    DecisionRequest::new(
        "integration-request",
        DecisionRisk::Ranking,
        json!({"schemaVersion":1,"task":"redacted"}),
        "fixture-model",
        [(
            "browser_relevant".to_owned(),
            DecisionQuestion::noul("browser relevance"),
        )],
    )
}

fn success_payload() -> Vec<u8> {
    br#"{"answers":{"browser_relevant":{"type":"noul","noul":0.9}}}"#.to_vec()
}

#[test]
fn disabled_runtime_is_a_no_call_safe_default() {
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: None,
        release: None,
        result: Ok(success_payload()),
    });
    let runtime = DecisionRuntime::new(provider.clone());

    assert_eq!(runtime.evaluate(&request()), Err(DecisionError::Disabled));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_credential_is_not_retried_in_the_same_generation() {
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: None,
        release: None,
        result: Err(DecisionError::CredentialInvalid),
    });
    let runtime = DecisionRuntime::new(provider.clone());
    runtime.enable();

    assert_eq!(
        runtime.evaluate(&request()),
        Err(DecisionError::CredentialInvalid)
    );
    assert_eq!(
        runtime.evaluate(&request()),
        Err(DecisionError::CredentialInvalid)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn response_from_an_older_generation_is_discarded() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: Some(Arc::clone(&entered)),
        release: Some(Arc::clone(&release)),
        result: Ok(success_payload()),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider));
    runtime.enable();
    let pending = Arc::clone(&runtime);
    let handle = thread::spawn(move || pending.evaluate(&request()));

    entered.wait();
    runtime.disable();
    release.wait();

    assert_eq!(handle.join().unwrap(), Err(DecisionError::StaleResponse));
    assert_eq!(
        runtime.audit().snapshot()[0].outcome,
        davinci_agent::decision::audit::DecisionAuditOutcome::Stale
    );
}

#[test]
fn guarded_policy_only_adds_available_authorized_non_vetoed_capabilities() {
    let deterministic = vec!["mandatory_verification".to_owned()];
    let optional = vec![
        OptionalCapability::new("browser", 0.90),
        OptionalCapability {
            id: "git".to_owned(),
            relevance: 0.99,
            available: false,
            authorized: true,
            vetoed: false,
        },
        OptionalCapability {
            id: "security".to_owned(),
            relevance: 0.99,
            available: true,
            authorized: true,
            vetoed: true,
        },
    ];

    assert_eq!(
        add_optional_capabilities(&deterministic, &optional, DecisionRollout::GuardedAdditive),
        vec!["mandatory_verification", "browser"]
    );
}

#[test]
fn shadow_submit_returns_while_provider_is_blocked_and_does_not_queue_work() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: Some(entered.clone()),
        release: Some(release.clone()),
        result: Ok(success_payload()),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider.clone()));
    runtime.enable();
    let (tx, rx) = std::sync::mpsc::channel();
    let submit = runtime.clone();
    thread::spawn(move || tx.send(submit.enqueue_shadow(request())).unwrap());
    // A blocking implementation cannot reach this result before release.
    assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_ok());
    entered.wait();
    let generation = runtime.generation();
    assert_eq!(runtime.enqueue_shadow(request()), Err(DecisionError::Busy));
    assert_eq!(runtime.generation(), generation);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    release.wait();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while runtime.audit().snapshot().is_empty() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        runtime.audit().snapshot()[0].outcome,
        davinci_agent::decision::audit::DecisionAuditOutcome::Success
    );
}

#[test]
fn runtime_enforces_hard_deadline_even_when_provider_ignores_it() {
    use davinci_agent::decision::provider::DecisionProviderHealth;
    use davinci_agent::decision::HARD_DECISION_BUDGET;
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: Some(entered.clone()),
        release: Some(release.clone()),
        result: Ok(success_payload()),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider.clone()));
    runtime.enable();
    let pending = runtime.clone();
    let start = std::time::Instant::now();
    let handle = thread::spawn(move || pending.evaluate(&request()));
    entered.wait();
    assert!(matches!(
        handle.join().unwrap(),
        Err(DecisionError::Unavailable(_))
    ));
    assert!(start.elapsed() >= HARD_DECISION_BUDGET);
    assert!(start.elapsed() < HARD_DECISION_BUDGET + Duration::from_secs(2));
    assert_eq!(runtime.telemetry().snapshot().timeouts, 1);
    assert_eq!(runtime.telemetry().snapshot().soft_deadline_misses, 1);
    // The orphaned provider worker remains bounded until it actually finishes.
    runtime.enable();
    assert_eq!(runtime.evaluate(&request()), Err(DecisionError::Busy));
    assert_eq!(runtime.health(), DecisionProviderHealth::Ready);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    release.wait();
}

#[test]
fn shadow_fact_preparation_is_nonblocking_and_covered_by_the_worker_bound() {
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
        entered: None,
        release: None,
        result: Ok(success_payload()),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider.clone()));
    runtime.enable();
    let (release, waiting) = std::sync::mpsc::channel();
    let (submitted, returned) = std::sync::mpsc::channel();
    let job = runtime.clone();
    let submit = thread::spawn(move || {
        submitted
            .send(job.enqueue_shadow_with(move || {
                waiting.recv_timeout(Duration::from_secs(2)).unwrap();
                request()
            }))
            .unwrap();
    });
    assert!(returned
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .is_ok());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.enqueue_shadow(request()), Err(DecisionError::Busy));
    runtime.disable();
    release.send(()).unwrap();
    submit.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while runtime.audit().snapshot().is_empty() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runtime.audit().snapshot()[0].outcome,
        davinci_agent::decision::audit::DecisionAuditOutcome::Stale
    );
}
