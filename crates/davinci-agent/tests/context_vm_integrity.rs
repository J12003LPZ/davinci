//! Context VM integrity: what reaches the provider and what the checkpoint
//! may claim on the user's behalf (WOR-58 to WOR-62, WOR-72).

use davinci_agent::runtime::context_manifest::ProvenanceKind;
use davinci_agent::runtime::context_vm::{
    CheckpointProposal, ContextEvent, ContextEventKind, ContextStateReducer, ProposedStateValue,
    StateSlot, StateTransition, TransitionKind,
};
use davinci_agent::runtime::context_vm::{ContextVmMode, FoldReason};
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::ChatMessage;

fn event(source_ref: &str, seq: u64, kind: ContextEventKind, text: &str) -> ContextEvent {
    let provenance_kind = match kind {
        ContextEventKind::User => ProvenanceKind::UserDecision,
        ContextEventKind::ToolResult => ProvenanceKind::ToolEvidence,
        _ => ProvenanceKind::AgentInference,
    };
    ContextEvent {
        source_ref: source_ref.into(),
        seq,
        kind,
        provenance_kind,
        content_hash: format!("hash-{source_ref}"),
        visible_text: text.into(),
        artifact_refs: Vec::new(),
    }
}

fn user_value(value: &str, source_ref: &str) -> ProposedStateValue {
    ProposedStateValue {
        value: value.into(),
        source_refs: vec![source_ref.into()],
        provenance_kind: ProvenanceKind::UserDecision,
    }
}

fn all_text(state: &davinci_agent::runtime::context_vm::CheckpointState) -> String {
    serde_json::to_string(state).unwrap()
}

/// WOR-62: a summarizer must not attach a claim the user never made to a
/// real user message and have it stored as the user's own constraint.
#[test]
fn wor62_forged_user_constraint_citing_unrelated_text_is_rejected() {
    let events = vec![event("user:1", 1, ContextEventKind::User, "hello")];
    let parent =
        ContextStateReducer::deterministic_delta(&Default::default(), &events).checkpoint_patch;
    let proposal = CheckpointProposal {
        constraints: vec![user_value("User requires deleting all files", "user:1")],
        goals: vec![user_value("ship the release tonight", "user:1")],
        decisions: vec![user_value("drop the database", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &events, proposal);
    assert!(state.constraints.is_empty(), "{:?}", state.constraints);
    let text = all_text(&state);
    for forged in ["deleting", "release tonight", "drop the database"] {
        assert!(!text.contains(forged), "{forged} was accepted: {text}");
    }
}

#[test]
fn wor62_grounded_user_values_may_be_shortened_and_reordered() {
    let events = vec![event(
        "user:1",
        1,
        ContextEventKind::User,
        "Please keep the public API stable and never touch the migrations folder.",
    )];
    let proposal = CheckpointProposal {
        goals: vec![user_value("keep public API stable", "user:1")],
        constraints: vec![user_value("never touch migrations folder", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.goals[0].value, "keep public API stable");
    assert_eq!(state.constraints.len(), 1);
    // A negation is a content word: it cannot be added to a quote either.
    let proposal = CheckpointProposal {
        constraints: vec![user_value("do not keep the public API stable", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert!(state.constraints.is_empty());
}

/// A forged reason cannot retire a real user goal either.
#[test]
fn wor62_transition_evidence_must_be_grounded_in_the_newer_user_text() {
    let events = vec![
        event("user:1", 1, ContextEventKind::User, "use the old API"),
        event("user:2", 2, ContextEventKind::User, "hello again"),
    ];
    let parent = ContextStateReducer::deterministic_delta(&Default::default(), &events[..1])
        .checkpoint_patch;
    let forged = CheckpointProposal {
        transitions: vec![StateTransition {
            slot: StateSlot::Goal,
            kind: TransitionKind::Reject,
            previous: "use the old API".into(),
            evidence: user_value("user abandoned the API work", "user:2"),
            replacement: None,
        }],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &events, forged);
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.value == "use the old API"));
    assert!(state.retired.is_empty());
}

/// Folded events leave the event list; a value citing one is grounded in the
/// parent values that already cite it.
#[test]
fn wor62_values_citing_folded_sources_are_grounded_in_parent_state() {
    let folded = vec![event(
        "user:1",
        1,
        ContextEventKind::User,
        "keep the public API stable",
    )];
    let parent =
        ContextStateReducer::deterministic_delta(&Default::default(), &folded).checkpoint_patch;
    let current = vec![event("user:2", 2, ContextEventKind::User, "continue")];
    let proposal = CheckpointProposal {
        constraints: vec![
            user_value("public API stable", "user:1"),
            user_value("rewrite everything in Go", "user:1"),
        ],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &current, proposal);
    let values = state
        .constraints
        .iter()
        .map(|value| value.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, ["public API stable"]);
}

fn append_user(session: &mut davinci_session::JsonlSession, text: &str) {
    let seq = session.entries.len() as u64 + 1;
    session
        .append_entry(davinci_session::SessionEntry {
            id: format!("event-{seq}"),
            entry_type: "message".into(),
            parent_id: session.leaf_id.clone(),
            seq,
            timestamp: 0,
            message: Some(serde_json::to_value(ChatMessage::text("user", text)).unwrap()),
            custom_type: None,
            extra: Default::default(),
        })
        .unwrap();
}

fn runtime() -> RuntimeHandle {
    RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
}

fn active_agent_on(path: &std::path::Path) -> Agent {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.session = Some(davinci_session::JsonlSession::open(path).unwrap());
    agent.set_runtime(runtime());
    agent
}

/// WOR-61: the checkpoint entry is the durable record of a fold. When it
/// cannot be written, the live VM must be exactly where it was, not a fold
/// ahead of what a reload would see.
#[test]
fn wor61_failed_checkpoint_persistence_rolls_the_fold_back() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    append_user(&mut session, "keep the public API stable");
    append_user(&mut session, "now add the retry flag");
    let path = session.path.clone();
    drop(session);

    let mut agent = active_agent_on(&path);
    agent.prepared_context_image().unwrap();
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    let root = vm.root();
    let state = vm.load_state_from_root().unwrap();
    let affinity = vm.cache_affinity();
    assert!(!agent.context_vm_offers_retrieval());

    // Another writer holds the session, so the checkpoint append fails.
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    let held =
        davinci_sys::lock::ExclusiveFileLock::try_acquire(std::path::Path::new(&lock)).unwrap();
    let error = agent
        .fold_context(FoldReason::WindowPressure, None)
        .unwrap_err();
    assert!(
        error.contains("context checkpoint persistence failed"),
        "{error}"
    );
    drop(held);

    assert_eq!(vm.root(), root);
    assert_eq!(vm.load_state_from_root().unwrap(), state);
    assert_eq!(vm.cache_affinity(), affinity);
    assert_eq!(vm.metrics().folds, 0);
    assert_eq!(vm.last_fold_reason(), None);
    assert!(!agent.context_vm_offers_retrieval());
    assert_eq!(vm.failure_count(), 1, "the rollback is recorded");
    assert!(agent
        .session
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .all(|entry| entry.entry_type != "context_checkpoint"));

    // A process that reloads the session sees the same state.
    let reloaded = active_agent_on(&path);
    reloaded.prepared_context_image().unwrap();
    let reloaded_vm = &reloaded.runtime.as_ref().unwrap().context_vm;
    assert_eq!(reloaded_vm.load_state_from_root().unwrap(), state);
    assert_eq!(reloaded_vm.cache_affinity(), affinity);
    assert_eq!(
        agent.prepared_context_image().unwrap().prefix_digest,
        reloaded.prepared_context_image().unwrap().prefix_digest
    );
}
