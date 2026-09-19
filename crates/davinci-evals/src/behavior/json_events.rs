//! Normalization of product JSONL output into the evaluator's trace schema.

use super::trace::{BehaviorEvent, BehaviorStats, BehaviorTrace};
use davinci_agent::prompt::manifest::PromptManifest;
use davinci_agent::AgentEvent;
use davinci_ai::ChatMessage;
use serde_json::{json, Value};

/// Convert the JSONL emitted by `davinci --mode json` into a deterministic
/// behavior trace. Unknown event types are ignored so extensions can add
/// informational events without making old evaluators unusable; malformed
/// JSON and malformed recognized events fail closed.
pub fn trace_from_json_lines(scenario_id: &str, lines: &[String]) -> Result<BehaviorTrace, String> {
    let mut agent_events = Vec::new();
    let mut supplemental_events = Vec::new();
    let mut manifest = None;
    let mut stats = BehaviorStats::default();

    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("invalid JSON event on line {}: {error}", index + 1))?;
        collect_stats(&mut stats, &value);

        if let Some(found) = find_prompt_manifest(&value) {
            supplemental_events.push(BehaviorEvent::PromptProfile {
                profile: found.profile.clone(),
            });
            for module in &found.modules {
                if let Some(capability) = module.id.strip_prefix("capability.") {
                    supplemental_events.push(BehaviorEvent::CapabilityIdentity {
                        capability: capability.to_string(),
                    });
                }
            }
            manifest = Some(found);
        }

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if let Ok(event) = serde_json::from_value::<AgentEvent>(value.clone()) {
            add_lifecycle_event(&mut supplemental_events, event_type);
            agent_events.push(event);
            continue;
        }

        if let Some(events) = synthetic_agent_events(&value, index)? {
            agent_events.extend(events);
            add_lifecycle_event(&mut supplemental_events, event_type);
        }

        match event_type {
            "permission_ask" | "permission_asked" | "permission_request" | "approval_request" => {
                let tool = string_field(&value, &["tool", "toolName", "name"])
                    .unwrap_or_else(|| "unknown".into());
                stats.permission_prompts += 1;
                supplemental_events.push(BehaviorEvent::PermissionAsked { tool });
            }
            "permission_denied" | "permission_denial" | "approval_denied" => {
                let tool = string_field(&value, &["tool", "toolName", "name"])
                    .unwrap_or_else(|| "unknown".into());
                supplemental_events.push(BehaviorEvent::PermissionDenied { tool });
            }
            "plan" | "plan_event" | "plan_start" | "plan_update" | "plan_end" => {
                supplemental_events.push(BehaviorEvent::PlanEvent {
                    kind: string_field(&value, &["phase", "event", "status"])
                        .unwrap_or_else(|| event_type.to_string()),
                });
            }
            "workflow_step" | "engineering_step" => {
                let step = string_field(&value, &["step", "workflowStep", "workflow_step"])
                    .ok_or_else(|| {
                        format!(
                            "recognized workflow-step event on line {} has no step",
                            index + 1
                        )
                    })?;
                supplemental_events.push(BehaviorEvent::PlanEvent {
                    kind: format!("workflow:{step}"),
                });
            }
            "capability" | "capability_identity" | "capability_activated" => {
                let capability =
                    string_field(&value, &["capability", "id", "name"]).ok_or_else(|| {
                        format!(
                            "recognized capability event on line {} has no id",
                            index + 1
                        )
                    })?;
                supplemental_events.push(BehaviorEvent::CapabilityIdentity { capability });
            }
            "prompt_profile" | "prompt" => {
                let profile = string_field(&value, &["profile", "promptProfile"]).or_else(|| {
                    value
                        .get("promptManifest")
                        .and_then(|manifest| string_field(manifest, &["profile", "promptProfile"]))
                });
                if let Some(profile) = profile {
                    supplemental_events.push(BehaviorEvent::PromptProfile { profile });
                }
            }
            "final_response" => {
                if let Some(text) = response_text(&value) {
                    for claim in super::trace::detect_verification_claims(&text) {
                        supplemental_events.push(BehaviorEvent::VerificationClaim { claim });
                    }
                }
                supplemental_events.push(BehaviorEvent::FinalResponse);
            }
            _ => {}
        }
    }

    let mut trace =
        BehaviorTrace::from_agent_events(scenario_id, manifest, &agent_events, Some(stats));
    trace.events.extend(supplemental_events);
    Ok(trace)
}

fn add_lifecycle_event(events: &mut Vec<BehaviorEvent>, event_type: &str) {
    let phase = match event_type {
        "message_start" => Some("start"),
        "message_update" => Some("update"),
        "message_end" => Some("end"),
        _ => None,
    };
    if let Some(phase) = phase {
        events.push(BehaviorEvent::MessageLifecycle {
            phase: phase.into(),
        });
    }
}

fn synthetic_agent_events(value: &Value, index: usize) -> Result<Option<Vec<AgentEvent>>, String> {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let id = string_field(value, &["toolCallId", "tool_call_id", "id"])
        .unwrap_or_else(|| format!("json-tool-{index}"));

    let events = match event_type {
        "tool_call" | "tool_use" => {
            let name = string_field(value, &["toolName", "tool_name", "name"])
                .ok_or_else(|| format!("tool call on line {} has no tool name", index + 1))?;
            let args = value
                .get("args")
                .or_else(|| value.get("arguments"))
                .or_else(|| value.get("input"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            vec![AgentEvent::ToolExecutionStart {
                tool_call_id: id,
                tool_name: name,
                args,
            }]
        }
        "tool_result" | "tool_execution_result" => {
            let name = string_field(value, &["toolName", "tool_name", "name"])
                .unwrap_or_else(|| "unknown".into());
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            let is_error = bool_field(value, &["isError", "is_error", "error"]).unwrap_or(false);
            vec![AgentEvent::ToolExecutionEnd {
                tool_call_id: id,
                tool_name: name,
                result,
                is_error,
                details: value.get("details").cloned(),
            }]
        }
        "shell_exit" => {
            let command = string_field(value, &["command", "cmd"]).unwrap_or_default();
            let tool_name =
                string_field(value, &["tool", "toolName"]).unwrap_or_else(|| "bash".into());
            let exit_code = integer_field(value, &["exitCode", "exit_code"]).unwrap_or(0);
            vec![
                AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: tool_name.clone(),
                    args: json!({ "command": command }),
                },
                AgentEvent::ToolExecutionEnd {
                    tool_call_id: id,
                    tool_name,
                    result: json!({ "exit_code": exit_code }),
                    is_error: exit_code != 0,
                    details: None,
                },
            ]
        }
        "message" | "assistant_message" | "final_response" => {
            let message = message_from_value(value);
            message
                .map(|message| vec![AgentEvent::MessageEnd { message }])
                .unwrap_or_default()
        }
        "message_start" | "message_end" => {
            let message = message_from_value(value).unwrap_or_else(|| {
                ChatMessage::text(
                    string_field(value, &["role"]).unwrap_or_else(|| "assistant".into()),
                    response_text(value).unwrap_or_default(),
                )
            });
            if event_type == "message_start" {
                vec![AgentEvent::MessageStart { message }]
            } else {
                vec![AgentEvent::MessageEnd { message }]
            }
        }
        "edit" => {
            let name = string_field(value, &["tool", "toolName"]).unwrap_or_else(|| "edit".into());
            let args = value.get("args").cloned().unwrap_or_else(|| value.clone());
            vec![
                AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    args,
                },
                AgentEvent::ToolExecutionEnd {
                    tool_call_id: id,
                    tool_name: name,
                    result: value.get("result").cloned().unwrap_or_else(|| json!({})),
                    is_error: bool_field(value, &["isError", "is_error", "error"]).unwrap_or(false),
                    details: value.get("details").cloned(),
                },
            ]
        }
        _ => return Ok(None),
    };

    Ok(Some(events))
}

fn message_from_value(value: &Value) -> Option<ChatMessage> {
    let candidate = value.get("message").unwrap_or(value);
    if let Ok(message) = serde_json::from_value::<ChatMessage>(candidate.clone()) {
        if message.content.is_empty() {
            if let Some(text) = response_text(candidate) {
                return Some(ChatMessage::text(message.role, text));
            }
        }
        return Some(message);
    }
    let role = string_field(candidate, &["role"])?;
    let text = response_text(candidate).unwrap_or_default();
    Some(ChatMessage::text(role, text))
}

fn response_text(value: &Value) -> Option<String> {
    let candidate = value.get("message").unwrap_or(value);
    for key in ["text", "content", "response", "reply"] {
        if let Some(text) = candidate.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    serde_json::from_value::<ChatMessage>(candidate.clone())
        .ok()
        .map(|message| davinci_ai::content_text(&message.content))
}

fn find_prompt_manifest(value: &Value) -> Option<PromptManifest> {
    for key in ["promptManifest", "prompt_manifest", "manifest"] {
        if let Some(candidate) = value.get(key) {
            if let Ok(manifest) = serde_json::from_value(candidate.clone()) {
                return Some(manifest);
            }
        }
    }
    if value.get("type").and_then(Value::as_str) == Some("prompt_manifest") {
        serde_json::from_value(value.clone()).ok()
    } else {
        None
    }
}

fn collect_stats(stats: &mut BehaviorStats, value: &Value) {
    let usage = value.get("usage").unwrap_or(value);
    stats.input_tokens = stats
        .input_tokens
        .saturating_add(integer_field(usage, &["inputTokens", "input_tokens"]).unwrap_or(0) as u64);
    stats.output_tokens = stats.output_tokens.saturating_add(
        integer_field(usage, &["outputTokens", "output_tokens"]).unwrap_or(0) as u64,
    );
    if let Some(wall_ms) = integer_field(value, &["wallMs", "wall_ms"]) {
        stats.wall_ms = stats.wall_ms.max(wall_ms as u64);
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn bool_field(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn integer_field(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::trace::BehaviorEvent;

    fn fixture_lines() -> Vec<String> {
        include_str!("../../fixtures/behavior/json-events.jsonl")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn normalizes_product_json_event_corpus() {
        let trace = trace_from_json_lines("json-fixture", &fixture_lines()).unwrap();

        assert_eq!(trace.scenario_id, "json-fixture");
        assert_eq!(trace.stats.model_turns, 1);
        assert_eq!(trace.stats.tool_calls, 4);
        assert_eq!(trace.stats.permission_prompts, 1);
        assert_eq!(trace.verification.len(), 1);
        assert!(trace.verification[0].passed);
        assert_eq!(trace.files_changed, vec!["src/lib.rs"]);
        assert_eq!(trace.prompt_manifest.as_ref().unwrap().profile, "stable");

        let shell_index = trace
            .events
            .iter()
            .position(|event| matches!(event, BehaviorEvent::Shell { .. }))
            .unwrap();
        let edit_index = trace
            .events
            .iter()
            .position(|event| matches!(event, BehaviorEvent::Edit { .. }))
            .unwrap();
        assert!(edit_index < shell_index);
        assert!(trace.events.iter().any(
            |event| matches!(event, BehaviorEvent::PermissionAsked { tool } if tool == "bash")
        ));
        assert!(trace
            .events
            .iter()
            .any(|event| matches!(event, BehaviorEvent::PlanEvent { kind } if kind == "complete")));
        assert!(trace.events.iter().any(
            |event| matches!(event, BehaviorEvent::CapabilityIdentity { capability } if capability == "debugging")
        ));
        assert!(trace
            .events
            .iter()
            .any(|event| matches!(event, BehaviorEvent::FinalResponse)));
        assert!(trace.events.iter().any(|event| matches!(
            event,
            BehaviorEvent::VerificationClaim { claim } if claim == "all tests pass"
        )));
    }

    #[test]
    fn malformed_json_fails_closed() {
        let error = trace_from_json_lines("bad", &["not-json".into()]).unwrap_err();
        assert!(error.contains("line 1"));
    }

    #[test]
    fn nested_prompt_manifest_exposes_profile_and_capability_identity() {
        let lines = vec![serde_json::json!({
            "type": "prompt_manifest",
            "promptManifest": {
                "profile": "preview",
                "profile_version": 3,
                "stable_sha256": "stable",
                "full_sha256": "full",
                "stable_estimated_tokens": 1,
                "dynamic_estimated_tokens": 1,
                "modules": [{
                    "id": "capability.frontend-design",
                    "version": 1,
                    "cache_class": "Dynamic",
                    "sha256": "module",
                    "estimated_tokens": 1
                }]
            }
        })
        .to_string()];
        let trace = trace_from_json_lines("manifest", &lines).unwrap();

        assert_eq!(trace.prompt_manifest.unwrap().profile, "preview");
        assert!(trace.events.iter().any(|event| matches!(
            event,
            BehaviorEvent::PromptProfile { profile } if profile == "preview"
        )));
        assert!(trace.events.iter().any(|event| matches!(
            event,
            BehaviorEvent::CapabilityIdentity { capability } if capability == "frontend-design"
        )));
    }

    #[test]
    fn workflow_step_events_are_normalized_for_engineering_receipts() {
        let trace = trace_from_json_lines(
            "workflow",
            &[serde_json::json!({
                "type": "workflow_step",
                "step": "transaction_verify"
            })
            .to_string()],
        )
        .unwrap();
        assert!(trace.events.iter().any(|event| matches!(
            event,
            BehaviorEvent::PlanEvent { kind } if kind == "workflow:transaction_verify"
        )));
    }
}
