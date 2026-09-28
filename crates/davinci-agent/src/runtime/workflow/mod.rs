//! Deterministic general workflow engine for Davinci.

pub mod executor;
pub mod limits;
pub mod spec;
pub mod state;
pub mod tools;
pub mod validate;

pub use executor::{
    PhaseExecutionState, PhaseStatus, WorkflowExecutionError, WorkflowExecutionState,
    WorkflowExecutor, WorkflowStatus,
};
pub use limits::{
    WorkflowSettings, WorkflowSizeGuideline, DEFAULT_MAX_CONCURRENT_AGENTS,
    MAX_CONCURRENT_AGENTS_CAP, MAX_TOTAL_AGENTS_CAP,
};
pub use spec::{WorkflowJoin, WorkflowLaunch, WorkflowPhaseSpec, WorkflowSpec, WorkflowWorkerSpec};
pub use state::{WorkflowArtifact, WorkflowStateError, WorkflowStateStore};
pub use tools::{
    find_saved_workflow, save_workflow_to_project, workflow_run_tool,
    workflow_run_tool_with_parent, workflow_status_tool, workflow_tool_specs,
};
pub use validate::{
    is_mutating_tool, is_mutating_tool_with_registry, validate_workflow,
    validate_workflow_with_capabilities, validate_workflow_with_permissions,
    WorkflowValidationError,
};

#[cfg(test)]
pub(crate) fn test_worktree_manager(root: &std::path::Path) -> crate::runtime::WorktreeManager {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.hooksPath=NUL",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    ] {
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    crate::runtime::WorktreeManager::new(repo, root.join("worktrees"))
}
