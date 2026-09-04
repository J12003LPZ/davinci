use std::collections::HashMap;

use davinci_agent::AgentEvent;
use serde_json::Value;

use crate::native_extensions::graph::types::VerificationResult;
use crate::native_extensions::learning::types::{
    LearningEvidence, ToolEvidence, VerificationEvidence,
};
use crate::native_extensions::vector_memory::{redact_secrets, MemoryMessage};

const MAX_MESSAGE_CHARS: usize = 4_000;
const MAX_MESSAGES_COUNT: usize = 10;
const MAX_TOOL_ARGS_CHARS: usize = 1_000;
const MAX_TOOL_RESULT_CHARS: usize = 2_000;

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

pub struct BuildEvidenceInput<'a> {
    pub session_id: String,
    pub repo_id: String,
    pub turn: u64,
    pub messages: &'a [MemoryMessage],
    pub events: &'a [AgentEvent],
    pub run_stats: davinci_agent::RunStats,
    pub verification: VerificationEvidence,
}

#[allow(dead_code)]
pub fn verification_evidence_from_graph(
    run_id: Option<String>,
    result: Option<&VerificationResult>,
) -> VerificationEvidence {
    let commands_ran = result
        .map(|r| r.commands.iter().filter(|c| !c.skipped).count() as u32)
        .unwrap_or(0);
    let passed = result.map(|r| r.passed).unwrap_or(false) && commands_ran > 0;
    VerificationEvidence {
        graph_run_id: run_id,
        commands_ran,
        passed,
        user_accepted: false,
        user_corrected: false,
        permission_denied: false,
    }
}

pub fn build_learning_evidence(input: BuildEvidenceInput<'_>) -> LearningEvidence {
    let mut sanitized_messages = Vec::new();

    let relevant_messages: Vec<&MemoryMessage> = input
        .messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();

    let start_idx = relevant_messages.len().saturating_sub(MAX_MESSAGES_COUNT);
    let mut user_corrected = false;

    for m in &relevant_messages[start_idx..] {
        let content_redacted = redact_secrets(&m.content);
        let content_truncated = truncate_chars(&content_redacted, MAX_MESSAGE_CHARS);
        if m.role == "user" {
            let lower = m.content.to_lowercase();
            if lower.contains("that's wrong")
                || lower.contains("thats wrong")
                || lower.contains("no, don't")
                || lower.contains("no, that is not")
                || lower.starts_with("correction:")
            {
                user_corrected = true;
            }
        }
        sanitized_messages.push(MemoryMessage {
            role: m.role.clone(),
            content: content_truncated,
        });
    }

    // Correlate tool starts and ends
    let mut tool_starts: HashMap<String, Value> = HashMap::new();
    let mut tools = Vec::new();
    let mut permission_denied = false;

    for event in input.events {
        match event {
            AgentEvent::ToolExecutionStart {
                tool_call_id, args, ..
            } => {
                tool_starts.insert(tool_call_id.clone(), args.clone());
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                ..
            } => {
                let raw_args = tool_starts
                    .get(tool_call_id)
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .unwrap_or_default();
                let redacted_args = redact_secrets(&raw_args);
                let args_summary = truncate_chars(&redacted_args, MAX_TOOL_ARGS_CHARS);

                let raw_result = if let Some(s) = result.as_str() {
                    s.to_string()
                } else {
                    result.to_string()
                };
                let redacted_result = redact_secrets(&raw_result);
                let result_summary = truncate_chars(&redacted_result, MAX_TOOL_RESULT_CHARS);

                let res_lower = raw_result.to_lowercase();
                let perm_denied = *is_error
                    && (res_lower.contains("permission denied")
                        || res_lower.contains("blocked by policy")
                        || res_lower.contains("permission_denied"));

                if perm_denied {
                    permission_denied = true;
                }

                tools.push(ToolEvidence {
                    name: tool_name.clone(),
                    is_error: *is_error,
                    args_summary,
                    result_summary,
                    permission_denied: perm_denied,
                });
            }
            _ => {}
        }
    }

    let mut verification = input.verification;
    if permission_denied {
        verification.permission_denied = true;
    }
    if user_corrected {
        verification.user_corrected = true;
    }

    LearningEvidence {
        session_id: input.session_id,
        repo_id: input.repo_id,
        turn: input.turn,
        messages: sanitized_messages,
        tools,
        run_stats: input.run_stats,
        verification,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{VerificationCommandResult, VerificationResult};

    fn fixture_tool_end(name: &str, output: String) -> AgentEvent {
        AgentEvent::ToolExecutionEnd {
            tool_call_id: "call-1".to_string(),
            tool_name: name.to_string(),
            result: Value::String(output),
            is_error: false,
            details: None,
        }
    }

    #[test]
    fn evidence_never_copies_unbounded_tool_output() {
        let huge = "x".repeat(100_000);
        let events = vec![fixture_tool_end("bash", huge)];
        let messages = vec![MemoryMessage {
            role: "user".into(),
            content: "run a big command".into(),
        }];
        let input = BuildEvidenceInput {
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            turn: 1,
            messages: &messages,
            events: &events,
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        };
        let evidence = build_learning_evidence(input);
        assert!(evidence.serialized_len() < 20_000);
        assert!(evidence.tools[0].result_summary.len() <= MAX_TOOL_RESULT_CHARS);
    }

    #[test]
    fn verification_evidence_from_graph_truth_table() {
        // 1. One real passing command -> pass
        let res1 = VerificationResult {
            passed: true,
            commands: vec![VerificationCommandResult {
                name: "test".into(),
                command: "cargo test".into(),
                exit_code: 0,
                duration_ms: 100,
                output_tail: "ok".into(),
                skipped: false,
            }],
        };
        let ev1 = verification_evidence_from_graph(Some("run-1".into()), Some(&res1));
        assert!(ev1.passed);
        assert_eq!(ev1.commands_ran, 1);
        assert_eq!(ev1.graph_run_id.as_deref(), Some("run-1"));

        // 2. Only skipped commands -> fail
        let res2 = VerificationResult {
            passed: true,
            commands: vec![VerificationCommandResult {
                name: "test".into(),
                command: "missing".into(),
                exit_code: 0,
                duration_ms: 0,
                output_tail: "".into(),
                skipped: true,
            }],
        };
        let ev2 = verification_evidence_from_graph(None, Some(&res2));
        assert!(!ev2.passed);
        assert_eq!(ev2.commands_ran, 0);

        // 3. Empty list -> fail
        let res3 = VerificationResult {
            passed: true,
            commands: vec![],
        };
        let ev3 = verification_evidence_from_graph(None, Some(&res3));
        assert!(!ev3.passed);
        assert_eq!(ev3.commands_ran, 0);

        // 4. One failed command -> fail
        let res4 = VerificationResult {
            passed: false,
            commands: vec![VerificationCommandResult {
                name: "test".into(),
                command: "cargo test".into(),
                exit_code: 1,
                duration_ms: 100,
                output_tail: "failed".into(),
                skipped: false,
            }],
        };
        let ev4 = verification_evidence_from_graph(None, Some(&res4));
        assert!(!ev4.passed);
        assert_eq!(ev4.commands_ran, 1);
    }
}
