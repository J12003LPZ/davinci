//! Normalized behavioral trace capturing agent execution.

use davinci_agent::prompt::manifest::PromptManifest;
use davinci_agent::AgentEvent;
use davinci_ai::{content_text, ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorTrace {
    pub scenario_id: String,
    pub prompt_manifest: Option<PromptManifest>,
    pub events: Vec<BehaviorEvent>,
    pub files_changed: Vec<String>,
    pub verification: Vec<VerificationEvent>,
    pub stats: BehaviorStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BehaviorEvent {
    Search {
        tool: String,
        query: String,
    },
    Read {
        path: String,
        start: Option<u64>,
        end: Option<u64>,
    },
    Edit {
        path: String,
    },
    Shell {
        command_class: String,
        exit_code: Option<i32>,
    },
    SubagentSpawn {
        count: usize,
    },
    PermissionAsked {
        tool: String,
    },
    PermissionDenied {
        tool: String,
    },
    VerificationClaim {
        claim: String,
    },
    FinalResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationEvent {
    pub kind: String,
    pub command: String,
    pub passed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorStats {
    pub model_turns: u64,
    pub tool_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_ms: u64,
    pub permission_prompts: u64,
}

pub fn classify_shell_command(command: &str) -> &'static str {
    let trimmed = command.trim();
    let lower = trimmed.to_lowercase();
    let first_word = lower.split_whitespace().next().unwrap_or("");

    if lower.contains("test")
        || lower.starts_with("pytest")
        || lower.starts_with("cargo test")
        || lower.starts_with("npm test")
        || lower.starts_with("vitest")
        || lower.starts_with("jest")
        || lower.starts_with("ctest")
    {
        "test"
    } else if lower.starts_with("cargo build")
        || lower.starts_with("npm run build")
        || lower.starts_with("make")
        || lower.starts_with("cargo check")
        || lower.starts_with("rustc")
        || lower.starts_with("gcc")
        || lower.starts_with("clang")
    {
        "build"
    } else if lower.starts_with("cargo clippy")
        || lower.starts_with("eslint")
        || lower.starts_with("tsc")
        || lower.starts_with("flake8")
        || lower.starts_with("ruff")
        || lower.starts_with("mypy")
    {
        "lint"
    } else if first_word == "git" {
        "git"
    } else if first_word == "grep"
        || first_word == "rg"
        || first_word == "find"
        || first_word == "ls"
        || first_word == "dir"
    {
        "search"
    } else {
        "other"
    }
}

pub fn detect_verification_claims(text: &str) -> Vec<String> {
    let mut claims = Vec::new();
    let lower = text.to_lowercase();
    let patterns = [
        "all tests pass",
        "tests pass",
        "tests are passing",
        "test suite passed",
        "verified that",
        "verification passed",
        "build succeeded",
        "build passes",
        "all checks pass",
        "successfully verified",
    ];

    for pat in &patterns {
        if lower.contains(pat) {
            claims.push(pat.to_string());
        }
    }
    claims
}

impl BehaviorTrace {
    pub fn new(scenario_id: impl Into<String>, prompt_manifest: Option<PromptManifest>) -> Self {
        Self {
            scenario_id: scenario_id.into(),
            prompt_manifest,
            events: Vec::new(),
            files_changed: Vec::new(),
            verification: Vec::new(),
            stats: BehaviorStats::default(),
        }
    }

    pub fn from_agent_events(
        scenario_id: &str,
        manifest: Option<PromptManifest>,
        events: &[AgentEvent],
        initial_stats: Option<BehaviorStats>,
    ) -> Self {
        let mut trace = Self::new(scenario_id, manifest);
        if let Some(stats) = initial_stats {
            trace.stats = stats;
        }

        let mut pending_shell_class: Option<&'static str> = None;

        for event in events {
            match event {
                AgentEvent::TurnStart => {
                    trace.stats.model_turns += 1;
                }
                AgentEvent::ToolExecutionStart {
                    tool_name,
                    args,
                    ..
                } => {
                    trace.stats.tool_calls += 1;
                    match tool_name.as_str() {
                        "read" | "read_file" | "mcp_read" => {
                            let path = args
                                .get("path")
                                .or_else(|| args.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let start = args.get("start").or_else(|| args.get("offset")).and_then(|v| v.as_u64());
                            let end = args.get("end").or_else(|| args.get("limit")).and_then(|v| v.as_u64());
                            trace.events.push(BehaviorEvent::Read { path, start, end });
                        }
                        "grep" | "find" | "web_search" | "code_search" => {
                            let query = args
                                .get("query")
                                .or_else(|| args.get("pattern"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            trace.events.push(BehaviorEvent::Search {
                                tool: tool_name.clone(),
                                query,
                            });
                        }
                        "edit" | "write" | "apply_patch" | "notebook_edit" => {
                            let path = args
                                .get("path")
                                .or_else(|| args.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !path.is_empty() && !trace.files_changed.contains(&path) {
                                trace.files_changed.push(path.clone());
                            }
                            trace.events.push(BehaviorEvent::Edit { path });
                        }
                        "bash" | "powershell" | "exec_command" => {
                            let cmd = args
                                .get("command")
                                .or_else(|| args.get("cmd"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let class = classify_shell_command(cmd);
                            pending_shell_class = Some(class);
                        }
                        "agent" => {
                            trace.events.push(BehaviorEvent::SubagentSpawn { count: 1 });
                        }
                        _ => {}
                    }
                }
                AgentEvent::ToolExecutionEnd {
                    tool_name,
                    result,
                    is_error,
                    ..
                } => {
                    if tool_name == "bash" || tool_name == "powershell" || tool_name == "exec_command" {
                        let class = pending_shell_class.take().unwrap_or("other");
                        let exit_code = if *is_error {
                            result.get("exit_code").and_then(|v| v.as_i64()).map(|c| c as i32).or(Some(1))
                        } else {
                            result.get("exit_code").and_then(|v| v.as_i64()).map(|c| c as i32).or(Some(0))
                        };

                        trace.events.push(BehaviorEvent::Shell {
                            command_class: class.to_string(),
                            exit_code,
                        });

                        if class == "test" || class == "build" || class == "lint" {
                            trace.verification.push(VerificationEvent {
                                kind: class.to_string(),
                                command: class.to_string(),
                                passed: exit_code == Some(0),
                            });
                        }
                    }

                    if *is_error {
                        let err_str = result.to_string().to_lowercase();
                        if err_str.contains("permission") || err_str.contains("denied") {
                            trace.events.push(BehaviorEvent::PermissionDenied {
                                tool: tool_name.clone(),
                            });
                        }
                    }
                }
                AgentEvent::MessageEnd { message } | AgentEvent::TurnEnd { message, .. } => {
                    if message.role == "assistant" {
                        let text = content_text(&message.content);
                        for claim in detect_verification_claims(&text) {
                            trace.events.push(BehaviorEvent::VerificationClaim { claim });
                        }
                        let has_tool_calls = message.content.iter().any(|b| matches!(b, MessageContent::ToolCall { .. }));
                        if !has_tool_calls && !text.is_empty() {
                            trace.events.push(BehaviorEvent::FinalResponse);
                        }
                    }
                }
                _ => {}
            }
        }

        trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_agent_events_into_behavior_trace() {
        let events = vec![
            AgentEvent::TurnStart,
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c1".into(),
                tool_name: "grep".into(),
                args: json!({ "query": "fn process" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c1".into(),
                tool_name: "grep".into(),
                result: json!({ "matches": ["src/lib.rs:10"] }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c2".into(),
                tool_name: "read".into(),
                args: json!({ "path": "src/lib.rs", "start": 1, "end": 20 }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c2".into(),
                tool_name: "read".into(),
                result: json!({ "content": "fn process() {}" }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c3".into(),
                tool_name: "edit".into(),
                args: json!({ "path": "src/lib.rs" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c3".into(),
                tool_name: "edit".into(),
                result: json!({ "success": true }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c4".into(),
                tool_name: "bash".into(),
                args: json!({ "command": "cargo test" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c4".into(),
                tool_name: "bash".into(),
                result: json!({ "exit_code": 0 }),
                is_error: false,
                details: None,
            },
            AgentEvent::MessageEnd {
                message: ChatMessage::text(
                    "assistant",
                    "I finished the changes and all tests pass.",
                ),
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);

        assert_eq!(trace.scenario_id, "scenario-1");
        assert_eq!(trace.files_changed, vec!["src/lib.rs"]);
        assert_eq!(trace.verification.len(), 1);
        assert_eq!(trace.verification[0].kind, "test");
        assert!(trace.verification[0].passed);

        assert_eq!(trace.stats.model_turns, 1);
        assert_eq!(trace.stats.tool_calls, 4);

        assert_eq!(
            trace.events,
            vec![
                BehaviorEvent::Search {
                    tool: "grep".into(),
                    query: "fn process".into(),
                },
                BehaviorEvent::Read {
                    path: "src/lib.rs".into(),
                    start: Some(1),
                    end: Some(20),
                },
                BehaviorEvent::Edit {
                    path: "src/lib.rs".into(),
                },
                BehaviorEvent::Shell {
                    command_class: "test".into(),
                    exit_code: Some(0),
                },
                BehaviorEvent::VerificationClaim {
                    claim: "all tests pass".into(),
                },
                BehaviorEvent::VerificationClaim {
                    claim: "tests pass".into(),
                },
                BehaviorEvent::FinalResponse,
            ]
        );
    }

    #[test]
    fn classifies_commands_accurately() {
        assert_eq!(classify_shell_command("cargo test --lib"), "test");
        assert_eq!(classify_shell_command("pytest -v tests/"), "test");
        assert_eq!(classify_shell_command("npm test"), "test");
        assert_eq!(classify_shell_command("cargo build --release"), "build");
        assert_eq!(classify_shell_command("cargo clippy -- -D warnings"), "lint");
        assert_eq!(classify_shell_command("git status"), "git");
        assert_eq!(classify_shell_command("ls -la"), "search");
        assert_eq!(classify_shell_command("curl http://example.com"), "other");
    }
}
