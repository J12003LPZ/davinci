//! Live progress of delegated workers, for the lead's transcript.
//!
//! No TypeScript counterpart. Claude Code shows a running subagent under its
//! call — the last few tool calls, `+N more tool uses`, then `Done (7 tool
//! uses · 23.4k tokens · 41s)`, and a `├─` tree when several run at once.
//! The worker's own agent loop feeds a [`ProgressReporter`]; every change is
//! re-emitted on the parent's event sink as [`AgentEvent::SubagentProgress`],
//! keyed by the parent's `agent` tool call, so hosts draw it without knowing
//! anything about the worker.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::AgentEvent;

/// How many recent tool calls a snapshot keeps.
pub const RECENT_CALLS: usize = 8;

/// How often a streaming reply re-estimates its tokens for the live count.
const STREAM_ESTIMATE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentProgressState {
    #[default]
    Running,
    /// Spawned in the background; its result arrives later as a report.
    Background,
    Done,
    Failed,
}

/// One worker's progress, as the lead's host draws it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProgress {
    pub agent_id: String,
    pub label: String,
    /// Position among the workers of one `agent` call, and how many there are.
    pub index: usize,
    pub total: usize,
    /// `oneshot`, `background` or `teammate`.
    pub mode: String,
    pub state: SubagentProgressState,
    pub tool_uses: u64,
    pub tokens: u64,
    /// Newest last, at most [`RECENT_CALLS`]: `Read(src/lib.rs)`.
    pub recent: Vec<String>,
    pub elapsed_ms: u64,
}

struct Inner {
    progress: SubagentProgress,
    started: Instant,
    /// Tokens of finished replies, as the provider reported them.
    settled: u64,
    /// Estimate for the reply still streaming; replaced by its reported
    /// usage when it ends.
    streaming: u64,
    last_estimate: Option<Instant>,
}

/// Collects one worker's activity and forwards it to the lead.
#[derive(Clone)]
pub struct ProgressReporter {
    tool_call_id: String,
    sink: crate::EventSink,
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for ProgressReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressReporter")
            .field("tool_call_id", &self.tool_call_id)
            .finish()
    }
}

impl ProgressReporter {
    pub fn new(
        tool_call_id: impl Into<String>,
        sink: crate::EventSink,
        agent_id: impl Into<String>,
        label: impl Into<String>,
        position: (usize, usize),
        mode: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            sink,
            inner: Arc::new(Mutex::new(Inner {
                progress: SubagentProgress {
                    agent_id: agent_id.into(),
                    label: label.into(),
                    index: position.0,
                    total: position.1,
                    mode: mode.into(),
                    ..SubagentProgress::default()
                },
                started: Instant::now(),
                settled: 0,
                streaming: 0,
                last_estimate: None,
            })),
        }
    }

    fn update(&self, change: impl FnOnce(&mut SubagentProgress)) {
        let snapshot = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            change(&mut inner.progress);
            inner.progress.elapsed_ms = inner.started.elapsed().as_millis() as u64;
            inner.progress.clone()
        };
        (self.sink.0)(&AgentEvent::SubagentProgress {
            tool_call_id: self.tool_call_id.clone(),
            progress: snapshot,
        });
    }

    /// The worker exists; draw its row before its first tool call.
    pub fn started(&self) {
        self.update(|_| {});
    }

    pub fn tool_started(&self, tool_name: &str, args: &Value) {
        let summary = call_summary(tool_name, args);
        self.update(|progress| {
            progress.tool_uses += 1;
            progress.recent.push(summary);
            let excess = progress.recent.len().saturating_sub(RECENT_CALLS);
            progress.recent.drain(..excess);
        });
    }

    pub fn add_tokens(&self, tokens: u64) {
        self.update_tokens(|inner| {
            inner.settled += tokens;
            inner.streaming = 0;
            inner.last_estimate = None;
            true
        });
    }

    /// A reply is streaming: count its estimated tokens now, so the lead sees
    /// the worker's usage climb while it works, not only after each reply.
    /// Re-estimated at most every [`STREAM_ESTIMATE_INTERVAL`].
    pub fn streaming(&self, message: &davinci_ai::ChatMessage) {
        self.update_tokens(|inner| {
            if inner
                .last_estimate
                .is_some_and(|at| at.elapsed() < STREAM_ESTIMATE_INTERVAL)
            {
                return false;
            }
            inner.last_estimate = Some(Instant::now());
            let estimate = crate::compaction::estimate_tokens(message);
            if estimate <= inner.streaming {
                return false;
            }
            inner.streaming = estimate;
            true
        });
    }

    /// Apply `change` to the token counters and emit a snapshot only when it
    /// says the visible count moved.
    fn update_tokens(&self, change: impl FnOnce(&mut Inner) -> bool) {
        let snapshot = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let before = inner.progress.tokens;
            if !change(&mut inner) {
                return;
            }
            inner.progress.tokens = inner.settled + inner.streaming;
            if inner.progress.tokens == before {
                return;
            }
            inner.progress.elapsed_ms = inner.started.elapsed().as_millis() as u64;
            inner.progress.clone()
        };
        (self.sink.0)(&AgentEvent::SubagentProgress {
            tool_call_id: self.tool_call_id.clone(),
            progress: snapshot,
        });
    }

    pub fn backgrounded(&self) {
        self.update(|progress| progress.state = SubagentProgressState::Background);
    }

    pub fn finish(&self, success: bool) {
        self.update(|progress| {
            progress.state = if success {
                SubagentProgressState::Done
            } else {
                SubagentProgressState::Failed
            }
        });
    }

    /// Feed one event of the worker's own loop.
    pub fn observe(&self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => self.tool_started(tool_name, args),
            AgentEvent::MessageUpdate { message, .. } if message.role == "assistant" => {
                self.streaming(message)
            }
            AgentEvent::MessageEnd { message } if message.role == "assistant" => {
                // A reply without reported usage keeps its streamed estimate.
                let reported = message_tokens(message);
                let tokens = if reported > 0 {
                    reported
                } else {
                    crate::compaction::estimate_tokens(message)
                };
                self.add_tokens(tokens)
            }
            _ => {}
        }
    }

    pub fn snapshot(&self) -> SubagentProgress {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .progress
            .clone()
    }
}

/// Tokens one assistant message used: `usage.totalTokens`, or the sum of its
/// parts when the provider did not report a total.
pub fn message_tokens(message: &davinci_ai::ChatMessage) -> u64 {
    let Some(usage) = message.extra.get("usage") else {
        return 0;
    };
    let field = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    match usage.get("totalTokens").and_then(Value::as_u64) {
        Some(total) if total > 0 => total,
        _ => field("input") + field("output") + field("cacheRead") + field("cacheWrite"),
    }
}

/// `Read(src/lib.rs)`, `Search("pattern")`, `Bash(cargo test)` — one call,
/// the way Claude Code names it under a running subagent.
pub fn call_summary(tool_name: &str, args: &Value) -> String {
    let first = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|key| args.get(*key).and_then(Value::as_str))
            .map(|value| {
                let line = value.lines().next().unwrap_or("").trim();
                let mut clipped: String = line.chars().take(60).collect();
                if line.chars().count() > 60 {
                    clipped.push('…');
                }
                clipped
            })
            .unwrap_or_default()
    };
    let (name, argument) = match tool_name {
        "read" => ("Read", first(&["path"])),
        "write" => ("Write", first(&["path"])),
        "edit" | "notebook_edit" => ("Update", first(&["path"])),
        "apply_patch" => ("Update", String::from("patch")),
        "ls" => ("List", first(&["path"])),
        "grep" | "find" => ("Search", format!("\"{}\"", first(&["pattern"]))),
        "bash" | "powershell" | "exec_command" => ("Bash", first(&["command", "cmd"])),
        "web_fetch" => ("Fetch", first(&["url"])),
        "web_search" => ("WebSearch", format!("\"{}\"", first(&["query"]))),
        "batch" => (
            "Batch",
            format!(
                "{} calls",
                args.get("operations")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)
            ),
        ),
        other => (
            other,
            first(&["path", "pattern", "query", "command", "url"]),
        ),
    };
    if argument.is_empty() {
        name.to_string()
    } else {
        format!("{name}({argument})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collecting() -> (crate::EventSink, Arc<Mutex<Vec<SubagentProgress>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let store = seen.clone();
        let sink = crate::EventSink(Arc::new(move |event: &AgentEvent| {
            if let AgentEvent::SubagentProgress { progress, .. } = event {
                store.lock().unwrap().push(progress.clone());
            }
        }));
        (sink, seen)
    }

    #[test]
    fn a_worker_reports_calls_tokens_and_completion() {
        let (sink, seen) = collecting();
        let reporter = ProgressReporter::new("call-1", sink, "a1", "map auth", (0, 1), "oneshot");
        reporter.started();
        reporter.observe(&AgentEvent::ToolExecutionStart {
            tool_call_id: "t".into(),
            tool_name: "read".into(),
            args: serde_json::json!({"path": "src/auth.rs"}),
        });
        let mut answer = davinci_ai::ChatMessage::text("assistant", "done");
        answer.extra.insert(
            "usage".into(),
            serde_json::json!({"input": 1000, "output": 200, "cacheRead": 50}),
        );
        reporter.observe(&AgentEvent::MessageEnd { message: answer });
        reporter.finish(true);
        let seen = seen.lock().unwrap();
        let last = seen.last().unwrap();
        assert_eq!(seen.first().unwrap().state, SubagentProgressState::Running);
        assert_eq!(last.state, SubagentProgressState::Done);
        assert_eq!(last.tool_uses, 1);
        assert_eq!(last.tokens, 1250);
        assert_eq!(last.recent, vec!["Read(src/auth.rs)".to_string()]);
        assert_eq!(last.label, "map auth");
    }

    #[test]
    fn streaming_replies_count_live_then_settle_to_reported_usage() {
        let (sink, seen) = collecting();
        let reporter = ProgressReporter::new("call-1", sink, "a1", "map auth", (0, 1), "oneshot");
        let partial = davinci_ai::ChatMessage::text("assistant", "word ".repeat(400));
        reporter.streaming(&partial);
        let live = seen.lock().unwrap().last().unwrap().tokens;
        assert!(live > 0, "a streaming reply counts before it ends");
        // Throttled: an immediate second delta does not re-emit.
        let emitted = seen.lock().unwrap().len();
        reporter.streaming(&partial);
        assert_eq!(seen.lock().unwrap().len(), emitted);
        let mut answer = davinci_ai::ChatMessage::text("assistant", "done");
        answer.extra.insert(
            "usage".into(),
            serde_json::json!({"input": 3000, "output": 500}),
        );
        reporter.observe(&AgentEvent::MessageEnd { message: answer });
        assert_eq!(seen.lock().unwrap().last().unwrap().tokens, 3500);
    }

    #[test]
    fn recent_calls_are_bounded_and_newest_last() {
        let (sink, _) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        for index in 0..20 {
            reporter.tool_started("grep", &serde_json::json!({"pattern": format!("p{index}")}));
        }
        let snapshot = reporter.snapshot();
        assert_eq!(snapshot.tool_uses, 20);
        assert_eq!(snapshot.recent.len(), RECENT_CALLS);
        assert_eq!(snapshot.recent.last().unwrap(), "Search(\"p19\")");
    }

    #[test]
    fn calls_are_named_like_claude_code() {
        assert_eq!(
            call_summary("bash", &serde_json::json!({"command": "cargo test\nmore"})),
            "Bash(cargo test)"
        );
        assert_eq!(call_summary("todo", &serde_json::json!({})), "todo");
        assert_eq!(
            call_summary("edit", &serde_json::json!({"path": "a.rs"})),
            "Update(a.rs)"
        );
    }
}
