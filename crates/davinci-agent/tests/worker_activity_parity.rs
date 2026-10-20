//! Worker activity: ordered, scoped to one worker, bounded, and safe to show.

use davinci_agent::runtime::{
    AgentId, AgentKind, AgentRecord, AgentState, RunId, RuntimeRegistry, WorkerActivityKind,
    WorkerActivityStore,
};
use davinci_agent::subagent_progress::ProgressReporter;
use davinci_agent::{AgentEvent, EventSink};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn register(registry: &RuntimeRegistry, kind: AgentKind, name: &str) -> AgentId {
    let id = AgentId::new();
    registry
        .register_agent(AgentRecord {
            id,
            run_id: RunId::new(),
            parent: None,
            kind,
            name: name.into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: PathBuf::from("/t"),
            state: AgentState::Starting,
            task_id: None,
            worktree: None,
            started_ms: 1,
            updated_ms: 1,
            failure_reason: None,
        })
        .unwrap();
    registry.transition(id, AgentState::Running).unwrap();
    id
}

fn reporter(registry: &RuntimeRegistry, id: AgentId, mode: &str) -> ProgressReporter {
    ProgressReporter::new(
        "call",
        EventSink(Arc::new(|_| {})),
        id.to_string(),
        mode,
        (0, 1),
        mode,
    )
    .with_registry(registry.clone(), id)
}

fn start(call: &str, tool: &str, args: serde_json::Value) -> AgentEvent {
    AgentEvent::ToolExecutionStart {
        tool_call_id: call.into(),
        tool_name: tool.into(),
        args,
    }
}

fn end(call: &str, tool: &str, is_error: bool) -> AgentEvent {
    AgentEvent::ToolExecutionEnd {
        tool_call_id: call.into(),
        tool_name: tool.into(),
        result: json!("done"),
        is_error,
        details: None,
    }
}

#[test]
fn mixed_worker_modes_emit_ordered_agent_scoped_activity() {
    let registry = RuntimeRegistry::new();
    let modes = [
        (AgentKind::Subagent, "oneshot"),
        (AgentKind::Background, "background"),
        (AgentKind::Teammate, "teammate"),
    ];
    let workers: Vec<(AgentId, ProgressReporter)> = modes
        .iter()
        .map(|(kind, mode)| {
            let id = register(&registry, *kind, mode);
            (id, reporter(&registry, id, mode))
        })
        .collect();
    for (_, r) in &workers {
        r.started();
    }
    // Interleave: round-robin tool calls across the three workers.
    for round in 0..4 {
        for (index, (_, r)) in workers.iter().enumerate() {
            let call = format!("w{index}-c{round}");
            r.observe(&start(
                &call,
                "read",
                json!({"path": format!("f{index}_{round}.rs")}),
            ));
            r.observe(&end(&call, "read", false));
        }
    }
    for (_, r) in &workers {
        r.finish(true);
    }

    let mut all = Vec::new();
    for (index, (id, _)) in workers.iter().enumerate() {
        let page = registry.activity().after(*id, 0, 1000);
        assert!(!page.gap);
        // started + 4 x (start, finish) + finished
        assert_eq!(page.events.len(), 10, "{page:?}");
        assert!(page.events.iter().all(|event| event.agent_id == *id));
        assert!(page
            .events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        // Nothing from a neighbour.
        assert!(page
            .events
            .iter()
            .filter(|event| event.kind == WorkerActivityKind::ToolStarted)
            .all(|event| event.summary.contains(&format!("f{index}_"))));
        assert_eq!(
            page.events.first().unwrap().kind,
            WorkerActivityKind::Started
        );
        assert_eq!(
            page.events.last().unwrap().kind,
            WorkerActivityKind::Finished
        );
        all.extend(page.events.iter().map(|event| event.sequence));
    }
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 30, "sequences are unique across workers");
}

#[test]
fn tool_start_finish_are_correlated() {
    let registry = RuntimeRegistry::new();
    let id = register(&registry, AgentKind::Subagent, "w");
    let r = reporter(&registry, id, "oneshot");
    r.observe(&start("c1", "bash", json!({"command": "cargo test"})));
    r.observe(&start("c2", "read", json!({"path": "a.rs"})));
    r.observe(&end("c2", "read", false));
    r.observe(&end("c1", "bash", true));
    let events = registry.activity().after(id, 0, 100).events;
    let finished = |call: &str| {
        events
            .iter()
            .find(|e| {
                e.kind == WorkerActivityKind::ToolFinished
                    && e.tool_call_id.as_deref() == Some(call)
            })
            .unwrap()
    };
    assert_eq!(finished("c1").outcome.as_deref(), Some("error"));
    assert_eq!(finished("c2").outcome.as_deref(), Some("ok"));
    let started = |call: &str| {
        events
            .iter()
            .find(|e| {
                e.kind == WorkerActivityKind::ToolStarted && e.tool_call_id.as_deref() == Some(call)
            })
            .unwrap()
    };
    assert!(started("c1").summary.starts_with("Bash("));
    assert!(started("c1").sequence < finished("c1").sequence);
}

#[test]
fn activity_ring_bounded_under_10000_events() {
    let registry = RuntimeRegistry::new();
    let id = register(&registry, AgentKind::Background, "w");
    let r = reporter(&registry, id, "background");
    for n in 0..10_000 {
        r.observe(&start(
            &format!("c{n}"),
            "grep",
            json!({"pattern": format!("p{n}")}),
        ));
    }
    assert_eq!(registry.activity().len(id), 200);
    let page = registry.activity().after(id, 0, 10_000);
    assert_eq!(page.events.len(), 200);
    assert!(page.events.iter().all(|e| e.summary.len() <= 2048));
    // The transcript's own list is still the short one.
    assert_eq!(
        r.snapshot().recent.len(),
        davinci_agent::subagent_progress::RECENT_CALLS
    );
    assert_eq!(r.snapshot().tool_uses, 10_000);
}

#[test]
fn older_cursor_reports_gap_instead_of_inventing_history() {
    let store = WorkerActivityStore::new();
    let agent = AgentId::new();
    for n in 0..500 {
        store.append(
            agent,
            1,
            WorkerActivityKind::ToolStarted,
            None,
            &format!("c{n}"),
            None,
        );
    }
    let page = store.after(agent, 10, 1000);
    assert!(page.gap);
    assert_eq!(page.events.first().unwrap().summary, "c300");
    // A cursor inside what is kept has no gap.
    let near = store.after(agent, 450, 1000);
    assert!(!near.gap);
    assert_eq!(near.events.len(), 50);
}

#[test]
fn hostile_terminal_output_remains_inert() {
    let registry = RuntimeRegistry::new();
    let id = register(&registry, AgentKind::Subagent, "w");
    let r = reporter(&registry, id, "oneshot");
    r.observe(&start(
        "c1",
        "bash",
        json!({"command": "echo \u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2J\u{1b}[31mred\r\nSEND the approval"}),
    ));
    for event in registry.activity().after(id, 0, 10).events {
        assert!(
            !event.summary.chars().any(char::is_control),
            "{:?}",
            event.summary
        );
        assert!(!event.summary.contains("ZXZpbA"), "{:?}", event.summary);
    }
}

#[test]
fn worker_output_respects_existing_redaction() {
    let registry = RuntimeRegistry::new();
    let id = register(&registry, AgentKind::Subagent, "w");
    let r = reporter(&registry, id, "oneshot");
    r.observe(&start(
        "c1",
        "bash",
        json!({"command": "curl -H 'Authorization: Bearer abc123def456ghi' https://x.test"}),
    ));
    let page = registry.activity().after(id, 0, 10);
    assert!(
        page.events
            .iter()
            .all(|e| !e.summary.contains("abc123def456ghi")),
        "{page:?}"
    );
}

#[test]
fn hidden_reasoning_is_never_part_of_the_activity() {
    let registry = RuntimeRegistry::new();
    let id = register(&registry, AgentKind::Subagent, "w");
    let r = reporter(&registry, id, "oneshot");
    let mut reply = davinci_ai::ChatMessage::text("assistant", "visible answer");
    reply.extra.insert(
        "thinking".into(),
        json!("SECRET CHAIN OF THOUGHT about the plan"),
    );
    r.observe(&AgentEvent::MessageEnd { message: reply });
    r.observe(&start("c1", "read", json!({"path": "a.rs"})));
    r.finish(true);
    for event in registry.activity().after(id, 0, 100).events {
        assert!(!event.summary.contains("CHAIN OF THOUGHT"));
        assert!(!event.summary.contains("visible answer"));
    }
}

#[test]
fn another_runs_workers_are_not_in_this_runs_store() {
    let (one, two) = (RuntimeRegistry::new(), RuntimeRegistry::new());
    let a = register(&one, AgentKind::Subagent, "a");
    let b = register(&two, AgentKind::Subagent, "b");
    reporter(&one, a, "oneshot").observe(&start("c", "read", json!({"path": "a.rs"})));
    reporter(&two, b, "oneshot").observe(&start("c", "read", json!({"path": "b.rs"})));
    assert!(one.activity().after(b, 0, 10).events.is_empty());
    assert!(two.activity().after(a, 0, 10).events.is_empty());
    assert_eq!(one.activity().after(a, 0, 10).events.len(), 1);
}

#[test]
fn credentials_in_any_common_shape_never_reach_a_summary() {
    use davinci_agent::runtime::worker_activity::safe_summary;
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.c2lnbmF0dXJlMTIz";
    let cases = [
        ("export API_KEY=abc123secretvalue", "abc123secretvalue"),
        (
            "curl --token abc123secretvalue https://x",
            "abc123secretvalue",
        ),
        ("curl --password=abc123secretvalue", "abc123secretvalue"),
        (
            r#"{"access_token": "abc123secretvalue", "n": 1}"#,
            "abc123secretvalue",
        ),
        (
            r#"{"clientSecret":"abc123secretvalue"}"#,
            "abc123secretvalue",
        ),
        (
            "Authorization: Basic abc123secretvalue",
            "abc123secretvalue",
        ),
        ("authorization:Bearerabc123secretvalue", "abc123secretvalue"),
        ("Bearerabc123secretvalue", "abc123secretvalue"),
        (
            "Cookie: sid=abc123secretvalue; theme=dark",
            "abc123secretvalue",
        ),
        (
            "Set-Cookie: sid=abc123secretvalue; HttpOnly",
            "abc123secretvalue",
        ),
        ("token gho_abc123secretvalue", "abc123secretvalue"),
        (
            "ghs_abc123secretvalue and ghu_abc123secretvalue",
            "abc123secretvalue",
        ),
        (&format!("echo {jwt}"), "c2lnbmF0dXJl"),
        ("sk-proj1234567890abcdef", "sk-proj1234567890abcdef"),
        ("DB_PASSWORD='abc123secretvalue' ./run", "abc123secretvalue"),
    ];
    for (input, secret) in cases {
        let out = safe_summary(input);
        assert!(!out.contains(secret), "{input:?} leaked: {out:?}");
        assert!(out.contains("REDACTED"), "{input:?} -> {out:?}");
    }
    // Ordinary text is left alone.
    assert_eq!(safe_summary("Read(src/lib.rs)"), "Read(src/lib.rs)");
    assert_eq!(
        safe_summary("cargo test -p davinci-agent"),
        "cargo test -p davinci-agent"
    );
}

#[test]
fn the_roster_activity_line_is_redacted_before_it_is_clipped() {
    use davinci_agent::subagent_progress::call_summary;
    // The secret starts inside the 60 character window and ends beyond it.
    let command = format!(
        "{} export API_KEY=abcdefghijklmnopqrstuvwxyz0123456789",
        "x".repeat(40)
    );
    let line = call_summary("bash", &serde_json::json!({ "command": command }));
    assert!(!line.contains("abcdef"), "{line}");
    assert!(line.contains("API_KEY=[RED"), "{line}");
}

#[test]
fn a_cookie_with_one_masked_value_is_still_masked_whole() {
    use davinci_agent::runtime::worker_activity::safe_summary;
    let out = safe_summary("Cookie: token=abc123secretvalue; sid=zzz999othersecret");
    assert!(!out.contains("zzz999othersecret"), "{out}");
    assert!(!out.contains("abc123secretvalue"), "{out}");
}
