//! Workflow coordination tools for Davinci agent runtime.
//!
//! Exposes `workflow_run` and `workflow_status` tools allowing agents to
//! construct, validate, persist, and execute deterministic multi-phase workflows.

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::executor::WorkflowStatus;
use super::spec::WorkflowSpec;
use super::validate::validate_workflow;
use crate::runtime::ids::WorkflowId;
use crate::runtime::workflow::WorkflowExecutor;
use crate::runtime::workflow::WorkflowStateStore;
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};

pub fn workflow_tool_specs() -> Vec<AgentTool> {
    vec![
        AgentTool {
            name: "workflow_run".into(),
            description: "Execute a multi-phase deterministic agent workflow from a validated JSON spec, or run a saved workflow by name.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "spec": {
                        "type": "object",
                        "description": "Workflow specification schema (name, description, max_parallel_agents, max_total_agents, phases)"
                    },
                    "name": {
                        "type": "string",
                        "description": "Name of a saved workflow to run from .davinci/workflows/<name>.json"
                    },
                    "save_as": {
                        "type": "string",
                        "description": "Optional name to save this workflow under .davinci/workflows/<save_as>.json for future reuse"
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Whether to execute asynchronously in the background (default: true)"
                    }
                }
            }),
        },
        AgentTool {
            name: "workflow_status".into(),
            description: "Check status, phases, and artifacts of a workflow by workflow_id, or list all tracked workflows.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": {
                        "type": "string",
                        "description": "Workflow ID to inspect (optional; if omitted, lists all workflows)"
                    }
                }
            }),
        },
    ]
}

fn is_valid_workflow_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn find_saved_workflow(
    cwd: &Path,
    name: &str,
    project_trusted: bool,
) -> Result<WorkflowSpec, String> {
    if !is_valid_workflow_name(name) {
        return Err(format!("Invalid workflow name: '{name}'"));
    }

    // Check project locations first
    let project_candidates = [
        cwd.join(".davinci")
            .join("workflows")
            .join(format!("{name}.json")),
        cwd.join(".pi")
            .join("workflows")
            .join(format!("{name}.json")),
    ];

    for path in &project_candidates {
        if path.is_file() {
            if !project_trusted {
                return Err(format!(
                    "Project workflow '{name}' requires project trust. Run /trust to approve this checkout."
                ));
            }
            let data = std::fs::read_to_string(path)
                .map_err(|e| format!("Failed reading workflow file {}: {e}", path.display()))?;
            let spec: WorkflowSpec = serde_json::from_str(&data)
                .map_err(|e| format!("Invalid workflow spec in {}: {e}", path.display()))?;
            return Ok(spec);
        }
    }

    // Check user global locations
    let home_dir = davinci_session::default_agent_dir();
    let global_candidates = [home_dir.join("workflows").join(format!("{name}.json"))];

    for path in &global_candidates {
        if path.is_file() {
            let data = std::fs::read_to_string(path)
                .map_err(|e| format!("Failed reading workflow file {}: {e}", path.display()))?;
            let spec: WorkflowSpec = serde_json::from_str(&data)
                .map_err(|e| format!("Invalid workflow spec in {}: {e}", path.display()))?;
            return Ok(spec);
        }
    }

    Err(format!(
        "Saved workflow '{name}' not found in project (.davinci/workflows/) or global (~/.davinci/workflows/)"
    ))
}

pub fn save_workflow_to_project(
    cwd: &Path,
    name: &str,
    spec: &WorkflowSpec,
    project_trusted: bool,
) -> Result<PathBuf, String> {
    if !is_valid_workflow_name(name) {
        return Err(format!("Invalid workflow name: '{name}'"));
    }
    if !project_trusted {
        return Err(format!(
            "Cannot save project workflow '{name}' in untrusted checkout. Run /trust first."
        ));
    }

    let dir = cwd.join(".davinci").join("workflows");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed creating directory {}: {e}", dir.display()))?;

    let file_path = dir.join(format!("{name}.json"));
    let serialized = serde_json::to_string_pretty(spec)
        .map_err(|e| format!("Failed serializing workflow spec: {e}"))?;

    std::fs::write(&file_path, serialized)
        .map_err(|e| format!("Failed writing workflow file {}: {e}", file_path.display()))?;

    Ok(file_path)
}

pub fn workflow_run_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let name_val = input.get("name").and_then(Value::as_str);
    let spec_val = input.get("spec");
    let save_as = input.get("save_as").and_then(Value::as_str);
    let background = input
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let spec: WorkflowSpec = if let Some(name) = name_val {
        find_saved_workflow(cwd, name, runtime.project_trusted).map_err(ToolError::Failed)?
    } else if let Some(spec_obj) = spec_val {
        serde_json::from_value(spec_obj.clone())
            .map_err(|e| ToolError::Failed(format!("Invalid workflow spec: {e}")))?
    } else {
        return Err(ToolError::Failed(
            "Either 'spec' (JSON definition) or 'name' (saved workflow name) must be provided"
                .into(),
        ));
    };

    validate_workflow(&spec).map_err(|e| ToolError::Failed(e.to_string()))?;

    let mut saved_path_info = None;
    if let Some(save_name) = save_as {
        let path = save_workflow_to_project(cwd, save_name, &spec, runtime.project_trusted)
            .map_err(ToolError::Failed)?;
        saved_path_info = Some(path.display().to_string());
    }

    let executor = match &runtime.workflow_executor {
        Some(exec) => exec.clone(),
        None => {
            let store = WorkflowStateStore::new();
            std::sync::Arc::new(WorkflowExecutor::new(runtime.clone(), store, None))
        }
    };

    if background {
        let wf_id = executor
            .execute_background(spec.clone())
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let resp = serde_json::json!({
            "workflow_id": wf_id.to_string(),
            "name": spec.name,
            "status": "running",
            "saved_to": saved_path_info,
            "message": "Workflow started in background. Use workflow_status to inspect progress."
        });

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&resp).unwrap(),
            is_error: false,
            details: None,
        })
    } else {
        let state = executor
            .execute(spec)
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let mut val = serde_json::to_value(&state).unwrap();
        if let Some(saved) = saved_path_info {
            val["saved_to"] = serde_json::Value::String(saved);
        }

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&val).unwrap(),
            is_error: state.status == WorkflowStatus::Failed,
            details: None,
        })
    }
}

pub fn workflow_status_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let executor = match &runtime.workflow_executor {
        Some(exec) => exec.clone(),
        None => {
            return Ok(ToolResult {
                content: serde_json::to_string_pretty(&serde_json::json!([])).unwrap(),
                is_error: false,
                details: None,
            });
        }
    };

    if let Some(id_str) = input.get("workflow_id").and_then(Value::as_str) {
        let wf_id = id_str
            .parse::<WorkflowId>()
            .map_err(|_| ToolError::Failed(format!("Invalid workflow ID format: '{id_str}'")))?;

        let state = executor
            .get_state(&wf_id)
            .ok_or_else(|| ToolError::Failed(format!("Workflow '{id_str}' not found")))?;

        let mut phase_artifacts = serde_json::Map::new();
        for phase_id in state.phases.keys() {
            let arts = executor.store.list_phase_artifacts(wf_id, phase_id);
            let summaries: Vec<_> = arts.into_iter().map(|a| a.to_reference_summary()).collect();
            phase_artifacts.insert(phase_id.clone(), serde_json::Value::Array(summaries));
        }

        let resp = serde_json::json!({
            "workflow": state,
            "artifacts": phase_artifacts,
        });

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&resp).unwrap(),
            is_error: false,
            details: None,
        })
    } else {
        let workflows = executor.list_workflows();
        let summaries: Vec<_> = workflows
            .into_iter()
            .map(|w| {
                serde_json::json!({
                    "workflow_id": w.id.to_string(),
                    "name": w.name,
                    "status": w.status,
                    "phases_count": w.phases.len(),
                    "started_ms": w.started_ms,
                    "finished_ms": w.finished_ms,
                    "error": w.error,
                })
            })
            .collect();

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&summaries).unwrap(),
            is_error: false,
            details: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::workflow::spec::VALID_3_PHASE_WORKFLOW_JSON;
    use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};

    fn setup_context(trusted: bool) -> (ToolContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bus = RuntimeBus::new();
        let runtime =
            RuntimeHandle::new(RunId::new(), AgentId::new(), bus).with_project_trusted(trusted);
        let store = WorkflowStateStore::new();
        let executor = std::sync::Arc::new(WorkflowExecutor::new(runtime.clone(), store, None));
        let runtime = runtime.with_workflow_executor(executor);

        let context = ToolContext {
            runtime: Some(runtime),
            ..Default::default()
        };
        (context, dir)
    }

    #[test]
    fn test_workflow_tool_specs() {
        let specs = workflow_tool_specs();
        assert_eq!(specs.len(), 2);
        assert!(specs.iter().any(|s| s.name == "workflow_run"));
        assert!(specs.iter().any(|s| s.name == "workflow_status"));
    }

    #[test]
    fn test_workflow_run_sync_and_status() {
        let (context, dir) = setup_context(true);
        let spec_val: Value = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();

        let run_input = serde_json::json!({
            "spec": spec_val,
            "background": false,
            "save_as": "test-flow"
        });

        let res = workflow_run_tool(dir.path(), &run_input, &context).unwrap();
        assert!(!res.is_error);
        assert!(res.content.contains("completed"));
        assert!(res.content.contains("test-flow.json"));

        // Status check for specific workflow
        let parsed_res: Value = serde_json::from_str(&res.content).unwrap();
        let wf_id = parsed_res["id"].as_str().unwrap();

        let status_input = serde_json::json!({ "workflow_id": wf_id });
        let status_res = workflow_status_tool(&status_input, &context).unwrap();
        assert!(!status_res.is_error);
        assert!(status_res.content.contains("investigate"));

        // Status list
        let list_res = workflow_status_tool(&serde_json::json!({}), &context).unwrap();
        assert!(!list_res.is_error);
        assert!(list_res.content.contains(wf_id));
    }

    #[test]
    fn test_workflow_save_and_run_untrusted_project_rejected() {
        let (context, dir) = setup_context(false); // untrusted!
        let spec_val: Value = serde_json::from_str(VALID_3_PHASE_WORKFLOW_JSON).unwrap();

        let run_input = serde_json::json!({
            "spec": spec_val,
            "save_as": "untrusted-flow"
        });

        let err = workflow_run_tool(dir.path(), &run_input, &context).unwrap_err();
        assert!(err.to_string().contains("untrusted checkout"));

        // Attempting to run by name from untrusted checkout
        let wf_dir = dir.path().join(".davinci").join("workflows");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("saved.json"), VALID_3_PHASE_WORKFLOW_JSON).unwrap();

        let load_input = serde_json::json!({ "name": "saved" });
        let err2 = workflow_run_tool(dir.path(), &load_input, &context).unwrap_err();
        assert!(err2.to_string().contains("requires project trust"));
    }
}
