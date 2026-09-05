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
