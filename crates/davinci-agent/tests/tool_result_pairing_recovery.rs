//! R2: persisted tool-use / tool-result pairing is repaired or rejected when a
//! session is resumed, by call id only. Cut-stream behaviour is covered by
//! `ws3_stream_cut.rs`; publish-once after a crash by `operation_publication.rs`.

use davinci_agent::runtime::operations::{
    CallerType, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationAdmission,
    OperationContext, OperationJournal, RootNamespaceId, ToolOperationDispatcher,
    ToolOperationPlanner, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_agent::{Agent, SummarizeResponse, Summarizer};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, MessageContent, StopReason};
use davinci_session::{JsonlSession, SessionEntry};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn done() -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
        id: "pairing-done".into(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text { text: "ok".into() }],
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(StopReason::Stop),
        error_message: None,
    }
}

fn call_message(id: &str, name: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: json!({}),
        }],
        ..ChatMessage::default()
    }
}

fn result_message(id: &str, name: &str, text: &str) -> ChatMessage {
    ChatMessage {
        role: "toolResult".into(),
        content: vec![MessageContent::Text { text: text.into() }],
        tool_call_id: Some(id.into()),
        tool_name: Some(name.into()),
        is_error: Some(false),
        ..ChatMessage::default()
    }
}

fn append(session: &mut JsonlSession, message: &ChatMessage) {
    let mut entry = SessionEntry::message(
        &message.role,
        serde_json::to_value(&message.content).unwrap(),
    );
    entry.message = Some(serde_json::to_value(message).unwrap());
    session.append_entry(entry).unwrap();
}

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    session_path: PathBuf,
    session_id: String,
    identity: JournalIdentity,
    root: RootNamespaceId,
    run_id: RunId,
    agent_id: AgentId,
    owner: ExecutionOwner,
    journal: Arc<OperationJournal>,
}

impl Fixture {
    fn new(history: &[ChatMessage]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut session = JsonlSession::create_in_directory(
            &base.join("sessions"),
            workspace.to_str().unwrap(),
            None,
        )
        .unwrap();
        for message in history {
            append(&mut session, message);
        }
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let root = RootNamespaceId::new();
        let journal = Arc::new(
            OperationJournal::open(&base.join("journal"), identity.clone(), root).unwrap(),
        );
        Self {
            _temp: temp,
            workspace,
            session_path: session.path.clone(),
            session_id: session.header.id.clone(),
            identity,
            root,
            run_id: RunId::new(),
            agent_id: AgentId::new(),
            owner: ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            journal,
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

    /// A call whose effect latch was committed by a process that then died.
    fn latch_started_effect(&self, call_id: &str, tool: &str) {
        let plan = ToolOperationPlanner::provider_call(
            self.context(),
            call_id,
            tool,
            &json!({}),
            None,
            None,
        )
        .unwrap();
        let dispatcher = ToolOperationDispatcher::new(self.journal.clone(), self.owner);
        let admitted = match dispatcher.admit(plan.clone(), 7).unwrap() {
            OperationAdmission::New(admitted) => admitted,
            other => panic!("expected a new operation, got {other:?}"),
        };
        dispatcher
            .dispatch(&admitted, &plan, Some(7), false, || Ok(()), || ())
            .unwrap();
    }

    fn agent(&self) -> Agent {
        let operations = ToolOperationRuntime::new(
            self.journal.clone(),
            self.context(),
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            &self.workspace,
        )
        .unwrap();
        let runtime = RuntimeHandle::new(self.run_id, self.agent_id, RuntimeBus::new())
            .with_session(&self.session_id)
            .with_operation_runtime(operations);
        let mut agent = Agent::new("pairing fixture");
        agent
            .load_from_session(JsonlSession::open(&self.session_path).unwrap())
            .unwrap();
        agent.cwd = self.workspace.clone();
        agent.set_runtime(runtime);
        agent
    }
}

fn tool_results<'a>(messages: &'a [ChatMessage], id: &str) -> Vec<&'a ChatMessage> {
    messages
        .iter()
        .filter(|m| m.role == "toolResult" && m.tool_call_id.as_deref() == Some(id))
        .collect()
}

fn text(message: &ChatMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|c| match c {
            MessageContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn orphan_result_id_fails_closed() {
    let fixture = Fixture::new(&[
        ChatMessage::text("user", "start"),
        call_message("real_call", "read"),
        result_message("real_call", "read", "contents"),
        result_message("ghost_call", "read", "answers a call nobody made"),
    ]);
    let mut agent = fixture.agent();
    agent.prompt("continue");
    let mut requests = 0;
    let outcome = agent.run_loop(|_| {
        requests += 1;
        Ok(done())
    });
    let error = outcome.expect_err("an unpaired history must not reach the provider");
    assert!(error.contains("ghost_call"), "{error}");
    assert_eq!(requests, 0, "no provider request is built from an unpaired history");
}

#[test]
fn missing_result_after_recovery_has_explicit_nonexecution_or_unknown() {
    let fixture = Fixture::new(&[
        ChatMessage::text("user", "start"),
        call_message("never_started", "write"),
        call_message("effect_latched", "bash"),
    ]);
    // The second call's effect was latched in the journal; the first never was.
    fixture.latch_started_effect("effect_latched", "bash");
    let mut agent = fixture.agent();
    agent.prompt("continue");
    let mut seen = Vec::new();
    agent
        .run_loop(|agent| {
            seen = agent.messages.clone();
            Ok(done())
        })
        .unwrap();

    let not_started = tool_results(&seen, "never_started");
    assert_eq!(not_started.len(), 1);
    assert_eq!(not_started[0].is_error, Some(true));
    assert!(text(not_started[0]).starts_with("Not executed"), "{}", text(not_started[0]));

    let latched = tool_results(&seen, "effect_latched");
    assert_eq!(latched.len(), 1);
    assert_eq!(latched[0].is_error, Some(true));
    assert!(text(latched[0]).starts_with("Outcome unknown"), "{}", text(latched[0]));

    // Each answer sits right after its own call and before the new prompt.
    let roles: Vec<_> = seen.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles[..6],
        ["user", "assistant", "toolResult", "assistant", "toolResult", "user"]
    );
}

#[test]
fn duplicate_result_does_not_double_publish() {
    let fixture = Fixture::new(&[
        ChatMessage::text("user", "start"),
        call_message("dup_call", "read"),
        result_message("dup_call", "read", "first"),
        result_message("dup_call", "read", "second"),
    ]);
    let mut agent = fixture.agent();
    agent.prompt("continue");
    let mut wire = Vec::new();
    agent
        .run_loop(|agent| {
            wire = davinci_ai::openai_responses_input(&agent.messages);
            Ok(done())
        })
        .unwrap();
    let outputs: Vec<_> = wire
        .iter()
        .filter(|item| item["type"] == "function_call_output" && item["call_id"] == "dup_call")
        .collect();
    assert_eq!(outputs.len(), 1, "{wire:?}");
    assert!(outputs[0]["output"].to_string().contains("first"));
}

#[test]
fn compacted_history_retains_matching_ids() {
    let fixture = Fixture::new(&[]);
    let mut agent = fixture.agent();
    agent.compaction.keep_recent_tokens = 1;
    agent.summarizer = Some(Summarizer::new(|_| {
        Ok(SummarizeResponse {
            text: "Summary of earlier work.".into(),
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    let mut history = Vec::new();
    for round in 0..4 {
        history.push(ChatMessage::text("user", format!("task {round} {}", "x ".repeat(300))));
        history.push(call_message(&format!("call_{round}"), "read"));
        history.push(result_message(&format!("call_{round}"), "read", &"y ".repeat(300)));
    }
    for message in &history {
        agent.messages.push(message.clone());
        append(agent.session.as_mut().unwrap(), message);
    }
    assert!(agent.compact(None).compacted);
    let session = agent.session.take().unwrap();
    drop(agent);

    let mut resumed = Agent::new("resumed");
    resumed.load_from_session(session).unwrap();
    let mut called = std::collections::HashSet::new();
    let mut results = 0;
    for message in &resumed.messages {
        for content in &message.content {
            if let MessageContent::ToolCall { id, .. } = content {
                called.insert(id.clone());
            }
        }
        if message.role == "toolResult" {
            results += 1;
            let id = message.tool_call_id.clone().unwrap();
            assert!(called.contains(&id), "result {id} lost its call after compaction");
        }
    }
    assert!(results > 0, "the kept tail must still contain a tool exchange");
    resumed.prompt("continue");
    resumed.run_loop(|_| Ok(done())).unwrap();
}
