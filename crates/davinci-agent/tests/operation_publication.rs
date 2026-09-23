use davinci_agent::runtime::operations::{
    AttemptId, CallerType, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity,
    OperationAdmission, OperationContext, OperationJournal, OperationResultReady, OperationSpec,
    OperationState, OutboxState, PayloadDigest, RootNamespaceId, ToolOperationDispatcher,
    ToolOperationPlanner, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
    JOURNAL_DATABASE_FILE_NAME, MAX_OUTBOX_BATCH_SIZE, SESSION_RESULT_CONSUMER,
};
use davinci_agent::runtime::{
    AgentId, CapabilitySource, DeclaredEffect, RunId, RuntimeBus, RuntimeCapability, RuntimeHandle,
};
use davinci_agent::{Agent, PermissionMode, PostToolHook, ToolClass, ToolResult};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, StopReason};
use davinci_session::{JsonlSession, SessionEntry};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    workspace: PathBuf,
    session_path: PathBuf,
    session_id: String,
    journal_dir: PathBuf,
    identity: JournalIdentity,
    root: RootNamespaceId,
    run_id: RunId,
    agent_id: AgentId,
    owner: ExecutionOwner,
    journal: Arc<OperationJournal>,
    dispatcher: ToolOperationDispatcher,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = JsonlSession::create_in_directory(
            &temp.path().join("sessions"),
            workspace.to_str().unwrap(),
            None,
        )
        .unwrap();
        let session_path = session.path.clone();
        let session_id = session.header.id.clone();
        drop(session);

        let root = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
        let journal_dir = temp.path().join("operation-journal");
        let journal =
            Arc::new(OperationJournal::open(&journal_dir, identity.clone(), root).unwrap());
        let dispatcher = ToolOperationDispatcher::new(journal.clone(), owner);
        Self {
            _temp: temp,
            workspace,
            session_path,
            session_id,
            journal_dir,
            identity,
            root,
            run_id,
            agent_id,
            owner,
            journal,
            dispatcher,
        }
    }

    fn context(&self) -> OperationContext {
        OperationContext {
            journal_id: self.identity.journal_id,
            root_namespace_id: self.root,
            session_id: self.session_id.clone(),
            runtime_run_id: self.run_id,
            parent_operation_id: None,
            agent_id: self.agent_id,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: self.identity.workspace.clone(),
            caller: CallerType::ProviderToolCall,
            wire_tool_call_id: None,
        }
    }

    fn admit_and_start(
        &self,
        plan: davinci_agent::runtime::operations::PlannedToolOperation,
    ) -> davinci_agent::runtime::operations::AdmittedOperation {
        let admitted = match self.dispatcher.admit(plan.clone(), 7).unwrap() {
            OperationAdmission::New(admitted) => admitted,
            other => panic!("expected a new operation, got {other:?}"),
        };
        self.dispatcher
            .dispatch(&admitted, &plan, Some(7), false, || Ok(()), || ())
            .unwrap();
        admitted
    }

    fn provider_plan(
        &self,
        call_id: &str,
        tool: &str,
        arguments: &Value,
    ) -> davinci_agent::runtime::operations::PlannedToolOperation {
        ToolOperationPlanner::provider_call(self.context(), call_id, tool, arguments, None, None)
            .unwrap()
    }

    fn complete_provider_call(
        &self,
        call_id: &str,
        tool: &str,
        arguments: &Value,
        result: &ToolResult,
    ) -> AttemptId {
        let admitted = self.admit_and_start(self.provider_plan(call_id, tool, arguments));
        self.dispatcher
            .complete_for_session(&admitted, result, true)
            .unwrap();
        admitted.attempt.attempt_id()
    }

    fn ready_for(&self, attempt_id: AttemptId) -> (OperationResultReady, uuid::Uuid) {
        let outbox = self
            .journal
            .pending_outbox_for_consumer(SESSION_RESULT_CONSUMER, MAX_OUTBOX_BATCH_SIZE)
            .unwrap()
            .into_iter()
            .find(|outbox| outbox.attempt_id == attempt_id)
            .expect("result outbox item");
        (serde_json::from_value(outbox.payload).unwrap(), outbox.id)
    }

    fn runtime(&self) -> RuntimeHandle {
        let operations = ToolOperationRuntime::new(
            self.journal.clone(),
            self.context(),
            self.owner,
            &self.workspace,
        )
        .unwrap();
        RuntimeHandle::new(self.run_id, self.agent_id, RuntimeBus::new())
            .with_session(&self.session_id)
            .with_operation_runtime(operations)
    }

    fn agent(&self) -> Agent {
        let mut agent = Agent::new("operation publication fixture");
        agent
            .load_from_session(JsonlSession::open(&self.session_path).unwrap())
            .unwrap();
        agent.cwd = self.workspace.clone();
        agent.set_runtime(self.runtime());
        agent
    }

    fn batch_child_plan(
        &self,
        parent: &OperationSpec,
    ) -> davinci_agent::runtime::operations::PlannedToolOperation {
        ToolOperationPlanner::batch_child(
            self.context(),
            parent.operation_id(),
            1,
            "read",
            &json!({"path": "child.txt"}),
            None,
            None,
        )
        .unwrap()
    }
}

fn rich_result(is_error: bool) -> ToolResult {
    ToolResult {
        content: if is_error {
            "mutation completed but reporting failed".into()
        } else {
            "read completed".into()
        },
        is_error,
        details: Some(json!({
            "artifactRef": {"uri": "artifact://run/output", "digest": "sha256:artifact"},
            "imageRef": {"uri": "image://run/preview", "digest": "sha256:image"},
            "diagnostics": {"attempt": 2, "source": "adapter"}
        })),
    }
}

fn done_response() -> AssistantMessage {
    AssistantMessage {
        id: "publication-done".into(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text {
            text: "continued".into(),
        }],
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(StopReason::Stop),
        error_message: None,
    }
}

fn assert_tool_result(message: &ChatMessage, call_id: &str, content: &str, is_error: bool) {
    assert_eq!(message.role, "toolResult");
    assert_eq!(message.tool_call_id.as_deref(), Some(call_id));
    assert_eq!(message.is_error, Some(is_error));
    assert_eq!(
        message.content[0],
        davinci_ai::MessageContent::Text {
            text: content.into()
        }
    );
}

fn projection_entry(
    ready: &OperationResultReady,
    result: &ToolResult,
    batch_child: bool,
) -> SessionEntry {
    let mut presentation = result.clone();
    if batch_child {
        let details = presentation.details.get_or_insert_with(|| json!({}));
        details["operation_recovered_from_raw"] = Value::Bool(true);
    }
    let value = if batch_child {
        serde_json::to_value(&presentation).unwrap()
    } else {
        let mut message = ChatMessage::tool_result(
            &ready.tool_call_id,
            "write",
            presentation.content,
            presentation.is_error,
        );
        if let Some(details) = presentation.details {
            message.extra.insert("details".into(), details);
        }
        serde_json::to_value(&message).unwrap()
    };
    let presentation_digest = PayloadDigest::of_json(&value).unwrap();
    let mut entry = SessionEntry::message(
        if batch_child {
            "operation_result"
        } else {
            "toolResult"
        },
        Value::Null,
    );
    entry.message = Some(value);
    if batch_child {
        entry.entry_type = "custom".into();
        entry.custom_type = Some("operation_result".into());
    }
    entry.extra.insert(
        "operationEventId".into(),
        Value::String(ready.event_id.clone()),
    );
    entry.extra.insert(
        "operationId".into(),
        Value::String(ready.operation_id.to_string()),
    );
    entry.extra.insert(
        "attemptId".into(),
        Value::String(ready.attempt_id.to_string()),
    );
    entry.extra.insert(
        "rawResultDigest".into(),
        Value::String(ready.result_digest.to_string()),
    );
    entry.extra.insert(
        "presentationDigest".into(),
        Value::String(presentation_digest.to_string()),
    );
    entry.extra.insert(
        "toolCallId".into(),
        Value::String(ready.tool_call_id.clone()),
    );
    entry
}

#[test]
fn full_success_and_failure_tool_results_are_durably_bound_to_outbox_digests() {
    let fixture = Fixture::new();
    let success = rich_result(false);
    let success_attempt = fixture.complete_provider_call(
        "success-call",
        "read",
        &json!({"path": "output.txt"}),
        &success,
    );
    let failure = rich_result(true);
    let failure_attempt = fixture.complete_provider_call(
        "failure-call",
        "write",
        &json!({"path": "output.txt", "content": "changed"}),
        &failure,
    );

    for (attempt_id, expected) in [(success_attempt, success), (failure_attempt, failure)] {
        let stored = fixture
            .journal
            .load_result_for_attempt(attempt_id)
            .unwrap()
            .expect("full raw result artifact");
        let expected_payload = serde_json::to_value(expected).unwrap();
        assert_eq!(stored.payload, expected_payload);
        let (ready, _) = fixture.ready_for(attempt_id);
        assert_eq!(ready.result_digest, stored.reference.payload_digest);
        assert_eq!(
            PayloadDigest::of_json(&stored.payload).unwrap(),
            ready.result_digest
        );
        assert_eq!(ready.attempt_id, attempt_id);
        assert_eq!(ready.session_id, fixture.session_id);
    }
    let snapshot = fixture.journal.snapshot().unwrap();
    assert_eq!(snapshot.results.len(), 2);
    assert_eq!(snapshot.outbox.len(), 2);
}

#[test]
fn result_transition_and_session_outbox_are_atomic() {
    let fixture = Fixture::new();
    let plan = fixture.provider_plan("atomic-call", "write", &json!({"path": "x"}));
    let admitted = fixture.admit_and_start(plan);
    let before = fixture
        .journal
        .load_attempt(admitted.attempt.attempt_id())
        .unwrap();
    let before_json = serde_json::to_value(&before).unwrap();
    let connection =
        Connection::open(fixture.journal_dir.join(JOURNAL_DATABASE_FILE_NAME)).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_result_outbox BEFORE INSERT ON operation_outbox
             BEGIN SELECT RAISE(ABORT, 'injected outbox failure'); END;",
        )
        .unwrap();

    assert!(fixture
        .dispatcher
        .complete_for_session(&admitted, &rich_result(false), true)
        .is_err());
    let attempt_json: String = connection
        .query_row(
            "SELECT attempt_json FROM attempts WHERE attempt_id = ?1",
            params![admitted.attempt.attempt_id().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&attempt_json).unwrap(),
        before_json
    );
    let result_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM operation_results", [], |row| {
            row.get(0)
        })
        .unwrap();
    let outbox_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM operation_outbox", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(result_count, 0);
    assert_eq!(outbox_count, 0);
}

#[test]
fn reopening_a_session_replays_pending_result_before_provider_continuation() {
    let fixture = Fixture::new();
    let raw = rich_result(false);
    let attempt_id = fixture.complete_provider_call(
        "recover-call",
        "read",
        &json!({"path": "output.txt"}),
        &raw,
    );
    let (ready, _) = fixture.ready_for(attempt_id);
    let mut agent = fixture.agent();
    let mut calls = 0;
    agent
        .run_loop(|agent| {
            calls += 1;
            let projected = agent
                .messages
                .iter()
                .find(|message| message.tool_call_id.as_deref() == Some("recover-call"))
                .expect("result must be present before provider continuation");
            assert_tool_result(projected, "recover-call", &raw.content, false);
            assert_eq!(
                projected.extra["details"]["artifactRef"]["uri"],
                "artifact://run/output"
            );
            Ok(done_response())
        })
        .unwrap();
    assert_eq!(calls, 1);
    let session = JsonlSession::open(&fixture.session_path).unwrap();
    assert_eq!(
        session
            .entries
            .iter()
            .filter(|entry| entry.id == ready.event_id)
            .count(),
        1
    );
    assert!(fixture
        .journal
        .pending_outbox_for_consumer(SESSION_RESULT_CONSUMER, MAX_OUTBOX_BATCH_SIZE)
        .unwrap()
        .is_empty());
}

#[test]
fn append_before_ack_crash_is_deduplicated_and_acknowledged_after_reopen() {
    let fixture = Fixture::new();
    let raw = rich_result(false);
    let attempt_id = fixture.complete_provider_call(
        "append-crash-call",
        "write",
        &json!({"path": "output.txt", "content": "saved"}),
        &raw,
    );
    let (ready, outbox_id) = fixture.ready_for(attempt_id);
    let mut session = JsonlSession::open(&fixture.session_path).unwrap();
    let entry = projection_entry(&ready, &raw, false);
    let parent = session.leaf_id.clone();
    session
        .append_entry_once(&ready.event_id, parent.as_deref(), entry)
        .unwrap();
    drop(session);

    let mut agent = fixture.agent();
    let mut calls = 0;
    agent
        .run_loop(|agent| {
            calls += 1;
            assert_eq!(
                agent
                    .messages
                    .iter()
                    .filter(|message| message.tool_call_id.as_deref() == Some("append-crash-call"))
                    .count(),
                1
            );
            Ok(done_response())
        })
        .unwrap();
    assert_eq!(calls, 1);
    let reopened = JsonlSession::open(&fixture.session_path).unwrap();
    assert_eq!(
        reopened
            .entries
            .iter()
            .filter(|entry| entry.id == ready.event_id)
            .count(),
        1
    );
    let item = fixture
        .journal
        .snapshot()
        .unwrap()
        .outbox
        .into_iter()
        .find(|item| item.id == outbox_id)
        .unwrap();
    assert_eq!(item.state, OutboxState::Acknowledged);
}

#[test]
fn missing_or_corrupt_result_artifact_blocks_provider_continuation() {
    for corrupt in [false, true] {
        let fixture = Fixture::new();
        let attempt_id = fixture.complete_provider_call(
            if corrupt {
                "corrupt-call"
            } else {
                "missing-call"
            },
            "read",
            &json!({"path": "output.txt"}),
            &rich_result(false),
        );
        let connection =
            Connection::open(fixture.journal_dir.join(JOURNAL_DATABASE_FILE_NAME)).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER operation_results_no_update;
                 DROP TRIGGER operation_results_no_delete;",
            )
            .unwrap();
        if corrupt {
            connection
                .execute(
                    "UPDATE operation_results SET payload_json = 'null' WHERE attempt_id = ?1",
                    params![attempt_id.to_string()],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "DELETE FROM operation_results WHERE attempt_id = ?1",
                    params![attempt_id.to_string()],
                )
                .unwrap();
        }
        drop(connection);

        let mut agent = fixture.agent();
        let mut calls = 0;
        let error = agent
            .run_loop(|_| {
                calls += 1;
                Ok(done_response())
            })
            .unwrap_err();
        assert_eq!(calls, 0);
        assert!(
            error.contains("blocked") || error.contains("corrupt"),
            "unexpected recovery error: {error}"
        );
        assert_eq!(
            fixture
                .journal
                .pending_outbox_for_consumer(SESSION_RESULT_CONSUMER, MAX_OUTBOX_BATCH_SIZE)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn pending_result_for_another_session_does_not_block_this_session() {
    let fixture = Fixture::new();
    let mut other_context = fixture.context();
    other_context.session_id = "another-session".into();
    let plan = ToolOperationPlanner::provider_call(
        other_context,
        "other-session-call",
        "read",
        &json!({"path": "other.txt"}),
        None,
        None,
    )
    .unwrap();
    let admitted = fixture.admit_and_start(plan);
    fixture
        .dispatcher
        .complete_for_session(&admitted, &rich_result(false), true)
        .unwrap();

    let mut agent = fixture.agent();
    let mut calls = 0;
    agent
        .run_loop(|_| {
            calls += 1;
            Ok(done_response())
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        fixture
            .journal
            .pending_outbox_for_consumer(SESSION_RESULT_CONSUMER, MAX_OUTBOX_BATCH_SIZE)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn duplicate_batch_child_publication_is_acknowledged_without_a_second_entry() {
    let fixture = Fixture::new();
    let batch_capability = RuntimeCapability::new(
        "batch",
        CapabilitySource::Builtin,
        ToolClass::Read,
        true,
        &json!({"type": "object"}),
        None,
    )
    .with_declared_effects(vec![DeclaredEffect::FileSystemRead]);
    let parent_plan = ToolOperationPlanner::provider_call(
        fixture.context(),
        "batch-parent-call",
        "batch",
        &json!({"operations": [{"tool": "read", "args": {"path": "child.txt"}}]}),
        Some(&batch_capability),
        None,
    )
    .unwrap();
    let parent = fixture.admit_and_start(parent_plan);
    let child_plan = fixture.batch_child_plan(&parent.spec);
    let child = fixture.admit_and_start(child_plan);
    let child_result = rich_result(false);
    fixture
        .dispatcher
        .complete_for_session(&child, &child_result, true)
        .unwrap();
    let parent_result = ToolResult {
        content: "batch finished".into(),
        is_error: false,
        details: None,
    };
    fixture
        .dispatcher
        .complete_for_session(&parent, &parent_result, true)
        .unwrap();

    let child_ready = fixture.ready_for(child.attempt.attempt_id()).0;
    let mut session = JsonlSession::open(&fixture.session_path).unwrap();
    let parent_entry = session.leaf_id.clone();
    session
        .append_entry_once(
            &child_ready.event_id,
            parent_entry.as_deref(),
            projection_entry(&child_ready, &child_result, true),
        )
        .unwrap();
    drop(session);

    let mut agent = fixture.agent();
    agent.run_loop(|_| Ok(done_response())).unwrap();
    let reopened = JsonlSession::open(&fixture.session_path).unwrap();
    assert_eq!(
        reopened
            .entries
            .iter()
            .filter(|entry| entry.id == child_ready.event_id)
            .count(),
        1
    );
    let snapshot = fixture.journal.snapshot().unwrap();
    assert_eq!(snapshot.outbox.len(), 2);
    assert!(snapshot
        .outbox
        .iter()
        .all(|item| item.state == OutboxState::Acknowledged));
}

#[test]
fn post_tool_hook_failure_preserves_successful_raw_mutation_and_failed_presentation() {
    let fixture = Fixture::new();
    let mut agent = fixture.agent();
    agent.tools = vec!["write".into()];
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.post_tool = Some(PostToolHook(Arc::new(|_, _, _, _, mut result| {
        result.content = "post-tool presentation failed".into();
        result.is_error = true;
        result.details = Some(json!({"postHookFailure": "rendering failed"}));
        result
    })));
    let mut calls = 0;
    agent
        .run_loop(|_| {
            calls += 1;
            if calls == 1 {
                Ok(AssistantMessage {
                    id: "write-call".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: "hook-failure-call".into(),
                        name: "write".into(),
                        arguments: json!({"path": "effect.txt", "content": "mutated"}),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                })
            } else {
                Ok(done_response())
            }
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("effect.txt")).unwrap(),
        "mutated"
    );

    let snapshot = fixture.journal.snapshot().unwrap();
    assert_eq!(snapshot.attempts.len(), 1);
    assert_eq!(snapshot.attempts[0].state(), OperationState::Succeeded);
    let stored = fixture
        .journal
        .load_result_for_attempt(snapshot.attempts[0].attempt_id())
        .unwrap()
        .unwrap();
    let raw: ToolResult = serde_json::from_value(stored.payload).unwrap();
    assert!(!raw.is_error);
    assert!(!raw.content.contains("post-tool presentation failed"));

    let session = JsonlSession::open(&fixture.session_path).unwrap();
    let presented = session
        .entries
        .iter()
        .filter_map(|entry| entry.message.as_ref())
        .find(|message| {
            message.get("toolCallId").and_then(Value::as_str) == Some("hook-failure-call")
        })
        .expect("post-hook presentation is persisted");
    assert_eq!(presented["isError"], true);
    assert_eq!(
        presented["content"][0]["text"],
        "post-tool presentation failed"
    );
    assert_eq!(presented["details"]["postHookFailure"], "rendering failed");
    assert!(snapshot
        .outbox
        .iter()
        .all(|item| item.state == OutboxState::Acknowledged));
}
