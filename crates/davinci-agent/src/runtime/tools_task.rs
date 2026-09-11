//! Model tools for shared task management across collaborating agents.

use serde_json::{json, Value};
use std::str::FromStr;

use super::ids::{AgentId, TaskId};
use super::task_store::TaskCreateRequest;
use super::tasks::{TaskOwner, TaskRecord, TaskState};
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};

pub fn task_tool_specs() -> Vec<AgentTool> {
    vec![
        AgentTool {
            name: "task_create".to_string(),
            description: "Create a shared task with optional dependencies and self-assignment.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short task title (max 512 UTF-8 bytes)."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional detailed instructions or criteria (max 16,384 UTF-8 bytes)."
                    },
                    "dependencies": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of prerequisite Task IDs (UUIDs) that must complete before this task is ready."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Optional own Agent ID (UUID) for self-assignment."
                    },
                    "operation_id": {
                        "type": "string",
                        "description": "UUID identifying this creation. Reuse with identical fields to retrieve the original task ID and response."
                    }
                },
                "required": ["title", "operation_id"],
                "additionalProperties": false
            }),
        },
        AgentTool {
            name: "task_update".to_string(),
            description: "Claim an unowned ready task for yourself, or update your own task status. Claims and status changes are separate commands. Completion routes through verification gates.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (UUID) to update."
                    },
                    "expected_revision": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Current task revision from task_list; stale updates are rejected."
                    },
                    "operation_id": {
                        "type": "string",
                        "description": "UUID identifying this claim or status update. Reuse only with the same command and expected revision to retrieve its original committed response."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["completed", "failed", "cancelled"],
                        "description": "New status for the task."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Your own Agent ID (UUID) to claim an unowned ready task."
                    },
                    "result": {
                        "type": "string",
                        "description": "Optional completion result or error reason (max 65,536 UTF-8 bytes)."
                    }
                },
                "required": ["task_id", "expected_revision", "operation_id"],
                "additionalProperties": false
            }),
        },
        AgentTool {
            name: "task_get".to_string(),
            description: "Read one task in the current run, including bounded result and evidence references. This does not verify evidence or change task state.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "task_id": { "type": "string", "description": "Task UUID." } },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        },
        AgentTool {
            name: "task_list".to_string(),
            description: "List shared tasks in the current run in task-ID order. Pages reflect current state; concurrent changes can shift offsets.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["all", "ready", "pending", "running", "completed", "failed", "blocked", "cancelled"],
                        "description": "Optional internal state filter (defaults to 'all'); ready and pending retain distinct dependency readiness."
                    },
                    "public_status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "completed", "failed", "blocked", "cancelled"],
                        "description": "Optional board status filter; pending includes ready tasks. Intersects with other filters."
                    },
                    "assigned_to": {
                        "type": "string",
                        "description": "Optional filter by assigned Agent ID (UUID)."
                    },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 }
                },
                "additionalProperties": false
            }),
        },
    ]
}

fn task_runtime(context: &ToolContext) -> Result<&super::RuntimeHandle, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;
    if runtime.cancellation_token.is_cancelled() || context.is_aborted() {
        return Err(ToolError::Failed(
            "worker task authority is no longer active".into(),
        ));
    }
    if let Some(parent) = runtime.parent_agent_id {
        let active = runtime
            .registry
            .get(&runtime.agent_id)
            .is_some_and(|record| {
                record.run_id == runtime.run_id
                    && record.parent == Some(parent)
                    && record.state == super::AgentState::Running
            });
        if !active {
            return Err(ToolError::Failed(
                "worker task authority is no longer active".into(),
            ));
        }
    }
    Ok(runtime)
}

pub fn task_create_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    if let Some(coordinator) = &context.task_coordinator {
        return coordinator.call_with_abort("task_create", input, context.abort.as_deref());
    }
    let runtime = task_runtime(context)?;

    let title = input
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'title'".into()))?;

    let fields = input
        .as_object()
        .ok_or_else(|| ToolError::Failed("Expected task creation object".into()))?;
    if fields.keys().any(|key| {
        !matches!(
            key.as_str(),
            "title"
                | "description"
                | "dependencies"
                | "assigned_to"
                | "operation_id"
                | "decision_prerequisites"
        )
    }) {
        return Err(ToolError::Failed("Unknown task creation field".into()));
    }
    let optional_string = |key: &str| -> Result<Option<&str>, ToolError> {
        input
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| ToolError::Failed(format!("Expected string '{key}'")))
            })
            .transpose()
    };
    let operation_id = optional_string("operation_id")?
        .map(|id| {
            uuid::Uuid::parse_str(id.trim())
                .map_err(|_| ToolError::Failed("Invalid operation_id UUID".into()))
        })
        .transpose()?;
    let mut request = TaskCreateRequest {
        title: title.into(),
        ..TaskCreateRequest::default()
    };
    if operation_id.is_none() {
        return Err(ToolError::Failed(
            "Missing required field 'operation_id'".into(),
        ));
    }

    if let Some(desc) = optional_string("description")? {
        if desc.len() > 16_384 {
            return Err(ToolError::Failed(
                "Task description exceeds maximum 16KB".into(),
            ));
        }
        request.description = Some(desc.into());
    }

    if let Some(value) = input.get("dependencies") {
        let deps_arr = value
            .as_array()
            .ok_or_else(|| ToolError::Failed("Expected dependencies array".into()))?;
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
        request.dependencies = deps;
    }

    if let Some(value) = input.get("decision_prerequisites") {
        let prereqs_arr = value
            .as_array()
            .ok_or_else(|| ToolError::Failed("Expected decision_prerequisites array".into()))?;
        let mut prereqs = Vec::new();
        for item in prereqs_arr {
            let obj = item
                .as_object()
                .ok_or_else(|| ToolError::Failed("Expected decision prerequisite object".into()))?;
            let decision_id = obj
                .get("decision_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::Failed("Missing decision_id in prerequisite".into()))?;
            let expected_revision = obj
                .get("expected_revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ToolError::Failed("Missing expected_revision in prerequisite".into())
                })?;
            prereqs.push(super::tasks::DecisionPrerequisiteRef::new(
                decision_id,
                expected_revision,
            ));
        }
        request.decision_prerequisites = prereqs;
    }

    if let Some(agent_str) = optional_string("assigned_to")? {
        let aid = AgentId::from_str(agent_str.trim()).map_err(|e| {
            ToolError::Failed(format!("Invalid assigned_to AgentId '{agent_str}': {e}"))
        })?;
        request.assigned_to = Some(aid);
    }

    let created = runtime
        .task_registry
        .create_command(request, runtime.run_id, runtime.agent_id, operation_id)
        .map_err(|e| ToolError::Failed(format!("Failed to create task: {e}")))?;
    let task_id = created.id;

    let details = json!({
        "task_id": task_id.to_string(),
        "title": created.title,
        "revision": created.revision,
        "owner_generation": created.owner_generation,
        "state": created.state,
        "dependencies": created.dependencies.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "decision_prerequisites": created.decision_prerequisites,
        "assigned_to": created.assigned_to.map(|a| a.to_string()),
    });

    Ok(ToolResult {
        content: format!("Task '{task_id}' created with status '{:?}'", created.state),
        is_error: false,
        details: Some(details),
    })
}

pub fn task_update_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    if let Some(coordinator) = &context.task_coordinator {
        return coordinator.call_with_abort("task_update", input, context.abort.as_deref());
    }
    let runtime = task_runtime(context)?;

    let tid_str = input
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'task_id'".into()))?;

    let task_id = TaskId::from_str(tid_str.trim())
        .map_err(|e| ToolError::Failed(format!("Invalid task_id '{tid_str}': {e}")))?;

    let fields = input
        .as_object()
        .ok_or_else(|| ToolError::Failed("Expected task update object".into()))?;
    if fields.keys().any(|key| {
        !matches!(
            key.as_str(),
            "task_id" | "expected_revision" | "operation_id" | "assigned_to" | "status" | "result"
        )
    }) {
        return Err(ToolError::Failed("Unknown task update field".into()));
    }
    let revision = input
        .get("expected_revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::Failed("Expected unsigned integer 'expected_revision'".into()))?;
    let optional_string = |key: &str| -> Result<Option<&str>, ToolError> {
        input
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| ToolError::Failed(format!("Expected string '{key}'")))
            })
            .transpose()
    };
    let result_str = optional_string("result")?;
    if result_str.is_some_and(|s| s.len() > 65_536) {
        return Err(ToolError::Failed(
            "Task result exceeds 65,536 UTF-8 bytes".into(),
        ));
    }
    let assignment = optional_string("assigned_to")?;
    let operation_id = optional_string("operation_id")?
        .map(|id| {
            uuid::Uuid::parse_str(id.trim())
                .map_err(|_| ToolError::Failed("Invalid operation_id UUID".into()))
        })
        .transpose()?;
    let status = optional_string("status")?.map(|s| s.trim().to_ascii_lowercase());
    if operation_id.is_none() {
        return Err(ToolError::Failed(
            "Missing required field 'operation_id'".into(),
        ));
    }
    if let Some(status) = &status {
        if !matches!(status.as_str(), "completed" | "failed" | "cancelled") {
            return Err(ToolError::Failed(format!(
                "Unsupported status update '{status}'"
            )));
        }
    }
    if assignment.is_some() == status.is_some() || (assignment.is_some() && result_str.is_some()) {
        return Err(ToolError::Failed(
            "Provide either a claim or a status update in one command".into(),
        ));
    }
    if status.as_deref() == Some("cancelled") && result_str.is_some() {
        return Err(ToolError::Failed(
            "Cancellation does not accept a result".into(),
        ));
    }
    let current = runtime
        .task_registry
        .get_task(&task_id)
        .ok_or_else(|| ToolError::Failed(format!("Task '{task_id}' not found")))?;
    let owner = TaskOwner {
        run_id: runtime.run_id,
        agent_id: runtime.agent_id,
        revision,
        generation: current.owner_generation,
    };

    // Claims use the host identity; arbitrary reassignment requires a controller.
    let mut operation_response = None;
    if let Some(agent_str) = assignment {
        let aid = AgentId::from_str(agent_str.trim()).map_err(|e| {
            ToolError::Failed(format!("Invalid assigned_to AgentId '{agent_str}': {e}"))
        })?;
        if aid != runtime.agent_id {
            return Err(ToolError::Failed(
                "A task claim must name the calling agent".into(),
            ));
        }
        if let Some(operation_id) = operation_id {
            operation_response = Some(
                runtime
                    .task_registry
                    .claim_operation(
                        task_id,
                        runtime.run_id,
                        runtime.agent_id,
                        revision,
                        operation_id,
                    )
                    .map_err(|e| ToolError::Failed(format!("Failed to claim task: {e}")))?,
            );
        } else {
            runtime
                .task_registry
                .claim_task(task_id, runtime.run_id, runtime.agent_id, revision)
                .map_err(|e| ToolError::Failed(format!("Failed to claim task: {e}")))?;
        }
    }

    // Handle status transition
    if let Some(status_str) = status {
        if let Some(operation_id) = operation_id {
            let result = result_str.map(str::to_string);
            let response = if status_str == "completed" {
                runtime
                    .task_registry
                    .complete_operation(task_id, result, owner, operation_id)
            } else {
                let state = if status_str == "failed" {
                    TaskState::Failed
                } else {
                    TaskState::Cancelled
                };
                runtime
                    .task_registry
                    .status_operation(task_id, state, result, owner, operation_id)
            };
            operation_response =
                Some(response.map_err(|e| ToolError::Failed(format!("Task update refused: {e}")))?);
        } else {
            match status_str.as_str() {
                "completed" => {
                    runtime
                        .task_registry
                        .complete_owned_task(task_id, result_str.map(str::to_string), owner)
                        .map_err(|e| ToolError::Failed(format!("Task completion refused: {e}")))?;
                }
                "failed" => {
                    runtime
                        .task_registry
                        .fail_owned_task(task_id, result_str.map(str::to_string), owner)
                        .map_err(|e| ToolError::Failed(format!("Failed to fail task: {e}")))?;
                }
                "cancelled" => {
                    runtime
                        .task_registry
                        .cancel_owned_task(task_id, owner)
                        .map_err(|e| ToolError::Failed(format!("Failed to cancel task: {e}")))?;
                }
                other => {
                    return Err(ToolError::Failed(format!(
                    "Unsupported status update '{other}'. Valid options: completed, failed, cancelled"
                )));
                }
            }
        }
    }

    let updated = operation_response
        .or_else(|| runtime.task_registry.get_task(&task_id))
        .ok_or_else(|| ToolError::Failed(format!("Task '{task_id}' not found after update")))?;

    let details = json!({
        "task_id": task_id.to_string(),
        "title": updated.title,
        "revision": updated.revision,
        "owner_generation": updated.owner_generation,
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

pub fn task_get_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    if let Some(coordinator) = &context.task_coordinator {
        return coordinator.call_with_abort("task_get", input, context.abort.as_deref());
    }
    let fields = input
        .as_object()
        .ok_or_else(|| ToolError::Failed("Expected task get object".into()))?;
    if fields.keys().any(|key| key != "task_id") {
        return Err(ToolError::Failed("Unknown task get field".into()));
    }
    let id = input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(|id| TaskId::from_str(id.trim()).ok())
        .ok_or_else(|| ToolError::Failed("Expected task_id UUID".into()))?;
    let runtime = task_runtime(context)?;
    let task = runtime
        .task_registry
        .get_task(&id)
        .filter(|task| {
            runtime
                .task_registry
                .matches_run(runtime.run_id, task.run_id)
        })
        .ok_or_else(|| ToolError::Failed("Task not found in current run".into()))?;
    let mut details = task_summary(&task);
    details["description"] = json!(task
        .description
        .as_deref()
        .map(|value| bounded_text(value, 16_384)));
    details["description_truncated"] = json!(task
        .description
        .as_ref()
        .is_some_and(|value| value.len() > 16_384));
    details["evidence_refs"] = json!(task.evidence_refs.iter().take(128).collect::<Vec<_>>());
    details["evidence_refs_truncated"] = json!(task.evidence_refs.len() > 128);
    details["blocked_reasons"] = json!(task.blocked_reasons.iter().take(16).map(|reason| json!({"code": bounded_text(&reason.code, 128), "message": bounded_text(&reason.message, 512)})).collect::<Vec<_>>());
    details["blocked_reasons_truncated"] = json!(
        task.blocked_reasons.len() > 16
            || task
                .blocked_reasons
                .iter()
                .take(16)
                .any(|reason| reason.code.len() > 128 || reason.message.len() > 512)
    );
    details["created_at_ms"] = json!(task.created_at_ms);
    details["updated_at_ms"] = json!(task.updated_at_ms);
    details["attempt"] = json!(task.attempt);
    Ok(ToolResult {
        content: details.to_string(),
        is_error: false,
        details: Some(details),
    })
}

fn task_summary(task: &TaskRecord) -> Value {
    json!({
        "task_id": task.id.to_string(),
        "title": bounded_text(&task.title, 512),
        "title_truncated": task.title.len() > 512,
        "revision": task.revision,
        "owner_generation": task.owner_generation,
        "state": task.state,
        "public_status": task.state.public_status(),
        "dependencies": task.dependencies.iter().take(64).map(|id| id.to_string()).collect::<Vec<_>>(),
        "dependencies_truncated": task.dependencies.len() > 64,
        "assigned_to": task.assigned_to.map(|id| id.to_string()),
        "result": task.result.as_deref().map(|value| bounded_text(value, 4096)),
        "result_truncated": task.result.as_ref().is_some_and(|value| value.len() > 4096),
    })
}

pub fn task_list_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    if let Some(coordinator) = &context.task_coordinator {
        return coordinator.call_with_abort("task_list", input, context.abort.as_deref());
    }
    let fields = input
        .as_object()
        .ok_or_else(|| ToolError::Failed("Expected task list object".into()))?;
    if fields.keys().any(|key| {
        !matches!(
            key.as_str(),
            "status" | "public_status" | "assigned_to" | "limit" | "offset"
        )
    }) {
        return Err(ToolError::Failed("Unknown task list field".into()));
    }
    let page_number = |key: &str, default: usize| -> Result<usize, ToolError> {
        input
            .get(key)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| ToolError::Failed(format!("Invalid {key}")))
            })
            .unwrap_or(Ok(default))
    };
    let limit = page_number("limit", 50)?;
    let offset = page_number("offset", 0)?;
    if !(1..=100).contains(&limit) {
        return Err(ToolError::Failed("limit must be between 1 and 100".into()));
    }
    let runtime = task_runtime(context)?;

    let status_filter = input
        .get("status")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| ToolError::Failed("Invalid status filter".into()))
        })
        .transpose()?
        .unwrap_or("all")
        .to_ascii_lowercase();
    if !matches!(
        status_filter.as_str(),
        "all" | "ready" | "pending" | "running" | "completed" | "failed" | "blocked" | "cancelled"
    ) {
        return Err(ToolError::Failed("Invalid status filter".into()));
    }
    let public_filter = input
        .get("public_status")
        .map(|value| {
            value
                .as_str()
                .filter(|status| {
                    matches!(
                        *status,
                        "pending"
                            | "in_progress"
                            | "completed"
                            | "failed"
                            | "blocked"
                            | "cancelled"
                    )
                })
                .ok_or_else(|| ToolError::Failed("Invalid public_status filter".into()))
        })
        .transpose()?;
    let assigned_filter = input
        .get("assigned_to")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| ToolError::Failed("Invalid assigned_to filter".into()))
        })
        .transpose()?
        .map(|s| AgentId::from_str(s.trim()))
        .transpose()
        .map_err(|e| ToolError::Failed(format!("Invalid filter assigned_to AgentId: {e}")))?;

    let mut tasks = runtime.task_registry.list_tasks(Some(runtime.run_id));
    tasks.sort_by_key(|task| task.id.to_string());

    let filtered: Vec<_> = tasks
        .into_iter()
        .filter(|t| {
            if public_filter.is_some_and(|status| t.state.public_status() != status) {
                return false;
            }
            if let Some(target_agent) = assigned_filter {
                if t.assigned_to != Some(target_agent) {
                    return false;
                }
            }
            match status_filter.as_str() {
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
        .collect();
    let total = filtered.len();
    let page: Vec<Value> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|task| task_summary(&task))
        .collect();

    let count = page.len();
    let details = json!({
        "count": count,
        "tasks": page,
        "total": total,
        "next_offset": if offset < total && count < total - offset { Some(offset + count) } else { None },
    });

    Ok(ToolResult {
        content: serde_json::to_string_pretty(&details).unwrap_or_default(),
        is_error: false,
        details: Some(details),
    })
}

fn bounded_text(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::super::tasks::TaskRecord;

    #[test]
    fn f03_task_get_uses_persisted_migration_scope() {
        let dir = tempfile::tempdir().unwrap();
        let historical = TaskRecord::new(RunId::new(), "historical");
        let (registry, run) = crate::runtime::TaskRegistry::open_session_durable_with_legacy(
            dir.path().join("tasks.jsonl"),
            "session",
            vec![historical.clone()],
        )
        .unwrap();
        let mut context = make_test_context(run, AgentId::new());
        context.runtime.as_mut().unwrap().task_registry = registry;
        let input = json!({"task_id": historical.id.to_string()});
        let task = task_get_tool(&input, &context).unwrap().details.unwrap();
        assert_eq!(task["task_id"], historical.id.to_string());
        assert_eq!(
            context
                .runtime
                .as_ref()
                .unwrap()
                .task_registry
                .get_task(&historical.id),
            Some(historical.clone())
        );
        context.runtime.as_mut().unwrap().run_id = RunId::new();
        assert!(task_get_tool(&input, &context).is_err());
        assert_eq!(
            task_list_tool(&json!({}), &context)
                .unwrap()
                .details
                .unwrap()["total"],
            0
        );
    }

    #[test]
    fn f03_task_get_is_scoped_and_bounds_results() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        let own = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "own"))
            .unwrap();
        rt.task_registry
            .complete_task(own, Some("€".repeat(3000)))
            .unwrap();
        let foreign = rt
            .task_registry
            .create_task(TaskRecord::new(RunId::new(), "private"))
            .unwrap();
        let result = task_get_tool(&json!({"task_id": own.to_string()}), &context)
            .unwrap()
            .details
            .unwrap();
        assert_eq!(result["task_id"], own.to_string());
        assert_eq!(result["result"].as_str().unwrap().len(), 4095);
        assert_eq!(result["result_truncated"], true);
        assert_eq!(result["revision"], 2);
        let listed = task_list_tool(&json!({}), &context)
            .unwrap()
            .details
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["tasks"][0]["result"], result["result"]);
        assert_eq!(listed["tasks"][0]["result_truncated"], true);
        assert_eq!(
            rt.task_registry
                .get_task(&own)
                .unwrap()
                .result
                .unwrap()
                .len(),
            9000
        );
        for input in [
            json!({"task_id": foreign.to_string()}),
            json!({"task_id": "bad"}),
            json!({"task_id":own.to_string(),"run_id":rt.run_id.to_string()}),
        ] {
            assert!(task_get_tool(&input, &context).is_err());
        }
        assert_eq!(
            crate::permission::tool_class("task_get"),
            crate::permission::ToolClass::Read
        );
    }

    #[test]
    fn f03_task_list_pages_are_bounded_and_deterministic() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        for _ in 0..53 {
            rt.task_registry
                .create_task(TaskRecord::new(rt.run_id, "page"))
                .unwrap();
        }
        let first = task_list_tool(&json!({}), &context)
            .unwrap()
            .details
            .unwrap();
        assert_eq!(first["count"], 50);
        assert_eq!(first["total"], 53);
        assert_eq!(first["next_offset"], 50);
        assert_eq!(
            first,
            task_list_tool(&json!({}), &context)
                .unwrap()
                .details
                .unwrap()
        );
        let last = task_list_tool(&json!({"offset": 50}), &context)
            .unwrap()
            .details
            .unwrap();
        assert_eq!(last["count"], 3);
        assert!(last["next_offset"].is_null());
        for task in last["tasks"].as_array().unwrap() {
            assert!(!first["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|first| first["task_id"] == task["task_id"]));
        }
        let beyond = task_list_tool(&json!({"offset": u64::MAX}), &context);
        if usize::BITS == 64 {
            let beyond = beyond.unwrap().details.unwrap();
            assert_eq!(beyond["count"], 0);
            assert!(beyond["next_offset"].is_null());
        } else {
            assert!(beyond.is_err());
        }
        for input in [
            json!({"limit": 0}),
            json!({"limit": 101}),
            json!({"offset": -1}),
            json!({"status":"typo"}),
            json!({"assigned_to":false}),
            json!({"actor":"ignored"}),
        ] {
            assert!(task_list_tool(&input, &context).is_err(), "{input}");
        }
    }

    #[test]
    fn f03_task_commands_require_operation_ids() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        assert!(task_create_tool(&json!({"title": "missing operation"}), &context).is_err());
        let id = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "claim"))
            .unwrap();
        assert!(task_update_tool(&json!({"task_id": id.to_string(), "expected_revision": 1, "assigned_to": rt.agent_id.to_string()}), &context).is_err());
        assert!(rt
            .task_registry
            .get_task(&id)
            .unwrap()
            .assigned_to
            .is_none());
        for name in ["task_create", "task_update"] {
            let spec = task_tool_specs()
                .into_iter()
                .find(|spec| spec.name == name)
                .unwrap();
            assert!(spec.parameters["required"]
                .as_array()
                .unwrap()
                .contains(&json!("operation_id")));
        }
    }

    #[test]
    fn f03_tool_creation_rejects_malformed_fields_before_mutation() {
        let context = make_test_context(RunId::new(), AgentId::new());
        for input in [
            json!({"title":"bad", "operation_id": uuid::Uuid::new_v4().to_string(), "description": 1}),
            json!({"title":"bad", "operation_id": uuid::Uuid::new_v4().to_string(), "dependencies": "ignored"}),
            json!({"title":"bad", "operation_id": uuid::Uuid::new_v4().to_string(), "assigned_to": false}),
            json!({"title":"bad", "operation_id": uuid::Uuid::new_v4().to_string(), "assigned_to": AgentId::new().to_string()}),
            json!({"title":"bad", "operation_id": "invalid"}),
            json!({"title":"bad", "operation_id": uuid::Uuid::new_v4().to_string(), "actor": AgentId::new().to_string()}),
        ] {
            assert!(task_create_tool(&input, &context).is_err());
        }
        assert!(context
            .runtime
            .as_ref()
            .unwrap()
            .task_registry
            .list_tasks(None)
            .is_empty());
    }

    #[test]
    fn f03_tool_creation_replays_original_response() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let input =
            json!({"title": "retry creation", "operation_id": uuid::Uuid::new_v4().to_string()});
        let first = task_create_tool(&input, &context).unwrap();
        let second = task_create_tool(&input, &context).unwrap();
        assert_eq!(first.details, second.details);
        assert_eq!(first.content, second.content);
        assert_eq!(
            context
                .runtime
                .as_ref()
                .unwrap()
                .task_registry
                .list_tasks(None)
                .len(),
            1
        );
    }

    #[test]
    fn f03_tool_status_operation_replays_response() {
        for status in ["completed", "failed", "cancelled"] {
            let context = make_test_context(RunId::new(), AgentId::new());
            let rt = context.runtime.as_ref().unwrap();
            let id = rt
                .task_registry
                .create_task(TaskRecord::new(rt.run_id, status).with_assigned(rt.agent_id))
                .unwrap();
            let input = json!({"task_id": id.to_string(), "status": status, "expected_revision": 1, "operation_id": uuid::Uuid::new_v4().to_string()});
            let first = task_update_tool(&input, &context).unwrap();
            let replay = task_update_tool(&input, &context).unwrap();
            assert_eq!(first.content, replay.content);
            assert_eq!(first.details, replay.details);
            assert_eq!(rt.task_registry.get_task(&id).unwrap().revision, 2);
        }
    }

    #[test]
    fn f03_tool_claim_replay_returns_original_response() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        let id = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "retry"))
            .unwrap();
        let input = json!({"task_id": id.to_string(), "assigned_to": rt.agent_id.to_string(), "expected_revision": 1, "operation_id": uuid::Uuid::new_v4().to_string()});
        let original = task_update_tool(&input, &context).unwrap();
        rt.task_registry
            .complete_task(id, Some("later".into()))
            .unwrap();
        let replay = task_update_tool(&input, &context).unwrap();
        assert_eq!(original.content, replay.content);
        assert_eq!(original.details, replay.details);
        assert_eq!(original.is_error, replay.is_error);
    }

    #[test]
    fn f03_update_rejects_malformed_and_stale_commands() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        let id = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "owned").with_assigned(rt.agent_id))
            .unwrap();
        let before = rt.task_registry.get_task(&id).unwrap();
        for patch in [
            json!({"expected_revision": 0}),
            json!({"expected_revision": -1}),
            json!({"expected_revision": "1"}),
            json!({"status": 7}),
            json!({"result": 7}),
            json!({"result": format!("{}é", "x".repeat(65_535))}),
            json!({"actor": rt.agent_id.to_string()}),
            json!({"assigned_to": rt.agent_id.to_string()}),
        ] {
            let mut input = json!({"operation_id": uuid::Uuid::new_v4().to_string(), "task_id": id.to_string(), "status": "completed", "expected_revision": before.revision});
            input
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(task_update_tool(&input, &context).is_err());
            assert_eq!(rt.task_registry.get_task(&id), Some(before.clone()));
        }
        let result = "é".repeat(32_768);
        task_update_tool(&json!({"operation_id": uuid::Uuid::new_v4().to_string(), "task_id": id.to_string(), "status": "completed", "expected_revision": before.revision, "result": result}), &context).unwrap();
        assert_eq!(rt.task_registry.get_task(&id).unwrap().result, Some(result));
    }

    #[test]
    fn f03_update_rejects_forged_owner_without_mutation() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        let other = AgentId::new();
        let id = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "other").with_assigned(other))
            .unwrap();
        let before = rt.task_registry.get_task(&id).unwrap();
        for status in ["completed", "failed", "cancelled"] {
            assert!(task_update_tool(&json!({"operation_id": uuid::Uuid::new_v4().to_string(), "task_id": id.to_string(), "status": status, "expected_revision": before.revision}), &context).is_err());
            assert_eq!(rt.task_registry.get_task(&id), Some(before.clone()));
        }
    }

    #[test]
    fn f03_update_validates_before_assignment() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let rt = context.runtime.as_ref().unwrap();
        let id = rt
            .task_registry
            .create_task(TaskRecord::new(rt.run_id, "ready"))
            .unwrap();
        let before = rt.task_registry.get_task(&id).unwrap();
        assert!(task_update_tool(&json!({"operation_id": uuid::Uuid::new_v4().to_string(), "task_id": id.to_string(), "assigned_to": rt.agent_id.to_string(), "status": "invalid", "expected_revision": before.revision}), &context).is_err());
        assert_eq!(rt.task_registry.get_task(&id), Some(before));
    }

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
    fn f03_list_exposes_public_status_without_losing_ready_state() {
        let context = make_test_context(RunId::new(), AgentId::new());
        task_create_tool(
            &json!({ "operation_id": uuid::Uuid::new_v4().to_string(), "title": "ready" }),
            &context,
        )
        .unwrap();
        let listed = task_list_tool(&json!({"status": "ready"}), &context).unwrap();
        let row = &listed.details.unwrap()["tasks"][0];
        assert_eq!(row["state"], "ready");
        assert_eq!(row["public_status"], "pending");
    }

    #[test]
    fn f03_tool_uses_registry_title_byte_limit() {
        let context = make_test_context(RunId::new(), AgentId::new());
        assert!(task_create_tool(
            &json!({"operation_id": uuid::Uuid::new_v4().to_string(), "title": "é".repeat(256)}),
            &context
        )
        .is_ok());
        assert!(task_create_tool(
            &json!({"operation_id": uuid::Uuid::new_v4().to_string(), "title": "é".repeat(257)}),
            &context
        )
        .is_err());
        assert_eq!(
            context
                .runtime
                .as_ref()
                .unwrap()
                .task_registry
                .list_tasks(None)
                .len(),
            1
        );
    }

    #[test]
    fn f03_public_filters_preserve_internal_status_filters() {
        let context = make_test_context(RunId::new(), AgentId::new());
        let runtime = context.runtime.as_ref().unwrap();
        let ready = runtime
            .task_registry
            .create_task(TaskRecord::new(runtime.run_id, "ready"))
            .unwrap();
        let running = runtime
            .task_registry
            .create_task(TaskRecord::new(runtime.run_id, "running"))
            .unwrap();
        runtime
            .task_registry
            .assign_task(running, runtime.agent_id)
            .unwrap();
        runtime
            .task_registry
            .create_task(TaskRecord::new(runtime.run_id, "pending").with_dependencies(vec![ready]))
            .unwrap();
        for (filter, expected) in [
            (json!({"public_status":"pending"}), 2),
            (json!({"public_status":"in_progress"}), 1),
            (json!({"status":"pending"}), 1),
            (json!({"status":"ready"}), 1),
            (json!({"public_status":"completed"}), 0),
        ] {
            let result = task_list_tool(&filter, &context).unwrap();
            assert_eq!(result.details.unwrap()["count"], expected, "{filter}");
        }
        assert!(task_list_tool(&json!({"public_status":"typo"}), &context).is_err());
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
                "operation_id": uuid::Uuid::new_v4().to_string(),
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
                "operation_id": uuid::Uuid::new_v4().to_string(),
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

        // 4. Claim Task 1, then complete the owned revision.
        let claimed = task_update_tool(
            &json!({
                "task_id": t1_id,
                "assigned_to": agent_id.to_string(),
                "operation_id": uuid::Uuid::new_v4().to_string(),
                "expected_revision": 1,
            }),
            &context,
        )
        .unwrap();
        let update_res = task_update_tool(
            &json!({
                "task_id": t1_id,
                "status": "completed",
                "operation_id": uuid::Uuid::new_v4().to_string(),
                "expected_revision": claimed.details.as_ref().unwrap()["revision"],
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
