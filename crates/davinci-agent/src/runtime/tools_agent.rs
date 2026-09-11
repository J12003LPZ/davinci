//! Model tools for persistent agent status, messaging, and stopping.

use serde_json::{json, Value};
use std::str::FromStr;

use super::events::AgentState;
use super::ids::AgentId;
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};

pub fn agent_tool_specs() -> Vec<AgentTool> {
    vec![
        AgentTool {
            name: "agent_status".to_string(),
            description: "Inspect status, kind, model, and pending messages of subagents or teammates in the current run.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Optional agent ID (UUID) to inspect. If omitted, returns all agents in the run."
                    }
                }
            }),
        },
        AgentTool {
            name: "agent_message".to_string(),
            description: "Send an asynchronous message to another agent's mailbox (max 64KB).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Recipient agent ID (UUID)."
                    },
                    "message": {
                        "type": "string",
                        "description": "Content of the message (max 65,536 characters)."
                    }
                },
                "required": ["to", "message"]
            }),
        },
        AgentTool {
            name: "agent_stop".to_string(),
            description: "Request an agent to stop execution.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID (UUID) to stop."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional reason for stopping the agent."
                    }
                },
                "required": ["agent_id"]
            }),
        },
    ]
}

pub fn agent_status_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    if let Some(aid_str) = input.get("agent_id").and_then(Value::as_str) {
        let aid = AgentId::from_str(aid_str.trim())
            .map_err(|e| ToolError::Failed(format!("Invalid agent_id '{aid_str}': {e}")))?;

        let record = runtime
            .registry
            .get(&aid)
            .ok_or_else(|| ToolError::Failed(format!("Agent '{aid}' not found in registry")))?;

        let pending_messages = runtime.mailbox.pending_count(&aid);

        let details = json!({
            "agent_id": record.id.to_string(),
            "name": record.name,
            "kind": record.kind,
            "state": record.state,
            "provider": record.provider,
            "model_id": record.model_id,
            "cwd": record.cwd.display().to_string(),
            "started_ms": record.started_ms,
            "updated_ms": record.updated_ms,
            "pending_messages": pending_messages,
        });

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&details).unwrap_or_default(),
            is_error: false,
            details: Some(details),
        })
    } else {
        let records = runtime.registry.get_by_run(&runtime.run_id);
        let list: Vec<Value> = records
            .into_iter()
            .map(|r| {
                let pending = runtime.mailbox.pending_count(&r.id);
                json!({
                    "agent_id": r.id.to_string(),
                    "name": r.name,
                    "kind": r.kind,
                    "state": r.state,
                    "model_id": r.model_id,
                    "pending_messages": pending,
                })
            })
            .collect();

        let details = json!({ "agents": list });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&details).unwrap_or_default(),
            is_error: false,
            details: Some(details),
        })
    }
}

pub fn agent_message_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let to_str = input
        .get("to")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'to'".into()))?;

    let to = AgentId::from_str(to_str.trim())
        .map_err(|e| ToolError::Failed(format!("Invalid recipient agent_id '{to_str}': {e}")))?;

    let message = input
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'message'".into()))?;

    if message.len() > 65_536 {
        return Err(ToolError::Failed(
            "Message content exceeds 64KB limit".into(),
        ));
    }

    if let Some(record) = runtime.registry.get(&to) {
        if matches!(
            record.state,
            AgentState::Completed | AgentState::Failed | AgentState::Cancelled
        ) {
            return Err(ToolError::Failed(format!(
                "Recipient agent '{to}' is in terminal state {:?}",
                record.state
            )));
        }
    }

    let msg_id = runtime
        .send_message(to, message)
        .map_err(|e| ToolError::Failed(format!("Failed to deliver message: {e}")))?;

    let details = json!({
        "message_id": msg_id.to_string(),
        "to": to.to_string(),
        "status": "queued",
    });

    Ok(ToolResult {
        content: format!("Message delivered to queue for agent '{to}' (id: {msg_id})"),
        is_error: false,
        details: Some(details),
    })
}

pub fn agent_stop_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("Runtime subsystem not initialized".into()))?;

    let aid_str = input
        .get("agent_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing required field 'agent_id'".into()))?;

    let aid = AgentId::from_str(aid_str.trim())
        .map_err(|e| ToolError::Failed(format!("Invalid agent_id '{aid_str}': {e}")))?;

    let reason = input.get("reason").and_then(Value::as_str);

    // 1. Transition to Stopping (cooperative stop requested)
    runtime
        .registry
        .transition(aid, AgentState::Stopping)
        .map_err(|e| ToolError::Failed(format!("Failed to stop agent '{aid}': {e}")))?;

    // 2. Kill associated process tree if background jobs exist
    let killed_jobs = if let Ok(mut jobs) = context.jobs.lock() {
        jobs.kill_jobs_for_agent(&aid)
    } else {
        Vec::new()
    };

    // 3. Reject undelivered mailbox messages
    runtime
        .mailbox
        .reject_undelivered_for_cancelled(&aid, reason.unwrap_or("agent_stop_requested"));

    let status = crate::jobs::stop_status(true, false, false);

    let details = json!({
        "agent_id": aid.to_string(),
        "status": status,
        "reason": reason,
        "killed_jobs": killed_jobs,
    });

    Ok(ToolResult {
        content: format!("Agent '{aid}' transition requested to stopping state"),
        is_error: false,
        details: Some(details),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::RuntimeBus;
    use crate::runtime::events::{AgentKind, AgentRecord};
    use crate::runtime::ids::RunId;
    use crate::runtime::RuntimeHandle;
    use std::path::PathBuf;

    fn make_test_context(run_id: RunId, agent_id: AgentId) -> ToolContext {
        let bus = RuntimeBus::new();
        let rt = RuntimeHandle::new(run_id, agent_id, bus);
        ToolContext {
            runtime: Some(rt),
            ..ToolContext::default()
        }
    }

    #[test]
    fn test_agent_status_and_messaging() {
        let run_id = RunId::new();
        let agent1 = AgentId::new();
        let agent2 = AgentId::new();
        let context = make_test_context(run_id, agent1);
        let rt = context.runtime.as_ref().unwrap();

        rt.registry
            .register_agent(AgentRecord {
                id: agent1,
                run_id,
                parent: None,
                kind: AgentKind::Main,
                name: "lead".into(),
                provider: "mock".into(),
                model_id: "mock-model".into(),
                cwd: PathBuf::from("/"),
                state: AgentState::Running,
                task_id: None,
                worktree: None,
                started_ms: 100,
                updated_ms: 100,
                failure_reason: None,
            })
            .unwrap();

        rt.registry
            .register_agent(AgentRecord {
                id: agent2,
                run_id,
                parent: Some(agent1),
                kind: AgentKind::Teammate,
                name: "worker".into(),
                provider: "mock".into(),
                model_id: "mock-model".into(),
                cwd: PathBuf::from("/"),
                state: AgentState::Idle,
                task_id: None,
                worktree: None,
                started_ms: 100,
                updated_ms: 100,
                failure_reason: None,
            })
            .unwrap();

        // 1. Inspect all agents
        let res_all = agent_status_tool(&json!({}), &context).unwrap();
        assert!(!res_all.is_error);
        assert!(res_all.content.contains("lead"));
        assert!(res_all.content.contains("worker"));

        // 2. Inspect single agent
        let res_single =
            agent_status_tool(&json!({ "agent_id": agent2.to_string() }), &context).unwrap();
        assert!(!res_single.is_error);
        assert!(res_single.content.contains("worker"));

        // 3. Send message to agent2 (idle -> wakes to running)
        let msg_res = agent_message_tool(
            &json!({
                "to": agent2.to_string(),
                "message": "Start working on task 1"
            }),
            &context,
        )
        .unwrap();
        assert!(!msg_res.is_error);
        assert!(msg_res.content.contains("Message delivered"));

        let rec2 = rt.registry.get(&agent2).unwrap();
        assert_eq!(rec2.state, AgentState::Running);

        // 4. Stop agent2
        let stop_res = agent_stop_tool(
            &json!({
                "agent_id": agent2.to_string(),
                "reason": "Task completed"
            }),
            &context,
        )
        .unwrap();
        assert!(!stop_res.is_error);
        let rec2_stopped = rt.registry.get(&agent2).unwrap();
        assert_eq!(rec2_stopped.state, AgentState::Stopping);
    }
}
