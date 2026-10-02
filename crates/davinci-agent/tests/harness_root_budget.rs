//! Offline admission and durable reconciliation; no provider calls.
use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
use std::sync::{Arc, Barrier};

fn limits() -> BudgetLimits {
    BudgetLimits {
        max_requests: 2,
        max_output_tokens: 100,
        max_cost_microusd: None,
        deadline_unix_ms: u64::MAX,
    }
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
