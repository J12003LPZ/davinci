//! Model tools for persistent agent status, messaging, and stopping.

use serde_json::{json, Value};

use super::events::AgentState;
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
                        "description": "Optional agent name or ID (UUID) to inspect. If omitted, returns all agents in the run."
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
                        "description": "Recipient agent name (as given at spawn) or agent ID (UUID)."
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
                        "description": "Agent name or ID (UUID) to stop."
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
        let aid = super::team::resolve_agent(runtime, aid_str).map_err(ToolError::Failed)?;

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
            "failure_reason": record.failure_reason,
            "worktree": record.worktree.as_ref().map(|p| p.display().to_string()),
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

    let to = super::team::resolve_agent(runtime, to_str).map_err(ToolError::Failed)?;

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

    let aid = super::team::resolve_worker(runtime, aid_str).map_err(ToolError::Failed)?;

    let reason = input.get("reason").and_then(Value::as_str);

    // 1. Transition to Stopping (cooperative stop requested)
    runtime
        .registry
        .transition(aid, AgentState::Stopping)
        .map_err(|e| ToolError::Failed(format!("Failed to stop agent '{aid}': {e}")))?;

    let cancelled = runtime.team.cancel(&aid);

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
        "cancelled": cancelled,
    });

    Ok(ToolResult {
        content: format!("Agent '{aid}' transition requested to stopping state"),
        is_error: false,
        details: Some(details),
    })
}

#[cfg(test)]
mod tests {
    use super::super::ids::AgentId;
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
    #[test]
    fn agent_message_accepts_names_and_rejects_unknown_recipients() {
        let run_id = RunId::new();
        let lead = AgentId::new();
        let context = make_test_context(run_id, lead);
        let rt = context.runtime.as_ref().unwrap();
        rt.ensure_lead_registered("p", "m", std::path::Path::new("."));
        let mate = AgentId::new();
        rt.registry
            .register_agent(AgentRecord {
                id: mate,
                run_id,
                parent: Some(lead),
                kind: AgentKind::Teammate,
                name: "reviewer".into(),
                provider: String::new(),
                model_id: String::new(),
                cwd: PathBuf::from("."),
                state: AgentState::Starting,
                task_id: None,
                worktree: None,
                started_ms: 0,
                updated_ms: 0,
                failure_reason: None,
            })
            .unwrap();
        rt.registry.transition(mate, AgentState::Running).unwrap();

        let ok = agent_message_tool(&json!({"to": "reviewer", "message": "hi"}), &context).unwrap();
        assert!(!ok.is_error);
        assert_eq!(rt.mailbox.pending_count(&mate), 1);

        let err = agent_message_tool(
            &json!({"to": AgentId::new().to_string(), "message": "hi"}),
            &context,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn agent_stop_refuses_the_lead_and_itself() {
        let run_id = RunId::new();
        let lead = AgentId::new();
        let context = make_test_context(run_id, lead);
        let rt = context.runtime.as_ref().unwrap();
        rt.ensure_lead_registered("p", "m", std::path::Path::new("."));
        for target in ["lead".to_string(), lead.to_string()] {
            let err = agent_stop_tool(&json!({"agent_id": target}), &context).unwrap_err();
            assert!(err.to_string().contains("stop itself") || err.to_string().contains("lead"));
        }
        assert_eq!(rt.registry.get(&lead).unwrap().state, AgentState::Running);
    }

    #[test]
    fn agent_stop_cancels_the_worker_token() {
        let run_id = RunId::new();
        let lead = AgentId::new();
        let context = make_test_context(run_id, lead);
        let rt = context.runtime.as_ref().unwrap();
        let mate = AgentId::new();
        rt.registry
            .register_agent(AgentRecord {
                id: mate,
                run_id,
                parent: Some(lead),
                kind: AgentKind::Teammate,
                name: "worker".into(),
                provider: String::new(),
                model_id: String::new(),
                cwd: PathBuf::from("."),
                state: AgentState::Starting,
                task_id: None,
                worktree: None,
                started_ms: 0,
                updated_ms: 0,
                failure_reason: None,
            })
            .unwrap();
        rt.registry.transition(mate, AgentState::Running).unwrap();
        let token = rt.team.admit(mate);
        agent_stop_tool(&json!({"agent_id": "worker"}), &context).unwrap();
        assert!(token.is_cancelled());
        assert_eq!(rt.registry.get(&mate).unwrap().state, AgentState::Stopping);
    }
}
