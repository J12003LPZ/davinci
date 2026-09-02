//! Nested agents: one-shot workers with a scoped tool list.
//!
//! No TypeScript counterpart. Phase 5 spec:
//! `docs/superpowers/specs/2026-09-01-plan-and-subagents-design.md`.

use std::sync::Arc;

use serde_json::Value;

use crate::tools::{ToolError, ToolResult};

pub const PLAN_MODE_APPENDIX: &str =
    "You are in plan mode. You may read the project, search the web, \
and keep the todo list current. You must not edit files, run shell commands, \
or start a subagent. Wait for the user to /act before making changes.";

pub const PLAN_MODE_DENIAL: &str = "plan mode: mutations are off until /act";

pub const DEFAULT_SUBAGENT_TOOLS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "web_fetch",
    "web_search",
    "mcp_read",
];

const MUTATION_TOOLS: &[&str] = &[
    "bash",
    "powershell",
    "write",
    "edit",
    "notebook_edit",
    "agent",
];

pub const SUBAGENT_OUTPUT_CAP: usize = 50 * 1024;

#[derive(Debug, Clone)]
pub struct SubagentRequest {
    pub prompt: String,
    pub tools: Vec<String>,
    pub description: Option<String>,
}

type SubagentFn = dyn Fn(&SubagentRequest) -> Result<String, String> + Send + Sync;

#[derive(Clone)]
pub struct SubagentRunner {
    inner: Arc<SubagentFn>,
}

impl std::fmt::Debug for SubagentRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SubagentRunner")
    }
}

impl SubagentRunner {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&SubagentRequest) -> Result<String, String> + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    pub fn run(&self, request: &SubagentRequest) -> Result<String, String> {
        (self.inner)(request)
    }
}

pub fn scoped_tools(requested: Option<&[String]>, parent: &[String]) -> Vec<String> {
    let wanted: Vec<String> = match requested {
        Some(list) if !list.is_empty() => list.to_vec(),
        _ => DEFAULT_SUBAGENT_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    };
    wanted
        .into_iter()
        .filter(|name| parent.iter().any(|known| known == name))
        .filter(|name| !MUTATION_TOOLS.contains(&name.as_str()))
        .collect()
}

pub fn run_tool(
    input: &Value,
    parent_tools: &[String],
    runner: Option<&SubagentRunner>,
) -> Result<ToolResult, ToolError> {
    let Some(runner) = runner else {
        return Err(ToolError::Failed("agent tool is not configured".into()));
    };
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if prompt.is_empty() {
        return Err(ToolError::Failed("Missing prompt".into()));
    }
    let requested = input.get("tools").and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let tools = scoped_tools(requested.as_deref(), parent_tools);
    let description = input
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let text = runner
        .run(&SubagentRequest {
            prompt: prompt.to_string(),
            tools,
            description,
        })
        .map_err(ToolError::Failed)?;
    let mut content = text;
    if content.len() > SUBAGENT_OUTPUT_CAP {
        content.truncate(SUBAGENT_OUTPUT_CAP);
        content.push_str("\n… truncated");
    }
    Ok(ToolResult {
        content,
        is_error: false,
        details: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mutation_tools_and_agent_are_stripped() {
        let parent = vec![
            "read".into(),
            "bash".into(),
            "write".into(),
            "agent".into(),
            "grep".into(),
        ];
        let scoped = scoped_tools(
            Some(&["read".into(), "bash".into(), "agent".into(), "nope".into()]),
            &parent,
        );
        assert_eq!(scoped, vec!["read".to_string()]);
    }

    #[test]
    fn a_canned_runner_returns_the_reply() {
        let runner = SubagentRunner::new(|req| Ok(req.prompt.chars().rev().collect()));
        let result = run_tool(
            &json!({"prompt": "abc", "description": "flip"}),
            &["read".into()],
            Some(&runner),
        )
        .unwrap();
        assert_eq!(result.content, "cba");
    }

    #[test]
    fn a_missing_runner_is_named() {
        let err = run_tool(&json!({"prompt": "x"}), &["read".into()], None).unwrap_err();
        assert!(err.to_string().contains("not configured"), "{err}");
    }
}
