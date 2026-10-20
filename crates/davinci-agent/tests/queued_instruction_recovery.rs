//! A message the user was told is queued survives a restart, lands in exactly
//! one final state, and never reaches a worker that already finished.
//! Everything goes through the real runtime log file, as production does.

use davinci_agent::runtime::session::RuntimeLogSubscriber;
use davinci_agent::runtime::{
    AgentId, AgentKind, AgentMailbox, AgentRecord, AgentState, MailboxError, RunId, RuntimeBus,
    RuntimeEventEnvelope, RuntimeRegistry,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

fn record(id: AgentId, run_id: RunId) -> AgentRecord {
    AgentRecord {
        id,
        run_id,
        parent: None,
        kind: AgentKind::Subagent,
        name: "w".into(),
        provider: "mock".into(),
        model_id: "mock".into(),
        cwd: PathBuf::from("/t"),
        state: AgentState::Starting,
        task_id: None,
        worktree: None,
        started_ms: 1,
        updated_ms: 1,
        failure_reason: None,
    }
}

/// One process: a registry with one running worker, a mailbox wired to a log.
struct Process {
    registry: RuntimeRegistry,
    mailbox: AgentMailbox,
    agent: AgentId,
}

fn process(log: &Path, agent: AgentId, run: RunId, state: AgentState) -> Process {
    let registry = RuntimeRegistry::new();
    registry.register_agent(record(agent, run)).unwrap();
    registry.transition(agent, AgentState::Running).unwrap();
    if state != AgentState::Running {
        registry.transition(agent, state).unwrap();
    }
    let bus = RuntimeBus::new();
    bus.subscribe(Arc::new(RuntimeLogSubscriber::open(log).unwrap()));
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    Process {
        registry,
        mailbox,
        agent,
    }
}

fn events(log: &Path) -> Vec<RuntimeEventEnvelope> {
    davinci_session::read_runtime_log::<RuntimeEventEnvelope>(log).unwrap_or_default()
}

/// Start again from the log alone: a new registry, a new mailbox.
fn restart(log: &Path, agent: AgentId, run: RunId, state: AgentState) -> Process {
    let registry = RuntimeRegistry::new();
    registry.register_agent(record(agent, run)).unwrap();
    registry.transition(agent, AgentState::Running).unwrap();
    if state != AgentState::Running {
        registry.transition(agent, state).unwrap();
    }
    let mailbox = AgentMailbox::new().with_registry(registry.clone());
    mailbox.rehydrate_from_events(&events(log));
    Process {
        registry,
        mailbox,
        agent,
    }
}

fn steer(process: &Process, id: Uuid, text: &str) -> String {
    let generation = process.registry.get_generation(&process.agent);
    process
        .mailbox
        .send_steer_with_id(id, process.agent, generation, text.into(), false)
        .unwrap()
        .state
}

fn fixture() -> (tempfile::TempDir, PathBuf, AgentId, RunId) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("session.runtime.jsonl");
    (dir, log, AgentId::new(), RunId::new())
}

#[test]
fn acceptance_is_written_before_queued_ack() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    assert_eq!(steer(&live, id, "use the v2 schema"), "queued");
    // The ack has returned; the log already holds the message, text included.
    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains(&id.to_string()), "{written}");
    assert!(written.contains("use the v2 schema"), "{written}");
}

#[test]
fn acknowledged_followup_survives_restart() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "use the v2 schema");
    drop(live);

    let after = restart(&log, agent, run, AgentState::Running);
    assert_eq!(after.mailbox.pending_count(&agent), 1);
    assert_eq!(
        after.mailbox.get_steering_receipt(&id).unwrap().state,
        "queued"
    );
    // The same id sent again (a retry after the lost answer) queues nothing new.
    assert_eq!(steer(&after, id, "use the v2 schema"), "queued");
    assert_eq!(after.mailbox.pending_count(&agent), 1);
    // Replaying the same log twice does not duplicate it either.
    after.mailbox.rehydrate_from_events(&events(&log));
    assert_eq!(after.mailbox.pending_count(&agent), 1);
    let taken = after.mailbox.drain(agent, 10);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].content, "use the v2 schema");
}

#[test]
fn crash_between_delivery_and_apply_never_double_applies() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "stop using unwrap");
    assert_eq!(live.mailbox.drain(agent, 10).len(), 1);
    // Process dies here: delivered, never marked applied.
    drop(live);

    let after = restart(&log, agent, run, AgentState::Running);
    assert_eq!(after.mailbox.pending_count(&agent), 0, "must not re-queue");
    let receipt = after.mailbox.get_steering_receipt(&id).unwrap();
    assert_eq!(receipt.state, "unknown");
    assert_eq!(receipt.reason.as_deref(), Some("restart_before_apply"));
    assert!(!after.mailbox.mark_applied(&agent, 1, &id));
    assert!(after.mailbox.drain(agent, 10).is_empty());
}

#[test]
fn an_applied_message_stays_applied_after_restart() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "rename the helper");
    live.mailbox.drain(agent, 10);
    assert!(live.mailbox.mark_applied(&agent, 1, &id));
    drop(live);

    let after = restart(&log, agent, run, AgentState::Running);
    assert_eq!(
        after.mailbox.get_steering_receipt(&id).unwrap().state,
        "applied"
    );
    assert_eq!(after.mailbox.pending_count(&agent), 0);
    assert!(!after.mailbox.mark_applied(&agent, 1, &id), "applies once");
}

#[test]
fn a_rejection_is_remembered_across_restart() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "never delivered");
    live.mailbox
        .reject_undelivered_for_cancelled(&agent, "worker_stopped");
    drop(live);

    let after = restart(&log, agent, run, AgentState::Running);
    let receipt = after.mailbox.get_steering_receipt(&id).unwrap();
    assert_eq!(receipt.state, "rejected");
    assert_eq!(receipt.reason.as_deref(), Some("worker_stopped"));
    assert_eq!(after.mailbox.pending_count(&agent), 0);
}

#[test]
fn queued_for_a_worker_that_finished_is_rejected_on_restart() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "too late");
    drop(live);

    let after = restart(&log, agent, run, AgentState::Completed);
    let receipt = after.mailbox.get_steering_receipt(&id).unwrap();
    assert_eq!(receipt.state, "rejected");
    assert_eq!(
        receipt.reason.as_deref(),
        Some("recipient_terminal_after_restart")
    );
    assert_eq!(after.mailbox.pending_count(&agent), 0);
    assert_eq!(
        after.registry.get(&agent).unwrap().state,
        AgentState::Completed
    );
}

#[test]
fn late_message_to_completed_one_shot_is_rejected_once() {
    let (_dir, log, agent, run) = fixture();
    let done = process(&log, agent, run, AgentState::Completed);
    let id = Uuid::new_v4();
    assert_eq!(steer(&done, id, "are you there"), "rejected");
    // Asking again with the same id gives the same visible answer and queues
    // nothing, and the worker is not brought back.
    assert_eq!(steer(&done, id, "are you there"), "rejected");
    assert_eq!(done.mailbox.pending_count(&agent), 0);
    assert_eq!(
        done.registry.get(&agent).unwrap().state,
        AgentState::Completed
    );
    let receipt = done.mailbox.get_steering_receipt(&id).unwrap();
    assert_eq!(receipt.reason.as_deref(), Some("recipient_terminal"));
}

#[test]
fn different_session_never_consumes_queue() {
    let (_dir, log_a, agent, run) = fixture();
    let other = tempfile::tempdir().unwrap();
    let log_b = other.path().join("other.runtime.jsonl");
    let a = process(&log_a, agent, run, AgentState::Running);
    let b = process(&log_b, agent, run, AgentState::Running);
    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();
    steer(&a, id_a, "for session a");
    steer(&b, id_b, "for session b");
    drop((a, b));

    let after = restart(&log_a, agent, run, AgentState::Running);
    let taken = after.mailbox.drain(agent, 10);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].content, "for session a");
    assert!(after.mailbox.get_steering_receipt(&id_b).is_none());
}

/// A log that cannot be written: the disk is full, the file is read-only.
struct FailingLog(std::sync::atomic::AtomicBool);

impl davinci_agent::runtime::RuntimeSubscriber for FailingLog {
    fn on_event(&self, _event: &RuntimeEventEnvelope) -> davinci_agent::runtime::RuntimeDecision {
        davinci_agent::runtime::RuntimeDecision::Continue
    }

    fn record_durably(&self, _event: &RuntimeEventEnvelope) -> Result<(), String> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err("runtime log append failed: no space left on device".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_message_the_log_could_not_keep_is_not_queued() {
    let agent = AgentId::new();
    let run = RunId::new();
    let registry = RuntimeRegistry::new();
    registry.register_agent(record(agent, run)).unwrap();
    registry.transition(agent, AgentState::Running).unwrap();
    let bus = RuntimeBus::new();
    let log = Arc::new(FailingLog(std::sync::atomic::AtomicBool::new(true)));
    bus.subscribe_session(log.clone());
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    let id = Uuid::new_v4();
    let generation = registry.get_generation(&agent);

    let error = mailbox
        .send_steer_with_id(id, agent, generation, "do not lose me".into(), false)
        .unwrap_err();
    assert!(
        matches!(error, MailboxError::NotRecorded(ref reason) if reason.contains("no space")),
        "{error:?}"
    );
    assert_eq!(
        mailbox.pending_count(&agent),
        0,
        "the queue was rolled back"
    );
    assert!(
        mailbox.get_steering_receipt(&id).is_none(),
        "no receipt claims it"
    );

    // The disk recovers; the same id can be sent again and is queued once.
    log.0.store(false, std::sync::atomic::Ordering::SeqCst);
    let receipt = mailbox
        .send_steer_with_id(id, agent, generation, "do not lose me".into(), false)
        .unwrap();
    assert_eq!(receipt.state, "queued");
    assert_eq!(mailbox.pending_count(&agent), 1);
}

#[test]
fn outcome_records_are_versioned_so_an_older_build_stops_with_a_version_error() {
    let (_dir, log, agent, run) = fixture();
    let live = process(&log, agent, run, AgentState::Running);
    let id = Uuid::new_v4();
    steer(&live, id, "version me");
    live.mailbox.drain(agent, 10);
    assert!(live.mailbox.mark_applied(&agent, 1, &id));

    // What a build that only knows version 1 would see line by line.
    let versions: Vec<(String, u64)> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            (
                value["payload"]["kind"].as_str().unwrap().to_string(),
                value["schema_version"].as_u64().unwrap(),
            )
        })
        .collect();
    let version_of = |kind: &str| versions.iter().find(|(k, _)| k == kind).map(|(_, v)| *v);
    assert_eq!(version_of("agent_message_queued"), Some(1));
    assert_eq!(version_of("agent_message_delivered"), Some(1));
    assert_eq!(version_of("agent_message_applied"), Some(2));
    // This build reads all of it.
    assert_eq!(events(&log).len(), versions.len());
}
