//! R1: a process killed after a tool effect started must resume with an
//! explicit outcome-unknown tool result, never a replay and never a success.
//!
//! The first Agent runs in a child process (this test binary re-invoked with
//! `--exact agent_tool_resume_child`). Its tool executor writes `START`,
//! announces itself, then sleeps instead of writing `END`; the parent kills
//! the child at that point. A second Agent, with a fresh execution owner as
//! after a real restart, resumes the same session and journal.
//!
//! The effect tool is a native-extension tool (class Edit), not the built-in
//! `bash`: the built-in shell needs the process supervisor host that only the
//! CLI wires, and the latch under test (`EffectPossible` before the executor
//! runs) is the same journal path for both.

#[path = "support/operation_harness.rs"]
#[allow(dead_code)]
mod operation_harness;

use davinci_agent::runtime::operations::{
    ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationContext,
    OperationJournal, OperationState, RootNamespaceId, ToolOperationRuntime, WorkspaceId,
    WorkspaceIdentity,
};
use davinci_agent::runtime::{
    AgentId, CapabilitySource, RunId, RuntimeBus, RuntimeCapability, RuntimeHandle,
};
use davinci_agent::{
    Agent, CompletionEvidence, CustomToolExecutor, MutationVerificationState, PermissionMode,
    ToolClass, ToolResult, VerificationCoverage, VerificationEvidence,
};
use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
use davinci_session::{JsonlSession, SessionEntry};
use operation_harness::{wait_for_marker, SerializedIdentity};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CHILD_ENV: &str = "DAVINCI_TOOL_RESUME_CHILD_CONFIG";
const CALL_ID: &str = "slow_call_1";

#[derive(Serialize, Deserialize)]
struct World {
    workspace: PathBuf,
    session_path: PathBuf,
    session_id: String,
    journal_dir: PathBuf,
    identity: SerializedIdentity,
    root: RootNamespaceId,
    run_id: RunId,
    agent_id: AgentId,
    marker: PathBuf,
}

impl World {
    fn effects(&self) -> String {
        std::fs::read_to_string(self.workspace.join("effects.txt")).unwrap_or_default()
    }

    fn agent(&self, owner: ExecutionOwner, executor: CustomToolExecutor) -> Agent {
        let identity = self.identity.clone().into_journal_identity();
        let journal = Arc::new(
            OperationJournal::open(&self.journal_dir, identity.clone(), self.root).unwrap(),
        );
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: self.root,
            session_id: self.session_id.clone(),
            runtime_run_id: self.run_id,
            parent_operation_id: None,
            agent_id: self.agent_id,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: davinci_agent::runtime::operations::CallerType::ProviderToolCall,
            wire_tool_call_id: None,
        };
        let operations =
            ToolOperationRuntime::new(journal, context, owner, &self.workspace).unwrap();
        let runtime = RuntimeHandle::new(self.run_id, self.agent_id, RuntimeBus::new())
            .with_session(&self.session_id)
            .with_operation_runtime(operations);
        let mut agent = Agent::new("tool resume fixture");
        agent
            .load_from_session(JsonlSession::open(&self.session_path).unwrap())
            .unwrap();
        agent.cwd = self.workspace.clone();
        agent.set_runtime(runtime);
        agent
            .runtime
            .as_ref()
            .unwrap()
            .capability_registry
            .register(RuntimeCapability::new(
                "slow_effect",
                CapabilitySource::NativeExtension,
                ToolClass::Edit,
                false,
                &json!({"type":"object"}),
                None,
            ));
        agent.apply_extension_tools(&["slow_effect".into()]);
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.retry_attempts = 2;
        agent.retry_base_delay_ms = 0;
        agent.custom_tool_executor = Some(executor);
        agent
    }
}

fn assistant(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
        id: "resume-fixture".into(),
        role: "assistant".into(),
        content,
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(stop),
        error_message: None,
    }
}

fn slow_call() -> AssistantMessage {
    assistant(
        vec![ContentBlock::ToolCall {
            id: CALL_ID.into(),
            name: "slow_effect".into(),
            arguments: json!({}),
        }],
        StopReason::ToolUse,
    )
}

fn append(path: &Path, line: &str) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
    file.sync_all().unwrap();
}

/// Runs only when the parent re-invokes this binary with the config env var.
#[test]
fn agent_tool_resume_child() {
    let Some(path) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let world: World = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let marker = world.marker.clone();
    let executor = CustomToolExecutor::new(move |cwd, _, _| {
        append(&cwd.join("effects.txt"), "START");
        std::fs::write(&marker, b"started").unwrap();
        // The parent kills this process here; END is never written.
        std::thread::sleep(Duration::from_secs(120));
        append(&cwd.join("effects.txt"), "END");
        Ok(ToolResult {
            content: "finished".into(),
            is_error: false,
            details: None,
        })
    });
    let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
    let mut agent = world.agent(owner, executor);
    agent.prompt("Run the slow effect.");
    let _ = agent.run_loop(|_| Ok(slow_call()));
}

fn new_world() -> (tempfile::TempDir, World) {
    let temp = tempfile::tempdir().unwrap();
    let root_dir = std::fs::canonicalize(temp.path()).unwrap();
    let workspace = root_dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let session = JsonlSession::create_in_directory(
        &root_dir.join("sessions"),
        workspace.to_str().unwrap(),
        None,
    )
    .unwrap();
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let world = World {
        workspace,
        session_path: session.path.clone(),
        session_id: session.header.id.clone(),
        journal_dir: root_dir.join("operation-journal"),
        identity: SerializedIdentity {
            journal_id: identity.journal_id,
            workspace: identity.workspace.clone(),
        },
        root: RootNamespaceId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
        marker: root_dir.join("started.marker"),
    };
    drop(session);
    (temp, world)
}

fn kill_child_after_effect_started(temp: &tempfile::TempDir, world: &World) {
    let config_path = temp.path().join("child-config.json");
    std::fs::write(&config_path, serde_json::to_vec(world).unwrap()).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "agent_tool_resume_child", "--nocapture"])
        .env(CHILD_ENV, config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_marker(&mut child, &world.marker, Duration::from_secs(60));
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
}

#[test]
fn agent_crash_after_started_effect_reports_unknown_without_replay() {
    let (temp, world) = new_world();
    kill_child_after_effect_started(&temp, &world);
    assert_eq!(world.effects().lines().collect::<Vec<_>>(), ["START"]);

    // Fresh process: a new execution owner, the same session and journal.
    let reruns = Arc::new(AtomicUsize::new(0));
    let counter = reruns.clone();
    let executor = CustomToolExecutor::new(move |cwd, _, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        append(&cwd.join("effects.txt"), "START");
        Ok(ToolResult {
            content: "replayed".into(),
            is_error: false,
            details: None,
        })
    });
    let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
    let mut agent = world.agent(owner, executor);
    // Generation 0 and a killed mutating call: unverified, never not-required.
    assert_eq!(agent.mutation_verification_state().mutation_generation, 1);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    agent.prompt("What happened to the slow effect?");
    let mut requests = 0;
    let mut wire = Vec::new();
    agent
        .run_loop(|agent| {
            requests += 1;
            if requests == 1 {
                let result = agent
                    .messages
                    .iter()
                    .find(|message| {
                        message.role == "toolResult"
                            && message.tool_call_id.as_deref() == Some(CALL_ID)
                    })
                    .expect("the first request after resume must answer the killed call");
                assert_eq!(result.is_error, Some(true), "unknown is never a success");
                let text = format!("{:?}", result.content).to_lowercase();
                assert!(
                    text.contains("unknown"),
                    "result must say the outcome is unknown: {text}"
                );
                wire = davinci_ai::openai_responses_input(&agent.messages);
                // Even a model that insists on the same call is not re-run.
                return Ok(slow_call());
            }
            Ok(assistant(
                vec![ContentBlock::Text {
                    text: "acknowledged".into(),
                }],
                StopReason::Stop,
            ))
        })
        .unwrap();

    assert_eq!(
        wire.iter()
            .filter(|item| item["type"] == "function_call_output" && item["call_id"] == CALL_ID)
            .count(),
        1,
        "the wire carries exactly one output for the killed call: {wire:?}"
    );
    assert_eq!(reruns.load(Ordering::SeqCst), 0, "no re-dispatch");
    assert_eq!(world.effects().lines().collect::<Vec<_>>(), ["START"]);
    assert!(!world.effects().contains("END"));

    // Durable: the answer sits right after the assistant call in the session.
    let session = JsonlSession::open(&world.session_path).unwrap();
    let roles: Vec<_> = session
        .entries
        .iter()
        .filter_map(|entry| entry.message.as_ref())
        .filter_map(|message| message["role"].as_str())
        .collect();
    assert_eq!(&roles[..3], ["user", "assistant", "toolResult"]);

    drop(agent);
    let journal = OperationJournal::open(
        &world.journal_dir,
        world.identity.clone().into_journal_identity(),
        world.root,
    )
    .unwrap();
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.attempts.len(), 1);
    assert_ne!(snapshot.attempts[0].state(), OperationState::Succeeded);
}

/// Persist a passing verification of `generation`, as a finished earlier run would.
fn seed_verified_generation(world: &World, generation: u64) {
    let mut session = JsonlSession::open(&world.session_path).unwrap();
    let mut entry = SessionEntry::message("user", json!([]));
    entry.entry_type = "custom".into();
    entry.message = None;
    entry.custom_type = Some("mutation_verification".into());
    entry.extra.insert(
        "data".into(),
        serde_json::to_value(MutationVerificationState {
            mutation_generation: generation,
            verified_generation: Some(generation),
            last_verification_succeeded: true,
            latest_evidence: Some(VerificationEvidence {
                generation,
                command: "cargo test".into(),
                succeeded: true,
                mutation_paths: Vec::new(),
                verification_targets: Vec::new(),
                coverage: VerificationCoverage::Broad,
            }),
            ..MutationVerificationState::default()
        })
        .unwrap(),
    );
    session.append_entry(entry).unwrap();
}

#[test]
fn agent_crash_after_started_effect_invalidates_a_verified_generation_before_any_request() {
    const SEEDED: u64 = 3;
    let (temp, world) = new_world();
    seed_verified_generation(&world, SEEDED);
    let resume = |world: &World| {
        let executor = CustomToolExecutor::new(|_, _, _| panic!("a resume must not run the tool"));
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
        world.agent(owner, executor)
    };
    assert_eq!(
        resume(&world).completion_evidence(),
        CompletionEvidence::Verified,
        "the seed must start verified"
    );
    kill_child_after_effect_started(&temp, &world);

    let mut agent = resume(&world);
    let state = agent.mutation_verification_state();
    assert_eq!(state.mutation_generation, SEEDED + 1);
    assert_eq!(state.verified_generation, None);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);

    // The interrupted-call answer lands after the bumped state, and a second
    // restore sees both: it must not bump again.
    agent.prompt("What happened?");
    agent
        .run_loop(|_| {
            Ok(assistant(
                vec![ContentBlock::Text { text: "ok".into() }],
                StopReason::Stop,
            ))
        })
        .unwrap();
    drop(agent);
    let agent = resume(&world);
    assert_eq!(
        agent.mutation_verification_state().mutation_generation,
        SEEDED + 1
    );
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    assert_eq!(world.effects().lines().collect::<Vec<_>>(), ["START"]);
}
