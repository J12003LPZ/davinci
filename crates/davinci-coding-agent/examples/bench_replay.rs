//! Offline benchmark replay. This executable never constructs a live provider.

use davinci_agent::{Agent, AgentEvent, CustomToolExecutor, EventSink, PermissionMode, ToolError};
use davinci_ai::{AssistantMessage, MessageContent};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
struct Frame {
    assistant: AssistantMessage,
    reminders_before: Vec<String>,
    results_before: Vec<RecordedResult>,
}

#[derive(Deserialize)]
struct RecordedResult {
    id: String,
    is_error: bool,
}

#[derive(Deserialize)]
struct ReplayInput {
    prompt: String,
    frames: Vec<Frame>,
}

fn replay(input: ReplayInput, cwd: &Path) -> Result<Value, String> {
    if input.frames.is_empty() || input.frames.len() > 200 {
        return Err("replay requires 1..=200 recorded responses".into());
    }
    let mut agent = Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
    // Keep normal Windows drive syntax: cmd.exe cannot use a verbatim UNC cwd.
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("replay requires an existing absolute working directory".into());
    }
    agent.cwd = cwd.to_path_buf();
    let state = cwd.join(".davinci");
    std::fs::create_dir_all(&state).map_err(|error| error.to_string())?;
    // The CLI stores native caches in its separate agent directory. Keeping
    // them under cwd makes concurrent grep race cache-file atomic replacement.
    let runtime_state = tempfile::tempdir().map_err(|error| error.to_string())?;
    let cache = davinci_agent::runtime::cache::CacheRuntime::shared(
        Default::default(),
        runtime_state.path().to_path_buf(),
    );
    let build =
        davinci_coding_agent::native_extensions::build_intelligence::BuildIntelligence::with_root(
            cwd, cache,
        );
    let repo = davinci_coding_agent::native_extensions::repo_intelligence::RepoIntelligence::new(
        cwd,
        runtime_state.path(),
        Default::default(),
    );
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_, name, args| match name {
        "workspace_packages" => build.execute_tool(name, args),
        "repo_map" => repo.execute_tool(name, args),
        _ => Err(ToolError::Unknown(name.to_owned())),
    }));
    for name in ["workspace_packages", "repo_map"] {
        agent.tools.push(name.into());
        agent.tool_registry.push(name.into());
        agent
            .tool_context
            .authorized_tools
            .lock()
            .unwrap()
            .insert(name.into());
        agent
            .tool_context
            .tool_exposure
            .lock()
            .unwrap()
            .activate_authorized(name, true);
    }
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-6-luna".into();
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.auto_retry = false;
    agent.auto_compaction = false;
    agent.install_telemetry = false;
    agent.max_model_turns = Some(201);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    agent.event_sink = Some(EventSink(Arc::new(move |event| {
        captured.lock().unwrap().push(event.clone());
    })));
    agent.prompt(&input.prompt);
    let mut provided = 0;
    let mut requested = 0;
    let mut stop_reason = None;
    let mut divergence = None;
    let outcome = agent.run_loop(|current: &Agent| -> Result<AssistantMessage, String> {
        requested += 1;
        let Some(frame) = input.frames.get(provided) else {
            stop_reason = Some("recording_exhausted");
            return Err("recording_exhausted".into());
        };
        let reminders: Vec<String> = current
            .messages
            .iter()
            .filter_map(|message| {
                message
                    .extra
                    .get("davinciCapabilityReminder")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        if reminders != frame.reminders_before {
            stop_reason = Some("reminder_context_diverged");
            return Err("reminder_context_diverged".into());
        }
        let observed_events = observed.lock().map_err(|_| "poisoned replay observation")?;
        let mut actual: Vec<_> = observed_events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    is_error,
                    ..
                } => Some((tool_call_id.as_str(), *is_error)),
                _ => None,
            })
            .collect();
        let mut expected: Vec<_> = frame
            .results_before
            .iter()
            .map(|result| (result.id.as_str(), result.is_error))
            .collect();
        // Independent tool completions may arrive in a different order.
        actual.sort_unstable();
        expected.sort_unstable();
        if actual != expected {
            divergence = Some(json!({"actual": actual, "expected": expected}));
            stop_reason = Some("tool_outcome_diverged");
            return Err("tool_outcome_diverged".into());
        }
        provided += 1;
        Ok(frame.assistant.clone())
    });
    let events = observed.lock().map_err(|_| "poisoned replay observation")?;
    let mut reasons = std::collections::BTreeMap::<String, usize>::new();
    let mut auto_ids = std::collections::HashSet::new();
    let mut auto_runs = 0;
    let mut tool_calls = 0;
    let mut failed_tools = 0;
    let mut failures = Vec::new();
    let executed_leaf_operations = events
        .iter()
        .filter_map(|event| {
            serde_json::to_value(event)
                .ok()?
                .get("executed_leaf_operations")?
                .as_u64()
        })
        .max();
    let mutation_observations: Vec<_> = events
        .iter()
        .filter_map(|event| {
            let value = serde_json::to_value(event).ok()?;
            (value.get("type")?.as_str()? == "mutation_observation")
                .then(|| value["generation"].clone())
        })
        .collect();
    for event in events.iter() {
        match event {
            AgentEvent::MessageEnd { message } => {
                if let Some(reason) = message
                    .extra
                    .get("davinciCapabilityReminder")
                    .and_then(Value::as_str)
                {
                    *reasons.entry(reason.to_owned()).or_default() += 1;
                }
                if message.extra.get("davinciHarnessVerification") == Some(&Value::Bool(true)) {
                    for block in &message.content {
                        if let MessageContent::ToolCall { id, .. } = block {
                            auto_ids.insert(id.to_owned());
                        }
                    }
                }
            }
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                if auto_ids.contains(tool_call_id) {
                    auto_runs += 1;
                } else {
                    tool_calls += 1;
                }
            }
            AgentEvent::ToolExecutionEnd {
                is_error: true,
                tool_call_id,
                tool_name,
                result,
                ..
            } => {
                failed_tools += 1;
                failures.push(json!({"id": tool_call_id, "name": tool_name, "result": result}));
            }
            _ => {}
        }
    }
    Ok(json!({"schema_version": 1, "provided_requests": provided,
        "requested_requests": requested, "dropped_recorded_requests": input.frames.len() - provided,
        "completed": outcome.is_ok() && stop_reason.is_none(), "stop_reason": stop_reason,
        "divergence": divergence,
        "gate_reminders": reasons, "auto_verify_runs": auto_runs, "tool_calls": tool_calls,
        "failed_tools": failed_tools,
        "failures": failures,
        "mutation_observations": mutation_observations,
        "executed_leaf_operations": executed_leaf_operations,
        "mutation_generation": agent.mutation_verification_state().mutation_generation,
        "scope": "offline harness replay, not a model performance prediction"}))
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: bench_replay <public-recording.json> <fresh-public-workdir>".into());
    }
    let bytes = std::fs::read(&args[0]).map_err(|error| error.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("recording exceeds 16 MiB".into());
    }
    let input = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    println!("{}", replay(input, Path::new(&args[1]))?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn final_frame(id: &str) -> Frame {
        Frame {
            reminders_before: vec![],
            results_before: vec![],
            assistant: serde_json::from_value(serde_json::json!({
                "id": id, "role": "assistant", "model": "fixture",
                "content": [{"type": "text", "text": "done"}], "stopReason": "stop"
            }))
            .unwrap(),
        }
    }

    #[test]
    fn native_tool_cache_stays_outside_the_public_repository() {
        let directory = tempfile::tempdir().unwrap();
        let call = Frame {
            reminders_before: vec![],
            results_before: vec![],
            assistant: serde_json::from_value(json!({
                "id": "inspect", "role": "assistant", "model": "fixture",
                "content": [{"type": "toolCall", "id": "packages", "name": "workspace_packages",
                    "arguments": {}}], "stopReason": "toolUse"
            }))
            .unwrap(),
        };
        let mut done = final_frame("done");
        done.results_before.push(RecordedResult {
            id: "packages".into(),
            is_error: false,
        });
        let result = replay(
            ReplayInput {
                prompt: "Inspect packages".into(),
                frames: vec![call, done],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(result["completed"], true);
        assert!(!directory.path().join(".davinci/cache-runtime").exists());
    }

    #[test]
    fn replay_ends_at_earlier_final_without_consuming_later_turns() {
        let directory = tempfile::tempdir().unwrap();
        let result = replay(
            ReplayInput {
                prompt: "Say done".into(),
                frames: vec![final_frame("first"), final_frame("later")],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(result["provided_requests"], 1);
        assert_eq!(result["dropped_recorded_requests"], 1);
        assert_eq!(result["completed"], true);
    }

    #[test]
    fn replay_refuses_a_different_reminder_context() {
        let directory = tempfile::tempdir().unwrap();
        let mut frame = final_frame("first");
        frame.reminders_before.push("verification_required".into());
        let result = replay(
            ReplayInput {
                prompt: "Say done".into(),
                frames: vec![frame],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(result["provided_requests"], 0);
        assert_eq!(result["completed"], false);
        assert_eq!(result["stop_reason"], "reminder_context_diverged");
    }

    #[test]
    fn replay_executes_tools_in_the_fresh_repository() {
        let directory = tempfile::tempdir().unwrap();
        let write = Frame {
            reminders_before: vec![],
            results_before: vec![],
            assistant: serde_json::from_value(json!({
                "id": "edit", "role": "assistant", "model": "fixture",
                "content": [{"type": "toolCall", "id": "write1", "name": "write",
                    "arguments": {"path": "example.py", "content": "value = 1\n"}}],
                "stopReason": "toolUse"
            }))
            .unwrap(),
        };
        let mut final_response = final_frame("done");
        final_response.results_before.push(RecordedResult {
            id: "write1".into(),
            is_error: false,
        });
        let result = replay(
            ReplayInput {
                prompt: "Write example.py".into(),
                frames: vec![write, final_response],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("example.py")).unwrap(),
            "value = 1\n"
        );
        assert_eq!(result["tool_calls"], 1);
        assert_eq!(result["failed_tools"], 0);
        assert_eq!(result["mutation_generation"], 1);
        assert_eq!(result["mutation_observations"], json!([0, 1]));
        assert_eq!(result["executed_leaf_operations"], 1);
    }

    #[test]
    fn replay_refuses_changed_tool_outcomes() {
        let directory = tempfile::tempdir().unwrap();
        let mut frame = final_frame("done");
        frame.results_before.push(RecordedResult {
            id: "missing".into(),
            is_error: false,
        });
        let result = replay(
            ReplayInput {
                prompt: "Done".into(),
                frames: vec![frame],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(result["provided_requests"], 0);
        assert_eq!(result["completed"], false);
        assert_eq!(result["stop_reason"], "tool_outcome_diverged");
    }

    #[test]
    fn replay_accepts_independent_tool_completion_order() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("public.txt"), "fixture").unwrap();
        let mut calls = final_frame("reads");
        calls.assistant = serde_json::from_value(json!({
            "id": "reads", "role": "assistant", "model": "fixture",
            "content": [
                {"type": "toolCall", "id": "a", "name": "read", "arguments": {"path": "public.txt"}},
                {"type": "toolCall", "id": "b", "name": "read", "arguments": {"path": "public.txt"}}
            ], "stopReason": "toolUse"
        })).unwrap();
        let mut done = final_frame("done");
        done.results_before = vec![
            RecordedResult {
                id: "b".into(),
                is_error: false,
            },
            RecordedResult {
                id: "a".into(),
                is_error: false,
            },
        ];
        let result = replay(
            ReplayInput {
                prompt: "Read public.txt".into(),
                frames: vec![calls, done],
            },
            directory.path(),
        )
        .unwrap();
        assert_eq!(result["completed"], true);
        assert_eq!(result["provided_requests"], 2);
    }
}
