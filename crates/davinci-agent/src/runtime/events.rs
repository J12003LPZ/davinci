//! Versioned runtime events and envelopes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

use super::ids::{AgentId, RunId, TaskId, WorkflowId};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEventEnvelope {
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
            schema_version: 1,
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
    },
    AgentMessageDelivered {
        to: AgentId,
        message_id: Uuid,
    },
    TaskCreated {
        task_id: TaskId,
    },
    TaskAssigned {
        task_id: TaskId,
        agent_id: AgentId,
    },
    TaskCompleted {
        task_id: TaskId,
        success: bool,
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
    CacheAffinity {
        key: String,
        reason: String,
    },
    RuntimeWarning {
        code: String,
        message: String,
    },
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
}
