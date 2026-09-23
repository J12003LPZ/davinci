use davinci_agent::jobs::JobBook;
use davinci_agent::runtime::operations::CallerType;
use davinci_agent::runtime::operations::{
    AgentLaunchDisposition, AgentOperationAdapter, ChildExecutionContext, ChildExecutionKind,
    ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationContext,
    OperationJournal, OperationKind, RootNamespaceId, ToolOperationRuntime, WorkspaceId,
    WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_agent::ToolResult;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    runtime: RuntimeHandle,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let root = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let journal = Arc::new(
            OperationJournal::open(&temp_root.join("operations"), identity.clone(), root)
                .unwrap(),
        );
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: root,
            session_id: "operation-agent-launch".into(),
            runtime_run_id: run_id,
            parent_operation_id: None,
            agent_id,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace,
            caller: CallerType::HostControl,
            wire_tool_call_id: None,
        };
        let operations = ToolOperationRuntime::new(
            journal,
            context,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            &temp_root,
        )
        .unwrap();
        let runtime = RuntimeHandle::new(run_id, agent_id, RuntimeBus::new())
            .with_operation_runtime(operations);
        Self {
            _temp: temp,
            runtime,
        }
    }

    fn adapter(&self) -> AgentOperationAdapter {
        self.runtime.child_operation_adapter().unwrap()
    }

    fn child(
        &self,
        logical_id: &str,
        kind: ChildExecutionKind,
    ) -> (ChildExecutionContext, serde_json::Value) {
        let mut child = ChildExecutionContext::new(logical_id, &self.runtime, None);
        child.host = match kind {
            ChildExecutionKind::Subagent => "agent_tool",
            ChildExecutionKind::WorkflowPhase => "workflow_executor",
            ChildExecutionKind::BackgroundJob => "background_job",
        }
        .into();
        let payload = json!({
            "kind": kind,
            "logical_id": logical_id,
            "command_digest": "fixture",
        });
        (child, payload)
    }
}

fn result(content: &str) -> ToolResult {
    ToolResult {
        content: content.into(),
        is_error: false,
        details: Some(json!({"fixture": true})),
    }
}

#[test]
fn direct_subagent_launch_is_durable_and_replayed_once() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let (child, payload) = fixture.child("subagent:direct", ChildExecutionKind::Subagent);
    let operation = adapter
        .start(ChildExecutionKind::Subagent, child.clone(), payload.clone())
        .unwrap();
    assert_eq!(operation.disposition(), AgentLaunchDisposition::New);

    let executions = Arc::new(AtomicUsize::new(0));
    let first = operation
        .execute(false, || Ok(()), {
            let executions = Arc::clone(&executions);
            move || {
                executions.fetch_add(1, Ordering::SeqCst);
                result("subagent result")
            }
        })
        .unwrap();
    assert_eq!(first.content, "subagent result");

    let replay = adapter
        .start(ChildExecutionKind::Subagent, child, payload)
        .unwrap();
    assert_eq!(replay.disposition(), AgentLaunchDisposition::ExistingResult);
    assert!(
        replay.replay_result().unwrap().details.unwrap()["replayed_from_operation_journal"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert!(fixture
        .runtime
        .unresolved_child_operations()
        .unwrap()
        .is_empty());
}

#[test]
fn batched_read_only_children_admit_independently() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let mut operations = Vec::new();
    for index in 0..2 {
        let (child, payload) = fixture.child(
            &format!("subagent:batch-{index}"),
            ChildExecutionKind::Subagent,
        );
        operations.push(
            adapter
                .start(ChildExecutionKind::Subagent, child, payload)
                .unwrap(),
        );
    }
    assert!(operations
        .iter()
        .all(|operation| operation.should_execute()));
    assert_ne!(operations[0].operation_id(), operations[1].operation_id());

    let first = operations.remove(0);
    let second = operations.remove(0);
    assert_eq!(
        first
            .execute(false, || Ok(()), || result("left"))
            .unwrap()
            .content,
        "left"
    );
    assert_eq!(
        second
            .execute(false, || Ok(()), || result("right"))
            .unwrap()
            .content,
        "right"
    );
    assert!(fixture
        .runtime
        .unresolved_child_operations()
        .unwrap()
        .is_empty());
}

#[test]
fn workflow_phase_completion_is_linked_to_the_child_record() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let (mut child, payload) = fixture.child(
        "workflow:wf-1:phase-a:worker-1",
        ChildExecutionKind::WorkflowPhase,
    );
    child.workflow_id = Some(davinci_agent::WorkflowId::new());
    child.task_id = Some(davinci_agent::TaskId::new());
    let operation = adapter
        .start(ChildExecutionKind::WorkflowPhase, child, payload)
        .unwrap();
    let completed = operation
        .execute(false, || Ok(()), || result("phase complete"))
        .unwrap();
    assert_eq!(completed.content, "phase complete");
    let snapshot = adapter.runtime().dispatcher().journal().snapshot().unwrap();
    assert!(snapshot.operations.iter().any(|spec| {
        spec.kind() == OperationKind::WorkflowPhase
            && snapshot.attempts.iter().any(|attempt| {
                attempt.operation_id() == spec.operation_id()
                    && attempt.state()
                        == davinci_agent::runtime::operations::OperationState::Succeeded
            })
    }));
}

#[test]
fn background_notice_completes_the_durable_job_operation() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let (child, payload) = fixture.child("background:job-1", ChildExecutionKind::BackgroundJob);
    let operation = adapter
        .start(ChildExecutionKind::BackgroundJob, child, payload)
        .unwrap();
    let spawned = operation
        .begin(
            false,
            || Ok(()),
            || {
                #[cfg(windows)]
                {
                    std::process::Command::new("cmd")
                        .args(["/C", "exit 0"])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|error| davinci_agent::ToolError::Failed(error.to_string()))
                }
                #[cfg(not(windows))]
                {
                    std::process::Command::new("sh")
                        .args(["-c", "exit 0"])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|error| davinci_agent::ToolError::Failed(error.to_string()))
                }
            },
        )
        .unwrap()
        .unwrap();
    let mut jobs = JobBook::default();
    jobs.register_with_provenance_and_operation(
        "fixture job",
        spawned,
        None,
        None,
        None,
        Some(operation),
    );

    let notice = loop {
        let notices = jobs.take_unannounced();
        if let Some(notice) = notices.into_iter().next() {
            break notice;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(notice.status.succeeded());
    let (child, payload) = fixture.child("background:job-1", ChildExecutionKind::BackgroundJob);
    let replay = adapter
        .start(ChildExecutionKind::BackgroundJob, child, payload)
        .unwrap();
    assert_eq!(replay.disposition(), AgentLaunchDisposition::ExistingResult);
    assert!(replay
        .replay_result()
        .unwrap()
        .content
        .contains("fixture job"));
}

#[test]
fn reopening_the_parent_preserves_child_ownership_and_blocks_replacement() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let (child, payload) = fixture.child("subagent:unresolved", ChildExecutionKind::Subagent);
    let operation = adapter
        .start(ChildExecutionKind::Subagent, child.clone(), payload.clone())
        .unwrap();
    let resumed = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session_state_from(&fixture.runtime);
    let unresolved = resumed.unresolved_child_operations().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].operation_id, operation.operation_id());
    assert_eq!(unresolved[0].owner, operation.admitted.attempt.owner());

    let replacement = resumed
        .child_operation_adapter()
        .unwrap()
        .start(ChildExecutionKind::Subagent, child, payload)
        .unwrap();
    assert_eq!(
        replacement.disposition(),
        AgentLaunchDisposition::ExistingInFlight
    );
    assert!(!replacement.should_execute());
}
