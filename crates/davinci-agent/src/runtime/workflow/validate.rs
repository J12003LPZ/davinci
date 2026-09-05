//! Validation of deterministic workflow specifications.

use std::collections::{HashMap, HashSet};
use thiserror::Error;

use super::spec::{WorkflowJoin, WorkflowSpec};
use crate::permission::{tool_class, PermissionMode, ToolClass};

const KNOWN_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "powershell",
    "grep",
    "find",
    "ls",
    "web_fetch",
    "web_search",
    "todo",
    "job_output",
    "job_kill",
    "notebook_edit",
    "mcp_read",
    "agent",
    "batch",
    "apply_patch",
    "retrieve_output",
    "memory_search",
    "update_plan",
    "tool_search",
    "agent_status",
    "agent_message",
    "agent_stop",
    "task_create",
    "task_update",
    "task_list",
    "workflow_status",
];

/// Graph internal tools that cannot be directly called by general workflows.
pub const GRAPH_INTERNAL_TOOLS: &[&str] = &["graph_submit"];

const MUTATION_TOOLS: &[&str] = &[
    "write",
    "edit",
    "notebook_edit",
    "bash",
    "powershell",
    "apply_patch",
    "graph_run",
];

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WorkflowValidationError {
    #[error("invalid schema version: {0} (expected 1)")]
    InvalidSchemaVersion(u16),
    #[error("empty workflow name")]
    EmptyWorkflowName,
    #[error("workflow has no phases")]
    NoPhases,
    #[error("empty phase id")]
    EmptyPhaseId,
    #[error("duplicate phase id: {0}")]
    DuplicatePhaseId(String),
    #[error("phase '{phase}' depends on missing phase '{dependency}'")]
    MissingPhaseDependency { phase: String, dependency: String },
    #[error("dependency cycle detected involving phase: {0}")]
    DependencyCycle(String),
    #[error("max_parallel_agents must be between 1 and 8 (found {0})")]
    InvalidMaxParallelAgents(usize),
    #[error("max_total_agents must be between 1 and 64 (found {0})")]
    InvalidMaxTotalAgents(usize),
    #[error("total workers count ({total}) exceeds max_total_agents ({max})")]
    TotalWorkersExceedsCap { total: usize, max: usize },
    #[error("phase '{0}' has no workers")]
    EmptyPhaseWorkers(String),
    #[error("empty worker id in phase '{0}'")]
    EmptyWorkerId(String),
    #[error("phase '{phase}' has duplicate worker id: {worker}")]
    DuplicateWorkerId { phase: String, worker: String },
    #[error("invalid quorum: required {required} but phase '{phase}' has {workers} workers")]
    InvalidQuorum {
        phase: String,
        required: usize,
        workers: usize,
    },
    #[error("parallel writer violation in phase '{phase}': multiple workers declare mutating tools without worktree isolation")]
    ParallelWriterViolation { phase: String },
    #[error("worker '{worker}' declares unknown or hidden tool: '{tool}'")]
    HiddenTool { worker: String, tool: String },
    #[error("worker '{worker}' cannot call internal graph tool '{tool}' directly")]
    GraphInternalToolRejected { worker: String, tool: String },
    #[error("worker '{worker}' requires mutation tool '{tool}' which is denied under permission mode '{mode}'")]
    PermissionViolation {
        worker: String,
        tool: String,
        mode: &'static str,
    },
}

pub fn is_mutating_tool(tool: &str) -> bool {
    MUTATION_TOOLS.contains(&tool) || matches!(tool_class(tool), ToolClass::Edit | ToolClass::Shell)
}

/// Validate a workflow specification against static structure and invariants.
pub fn validate_workflow(spec: &WorkflowSpec) -> Result<(), WorkflowValidationError> {
    validate_workflow_with_permissions(spec, None)
}

/// Validate a workflow specification with optional parent permission mode enforcement.
pub fn validate_workflow_with_permissions(
    spec: &WorkflowSpec,
    parent_permission_mode: Option<PermissionMode>,
) -> Result<(), WorkflowValidationError> {
    validate_workflow_with_permissions_and_tools(spec, parent_permission_mode, &[])
}

/// Validate a workflow specification with optional parent permission mode enforcement
/// and explicitly exposed extra tools (e.g. `graph_run`).
pub fn validate_workflow_with_permissions_and_tools(
    spec: &WorkflowSpec,
    parent_permission_mode: Option<PermissionMode>,
    extra_tools: &[&str],
) -> Result<(), WorkflowValidationError> {
    if spec.schema_version != 1 {
        return Err(WorkflowValidationError::InvalidSchemaVersion(
            spec.schema_version,
        ));
    }
    if spec.name.trim().is_empty() {
        return Err(WorkflowValidationError::EmptyWorkflowName);
    }
    if spec.phases.is_empty() {
        return Err(WorkflowValidationError::NoPhases);
    }
    if spec.max_parallel_agents < 1 || spec.max_parallel_agents > 8 {
        return Err(WorkflowValidationError::InvalidMaxParallelAgents(
            spec.max_parallel_agents,
        ));
    }
    if spec.max_total_agents < 1 || spec.max_total_agents > 64 {
        return Err(WorkflowValidationError::InvalidMaxTotalAgents(
            spec.max_total_agents,
        ));
    }

    // 1. Validate Phase IDs and unique existence
    let mut phase_ids = HashSet::new();
    let mut total_workers = 0;

    for phase in &spec.phases {
        let trimmed_id = phase.id.trim();
        if trimmed_id.is_empty() {
            return Err(WorkflowValidationError::EmptyPhaseId);
        }
        if !phase_ids.insert(trimmed_id.to_string()) {
            return Err(WorkflowValidationError::DuplicatePhaseId(
                trimmed_id.to_string(),
            ));
        }
        if phase.workers.is_empty() {
            return Err(WorkflowValidationError::EmptyPhaseWorkers(
                trimmed_id.to_string(),
            ));
        }
        total_workers += phase.workers.len();
    }

    if total_workers > spec.max_total_agents {
        return Err(WorkflowValidationError::TotalWorkersExceedsCap {
            total: total_workers,
            max: spec.max_total_agents,
        });
    }

    // 2. Validate phase dependencies existence
    for phase in &spec.phases {
        for dep in &phase.depends_on {
            if dep == &phase.id {
                return Err(WorkflowValidationError::DependencyCycle(phase.id.clone()));
            }
            if !phase_ids.contains(dep) {
                return Err(WorkflowValidationError::MissingPhaseDependency {
                    phase: phase.id.clone(),
                    dependency: dep.clone(),
                });
            }
        }
    }

    // 3. Validate dependency DAG acyclicity (DFS with recursion stack)
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for phase in &spec.phases {
        adj.entry(&phase.id)
            .or_default()
            .extend(phase.depends_on.iter().map(String::as_str));
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut rec_stack: HashSet<&str> = HashSet::new();

    fn dfs_cycle<'a>(
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
    ) -> Option<String> {
        visited.insert(node);
        rec_stack.insert(node);

        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if let Some(cycle_node) = dfs_cycle(neighbor, adj, visited, rec_stack) {
                        return Some(cycle_node);
                    }
                } else if rec_stack.contains(neighbor) {
                    return Some(neighbor.to_string());
                }
            }
        }

        rec_stack.remove(node);
        None
    }

    for phase in &spec.phases {
        if !visited.contains(phase.id.as_str()) {
            if let Some(cycle_node) =
                dfs_cycle(phase.id.as_str(), &adj, &mut visited, &mut rec_stack)
            {
                return Err(WorkflowValidationError::DependencyCycle(cycle_node));
            }
        }
    }

    // 4. Validate Phase Workers, Quorums, Writers and Permissions
    for phase in &spec.phases {
        let mut worker_ids = HashSet::new();
        let mut has_mutating_worker = false;
        let mut has_unisolated_mutating_worker = false;

        // Quorum check
        if let WorkflowJoin::Quorum { required } = phase.join {
            if required < 1 || required > phase.workers.len() {
                return Err(WorkflowValidationError::InvalidQuorum {
                    phase: phase.id.clone(),
                    required,
                    workers: phase.workers.len(),
                });
            }
        }

        for worker in &phase.workers {
            let wid = worker.id.trim();
            if wid.is_empty() {
                return Err(WorkflowValidationError::EmptyWorkerId(phase.id.clone()));
            }
            if !worker_ids.insert(wid.to_string()) {
                return Err(WorkflowValidationError::DuplicateWorkerId {
                    phase: phase.id.clone(),
                    worker: wid.to_string(),
                });
            }

            let mut worker_mutates = false;

            // Check tools
            for tool in &worker.tools {
                if GRAPH_INTERNAL_TOOLS.contains(&tool.as_str()) {
                    return Err(WorkflowValidationError::GraphInternalToolRejected {
                        worker: wid.to_string(),
                        tool: tool.clone(),
                    });
                }
                if !KNOWN_TOOLS.contains(&tool.as_str())
                    && !extra_tools.contains(&tool.as_str())
                    && !tool.starts_with("mcp__")
                {
                    return Err(WorkflowValidationError::HiddenTool {
                        worker: wid.to_string(),
                        tool: tool.clone(),
                    });
                }
                if is_mutating_tool(tool) {
                    worker_mutates = true;
                    if parent_permission_mode == Some(PermissionMode::ReadOnly) {
                        return Err(WorkflowValidationError::PermissionViolation {
                            worker: wid.to_string(),
                            tool: tool.clone(),
                            mode: "read-only",
                        });
                    }
                }
            }

            let is_isolated = worker.isolation.as_deref() == Some("worktree");
            if worker_mutates {
                has_mutating_worker = true;
                if !is_isolated {
                    has_unisolated_mutating_worker = true;
                }
            }
        }

        // Parallel writer guarantee:
        // If a phase has multiple workers and any worker mutates without worktree isolation,
        // or if multiple workers mutate, they cannot share the workspace.
        if phase.workers.len() > 1 && has_mutating_worker && has_unisolated_mutating_worker {
            return Err(WorkflowValidationError::ParallelWriterViolation {
                phase: phase.id.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::workflow::spec::*;

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

    #[test]
    fn test_valid_3_phase_fixture_passes_validation() {
        let spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        assert!(validate_workflow(&spec).is_ok());
    }

    #[test]
    fn test_cycle_detection() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        // Create cycle: investigate -> implement -> plan -> investigate
        spec.phases[0].depends_on = vec!["implement".into()];
        let err = validate_workflow(&spec).unwrap_err();
        assert!(matches!(err, WorkflowValidationError::DependencyCycle(_)));
    }

    #[test]
    fn test_self_dependency_cycle() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        spec.phases[0].depends_on = vec!["investigate".into()];
        let err = validate_workflow(&spec).unwrap_err();
        assert!(matches!(err, WorkflowValidationError::DependencyCycle(_)));
    }

    #[test]
    fn test_missing_dependency() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        spec.phases[1].depends_on = vec!["non-existent-phase".into()];
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::MissingPhaseDependency {
                phase: "plan".into(),
                dependency: "non-existent-phase".into()
            }
        );
    }

    #[test]
    fn test_invalid_quorum() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        // Phase investigate has 2 workers; quorum required = 3 is invalid
        spec.phases[0].join = WorkflowJoin::Quorum { required: 3 };
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::InvalidQuorum {
                phase: "investigate".into(),
                required: 3,
                workers: 2
            }
        );

        // Required 0 is also invalid
        spec.phases[0].join = WorkflowJoin::Quorum { required: 0 };
        let err0 = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err0,
            WorkflowValidationError::InvalidQuorum {
                phase: "investigate".into(),
                required: 0,
                workers: 2
            }
        );
    }

    #[test]
    fn test_parallel_writer_violation() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        // Phase implement has 2 workers; make writer-1 isolation shared instead of worktree
        spec.phases[2].workers[0].isolation = Some("shared".into());
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::ParallelWriterViolation {
                phase: "implement".into()
            }
        );
    }

    #[test]
    fn test_hidden_tool_rejected() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        spec.phases[0].workers[0]
            .tools
            .push("secret_arbitrary_eval".into());
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::HiddenTool {
                worker: "repo-searcher".into(),
                tool: "secret_arbitrary_eval".into()
            }
        );
    }

    #[test]
    fn test_permission_violation_in_readonly_mode() {
        let spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        let err =
            validate_workflow_with_permissions(&spec, Some(PermissionMode::ReadOnly)).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::PermissionViolation { .. }
        ));
    }

    #[test]
    fn test_workflow_cannot_call_graph_internals_directly() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        spec.phases[0].workers[0].tools.push("graph_submit".into());

        // Default validation rejects graph internals
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::GraphInternalToolRejected {
                worker: "repo-searcher".into(),
                tool: "graph_submit".into(),
            }
        );

        // Even when explicitly passed in extra_tools, graph internals are rejected
        let err_extra =
            validate_workflow_with_permissions_and_tools(&spec, None, &["graph_submit"])
                .unwrap_err();
        assert_eq!(
            err_extra,
            WorkflowValidationError::GraphInternalToolRejected {
                worker: "repo-searcher".into(),
                tool: "graph_submit".into(),
            }
        );
    }

    #[test]
    fn test_workflow_graph_run_rejected_unless_explicitly_exposed() {
        let mut spec: WorkflowSpec = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();
        // Add graph_run to an isolated worker in the implement phase
        spec.phases[2].workers[0].tools.push("graph_run".into());

        // Without explicitly exposing graph_run, it is rejected as HiddenTool
        let err = validate_workflow(&spec).unwrap_err();
        assert_eq!(
            err,
            WorkflowValidationError::HiddenTool {
                worker: "writer-1".into(),
                tool: "graph_run".into(),
            }
        );

        // When graph_run is explicitly exposed, validation succeeds
        let res = validate_workflow_with_permissions_and_tools(&spec, None, &["graph_run"]);
        assert!(res.is_ok());

        // But in ReadOnly mode, graph_run is rejected as a mutation tool
        let err_ro = validate_workflow_with_permissions_and_tools(
            &spec,
            Some(PermissionMode::ReadOnly),
            &["graph_run"],
        )
        .unwrap_err();
        assert!(matches!(
            err_ro,
            WorkflowValidationError::PermissionViolation { .. }
        ));
    }
}
