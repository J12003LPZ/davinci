//! Offline admission and durable reconciliation; no provider calls.
use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
use std::sync::{Arc, Barrier};

fn limits() -> BudgetLimits {
    BudgetLimits {
        max_requests: 2,
        max_output_tokens: Some(100),
        max_cost_microusd: None,
        codex_subscription: None,
        deadline_unix_ms: u64::MAX,
    }
}

fn subscription_event() -> davinci_ai::provider_observation::ProviderAttemptObservation {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"kind":"attempt_start","logical_request_id":"request",
        "root_id":"root","actor_id":"actor","attempt_id":1,"purpose":"coding",
        "model":"openai-codex/gpt-6-luna","selected_effort":"high","schema_hash":"fixture",
        "transport":"http","elapsed_ms":0.0,"duration_ms":null,"status":"completed",
        "http_status":200,"usage":null,"returned_model":"gpt-6-luna"
    }))
    .unwrap()
}

fn subscription_limits() -> BudgetLimits {
    serde_json::from_value(serde_json::json!({"max_requests":1,
        "max_output_tokens":null,"max_cost_microusd":null,"deadline_unix_ms":u64::MAX,
        "codex_subscription":{"model":"gpt-6-luna","effort":"high"}
    }))
    .unwrap()
}

#[test]
fn subscription_counts_requests_without_claiming_unknown_output_or_cost() {
    use davinci_ai::provider_observation::AttemptBudget;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.json");
    let budget = RootBudget::open(path.clone(), "root", subscription_limits()).unwrap();
    let event = subscription_event();
    AttemptBudget::reserve(&budget, &event, None).unwrap();
    AttemptBudget::reconcile(&budget, &event).unwrap();
    let child = RootBudget::reopen(path.clone(), "root", subscription_limits()).unwrap();
    let mut next = event;
    next.attempt_id = Some(2);
    assert!(AttemptBudget::reserve(&child, &next, None)
        .unwrap_err()
        .contains("allowance exhausted"));
    assert_eq!(child.snapshot().unwrap().requests, 1);
    let ledger: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(ledger["reservations"]["request:1"]["output"].is_null());
    assert!(ledger["reservations"]["request:1"]["cost_microusd"].is_null());
    assert!(ledger["reservations"]["request:1"]["output_ceiling"].is_null());
}

#[test]
fn subscription_unknown_failed_and_changed_model_receipts_halt_after_reopen() {
    use davinci_ai::provider_observation::AttemptBudget;
    for case in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("budget.json");
        let mut limits = subscription_limits();
        limits.max_requests = 10;
        let budget = RootBudget::open(path.clone(), "root", limits.clone()).unwrap();
        let mut event = subscription_event();
        AttemptBudget::reserve(&budget, &event, None).unwrap();
        match case {
            0 => event.status = "failed".into(),
            1 => event.status = "unknown".into(),
            2 => event.returned_model = None,
            3 => event.returned_model = Some("other".into()),
            _ => event.status = "cancelled".into(),
        }
        assert!(AttemptBudget::reconcile(&budget, &event).is_err());
        let child = RootBudget::reopen(path, "root", limits).unwrap();
        assert!(child.snapshot().unwrap().halted);
        assert!(child.release_unsent("request:1").is_err());
        assert_eq!(child.snapshot().unwrap().unknown, 1);
        event.attempt_id = Some(2);
        assert!(AttemptBudget::reserve(&child, &event, None).is_err());
        assert_eq!(child.snapshot().unwrap().requests, 1);
    }
}

#[test]
fn subscription_unsent_release_and_cold_recovery_do_not_fabricate_settlement() {
    use davinci_ai::provider_observation::AttemptBudget;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.json");
    let budget = RootBudget::open(path.clone(), "root", subscription_limits()).unwrap();
    let event = subscription_event();
    AttemptBudget::reserve(&budget, &event, None).unwrap();
    budget.release_unsent("request:1").unwrap();
    assert_eq!(budget.snapshot().unwrap().requests, 0);
    let mut sent = event;
    sent.attempt_id = Some(2);
    AttemptBudget::reserve(&budget, &sent, None).unwrap();
    let recovered = RootBudget::reopen(path, "root", subscription_limits()).unwrap();
    recovered.recover_pending().unwrap();
    assert_eq!(recovered.snapshot().unwrap().requests, 1);
    assert_eq!(recovered.snapshot().unwrap().unknown, 1);
    assert!(recovered.release_unsent("request:2").is_err());
    sent.attempt_id = Some(3);
    assert!(AttemptBudget::reserve(&recovered, &sent, None).is_err());
}

#[test]
fn subscription_baseline_changes_are_denied_before_reservation() {
    use davinci_ai::provider_observation::AttemptBudget;
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(
        directory.path().join("budget.json"),
        "root",
        subscription_limits(),
    )
    .unwrap();
    for case in 0..3 {
        let mut event = subscription_event();
        match case {
            0 => event.model = "openai/gpt-6-luna".into(),
            1 => event.selected_effort = Some("low".into()),
            _ => event.transport = Some("websocket".into()),
        }
        assert!(AttemptBudget::reserve(&budget, &event, None).is_err());
    }
    assert_eq!(budget.snapshot().unwrap().requests, 0);
}

#[test]
fn harness_children_cannot_multiply_parent_budget() {
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(directory.path().join("budget.json"), "root", limits()).unwrap();
    let barrier = Arc::new(Barrier::new(9));
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let budget = budget.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                budget
                    .reserve(&format!("worker-{i}"), &format!("attempt-{i}"), 50, None)
                    .is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .filter(|ok| *ok)
            .count(),
        2
    );
    let snapshot = budget.snapshot().unwrap();
    assert_eq!(snapshot.requests, 2);
    assert_eq!(snapshot.reserved_output_tokens, 100);
}

#[test]
fn harness_reservations_reconcile_idempotently() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.json");
    let budget = RootBudget::open(path.clone(), "root", limits()).unwrap();
    budget.reserve("lead", "first", 80, None).unwrap();
    budget.reconcile("first", Some(20), None).unwrap();
    budget.reconcile("first", Some(20), None).unwrap();
    drop(budget);
    let restored = RootBudget::open(path, "root", limits()).unwrap();
    assert_eq!(restored.snapshot().unwrap().committed_output_tokens, 20);
    restored.reserve("worker", "second", 80, None).unwrap();
    restored.reconcile("second", None, None).unwrap();
    assert_eq!(restored.snapshot().unwrap().reserved_output_tokens, 80);
    assert!(restored.reserve("worker", "third", 1, None).is_err());
    // A delayed receipt settles the same reservation after a restart.
    restored.reconcile("second", Some(10), None).unwrap();
    assert_eq!(restored.snapshot().unwrap().committed_output_tokens, 30);
    assert!(restored.reconcile("second", Some(11), None).is_err());
    assert!(restored.snapshot().unwrap().halted);
}

#[test]
fn harness_unknown_price_blocks_strict_spend_guarantee() {
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(
        directory.path().join("budget.json"),
        "root",
        BudgetLimits {
            max_cost_microusd: Some(100),
            ..limits()
        },
    )
    .unwrap();
    assert!(budget.reserve("lead", "unknown", 10, None).is_err());
    budget
        .reserve("lead", "known-bound", 10, Some(100))
        .unwrap();
    budget.reconcile("known-bound", Some(5), None).unwrap();
    assert!(budget.reserve("lead", "next", 10, Some(1)).is_err());
}

#[test]
fn harness_budget_identity_limits_and_corruption_cannot_reset_allowance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.json");
    RootBudget::open(path.clone(), "root", limits()).unwrap();
    assert!(RootBudget::open(path.clone(), "other", limits()).is_err());
    assert!(RootBudget::open(
        path.clone(),
        "root",
        BudgetLimits {
            max_requests: 3,
            ..limits()
        }
    )
    .is_err());
    std::fs::write(&path, "{torn").unwrap();
    assert!(RootBudget::open(path.clone(), "root", limits()).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "{torn");
}

#[test]
fn harness_inherited_budget_never_recreates_a_deleted_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.json");
    let budget = RootBudget::open(path.clone(), "root", limits()).unwrap();
    budget.reserve("lead", "sent", 10, None).unwrap();
    assert_eq!(
        RootBudget::reopen(path.clone(), "root", limits())
            .unwrap()
            .snapshot()
            .unwrap()
            .requests,
        1
    );
    std::fs::remove_file(&path).unwrap();
    assert!(RootBudget::reopen(path.clone(), "root", limits()).is_err());
    assert!(!path.exists());
}

#[test]
fn harness_budgeted_session_cannot_resume_without_its_root_binding() {
    use davinci_agent::Agent;
    use davinci_session::JsonlSession;
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(directory.path().join("budget.json"), "root", limits()).unwrap();
    budget.reserve("lead", "used", 50, None).unwrap();
    let mut original = Agent::new("fixture");
    original.session = Some(JsonlSession::create(directory.path(), "budgeted", None).unwrap());
    original.bind_root_budget(budget.clone()).unwrap();
    let path = original.session.as_ref().unwrap().path.clone();
    drop(original);
    let mut unbound = Agent::new("fixture");
    assert!(
        unbound
            .load_from_session(JsonlSession::open(&path).unwrap())
            .is_err(),
        "resuming must not silently remove the root allowance"
    );
    drop(unbound);
    let mut resumed = Agent::new("fixture");
    resumed.bind_root_budget(budget.clone()).unwrap();
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(budget.snapshot().unwrap().requests, 1);
}

#[test]
fn harness_runtime_refresh_and_worker_handoff_keep_root_allowance() {
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(directory.path().join("budget.json"), "root", limits()).unwrap();
    budget.reserve("lead", "sent", 50, None).unwrap();
    let mut original = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    original.root_budget = Some(budget);
    for next in [
        RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_session_state_from(&original),
        RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_worker_state_from(&original),
    ] {
        assert_eq!(next.root_budget.unwrap().snapshot().unwrap().requests, 1);
    }
}

#[test]
fn harness_transport_refuses_second_send_and_preserves_partial_usage() {
    use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
    use davinci_ai::{
        live_complete_with, ChatMessage, Model, ModelCost, ResolvedAuth, StreamOptions,
    };
    use std::io::{Read, Write};
    let directory = tempfile::tempdir().unwrap();
    let budget = RootBudget::open(
        directory.path().join("budget.json"),
        "root",
        BudgetLimits {
            max_requests: 1,
            ..limits()
        },
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        // Drain the complete request before closing; an unread body can cause
        // Windows to reset the socket and discard the response.
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 8192];
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let header = std::str::from_utf8(&bytes[..split]).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    break;
                }
            }
        }
        let body = r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":0}}}"#;
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let model = Model {
        id: "fixture".into(),
        name: "fixture".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: Some(format!("http://{address}")),
        reasoning: false,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 1000,
        max_tokens: 50,
        compat: serde_json::Value::Null,
        headers: Default::default(),
        thinking_level_map: Default::default(),
    };
    let auth = ResolvedAuth {
        api_key: Some("fixture".into()),
        headers: Default::default(),
        source: "fixture".into(),
    };
    let mut agent = Agent::new("fixture");
    agent.cwd = directory.path().into();
    agent.auto_compaction = false;
    agent.bind_root_budget(budget.clone()).unwrap();
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.prompt("say done");
    agent
        .run_loop(|_| {
            live_complete_with(
                &model,
                &[ChatMessage::text("user", "done")],
                &auth,
                None,
                &[],
                &StreamOptions {
                    max_tokens: Some(50),
                    max_retries: Some(0),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    server.join().unwrap();
    assert_eq!(agent.stats.provider_attempts, Some(1));
    assert_eq!(agent.stats.usage_unknown_attempts, Some(1)); // absent cache-write remains unknown
    assert_eq!(budget.snapshot().unwrap().committed_output_tokens, 5);
    agent.prompt("again");
    let error = agent
        .run_loop(|_| {
            live_complete_with(
                &model,
                &[],
                &auth,
                None,
                &[],
                &StreamOptions {
                    max_tokens: Some(50),
                    max_retries: Some(0),
                    ..Default::default()
                },
            )
        })
        .unwrap_err();
    assert!(error.contains("allowance exhausted"), "{error}");
    assert_eq!(budget.snapshot().unwrap().requests, 1);
}
