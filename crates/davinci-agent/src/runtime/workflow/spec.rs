//! Workflow specification types.

use serde::{Deserialize, Serialize};

/// Specification for a deterministic, multi-phase agent workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSpec {
    pub schema_version: u16,
    pub name: String,
    pub phases: Vec<WorkflowPhaseSpec>,
    pub max_parallel_agents: usize,
    pub max_total_agents: usize,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
}

/// Specification for a single phase within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowPhaseSpec {
    pub id: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub workers: Vec<WorkflowWorkerSpec>,
    pub join: WorkflowJoin,
}

/// Specification for an individual worker within a phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowWorkerSpec {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub agent_profile: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub isolation: Option<String>, // e.g. "shared" or "worktree"
    #[serde(default)]
    pub max_turns: Option<usize>,
    #[serde(default)]
    pub retry_budget: Option<usize>,
}

/// Join policy determining when a phase is considered complete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowJoin {
    All,
    Any,
    Quorum { required: usize },
}

pub const VALID_3_PHASE_WORKFLOW_JSON: &str = r#"{
    "schema_version": 1,
    "name": "full-feature-pipeline",
    "max_parallel_agents": 4,
    "max_total_agents": 8,
    "phases": [
        {
            "id": "investigate",
            "depends_on": [],
            "join": "all",
            "workers": [
                {
                    "id": "repo-searcher",
                    "prompt": "search repo for feature references",
                    "tools": ["grep", "find", "read"],
                    "isolation": "shared"
                },
                {
                    "id": "doc-searcher",
                    "prompt": "fetch documentation",
                    "tools": ["web_fetch", "web_search"],
                    "isolation": "shared"
                }
            ]
        },
        {
            "id": "plan",
            "depends_on": ["investigate"],
            "join": "all",
            "workers": [
                {
                    "id": "architect",
                    "prompt": "produce architecture plan",
                    "tools": ["read", "todo"],
                    "isolation": "shared"
                }
            ]
        },
        {
            "id": "implement",
            "depends_on": ["plan"],
            "join": "all",
            "workers": [
                {
                    "id": "writer-1",
                    "prompt": "implement component A",
                    "tools": ["read", "write", "edit"],
                    "isolation": "worktree"
                },
                {
                    "id": "writer-2",
                    "prompt": "implement component B",
                    "tools": ["read", "write", "edit"],
                    "isolation": "worktree"
                }
            ]
        }
    ]
}"#;

/// What the calling turn contributes to a workflow run.
#[derive(Debug, Clone, Default)]
pub struct WorkflowLaunch {
    pub service_tier: davinci_ai::CodexServiceTier,
    pub parent_permission_mode: Option<crate::PermissionMode>,
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub parent_tools: Vec<String>,
    /// Host-owned execution authority inherited from the launching agent.
    /// Workflow JSON never deserializes these fields.
    pub foreground_supervisor: Option<crate::jobs::supervisor::SupervisorCommand>,
    pub sandbox: Option<davinci_protocol::SandboxSpec>,
    /// Report completion to the lead's mailbox (background runs).
    pub report_to_lead: bool,
    /// The calling turn's cancellation. A synchronous run stops when the
    /// turn is interrupted; a background run ignores it and descends from
    /// the session's team token instead.
    pub turn_token: Option<crate::runtime::CancellationToken>,
    /// Session concurrency ceiling (`workflowMaxConcurrentAgents`).
    pub max_concurrent_agents: Option<usize>,
    /// Agents a run may schedule before it is flagged as a large workflow.
    pub large_workflow_threshold: Option<usize>,
}
