//! Versioned runtime events and envelopes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

use super::ids::{AgentId, RunId, TaskId, WorkflowId};
use super::mailbox::AgentMessage;
use super::tasks::TaskRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Main,
    Subagent,
    GraphWorker,
    WorkflowWorker,
    Teammate,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Starting,
    Running,
    Waiting,
    Idle,
    Stopping,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRecord {
    pub id: AgentId,
    pub run_id: RunId,
    pub parent: Option<AgentId>,
    pub kind: AgentKind,
    pub name: String,
    pub provider: String,
    pub model_id: String,
    pub cwd: PathBuf,
    pub state: AgentState,
    pub task_id: Option<TaskId>,
    pub worktree: Option<PathBuf>,
    pub started_ms: i64,
    pub updated_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl AgentRecord {
    pub fn is_reconnectable(&self) -> bool {
        false
    }
}

pub const RUNTIME_EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEventEnvelope {
    #[serde(default = "default_runtime_event_schema_version")]
    pub schema_version: u16, // starts at 1
    pub event_id: Uuid,
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub run_id: RunId,
    pub session_id: Option<String>,
    pub agent_id: Option<AgentId>,
    pub parent_agent_id: Option<AgentId>,
    pub payload: RuntimeEvent,
}

fn default_runtime_event_schema_version() -> u16 {
    RUNTIME_EVENT_SCHEMA_VERSION
}

impl RuntimeEventEnvelope {
    pub fn new(
        sequence: u64,
        run_id: RunId,
        session_id: Option<String>,
        agent_id: Option<AgentId>,
        parent_agent_id: Option<AgentId>,
        payload: RuntimeEvent,
    ) -> Self {
        Self {
            schema_version: RUNTIME_EVENT_SCHEMA_VERSION,
            event_id: Uuid::now_v7(),
            sequence,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            run_id,
            session_id,
            agent_id,
            parent_agent_id,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeEvent {
    SessionStarted {
        resumed: bool,
    },
    SessionEnded {
        reason: String,
    },
    TurnStarted,
    TurnEnded {
        success: bool,
    },
    UserPromptSubmitted,
    InstructionsLoaded {
        paths: Vec<PathBuf>,
    },
    PreToolUse {
        call_id: String,
        tool: String,
        args: Value,
    },
    PermissionRequested {
        call_id: String,
        tool: String,
    },
    PermissionDenied {
        call_id: String,
        reason: String,
    },
    PostToolUse {
        call_id: String,
        tool: String,
        is_error: bool,
    },
    PostToolBatch {
        calls: usize,
        failures: usize,
    },
    AgentStarted {
        record: AgentRecord,
    },
    AgentStateChanged {
        from: AgentState,
        to: AgentState,
    },
    AgentMessageQueued {
        to: AgentId,
        message_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<AgentMessage>,
    },
    AgentMessageDelivered {
        to: AgentId,
        message_id: Uuid,
    },
    TaskCreated {
        task_id: TaskId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        record: Option<TaskRecord>,
    },
    TaskAssigned {
        task_id: TaskId,
        agent_id: AgentId,
    },
    /// Approval proposal only: never replayed as a committed task state.
    TaskCompletionRequested {
        task_id: TaskId,
        expected_revision: u64,
    },
    /// Observation emitted after the task projection has been committed.
    TaskCompleted {
        task_id: TaskId,
        success: bool,
    },
    /// Canonical contract decision event recording allowed or denied effects with secrets redacted.
    ContractDecision {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<AgentId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<TaskId>,
        contract_revision: u64,
        allowed: bool,
        effect: String,
        reason: String,
    },
    WorkflowStarted {
        workflow_id: WorkflowId,
    },
    WorkflowPhaseChanged {
        workflow_id: WorkflowId,
        phase: String,
    },
    WorkflowEnded {
        workflow_id: WorkflowId,
        success: bool,
    },
    PreCompact {
        estimated_tokens: u64,
    },
    PostCompact {
        before_tokens: u64,
        after_tokens: u64,
    },
    PreModelSwitch {
        provider: String,
        model_id: String,
    },
    PostModelSwitch {
        provider: String,
        model_id: String,
    },
    WorktreeCreated {
        path: PathBuf,
    },
    WorktreeRemoved {
        path: PathBuf,
    },
    ContextBuilt {
        estimated_tokens: u64,
        cache_key: Option<String>,
    },
    ContextVmShadowCompared {
        legacy_tokens: u64,
        vm_tokens: u64,
        missing_user_refs: u64,
        missing_tool_refs: u64,
    },
    ContextVmFolded {
        epoch: u64,
        reason: String,
        checkpoint_id: String,
        before_tokens: u64,
        after_tokens: u64,
    },
    CacheAffinity {
        key: String,
        reason: String,
    },
    RuntimeWarning {
        code: String,
        message: String,
    },
    WorkerControlAcknowledged {
        command_id: Uuid,
        task_id: Option<TaskId>,
        agent_id: AgentId,
        generation: u64,
        action: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    BeforeWrite {
        path: PathBuf,
        bytes: usize,
    },
    AfterWrite {
        path: PathBuf,
        bytes: usize,
        is_error: bool,
    },
    BeforeProcessStart {
        executable: String,
        argv: Vec<String>,
        cwd: PathBuf,
    },
    AfterProcessExit {
        executable: String,
        argv: Vec<String>,
        exit_code: Option<i32>,
        is_error: bool,
    },
    BeforeTest {
        framework: String,
        targets: Vec<String>,
    },
    AfterTest {
        framework: String,
        targets: Vec<String>,
        passed: bool,
        failures: usize,
    },
    BeforeCommit {
        message: String,
        files: Vec<String>,
    },
    AfterCommit {
        commit_id: Option<String>,
        message: String,
        is_error: bool,
    },
    BeforeCompletion {
        task_id: Option<TaskId>,
    },
}

impl RuntimeEvent {
    /// Creates a ContractDecision event for a denied action with secrets redacted from effect and reason.
    pub fn contract_denied(
        actor_id: Option<AgentId>,
        task_id: Option<TaskId>,
        contract_revision: u64,
        effect: &str,
        reason: &str,
    ) -> Self {
        Self::ContractDecision {
            actor_id,
            task_id,
            contract_revision,
            allowed: false,
            effect: crate::runtime::contracts::redact_secrets(effect),
            reason: crate::runtime::contracts::redact_secrets(reason),
        }
    }

    /// Creates a ContractDecision event for an allowed action with secrets redacted.
    pub fn contract_allowed(
        actor_id: Option<AgentId>,
        task_id: Option<TaskId>,
        contract_revision: u64,
        effect: &str,
        reason: &str,
    ) -> Self {
        Self::ContractDecision {
            actor_id,
            task_id,
            contract_revision,
            allowed: true,
            effect: crate::runtime::contracts::redact_secrets(effect),
            reason: crate::runtime::contracts::redact_secrets(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_event_envelope_roundtrip_pre_tool_use() {
        let envelope = RuntimeEventEnvelope::new(
            1,
            RunId::new(),
            Some("session-123".into()),
            Some(AgentId::new()),
            None,
            RuntimeEvent::PreToolUse {
                call_id: "call_abc".into(),
                tool: "read".into(),
                args: json!({"path": "src/main.rs"}),
            },
        );

        let serialized = serde_json::to_string(&envelope).unwrap();
        let deserialized: RuntimeEventEnvelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn runtime_event_envelope_legacy_schema_version_defaults() {
        let envelope = RuntimeEventEnvelope::new(
            7,
            RunId::new(),
            Some("legacy-session".into()),
            None,
            None,
            RuntimeEvent::TurnStarted,
        );

        let mut legacy_json = serde_json::to_value(&envelope).unwrap();
        legacy_json
            .as_object_mut()
            .unwrap()
            .remove("schema_version");

        let deserialized: RuntimeEventEnvelope = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(deserialized.schema_version, RUNTIME_EVENT_SCHEMA_VERSION);

        let serialized = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            serialized.get("schema_version"),
            Some(&json!(RUNTIME_EVENT_SCHEMA_VERSION))
        );
    }

    #[test]
    fn runtime_event_envelope_roundtrip_post_compact() {
        let envelope = RuntimeEventEnvelope::new(
            42,
            RunId::new(),
            Some("session-456".into()),
            Some(AgentId::new()),
            Some(AgentId::new()),
            RuntimeEvent::PostCompact {
                before_tokens: 120_000,
                after_tokens: 45_000,
            },
        );

        let serialized = serde_json::to_string(&envelope).unwrap();
        let deserialized: RuntimeEventEnvelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn runtime_event_envelope_roundtrip_agent_state_changed() {
        let envelope = RuntimeEventEnvelope::new(
            5,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::AgentStateChanged {
                from: AgentState::Starting,
                to: AgentState::Running,
            },
        );

        let serialized = serde_json::to_string(&envelope).unwrap();
        let deserialized: RuntimeEventEnvelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn runtime_event_envelope_roundtrip_contract_decision() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();

        // Contract denied event with secret redaction
        let denied = RuntimeEvent::contract_denied(
            Some(agent_id),
            Some(task_id),
            2,
            "write: /secrets/token?key=sk-secret9999",
            "access denied: token ghp_privatetoken was forbidden",
        );

        match &denied {
            RuntimeEvent::ContractDecision {
                actor_id,
                task_id: tid,
                contract_revision,
                allowed,
                effect,
                reason,
            } => {
                assert_eq!(*actor_id, Some(agent_id));
                assert_eq!(*tid, Some(task_id));
                assert_eq!(*contract_revision, 2);
                assert!(!*allowed);
                assert!(!effect.contains("sk-secret9999"));
                assert!(effect.contains("sk-[REDACTED]"));
                assert!(!reason.contains("ghp_privatetoken"));
                assert!(reason.contains("ghp_[REDACTED]"));
            }
            other => panic!("expected ContractDecision, got {other:?}"),
        }

        let envelope = RuntimeEventEnvelope::new(
            10,
            RunId::new(),
            Some("session-contract".into()),
            Some(agent_id),
            None,
            denied,
        );

        let serialized = serde_json::to_string(&envelope).unwrap();
        let deserialized: RuntimeEventEnvelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(envelope, deserialized);
    }
}
