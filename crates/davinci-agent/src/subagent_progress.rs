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

/// Where a worker's token count comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// Nothing reported and nothing estimated yet: shown as `unknown`, never 0.
    #[default]
    Unknown,
    /// Every counted reply carried the provider's own usage.
    Reported,
    /// At least part of the count is DaVinci's estimate (shown with `~`).
    Estimated,
}

/// Cumulative tokens one worker has used, input and output counted once per
/// reply. Cache reads and writes are kept apart (they overlap the input the
/// provider already counts) and are never added to the headline number.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Replies whose provider reported a total but no input/output split.
    pub unsplit: u64,
    /// Estimated tokens: replies without reported usage, plus the reply that
    /// is still streaming. Replaced, never added twice, when usage arrives.
    pub estimated: u64,
    /// Replies counted from reported usage.
    pub reported_replies: u64,
}

impl SubagentUsage {
    /// The headline count: input + output (+ estimates).
    pub fn total(&self) -> u64 {
        self.input + self.output + self.unsplit + self.estimated
    }

    pub fn source(&self) -> UsageSource {
        if self.estimated > 0 {
            UsageSource::Estimated
        } else if self.reported_replies > 0 {
            UsageSource::Reported
        } else {
            UsageSource::Unknown
        }
    }
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
    /// `usage.total()`: input + output (+ estimates). Zero with an unknown
    /// source means no usage yet, not a measured zero.
    pub tokens: u64,
    #[serde(default)]
    pub usage: SubagentUsage,
    /// Newest last, at most [`RECENT_CALLS`]: `Read(src/lib.rs)`.
    pub recent: Vec<String>,
    pub elapsed_ms: u64,
}

struct Inner {
    progress: SubagentProgress,
    started: Instant,
    /// Tokens of finished replies.
    settled: SubagentUsage,
    /// Estimate for the reply still streaming; replaced by its reported
    /// usage when it ends.
    streaming: u64,
    last_estimate: Option<Instant>,
    /// Elapsed time once the worker is finished: it stops counting there.
    frozen_ms: Option<u64>,
}

impl Inner {
    fn elapsed_ms(&self) -> u64 {
        self.frozen_ms
            .unwrap_or_else(|| self.started.elapsed().as_millis() as u64)
    }
}

/// Collects one worker's activity and forwards it to the lead.
#[derive(Clone)]
pub struct ProgressReporter {
    tool_call_id: String,
    sink: crate::EventSink,
    inner: Arc<Mutex<Inner>>,
    /// Where the newest snapshot is also kept, so the roster and RPC read the
    /// same numbers the transcript does.
    registry: Option<(crate::runtime::RuntimeRegistry, crate::runtime::AgentId)>,
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
            registry: None,
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
                settled: SubagentUsage::default(),
                streaming: 0,
                last_estimate: None,
                frozen_ms: None,
            })),
        }
    }

    /// Also keep every snapshot in `registry` under `agent_id`.
    pub fn with_registry(
        mut self,
        registry: crate::runtime::RuntimeRegistry,
        agent_id: crate::runtime::AgentId,
    ) -> Self {
        self.registry = Some((registry, agent_id));
        self
    }

    /// `with_registry` when there is a runtime to keep the numbers in.
    pub fn with_registry_if(
        self,
        registry: Option<crate::runtime::RuntimeRegistry>,
        agent_id: crate::runtime::AgentId,
    ) -> Self {
        match registry {
            Some(registry) => self.with_registry(registry, agent_id),
            None => self,
        }
    }

    fn emit(&self, snapshot: SubagentProgress) {
        if let Some((registry, agent_id)) = &self.registry {
            registry.set_progress(*agent_id, snapshot.clone());
        }
        (self.sink.0)(&AgentEvent::SubagentProgress {
            tool_call_id: self.tool_call_id.clone(),
            progress: snapshot,
        });
    }

    fn update(&self, change: impl FnOnce(&mut SubagentProgress)) {
        let snapshot = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            change(&mut inner.progress);
            inner.progress.elapsed_ms = inner.elapsed_ms();
            inner.progress.clone()
        };
        self.emit(snapshot);
    }

    /// The worker exists; draw its row before its first tool call.
    pub fn started(&self) {
        let label = self.snapshot().label;
        self.note(
            crate::runtime::WorkerActivityKind::Started,
            None,
            &format!("started {label}"),
            None,
        );
        self.update(|_| {});
    }

    /// Add one entry to the worker's activity history, when it has one.
    fn note(
        &self,
        kind: crate::runtime::WorkerActivityKind,
        tool_call_id: Option<&str>,
        summary: &str,
        outcome: Option<&str>,
    ) {
        if let Some((registry, agent_id)) = &self.registry {
            registry.activity().append(
                *agent_id,
                registry.get_generation(agent_id),
                kind,
                tool_call_id,
                summary,
                outcome,
            );
        }
    }

    pub fn tool_started(&self, tool_name: &str, args: &Value) {
        self.tool_started_with_id(None, tool_name, args);
    }

    fn tool_started_with_id(&self, tool_call_id: Option<&str>, tool_name: &str, args: &Value) {
        let summary = call_summary(tool_name, args);
        self.note(
            crate::runtime::WorkerActivityKind::ToolStarted,
            tool_call_id,
            &summary,
            None,
        );
        self.update(|progress| {
            progress.tool_uses += 1;
            progress.recent.push(summary);
            let excess = progress.recent.len().saturating_sub(RECENT_CALLS);
            progress.recent.drain(..excess);
        });
    }

    /// A reply ended with the provider's usage: it replaces the streaming
    /// estimate for that reply.
    pub fn add_reply_usage(&self, usage: &Value) {
        self.update_tokens(|inner| {
            let field = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let (input, output) = (field("input"), field("output"));
            inner.settled.input += input;
            inner.settled.output += output;
            inner.settled.cache_read += field("cacheRead");
            inner.settled.cache_write += field("cacheWrite");
            if input + output == 0 {
                inner.settled.unsplit += field("totalTokens");
            }
            inner.settled.reported_replies += 1;
            inner.streaming = 0;
            inner.last_estimate = None;
            true
        });
    }

    /// A reply ended without usage: keep an estimate for it, tagged as one.
    pub fn add_estimated_reply(&self, tokens: u64) {
        self.update_tokens(|inner| {
            inner.settled.estimated += tokens;
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
            let before = inner.progress.usage.clone();
            if !change(&mut inner) {
                return;
            }
            let mut usage = inner.settled.clone();
            usage.estimated += inner.streaming;
            inner.progress.tokens = usage.total();
            if usage == before {
                return;
            }
            inner.progress.usage = usage;
            inner.progress.elapsed_ms = inner.elapsed_ms();
            inner.progress.clone()
        };
        self.emit(snapshot);
    }

    pub fn backgrounded(&self) {
        self.update(|progress| progress.state = SubagentProgressState::Background);
    }

    pub fn finish(&self, success: bool) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Time stops at the first terminal report, whatever follows.
            if inner.frozen_ms.is_none() {
                inner.frozen_ms = Some(inner.started.elapsed().as_millis() as u64);
            }
        }
        self.note(
            crate::runtime::WorkerActivityKind::Finished,
            None,
            if success { "finished" } else { "failed" },
            Some(if success { "done" } else { "failed" }),
        );
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
                tool_call_id,
                tool_name,
                args,
            } => self.tool_started_with_id(Some(tool_call_id), tool_name, args),
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                is_error,
                ..
            } => self.note(
                crate::runtime::WorkerActivityKind::ToolFinished,
                Some(tool_call_id),
                tool_name,
                Some(if *is_error { "error" } else { "ok" }),
            ),
            AgentEvent::MessageUpdate { message, .. } if message.role == "assistant" => {
                self.streaming(message)
            }
            AgentEvent::MessageEnd { message } if message.role == "assistant" => {
                match message.extra.get("usage") {
                    Some(usage) if message_tokens(message) > 0 => self.add_reply_usage(usage),
                    // A reply without reported usage keeps an estimate, tagged.
                    _ => self.add_estimated_reply(crate::compaction::estimate_tokens(message)),
                }
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
        // Input + output; the 50 cached tokens are shown apart, not added.
        assert_eq!(last.tokens, 1200);
        assert_eq!(last.usage.cache_read, 50);
        assert_eq!(last.usage.source(), UsageSource::Reported);
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

    fn reply_with_usage(usage: serde_json::Value) -> davinci_ai::ChatMessage {
        let mut reply = davinci_ai::ChatMessage::text("assistant", "ok");
        reply.extra.insert("usage".into(), usage);
        reply
    }

    #[test]
    fn unknown_usage_is_never_shown_as_a_measured_zero() {
        let (sink, seen) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        reporter.started();
        let first = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(first.tokens, 0);
        assert_eq!(first.usage.source(), UsageSource::Unknown);
        // A reply whose usage is all zero reports nothing either.
        reporter.observe(&AgentEvent::MessageEnd {
            message: reply_with_usage(serde_json::json!({"input": 0, "output": 0})),
        });
        let after = reporter.snapshot();
        assert_eq!(after.usage.reported_replies, 0, "{after:?}");
        assert_eq!(after.usage.source(), UsageSource::Estimated);
    }

    #[test]
    fn a_streaming_estimate_is_replaced_by_reported_usage_not_added_to_it() {
        let (sink, _) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        reporter.streaming(&davinci_ai::ChatMessage::text(
            "assistant",
            "word ".repeat(400),
        ));
        let live = reporter.snapshot();
        assert!(live.tokens > 0);
        assert_eq!(live.usage.source(), UsageSource::Estimated);
        reporter.observe(&AgentEvent::MessageEnd {
            message: reply_with_usage(serde_json::json!({"input": 100, "output": 20})),
        });
        let settled = reporter.snapshot();
        assert_eq!(settled.tokens, 120, "{settled:?}");
        assert_eq!(settled.usage.estimated, 0);
        assert_eq!(settled.usage.source(), UsageSource::Reported);
    }

    #[test]
    fn a_reply_without_usage_is_counted_as_an_estimate_and_says_so() {
        let (sink, _) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        reporter.observe(&AgentEvent::MessageEnd {
            message: davinci_ai::ChatMessage::text("assistant", "word ".repeat(200)),
        });
        let snapshot = reporter.snapshot();
        assert!(snapshot.tokens > 0);
        assert_eq!(snapshot.usage.source(), UsageSource::Estimated);
        // Reported replies afterwards keep the estimate flagged: the total
        // is still partly an estimate.
        reporter.observe(&AgentEvent::MessageEnd {
            message: reply_with_usage(serde_json::json!({"input": 10, "output": 5})),
        });
        assert_eq!(reporter.snapshot().usage.source(), UsageSource::Estimated);
    }

    #[test]
    fn a_total_without_a_split_still_counts_once() {
        let (sink, _) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        reporter.observe(&AgentEvent::MessageEnd {
            message: reply_with_usage(serde_json::json!({"totalTokens": 900})),
        });
        let snapshot = reporter.snapshot();
        assert_eq!(snapshot.tokens, 900);
        assert_eq!(snapshot.usage.source(), UsageSource::Reported);
    }

    #[test]
    fn elapsed_time_stops_when_the_worker_finishes() {
        let (sink, seen) = collecting();
        let reporter = ProgressReporter::new("c", sink, "a", "x", (0, 1), "oneshot");
        reporter.started();
        std::thread::sleep(Duration::from_millis(30));
        reporter.finish(true);
        let at_finish = seen.lock().unwrap().last().unwrap().elapsed_ms;
        assert!(at_finish >= 30, "{at_finish}");
        std::thread::sleep(Duration::from_millis(60));
        // Anything reported after the end (a late token count, a snapshot)
        // carries the frozen time.
        reporter.add_estimated_reply(10);
        reporter.started();
        assert_eq!(seen.lock().unwrap().last().unwrap().elapsed_ms, at_finish);
        assert_eq!(reporter.snapshot().elapsed_ms, at_finish);
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
