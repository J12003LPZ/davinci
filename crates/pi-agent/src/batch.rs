//! The `batch` tool: several independent operations behind one
//! model-visible boundary.
//!
//! No TypeScript counterpart. A model that already knows it wants three
//! greps and four reads pays one inference per call when each is its own
//! turn; `batch` lets it pay one. Every operation still passes the same
//! gate as a direct call (the extension hook, the permission policy, the
//! post hook), runs in the same lanes (`scheduler.rs`), and is counted in
//! `RunStats::batch_operations`. What the model gets back is bounded:
//! `VISIBLE_PER_OPERATION` per operation and `VISIBLE_TOTAL` for the
//! batch; anything past a cap goes to the evidence store and the result
//! names the file, so a `read` with `offset`/`limit` can fetch the rest.
//! A batch may not contain another batch or an `agent` call.

use std::path::Path;

use serde_json::Value;

use crate::evidence::cut_at_char_boundary;
use crate::tools::ToolResult;
use crate::Agent;

pub const BATCH_MAX_OPERATIONS: usize = 16;
pub const VISIBLE_PER_OPERATION: usize = 12 * 1024;
pub const VISIBLE_TOTAL: usize = 64 * 1024;

pub fn batch_parameters() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "operations": {
                "type": "array",
                "maxItems": BATCH_MAX_OPERATIONS,
                "description": "Independent operations to run together. Read-only ones (read, grep, find, ls, web_fetch, mcp_read) overlap; edits and shell commands run in order between them.",
                "items": {
                    "type": "object",
                    "properties": {
                        "tool": {"type": "string", "description": "Name of the tool to call"},
                        "args": {"type": "object", "description": "That tool's arguments"}
                    },
                    "required": ["tool", "args"]
                }
            }
        },
        "required": ["operations"]
    })
}

pub fn batch_description() -> String {
    format!(
        "Run up to {BATCH_MAX_OPERATIONS} independent tool operations in one call and get all their results back together. \
         Use it whenever the next several reads, searches or listings are already known: it saves one model turn per operation. \
         Read-only operations run concurrently; edits and shell commands run one at a time, in order. \
         Each operation's output is capped at {} KB and the whole result at {} KB; overflow is saved to a file the result names, which read can open. \
         Not for images, and batch/agent cannot nest.",
        VISIBLE_PER_OPERATION / 1024,
        VISIBLE_TOTAL / 1024
    )
}

struct Operation {
    tool: String,
    args: Value,
}

fn parse_operations(input: &Value) -> Result<Vec<Operation>, String> {
    let Some(items) = input.get("operations").and_then(Value::as_array) else {
        return Err("Missing operations: pass an array of { tool, args }".into());
    };
    if items.is_empty() {
        return Err("operations is empty".into());
    }
    if items.len() > BATCH_MAX_OPERATIONS {
        return Err(format!(
            "Too many operations ({}); the limit is {BATCH_MAX_OPERATIONS}. Split the batch.",
            items.len()
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let tool = item
                .get("tool")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("operations[{index}] has no tool name"))?;
            let args = item
                .get("args")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            if !args.is_object() {
                return Err(format!("operations[{index}].args must be an object"));
            }
            Ok(Operation {
                tool: tool.to_string(),
                args,
            })
        })
        .collect()
}

/// A short rendering of the arguments for the result header: the first
/// few scalar fields, no bodies.
fn args_summary(args: &Value) -> String {
    let Value::Object(map) = args else {
        return String::new();
    };
    if map.is_empty() {
        return String::new();
    }
    // The keys that identify an operation come first, whatever the
    // serializer's order; then whatever else fits.
    const LEAD: &[&str] = &[
        "path", "pattern", "command", "url", "query", "server", "uri",
    ];
    let ordered = LEAD
        .iter()
        .filter_map(|key| map.get_key_value(*key))
        .chain(map.iter().filter(|(key, _)| !LEAD.contains(&key.as_str())))
        .take(4);
    let mut parts = Vec::new();
    for (key, value) in ordered {
        let shown = match value {
            Value::String(text) => {
                let one_line = text.replace('\n', " ");
                format!("{key}={:?}", cut_at_char_boundary(&one_line, 60))
            }
            Value::Number(number) => format!("{key}={number}"),
            Value::Bool(flag) => format!("{key}={flag}"),
            _ => format!("{key}=…"),
        };
        parts.push(shown);
    }
    parts.join(" ")
}

impl Agent {
    /// Run one `batch` call. `id` is the batch's own tool-call id; each
    /// operation reports to the post hook as `id#n`.
    pub(crate) fn run_batch(&self, cwd: &Path, id: &str, input: &Value) -> ToolResult {
        let operations = match parse_operations(input) {
            Ok(operations) => operations,
            Err(message) => {
                return ToolResult {
                    content: message,
                    is_error: true,
                    details: None,
                }
            }
        };
        let sequential = self.tool_execution_mode == crate::ToolExecutionMode::Sequential;
        let mut scheduled = Vec::with_capacity(operations.len());
        for (index, operation) in operations.iter().enumerate() {
            // An interrupted batch stops asking: the operations left are
            // reported as not run, the way the top-level loop reports them.
            if self.abort_requested() {
                break;
            }
            let op_id = format!("{id}#{}", index + 1);
            let preparation =
                self.prepare_tool_call(cwd, &op_id, &operation.tool, &operation.args, 1);
            let lane = match &preparation {
                crate::turn::Preparation::Ready { lane } => *lane,
                crate::turn::Preparation::Wait { lane, .. } => *lane,
                crate::turn::Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
            };
            let agent: &Agent = self;
            let (tool, args) = (operation.tool.clone(), operation.args.clone());
            scheduled.push(crate::scheduler::ScheduledCall {
                lane,
                run: Box::new(move || {
                    let mut result = match preparation {
                        crate::turn::Preparation::Immediate(result) => result,
                        crate::turn::Preparation::Wait { call_id, .. } => {
                            agent.wait_for_tool_call(&call_id)
                        }
                        crate::turn::Preparation::Ready { .. } => {
                            agent.run_prepared_call(cwd, &op_id, &tool, &args, 1)
                        }
                    };
                    if let Some(hook) = &agent.post_tool {
                        result = (hook.0)(&op_id, cwd, &tool, &args, result);
                    }
                    result
                }),
            });
        }
        let abort = self.abort_signal.clone();
        let (results, report) = crate::scheduler::run_lanes(
            scheduled,
            sequential,
            crate::scheduler::MAX_TOOL_PARALLELISM,
            abort.as_deref(),
            |_| {},
        );
        crate::stats::SharedCounters::add(&self.counters.batch_operations, results.len() as u64);
        let rendered = self.render_batch(id, &operations, results, report);
        if let Some(files) = rendered
            .details
            .as_ref()
            .and_then(|details| details.get("evidenceFiles"))
            .and_then(Value::as_u64)
        {
            crate::stats::SharedCounters::add(&self.counters.evidence_files, files);
        }
        rendered
    }

    fn render_batch(
        &self,
        id: &str,
        operations: &[Operation],
        results: Vec<ToolResult>,
        report: crate::scheduler::ScheduleReport,
    ) -> ToolResult {
        let mut body = String::new();
        let mut rows = Vec::with_capacity(operations.len());
        let mut any_error = false;
        let mut visible_left = VISIBLE_TOTAL;
        let mut evidence_files = 0_u64;
        for (index, operation) in operations.iter().enumerate() {
            let number = index + 1;
            let head = {
                let summary = args_summary(&operation.args);
                if summary.is_empty() {
                    format!("[{number}] {}", operation.tool)
                } else {
                    format!("[{number}] {} {summary}", operation.tool)
                }
            };
            let Some(result) = results.get(index) else {
                body.push_str(&format!("{head} → not run (interrupted)\n\n"));
                rows.push(serde_json::json!({
                    "tool": operation.tool,
                    "status": "skipped",
                }));
                continue;
            };
            any_error |= result.is_error;
            let status = if result.is_error { "error" } else { "ok" };
            let has_image = result.details.as_ref().is_some_and(|details| {
                details.get("image").is_some() || details.get("images").is_some()
            });
            let full = &result.content;
            let cap = VISIBLE_PER_OPERATION.min(visible_left);
            let shown = cut_at_char_boundary(full, cap);
            let truncated = shown.len() < full.len();
            let mut note = String::new();
            let mut evidence_path = None;
            if truncated {
                let saved = self
                    .evidence
                    .as_ref()
                    .and_then(|store| store.store(&format!("batch-{id}-{number}"), full).ok());
                note = match &saved {
                    Some(path) => {
                        evidence_files += 1;
                        format!(
                            "\n… [{} more chars; full output saved to {} — read it with offset/limit]",
                            full.len() - shown.len(),
                            path.display()
                        )
                    }
                    None => format!(
                        "\n… [{} more chars truncated; run this operation alone for the full output]",
                        full.len() - shown.len()
                    ),
                };
                evidence_path = saved.map(|path| path.display().to_string());
            }
            if has_image {
                note.push_str("\n[image omitted: read it directly, outside a batch]");
            }
            visible_left = visible_left.saturating_sub(shown.len());
            body.push_str(&format!(
                "{head} → {status} ({} chars)\n{shown}{note}\n\n",
                full.len()
            ));
            let mut row = serde_json::json!({
                "tool": operation.tool,
                "status": status,
                "chars": full.len(),
                "truncated": truncated,
            });
            if let Some(path) = evidence_path {
                row["evidence"] = Value::String(path);
            }
            rows.push(row);
        }
        let ran = results.len();
        let summary = format!(
            "batch: {ran}/{} operations ran, {} concurrent group{}, {}\n\n",
            operations.len(),
            report.parallel_groups,
            if report.parallel_groups == 1 { "" } else { "s" },
            if any_error { "some failed" } else { "all ok" }
        );
        ToolResult {
            content: format!("{summary}{}", body.trim_end()),
            // A batch with one failed operation is still an answer: the
            // model reads each status. Only a batch that ran nothing at all
            // is an error.
            is_error: ran == 0,
            details: Some(serde_json::json!({
                "batch": true,
                "operations": rows,
                "parallelGroups": report.parallel_groups,
                "maxGroupWidth": report.max_group_width,
                "evidenceFiles": evidence_files,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_are_validated() {
        assert!(parse_operations(&serde_json::json!({})).is_err());
        assert!(parse_operations(&serde_json::json!({"operations": []})).is_err());
        let too_many: Vec<Value> = (0..17)
            .map(|_| serde_json::json!({"tool": "ls", "args": {}}))
            .collect();
        assert!(parse_operations(&serde_json::json!({"operations": too_many})).is_err());
        assert!(
            parse_operations(&serde_json::json!({"operations": [{"tool": "", "args": {}}]}))
                .is_err()
        );
        assert!(
            parse_operations(&serde_json::json!({"operations": [{"tool": "ls", "args": 3}]}))
                .is_err()
        );
        let ok = parse_operations(&serde_json::json!({"operations": [{"tool": "ls"}]})).unwrap();
        assert_eq!(ok[0].tool, "ls");
        assert!(ok[0].args.is_object());
    }

    #[test]
    fn args_summary_is_short_and_scalar() {
        let summary = args_summary(&serde_json::json!({
            "path": "a.rs",
            "content": "x".repeat(500),
            "limit": 3,
            "edits": [1, 2],
            "extra": true,
        }));
        assert!(summary.starts_with("path=\"a.rs\""), "{summary}");
        assert!(summary.contains("content=\"xxxx"), "{summary}");
        assert!(!summary.contains(&"x".repeat(100)), "{summary}");
        assert!(summary.len() < 200, "{summary}");
    }
}
