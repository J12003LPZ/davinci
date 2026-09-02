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

/// Fan-out limits, from the reference extension
/// (`vendor/pi/packages/coding-agent/examples/extensions/subagent/index.ts`:
/// `MAX_PARALLEL_TASKS = 8`, `MAX_CONCURRENCY = 4`).
pub const MAX_PARALLEL_TASKS: usize = 8;
pub const MAX_TASK_CONCURRENCY: usize = 4;

pub fn tool_parameters() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string", "description": "The question for one worker. Omit when passing tasks."},
            "tools": {"type": "array", "items": {"type": "string"}, "description": "Allow-list of read-only tools for the worker"},
            "description": {"type": "string", "description": "A few words naming the task, shown in the UI"},
            "tasks": {
                "type": "array",
                "maxItems": MAX_PARALLEL_TASKS,
                "description": "Up to 8 independent workers that run concurrently, each with its own prompt",
                "items": {
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string"},
                        "description": {"type": "string"},
                        "tools": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["prompt"]
                }
            }
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct SubagentRequest {
    pub prompt: String,
    pub tools: Vec<String>,
    pub description: Option<String>,
    /// The parent's current provider and model, so the worker follows a
    /// `/model` change or a restored session rather than the launch flags.
    pub provider: Option<String>,
    pub model_id: Option<String>,
    /// The parent's abort flag: an interrupted turn stops the worker too.
    pub abort: Option<Arc<std::sync::atomic::AtomicBool>>,
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
    // The worker is read-only by class, not by name: an extension or MCP
    // tool the policy cannot classify (`Other`) could change anything, so
    // only tools known to read or reach the network go through. The name
    // list stays as the documented floor.
    wanted
        .into_iter()
        .filter(|name| parent.iter().any(|known| known == name))
        .filter(|name| !MUTATION_TOOLS.contains(&name.as_str()))
        .filter(|name| {
            matches!(
                crate::permission::tool_class(name),
                crate::permission::ToolClass::Read | crate::permission::ToolClass::Network
            )
        })
        .collect()
}

/// What the worker inherits from the parent turn.
#[derive(Debug, Clone, Default)]
pub struct SubagentParent {
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub abort: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// One worker's request as the model wrote it.
struct TaskSpec {
    prompt: String,
    tools: Option<Vec<String>>,
    description: Option<String>,
}

fn task_spec(input: &Value) -> Result<TaskSpec, ToolError> {
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if prompt.is_empty() {
        return Err(ToolError::Failed("Missing prompt".into()));
    }
    let tools = input.get("tools").and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let description = input
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(TaskSpec {
        prompt: prompt.to_string(),
        tools,
        description,
    })
}

fn cap_output(mut content: String, cap: usize) -> String {
    if content.len() > cap {
        // Back off to a char boundary: `truncate` panics inside a multibyte
        // character.
        let mut cut = cap;
        while !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        content.push_str("\n… truncated");
    }
    content
}

pub fn run_tool(
    input: &Value,
    parent_tools: &[String],
    runner: Option<&SubagentRunner>,
    parent: &SubagentParent,
) -> Result<ToolResult, ToolError> {
    let Some(runner) = runner else {
        return Err(ToolError::Failed("agent tool is not configured".into()));
    };
    let specs: Vec<TaskSpec> = match input.get("tasks").and_then(Value::as_array) {
        Some(tasks) if !tasks.is_empty() => {
            if tasks.len() > MAX_PARALLEL_TASKS {
                return Err(ToolError::Failed(format!(
                    "Too many tasks ({}); the limit is {MAX_PARALLEL_TASKS}",
                    tasks.len()
                )));
            }
            tasks.iter().map(task_spec).collect::<Result<Vec<_>, _>>()?
        }
        _ => vec![task_spec(input)?],
    };
    let requests: Vec<SubagentRequest> = specs
        .iter()
        .map(|spec| SubagentRequest {
            prompt: spec.prompt.clone(),
            tools: scoped_tools(spec.tools.as_deref(), parent_tools),
            description: spec.description.clone(),
            provider: parent.provider.clone(),
            model_id: parent.model_id.clone(),
            abort: parent.abort.clone(),
        })
        .collect();
    if requests.len() == 1 {
        let text = runner.run(&requests[0]).map_err(ToolError::Failed)?;
        return Ok(ToolResult {
            content: cap_output(text, SUBAGENT_OUTPUT_CAP),
            is_error: false,
            details: None,
        });
    }
    // Several workers: fan out over the scheduler's parallel lane, at most
    // `MAX_TASK_CONCURRENCY` at once, and report each under its own heading
    // in task order. One failed worker does not hide the others' answers.
    let calls = requests
        .iter()
        .map(|request| crate::scheduler::ScheduledCall {
            lane: crate::scheduler::ToolLane::Parallel,
            run: Box::new(move || runner.run(request)),
        })
        .collect();
    let (outcomes, _) = crate::scheduler::run_lanes(
        calls,
        false,
        MAX_TASK_CONCURRENCY,
        parent.abort.as_deref(),
        |_| {},
    );
    let per_task_cap = SUBAGENT_OUTPUT_CAP / requests.len().max(1);
    let mut sections = Vec::with_capacity(requests.len());
    let mut failures = 0;
    for (index, request) in requests.iter().enumerate() {
        let title = request
            .description
            .clone()
            .unwrap_or_else(|| format!("task {}", index + 1));
        let body = match outcomes.get(index) {
            Some(Ok(text)) => cap_output(text.clone(), per_task_cap),
            Some(Err(err)) => {
                failures += 1;
                format!("(failed: {err})")
            }
            None => {
                failures += 1;
                "(not run: interrupted)".to_string()
            }
        };
        sections.push(format!("## {} — {title}\n{body}", index + 1));
    }
    if failures == requests.len() {
        return Err(ToolError::Failed(format!(
            "all {} workers failed:\n{}",
            requests.len(),
            sections.join("\n\n")
        )));
    }
    Ok(ToolResult {
        content: sections.join("\n\n"),
        is_error: false,
        details: Some(serde_json::json!({
            "tasks": requests.len(),
            "failed": failures,
        })),
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
    fn tools_the_policy_cannot_classify_are_stripped_too() {
        // An MCP or extension tool is `Other` to the gate: it could write
        // anything, so the worker never gets it even when asked by name.
        let parent = vec![
            "read".into(),
            "web_fetch".into(),
            "mcp__memory__store".into(),
            "graph_write".into(),
        ];
        let scoped = scoped_tools(
            Some(&[
                "read".into(),
                "web_fetch".into(),
                "mcp__memory__store".into(),
                "graph_write".into(),
            ]),
            &parent,
        );
        assert_eq!(scoped, vec!["read".to_string(), "web_fetch".to_string()]);
    }

    #[test]
    fn a_canned_runner_returns_the_reply() {
        let runner = SubagentRunner::new(|req| Ok(req.prompt.chars().rev().collect()));
        let result = run_tool(
            &json!({"prompt": "abc", "description": "flip"}),
            &["read".into()],
            Some(&runner),
            &SubagentParent::default(),
        )
        .unwrap();
        assert_eq!(result.content, "cba");
    }

    #[test]
    fn tasks_fan_out_and_report_in_order() {
        let runner = SubagentRunner::new(|req| {
            if req.prompt == "boom" {
                Err("nope".into())
            } else {
                std::thread::sleep(std::time::Duration::from_millis(if req.prompt == "slow" {
                    60
                } else {
                    5
                }));
                Ok(format!("answer to {}", req.prompt))
            }
        });
        let start = std::time::Instant::now();
        let result = run_tool(
            &json!({"tasks": [
                {"prompt": "slow", "description": "first"},
                {"prompt": "fast"},
                {"prompt": "boom", "description": "broken"},
            ]}),
            &["read".into()],
            Some(&runner),
            &SubagentParent::default(),
        )
        .unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(120));
        let text = result.content;
        let first = text.find("## 1 — first\nanswer to slow").unwrap();
        let second = text.find("## 2 — task 2\nanswer to fast").unwrap();
        let third = text.find("## 3 — broken\n(failed: nope)").unwrap();
        assert!(first < second && second < third, "{text}");
        assert_eq!(result.details.unwrap()["failed"], 1);
    }

    #[test]
    fn every_task_failing_is_an_error() {
        let runner = SubagentRunner::new(|_| Err("down".into()));
        let err = run_tool(
            &json!({"tasks": [{"prompt": "a"}, {"prompt": "b"}]}),
            &["read".into()],
            Some(&runner),
            &SubagentParent::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("all 2 workers failed"), "{err}");
    }

    #[test]
    fn a_missing_runner_is_named() {
        let err = run_tool(
            &json!({"prompt": "x"}),
            &["read".into()],
            None,
            &SubagentParent::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not configured"), "{err}");
    }
}
