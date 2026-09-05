//! Model tools for shared task management across collaborating agents.

use serde_json::{json, Value};
use std::str::FromStr;

use super::ids::{AgentId, TaskId};
use super::tasks::{TaskRecord, TaskState};
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};

pub fn task_tool_specs() -> Vec<AgentTool> {
    vec![
        AgentTool {
            name: "task_create".to_string(),
            description: "Create a shared task with optional dependencies and agent assignment.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short task title (max 256 characters)."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional detailed instructions or criteria (max 16,384 characters)."
                    },
                    "dependencies": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of prerequisite Task IDs (UUIDs) that must complete before this task is ready."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Optional Agent ID (UUID) to assign this task to."
                    }
                },
                "required": ["title"]
            }),
        },
        AgentTool {
            name: "task_update".to_string(),
            description: "Update task assignment, result, or status. Setting status to 'completed' routes through verification gates.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (UUID) to update."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["completed", "failed", "cancelled"],
                        "description": "New status for the task."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Optional Agent ID (UUID) to reassign the task."
                    },
                    "result": {
                        "type": "string",
                        "description": "Optional completion result or error reason (max 65,536 characters)."
                    }
                },
                "required": ["task_id"]
            }),
        },
        AgentTool {
            name: "task_list".to_string(),
            description: "List shared tasks in the current run, optionally filtered by status or assigned agent.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["all", "ready", "pending", "running", "completed", "failed", "blocked", "cancelled"],
                        "description": "Optional status filter (defaults to 'all')."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Optional filter by assigned Agent ID (UUID)."
                    }
                }
            }),
        },
    ]
}

pub fn task_create_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let title = input
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'title'".into()))?;

    if title.len() > 256 {
        return Err(ToolError::Failed(
            "Task title exceeds maximum 256 characters".into(),
        ));
    }

    let mut record = TaskRecord::new(runtime.run_id, title);

    if let Some(desc) = input.get("description").and_then(Value::as_str) {
        if desc.len() > 16_384 {
            return Err(ToolError::Failed(
                "Task description exceeds maximum 16KB".into(),
            ));
        }
        record = record.with_description(desc);
    }

    if let Some(deps_arr) = input.get("dependencies").and_then(Value::as_array) {
        let mut deps = Vec::new();
        for dep_val in deps_arr {
            let dep_str = dep_val.as_str().ok_or_else(|| {
                ToolError::Failed("Invalid dependency: expected string UUID".into())
            })?;
            let dep_id = TaskId::from_str(dep_str.trim()).map_err(|e| {
                ToolError::Failed(format!("Invalid dependency TaskId '{dep_str}': {e}"))
            })?;
            deps.push(dep_id);
        }
        record = record.with_dependencies(deps);
    }

    if let Some(agent_str) = input.get("assigned_to").and_then(Value::as_str) {
        let aid = AgentId::from_str(agent_str.trim()).map_err(|e| {
            ToolError::Failed(format!("Invalid assigned_to AgentId '{agent_str}': {e}"))
        })?;
        record = record.with_assigned(aid);
    }

    let task_id = runtime
        .task_registry
        .create_task(record)
        .map_err(|e| ToolError::Failed(format!("Failed to create task: {e}")))?;

    let created = runtime
        .task_registry
        .get_task(&task_id)
        .ok_or_else(|| ToolError::Failed("Failed to fetch created task record".into()))?;

    let details = json!({
        "task_id": task_id.to_string(),
        "title": created.title,
        "state": created.state,
        "dependencies": created.dependencies.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "assigned_to": created.assigned_to.map(|a| a.to_string()),
    });

    Ok(ToolResult {
        content: format!("Task '{task_id}' created with status '{:?}'", created.state),
        is_error: false,
        details: Some(details),
    })
}

pub fn task_update_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let tid_str = input
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'task_id'".into()))?;

    let task_id = TaskId::from_str(tid_str.trim())
        .map_err(|e| ToolError::Failed(format!("Invalid task_id '{tid_str}': {e}")))?;

    let result_str = input.get("result").and_then(Value::as_str).map(|s| {
        if s.len() > 65_536 {
            &s[..65_536]
        } else {
            s
        }
    });

    // Handle reassignment if requested
    if let Some(agent_str) = input.get("assigned_to").and_then(Value::as_str) {
        let aid = AgentId::from_str(agent_str.trim()).map_err(|e| {
            ToolError::Failed(format!("Invalid assigned_to AgentId '{agent_str}': {e}"))
        })?;
        runtime
            .task_registry
            .assign_task(task_id, aid)
            .map_err(|e| ToolError::Failed(format!("Failed to assign task: {e}")))?;
    }

    // Handle status transition
    if let Some(status_str) = input.get("status").and_then(Value::as_str) {
        match status_str.trim().to_ascii_lowercase().as_str() {
            "completed" => {
                runtime
                    .task_registry
                    .complete_task(task_id, result_str.map(str::to_string))
                    .map_err(|e| ToolError::Failed(format!("Task completion refused: {e}")))?;
            }
            "failed" => {
                runtime
                    .task_registry
                    .fail_task(task_id, result_str.map(str::to_string))
                    .map_err(|e| ToolError::Failed(format!("Failed to fail task: {e}")))?;
            }
            "cancelled" => {
                runtime
                    .task_registry
                    .cancel_task(task_id)
                    .map_err(|e| ToolError::Failed(format!("Failed to cancel task: {e}")))?;
            }
            other => {
                return Err(ToolError::Failed(format!(
                    "Unsupported status update '{other}'. Valid options: completed, failed, cancelled"
                )));
            }
        }
    }

    let updated = runtime
        .task_registry
        .get_task(&task_id)
        .ok_or_else(|| ToolError::Failed(format!("Task '{task_id}' not found after update")))?;

    let details = json!({
        "task_id": task_id.to_string(),
        "title": updated.title,
        "state": updated.state,
        "assigned_to": updated.assigned_to.map(|a| a.to_string()),
        "result": updated.result,
        "updated_at_ms": updated.updated_at_ms,
    });

    Ok(ToolResult {
        content: format!("Task '{task_id}' updated to status '{:?}'", updated.state),
        is_error: false,
        details: Some(details),
    })
}

pub fn task_list_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let status_filter = input.get("status").and_then(Value::as_str).unwrap_or("all");
    let assigned_filter = input
        .get("assigned_to")
        .and_then(Value::as_str)
        .map(|s| AgentId::from_str(s.trim()))
        .transpose()
        .map_err(|e| ToolError::Failed(format!("Invalid filter assigned_to AgentId: {e}")))?;

    let tasks = runtime.task_registry.list_tasks(Some(runtime.run_id));

    let filtered: Vec<Value> = tasks
        .into_iter()
        .filter(|t| {
            if let Some(target_agent) = assigned_filter {
                if t.assigned_to != Some(target_agent) {
                    return false;
                }
            }
            match status_filter.to_ascii_lowercase().as_str() {
                "all" => true,
                "ready" => t.state == TaskState::Ready,
                "pending" => t.state == TaskState::Pending,
                "running" => t.state == TaskState::Running,
                "completed" => t.state == TaskState::Completed,
                "failed" => t.state == TaskState::Failed,
                "blocked" => t.state == TaskState::Blocked,
                "cancelled" => t.state == TaskState::Cancelled,
                _ => true,
            }
        })
        .map(|t| {
            json!({
                "task_id": t.id.to_string(),
                "title": t.title,
                "state": t.state,
                "dependencies": t.dependencies.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                "assigned_to": t.assigned_to.map(|a| a.to_string()),
                "result": t.result,
            })
        })
        .collect();

    let count = filtered.len();
    let details = json!({
        "count": count,
        "tasks": filtered,
    });

    Ok(ToolResult {
        content: serde_json::to_string_pretty(&details).unwrap_or_default(),
        is_error: false,
        details: Some(details),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::RuntimeBus;
    use crate::runtime::ids::RunId;
    use crate::runtime::RuntimeHandle;

    fn make_test_context(run_id: RunId, agent_id: AgentId) -> ToolContext {
        let bus = RuntimeBus::new();
        let rt = RuntimeHandle::new(run_id, agent_id, bus);
        ToolContext {
            runtime: Some(rt),
            ..ToolContext::default()
        }
    }

    #[test]
    fn test_task_tools_lifecycle() {
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let context = make_test_context(run_id, agent_id);

        // 1. Create Task 1
        let res1 = task_create_tool(
            &json!({
                "title": "Build user auth",
                "description": "Implement authentication service",
            }),
            &context,
        )
        .unwrap();
        assert!(!res1.is_error);
        let t1_id = res1.details.as_ref().unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();

        // 2. Create Task 2 depending on Task 1
        let res2 = task_create_tool(
            &json!({
                "title": "Build user profile page",
                "dependencies": [t1_id],
                "assigned_to": agent_id.to_string(),
            }),
            &context,
        )
        .unwrap();
        assert!(!res2.is_error);
        let t2_id = res2.details.as_ref().unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();

        // 3. List tasks
        let list_res = task_list_tool(&json!({ "status": "all" }), &context).unwrap();
        assert_eq!(list_res.details.as_ref().unwrap()["count"], 2);

        // 4. Update Task 1 to completed
        let update_res = task_update_tool(
            &json!({
                "task_id": t1_id,
                "status": "completed",
                "result": "Auth module passed all tests",
            }),
            &context,
        )
        .unwrap();
        assert!(!update_res.is_error);

        // 5. Now Task 2 should be ready!
        let list_ready = task_list_tool(&json!({ "status": "ready" }), &context).unwrap();
        assert_eq!(list_ready.details.as_ref().unwrap()["count"], 1);
        assert_eq!(
            list_ready.details.as_ref().unwrap()["tasks"][0]["task_id"],
            t2_id
        );
    }
}
