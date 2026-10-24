//! Model tools for persistent agent status, messaging, and stopping.

use serde_json::{json, Value};

use super::events::AgentState;
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};

const DEFAULT_RESULTS_LIMIT: usize = 5;
const MAX_RESULTS_LIMIT: usize = 20;

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
                        "description": "Optional agent name or ID (UUID) to inspect. If omitted, returns all agents in the run with only their latest result's preview."
                    },
                    "results_offset": {
                        "type": "integer",
                        "description": "With agent_id: skip this many of the newest results (default 0)."
                    },
                    "results_limit": {
                        "type": "integer",
                        "description": "With agent_id: how many results to return, 1-20 (default 5)."
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

    runtime.team.retry_results(runtime);
    if let Some(aid_str) = input.get("agent_id").and_then(Value::as_str) {
        let aid = super::team::resolve_agent(runtime, aid_str).map_err(ToolError::Failed)?;

        let record = runtime
            .registry
            .get(&aid)
            .ok_or_else(|| ToolError::Failed(format!("Agent '{aid}' not found in registry")))?;

        let pending_messages = runtime.mailbox.pending_count(&aid);
        let number = |key: &str| input.get(key).and_then(Value::as_u64).map(|n| n as usize);
        let limit = number("results_limit")
            .unwrap_or(DEFAULT_RESULTS_LIMIT)
            .clamp(1, MAX_RESULTS_LIMIT);
        let page = runtime
            .team
            .result_page(&aid, number("results_offset").unwrap_or(0), limit);
        let older_in_journal = page.offset + page.items.len() < page.total;
        // Only the newest results stay in memory; the rest is not served here.
        let past_window = older_in_journal && page.offset + page.items.len() >= page.retained;
        let note = (past_window || page.total == 0).then_some(
            "Results are not held in memory (older, or this worker's window was evicted); \
             the full records are the AgentResultRecorded events in this session's \
             runtime log (<session>.runtime.jsonl).",
        );

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
            "results": {
                "total": page.total,
                "offset": page.offset,
                "limit": limit,
                "items": page.items,
                "older_in_journal": older_in_journal,
                "note": note,
            },
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
                let page = runtime.team.result_page(&r.id, 0, 1);
                let latest = page.items.first().map(|m| {
                    json!({
                        "id": m.id,
                        "sent_at_ms": m.sent_at_ms,
                        "preview": m.content.chars().take(200).collect::<String>(),
                    })
                });
                json!({
                    "agent_id": r.id.to_string(),
                    "name": r.name,
                    "kind": r.kind,
                    "state": r.state,
                    "model_id": r.model_id,
                    "pending_messages": pending,
                    "failure_reason": r.failure_reason,
                    "results": {"total": page.total, "latest": latest},
                })
            })
            .collect();

        let details = json!({ "agents": list, "workerResources": {
            "accounting": "estimated reservations; not an OS RSS cap",
            "usage": runtime.worker_admission.stats(),
        } });
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
    fn agent_status_summarizes_results_and_pages_them_for_one_agent() {
        use crate::runtime::mailbox::AgentMessage;
        let run_id = RunId::new();
        let lead = AgentId::new();
        let context = make_test_context(run_id, lead);
        let rt = context.runtime.as_ref().unwrap();
        rt.ensure_lead_registered("p", "m", std::path::Path::new("."));
        let worker = AgentId::new();
        rt.registry
            .register_agent(AgentRecord {
                id: worker,
                run_id: rt.run_id,
                parent: Some(rt.agent_id),
                kind: AgentKind::Teammate,
                name: "worker".into(),
                provider: "mock".into(),
                model_id: "mock-model".into(),
                cwd: PathBuf::from("/"),
                state: AgentState::Running,
                task_id: None,
                worktree: None,
                started_ms: 1,
                updated_ms: 1,
                failure_reason: None,
            })
            .unwrap();
        for n in 0..30 {
            let body = format!("result {n} {}", "x".repeat(400));
            rt.team.keep_result(
                AgentMessage::new(rt.run_id, worker, rt.agent_id, body),
                false,
            );
        }

        let all = agent_status_tool(&json!({}), &context)
            .unwrap()
            .details
            .unwrap();
        let entry = all["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "worker")
            .unwrap();
        assert_eq!(entry["results"]["total"], 30);
        let latest = &entry["results"]["latest"];
        assert!(latest["preview"].as_str().unwrap().starts_with("result 29"));
        assert_eq!(latest["preview"].as_str().unwrap().chars().count(), 200);
        assert!(latest.get("content").is_none() && entry["results"].get("items").is_none());

        let id = worker.to_string();
        let page = |extra: Value| {
            let mut input = json!({ "agent_id": id });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            agent_status_tool(&input, &context)
                .unwrap()
                .details
                .unwrap()["results"]
                .clone()
        };
        let first = page(json!({}));
        assert_eq!(first["items"].as_array().unwrap().len(), 5);
        assert_eq!(first["older_in_journal"], true);
        assert!(first["items"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("result 29"));
        let second = page(json!({"results_offset": 5, "results_limit": 2}));
        assert!(second["items"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("result 24"));
        assert_eq!(second["items"].as_array().unwrap().len(), 2);
        let clamped = page(json!({"results_limit": 500, "results_offset": 28}));
        assert_eq!(clamped["limit"], 20);
        assert_eq!(clamped["items"].as_array().unwrap().len(), 2);
        assert_eq!(clamped["older_in_journal"], false);
        let beyond = page(json!({"results_offset": 99_999, "results_limit": 0}));
        assert_eq!(beyond["limit"], 1);
        assert!(beyond["items"].as_array().unwrap().is_empty());
        assert_eq!(beyond["older_in_journal"], false);

        // Past the retained window the page is clamped and points at the journal.
        for n in 30..60 {
            rt.team.keep_result(
                AgentMessage::new(rt.run_id, worker, rt.agent_id, format!("result {n}")),
                false,
            );
        }
        let old = page(json!({"results_offset": 99_999}));
        assert_eq!(old["total"], 60);
        assert_eq!(old["offset"], 50);
        assert!(old["items"].as_array().unwrap().is_empty());
        assert_eq!(old["older_in_journal"], true);
        assert!(old["note"].as_str().unwrap().contains("runtime log"));
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
