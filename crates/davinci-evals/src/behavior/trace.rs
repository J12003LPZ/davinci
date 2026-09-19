//! Normalized behavioral trace capturing agent execution.

use davinci_agent::prompt::manifest::PromptManifest;
use davinci_agent::AgentEvent;
use davinci_ai::{content_text, ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorTrace {
    pub scenario_id: String,
    pub prompt_manifest: Option<PromptManifest>,
    pub events: Vec<BehaviorEvent>,
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub file_diffs: Vec<FileDiff>,
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
    MessageLifecycle {
        phase: String,
    },
    PlanEvent {
        kind: String,
    },
    CapabilityIdentity {
        capability: String,
    },
    PromptProfile {
        profile: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
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

    if lower.starts_with("npm run dev")
        || lower.starts_with("npm start")
        || lower.starts_with("pnpm dev")
        || lower.starts_with("yarn dev")
        || lower.starts_with("vite")
        || lower.starts_with("next dev")
        || lower.starts_with("cargo run")
    {
        "dev_server"
    } else if lower.contains("test")
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
            file_diffs: Vec::new(),
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

        let mut pending_shells: HashMap<String, PendingShell> = HashMap::new();
        let mut pending_edits: HashMap<String, PendingEdit> = HashMap::new();

        for event in events {
            match event {
                AgentEvent::TurnStart => {
                    trace.stats.model_turns += 1;
                }
                AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name,
                    args,
                } => {
                    trace.stats.tool_calls += 1;
                    if let Some((capability, operation)) = native_capability(tool_name) {
                        trace.events.push(BehaviorEvent::CapabilityIdentity {
                            capability: capability.to_string(),
                        });
                        trace.events.push(BehaviorEvent::CapabilityIdentity {
                            capability: operation.to_string(),
                        });
                    }
                    match tool_name.as_str() {
                        "read" | "read_file" | "mcp_read" => {
                            let path = args
                                .get("path")
                                .or_else(|| args.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let start = args
                                .get("start")
                                .or_else(|| args.get("offset"))
                                .and_then(|v| v.as_u64());
                            let end = args
                                .get("end")
                                .or_else(|| args.get("limit"))
                                .and_then(|v| v.as_u64());
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
                            let patch = (tool_name == "apply_patch")
                                .then(|| args.get("input").and_then(Value::as_str))
                                .flatten()
                                .map(str::to_owned);
                            let mut paths = patch.as_deref().map(patch_paths).unwrap_or_default();
                            if paths.is_empty() {
                                if let Some(path) = args
                                    .get("path")
                                    .or_else(|| args.get("file_path"))
                                    .and_then(|v| v.as_str())
                                    .filter(|path| !path.is_empty())
                                {
                                    paths.push(path.to_owned());
                                }
                            }
                            for path in &paths {
                                if !trace.files_changed.contains(path) {
                                    trace.files_changed.push(path.clone());
                                }
                                trace
                                    .events
                                    .push(BehaviorEvent::Edit { path: path.clone() });
                            }
                            if paths.is_empty() {
                                trace.events.push(BehaviorEvent::Edit {
                                    path: String::new(),
                                });
                            }
                            pending_edits
                                .insert(tool_call_id.clone(), PendingEdit { paths, patch });
                        }
                        "bash" | "powershell" | "exec_command" => {
                            let cmd = args
                                .get("command")
                                .or_else(|| args.get("cmd"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let class = classify_shell_command(cmd);
                            pending_shells.insert(
                                tool_call_id.clone(),
                                PendingShell {
                                    class,
                                    command: cmd.to_string(),
                                },
                            );
                        }
                        "agent" => {
                            trace.events.push(BehaviorEvent::SubagentSpawn { count: 1 });
                        }
                        _ => {}
                    }
                }
                AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name,
                    result,
                    is_error,
                    details,
                    ..
                } => {
                    if tool_name == "bash"
                        || tool_name == "powershell"
                        || tool_name == "exec_command"
                    {
                        let pending = pending_shells.remove(tool_call_id);
                        let class = pending.as_ref().map(|shell| shell.class).unwrap_or("other");
                        let command = pending
                            .as_ref()
                            .map(|shell| shell.command.clone())
                            .unwrap_or_default();
                        let exit_code = if *is_error {
                            result
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .map(|c| c as i32)
                                .or(Some(1))
                        } else {
                            result
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .map(|c| c as i32)
                                .or(Some(0))
                        };

                        trace.events.push(BehaviorEvent::Shell {
                            command_class: class.to_string(),
                            exit_code,
                        });

                        if class == "test" || class == "build" || class == "lint" {
                            trace.verification.push(VerificationEvent {
                                kind: class.to_string(),
                                command,
                                passed: exit_code == Some(0),
                            });
                        }
                    }

                    if is_edit_tool(tool_name) {
                        let pending = pending_edits.remove(tool_call_id);
                        if !*is_error {
                            let diff = details
                                .as_ref()
                                .and_then(|details| details.get("diff"))
                                .and_then(Value::as_str)
                                .filter(|diff| !diff.is_empty())
                                .map(str::to_owned)
                                .or_else(|| pending.as_ref().and_then(|edit| edit.patch.clone()));
                            if let Some(diff) = diff {
                                let paths = details
                                    .as_ref()
                                    .and_then(|details| details.get("paths"))
                                    .and_then(Value::as_array)
                                    .map(|paths| {
                                        paths
                                            .iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_owned)
                                            .collect::<Vec<_>>()
                                    })
                                    .filter(|paths| !paths.is_empty())
                                    .or_else(|| {
                                        details
                                            .as_ref()
                                            .and_then(|details| details.get("path"))
                                            .and_then(Value::as_str)
                                            .filter(|path| !path.is_empty())
                                            .map(|path| vec![path.to_owned()])
                                    })
                                    .or_else(|| pending.as_ref().map(|edit| edit.paths.clone()))
                                    .filter(|paths| !paths.is_empty())
                                    .unwrap_or_else(|| vec![String::new()]);
                                trace
                                    .file_diffs
                                    .extend(paths.into_iter().map(|path| FileDiff {
                                        path,
                                        diff: diff.clone(),
                                    }));
                            }
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
                AgentEvent::MessageStart { .. } | AgentEvent::MessageUpdate { .. } => {}
                AgentEvent::MessageEnd { message } | AgentEvent::TurnEnd { message, .. } => {
                    record_assistant_message(&mut trace, message);
                }
                AgentEvent::AgentEnd { messages, .. } => {
                    for message in messages {
                        record_assistant_message(&mut trace, message);
                    }
                }
                _ => {}
            }
        }

        trace
    }
}

fn record_assistant_message(trace: &mut BehaviorTrace, message: &ChatMessage) {
    if message.role != "assistant" {
        return;
    }

    let text = content_text(&message.content);
    for claim in detect_verification_claims(&text) {
        trace
            .events
            .push(BehaviorEvent::VerificationClaim { claim });
    }
    let has_tool_calls = message
        .content
        .iter()
        .any(|block| matches!(block, MessageContent::ToolCall { .. }));
    if !has_tool_calls && !text.is_empty() {
        trace.events.push(BehaviorEvent::FinalResponse);
    }
}

#[derive(Debug, Clone)]
struct PendingShell {
    class: &'static str,
    command: String,
}

#[derive(Debug, Clone)]
struct PendingEdit {
    paths: Vec<String>,
    patch: Option<String>,
}

fn patch_paths(patch: &str) -> Vec<String> {
    davinci_agent::apply_patch::parse_codex_patch(patch)
        .map(|parsed| {
            parsed
                .actions
                .into_iter()
                .map(|action| match action {
                    davinci_agent::apply_patch::FileAction::Add { path, .. }
                    | davinci_agent::apply_patch::FileAction::Delete { path }
                    | davinci_agent::apply_patch::FileAction::Update { path, .. } => path,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_edit_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "edit" | "write" | "apply_patch" | "notebook_edit"
    )
}

fn native_capability(tool_name: &str) -> Option<(&'static str, &str)> {
    let capability = if matches!(
        tool_name,
        "repo_map"
            | "symbol_search"
            | "file_symbols"
            | "file_dependencies"
            | "symbol_relationships"
            | "related_files"
            | "code_query"
    ) {
        "repo_intelligence"
    } else if tool_name.starts_with("lsp_") {
        "language_intelligence"
    } else if tool_name.starts_with("package_") {
        "package_intelligence"
    } else if matches!(
        tool_name,
        "workspace_packages"
            | "build_targets"
            | "build_dependencies"
            | "build_affected"
            | "build_command"
    ) {
        "build_intelligence"
    } else if tool_name.starts_with("git_") {
        "git_intelligence"
    } else if tool_name.starts_with("test_") {
        "test_impact"
    } else if tool_name == "impact_analyze" {
        "change_impact"
    } else if tool_name == "verification_plan" {
        "verification_planner"
    } else if matches!(
        tool_name,
        "workspace_checkpoint" | "workspace_diff" | "workspace_restore"
    ) {
        "workspace_snapshots"
    } else if tool_name.starts_with("browser_") {
        "browser_verification"
    } else if matches!(tool_name, "process_start" | "process_write") {
        "process_manager"
    } else {
        return None;
    };
    Some((capability, tool_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::ChatMessage;
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
                details: Some(json!({ "diff": "+return Err(error)" })),
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
        assert_eq!(trace.verification[0].command, "cargo test");
        assert!(trace.verification[0].passed);
        assert_eq!(
            trace.file_diffs,
            vec![FileDiff {
                path: "src/lib.rs".into(),
                diff: "+return Err(error)".into(),
            }]
        );

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
        assert_eq!(
            classify_shell_command("cargo clippy -- -D warnings"),
            "lint"
        );
        assert_eq!(classify_shell_command("git status"), "git");
        assert_eq!(classify_shell_command("ls -la"), "search");
        assert_eq!(classify_shell_command("curl http://example.com"), "other");
    }

    #[test]
    fn correlates_interleaved_shell_results_by_tool_call_id() {
        let events = vec![
            AgentEvent::ToolExecutionStart {
                tool_call_id: "test".into(),
                tool_name: "bash".into(),
                args: json!({ "command": "cargo test --test regression" }),
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "lint".into(),
                tool_name: "powershell".into(),
                args: json!({ "command": "cargo clippy" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "lint".into(),
                tool_name: "powershell".into(),
                result: json!({ "exit_code": 1 }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "test".into(),
                tool_name: "bash".into(),
                result: json!({ "exit_code": 0 }),
                is_error: false,
                details: None,
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);

        assert_eq!(trace.verification[0].command, "cargo clippy");
        assert!(!trace.verification[0].passed);
        assert_eq!(
            trace.verification[1].command,
            "cargo test --test regression"
        );
        assert!(trace.verification[1].passed);
    }

    #[test]
    fn captures_successful_apply_patch_diff_without_result_details() {
        let patch = "*** Begin Patch\n*** Add File: src/service.rs\n+return Ok(())\n*** End Patch";
        let events = vec![
            AgentEvent::ToolExecutionStart {
                tool_call_id: "patch".into(),
                tool_name: "apply_patch".into(),
                args: json!({ "input": patch }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "patch".into(),
                tool_name: "apply_patch".into(),
                result: json!({ "success": true }),
                is_error: false,
                details: None,
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);
        assert_eq!(trace.files_changed, vec!["src/service.rs"]);
        assert_eq!(
            trace.file_diffs,
            vec![FileDiff {
                path: "src/service.rs".into(),
                diff: patch.into(),
            }]
        );
    }
}
