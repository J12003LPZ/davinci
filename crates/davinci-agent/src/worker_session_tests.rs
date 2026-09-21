use super::*;

#[test]
fn worker_session_load_preserves_parent_task_authority() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "worker", None).unwrap();
    let path = session.path.clone();
    let parent = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    let task = parent
        .task_registry
        .create_task(TaskRecord::new(parent.run_id, "parent-owned"))
        .unwrap();
    let child_id = AgentId::new();
    let make_worker = || {
        RuntimeHandle::new(parent.run_id, child_id, RuntimeBus::new())
            .with_parent(parent.agent_id)
            .with_task_registry(parent.task_registry.clone())
    };
    let mut agent = Agent::new("worker").with_runtime(make_worker());
    agent.load_from_session(session).unwrap();
    let runtime = agent.runtime_for_session().unwrap();
    assert_eq!(runtime.parent_agent_id, Some(parent.agent_id));
    assert_eq!(runtime.run_id, parent.run_id);
    assert_eq!(runtime.agent_id, child_id);
    assert_eq!(
        runtime.task_registry.get_task(&task),
        parent.task_registry.get_task(&task)
    );
    assert!(!path.with_extension("tasks.jsonl").exists());
    runtime.emit_session_started(false);
    runtime
        .emit_decision(RuntimeEvent::UserPromptSubmitted)
        .unwrap();
    agent.prompt("continue this worker");
    agent.record_assistant("retained reply");
    let before = agent
        .runtime_for_session()
        .unwrap()
        .conversation
        .lock()
        .unwrap()
        .snapshot();
    let mut competing = Agent::new("worker").with_runtime(make_worker());
    assert!(competing
        .load_from_session(JsonlSession::open(&path).unwrap())
        .is_err());
    assert!(competing.session.is_none());
    agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(
        agent
            .runtime_for_session()
            .unwrap()
            .conversation
            .lock()
            .unwrap()
            .snapshot(),
        before
    );
    drop(agent);
    let mut resumed = Agent::new("worker").with_runtime(make_worker());
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(
        resumed
            .runtime_for_session()
            .unwrap()
            .conversation
            .lock()
            .unwrap()
            .snapshot(),
        before
    );
    assert_eq!(
        resumed.last_assistant_text().as_deref(),
        Some("retained reply")
    );
    assert!(!path.with_extension("tasks.jsonl").exists());
    resumed
        .runtime_for_session()
        .unwrap()
        .emit_session_started(true);
    let events = davinci_session::read_runtime_log::<RuntimeEventEnvelope>(
        &davinci_session::runtime_log_path(&path),
    )
    .unwrap();
    assert_eq!(
        events.len(),
        3,
        "reloading must not duplicate journal subscribers"
    );
    assert!(events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert_eq!(
        parent.conversation.lock().unwrap().snapshot().last_sequence,
        0
    );
}

#[test]
fn worker_session_rejects_foreign_lineage_and_session_switch() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "worker", None).unwrap();
    let path = session.path.clone();
    let run = RunId::new();
    let child = AgentId::new();
    let parent = AgentId::new();
    let make_worker =
        |run, child, parent| RuntimeHandle::new(run, child, RuntimeBus::new()).with_parent(parent);
    let mut agent = Agent::new("worker").with_runtime(make_worker(run, child, parent));
    agent.load_from_session(session).unwrap();
    agent.prompt("original");
    let other = JsonlSession::create(dir.path(), "other", None).unwrap();
    assert!(agent.load_from_session(other).is_err());
    assert_eq!(agent.session.as_ref().unwrap().path, path);
    drop(agent);
    let mut root = Agent::new("ordinary session");
    assert!(root
        .load_from_session(JsonlSession::open(&path).unwrap())
        .is_err());
    assert!(!path.with_extension("tasks.jsonl").exists());
    for (run, child, parent) in [
        (RunId::new(), child, parent),
        (run, AgentId::new(), parent),
        (run, child, AgentId::new()),
    ] {
        let mut wrong = Agent::new("worker").with_runtime(make_worker(run, child, parent));
        assert!(wrong
            .load_from_session(JsonlSession::open(&path).unwrap())
            .is_err());
        assert!(wrong.session.is_none());
    }
}

#[test]
fn worker_session_ownership_survives_until_process_exit() {
    use std::io::{BufRead, Read, Write};
    use std::process::{Command, Stdio};
    const FIXTURE: &str = "DAVINCI_WORKER_SESSION_LEASE_FIXTURE";
    let load = |path: &std::path::Path, run, child, parent| {
        let mut agent = Agent::new("worker")
            .with_runtime(RuntimeHandle::new(run, child, RuntimeBus::new()).with_parent(parent));
        agent
            .load_from_session(JsonlSession::open(path).unwrap())
            .map(|()| agent)
    };
    if let Ok(raw) = std::env::var(FIXTURE) {
        let (path, run, child, parent): (PathBuf, RunId, AgentId, AgentId) =
            serde_json::from_str(&raw).unwrap();
        let _agent = load(&path, run, child, parent).unwrap();
        println!("WORKER_SESSION_READY");
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_exact(&mut [0]);
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "worker", None).unwrap();
    let (run, child, parent) = (RunId::new(), AgentId::new(), AgentId::new());
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "worker_session_tests::worker_session_ownership_survives_until_process_exit",
            "--nocapture",
        ])
        .env(
            FIXTURE,
            serde_json::to_string(&(&session.path, run, child, parent)).unwrap(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(process.stdout.take().unwrap());
    let mut line = String::new();
    let ready = loop {
        if output.read_line(&mut line).unwrap() == 0 {
            break false;
        }
        if line.contains("WORKER_SESSION_READY") {
            break true;
        }
        line.clear();
    };
    let refused = ready && load(&session.path, run, child, parent).is_err();
    let _ = process.kill();
    process.wait().unwrap();
    assert!(ready, "worker exited before acquiring its conversation");
    assert!(
        refused,
        "a live worker must retain exclusive conversation ownership"
    );
    assert!(load(&session.path, run, child, parent).is_ok());
}

#[test]
fn failed_worker_session_activation_releases_its_writer() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "worker", None).unwrap();
    let path = session.path.clone();
    let ledger = path.with_extension("tool-ledger.json");
    std::fs::write(&ledger, "{").unwrap();
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_parent(AgentId::new());
    let mut agent = Agent::new("worker").with_runtime(runtime);
    assert!(agent.load_from_session(session).is_err());
    assert!(agent.session.is_none());
    std::fs::remove_file(ledger).unwrap();
    assert!(agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .is_ok());
}
