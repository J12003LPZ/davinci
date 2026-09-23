//! The davinci TUI, driven by the real agent.
//!
//! The shell in `davinci_tui::davinci` knows nothing about agents; this module owns
//! the loop that turns a sent composer line into an agent turn and the agent's
//! events back into transcript blocks (`docs/ui/design.md` §6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use davinci_agent::{
    Agent, AgentEvent, DecisionHostRequest, DecisionHostResponse, EventSink, PermissionMode,
    ToolApprovalDecision, ToolApprovalRequest,
};
use davinci_tui::davinci::model::{
    Ask, CatalogRow, Choice, Compaction, CorpusItem, Credential, Entry, ExportLedger, FailedRun,
    Finding, GovernorCounter, GovernorSheet, GovernorStored, GraphRunSheet, GraphTask, Hunk,
    HunkKind, KeymapGroup, McpServerRow, McpSheet, Model, ModelItem, Overlay, PermissionRow,
    PickerItem, PlanStep, ProviderRow, ResumeRow, ReviewFile, ReviewSheet, Screen, SecurityScan,
    SettingRow, Severity, Step, ThinkingRow, Tone, TreeNode, VectorIndex, WorkflowRow,
    WorkflowsSheet, Working, WorkshopSheet,
};
use davinci_tui::davinci::theme::State;

use crate::extension_host::ExtensionHost;
mod graph_feedback;
mod graph_setup;

struct NativeApproval {
    request: ToolApprovalRequest,
    reply: mpsc::Sender<NativeApprovalAnswer>,
    live: Arc<AtomicBool>,
    expires_at_ms: u64,
    deadline: Instant,
}

#[derive(Debug, PartialEq, Eq)]
struct NativeApprovalAnswer {
    decision: ToolApprovalDecision,
    instructions: Option<String>,
}

impl From<ToolApprovalDecision> for NativeApprovalAnswer {
    fn from(decision: ToolApprovalDecision) -> Self {
        Self {
            decision,
            instructions: None,
        }
    }
}

impl NativeApprovalAnswer {
    fn into_reply(
        self,
        challenge: &davinci_agent::approval::ApprovalChallenge,
    ) -> davinci_agent::approval::ApprovalReply {
        let mut reply =
            davinci_agent::approval::ApprovalReply::from_legacy(challenge, self.decision);
        if self.decision == ToolApprovalDecision::Deny && self.instructions.is_some() {
            reply.choice_id = "deny_with_instructions".into();
            reply.instructions = self.instructions;
        }
        reply
    }
}

impl NativeApproval {
    fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
            && davinci_session::now_ms() < self.expires_at_ms
            && Instant::now() < self.deadline
    }
}

// Declared inside the scoped UI closure: an error/panic cancels the approval
// waiter before std::thread::scope joins its worker.
struct ApprovalUiExit(Option<Arc<AtomicBool>>);

impl ApprovalUiExit {
    fn new(abort: Arc<AtomicBool>) -> Self {
        Self(Some(abort))
    }
}

impl Drop for ApprovalUiExit {
    fn drop(&mut self) {
        if let Some(abort) = &self.0 {
            abort.store(true, Ordering::Relaxed);
        }
    }
}

fn wait_native_approval(
    request: &ToolApprovalRequest,
    expires_at_ms: u64,
    tx: &mpsc::Sender<NativeApproval>,
    abort: &AtomicBool,
) -> NativeApprovalAnswer {
    let available = Duration::from_millis(expires_at_ms.saturating_sub(davinci_session::now_ms()));
    let Some(deadline) = Instant::now().checked_add(available) else {
        return ToolApprovalDecision::Deny.into();
    };
    if abort.load(Ordering::Relaxed) || available.is_zero() {
        return ToolApprovalDecision::Deny.into();
    }
    let (reply, rx) = mpsc::channel();
    let live = Arc::new(AtomicBool::new(true));
    let pending = NativeApproval {
        request: request.clone(),
        reply,
        live: live.clone(),
        expires_at_ms,
        deadline,
    };
    if tx.send(pending).is_err() {
        return ToolApprovalDecision::Deny.into();
    }
    let decision = loop {
        if abort.load(Ordering::Relaxed)
            || davinci_session::now_ms() >= expires_at_ms
            || Instant::now() >= deadline
        {
            break ToolApprovalDecision::Deny.into();
        }
        match rx.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        ) {
            Ok(decision) => {
                if abort.load(Ordering::Relaxed)
                    || davinci_session::now_ms() >= expires_at_ms
                    || Instant::now() >= deadline
                {
                    break ToolApprovalDecision::Deny.into();
                }
                break decision;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break ToolApprovalDecision::Deny.into(),
        }
    };
    live.store(false, Ordering::Relaxed);
    decision
}

struct NativeDecision {
    request: DecisionHostRequest,
    reply: mpsc::Sender<DecisionHostResponse>,
    live: Arc<AtomicBool>,
    expires_at_ms: u64,
    deadline: Instant,
}

impl NativeDecision {
    fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
            && davinci_session::now_ms() < self.expires_at_ms
            && Instant::now() < self.deadline
    }
}

fn wait_native_decision(
    request: DecisionHostRequest,
    expires_at_ms: u64,
    tx: &mpsc::Sender<NativeDecision>,
    abort: &AtomicBool,
) -> DecisionHostResponse {
    let available = Duration::from_millis(expires_at_ms.saturating_sub(davinci_session::now_ms()));
    let Some(deadline) = Instant::now().checked_add(available) else {
        return DecisionHostResponse::Cancelled;
    };
    if abort.load(Ordering::Relaxed) || available.is_zero() {
        return DecisionHostResponse::Cancelled;
    }
    let (reply, rx) = mpsc::channel();
    let live = Arc::new(AtomicBool::new(true));
    let pending = NativeDecision {
        request,
        reply,
        live: live.clone(),
        expires_at_ms,
        deadline,
    };
    if tx.send(pending).is_err() {
        return DecisionHostResponse::Unavailable;
    }
    let decision = loop {
        if abort.load(Ordering::Relaxed) {
            break DecisionHostResponse::Cancelled;
        }
        if davinci_session::now_ms() >= expires_at_ms || Instant::now() >= deadline {
            break DecisionHostResponse::Timeout;
        }
        match rx.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        ) {
            Ok(decision) => {
                if abort.load(Ordering::Relaxed) {
                    break DecisionHostResponse::Cancelled;
                }
                if davinci_session::now_ms() >= expires_at_ms || Instant::now() >= deadline {
                    break DecisionHostResponse::Timeout;
                }
                break decision;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break DecisionHostResponse::Cancelled;
            }
        }
    };
    live.store(false, Ordering::Relaxed);
    decision
}

/// Which instrument a tool belongs to (design.md §5). Shell execution is
/// Manus; everything else the agent reaches for is Instrumenta.
pub fn instrument_of(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" | "powershell" | "job_output" | "job_kill" => "manus",
        name if name.starts_with("memory") => "memoria",
        name if name.starts_with("graph") => "grafo",
        name if name.starts_with("agent_") || name.starts_with("task_") => "societas",
        name if name.starts_with("workflow_") => "opus",
        _ => "instrumenta",
    }
}

/// The glyph a tool call carries while it runs and once it is done.
pub fn state_of(tool_name: &str, failed: bool) -> State {
    if failed {
        return State::Failed;
    }
    match tool_name {
        "read" | "ls" | "job_output" | "mcp_read" | "agent_status" | "task_list"
        | "workflow_status" => State::Read,
        "grep" | "find" | "web_fetch" | "web_search" => State::Search,
        name if name.starts_with("memory") => State::Search,
        "edit" | "write" | "notebook_edit" => State::Delta,
        _ => State::Done,
    }
}

/// `docs.rs/similar/latest` from a URL: the scheme says nothing on a row.
fn bare_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    without_scheme.trim_end_matches('/').to_string()
}

fn mcp_target(tool_name: &str, args: &serde_json::Value) -> String {
    let rest = tool_name.strip_prefix("mcp__").unwrap_or(tool_name);
    let (server, tool) = rest.split_once("__").unwrap_or((rest, rest));
    let first = args.as_object().and_then(|map| {
        map.values().find_map(|value| match value {
            serde_json::Value::String(text) if !text.is_empty() => {
                Some(clip(text.lines().next().unwrap_or(""), 40))
            }
            _ => None,
        })
    });
    match first {
        Some(arg) => format!("mcp {server} {tool} {arg}"),
        None => format!("mcp {server} {tool}"),
    }
}

fn job_id_of(args: &serde_json::Value) -> String {
    match args.get("jobId").or_else(|| args.get("id")) {
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::String(s)) => s.trim().trim_start_matches("job ").to_string(),
        _ => "?".into(),
    }
}

/// `read crates\pi-tui\src\lib.rs`, `cargo test -p pi-session`, and so on —
/// the target half of a tool line.
pub fn target_of(tool_name: &str, args: &serde_json::Value) -> String {
    let field = |key: &str| -> String {
        args.get(key)
            .and_then(serde_json::Value::as_str)
            .map(|value| {
                let line = value.lines().next().unwrap_or("");
                clip(line, 60)
            })
            .unwrap_or_default()
    };
    let background = args
        .get("background")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    match tool_name {
        "read" => format!("read {}", field("path")),
        "ls" => format!("list {}", field("path")),
        "write" => format!("write {}", field("path")),
        "edit" => format!("edit {}", field("path")),
        "grep" => format!("search \"{}\"", field("pattern")),
        "find" => format!("find \"{}\"", field("pattern")),
        "bash" | "powershell" if background => format!("{} · background", field("command")),
        "bash" | "powershell" => field("command"),
        "web_fetch" => format!(
            "fetch {}",
            clip(
                &bare_url(
                    args.get("url")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                ),
                60
            )
        ),
        "web_search" => format!("search web \"{}\"", field("query")),
        "todo" => {
            let count = args
                .get("items")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            format!("plan · {}", plural_of(count, "item"))
        }
        "job_output" => format!("job {} output", job_id_of(args)),
        "job_kill" => format!("kill job {}", job_id_of(args)),
        "notebook_edit" => {
            let cell = args
                .get("cell")
                .and_then(serde_json::Value::as_u64)
                .map(|cell| format!(" · cell {cell}"))
                .unwrap_or_default();
            format!("edit {}{cell}", field("path"))
        }
        "agent" => {
            let label = args
                .get("description")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| clip(text, 60))
                .unwrap_or_else(|| field("prompt"));
            format!("agent {label}")
        }
        "mcp_read" => format!("mcp {} {}", field("server"), field("uri")),
        name if name.starts_with("mcp__") => mcp_target(name, args),
        other => {
            let detail = field("query");
            if detail.is_empty() {
                other.to_string()
            } else {
                format!("{other} \"{detail}\"")
            }
        }
    }
}

/// The verb the Studio ledger shows for a step in progress (design.md §5).
pub fn verb_of(tool_name: &str) -> &'static str {
    match tool_name {
        "read" | "ls" | "job_output" | "mcp_read" => "studying",
        "grep" | "find" | "web_fetch" | "web_search" => "surveying",
        "bash" | "powershell" | "job_kill" => "testing",
        "edit" | "write" | "notebook_edit" => "constructing",
        "todo" => "planning",
        "agent" => "delegating",
        name if name.starts_with("memory") => "recalling",
        name if name.starts_with("graph") => "tracing",
        _ => "working",
    }
}

fn plural_of(count: usize, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

/// `1.84s`, `0.42s` — the duration a tool line ends with.
pub fn duration_of(elapsed: Duration) -> String {
    format!("{:.2}s", elapsed.as_secs_f64())
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// A tool failure expands to at most four indented lines and keeps the exit
/// code (design.md §6). Live drawing uses `Entry::Tool.output` and the
/// transcript's four-row cap; this helper pins the clip for tests.
#[cfg(test)]
pub fn failure_lines(result: &serde_json::Value) -> Vec<String> {
    let text = match result {
        serde_json::Value::String(text) => text.clone(),
        other => other
            .get("output")
            .or_else(|| other.get("error"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| other.to_string()),
    };
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .take(4)
        .map(|line| clip(line, 100))
        .collect()
}

/// The text a tool result carries, whatever shape the event wrapped it in.
fn gate_denied(result: &serde_json::Value) -> bool {
    // The gate marks its refusals in `details.denied`; the text prefixes
    // cover a result that arrives as a bare string (a resumed session).
    // A command whose own stderr says `Permission denied` is a failure,
    // not a refusal, and does not start with the gate's prefix.
    if result
        .pointer("/details/denied")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return true;
    }
    let text = result_text(result);
    text.starts_with("Permission denied: ") || text.starts_with("plan mode:")
}

fn result_text(result: &serde_json::Value) -> String {
    match result {
        serde_json::Value::String(text) => text.clone(),
        other => other
            .get("output")
            .or_else(|| other.get("content"))
            .or_else(|| other.get("error"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
    }
}

/// What a finished call came back with, in the fewest words that still say it:
/// `412 lines`, `8 matches`, `+31 -8`. `None` where the call has nothing to
/// report beyond having happened — the duration already says that.
pub fn summary_of(
    tool_name: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
) -> Option<String> {
    let rows = || {
        let text = result_text(result);
        let count = text.lines().filter(|line| !line.trim().is_empty()).count();
        (count > 0).then_some(count)
    };
    let plural = |count: usize, unit: &str| {
        if count == 1 {
            format!("1 {unit}")
        } else {
            format!("{count} {unit}s")
        }
    };
    let background = args
        .get("background")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    match tool_name {
        "read" | "web_fetch" | "mcp_read" => rows().map(|count| plural(count, "line")),
        name if name.starts_with("mcp__") => rows().map(|count| plural(count, "line")),
        "ls" => rows().map(|count| plural(count, "entry").replace("entrys", "entries")),
        "grep" | "find" => Some(plural(rows().unwrap_or(0), "match").replace("matchs", "matches")),
        // `Started background job 3: …` — the row names the job.
        "bash" | "powershell" if background => result_text(result)
            .strip_prefix("Started background job ")
            .and_then(|rest| rest.split(':').next())
            .map(|id| format!("job {id}")),
        "bash" | "powershell" => rows().map(|count| plural(count, "line")),
        "web_search" => {
            let text = result_text(result);
            let hits = text
                .lines()
                .filter(|line| {
                    line.split_once(". ")
                        .is_some_and(|(n, _)| n.chars().all(|ch| ch.is_ascii_digit()))
                })
                .count();
            Some(plural(hits, "result"))
        }
        "todo" => {
            let items = args.get("items")?.as_array()?;
            let done = items
                .iter()
                .filter(|item| {
                    item.get("status")
                        .and_then(serde_json::Value::as_str)
                        .and_then(davinci_agent::TodoStatus::parse)
                        == Some(davinci_agent::TodoStatus::Done)
                })
                .count();
            Some(if items.is_empty() {
                "cleared".into()
            } else {
                format!("{done} of {} done", items.len())
            })
        }
        // `…\n\n[job 1 running · 12.4s]` — the lines above, and the word.
        "job_output" => {
            let text = result_text(result);
            let status = text
                .lines()
                .last()
                .and_then(|line| line.strip_prefix('['))
                .and_then(|line| line.split(" · ").next())
                .and_then(|line| line.splitn(3, ' ').nth(2))
                .map(str::to_string);
            let count = text
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with("[job "))
                .count();
            Some(match status {
                Some(status) => format!("{} · {status}", plural(count, "line")),
                None => plural(count, "line"),
            })
        }
        "notebook_edit" => args
            .get("cell")
            .and_then(serde_json::Value::as_u64)
            .map(|cell| format!("cell {cell}")),
        // The edit tool reports the path it touched, not the shape of the
        // change, so the change is counted from what was asked for.
        "edit" => {
            let edits = args.get("edits")?.as_array()?;
            let (adds, dels) = edits.iter().fold((0usize, 0usize), |(adds, dels), edit| {
                let count = |key: &str| {
                    edit.get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(|text| text.lines().count())
                        .unwrap_or(0)
                };
                (adds + count("newText"), dels + count("oldText"))
            });
            Some(format!("+{adds} -{dels}"))
        }
        "write" => args
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(|content| plural(content.lines().count(), "line")),
        _ => None,
    }
}

/// The transcript state a turn builds up, so events can find the block they
/// belong to without searching the transcript.
#[derive(Default)]
struct Turn {
    /// `tool_call_id` -> (index in the transcript, when it started, what it
    /// was asked to do). The arguments are kept because several outcomes —
    /// the shape of an edit, the size of a write — are only in the request.
    open: Vec<(String, usize, Instant, serde_json::Value)>,
    studio: Option<usize>,
    said_something: bool,
    /// What each finished call came to, in order — the recovery sheet (`6c`)
    /// replays it when the turn is interrupted.
    log: Vec<(State, String, String)>,
    /// Output tokens from the assistant messages that have already finished.
    /// The message still streaming is added on top, so the working line's
    /// counter climbs across a whole tool-calling run rather than resetting at
    /// every step.
    streamed: u64,
    /// The prose entry the current message is streaming into, so each text
    /// delta lands in the block it belongs to.
    prose: Option<usize>,
    /// The live reasoning entry of the current message, and when it began.
    thinking: Option<usize>,
    thinking_started: Option<Instant>,
    /// `hideThinkingBlock`: reasoning is neither shown live nor kept.
    hide_thinking: bool,
    /// The model keeps its own ledger (`todo`): the STUDIO box shows it
    /// instead of a synthesised step per tool, and the tool in hand is
    /// noted on the active item.
    ledger: bool,
    /// The active ledger item's own target, kept while a tool's is shown.
    ledger_target: Option<Option<String>>,
}

impl Turn {
    fn start_tool(
        &mut self,
        model: &mut Model,
        tool_call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        // A Δ block is a block of its own; the next call starts a new one.
        if matches!(model.transcript.last(), Some(Entry::Delta { .. })) {
            model.transcript.push(Entry::Gap);
        }
        let index = model.transcript.len();
        model.transcript.push(Entry::tool(
            state_of(tool_name, false),
            instrument_of(tool_name),
            &target_of(tool_name, args),
            None,
        ));
        self.open.push((
            tool_call_id.to_string(),
            index,
            Instant::now(),
            args.clone(),
        ));
        if tool_name == "todo" {
            if let Ok(list) = davinci_agent::TodoList::from_args(args) {
                self.take_ledger(model, &list);
                return;
            }
        }
        self.push_step(model, tool_name, args);
    }

    /// The model's list becomes the ledger: the STUDIO box and the plan
    /// sheet both show it, and tools stop adding steps of their own.
    fn take_ledger(&mut self, model: &mut Model, list: &davinci_agent::TodoList) {
        model.plan = plan_from_todos(list);
        let steps = steps_from_todos(list);
        self.ledger = !steps.is_empty();
        self.ledger_target = None;
        match self
            .studio
            .and_then(|index| model.transcript.get_mut(index))
        {
            Some(Entry::Studio(existing)) => *existing = steps,
            _ if !steps.is_empty() => {
                self.studio = Some(model.transcript.len());
                model.transcript.push(Entry::Studio(steps));
            }
            _ => {}
        }
    }

    fn end_tool(
        &mut self,
        model: &mut Model,
        tool_call_id: &str,
        tool_name: &str,
        result: &serde_json::Value,
        is_error: bool,
        details: Option<&serde_json::Value>,
    ) {
        let Some(position) = self
            .open
            .iter()
            .position(|(id, _, _, _)| id == tool_call_id)
        else {
            return;
        };
        let (_, index, started, args) = self.open.remove(position);
        let outcome = (!is_error)
            .then(|| summary_of(tool_name, &args, result))
            .flatten();
        self.log.push((
            state_of(tool_name, is_error),
            target_of(tool_name, &args),
            duration_of(started.elapsed()),
        ));
        if let Some(Entry::Tool {
            state,
            duration,
            summary,
            output,
            ..
        }) = model.transcript.get_mut(index)
        {
            let gated = is_error && gate_denied(result);
            *state = if gated {
                State::Done
            } else {
                state_of(tool_name, is_error)
            };
            *duration = Some(duration_of(started.elapsed()));
            *summary = if gated {
                Some("denied".into())
            } else {
                outcome
            };
            // What came back stays on the line: a failure draws its first
            // rows, `ctrl+t` draws any call's.
            *output = davinci_tui::davinci::model::tool_output_rows(&result_text(result));
        }
        // An edit shows its change as a Δ block right under its line, from
        // the diff the tool returned (phase 3, "Highlighted diffs").
        if !is_error {
            if let Some(diff) = details
                .and_then(|details| details.get("diff"))
                .and_then(serde_json::Value::as_str)
                .filter(|diff| !diff.trim().is_empty())
            {
                let path = details
                    .and_then(|details| details.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        args.get("path")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                let (adds, dels, hunks) = hunks_from_diff(diff);
                model.transcript.insert(index + 1, Entry::Gap);
                model.transcript.insert(
                    index + 2,
                    Entry::Delta {
                        path,
                        adds,
                        dels,
                        hunks,
                    },
                );
                self.shift(index, 2);
                if let Some(Entry::Tool { summary, .. }) = model.transcript.get_mut(index) {
                    if summary.is_none() {
                        *summary = Some(format!("+{adds} -{dels}"));
                    }
                }
            }
        }
        self.finish_step(model);
    }

    /// Keep the recorded indices valid when detail rows are spliced in.
    fn shift(&mut self, after: usize, by: usize) {
        for (_, index, _, _) in self.open.iter_mut() {
            if *index > after {
                *index += by;
            }
        }
        if let Some(studio) = self.studio.as_mut() {
            if *studio > after {
                *studio += by;
            }
        }
        if let Some(prose) = self.prose.as_mut() {
            if *prose > after {
                *prose += by;
            }
        }
        if let Some(thinking) = self.thinking.as_mut() {
            if *thinking > after {
                *thinking += by;
            }
        }
    }

    /// A new assistant message begins: whatever the last one streamed into
    /// is closed, and the next delta opens a block of its own.
    fn begin_message(&mut self, model: &mut Model) {
        self.settle_thinking(model);
        self.prose = None;
        self.thinking = None;
        self.thinking_started = None;
    }

    fn append_text(&mut self, model: &mut Model, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.settle_thinking(model);
        let index = match self.prose {
            Some(index) if matches!(model.transcript.get(index), Some(Entry::Prose(_))) => index,
            _ => {
                model.transcript.push(Entry::Gap);
                model.transcript.push(Entry::Prose(String::new()));
                let index = model.transcript.len() - 1;
                self.prose = Some(index);
                index
            }
        };
        if let Some(Entry::Prose(text)) = model.transcript.get_mut(index) {
            // The first delta of a block often opens with the newline that
            // separated it from a tool call; the block is already its own
            // paragraph.
            if text.is_empty() {
                text.push_str(delta.trim_start());
            } else {
                text.push_str(delta);
            }
            if !text.trim().is_empty() {
                self.said_something = true;
            }
        }
    }

    /// The final text of the block being streamed, or a whole block at once
    /// for a message that was never streamed.
    fn set_text(&mut self, model: &mut Model, text: &str) {
        let text = text.trim();
        match self.prose {
            Some(index) if matches!(model.transcript.get(index), Some(Entry::Prose(_))) => {
                if let Some(Entry::Prose(existing)) = model.transcript.get_mut(index) {
                    if !text.is_empty() {
                        *existing = text.to_string();
                    }
                }
            }
            _ if !text.is_empty() => {
                self.settle_thinking(model);
                model.transcript.push(Entry::Gap);
                model.transcript.push(Entry::prose(text));
                self.prose = Some(model.transcript.len() - 1);
            }
            _ => {}
        }
        if !text.is_empty() {
            self.said_something = true;
        }
    }

    fn append_thinking(&mut self, model: &mut Model, delta: &str) {
        if self.hide_thinking {
            return;
        }
        let index = match self.thinking {
            Some(index) if matches!(model.transcript.get(index), Some(Entry::Thinking { .. })) => {
                index
            }
            _ => {
                model.transcript.push(Entry::Gap);
                model.transcript.push(Entry::thinking("", true, 0));
                let index = model.transcript.len() - 1;
                self.thinking = Some(index);
                self.thinking_started = Some(Instant::now());
                index
            }
        };
        if let Some(Entry::Thinking { text, .. }) = model.transcript.get_mut(index) {
            text.push_str(delta);
        }
    }

    /// Close the live reasoning row, if any: it collapses to its one-line
    /// summary with how long the model spent.
    fn settle_thinking(&mut self, model: &mut Model) {
        let Some(index) = self.thinking.take() else {
            return;
        };
        let seconds = self
            .thinking_started
            .take()
            .map(|started| started.elapsed().as_secs())
            .unwrap_or(0);
        if let Some(Entry::Thinking {
            live,
            seconds: took,
            ..
        }) = model.transcript.get_mut(index)
        {
            *live = false;
            *took = seconds;
        }
    }

    /// The call is waiting on the user: say so on its ledger row and its
    /// tool line, so a turn that has stopped moving reads as a question and
    /// not as a hang.
    fn await_approval(&mut self, model: &mut Model, request: &ToolApprovalRequest) {
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            if let Some(step) = steps
                .iter_mut()
                .rev()
                .find(|step| step.state == State::Active)
            {
                let target = step.target.take().unwrap_or_default();
                step.target = Some(format!("{target} · awaiting approval"));
            }
        }
        if let Some(Entry::Tool { summary, .. }) = self
            .open
            .iter()
            .find(|(id, _, _, _)| *id == request.tool_call_id)
            .and_then(|(_, index, _, _)| model.transcript.get_mut(*index))
        {
            *summary = Some("awaiting approval".into());
        }
    }

    /// The user has spoken: the waiting marks come off. What happened next is
    /// the tool's own outcome — `end_tool` writes it — except a rule saved
    /// for good, which is worth a line of its own.
    fn settle_approval(
        &mut self,
        model: &mut Model,
        request: &ToolApprovalRequest,
        remembered: Option<&str>,
    ) {
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            for step in steps.iter_mut() {
                if let Some(target) = step.target.as_mut() {
                    if let Some(bare) = target.strip_suffix(" · awaiting approval") {
                        *target = bare.to_string();
                    }
                }
            }
        }
        if let Some(Entry::Tool { summary, .. }) = self
            .open
            .iter()
            .find(|(id, _, _, _)| *id == request.tool_call_id)
            .and_then(|(_, index, _, _)| model.transcript.get_mut(*index))
        {
            *summary = None;
        }
        if let Some(rule) = remembered {
            model.transcript.push(Entry::tool(
                State::Done,
                "instrumenta",
                &format!("remembered {rule} · .pi/settings.json"),
                None,
            ));
        }
    }

    fn push_step(&mut self, model: &mut Model, tool_name: &str, args: &serde_json::Value) {
        if self.ledger {
            // The model's own ledger holds the steps; the tool in hand is
            // noted on the active item, `◉ add the branch · edit src/x.rs`.
            if let Some(Entry::Studio(steps)) =
                self.studio.and_then(|i| model.transcript.get_mut(i))
            {
                if let Some(step) = steps.iter_mut().find(|step| step.state == State::Active) {
                    if self.ledger_target.is_none() {
                        self.ledger_target = Some(step.target.clone());
                    }
                    step.target = Some(target_of(tool_name, args));
                }
            }
            return;
        }
        let step = Step::new(
            State::Active,
            verb_of(tool_name),
            Some(&target_of(tool_name, args)),
        );
        match self
            .studio
            .and_then(|index| model.transcript.get_mut(index))
        {
            Some(Entry::Studio(steps)) => {
                for step in steps.iter_mut() {
                    if step.state == State::Active {
                        step.state = State::Done;
                    }
                }
                steps.push(step);
            }
            _ => {
                self.studio = Some(model.transcript.len());
                model.transcript.push(Entry::Studio(vec![step]));
            }
        }
    }

    fn finish_step(&mut self, model: &mut Model) {
        if self.ledger {
            if let Some(original) = self.ledger_target.take() {
                if let Some(Entry::Studio(steps)) =
                    self.studio.and_then(|i| model.transcript.get_mut(i))
                {
                    if let Some(step) = steps.iter_mut().find(|step| step.state == State::Active) {
                        step.target = original;
                    }
                }
            }
            return;
        }
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            if let Some(step) = steps.last_mut() {
                step.state = State::Done;
            }
        }
    }

    fn close(&mut self, model: &mut Model, interrupted: bool) {
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            for step in steps.iter_mut() {
                // The model's own ledger keeps its active item: the plan is
                // where it stands, not where the turn stopped.
                if step.state == State::Active && !self.ledger {
                    step.state = if interrupted {
                        State::Skipped
                    } else {
                        State::Done
                    };
                }
            }
        }
        self.open.clear();
        self.studio = None;
        self.settle_thinking(model);
        self.prose = None;
    }
}

/// Roughly what a run of text cost, for the working line's counter while the
/// provider has not reported a usage figure yet. Four characters a token is
/// the same approximation `estimate_context_tokens` uses.
fn estimate_tokens(text: &str) -> u64 {
    text.chars().count() as u64 / 4
}

/// The model's ledger as STUDIO steps: `✓` done, `◉` active, `○` pending.
pub fn steps_from_todos(list: &davinci_agent::TodoList) -> Vec<Step> {
    list.items
        .iter()
        .map(|item| Step::new(todo_state(item.status), &item.text, None))
        .collect()
}

/// The same ledger on the `1c` plan sheet, numbered.
pub fn plan_from_todos(list: &davinci_agent::TodoList) -> Vec<PlanStep> {
    list.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            PlanStep::new(
                &davinci_tui::davinci::views::disegno::roman(index + 1),
                todo_state(item.status),
                &item.text,
                None,
            )
        })
        .collect()
}

fn todo_state(status: davinci_agent::TodoStatus) -> State {
    match status {
        davinci_agent::TodoStatus::Done => State::Done,
        davinci_agent::TodoStatus::Active => State::Active,
        davinci_agent::TodoStatus::Pending => State::Queued,
    }
}

/// The Δ block of an edit's diff, as the tool returns it (`+12 text`,
/// `-12 text`, ` 12 text`, `    ...`): the numbers come off, additions and
/// deletions are counted, `...` becomes a context row of its own.
pub fn hunks_from_diff(diff: &str) -> (u32, u32, Vec<Hunk>) {
    let mut adds = 0u32;
    let mut dels = 0u32;
    let mut hunks = Vec::new();
    for line in diff.lines() {
        let mut chars = line.chars();
        let Some(sign) = chars.next() else {
            continue;
        };
        let rest = chars.as_str();
        // `NN text` — the number column, then one space, then the text.
        let trimmed = rest.trim_start();
        let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
        let text = if digits > 0 {
            trimmed[digits..]
                .strip_prefix(' ')
                .unwrap_or(&trimmed[digits..])
        } else {
            trimmed
        };
        let kind = match sign {
            '+' => {
                adds += 1;
                HunkKind::Add
            }
            '-' => {
                dels += 1;
                HunkKind::Del
            }
            _ => HunkKind::Context,
        };
        if kind == HunkKind::Context && text.trim() == "..." {
            hunks.push(Hunk::new(HunkKind::Context, "…"));
        } else {
            hunks.push(Hunk::new(kind, text));
        }
    }
    (adds, dels, hunks)
}

/// A finished background job on the transcript: `⎿ ✓ job 1 finished ·
/// cargo build · exit 0 · 31.2s`, with its last rows behind the line, so
/// the news reaches the user before it reaches the model.
pub fn job_row(notice: &davinci_agent::JobNotice) -> Entry {
    let state = if notice.status.succeeded() {
        State::Done
    } else {
        State::Failed
    };
    Entry::tool(
        state,
        "manus",
        &format!("job {} finished · {}", notice.id, clip(&notice.command, 50)),
        Some(&davinci_agent::jobs::format_elapsed(notice.elapsed)),
    )
    .summarised(&notice.status.describe())
    .with_output(&notice.tail.join("\n"))
}

/// Finished jobs the user has not seen become rows; the count of running
/// ones feeds the status bar. Safe mid-turn: rows go at the end.
pub fn poll_jobs(jobs: &Arc<Mutex<davinci_agent::JobBook>>, model: &mut Model) {
    let (notices, running) = {
        let mut book = jobs.lock().unwrap_or_else(|err| err.into_inner());
        (book.take_unseen(), book.running())
    };
    model.jobs_running = running;
    for notice in &notices {
        if !model.running && !matches!(model.transcript.last(), Some(Entry::Tool { .. })) {
            model.transcript.push(Entry::Gap);
        }
        model.transcript.push(job_row(notice));
    }
}

/// `high`, `medium`, … for the working line. `off` says nothing, because a
/// model that is not thinking has no effort to report.
fn thinking_effort(agent: &Agent) -> Option<String> {
    let level = agent.thinking_level.as_str();
    (level != "off").then(|| level.to_string())
}

/// Fold one agent event into the transcript.
fn apply(model: &mut Model, turn: &mut Turn, event: &AgentEvent) {
    match event {
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => turn.start_tool(model, tool_call_id, tool_name, args),

        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
            details,
        } => {
            turn.end_tool(
                model,
                tool_call_id,
                tool_name,
                result,
                *is_error,
                details.as_ref(),
            );
            if !is_error {
                if let Some(governor) = details.as_ref().and_then(|d| d.get("tokenGovernor")) {
                    let notice = if governor
                        .get("compressed")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        let original = governor
                            .get("originalBytes")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let withheld = original.saturating_sub(result_text(result).len() as u64);
                        Some(format!(
                            "{tool_name} compressed · {} bytes withheld · original saved",
                            sheet_thousands(withheld)
                        ))
                    } else if governor
                        .get("deduplicated")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        Some("Unchanged read reused · duplicate output avoided".to_string())
                    } else {
                        None
                    };
                    if let Some(notice) = notice {
                        model.governor_notice = Some((notice, std::time::Instant::now()));
                    }
                }
            }
        }

        AgentEvent::MessageStart { message } if message.role == "assistant" => {
            turn.begin_message(model);
        }

        // Every delta lands in the transcript as it arrives: text into the
        // block being streamed, reasoning into its live row. The stream's own
        // counter feeds the working line — a provider that reports usage
        // mid-stream is believed; one that does not gets the character
        // estimate, so the number climbs either way.
        AgentEvent::MessageUpdate {
            message,
            assistant_message_event,
        } => {
            use davinci_ai::AssistantMessageEvent as Ev;
            match assistant_message_event {
                Ev::TextStart { .. } => turn.settle_thinking(model),
                Ev::TextDelta { delta, .. } => turn.append_text(model, delta),
                Ev::TextEnd { content, .. } => turn.set_text(model, content),
                Ev::ThinkingDelta { delta, .. } => turn.append_thinking(model, delta),
                Ev::ThinkingEnd { content, .. } => {
                    if let Some(Entry::Thinking { text, .. }) = turn
                        .thinking
                        .and_then(|index| model.transcript.get_mut(index))
                    {
                        if !content.trim().is_empty() {
                            *text = content.clone();
                        }
                    }
                    turn.settle_thinking(model);
                }
                _ => {}
            }
            if let Some(working) = model.working.as_mut() {
                let reported = assistant_message_event
                    .message()
                    .usage
                    .as_ref()
                    .map(|usage| usage.output)
                    .unwrap_or_default();
                let estimated = estimate_tokens(&davinci_ai::content_text(&message.content));
                working.tokens = turn.streamed + reported.max(estimated);
            }
        }

        AgentEvent::MessageEnd { message } if message.role == "assistant" => {
            let text = davinci_ai::content_text(&message.content);
            turn.streamed += estimate_tokens(&text);
            if let Some(working) = model.working.as_mut() {
                working.tokens = working.tokens.max(turn.streamed);
            }
            turn.settle_thinking(model);
            turn.set_text(model, &text);
            turn.prose = None;
        }

        AgentEvent::AutoRetryStart {
            attempt,
            max_attempts,
            error_message,
            ..
        } => {
            model.transcript.push(Entry::tool(
                State::Attention,
                "manus",
                &format!(
                    "retrying {attempt} of {max_attempts} · {}",
                    clip(error_message, 60)
                ),
                None,
            ));
        }

        _ => {}
    }
}

/// Run one turn, keeping the window painted and the composer's clock running.
///
/// `esc` and `ctrl+c` set the abort flag; every other key is ignored until the
/// turn ends, so a keystroke can never land in the middle of a tool call.
#[allow(clippy::too_many_arguments)]
/// A key pressed while a turn is running. The composer stays live so a
/// follow-up can be typed and queued; esc and ctrl+c stop the run, and every
/// other chord is ignored rather than being taken for text.
///
/// Returns `true` when the user pressed the interrupt again after the abort
/// was already requested — the worker is not answering the flag (a hung
/// provider read, a wedged extension), and the only way out left is to give
/// the terminal back and leave. That path restores the screen first, which is
/// what killing the process from outside never did.
#[must_use]
fn mid_turn_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    abort: &Arc<AtomicBool>,
) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if model.voice.blocks_send && davinci_tui::davinci::app::voice_send_key(model, &key) {
        return false;
    }
    if let Some(data) = davinci_tui::key_event_bytes(&key) {
        if model.keybindings.matches(&data, "davinci.tools.expand") {
            model.show_tool_output = !model.show_tool_output;
            return false;
        }
        if key.code != KeyCode::Esc
            && !(ctrl && key.code == KeyCode::Char('c'))
            && model.edit_composer_key(&data)
        {
            return false;
        }
    }
    match key.code {
        KeyCode::Esc => {
            let again = abort.swap(true, Ordering::Relaxed);
            model.interrupt();
            return again;
        }
        KeyCode::Char('c') if ctrl => {
            let again = abort.swap(true, Ordering::Relaxed);
            model.interrupt();
            return again;
        }
        KeyCode::Char('j') if ctrl => model.newline(),
        // Tool output can be opened while the turn still runs: that is when
        // a long read is most worth seeing.
        KeyCode::Char(_) if ctrl => {}
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            model.newline()
        }
        KeyCode::Enter => {
            model.queue();
        }
        KeyCode::Backspace => model.backspace(),
        KeyCode::Tab => {
            model.complete();
        }
        KeyCode::Char(ch) => model.type_char(&ch.to_string()),
        _ => {}
    }
    false
}

/// A turn and everything queued behind it, in the order it was typed.
///
/// Each queued line goes back through `on_line`, exactly as if it had been
/// sent by hand — which is what the legacy loop does by re-entering
/// `submit_user_message`. Sending the queue straight to `agent.prompt`
/// bypassed every check: a queued `/command` went to the provider verbatim.
fn run_turns(shell: &mut Shell<'_>) -> Next {
    let host = shell.host.clone();
    if let Err(err) = run_turn(
        shell.parsed,
        shell.agent,
        shell.model,
        shell.terminal,
        host,
        shell.voice,
    ) {
        return Next::Fail(err.to_string());
    }
    while !shell.model.queued.is_empty() {
        let line = shell.model.queued.remove(0);
        // What the composer's own submit pushes before a line is routed.
        shell.model.transcript.push(Entry::Gap);
        shell.model.transcript.push(Entry::user(&line));
        shell.model.transcript.push(Entry::Gap);
        shell.model.transcript.push(Entry::agent("davinci"));
        shell.model.running = true;
        match on_line(shell, &line) {
            Next::Go => {}
            other => return other,
        }
    }
    Next::Go
}

fn interrupted_recovery_aftermath() -> Vec<(State, String)> {
    vec![(
        State::Attention,
        "the active turn was interrupted; review partial results before retrying".into(),
    )]
}

fn run_turn(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    model: &mut Model,
    session: &mut davinci_tui::davinci::runtime::Session,
    host: Arc<Mutex<ExtensionHost>>,
    voice: &mut crate::voice_input::VoiceInput,
) -> std::io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>();
    let abort = Arc::new(AtomicBool::new(false));
    agent.abort_signal = Some(abort.clone());
    agent.event_sink = Some(EventSink(Arc::new(move |event: &AgentEvent| {
        let _ = event_tx.send(event.clone());
    })));

    let settings = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    let mut turn = Turn {
        hide_thinking: settings.hide_thinking_block.unwrap_or(false),
        ..Turn::default()
    };
    // A tool call the policy cannot decide crosses from the worker to this
    // loop as a request with its own reply line; the worker blocks on the
    // reply while the panel is up. Only a trusted project may be offered
    // "always": its settings file is the one that would be read back.
    let trusted = crate::settings::is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    {
        let host_guard = host.lock().unwrap_or_else(|err| err.into_inner());
        host_guard.native_cancel_learning_review();
        host_guard.set_project_trusted(trusted);
    }
    let cwd = agent.cwd.clone();
    let (approval_tx, approval_rx) = mpsc::channel::<NativeApproval>();
    let approval_abort = abort.clone();
    agent.approver = None;
    agent.approval_responder = Some(davinci_agent::approval::ApprovalResponder(Arc::new(
        move |request, challenge| {
            let decision = wait_native_approval(
                request,
                challenge.expires_at_ms,
                &approval_tx,
                &approval_abort,
            );
            decision.into_reply(challenge)
        },
    )));
    let mut approval: Option<NativeApproval> = None;
    let mut approval_project_allowed = trusted;
    let (decision_tx, decision_rx) = mpsc::channel::<NativeDecision>();
    let decision_abort = abort.clone();
    agent.tool_context.decision_responder =
        Some(davinci_agent::DecisionResponder::new(move |request| {
            let expires_at_ms = davinci_session::now_ms().saturating_add(300_000);
            wait_native_decision(request, expires_at_ms, &decision_tx, &decision_abort)
        }));
    let mut decision: Option<NativeDecision> = None;
    let mut last_tick = Instant::now();
    // The job book, read on every tick while the worker holds the agent.
    let jobs = agent.tool_context.jobs.clone();
    // The working line above the composer, for as long as the turn runs.
    let started = Instant::now();
    model.working = Some(Working {
        seconds: 0,
        tokens: 0,
        thinking: thinking_effort(agent),
        interrupting: false,
    });
    if model.terminal_progress {
        let _ = session.set_progress(true);
    }
    let crashed = Arc::new(AtomicBool::new(false));
    // What the worker came back with. A provider failure is not an event of its
    // own: it arrives as a `MessageUpdate` whose message stopped on
    // `StopReason::Error`, which `apply` does not read. Dropping the worker's
    // return value threw the only copy of that message away, and every failed
    // request — a refused key, an unreachable base URL, a 400 — read as "the
    // model returned no text".
    let mut failure: Option<String> = None;
    let mut reply = String::new();
    let mut scope_expansion = None;
    let scope_contract = agent.active_contract();

    std::thread::scope(|scope| -> std::io::Result<()> {
        let mut ui_exit = ApprovalUiExit::new(abort.clone());
        let worker = scope
            .spawn(|| crate::complete_prompt_with_host(parsed, agent, Some(host.clone()), false));

        loop {
            while let Ok(event) = event_rx.try_recv() {
                apply(model, &mut turn, &event);
            }
            if approval.as_ref().is_some_and(|pending| !pending.is_live()) {
                approval = None;
                model.approval_instructions = None;
                if model.overlay == Some(Overlay::Ask) {
                    model.overlay = None;
                }
            }
            if decision.as_ref().is_some_and(|pending| !pending.is_live()) {
                decision = None;
                model.decision_modal = None;
                if model.overlay == Some(Overlay::Ask) {
                    model.overlay = None;
                }
            }
            if approval.is_none() && decision.is_none() {
                if let Ok(pending) = approval_rx.try_recv() {
                    if !pending.is_live() {
                        continue;
                    }
                    approval_project_allowed = trusted;
                    let request = &pending.request;
                    turn.await_approval(model, request);
                    model.ask = permission_ask(request, trusted);
                    open_ask_overlay(model);
                    voice.cancel(model);
                    approval = Some(pending);
                } else if let Ok(pending) = decision_rx.try_recv() {
                    if !pending.is_live() {
                        continue;
                    }
                    open_decision_modal(model, &pending.request.question);
                    voice.cancel(model);
                    decision = Some(pending);
                }
            }
            if let Some(working) = model.working.as_mut() {
                working.seconds = started.elapsed().as_secs();
            }
            voice.tick(model, session.input_pending());
            session.draw(model)?;
            voice.drawn();
            if model.voice.active && !session.mic_visible() {
                voice.cancel(model);
            }

            if worker.is_finished() {
                break;
            }
            if let Some(event) = session.poll_event(Duration::from_millis(40))? {
                if let crossterm::event::Event::Key(key) = event {
                    if voice.key(model, key) {
                        continue;
                    }
                }
                match event {
                    crossterm::event::Event::Key(key)
                        if key.kind != crossterm::event::KeyEventKind::Release
                            && decision.is_some() =>
                    {
                        let _ = davinci_tui::davinci::app::handle_key(model, key);
                        let pending = decision.as_ref().unwrap();
                        if !pending.is_live() || abort.load(Ordering::Relaxed) {
                            let pending = decision.take().unwrap();
                            let _ = pending
                                .reply
                                .send(davinci_agent::DecisionHostResponse::Cancelled);
                            model.decision_modal = None;
                            if model.overlay == Some(Overlay::Ask) {
                                model.overlay = None;
                            }
                            continue;
                        }
                        if let Some(state) = model.decision_modal.as_ref() {
                            if state.submitted {
                                let pending = decision.take().expect("checked above");
                                let outcome = state.outcome.clone();
                                model.decision_modal = None;
                                model.overlay = None;
                                let response = match outcome {
                                    Some(davinci_tui::davinci::views::decision_modal::DecisionResult::Choice(choice_id)) => {
                                        davinci_agent::DecisionHostResponse::Reply(davinci_agent::DecisionHostReply {
                                            action: davinci_agent::decisions::HostDecisionAction::AnswerChoice(choice_id),
                                            host_event_id: format!("tui_{}", davinci_session::now_ms()),
                                            answered_at_ms: davinci_session::now_ms(),
                                        })
                                    }
                                    Some(davinci_tui::davinci::views::decision_modal::DecisionResult::Custom(custom_text)) => {
                                        davinci_agent::DecisionHostResponse::Reply(davinci_agent::DecisionHostReply {
                                            action: davinci_agent::decisions::HostDecisionAction::AnswerCustom(custom_text),
                                            host_event_id: format!("tui_{}", davinci_session::now_ms()),
                                            answered_at_ms: davinci_session::now_ms(),
                                        })
                                    }
                                    Some(davinci_tui::davinci::views::decision_modal::DecisionResult::Defer) => {
                                        davinci_agent::DecisionHostResponse::Reply(davinci_agent::DecisionHostReply {
                                            action: davinci_agent::decisions::HostDecisionAction::Defer,
                                            host_event_id: format!("tui_{}", davinci_session::now_ms()),
                                            answered_at_ms: davinci_session::now_ms(),
                                        })
                                    }
                                    Some(davinci_tui::davinci::views::decision_modal::DecisionResult::Cancel) => {
                                        davinci_agent::DecisionHostResponse::Cancelled
                                    }
                                    None => davinci_agent::DecisionHostResponse::Cancelled,
                                };
                                let _ = pending.reply.send(response);
                            }
                        }
                    }
                    crossterm::event::Event::Key(key)
                        if key.kind != crossterm::event::KeyEventKind::Release
                            && approval.is_some() =>
                    {
                        let choices = approval
                            .as_ref()
                            .expect("checked above")
                            .request
                            .host_choices(approval_project_allowed);
                        let decision = approval_answer_key(
                            model,
                            key,
                            &choices,
                            &approval.as_ref().expect("checked above").request,
                            &abort,
                        );
                        if davinci_ai::trace::enabled() {
                            davinci_ai::trace::log(&format!(
                                "davinci permission panel: decision {:?}",
                                decision.as_ref().map(|answer| answer.decision)
                            ));
                        }
                        if let Some(answer) = decision {
                            let pending = approval.take().expect("checked above");
                            if !pending.is_live() || abort.load(Ordering::Relaxed) {
                                model.approval_instructions = None;
                                if model.overlay == Some(Overlay::Ask) {
                                    model.overlay = None;
                                }
                                continue;
                            }
                            let request = &pending.request;
                            if let Some(decision) = persist_permission_choice(
                                model,
                                request,
                                answer.decision,
                                &cwd,
                                &mut approval_project_allowed,
                            ) {
                                let saved_rule = (decision == ToolApprovalDecision::AllowAlways)
                                    .then_some(request.session_rule.as_str());
                                turn.settle_approval(model, request, saved_rule);
                                let _ = pending.reply.send(NativeApprovalAnswer {
                                    decision,
                                    instructions: answer.instructions,
                                });
                            } else {
                                approval = Some(pending);
                            }
                        }
                    }
                    crossterm::event::Event::Key(key)
                        if key.kind != crossterm::event::KeyEventKind::Release =>
                    {
                        if mid_turn_key(model, key, &abort) {
                            // The abort flag was already up and the worker is
                            // not answering it: a hung read holds it somewhere
                            // with no timeout left to fire. Give the terminal
                            // back first, then leave — the session file is
                            // already written through the last completed step.
                            let _ = davinci_tui::davinci::runtime::restore();
                            eprintln!(
                                "pi: the turn would not stop (a hung provider or extension); the session file is intact"
                            );
                            std::process::exit(130);
                        }
                    }
                    crossterm::event::Event::Resize(width, height) => {
                        model.width = width.max(20);
                        model.height = height.max(4);
                    }
                    crossterm::event::Event::Paste(text) => {
                        if !voice.paste(model, &text) {
                            model.paste(&text);
                        }
                    }
                    crossterm::event::Event::Mouse(mouse) => {
                        if session.handle_model_mouse(model, mouse) {
                            voice.toggle(model);
                        }
                    }
                    _ => {}
                }
            }
            if last_tick.elapsed() >= davinci_tui::davinci::runtime::TICK {
                model.tick = model.tick.wrapping_add(1);
                last_tick = Instant::now();
                poll_jobs(&jobs, model);
            }
        }

        // A panicking worker used to be discarded, which showed up as a turn
        // that quietly returned nothing. Report it as the failure it is.
        match worker.join() {
            Err(_) => crashed.store(true, Ordering::Relaxed),
            Ok((text, events)) => {
                // The same scan `--print` runs before choosing an exit code,
                // which catches a stream that stopped on an error. A fault
                // found before the request went out — no model, no credential
                // — never reaches the sink at all: it is folded into the reply
                // with this prefix, and is the whole of it.
                failure = crate::print_text_exit(&events).1.or_else(|| {
                    text.strip_prefix("Provider error: ")
                        .map(|reason| reason.trim().to_string())
                });
                scope_expansion =
                    scope_expansion_preview_from_events(scope_contract.as_ref(), &events);
                reply = text;
            }
        }
        if let Some(pending) = decision.take() {
            let _ = pending
                .reply
                .send(davinci_agent::DecisionHostResponse::Cancelled);
            model.decision_modal = None;
            if model.overlay == Some(Overlay::Ask) {
                model.overlay = None;
            }
        }
        ui_exit.0 = None;
        Ok(())
    })?;

    if crashed.load(Ordering::Relaxed) {
        // The worker's panic ran the panic hook, which restored the terminal
        // from that thread — the message went to the real screen, but the
        // alternate screen and raw mode went with it. Take them back before
        // drawing another frame, or the loop paints over the user's shell.
        session.reacquire()?;
    }

    while let Ok(event) = event_rx.try_recv() {
        apply(model, &mut turn, &event);
    }
    let interrupted = abort.load(Ordering::Relaxed);
    // The turn is over: the working line goes with it.
    model.working = None;
    turn.close(model, interrupted);

    for entry in turn_outcome(
        crashed.load(Ordering::Relaxed),
        interrupted,
        turn.said_something,
        failure,
        &reply,
    ) {
        model.transcript.push(entry);
    }

    if interrupted {
        // The `6c` sheet: what the interrupted turn came to — what ran, what
        // is kept, what is still on disk — so ctrl+c never reads as a hole.
        let prompt = agent
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| clip(davinci_ai::content_text(&message.content).trim(), 60))
            .unwrap_or_default();
        let kept = davinci_agent::estimate_context_tokens(&agent.messages);
        let error = turn
            .log
            .iter()
            .rev()
            .find(|(state, _, _)| *state == State::Failed)
            .map(|(_, text, _)| text.clone())
            .unwrap_or_default();
        model.failed_run = Some(FailedRun {
            prompt,
            tools: turn.log.clone(),
            error,
            files_written: String::new(),
            retry: String::new(),
            kept: format!(
                "{} tokens in context",
                davinci_tui::davinci::views::chrome::thousands(kept)
            ),
            billed: "in the next /session stats".into(),
            aftermath: interrupted_recovery_aftermath(),
            runtime_report: None,
        });
        open_sheet(model, Screen::Recovery);
    }

    agent.abort_signal = None;
    agent.event_sink = None;
    agent.approver = None;
    agent.approval_responder = None;
    agent.tool_context.decision_responder = None;
    // A question the worker never got an answer to (it was interrupted under
    // the panel) is closed with it.
    if approval.take().is_some() && model.overlay == Some(Overlay::Ask) {
        model.overlay = None;
    }
    if decision.take().is_some() {
        model.decision_modal = None;
        if model.overlay == Some(Overlay::Ask) {
            model.overlay = None;
        }
    }
    model.approval_instructions = None;
    model.running = false;
    if model.terminal_progress {
        let _ = session.set_progress(false);
    }
    if !interrupted {
        if let Some(preview) = scope_expansion.take() {
            let task_guard = capture_scope_expansion_task_guard(agent, &preview)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            if let Some(guidance) = resolve_scope_expansion_modal(
                agent,
                model,
                session,
                &preview,
                task_guard.as_ref(),
                voice,
            )? {
                model.transcript.push(Entry::Detail(guidance.clone()));
                model.queued.insert(0, guidance);
            }
            let host_guard = host.lock().unwrap_or_else(|err| err.into_inner());
            crate::apply_graph_session_context(parsed, agent, &host_guard);
        }
    }

    // Extensions may have asked for rows while the turn ran. Taking the queue
    // rather than reading it is what keeps a `notify` from being replayed into
    // the transcript on every later turn.
    drain_ui_calls(model, &host);
    {
        let host_guard = host.lock().unwrap_or_else(|err| err.into_inner());
        for notice in host_guard.drain_learning_notifications() {
            model.transcript.push(Entry::Detail(notice));
        }
    }
    apply_cache_miss_notices(model, agent);
    Ok(())
}

/// The cache-miss notice the old chrome showed after a turn, when the user has
/// asked to see them.
fn apply_cache_miss_notices(model: &mut Model, agent: &Agent) {
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    if !stored.show_cache_miss_notices.unwrap_or(false) {
        return;
    }
    let Some(store) = agent.session.as_ref() else {
        return;
    };
    let waste = crate::cache_stats::compute_cache_waste(&store.entries, &0.3);
    if waste.missed_tokens <= 0 {
        return;
    }
    model.transcript.push(Entry::Gap);
    model.transcript.push(Entry::tool(
        State::Attention,
        "mensura",
        &format!(
            "cache misses cost {} tokens this session",
            davinci_tui::davinci::views::chrome::thousands(waste.missed_tokens as u64)
        ),
        None,
    ));
}

/// Everything already in the session, as transcript blocks, so a resumed
/// session opens where it left off rather than empty.
pub fn transcript_from(messages: &[davinci_ai::ChatMessage]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    // `tool_call_id` -> where its line sits, and what it was asked to do, so a
    // resumed session states its outcomes the way the live one did.
    let mut open: Vec<(String, usize, String, serde_json::Value)> = Vec::new();

    for message in messages {
        let text = davinci_ai::content_text(&message.content);
        let text = text.trim();
        match message.role.as_str() {
            "user" if !text.is_empty() => {
                if !entries.is_empty() {
                    entries.push(Entry::Gap);
                }
                if message.extra.get("customType")
                    == Some(&serde_json::Value::String(
                        davinci_agent::JOB_NOTICE_TYPE.to_string(),
                    ))
                {
                    entries.push(job_entry_from_notice(message, text));
                } else {
                    entries.push(Entry::user(text));
                }
            }
            "assistant" => {
                let calls: Vec<(&String, &String, &serde_json::Value)> = message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        davinci_ai::MessageContent::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some((id, name, arguments)),
                        _ => None,
                    })
                    .collect();
                if text.is_empty() && calls.is_empty() {
                    continue;
                }
                if !entries.is_empty() {
                    entries.push(Entry::Gap);
                }
                entries.push(Entry::agent("davinci"));
                if !text.is_empty() {
                    entries.push(Entry::Gap);
                    entries.push(Entry::prose(text));
                }
                for (id, name, arguments) in calls {
                    open.push((id.clone(), entries.len(), name.clone(), arguments.clone()));
                    entries.push(Entry::tool(
                        state_of(name, false),
                        instrument_of(name),
                        &target_of(name, arguments),
                        None,
                    ));
                }
            }
            // A tool result closes the line its call opened. Its own text is
            // never a turn of its own: what it did is already on that line.
            "tool" | "toolResult" => {
                let Some(id) = message.tool_call_id.as_ref() else {
                    continue;
                };
                let Some(position) = open.iter().position(|(open_id, ..)| open_id == id) else {
                    continue;
                };
                let (_, index, name, arguments) = open.remove(position);
                let failed = message.is_error.unwrap_or(false);
                let result = serde_json::Value::String(text.to_string());
                // A refusal replays as it was drawn live: done, with the
                // summary `denied`, not as a failed call.
                let denied = failed && gate_denied(&result);
                let outcome = if denied {
                    Some("denied".to_string())
                } else {
                    (!failed)
                        .then(|| summary_of(&name, &arguments, &result))
                        .flatten()
                };
                if let Some(Entry::Tool {
                    state,
                    summary,
                    output,
                    ..
                }) = entries.get_mut(index)
                {
                    *state = state_of(&name, failed && !denied);
                    *summary = outcome;
                    // The result rides on the line, as it does live: a
                    // failure shows its first rows, `ctrl+t` shows any.
                    *output = davinci_tui::davinci::model::tool_output_rows(text);
                }
            }
            _ => {}
        }
    }
    entries
}

/// A persisted `customType: backgroundJob` user message as the tool row the
/// live tick would have drawn: `job N finished · command`, not `> [background
/// job …]`. The first line is the notice head; indented lines are the tail.
fn job_entry_from_notice(message: &davinci_ai::ChatMessage, text: &str) -> Entry {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or(text);
    let (meta, command) = head
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(meta, command)| (meta.trim(), command.trim()))
        .unwrap_or(("", head));
    let id = message
        .extra
        .get("jobId")
        .and_then(|value| match value {
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| {
            meta.strip_prefix("background job ")
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "?".into());
    let mut parts = meta.split(" · ");
    let _head = parts.next();
    let status = parts.next().unwrap_or("finished");
    let elapsed = parts.next().unwrap_or("");
    let tail: Vec<&str> = lines
        .map(str::trim_start)
        .filter(|line| !line.is_empty() && *line != "(no output)")
        .collect();
    let state = if status == "exit 0" {
        State::Done
    } else {
        State::Failed
    };
    Entry::tool(
        state,
        "manus",
        &format!("job {id} finished · {}", clip(command, 50)),
        (!elapsed.is_empty()).then_some(elapsed),
    )
    .summarised(status)
    .with_output(&tail.join("\n"))
}

/// What a finished turn owes the transcript beyond what it already said.
///
/// The instrument on these lines is `cogitator`, the model, not `manus`, the
/// shell (design.md §7): the turn is the model's, and naming the hands for what
/// the mind did sends the reader looking in the wrong place. A turn that said
/// nothing owes a reason, and "the model returned no text" is only the reason
/// when there is no better one — a failed request has the provider's own words
/// for it, and those used to be thrown away with the worker's return value.
fn turn_outcome(
    crashed: bool,
    interrupted: bool,
    said_something: bool,
    failure: Option<String>,
    reply: &str,
) -> Vec<Entry> {
    let line =
        |state: State, text: &str| vec![Entry::Gap, Entry::tool(state, "cogitator", text, None)];
    if crashed {
        return line(
            State::Failed,
            "the turn crashed · the transcript is kept, the session is intact",
        );
    }
    if interrupted {
        return line(State::Skipped, "interrupted · the transcript is kept");
    }
    if let Some(failure) = failure.filter(|failure| !failure.trim().is_empty()) {
        // What actually went wrong, in the provider's own words, on rows of
        // their own so a long one is read rather than clipped to the line.
        let mut out = line(State::Failed, "the request failed");
        out.extend(
            failure
                .trim()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(6)
                .map(|line| Entry::Detail(line.to_string())),
        );
        return out;
    }
    if said_something {
        return Vec::new();
    }
    if !reply.trim().is_empty() {
        // Text the run returned that no `MessageEnd` carried. Better said late
        // than dropped.
        return vec![Entry::Gap, Entry::prose(reply.trim())];
    }
    line(
        State::Attention,
        "the model returned no text · nothing was added to the session",
    )
}

/// Everything Instrumenta can reach: the real slash commands, the real tools,
/// and the sessions already on disk (`1d`).
pub fn corpus(
    agent: &Agent,
    commands: &[davinci_tui::SlashCommandSpec],
    sessions: &[davinci_tui::davinci::model::SessionItem],
) -> Vec<CorpusItem> {
    // Every command the composer completes, not only the built-in ones: an
    // extension command the palette cannot reach is a command the user cannot
    // find.
    let mut items: Vec<CorpusItem> = commands
        .iter()
        .map(|command| {
            CorpusItem::new(
                &format!("/{}", command.name),
                &command.description,
                "command",
            )
        })
        .collect();

    // Davinci's own commands, which no shared command list carries.
    items.push(CorpusItem::new(
        "Local voice setup",
        "download or import an approved local speech model",
        "voice",
    ));
    items.push(CorpusItem::new(
        "/diff",
        "review every change in the working tree",
        "command",
    ));
    items.push(CorpusItem::new(
        "/permissions",
        "Manual · Accept Edits · Plan Mode · Auto Mode · Always Approve",
        "command",
    ));
    items.push(CorpusItem::new(
        "/todo",
        "the model's ledger · /todo clear",
        "command",
    ));
    items.push(CorpusItem::new(
        "/jobs",
        "background jobs · /jobs kill <id>",
        "command",
    ));
    items.push(CorpusItem::new(
        "/mcp",
        "connected MCP servers · tools and errors",
        "command",
    ));
    items.push(CorpusItem::new(
        "/plan",
        "freeze mutations · the model may only read",
        "command",
    ));
    items.push(CorpusItem::new("/act", "leave plan mode", "command"));
    items.push(CorpusItem::new(
        "/cost",
        "tokens and USD this session",
        "command",
    ));
    items.push(CorpusItem::new(
        "/status",
        "model, permission, jobs, MCP, tokens",
        "command",
    ));
    items.push(CorpusItem::new(
        "/agents",
        "custom agent profiles · list and status",
        "command",
    ));
    items.push(CorpusItem::new(
        "/workflow",
        "run deterministic workflow · /workflow <goal>",
        "command",
    ));
    items.push(CorpusItem::new(
        "/workflows",
        "list active agent workflows",
        "command",
    ));
    items.push(CorpusItem::new(
        "/workflow-stop",
        "stop a running workflow · /workflow-stop <id>",
        "command",
    ));
    items.push(CorpusItem::new(
        "/workflow-resume",
        "resume a paused workflow · /workflow-resume <id>",
        "command",
    ));

    for tool in &agent.tools {
        items.push(CorpusItem::new(tool, &tool_summary(tool), "tool"));
    }
    for session in sessions.iter().take(8) {
        items.push(CorpusItem::new(
            &format!("memoria: {}", session.name),
            &format!("session · {}", session.age),
            "session",
        ));
    }
    items
}

/// The middle column of a tool row: what the tool does, in the fewest words
/// that still say it. The instrument used to stand here, which named
/// `instrumenta` on almost every row — the one thing design.md §3 says is
/// never named, and a column that repeats itself is a column that says nothing.
fn tool_summary(name: &str) -> String {
    let described = davinci_agent::tool_specs()
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.description)
        .or_else(|| {
            crate::native_extensions::NativeExtensionHost::tool_specs()
                .into_iter()
                .find(|tool| tool.name == name)
                .map(|tool| tool.description)
        })
        .or_else(|| {
            crate::native_extensions::NativeExtensionHost::describe_tool(name)
                .map(|tool| tool.description)
        });
    let Some(described) = described else {
        if let Some(rest) = name.strip_prefix("mcp__") {
            return rest.replace("__", " ");
        }
        // An extension's tool, whose description the host holds rather than the
        // registry. Its instrument is the only thing left worth saying, and
        // only when it is not the default one.
        let instrument = instrument_of(name);
        return if instrument == "instrumenta" {
            String::new()
        } else {
            instrument.to_string()
        };
    };
    let first = described
        .split(['.', '\n'])
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    first.chars().take(64).collect()
}

/// What the composer line means. A `/` line is a command, everything else is a
/// prompt (design.md §6: the composer is the only input).
pub enum Sent {
    Prompt(String),
    Quit,
    /// Say something back without asking the model.
    Say(String),
    /// A command that needs the agent and the session to carry it out.
    Command(crate::slash::SlashAction),
}

pub fn classify(line: &str) -> Sent {
    use crate::slash::SlashAction;

    match crate::slash::parse_line(line) {
        SlashAction::Prompt(text) => Sent::Prompt(text),
        SlashAction::Quit => Sent::Quit,
        SlashAction::Status(text) => Sent::Say(text),
        // Everything else — including /model, /settings, /hotkeys, /resume —
        // reaches `perform`, which opens the sheet each one designs
        // (screens 3a–6d) with live data behind it.
        other => Sent::Command(other),
    }
}

/// The instrument a native command answers as, so `/memory-status` lands on the
/// same paired name the tool lines use (design.md §3).
pub fn instrument_of_command(name: &str) -> &'static str {
    match name {
        name if name.starts_with("memory") => "memoria",
        name if name.starts_with("governor") => "mensura",
        name if name.starts_with("graph") => "grafo",
        name if name.starts_with("sec") => "speculum",
        name if name.starts_with("learning") => "doctrina",
        name if name.starts_with("skill") => "ars",
        _ => "instrumenta",
    }
}

/// Run a `/command` an extension owns — a native Rust one (`/graph-view`,
/// `/memory-status`, `/sec-report`) or a JavaScript extension's — and state
/// what came back in the transcript.
///
/// `None` means no extension claimed the line, so the shell should carry on
/// classifying it. Mirrors the extension-command arm of `prepare_user_input`
/// in `main.rs`, which the legacy chrome runs before every prompt.
fn run_extension_command(shell: &mut Shell<'_>, line: &str) -> Option<Next> {
    run_extension_command_inner(shell, line, true)
}

/// Starting or resuming work is not a request to replace the conversation.
fn graph_command_opens_view(name: &str, args: &str) -> bool {
    matches!(name, "graph-status" | "graph-view")
        || (name == "graph" && !graph_setup::is_launch(args))
}

fn run_extension_command_inner(shell: &mut Shell<'_>, line: &str, setup: bool) -> Option<Next> {
    let (name, args) = crate::parse_extension_command(line);
    if name.is_empty() {
        return None;
    }
    if setup && name == "graph" && graph_setup::is_launch(&args) {
        graph_setup::open(shell, &args);
        return Some(Next::Go);
    }

    let outcome = if matches!(
        name.as_str(),
        "security-scan" | "sec-resume" | "sec-status" | "sec-report" | "sec-abort"
    ) {
        let host = shell
            .host
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        crate::apply_graph_session_context(shell.parsed, shell.agent, &host);
        let admission = if matches!(name.as_str(), "security-scan" | "sec-resume") {
            crate::configure_security_review(shell.parsed, shell.agent, &host)
        } else {
            Ok(())
        };
        Some(
            admission
                .and_then(|()| host.execute_native_command(&name, &args))
                .and_then(|value| value.ok_or_else(|| "security command unavailable".into())),
        )
    } else {
        let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        crate::apply_graph_session_context(shell.parsed, shell.agent, &host);
        let admission = if matches!(name.as_str(), "security-scan" | "sec-resume") {
            crate::configure_security_review(shell.parsed, shell.agent, &host)
        } else {
            Ok(())
        };
        match admission.and_then(|()| host.execute_native_command(&name, &args)) {
            Ok(Some(value)) => Some(Ok(value)),
            Err(err) => Some(Err(err)),
            Ok(None) => {
                let path = host
                    .js
                    .iter()
                    .find(|ext| ext.commands.iter().any(|command| command == &name))
                    .map(|ext| ext.path.clone())?;
                host.runtime_active_tools = shell.agent.tools.clone();
                host.runtime_all_tools = shell.agent.tool_registry.clone();
                host.runtime_thinking_level = shell.agent.thinking_level.as_str().to_string();
                host.runtime_flag_values = crate::flag_values_json(shell.parsed);
                Some(
                    host.invoke_command(&path, &name)
                        .map(|value| value.unwrap_or(serde_json::Value::Null)),
                )
            }
        }
    };

    // Anything the extension drew while it ran, before what it returned.
    drain_ui_calls(shell.model, shell.host);
    shell.model.running = false;

    match outcome? {
        // The native status commands have whole screens designed for them
        // (`5a`–`5d`); their results open the sheet rather than printing rows.
        Ok(value) => match name.as_str() {
            "memory-status" => {
                shell.model.vector_index = Some(vectors_sheet(&value));
                open_sheet(shell.model, Screen::Vectors);
            }
            "governor-status" => {
                shell.model.governor = Some(governor_sheet(&value));
                open_sheet(shell.model, Screen::Governor);
            }
            "security-scan" | "sec-resume" | "sec-status" => {
                shell.model.security = Some(security_sheet(&value));
                shell.model.security_index = 0;
                open_sheet(shell.model, Screen::Securitas);
            }
            "sec-report" => {
                if value["schemaVersion"] == 2 {
                    shell.model.security = Some(security_sheet(&value));
                    shell.model.security_index = value["selectedFindingId"]
                        .as_str()
                        .and_then(|id| {
                            value["findings"].as_array().and_then(|findings| {
                                findings
                                    .iter()
                                    .position(|finding| finding["findingId"].as_str() == Some(id))
                            })
                        })
                        .unwrap_or(0);
                    open_sheet(shell.model, Screen::Securitas);
                    return Some(Next::Go);
                }
                // The report command answers with markdown; the sheet wants
                // the structured scan, which `sec-status` carries.
                let structured = {
                    let host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
                    host.execute_native_command("sec-status", "")
                };
                match structured {
                    Ok(Some(scan)) => {
                        shell.model.security = Some(security_sheet(&scan));
                        shell.model.security_index = 0;
                        open_sheet(shell.model, Screen::Securitas);
                    }
                    _ => push_command_result(shell.model, &name, &value),
                }
            }
            "graph" | "graph-status" | "graph-view" => match graph_sheet(&value) {
                Some(sheet) => {
                    graph_feedback::refresh(shell.model, sheet);
                    if graph_command_opens_view(&name, &args) {
                        open_sheet(shell.model, Screen::GraphRun);
                    }
                }
                // `graph-view` answers with a worker transcript, not the run;
                // the sheet is still the place to watch it from.
                None if refresh_graph_sheet(shell.model, shell.host) => {
                    if graph_command_opens_view(&name, &args) {
                        open_sheet(shell.model, Screen::GraphRun);
                    }
                }
                None => push_command_result(shell.model, &name, &value),
            },
            // A run just started (or resumed): watch it on the sheet rather
            // than reading its first checkpoint as rows. The sheet refreshes
            // itself every second while it is open.
            "graph-resume"
                if value.get("started").and_then(serde_json::Value::as_bool) == Some(true) =>
            {
                if !refresh_graph_sheet(shell.model, shell.host) {
                    push_command_result(shell.model, &name, &value);
                }
            }
            _ => push_command_result(shell.model, &name, &value),
        },
        Err(err) => shell.note(&format!("/{name}: {err}")),
    }
    // A command may have driven the session — sent a message, forked,
    // switched — through `pi.sendMessage` and friends; those calls sit on the
    // host until applied.
    match apply_host_effects(shell) {
        Next::Go => {}
        other => return Some(other),
    }
    shell.redress();
    Some(Next::Go)
}

/// Whether a composer line is a `/command` nobody owns, and what to say about
/// it. Only a bare token counts: `/graph-view`, or `/graph-view.` with the
/// punctuation a sentence leaves behind. A slash line carrying arguments or
/// prose is a prompt, and is sent as one.
fn unknown_command(model: &Model, line: &str) -> Option<String> {
    let token = line.trim().strip_prefix('/')?;
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }
    // Matched as typed, punctuation included: `/graph-view.` is not the command
    // `/graph-view`, and quietly running it as one would teach the wrong name.
    let typed = token.to_ascii_lowercase();
    if model
        .slash_commands
        .iter()
        .any(|item| item.name.eq_ignore_ascii_case(token))
    {
        return None;
    }

    // The nearest name the user could have meant, in three bands: a name the
    // typed token begins, a name a small edit away — which is what catches the
    // transposition and the stray full stop — and a name that merely contains
    // it. Ranking edit distance ahead of containment is what keeps `/graph-veiw`
    // from being answered with `/graph`.
    let budget = (typed.chars().count() / 2).max(3);
    let nearest = model
        .slash_commands
        .iter()
        .map(|item| item.name.as_str())
        .filter_map(|name| {
            let lower = name.to_ascii_lowercase();
            if lower.starts_with(&typed) {
                return Some((0usize, lower.chars().count(), name));
            }
            let distance = edit_distance(&lower, &typed);
            if distance <= budget {
                return Some((1, distance, name));
            }
            lower
                .contains(&typed)
                .then_some((2, lower.chars().count(), name))
        })
        .min();
    Some(match nearest {
        Some((_, _, name)) => format!("/{token} is not a command · did you mean /{name}?"),
        None => format!("/{token} is not a command · ctrl+p lists every one"),
    })
}

/// Levenshtein distance, one row at a time. Only ever run over slash-command
/// names, so the quadratic cost is a few hundred cells.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            let next = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + cost);
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// State a native command's result in the transcript: one tool line naming the
/// instrument and the command, then one detail row per field. The legacy chrome
/// frames this as an ANSI panel; davinci says it in the transcript, which is
/// the only place a davinci shell can say anything (design.md §6).
fn push_command_result(model: &mut Model, name: &str, value: &serde_json::Value) {
    let instrument = instrument_of_command(name);
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let rows = command_result_rows(value);

    model.transcript.push(Entry::Gap);
    model.transcript.push(Entry::tool(
        if error.is_some() {
            State::Attention
        } else {
            State::Done
        },
        instrument,
        &format!("/{name}"),
        None,
    ));
    if let Some(error) = error {
        model.transcript.push(Entry::Detail(error));
    } else if rows.is_empty() {
        // A command that returned an empty object still ran. Saying so beats a
        // line that names the command and then stops.
        model
            .transcript
            .push(Entry::Detail("nothing to report".into()));
    }
    for row in rows {
        model.transcript.push(Entry::Detail(row));
    }
}

/// A command result as detail rows. Scalars become `key · value`; a list of
/// objects becomes one row per item; everything else is left as compact JSON so
/// nothing is silently dropped.
fn command_result_rows(value: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;

    let scalar = |value: &Value| -> Option<String> {
        match value {
            Value::String(text) => Some(text.lines().next().unwrap_or("").to_string()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    };

    match value {
        Value::Null => Vec::new(),
        Value::Object(map) => {
            let mut rows = Vec::new();
            for (key, item) in map {
                // A field the command had nothing to put in is not news. Saying
                // `run · null` back to the user is worse than saying nothing.
                if key == "error" || item.is_null() {
                    continue;
                }
                let label = crate::humanize_key(key);
                match item {
                    Value::Array(items) if items.is_empty() => {
                        rows.push(format!("{label} · none"));
                    }
                    Value::Array(items) => {
                        rows.push(format!("{label} · {}", items.len()));
                        for item in items.iter().take(12) {
                            let row = scalar(item).unwrap_or_else(|| summarize_object(item));
                            if !row.is_empty() {
                                rows.push(format!("  {row}"));
                            }
                        }
                        if items.len() > 12 {
                            rows.push(format!("  … {} more", items.len() - 12));
                        }
                    }
                    item => {
                        let row = scalar(item).unwrap_or_else(|| summarize_object(item));
                        if !row.is_empty() {
                            rows.push(format!("{label} · {row}"));
                        }
                    }
                }
            }
            rows
        }
        Value::Array(items) => items
            .iter()
            .take(12)
            .map(|item| scalar(item).unwrap_or_else(|| summarize_object(item)))
            .filter(|row| !row.is_empty())
            .collect(),
        other => scalar(other).into_iter().collect(),
    }
}

/// One line for a nested object: its scalar fields, ` · ` separated, so a task
/// row reads `review-1 · running · reviewer` rather than as raw JSON.
fn summarize_object(value: &serde_json::Value) -> String {
    let serde_json::Value::Object(map) = value else {
        return serde_json::to_string(value).unwrap_or_default();
    };
    map.values()
        .filter_map(|item| match item {
            serde_json::Value::String(text) => {
                Some(text.lines().next().unwrap_or("").trim().to_string())
            }
            serde_json::Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Apply everything the extension host has queued for the interface, then clear
/// it. The host accumulates `ui_calls` for the life of the process; davinci
/// shares one host across every turn, so re-reading the vector without taking
/// it replayed every past `notify` into the transcript on each turn.
pub fn drain_ui_calls(model: &mut Model, host: &Arc<Mutex<ExtensionHost>>) -> Option<String> {
    let calls = {
        let mut host = host.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut host.ui_calls)
    };
    apply_ui_calls(model, &calls)
}

/// What carrying a command out amounted to. Every command says something: a
/// command that changed nothing visible still owes the user a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// Prose in the transcript.
    Said(String),
    /// A line that carries the attention glyph: a refusal, a usage error, a
    /// cancellation (design.md §4 — the glyph, not the colour, says so).
    Note(String),
    /// A sheet was built onto the model and is on screen; the sheet is the
    /// answer, so nothing more is said.
    Opened,
    /// Put a question to the user as a list, and act on the row they choose.
    Ask(Question),
    /// Leave the alt screen, run this, come back. Reserved for the flows that
    /// own the terminal themselves — a browser handshake and its prompts.
    Detach(Detached),
}

/// A question the shell puts to the user through the `Ask` instrument. The
/// rows the panel shows are derived from this; the answer comes back as the
/// index of the row chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    GraphSetup(graph_setup::Setup),
    /// Whether this project's `.pi` resources may be used.
    Trust {
        path: String,
        options: Vec<crate::trust::ProjectTrustOption>,
    },
    /// Which stored credential to remove.
    Logout {
        providers: Vec<String>,
    },
    /// The one thing first-run has to ask. The old setup also asked for a
    /// theme; davinci has one palette, negotiated from the terminal rather
    /// than chosen (design.md §2), so only the analytics question remains.
    FirstRun,
}

impl Question {
    /// The panel this question wears: its paired name, its key, the line that
    /// says what is being decided, and one row per answer.
    pub fn ask(&self, _agent: &Agent) -> Ask {
        match self {
            Question::GraphSetup(setup) => setup.ask(_agent),
            Question::Trust { path, options } => Ask {
                title: "Trust".into(),
                name: "TRUST".into(),
                key: "/trust".into(),
                note: format!("{path} · takes effect on the next start"),
                items: options
                    .iter()
                    .map(|option| {
                        PickerItem::new(
                            &option.label,
                            if option.trusted {
                                "project .pi resources are used"
                            } else {
                                "project .pi resources are ignored"
                            },
                        )
                    })
                    .collect(),
            },
            Question::FirstRun => Ask {
                title: "Welcome".into(),
                name: "WELCOME".into(),
                key: "first run".into(),
                note: "anonymous usage data helps pi improve; it is never required".into(),
                items: vec![
                    PickerItem::new("share anonymous usage data", "recommended"),
                    PickerItem::new("keep it to this machine", ""),
                ],
            },
            Question::Logout { providers } => Ask {
                title: "Credentials".into(),
                name: "CREDENTIALS".into(),
                key: "/logout".into(),
                note: "chosen credentials are removed from this machine".into(),
                items: providers
                    .iter()
                    .map(|provider| PickerItem::new(provider, "stored by /login"))
                    .collect(),
            },
        }
    }
}

/// The permission panel for one tool call the policy could not
/// decide on its own (spec: trust-and-control, *davinci*). The policy supplies
/// legal choices; an untrusted host additionally removes project persistence.
pub fn permission_ask(request: &ToolApprovalRequest, trusted: bool) -> Ask {
    let rule = &request.session_rule;
    let mut items: Vec<_> = request
        .host_choices(trusted)
        .into_iter()
        .map(|choice| match choice {
            ToolApprovalDecision::AllowOnce => PickerItem::new("allow once", "runs this call only"),
            ToolApprovalDecision::AllowForSession => {
                PickerItem::new("allow for this session", &format!("{rule} until pi exits"))
            }
            ToolApprovalDecision::AllowAlways => PickerItem::new(
                "always allow here",
                &format!("{rule} saved to .pi/settings.json"),
            ),
            ToolApprovalDecision::Deny => PickerItem::new("deny", "the model is told no"),
        })
        .collect();
    if offers_denial_instructions(request) {
        items.push(PickerItem::new(
            "deny with instructions",
            "tell the model what to do instead; this call will not run",
        ));
    }
    let mut note = request.summary.clone();
    if request.outside_project {
        note.push_str(" · outside the project");
    }
    Ask {
        title: "Permission".into(),
        name: "PERMISSION".into(),
        key: "/permissions".into(),
        note,
        items,
    }
}

/// What the chosen row of `permission_ask` means; `None` for a row that the
/// panel did not offer.
pub fn permission_choice(
    index: usize,
    choices: &[ToolApprovalDecision],
) -> Option<ToolApprovalDecision> {
    choices.get(index).copied()
}

/// Render an explicit task-contract expansion separately from ordinary permission consent.
pub fn scope_expansion_ask(
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
) -> Ask {
    Ask {
        title: "Scope expansion required".into(),
        name: "TASK CONTRACT".into(),
        key: "/scope-expansion".into(),
        note: format!(
            "{}\nrevision {} → {}\n+ writable: {}\ncurrent: {}\nproposed: {}\npreview: {}",
            preview.reason,
            preview.expected_revision,
            preview.proposed.revision,
            preview.requested_target,
            preview.current_digest,
            preview.proposed.digest,
            preview.preview_digest,
        ),
        items: vec![
            PickerItem::new(
                "approve expansion",
                "persist this exact contract revision before resuming execution",
            ),
            PickerItem::new(
                "request in-scope approach",
                "keep the current contract and ask for an alternative",
            ),
            PickerItem::new(
                "deny with instructions",
                "keep the current contract and provide host-authored guidance",
            ),
        ],
    }
}

/// Map only the three scope-expansion rows to trusted-host decisions.
pub fn scope_expansion_choice(
    index: usize,
) -> Option<davinci_agent::runtime::contracts::ScopeExpansionDecision> {
    use davinci_agent::runtime::contracts::ScopeExpansionDecision;
    match index {
        0 => Some(ScopeExpansionDecision::Approve),
        1 => Some(ScopeExpansionDecision::RequestInScopeApproach),
        2 => Some(ScopeExpansionDecision::Deny),
        _ => None,
    }
}

fn scope_expansion_answer_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
) -> Option<(
    davinci_agent::runtime::contracts::ScopeExpansionDecision,
    Option<String>,
)> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use davinci_agent::runtime::contracts::ScopeExpansionDecision;
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Esc && key.modifiers.is_empty()
        || key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
    {
        model.approval_instructions = None;
        model.overlay = None;
        return Some((ScopeExpansionDecision::Deny, None));
    }
    if let Some(input) = model.approval_instructions.as_mut() {
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
        {
            if !input.trim().is_empty() {
                let instructions = model.approval_instructions.take().unwrap().into_string();
                model.overlay = None;
                return Some((ScopeExpansionDecision::Deny, Some(instructions)));
            }
            return None;
        }
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            let editor = input.editor_mut();
            match key.code {
                KeyCode::Char(ch)
                    if !ch.is_control() && editor.buffer.len() + ch.len_utf8() <= 4096 =>
                {
                    editor.insert(ch)
                }
                KeyCode::Left => editor.move_left(),
                KeyCode::Right => editor.move_right(),
                KeyCode::Home => editor.move_line_start(),
                KeyCode::End => editor.move_line_end(),
                KeyCode::Backspace => editor.backspace(),
                KeyCode::Delete => editor.delete_forward(),
                _ => {}
            }
        }
        return None;
    }
    if model.overlay == Some(Overlay::Ask)
        && model.ask.key == "/scope-expansion"
        && model.ask_index == 2
        && model.overlay_offset.is_none()
        && key.code == KeyCode::Enter
        && key.modifiers.is_empty()
        && key.kind == KeyEventKind::Press
    {
        model.approval_instructions = Some(Default::default());
        return None;
    }
    use davinci_tui::davinci::app::{handle_key, Flow};
    match handle_key(model, key) {
        Flow::Choose(Choice::Ask(index)) => {
            scope_expansion_choice(index).map(|decision| (decision, None))
        }
        Flow::Interrupt | Flow::Quit => {
            model.overlay = None;
            Some((ScopeExpansionDecision::Deny, None))
        }
        Flow::Continue if model.overlay.is_none() => Some((ScopeExpansionDecision::Deny, None)),
        Flow::Continue
        | Flow::Choose(_)
        | Flow::Submit(_)
        | Flow::CyclePermissionMode
        | Flow::SecretInputSubmitted(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopeExpansionTaskGuard {
    task_id: davinci_agent::TaskId,
    run_id: davinci_agent::RunId,
    assigned_to: Option<davinci_agent::AgentId>,
    revision: u64,
    owner_generation: u64,
    contract_digest: Option<String>,
}

fn capture_scope_expansion_task_guard(
    agent: &Agent,
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
) -> Result<Option<ScopeExpansionTaskGuard>, String> {
    let current = agent
        .active_contract()
        .ok_or_else(|| "scope expansion has no active task contract".to_string())?;
    preview
        .verify_against(&current)
        .map_err(|error| error.to_string())?;
    let Some(runtime) = agent.runtime_for_session() else {
        return Ok(None);
    };
    Ok(runtime
        .task_registry
        .get_task(&current.task_id)
        .map(|task| ScopeExpansionTaskGuard {
            task_id: task.id,
            run_id: task.run_id,
            assigned_to: task.assigned_to,
            revision: task.revision,
            owner_generation: task.owner_generation,
            contract_digest: task.contract_digest,
        }))
}

fn apply_scope_expansion_decision(
    agent: &mut Agent,
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
    task_guard: Option<&ScopeExpansionTaskGuard>,
    decision: davinci_agent::runtime::contracts::ScopeExpansionDecision,
    instructions: Option<&str>,
) -> Result<Option<String>, String> {
    use davinci_agent::runtime::contracts::ScopeExpansionDecision;
    match decision {
        ScopeExpansionDecision::Approve => {
            commit_scope_expansion(agent, preview, task_guard)?;
            Ok(Some(format!(
                "Scope expansion was explicitly approved and durably committed as contract revision {} (digest {}). Continue from the blocked action using only that exact expanded scope.",
                preview.proposed.revision, preview.proposed.digest
            )))
        }
        ScopeExpansionDecision::RequestInScopeApproach => Ok(Some(format!(
            "Scope expansion for `{}` was not approved. Continue only with an in-scope approach under contract revision {} (digest {}). Do not retry the blocked target unless a new explicit scope expansion is approved.",
            preview.requested_target, preview.expected_revision, preview.current_digest
        ))),
        ScopeExpansionDecision::Deny => {
            let mut guidance = format!(
                "Scope expansion for `{}` was denied. Keep contract revision {} (digest {}) unchanged.",
                preview.requested_target, preview.expected_revision, preview.current_digest
            );
            if let Some(instructions) = instructions.filter(|text| !text.trim().is_empty()) {
                guidance.push_str(" Host instructions: ");
                guidance.push_str(instructions.trim());
            }
            Ok(Some(guidance))
        }
    }
}

fn commit_scope_expansion(
    agent: &mut Agent,
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
    task_guard: Option<&ScopeExpansionTaskGuard>,
) -> Result<(), String> {
    let current = agent
        .active_contract()
        .ok_or_else(|| "scope expansion has no active task contract".to_string())?;
    preview
        .verify_against(&current)
        .map_err(|error| error.to_string())?;

    let data = serde_json::json!({
        "event": "scope_expansion_approved",
        "preview_digest": preview.preview_digest,
        "expected_revision": preview.expected_revision,
        "current_digest": preview.current_digest,
        "requested_target": preview.requested_target,
        "reason": preview.reason,
        "proposed": preview.proposed,
        "authorized_by": "interactive_user",
    });
    let session = agent
        .session
        .as_mut()
        .ok_or_else(|| "scope expansion requires durable session storage".to_string())?;
    let prior_leaf = session.leaf_id.clone();
    let mut extra = serde_json::Map::new();
    extra.insert("data".into(), data);
    if let Err(error) = session.append_entry(davinci_session::SessionEntry {
        id: String::new(),
        entry_type: "custom".into(),
        parent_id: None,
        seq: 0,
        timestamp: 0,
        message: None,
        custom_type: Some("task_contract_scope_expansion".into()),
        extra,
    }) {
        session.leaf_id = prior_leaf;
        return Err(format!("Unable to persist scope expansion: {error}"));
    }

    match (agent.runtime_for_session().cloned(), task_guard) {
        (Some(runtime), Some(guard)) => {
            if guard.task_id != current.task_id
                || guard.run_id != runtime.run_id
                || guard.assigned_to != Some(runtime.agent_id)
                || guard.contract_digest.as_deref() != Some(current.digest.as_str())
            {
                return Err("Unable to persist bound task contract digest: stale scope expansion task binding".into());
            }
            runtime
                .task_registry
                .revise_contract_digest(
                    guard.task_id,
                    guard.run_id,
                    runtime.agent_id,
                    guard.revision,
                    guard.owner_generation,
                    &current.digest,
                    &preview.proposed.digest,
                )
                .map_err(|error| {
                    format!("Unable to persist bound task contract digest: {error}")
                })?;
        }
        (Some(runtime), None) => {
            if runtime.task_registry.get_task(&current.task_id).is_some() {
                return Err("Unable to persist bound task contract digest: task binding changed after scope expansion preview".into());
            }
        }
        (None, Some(_)) => {
            return Err("Unable to persist bound task contract digest: runtime binding changed after scope expansion preview".into());
        }
        (None, None) => {}
    }

    {
        let mut permissions = agent
            .permissions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        permissions.session_allow.clear();
    }
    agent.set_active_contract(preview.proposed.clone());
    Ok(())
}

fn scope_expansion_preview_from_events(
    contract: Option<&davinci_agent::runtime::contracts::TaskContract>,
    events: &[AgentEvent],
) -> Option<davinci_agent::runtime::contracts::ScopeExpansionPreview> {
    let contract = contract?;
    events.iter().find_map(|event| {
        let AgentEvent::ToolExecutionEnd {
            is_error: true,
            details: Some(details),
            ..
        } = event
        else {
            return None;
        };
        let violation: davinci_agent::runtime::contracts::ScopeViolation =
            serde_json::from_value(details.get("scope_violation")?.clone()).ok()?;
        contract
            .preview_scope_expansion(&violation.requested_target, &violation.reason)
            .ok()
    })
}

fn resolve_scope_expansion_modal(
    agent: &mut Agent,
    model: &mut Model,
    session: &mut davinci_tui::davinci::runtime::Session,
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
    task_guard: Option<&ScopeExpansionTaskGuard>,
    voice: &mut crate::voice_input::VoiceInput,
) -> std::io::Result<Option<String>> {
    model.ask = scope_expansion_ask(preview);
    model.ask_index = 0;
    model.approval_instructions = None;
    open_ask_overlay(model);
    voice.cancel(model);
    loop {
        session.draw(model)?;
        voice.drawn();
        let Some(event) = session.poll_event(Duration::from_millis(40))? else {
            continue;
        };
        match event {
            crossterm::event::Event::Key(key) => {
                if let Some((decision, instructions)) = scope_expansion_answer_key(model, key) {
                    model.approval_instructions = None;
                    model.overlay = None;
                    return apply_scope_expansion_decision(
                        agent,
                        preview,
                        task_guard,
                        decision,
                        instructions.as_deref(),
                    )
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error));
                }
            }
            crossterm::event::Event::Resize(width, height) => {
                model.width = width.max(20);
                model.height = height.max(4);
            }
            crossterm::event::Event::Mouse(mouse) => {
                if session.handle_model_mouse(model, mouse) {
                    voice.toggle(model);
                }
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
pub fn watchdog_ask(signal: &davinci_agent::runtime::progress_watchdog::LoopSignal) -> Ask {
    Ask {
        title: "Progress watchdog".into(),
        name: "WATCHDOG".into(),
        key: "/watchdog".into(),
        note: format!("! {}", signal.description()),
        items: vec![
            PickerItem::new(
                "1. Continue with remaining budget",
                "continue execution within remaining resources",
            ),
            PickerItem::new(
                "2. Return to Plan Mode with evidence",
                "quiesce mutation workers and switch back to plan mode",
            ),
            PickerItem::new(
                "3. Stop and preserve checkpoint",
                "terminate task and persist available checkpoint",
            ),
        ],
    }
}

#[allow(dead_code)]
pub fn watchdog_choice(index: usize) -> Option<&'static str> {
    match index {
        0 => Some("continue"),
        1 => Some("plan"),
        2 => Some("stop"),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn watchdog_answer_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
) -> Option<&'static str> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use davinci_tui::davinci::app::{handle_key, Flow};
    use davinci_tui::davinci::model::Choice;

    if key.kind == KeyEventKind::Release {
        return None;
    }
    if (key.code == KeyCode::Esc && key.modifiers.is_empty())
        || (key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL)
    {
        model.overlay = None;
        return Some("stop");
    }
    if key.code == KeyCode::Char('1') && key.modifiers.is_empty() {
        model.overlay = None;
        return Some("continue");
    }
    if key.code == KeyCode::Char('2') && key.modifiers.is_empty() {
        model.overlay = None;
        return Some("plan");
    }
    if key.code == KeyCode::Char('3') && key.modifiers.is_empty() {
        model.overlay = None;
        return Some("stop");
    }

    match handle_key(model, key) {
        Flow::Choose(Choice::Ask(index)) => {
            model.overlay = None;
            watchdog_choice(index)
        }
        Flow::Interrupt | Flow::Quit => {
            model.overlay = None;
            Some("stop")
        }
        Flow::Continue if model.overlay.is_none() => Some("stop"),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn apply_watchdog_choice(
    agent: &mut Agent,
    choice: &str,
    signal: &davinci_agent::runtime::progress_watchdog::LoopSignal,
) -> Result<String, String> {
    let tokens_remaining = if let Some(ledger) = agent
        .runtime
        .as_ref()
        .and_then(|rt| rt.budget_ledger.as_ref())
    {
        let snap = ledger.snapshot();
        snap.token_ceiling.saturating_sub(snap.tokens_charged)
    } else {
        u64::MAX
    };

    let decision = if let Some(runtime) = &agent.runtime {
        if let Ok(mut wd) = runtime.progress_watchdog.lock() {
            wd.reduce_choice(tokens_remaining, choice)
        } else {
            "hard_stop"
        }
    } else {
        match choice {
            "continue" => "continue",
            "plan" => "return_to_plan",
            "stop" => "stop_checkpoint",
            _ => "paused",
        }
    };

    match decision {
        "continue" => Ok("Continuing task with remaining budget".into()),
        "return_to_plan" => {
            agent.set_permission_mode(davinci_agent::PermissionMode::ReadOnly);
            Ok(format!(
                "Switched to Plan Mode due to watchdog signal: {}",
                signal.description()
            ))
        }
        "stop_checkpoint" => {
            agent.abort();
            Ok("Task stopped; preserved checkpoint".into())
        }
        "hard_stop" => {
            agent.abort();
            Err("Budget exhausted: hard stop enforced".into())
        }
        _ => Ok("Watchdog paused".into()),
    }
}

/// A failed save keeps the request pending until the user selects another scope.
fn persist_permission_choice(
    model: &mut Model,
    request: &ToolApprovalRequest,
    decision: ToolApprovalDecision,
    cwd: &std::path::Path,
    project_allowed: &mut bool,
) -> Option<ToolApprovalDecision> {
    if !request.allows(decision) {
        return Some(ToolApprovalDecision::Deny);
    }
    if decision != ToolApprovalDecision::AllowAlways {
        return Some(decision);
    }
    if !*project_allowed {
        return Some(ToolApprovalDecision::Deny);
    }
    if crate::permissions::remember_project_rule(cwd, &request.session_rule).is_ok() {
        return Some(decision);
    }
    *project_allowed = false;
    model.ask = permission_ask(request, false);
    model.ask.note = format!(
        "Project permission could not be saved. Choose another scope. {}",
        model.ask.note
    );
    open_ask_overlay(model);
    None
}

fn offers_denial_instructions(request: &ToolApprovalRequest) -> bool {
    request.legal_choices.iter().any(|choice| {
        choice.id == "deny_with_instructions"
            && choice.scope == davinci_agent::approval::GrantScope::DenyWithInstructions
    })
}

fn approval_answer_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    choices: &[ToolApprovalDecision],
    request: &ToolApprovalRequest,
    abort: &Arc<AtomicBool>,
) -> Option<NativeApprovalAnswer> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Esc && key.modifiers.is_empty()
        || key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
    {
        model.approval_instructions = None;
        model.overlay = None;
        if key.code == KeyCode::Char('c') {
            abort.store(true, Ordering::Relaxed);
            model.interrupt();
        }
        return Some(ToolApprovalDecision::Deny.into());
    }
    if let Some(input) = model.approval_instructions.as_mut() {
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
        {
            if !input.trim().is_empty() && offers_denial_instructions(request) {
                let instructions = model.approval_instructions.take().unwrap().into_string();
                model.overlay = None;
                return Some(NativeApprovalAnswer {
                    decision: ToolApprovalDecision::Deny,
                    instructions: Some(instructions),
                });
            }
            return None;
        }
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            let editor = input.editor_mut();
            match key.code {
                KeyCode::Char(ch)
                    if !ch.is_control() && editor.buffer.len() + ch.len_utf8() <= 4096 =>
                {
                    editor.insert(ch)
                }
                KeyCode::Left => editor.move_left(),
                KeyCode::Right => editor.move_right(),
                KeyCode::Home => editor.move_line_start(),
                KeyCode::End => editor.move_line_end(),
                KeyCode::Backspace => editor.backspace(),
                KeyCode::Delete => editor.delete_forward(),
                _ => {}
            }
        }
        return None;
    }
    // Ask owns navigation and explicit Enter semantics; only its chosen extra
    // policy-issued row opens the editor. No conversation draft is moved.
    if model.overlay == Some(Overlay::Ask)
        && model.ask.key == "/permissions"
        && model.ask_index == choices.len()
        && model.overlay_offset.is_none()
        && offers_denial_instructions(request)
        && key.code == KeyCode::Enter
        && key.modifiers.is_empty()
        && key.kind == KeyEventKind::Press
    {
        model.approval_instructions = Some(Default::default());
        return None;
    }
    approval_key(model, key, choices, abort).map(Into::into)
}

/// One key while the permission panel is up. The panel takes the keys every
/// other panel takes — ↑↓ move, enter chooses, esc closes — and esc closing
/// an unanswered question is a `deny`, not a shrug. `ctrl+c` raises the
/// abort flag as it does anywhere mid-turn, and denies so the worker sees
/// the flag promptly rather than after a call the user was refusing.
fn approval_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    choices: &[ToolApprovalDecision],
    abort: &Arc<AtomicBool>,
) -> Option<ToolApprovalDecision> {
    use davinci_tui::davinci::app::{handle_key, Flow};
    match handle_key(model, key) {
        Flow::Choose(Choice::Ask(index)) => {
            Some(permission_choice(index, choices).unwrap_or(ToolApprovalDecision::Deny))
        }
        Flow::Interrupt | Flow::Quit => {
            abort.store(true, Ordering::Relaxed);
            model.interrupt();
            model.overlay = None;
            Some(ToolApprovalDecision::Deny)
        }
        Flow::Continue if model.overlay.is_none() => Some(ToolApprovalDecision::Deny),
        Flow::Continue
        | Flow::Choose(_)
        | Flow::Submit(_)
        | Flow::CyclePermissionMode
        | Flow::SecretInputSubmitted(_) => None,
    }
}

/// Work that cannot happen underneath the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detached {
    Login {
        provider: String,
        key: Option<String>,
    },
}

/// Carry out a command that needs the live agent. Everything here mirrors the
/// arm of `handle_user_line` in `main.rs` that the old chrome ran, minus the
/// chrome: the same stores, the same extension events, the same order.
pub fn perform(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    model: &mut Model,
    action: crate::slash::SlashAction,
) -> Result<Done, String> {
    use crate::extension_host::ExtensionEvent;
    use crate::slash::SlashAction;
    use davinci_session::JsonlSession;
    use std::path::PathBuf;

    let agent_dir = crate::default_agent_dir();
    match action {
        // Routed by `classify`; carried here only so the match is total.
        SlashAction::Prompt(text) => Ok(Done::Said(text)),
        SlashAction::Quit => Ok(Done::Said("goodbye".into())),
        SlashAction::Status(text) => Ok(Done::Said(text)),
        SlashAction::OpenModel => {
            open_models_sheet(parsed, agent, model);
            Ok(Done::Opened)
        }
        SlashAction::Resume => {
            open_resume_sheet(parsed, agent, model);
            Ok(Done::Opened)
        }
        SlashAction::Settings => {
            open_settings_sheet(agent, model);
            Ok(Done::Opened)
        }
        SlashAction::Hotkeys => {
            open_keys_sheet(model);
            Ok(Done::Opened)
        }

        SlashAction::NewSession => {
            let session_dir = crate::resolved_session_dir(parsed, &agent.cwd);
            let store = JsonlSession::create(&session_dir, &agent.cwd.to_string_lossy(), None)
                .map_err(|err| err.to_string())?;
            agent.load_from_session(store)?;
            model.composer_epoch = model.composer_epoch.saturating_add(1);
            model.transcript.clear();
            Ok(Done::Said("started a new session".into()))
        }
        SlashAction::Compact(instructions) => {
            let mut host = crate::loaded_extension_host(parsed);
            host.runtime_flag_values = crate::flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeCompact);
            if host.last_result_cancelled() {
                return Ok(Done::Note("compaction cancelled".into()));
            }
            let messages_before = agent.messages.len();
            let result = agent.compact(instructions.as_deref());
            if result.compacted {
                host.emit(ExtensionEvent::SessionCompact);
            } else {
                host.emit(ExtensionEvent::SessionCompactFailed {
                    error: result.summary.clone(),
                });
            }
            model.transcript = transcript_from(&agent.messages);
            if !result.compacted {
                return Ok(Done::Said(result.summary));
            }
            // The `4c` sheet, as the receipt of what just happened: both
            // sides measured, what was kept named, what was folded counted.
            let window = agent.context_window.max(1);
            let before = result.tokens_before;
            let after = davinci_agent::estimate_context_tokens(&agent.messages);
            let folded_messages = messages_before.saturating_sub(agent.messages.len());
            let thousands = davinci_tui::davinci::views::chrome::thousands;
            let mut kept = vec![
                format!("the last {} messages, whole", agent.messages.len()),
                "the summary of everything folded".into(),
            ];
            if let Some(instructions) = instructions.as_deref().filter(|text| !text.is_empty()) {
                kept.push(format!("your instruction: {}", clip(instructions, 48)));
            }
            if !result.details.modified_files.is_empty() {
                kept.push(format!(
                    "the {} files the turn modified, named",
                    result.details.modified_files.len()
                ));
            }
            model.compaction = Some(Compaction {
                before_tokens: thousands(before),
                before_fraction: (before as f64 / window as f64).clamp(0.0, 1.0),
                before_note: format!(
                    "{:.0}% of {}",
                    before as f64 / window as f64 * 100.0,
                    thousands(window)
                ),
                after_tokens: thousands(after),
                after_fraction: (after as f64 / window as f64).clamp(0.0, 1.0),
                after_note: format!(
                    "{:.0}% of {}",
                    after as f64 / window as f64 * 100.0,
                    thousands(window)
                ),
                kept,
                folded: vec![format!(
                    "{folded_messages} messages folded into the summary"
                )],
                recovers: thousands(before.saturating_sub(after)),
                call_cost: "in the next /session stats".into(),
                cache_cost: "the cache re-primes on the next turn".into(),
                ..Default::default()
            });
            open_sheet(model, Screen::Compact);
            Ok(Done::Opened)
        }
        SlashAction::Export(path) => {
            let Some(store) = agent.session.as_ref() else {
                return Ok(Done::Note("no session to export".into()));
            };
            let output = PathBuf::from(path.unwrap_or_else(|| "session.html".into()));
            let started = Instant::now();
            let said = crate::export::export_session(store, &output)?;
            // The `4d` ledger: what left the session, measured from it.
            let turns = agent
                .messages
                .iter()
                .filter(|message| message.role == "user")
                .count();
            let calls = agent
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .filter(|part| matches!(part, davinci_ai::MessageContent::ToolCall { .. }))
                .count();
            let images = agent
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .filter(|part| matches!(part, davinci_ai::MessageContent::Image { .. }))
                .count();
            let size = std::fs::metadata(&output)
                .map(|meta| {
                    let bytes = meta.len();
                    if bytes >= 1_000_000 {
                        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
                    } else {
                        format!("{:.1} KB", bytes as f64 / 1_000.0)
                    }
                })
                .unwrap_or_default();
            model.export_ledger = Some(ExportLedger {
                included: vec![
                    format!("{turns} turns of prose and thinking"),
                    format!("{calls} tool calls with their output"),
                    "every Δ hunk".into(),
                    format!("{images} images, inlined as base64"),
                ],
                excluded: vec![
                    (
                        State::Attention,
                        "absolute paths · kept, they name your machine".into(),
                    ),
                    (
                        State::Attention,
                        "branch names and commit subjects · kept".into(),
                    ),
                ],
                size,
                elapsed: format!("{:.1}s", started.elapsed().as_secs_f64()),
                gist: output.display().to_string(),
                ..Default::default()
            });
            open_sheet(model, Screen::Export);
            let _ = said;
            Ok(Done::Opened)
        }
        SlashAction::Name(name) => {
            let Some(store) = agent.session.as_mut() else {
                return Ok(Done::Note("no session to name".into()));
            };
            store.set_name(&name).map_err(|err| err.to_string())?;
            Ok(Done::Said(format!("named this session {name}")))
        }
        SlashAction::Fork => {
            let mut host = crate::loaded_extension_host(parsed);
            host.runtime_flag_values = crate::flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeFork);
            if host.last_result_cancelled() {
                return Ok(Done::Note("fork cancelled".into()));
            }
            let Some(store) = agent.session.as_ref() else {
                return Ok(Done::Note("no session to fork".into()));
            };
            let session_dir = crate::resolved_session_dir(parsed, &agent.cwd);
            let next = store
                .fork(
                    store.leaf_id.as_deref().unwrap_or(&store.header.id),
                    &session_dir,
                )
                .map_err(|err| err.to_string())?;
            agent.load_from_session(next)?;
            model.composer_epoch = model.composer_epoch.saturating_add(1);
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(format!("forked to {}", session_id(agent))))
        }
        SlashAction::Clone => {
            let Some(store) = agent.session.as_ref() else {
                return Ok(Done::Note("no session to clone".into()));
            };
            let session_dir = crate::resolved_session_dir(parsed, &agent.cwd);
            let next = store
                .clone_session(&session_dir)
                .map_err(|err| err.to_string())?;
            agent.load_from_session(next)?;
            model.composer_epoch = model.composer_epoch.saturating_add(1);
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(format!("cloned to {}", session_id(agent))))
        }
        SlashAction::Import(path) => {
            if path.is_empty() {
                return Ok(Done::Note("usage: /import <path.jsonl>".into()));
            }
            let expanded = davinci_session::expand_tilde(&path);
            let next = JsonlSession::open(&expanded).map_err(|err| err.to_string())?;
            agent.load_from_session(next)?;
            model.composer_epoch = model.composer_epoch.saturating_add(1);
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(format!("imported {}", session_id(agent))))
        }
        SlashAction::Copy => match agent.last_assistant_text() {
            Some(text) => {
                davinci_tui::copy_text(&text);
                Ok(Done::Said("copied the last reply to the clipboard".into()))
            }
            None => Ok(Done::Note("no agent messages to copy yet".into())),
        },
        SlashAction::Share => Ok(Done::Said(crate::share_current_session(agent)?)),
        SlashAction::SessionInfo => {
            let models = crate::available_models(parsed);
            let found = models
                .iter()
                .find(|item| item.provider == agent.provider && item.id == agent.model_id);
            let stats = crate::rpc::session_stats_for_agent(agent, found);
            let waste = agent
                .session
                .as_ref()
                .map(|store| crate::cache_stats::compute_cache_waste(&store.entries, &0.3));
            Ok(Done::Said(crate::format_session_info(
                &stats,
                waste.as_ref(),
            )))
        }
        SlashAction::SetModel(value) => {
            let (provider, model_id) =
                crate::parse_model_ref(&agent.provider.clone(), Some(&value));
            // The `3a` catalog refuses a dimmed row; a hand-typed `/model` has
            // to refuse the same pair, or the switch succeeds and every turn
            // after it comes back unauthenticated.
            if !model_has_credential(parsed, &provider, &model_id) {
                return Ok(Done::Note(format!(
                    "no credential for {provider} — /login {provider} adds one"
                )));
            }
            agent.provider = provider;
            agent.model_id = model_id;
            crate::loaded_extension_host(parsed).emit(ExtensionEvent::ModelSelect {
                provider: agent.provider.clone(),
                model: agent.model_id.clone(),
            });
            adopt_model(parsed, agent, model);
            let remembered = persist_model_choice(agent);
            Ok(Done::Said(match remembered {
                Ok(()) => format!("model {} / {}", agent.provider, agent.model_id),
                Err(err) => format!(
                    "model {} / {} · this run only ({err})",
                    agent.provider, agent.model_id
                ),
            }))
        }
        SlashAction::SetThinking(level) => {
            let Some(parsed_level) = davinci_protocol::ThinkingLevel::parse(&level) else {
                return Ok(Done::Note(format!("unknown thinking level {level}")));
            };
            agent.thinking_level = parsed_level;
            crate::loaded_extension_host(parsed)
                .emit(ExtensionEvent::ThinkingLevelSelect { level });
            // The header reads `model.thinking_level`, not the agent's, so the
            // label stayed on the old level until the next model switch.
            sync_thinking_state(agent, model);
            let remembered = persist_thinking_choice(
                &agent.provider,
                &agent.model_id,
                agent.thinking_level.as_str(),
            );
            Ok(Done::Said(match remembered {
                Ok(()) => format!("thinking level {}", agent.thinking_level.as_str()),
                Err(err) => format!(
                    "thinking level {} · this run only ({err})",
                    agent.thinking_level.as_str()
                ),
            }))
        }
        SlashAction::Tree => {
            let mut host = crate::loaded_extension_host(parsed);
            host.runtime_flag_values = crate::flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeTree);
            if host.last_result_cancelled() {
                return Ok(Done::Note("tree navigation cancelled".into()));
            }
            host.emit(ExtensionEvent::UiPromptStart {
                kind: "tree".into(),
            });
            host.emit(ExtensionEvent::SessionTree);
            host.emit(ExtensionEvent::UiPromptEnd {
                kind: "tree".into(),
            });
            if open_tree_sheet(agent, model) {
                Ok(Done::Opened)
            } else {
                Ok(Done::Note(
                    "no session tree yet — it grows with the first turn".into(),
                ))
            }
        }
        SlashAction::Reload => {
            let keybindings_started = Instant::now();
            model.keybindings = davinci_tui::Keybindings::load(&agent_dir)
                .with_voice(model.voice.enabled)
                .0;
            let keybindings_ms = keybindings_started.elapsed().as_millis();
            let resources_started = Instant::now();
            crate::apply_discovered_resources(parsed, agent);
            let resources_ms = resources_started.elapsed().as_millis();
            let host_started = Instant::now();
            let mut host = crate::loaded_extension_host(parsed);
            let host_ms = host_started.elapsed().as_millis();
            host.runtime_flag_values = crate::flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionStart);
            // A reload that leaves the old command list behind has not
            // reloaded: an extension added on disk has to become reachable.
            model.slash_commands = crate::interactive_slash_commands(agent, parsed);
            model.corpus = corpus(agent, &model.slash_commands, &model.sessions);
            model.corpus_total = model.corpus.len();

            // The `6b` workshop sheet: what loaded, what failed, what it costs.
            let context_tokens: usize = agent
                .context_files
                .iter()
                .map(|file| file.body.len() / 4)
                .sum();
            let reload = vec![
                (
                    State::Done,
                    format!(
                        "keybindings · {} bindings",
                        davinci_tui::get_keybindings().len()
                    ),
                    format!("{keybindings_ms}ms"),
                    None,
                ),
                (
                    State::Done,
                    format!(
                        "skills · {} found, none loaded until named",
                        agent.skills.len()
                    ),
                    format!("{resources_ms}ms"),
                    None,
                ),
                (
                    State::Done,
                    format!(
                        "context files · {} files · {}k",
                        agent.context_files.len(),
                        (context_tokens as f64 / 1000.0).round() as u64
                    ),
                    String::new(),
                    None,
                ),
                (
                    State::Done,
                    format!("extensions · {} javascript, 4 native", host.js.len()),
                    format!("{host_ms}ms"),
                    None,
                ),
            ];
            let native_commands = |prefix: &str| {
                crate::native_extensions::command_specs()
                    .iter()
                    .filter(|(name, _, _)| name.starts_with(prefix))
                    .count()
            };
            let native_tool_names = host.native_tool_names();
            let native_tools = |prefix: &str| {
                native_tool_names
                    .iter()
                    .filter(|name| name.starts_with(prefix))
                    .count()
            };
            let native = vec![
                (
                    State::Done,
                    "vector-memory".to_string(),
                    format!(
                        "{} tools · {} cmds",
                        native_tools("memory") + 1,
                        native_commands("memory")
                    ),
                ),
                (
                    State::Done,
                    "token-governor".to_string(),
                    format!(
                        "{} tools · {} cmds",
                        native_tools("retrieve"),
                        native_commands("governor")
                    ),
                ),
                (
                    State::Done,
                    "graph".to_string(),
                    format!(
                        "{} tools · {} cmds",
                        native_tools("graph"),
                        native_commands("graph")
                    ),
                ),
                (
                    State::Done,
                    "security-scan".to_string(),
                    format!(
                        "{} tools · {} cmds",
                        native_tools("sec"),
                        native_commands("sec")
                    ),
                ),
            ];
            let javascript: Vec<(State, String, String)> = host
                .js
                .iter()
                .map(|ext| {
                    let name = std::path::Path::new(&ext.path)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| ext.path.clone());
                    (
                        State::Done,
                        name,
                        format!("{} tools · {} cmds", ext.tools.len(), ext.commands.len()),
                    )
                })
                .collect();
            let node = match crate::js_host::find_node() {
                Some(path) => path.display().to_string(),
                None => "node not found — JS extensions inactive".into(),
            };
            let native_specs = host.native_tool_specs();
            let schema_tokens = serde_json::to_string(&davinci_agent::tool_specs())
                .map(|text| text.len() / 4)
                .unwrap_or(0)
                + serde_json::to_string(&native_specs)
                    .map(|text| text.len() / 4)
                    .unwrap_or(0);
            let window = agent.context_window.max(1) as f64;
            let builtin = davinci_agent::tool_specs().len();
            let native_count = native_specs.len();
            let extension_count: usize = host.js.iter().map(|ext| ext.tools.len()).sum();
            let total = (builtin + native_count + extension_count).max(1) as f64;
            model.facts.tool_count = builtin + native_count + extension_count;
            model.facts.command_count = model.slash_commands.len();
            model.facts.tool_schema_tokens = schema_tokens as u64;
            model.workshop = Some(WorkshopSheet {
                reload,
                native,
                javascript,
                node,
                node_note: "one process, reused".into(),
                node_elapsed: format!("{host_ms}ms"),
                schema: format!(
                    "{}k · {:.0}%",
                    (schema_tokens as f64 / 1000.0).round() as u64,
                    schema_tokens as f64 / window * 100.0
                ),
                tools: vec![
                    (
                        "built-in tools".into(),
                        builtin.to_string(),
                        builtin as f64 / total,
                        "read write edit bash powershell grep find ls".into(),
                    ),
                    (
                        "native tools".into(),
                        native_count.to_string(),
                        native_count as f64 / total,
                        "memory, governor, graph, sec".into(),
                    ),
                    (
                        "extension tools".into(),
                        extension_count.to_string(),
                        extension_count as f64 / total,
                        String::new(),
                    ),
                ],
            });
            open_sheet(model, Screen::Officina);
            Ok(Done::Opened)
        }
        // A bare `/login` lists every provider and where its credential came
        // from (`3d`); with a provider named, the handshake runs detached.
        SlashAction::Login { provider, key } if provider.is_empty() => {
            let _ = key;
            open_login_sheet(parsed, model);
            Ok(Done::Opened)
        }
        SlashAction::Login { provider, key } => Ok(Done::Detach(Detached::Login { provider, key })),
        SlashAction::Logout { provider } => {
            let mut storage = davinci_ai::AuthStorage::create().map_err(|err| err.to_string())?;
            match provider {
                Some(provider) => {
                    storage.remove(&provider).map_err(|err| err.to_string())?;
                    Ok(Done::Said(format!("removed {provider}")))
                }
                None => {
                    let names = crate::logout_provider_options()
                        .map_err(|err| format!("could not read stored credentials: {err}"))?;
                    if names.is_empty() {
                        return Ok(Done::Note(
                            "no stored credentials to remove. /logout only removes what /login saved; environment variables and models.json are untouched".into(),
                        ));
                    }
                    Ok(Done::Ask(Question::Logout {
                        providers: names.iter().map(|item| item.id.clone()).collect(),
                    }))
                }
            }
        }
        SlashAction::Mcp => {
            if agent.tool_context.mcp.rows().is_empty() {
                Ok(Done::Said(
                    "No MCP servers configured. Run davinci doctor if this is unexpected — it lists MCP config files that failed validation. Otherwise, run davinci mcp --help to learn more."
                        .into(),
                ))
            } else {
                open_mcp_sheet(agent, model);
                Ok(Done::Opened)
            }
        }
        SlashAction::ShowCost => Ok(Done::Said(crate::format_session_cost(parsed, agent))),
        SlashAction::ShowStatus => Ok(Done::Said(crate::format_session_status(parsed, agent))),
        SlashAction::Agents => {
            let settings =
                crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
            let trusted =
                crate::settings::is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
            Ok(Done::Said(
                crate::agent_profiles::format_agent_profiles_status(&agent.cwd, None, trusted),
            ))
        }
        SlashAction::Tasks => {
            open_task_board_sheet(agent, model);
            Ok(Done::Opened)
        }
    }
}

fn session_id(agent: &Agent) -> String {
    agent
        .session
        .as_ref()
        .map(|store| store.header.id.clone())
        .unwrap_or_else(|| "in-memory".into())
}

/// Remember the model in hand, the way TS `setDefaultModelAndProvider`
/// (`vendor/pi/packages/coding-agent/src/core/agent-session.ts`) and the legacy
/// chrome's `SessionAction::SelectModelAsDefault` both do. A switch that only
/// lived in memory came back as the previous provider on the next start, which
/// read from the outside as a login that had been lost.
fn persist_model_choice(agent: &Agent) -> Result<(), String> {
    let dir = crate::default_agent_dir();
    let mut stored = crate::settings::load_settings(&dir);
    stored.default_provider = Some(agent.provider.clone());
    stored.default_model = Some(agent.model_id.clone());
    let level = agent.thinking_level.as_str().to_string();
    stored
        .model_thinking_levels
        .get_or_insert_with(Default::default)
        .insert(
            format!("{}/{}", agent.provider, agent.model_id),
            level.clone(),
        );
    stored.default_thinking_level = Some(level);
    crate::settings::save_settings(&dir, &stored)
}

/// Remember the thinking level for this model, mirroring TS
/// `settingsManager.setModelThinkingLevel`: the per-model entry is what a later
/// start reads back, and the global default follows the last choice so a model
/// with no entry of its own still opens where the user left off.
fn persist_thinking_choice(provider: &str, model_id: &str, level: &str) -> Result<(), String> {
    let dir = crate::default_agent_dir();
    let mut stored = crate::settings::load_settings(&dir);
    let mut levels = stored.model_thinking_levels.take().unwrap_or_default();
    levels.insert(format!("{provider}/{model_id}"), level.to_string());
    stored.model_thinking_levels = Some(levels);
    stored.default_thinking_level = Some(level.to_string());
    crate::settings::save_settings(&dir, &stored)
}

/// Whether the runtime has a credential for this provider/model pair. `/model
/// <provider>/<id>` typed by hand reaches providers the `3a` catalog draws
/// dimmed, and switching to one silently left every later turn unauthenticated.
fn model_has_credential(parsed: &crate::args::Args, provider: &str, model_id: &str) -> bool {
    crate::load_model_runtime(parsed)
        .available
        .iter()
        .any(|entry| entry.provider == provider && entry.id == model_id)
}

/// After a model switch: the name in the header, the cap on the context meter,
/// and the row Cogitator marks as the one in hand all move together.
fn sync_thinking_state(agent: &Agent, model: &mut Model) {
    model.thinking_level = agent.thinking_level.as_str().to_string();
    model.thinking_levels = crate::current_runtime_model(agent)
        .map(|runtime| {
            crate::get_supported_thinking_levels(&runtime)
                .iter()
                .map(|level| level.as_str().to_string())
                .collect()
        })
        .unwrap_or_default();
}

/// Keep a previous or remembered level within the destination model's support.
fn supported_thinking_choice(
    preferred: davinci_protocol::ThinkingLevel,
    supported: &[davinci_protocol::ThinkingLevel],
) -> davinci_protocol::ThinkingLevel {
    let all = davinci_protocol::ThinkingLevel::all();
    let rank = |level| {
        all.iter()
            .position(|candidate| *candidate == level)
            .unwrap_or(0)
    };
    supported
        .iter()
        .copied()
        .rev()
        .find(|level| rank(*level) <= rank(preferred))
        .or_else(|| supported.first().copied())
        .unwrap_or(davinci_protocol::ThinkingLevel::Off)
}

fn adopt_model(parsed: &crate::args::Args, agent: &mut Agent, model: &mut Model) {
    if let Some(found) = crate::available_models(parsed)
        .into_iter()
        .find(|item| item.provider == agent.provider && item.id == agent.model_id)
    {
        agent.context_window = found.context_window;
        let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
        let remembered = stored
            .model_thinking_levels
            .as_ref()
            .and_then(|levels| levels.get(&format!("{}/{}", agent.provider, agent.model_id)))
            .and_then(|level| davinci_protocol::ThinkingLevel::parse(level))
            .unwrap_or(agent.thinking_level);
        agent.thinking_level =
            supported_thinking_choice(remembered, &crate::get_supported_thinking_levels(&found));
    }
    model.model_index = model
        .models
        .iter()
        .position(|item| item.provider == agent.provider && item.id == agent.model_id)
        .unwrap_or(model.model_index);
    model.model_name = agent.model_id.clone();
    model.active_provider = agent.provider.clone();
    model.context.1 = agent.context_window;
    sync_thinking_state(agent, model);
}

/// Run the davinci TUI against a live agent until the user leaves.
pub fn run(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    raw: &[String],
    host: Arc<Mutex<ExtensionHost>>,
    migrated_auth_providers: &[String],
) -> Result<i32, String> {
    use davinci_tui::davinci::app::{self, Flow};
    use davinci_tui::davinci::runtime::Session;

    let cwd = agent.cwd.clone();
    let mut model = davinci_tui::davinci::boot(raw, 100, 44);
    let session_dir = davinci_session::default_session_dir();
    model.cwd = cwd.display().to_string();
    model.startup = crate::davinci_sources::startup(&cwd, "", !agent.messages.is_empty());
    // The initial scan can be slow in a home directory or a large repository.
    // Use the same background worker as subsequent refreshes.
    let dresser = crate::davinci_sources::WorkspaceDresser::start(cwd.clone(), session_dir.clone());
    dresser.request();
    crate::davinci_surfaces::dress_from_extensions(&mut model, &cwd, agent);
    model.model_name = agent.model_id.clone();
    model.active_provider = agent.provider.clone();
    model.config_path = crate::default_agent_dir()
        .join("config.json")
        .display()
        .to_string();
    model.transcript = transcript_from(&agent.messages);
    model.models = crate::available_models(parsed)
        .iter()
        .map(|entry| {
            ModelItem::new(
                &format!("{} / {}", entry.provider, entry.id),
                &davinci_tui::davinci::views::chrome::thousands(entry.context_window),
            )
            .of(&entry.provider, &entry.id, entry.context_window)
        })
        .collect();
    // What the composer completes: the same slash corpus, extension providers
    // and `/login` list the legacy chrome offers, through the same engine.
    model.slash_commands = crate::interactive_slash_commands(agent, parsed);
    // The palette lists what the composer completes, so it is built from the
    // same command list rather than from the built-ins alone.
    model.corpus = corpus(agent, &model.slash_commands, &model.sessions);
    model.corpus_total = model.corpus.len();
    model.extra_autocomplete = crate::interactive_extra_autocomplete(parsed);
    model.login_providers = crate::interactive_login_providers(parsed);
    model.model_names = model.models.iter().map(|item| item.name.clone()).collect();
    sync_thinking_state(agent, &mut model);
    sync_permission_state(agent, &mut model);
    model.show_tool_output =
        crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd)
            .show_tool_output
            .unwrap_or(false);
    // A resumed session opens on the ledger it closed on (phase 3).
    if agent.restore_todos() {
        let list = agent
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        model.plan = plan_from_todos(&list);
    }
    model.model_index = model
        .models
        .iter()
        .position(|item| item.name.ends_with(&agent.model_id))
        .unwrap_or(0);
    refresh_context(&mut model, agent);

    // Everything the old chrome printed before the first prompt: extension
    // startup events, notices, the trust warning, the changelog, the resource
    // listing. The transcript is the only place a davinci shell can say any of
    // it (design.md §6).
    {
        let mut host = host.lock().map_err(|err| err.to_string())?;
        host.runtime_flag_values = crate::flag_values_json(parsed);
        host.emit(crate::extension_host::ExtensionEvent::ResourcesDiscover {
            cwd: cwd.display().to_string(),
            reason: "startup".into(),
        });
        host.emit(crate::extension_host::ExtensionEvent::SessionStart);
        let calls = std::mem::take(&mut host.ui_calls);
        drop(host);
        apply_ui_calls(&mut model, &calls);
    }
    crate::start_catalog_refresh_async(parsed);
    for entry in opening_block(parsed, agent, migrated_auth_providers) {
        model.transcript.push(entry);
    }
    model.startup.found = opening_found(parsed, agent);

    // The user's own bindings, which davinci was rendering the defaults of
    // however `~/.pi/agent/keybindings.json` read.
    model.keybindings = davinci_tui::Keybindings::load(&crate::default_agent_dir());
    let mut voice = crate::voice_input::VoiceInput::new(&mut model);
    // Every `pi.registerShortcut` an extension made, resolved against those
    // bindings so a shortcut never shadows a reserved chord. Without this,
    // every registered shortcut was dead under davinci.
    {
        let host = host.lock().map_err(|err| err.to_string())?;
        let (shortcuts, diagnostics) = host.resolve_shortcuts(&model.keybindings);
        model.extension_shortcuts = shortcuts;
        drop(host);
        if let Some(warning) = diagnostics.first() {
            model.transcript.push(Entry::Gap);
            model
                .transcript
                .push(Entry::tool(State::Attention, "instrumenta", warning, None));
        }
    }

    // The stored settings the legacy startup honours, applied where davinci
    // has the same surface: the completion-list height, the model scope, the
    // terminal progress report, and the double-escape action (below). The
    // presentation settings the legacy transcript reads — mermaid mode, code
    // indent, editor padding, hidden thinking blocks — have no davinci
    // surface: the transcript's shape is the design contract's (design.md §3).
    let stored_settings = crate::settings::load_merged_settings(&crate::default_agent_dir(), &cwd);
    apply_theme_setting(&mut model, &stored_settings);
    let startup_checks =
        crate::startup::start_background_checks(crate::VERSION, stored_settings.clone());
    if let Some(rows) = stored_settings.autocomplete_max_visible {
        model.suggestion_rows = rows.clamp(3, 20) as usize;
    }
    model.terminal_progress = stored_settings.show_terminal_progress();
    if let Some(enabled) = stored_settings
        .enabled_models
        .clone()
        .filter(|enabled| !enabled.is_empty())
    {
        model
            .models
            .retain(|item| enabled.contains(&format!("{}/{}", item.provider, item.id)));
        model.model_names = model.models.iter().map(|item| item.name.clone()).collect();
        model.model_index = model
            .models
            .iter()
            .position(|item| item.name.ends_with(&agent.model_id))
            .unwrap_or(0);
    }
    model.double_escape_action = stored_settings
        .double_escape_action
        .clone()
        .unwrap_or_else(|| "tree".into());
    let mut last_escape: Option<Instant> = None;
    // `ctrl+c` at rest, as in claude code: the first press clears what was
    // being typed and arms the exit; a second within this window leaves.
    // Any other key disarms it.
    const DOUBLE_CTRL_C_MS: u64 = 2_000;
    let mut last_ctrl_c: Option<Instant> = None;
    // Images pasted with ctrl+v, sent with the next prompt so a vision model
    // is reachable from this interface.
    let mut attached_images: Vec<davinci_ai::MessageContent> = Vec::new();

    let mut terminal = Session::open().map_err(|err| err.to_string())?;
    // From here the alternate screen is ours, so a `println!` from shared code
    // is queued for the transcript instead of painted over the frame.
    crate::set_hosted_tui_active(true);
    // Proving the panic hook gives the terminal back needs a panic to happen
    // inside the alternate screen, which nothing else can arrange.
    if std::env::var("PI_DAVINCI_PANIC_FIXTURE").is_ok() {
        panic!("PI_DAVINCI_PANIC_FIXTURE");
    }
    terminal
        .set_title(&crate::format_terminal_title(
            agent
                .session
                .as_ref()
                .and_then(|store| store.display_name())
                .as_deref(),
            &cwd,
        ))
        .map_err(|err| err.to_string())?;
    let (width, height) = terminal.size().map_err(|err| err.to_string())?;
    model.width = width;
    model.height = height;

    let mut last_tick = Instant::now();
    // The `5a` sheet shows a run that is progressing on its own thread; it is
    // re-read from `graph-status` once a second while it is open.
    let mut last_graph_refresh = Instant::now();
    // The question the `Ask` instrument is currently putting, if any. It lives
    // here rather than in the model because only this module knows what the
    // rows mean.
    let mut pending: Option<Question> = None;
    if crate::settings::should_run_first_time_setup(&crate::settings::settings_path(
        &crate::default_agent_dir(),
    )) {
        pending = Some(Question::FirstRun);
        model.ask = Question::FirstRun.ask(agent);
        open_ask_overlay(&mut model);
    }

    // `pi "do the thing"` and `--file` open straight into a turn rather than
    // into an empty composer.
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &cwd);
    let prepared = crate::file_processor::prepare_initial_message(
        &parsed.messages,
        &parsed.file_args,
        None,
        &cwd,
        stored.image_auto_resize(),
    )?;
    let mut openers: Vec<(String, Vec<davinci_ai::MessageContent>)> = Vec::new();
    if let Some(text) = prepared.text.clone() {
        openers.push((text, prepared.images.clone()));
    }
    openers.extend(
        prepared
            .remaining_messages
            .iter()
            .map(|text| (text.clone(), Vec::new())),
    );
    for (text, images) in openers {
        let mut shell = Shell {
            voice: &mut voice,
            parsed,
            agent,
            model: &mut model,
            terminal: &mut terminal,
            host: &host,
            pending: &mut pending,
            cwd: &cwd,
            dresser: &dresser,
            images: &mut attached_images,
        };
        // What the composer's own submit would have pushed, so the opening
        // question is on screen above its answer.
        if !shell.model.transcript.is_empty() {
            shell.model.transcript.push(Entry::Gap);
        }
        shell.model.transcript.push(Entry::user(&text));
        shell.model.transcript.push(Entry::Gap);
        shell.model.transcript.push(Entry::agent("davinci"));
        shell.model.running = true;
        let next = submit_prompt(&mut shell, &text, &images);
        match next {
            Next::Go => {}
            // The terminal has to be given back on the way out, whatever the
            // opening turn came to.
            Next::Leave | Next::Fail(_) => {
                crate::set_hosted_tui_active(false);
                terminal.close().map_err(|err| err.to_string())?;
                for (_, line) in crate::take_hosted_lines() {
                    if !line.trim().is_empty() {
                        std::println!("{line}");
                    }
                }
                return match next {
                    Next::Fail(err) => Err(err),
                    _ => Ok(0),
                };
            }
        }
    }

    let result = loop {
        // Lines shared code printed while the screen was ours belong in the
        // transcript, which is the only place a davinci shell can say anything
        // (design.md §6).
        for (kind, line) in crate::take_hosted_lines() {
            let line = line.trim_end().to_string();
            if line.trim().is_empty() {
                continue;
            }
            model.transcript.push(Entry::Gap);
            if kind == "error" {
                model
                    .transcript
                    .push(Entry::tool(State::Attention, "instrumenta", &line, None));
            } else {
                model.transcript.push(Entry::prose(&line));
            }
        }
        // Between turns only: a live turn holds indices into the transcript,
        // and trimming under it would repoint its open tool lines.
        model.trim_transcript();
        // A workspace re-read that finished on the dresser's thread.
        if dresser.apply_ready(&mut model) {
            model.corpus = corpus(agent, &model.slash_commands, &model.sessions);
            model.corpus_total = model.corpus.len();
        }
        for notices in startup_checks.try_iter() {
            model.transcript.extend(startup_notice_entries(&notices));
        }
        if last_graph_refresh.elapsed() >= Duration::from_secs(1) {
            last_graph_refresh = Instant::now();
            if model.screen == Screen::GraphRun
                || model
                    .graph_run
                    .as_ref()
                    .is_some_and(|run| run.outcome().is_none())
            {
                refresh_graph_sheet(&mut model, &host);
            }
            if model.screen == Screen::Securitas {
                let locked = host.lock().unwrap_or_else(|e| e.into_inner());
                if let Ok(Some(value)) = locked.execute_native_command("sec-report", "") {
                    model.security = Some(security_sheet(&value));
                }
            }
        }
        voice.tick(&mut model, terminal.input_pending());
        if let Err(err) = terminal.draw(&model) {
            break Err(err.to_string());
        }
        voice.drawn();
        if model.voice.active && !terminal.mic_visible() {
            voice.cancel(&mut model);
        }

        let timeout = davinci_tui::davinci::runtime::TICK.saturating_sub(last_tick.elapsed());
        let timeout = if voice.polling() {
            timeout.min(Duration::from_millis(40))
        } else {
            timeout
        };
        match terminal.poll_event(timeout) {
            Ok(Some(event)) => match event {
                crossterm::event::Event::Key(key)
                    if key.kind != crossterm::event::KeyEventKind::Release =>
                {
                    // A credential overlay owns every key. Do this before
                    // voice, extension shortcuts, clipboard handling, and
                    // terminal hooks so the candidate cannot enter any other
                    // input path or transcript.
                    if model.overlay == Some(Overlay::SecretInput) {
                        if paste_secret_clipboard(
                            &mut model,
                            key,
                            crate::external_editor::clipboard_text,
                        ) {
                            continue;
                        }
                        let next = match app::handle_key(&mut model, key) {
                            Flow::SecretInputSubmitted(candidate) => on_secret_input(
                                &mut Shell {
                                    voice: &mut voice,
                                    parsed,
                                    agent,
                                    model: &mut model,
                                    terminal: &mut terminal,
                                    host: &host,
                                    pending: &mut pending,
                                    cwd: &cwd,
                                    dresser: &dresser,
                                    images: &mut attached_images,
                                },
                                candidate.into_inner(),
                            ),
                            Flow::Quit => Next::Leave,
                            Flow::Interrupt | Flow::Continue => Next::Go,
                            Flow::Submit(_) | Flow::Choose(_) | Flow::CyclePermissionMode => {
                                Next::Go
                            }
                        };
                        match next {
                            Next::Go => {}
                            Next::Leave => break Ok(0),
                            Next::Fail(err) => break Err(err),
                        }
                        continue;
                    }
                    if graph_setup::key(&mut model, &mut pending, agent, key) {
                        continue;
                    }
                    // An extension's registered shortcut gets the chord before
                    if voice.key(&mut model, key) {
                        last_escape = None;
                        continue;
                    }
                    // the shell's own keys, exactly as the legacy loop gives
                    // it. Resolution already refused the reserved chords.
                    let claimed = davinci_tui::key_event_bytes(&key).and_then(|data| {
                        model
                            .extension_shortcuts
                            .iter()
                            .find(|(chord, _)| davinci_tui::key_to_bytes(chord) == data)
                            .cloned()
                    });
                    if let Some((chord, path)) = claimed {
                        let mut shell = Shell {
                            voice: &mut voice,
                            parsed,
                            agent,
                            model: &mut model,
                            terminal: &mut terminal,
                            host: &host,
                            pending: &mut pending,
                            cwd: &cwd,
                            dresser: &dresser,
                            images: &mut attached_images,
                        };
                        match shell.run_shortcut(&chord, &path) {
                            Next::Go => {}
                            Next::Leave => break Ok(0),
                            Next::Fail(err) => break Err(err),
                        }
                        continue;
                    }
                    // An extension that registered `onTerminalInput` sees the
                    // raw chord before the shell's own keys, exactly as the
                    // legacy loop offers it through `dispatch_terminal_input`.
                    if model.terminal_input_registered {
                        let taken = davinci_tui::key_event_bytes(&key).is_some_and(|data| {
                            let mut locked = host.lock().unwrap_or_else(|err| err.into_inner());
                            locked.dispatch_terminal_input(&data)
                        });
                        if taken {
                            let mut shell = Shell {
                                voice: &mut voice,
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                dresser: &dresser,
                                images: &mut attached_images,
                            };
                            match apply_host_effects(&mut shell) {
                                Next::Go => {}
                                Next::Leave => break Ok(0),
                                Next::Fail(err) => break Err(err),
                            }
                            continue;
                        }
                    }
                    // ctrl+v reads the clipboard the way the legacy chrome's
                    // `PasteClipboard` does: an image is attached to the next
                    // prompt — the only way a vision model is reachable from
                    // this interface — and text goes into the composer.
                    if key.code == crossterm::event::KeyCode::Char('v')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        if let Some(png) = crate::external_editor::clipboard_image_png() {
                            let (bytes, note) = match crate::image_convert::resize_image_in_process(
                                &png,
                                "image/png",
                            ) {
                                Some(resized) => {
                                    let note = format!("{}x{}", resized.width, resized.height);
                                    let bytes = if resized.was_resized
                                        && resized.mime_type == "image/png"
                                    {
                                        resized.bytes
                                    } else {
                                        png
                                    };
                                    (bytes, note)
                                }
                                None => (png, "image".to_string()),
                            };
                            let data = base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                bytes,
                            );
                            attached_images.push(davinci_ai::MessageContent::Image {
                                data,
                                mime_type: "image/png".into(),
                            });
                            let count = attached_images.len();
                            model.extensions.set_status(
                                "§images",
                                Some(&format!(
                                    "{count} image{} attached ({note}) · sent with the next prompt",
                                    if count == 1 { "" } else { "s" },
                                )),
                            );
                            continue;
                        }
                        if let Some(text) = crate::external_editor::clipboard_text() {
                            model.paste(&text);
                            continue;
                        }
                        // An empty clipboard falls through to the editor's own
                        // ctrl+v, if the user bound one.
                    }
                    // Two escapes on an empty composer, within the same window
                    // the legacy chrome uses, run the stored double-escape
                    // action: the session tree by default, a fork if asked.
                    if key.code == crossterm::event::KeyCode::Esc
                        && key.modifiers.is_empty()
                        && model.overlay.is_none()
                        && model.suggestions.is_none()
                        && model.screen == davinci_tui::davinci::model::Screen::Agent
                        && !model.codex_open()
                        && model.composer.trim().is_empty()
                        && model.double_escape_action != "none"
                    {
                        let now = Instant::now();
                        let doubled = last_escape.is_some_and(|prev| {
                            now.duration_since(prev)
                                < Duration::from_millis(davinci_tui::DOUBLE_ESCAPE_MS)
                        });
                        if doubled {
                            last_escape = None;
                            match davinci_tui::DoubleEscapeAction::parse(
                                &model.double_escape_action,
                            ) {
                                davinci_tui::DoubleEscapeAction::Fork => {
                                    let mut shell = Shell {
                                        voice: &mut voice,
                                        parsed,
                                        agent,
                                        model: &mut model,
                                        terminal: &mut terminal,
                                        host: &host,
                                        pending: &mut pending,
                                        cwd: &cwd,
                                        dresser: &dresser,
                                        images: &mut attached_images,
                                    };
                                    match on_line(&mut shell, "/fork") {
                                        Next::Go => {}
                                        Next::Leave => break Ok(0),
                                        Next::Fail(err) => break Err(err),
                                    }
                                }
                                _ => model.toggle_overlay(Overlay::Sessions),
                            }
                            continue;
                        }
                        last_escape = Some(now);
                    } else {
                        last_escape = None;
                    }
                    let is_ctrl_c = key.code == crossterm::event::KeyCode::Char('c')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL);
                    if !is_ctrl_c {
                        last_ctrl_c = None;
                        model.exit_armed = false;
                    }
                    let was = model.screen;
                    let next = match app::handle_key(&mut model, key) {
                        Flow::Interrupt => {
                            let now = Instant::now();
                            let doubled = last_ctrl_c.is_some_and(|prev| {
                                now.duration_since(prev) < Duration::from_millis(DOUBLE_CTRL_C_MS)
                            });
                            if doubled {
                                run_stop_hooks(&mut Shell {
                                    voice: &mut voice,
                                    parsed,
                                    agent,
                                    model: &mut model,
                                    terminal: &mut terminal,
                                    host: &host,
                                    pending: &mut pending,
                                    cwd: &cwd,
                                    dresser: &dresser,
                                    images: &mut attached_images,
                                });
                                Next::Leave
                            } else {
                                if !model.composer.trim().is_empty() {
                                    let _ = model.composer.editor_mut().submit();
                                }
                                last_ctrl_c = Some(now);
                                model.exit_armed = true;
                                Next::Go
                            }
                        }
                        Flow::Quit => {
                            run_stop_hooks(&mut Shell {
                                voice: &mut voice,
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                dresser: &dresser,
                                images: &mut attached_images,
                            });
                            Next::Leave
                        }
                        Flow::Submit(line) => on_line(
                            &mut Shell {
                                voice: &mut voice,
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                dresser: &dresser,
                                images: &mut attached_images,
                            },
                            &line,
                        ),
                        Flow::Choose(choice) => on_choice(
                            &mut Shell {
                                voice: &mut voice,
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                dresser: &dresser,
                                images: &mut attached_images,
                            },
                            choice,
                        ),
                        Flow::CyclePermissionMode => {
                            cycle_permission_mode(agent, &mut model);
                            Next::Go
                        }
                        Flow::SecretInputSubmitted(candidate) => on_secret_input(
                            &mut Shell {
                                voice: &mut voice,
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                dresser: &dresser,
                                images: &mut attached_images,
                            },
                            candidate.into_inner(),
                        ),
                        Flow::Continue => Next::Go,
                    };
                    // Recall is a search, so it runs when the instrument is
                    // summoned rather than being kept warm behind it.
                    if model.screen == davinci_tui::davinci::model::Screen::Memoria
                        && was != davinci_tui::davinci::model::Screen::Memoria
                    {
                        let query = recall_query(&model, agent);
                        let (hits, meta) = crate::davinci_surfaces::recall(&cwd, &query, 8);
                        model.recall = hits;
                        model.recall_meta = meta;
                        model.recall_index = 0;
                    }
                    match next {
                        Next::Go => {}
                        Next::Leave => break Ok(0),
                        Next::Fail(err) => break Err(err),
                    }
                }
                crossterm::event::Event::Resize(width, height) => {
                    // A measure of nothing wraps nothing: the prose measure and
                    // the panel insets are all derived from this.
                    model.width = width.max(20);
                    model.height = height.max(4);
                }
                // A paste is text, never keys: dropping it made every newline
                // in the pasted block submit a turn of its own. On Windows the
                // burst of keys the console delivers is reassembled into this
                // event by the paste filter behind `poll_event`.
                crossterm::event::Event::Paste(text) => {
                    if graph_setup::paste(&mut model, &mut pending, agent, &text) {
                        continue;
                    }
                    if model.overlay == Some(Overlay::SecretInput)
                        || !voice.paste(&mut model, &text)
                    {
                        model.paste(&text);
                    }
                }
                crossterm::event::Event::Mouse(mouse) => {
                    if terminal.handle_model_mouse(&mut model, mouse) {
                        voice.toggle(&mut model);
                    }
                }
                _ => {}
            },
            Ok(None) => {}
            Err(err) => break Err(err.to_string()),
        }

        if last_tick.elapsed() >= davinci_tui::davinci::runtime::TICK {
            model.tick = model.tick.wrapping_add(1);
            last_tick = Instant::now();
            // A job that finishes between turns is news at once, not at
            // the next prompt.
            poll_jobs(&agent.tool_context.jobs, &mut model);
        }
    };

    crate::set_hosted_tui_active(false);
    {
        let locked = host.lock().unwrap_or_else(|e| e.into_inner());
        let _ = locked.execute_native_command("sec-abort", "");
    }
    terminal.close().map_err(|err| err.to_string())?;
    // Anything shared code printed while the screen was ours, said now that
    // stdout is the user's again rather than dropped.
    for (_, line) in crate::take_hosted_lines() {
        if !line.trim().is_empty() {
            std::println!("{line}");
        }
    }
    result
}

/// Everything the shell owes the user before the first prompt: startup
/// notices, the model scope, the trust warning, the changelog on a version
/// bump, the resources that loaded, and any custom messages the session
/// already holds. The old chrome printed these into its transcript; so does
/// this one, because the transcript is the interface (design.md §6).
fn opening_block(
    parsed: &crate::args::Args,
    agent: &Agent,
    migrated_auth_providers: &[String],
) -> Vec<Entry> {
    let agent_dir = crate::default_agent_dir();
    let stored = crate::settings::load_merged_settings(&agent_dir, &agent.cwd);
    let mut out: Vec<Entry> = Vec::new();

    let (_, models_json_error) = crate::load_available_models(parsed);
    let notices = crate::startup::StartupNotices {
        models_json_error,
        migrated_auth_providers: migrated_auth_providers.to_vec(),
        ..crate::startup::StartupNotices::default()
    };
    out.extend(startup_notice_entries(&notices));

    if !crate::settings::is_trusted(&stored, &agent.cwd, parsed.project_trust_override)
        && crate::trust::has_trust_requiring_project_resources(&agent.cwd)
    {
        out.push(Entry::Gap);
        out.push(Entry::tool(
            State::Attention,
            "instrumenta",
            "this project is not trusted, so its .pi resources are ignored — /trust to decide",
            None,
        ));
    }

    let entries = crate::changelog::parse_changelog(&crate::changelog::changelog_path());
    let has_messages = agent.session.as_ref().is_some_and(|store| {
        store
            .entries
            .iter()
            .any(|entry| entry.entry_type == "message")
    });
    let display = crate::changelog::changelog_for_display(
        stored.last_changelog_version.as_deref(),
        crate::VERSION,
        &entries,
        has_messages,
    );
    if let Some(text) = display.markdown {
        out.push(Entry::Gap);
        out.push(Entry::prose(text.trim()));
    }

    out.extend(custom_messages(agent));
    out
}

fn startup_notice_entries(notices: &crate::startup::StartupNotices) -> Vec<Entry> {
    let mut out = Vec::new();
    for (kind, line) in crate::startup::format_notices(notices) {
        out.push(Entry::Gap);
        if kind == "warning" || kind == "error" {
            out.push(Entry::tool(State::Attention, "instrumenta", &line, None));
        } else {
            out.push(Entry::prose(&line));
        }
    }
    out
}

/// What the session found, for the rows under the mark on the `1a` screen:
/// the context files, skills and prompts that loaded, and any model scope.
/// Kept off the transcript so a fresh session opens on the emblem rather
/// than on bookkeeping.
fn opening_found(parsed: &crate::args::Args, agent: &Agent) -> Vec<String> {
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    if stored.quiet_startup && !parsed.verbose {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut loaded: Vec<String> = Vec::new();
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    if !agent.context_files.is_empty() {
        loaded.push(plural(
            agent.context_files.len(),
            "context file",
            "context files",
        ));
    }
    if !agent.skills.is_empty() {
        loaded.push(plural(agent.skills.len(), "skill", "skills"));
    }
    if !agent.templates.is_empty() {
        loaded.push(plural(agent.templates.len(), "prompt", "prompts"));
    }
    if !loaded.is_empty() {
        found.push(format!("loaded {}", loaded.join(" · ")));
    }
    if !parsed.models.is_empty() {
        found.push(format!("models scoped to {}", parsed.models.join(", ")));
    }
    found
}

/// Custom messages an extension wrote into the session, replayed on open so a
/// resumed session reads the same as it did when it was live.
fn custom_messages(agent: &Agent) -> Vec<Entry> {
    let Some(store) = agent.session.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in &store.entries {
        if entry.entry_type != "custom_message" && entry.entry_type != "custom" {
            continue;
        }
        let text = entry
            .message
            .as_ref()
            .map(davinci_tui::CustomMessage::text_content)
            .filter(|text| !text.is_empty())
            .or_else(|| {
                entry
                    .extra
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        out.push(Entry::Gap);
        out.push(Entry::prose(text.trim()));
    }
    out
}

/// Apply everything the shared extension host has queued — UI rows first,
/// then session calls — with the davinci transcript as the visible surface.
///
/// The davinci counterpart of `apply_host_session_calls` in main.rs: the same
/// gating events before a fork, switch or tree move, the same state effects
/// through `apply_session_calls`, and the same consequences afterwards — a
/// reload rebuilds the command list, an unregistered provider leaves the
/// model picker. A `sendMessage` carrying `triggerTurn` runs a full davinci
/// turn, spinner and all, rather than a blind blocking completion.
fn apply_host_effects(shell: &mut Shell<'_>) -> Next {
    use crate::extension_host::ExtensionEvent;

    drain_ui_calls(shell.model, shell.host);
    let op_of = |call: &serde_json::Value| -> String {
        call.get("op")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let calls = {
        let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        let taken = std::mem::take(&mut host.session_calls);
        let mut allowed = Vec::new();
        for call in taken {
            match op_of(&call).as_str() {
                "fork" => {
                    host.emit(ExtensionEvent::SessionBeforeFork);
                    if host.last_result_cancelled() {
                        continue;
                    }
                }
                "switchSession" => {
                    host.emit(ExtensionEvent::SessionBeforeSwitch);
                    if host.last_result_cancelled() {
                        continue;
                    }
                }
                "navigateTree" => {
                    host.emit(ExtensionEvent::SessionBeforeTree);
                    if host.last_result_cancelled() {
                        continue;
                    }
                }
                "reload" => host.emit(ExtensionEvent::SessionShutdown {
                    reason: "reload".into(),
                }),
                _ => {}
            }
            allowed.push(call);
        }
        allowed
    };
    if calls.is_empty() {
        return Next::Go;
    }

    let wants_turn = calls.iter().any(|call| {
        matches!(op_of(call).as_str(), "sendMessage" | "sendUserMessage")
            && call
                .get("options")
                .and_then(|options| options.get("triggerTurn"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
    });
    let failures = crate::apply_session_calls(
        Some(shell.parsed),
        shell.agent,
        crate::SessionCallUi::Davinci(shell.model),
        &calls,
        false,
    );
    let calls: Vec<_> = calls
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !failures.iter().any(|(failed, _)| failed == index))
        .map(|(_, call)| call)
        .collect();

    if calls.iter().any(|call| op_of(call) == "reload") {
        crate::apply_discovered_resources(shell.parsed, shell.agent);
        shell.model.slash_commands = crate::interactive_slash_commands(shell.agent, shell.parsed);
    }
    for call in &calls {
        if op_of(call) != "unregisterProvider" {
            continue;
        }
        let Some(name) = call.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        shell.model.models.retain(|item| item.provider != name);
        shell
            .model
            .model_names
            .retain(|model| !model.starts_with(&format!("{name}/")));
        shell
            .model
            .login_providers
            .retain(|provider| provider != name);
    }
    if calls.iter().any(|call| {
        matches!(
            op_of(call).as_str(),
            "fork" | "switchSession" | "newSession"
        )
    }) {
        // The session in hand changed, so the transcript is rebuilt from it —
        // which wipes the status lines `apply_session_calls` just pushed.
        // State the change again on the fresh transcript.
        shell.model.transcript = transcript_from(&shell.agent.messages);
        for (_, error) in &failures {
            shell.note(error);
        }
        for call in &calls {
            if matches!(
                op_of(call).as_str(),
                "fork" | "switchSession" | "newSession"
            ) {
                if let Some(note) = crate::session_call_note(call) {
                    shell.note(&note);
                }
            }
        }
    }
    shell.redress();

    if wants_turn && failures.is_empty() {
        shell.model.running = true;
        match run_turns(shell) {
            Next::Go => {}
            other => return other,
        }
        shell.redress();
    }
    Next::Go
}

/// Apply the UI calls the loaded extensions have made, returning a window
/// title if one was asked for.
///
/// Davinci honours the calls that are rows or text — widgets, header, footer,
/// status, notifications, the composer, the title, the working message (as an
/// extension status row) and `onTerminalInput` (the shell offers raw chords
/// to the host before its own keys). It deliberately ignores the ones that
/// would take over the design itself: `setTheme` (one palette, negotiated
/// from the terminal, §2), `setEditorComponent` (the composer is the shell's,
/// §6), `setWorkingIndicator` (exactly two things animate off one clock, §8),
/// and `setToolsExpanded` (a tool call is one line, §6).
pub fn apply_ui_calls(model: &mut Model, calls: &[serde_json::Value]) -> Option<String> {
    let mut title = None;
    let text_lines = |value: Option<&serde_json::Value>| -> Vec<String> {
        value
            .and_then(serde_json::Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let field = |call: &serde_json::Value, key: &str| -> String {
        call.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    for call in calls {
        match call.get("op").and_then(serde_json::Value::as_str) {
            Some("setWidget") => {
                let key = field(call, "key");
                let mut lines = text_lines(call.get("lines"));
                if lines.is_empty() {
                    if let Some(content) = call.get("content").and_then(serde_json::Value::as_str) {
                        lines = content.lines().map(str::to_string).collect();
                    }
                }
                let below = field(call, "placement") == "belowEditor";
                model.extensions.set_widget(&key, lines, below);
            }
            Some("setStatus") => {
                let key = field(call, "key");
                let text = call.get("text").and_then(serde_json::Value::as_str);
                model.extensions.set_status(&key, text);
            }
            Some("setHeader") => model.extensions.header = text_lines(call.get("lines")),
            Some("setFooter") => model.extensions.footer = text_lines(call.get("lines")),
            Some("notify") => {
                let message = field(call, "message");
                if !message.trim().is_empty() {
                    model.transcript.push(Entry::Gap);
                    model.transcript.push(Entry::tool(
                        State::Attention,
                        "instrumenta",
                        message.trim(),
                        None,
                    ));
                }
            }
            Some("setEditorText") => model.replace_composer(field(call, "text")),
            Some("pasteToEditor") => {
                model.composer.push_str(&field(call, "text"));
                model.mark_caret_moved();
            }
            // The shell's own working row states what the *turn* costs and is
            // written by the turn loop alone, so an extension's working
            // message takes the shape extensions are given instead: a status
            // row above the composer.
            Some("setWorkingMessage") => {
                let message = call.get("message").and_then(serde_json::Value::as_str);
                model.extensions.set_status("§working", message);
            }
            Some("setWorkingVisible") => {
                let visible = call
                    .get("visible")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                if !visible {
                    model.extensions.set_status("§working", None);
                }
            }
            Some("onTerminalInput") => model.terminal_input_registered = true,
            Some("setTitle") => {
                let value = field(call, "title");
                if !value.is_empty() {
                    title = Some(value);
                }
            }
            _ => {}
        }
    }
    title
}

// --- the command sheets (screens 3a–6d) -------------------------------------
//
// Each builder reads live state — the model runtime, the settings store, the
// session store, git, the native extensions — into the sheet the Elixir
// reference designs (`docs/ui/davinci_tui/lib/davinci/views/*.ex`), then opens
// the screen. Nothing here is fixture data.

fn open_sheet(model: &mut Model, screen: Screen) {
    model.feature_scroll = 0;
    model.section_offset = None;
    model.overlay_offset = None;
    model.section_notice = None;
    model.running = false;
    model.overlay = None;
    model.screen = screen;
}

fn open_ask_overlay(model: &mut Model) {
    model.approval_instructions = None;
    model.overlay_offset = None;
    model.ask_index = 0;
    model.overlay = Some(Overlay::Ask);
}

pub(crate) fn open_decision_modal(
    model: &mut Model,
    question: &davinci_agent::decisions::DecisionQuestion,
) {
    let options = question
        .options
        .iter()
        .map(
            |opt| davinci_tui::davinci::views::decision_modal::DecisionModalOption {
                id: opt.id.clone(),
                label: opt.label.clone(),
                explanation: opt.explanation.clone(),
                recommended: opt.recommended,
            },
        )
        .collect();
    let state = davinci_tui::davinci::views::decision_modal::DecisionModalState::new(
        question.id.clone(),
        question.id.clone(),
        question.plan_revision,
        question.title.clone(),
        question.question.clone(),
        question.materiality.clone(),
        question.evidence_refs.clone(),
        options,
        question.allow_custom,
        question.custom_only,
    );
    model.decision_modal = Some(state);
    model.overlay = Some(Overlay::Ask);
}

#[allow(dead_code)]
pub(crate) fn open_rewind_modal(
    model: &mut Model,
    preview: &davinci_agent::runtime::rewind::RewindPreview,
    checkpoint_name: &str,
    checkpoint_time: &str,
) {
    let files = preview
        .files
        .iter()
        .map(|f| davinci_tui::davinci::views::rewind::RewindFileSummary {
            path: f.path.clone(),
            classification: f.classification.clone(),
            is_conflict: f.is_conflict,
            conflict_reason: f.conflict_reason.clone(),
        })
        .collect();
    let irreversible_effects = preview
        .irreversible_effects
        .iter()
        .map(
            |e| davinci_tui::davinci::views::rewind::RewindIrreversibleSummary {
                operation_id: e.operation_id.clone(),
                kind: e.kind.clone(),
                details: e.redacted_details().to_string(),
            },
        )
        .collect();
    let state = davinci_tui::davinci::views::rewind::RewindModalState::new(
        preview.checkpoint_id.clone(),
        checkpoint_name,
        checkpoint_time,
        preview.preview_digest.clone(),
        files,
        preview.conflict_count,
        irreversible_effects,
    );
    model.rewind_modal = Some(state);
    model.overlay = Some(Overlay::Ask);
}

fn attach_section_notice(model: &mut Model, text: &str) {
    if model.screen != Screen::Agent {
        let text = text.trim();
        if !text.is_empty() {
            model.section_notice = Some(text.to_string());
        }
    }
}

/// A path under the home directory, said the way the artboards say it:
/// `%USERPROFILE%\.pi\agent\auth.json` on Windows, `~/.pi/agent/auth.json`
/// elsewhere. Paths outside home are returned as they are.
fn home_label(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let Some(home) = davinci_session::home_dir() else {
        return text;
    };
    let home = home.display().to_string();
    match text.strip_prefix(&home) {
        Some(rest) if cfg!(windows) => format!("%USERPROFILE%{rest}"),
        Some(rest) => format!("~{rest}"),
        None => text,
    }
}

/// `2h ago` for a file's age; empty when the file is not there.
fn refreshed_label(path: &std::path::Path) -> String {
    let age = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|elapsed| elapsed.as_secs());
    match age {
        None => String::new(),
        Some(seconds) => age_label(seconds),
    }
}

fn age_label(seconds: u64) -> String {
    let text = crate::davinci_sources::humanise(seconds);
    if text == "just now" {
        text
    } else {
        format!("{text} ago")
    }
}

/// Reasoning tokens over the session's assistant turns: what the last turn
/// spent, and the share of output they took. Both empty/zero when no
/// provider reported any.
fn thinking_facts(entries: &[davinci_session::SessionEntry]) -> (String, f64) {
    let mut last = None;
    let mut reasoning = 0u64;
    let mut output = 0u64;
    for entry in entries.iter().filter(|entry| entry.entry_type == "message") {
        let Some(message) = entry.message.as_ref() else {
            continue;
        };
        if message.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let turn_output = usage
            .get("output")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let turn_reasoning = usage.get("reasoning").and_then(serde_json::Value::as_u64);
        output += turn_output;
        if let Some(spent) = turn_reasoning {
            reasoning += spent;
            last = Some(spent);
        }
    }
    let last_turn = last
        .map(|spent| {
            if spent >= 1_000 {
                format!("{:.1}k tokens", spent as f64 / 1_000.0)
            } else {
                format!("{spent} tokens")
            }
        })
        .unwrap_or_default();
    let share = if output > 0 && reasoning > 0 {
        reasoning as f64 / output as f64
    } else {
        0.0
    };
    (last_turn, share)
}

/// Bytes the session files take on disk; the volume total is not read, so
/// the cap is `0` and the sheet omits the meter.
fn sessions_disk(paths: &[std::path::PathBuf]) -> Option<(u64, u64)> {
    if paths.is_empty() {
        return None;
    }
    let used = paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    Some((used, 0))
}

fn open_mcp_sheet(agent: &Agent, model: &mut Model) {
    let servers = agent
        .tool_context
        .mcp
        .rows()
        .into_iter()
        .map(|row| McpServerRow {
            name: row.name,
            transport: row.transport,
            status: row.status,
            tools: row.tools,
            error: row.error,
        })
        .collect();
    model.mcp = Some(McpSheet {
        servers,
        config_path: crate::default_agent_dir()
            .join("mcp.json")
            .display()
            .to_string(),
    });
    open_sheet(model, Screen::Mcp);
}

/// `3a` — models from authenticated providers in the live runtime snapshot.
fn open_models_sheet(parsed: &crate::args::Args, agent: &Agent, model: &mut Model) {
    let snapshot = crate::load_model_runtime(parsed);
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    let available: std::collections::BTreeSet<String> = snapshot
        .available
        .iter()
        .map(|entry| format!("{}/{}", entry.provider, entry.id))
        .collect();
    let ringed: std::collections::BTreeSet<String> = model
        .models
        .iter()
        .map(|item| format!("{}/{}", item.provider, item.id))
        .collect();
    model.catalog_session_only = false;
    model.catalog_search = false;
    let prior_model = if model.screen == Screen::Models {
        model
            .catalog
            .get(model.catalog_index)
            .map(|row| (row.provider.clone(), row.id.clone()))
    } else {
        None
    };
    model.catalog = snapshot
        .all
        .iter()
        .map(|entry| {
            let key = format!("{}/{}", entry.provider, entry.id);
            let ready = available.contains(&key);
            let auth = snapshot.auth.get(&entry.provider);
            let price = if entry.cost.input == 0.0 && entry.cost.output == 0.0 {
                "free · local".to_string()
            } else {
                format!("{:.2} · {:.2}", entry.cost.input, entry.cost.output)
            };
            let supported = crate::get_supported_thinking_levels(entry);
            let preferred = if entry.provider == agent.provider && entry.id == agent.model_id {
                agent.thinking_level
            } else {
                stored
                    .model_thinking_levels
                    .as_ref()
                    .and_then(|levels| levels.get(&key))
                    .and_then(|level| davinci_protocol::ThinkingLevel::parse(level))
                    .unwrap_or(agent.thinking_level)
            };
            let resolved = supported_thinking_choice(preferred, &supported);
            CatalogRow {
                reasoning_index: supported
                    .iter()
                    .position(|level| *level == resolved)
                    .unwrap_or(0),
                reasoning_levels: supported
                    .iter()
                    .map(|level| level.as_str().to_string())
                    .collect(),
                name: key.clone(),
                detail: String::new(),
                window: davinci_tui::davinci::views::chrome::thousands(entry.context_window),
                thinking: if entry.reasoning {
                    "budget".into()
                } else {
                    "none".into()
                },
                price,
                credential: if ready {
                    Credential::Ready
                } else {
                    Credential::Absent
                },
                note: if ready {
                    auth.map(|check| check.kind.clone())
                        .unwrap_or_else(|| "ready".into())
                } else {
                    "none".into()
                },
                ring: ringed.contains(&key),
                provider: entry.provider.clone(),
                id: entry.id.clone(),
            }
        })
        .collect();
    model.catalog_index = order_catalog(&mut model.catalog, &agent.provider, &agent.model_id);
    if let Some((provider, id)) = prior_model {
        if let Some(index) = model
            .catalog
            .iter()
            .position(|row| row.provider == provider && row.id == id)
        {
            model.catalog_index = index;
        }
    }
    let providers: std::collections::BTreeSet<&str> = model
        .catalog
        .iter()
        .map(|row| row.provider.as_str())
        .collect();
    let ready: std::collections::BTreeSet<&str> = model
        .catalog
        .iter()
        .filter(|row| row.credential == Credential::Ready)
        .map(|row| row.provider.as_str())
        .collect();
    model.facts.catalog_total = model.catalog.len();
    model.facts.catalog_shown = model.catalog.len();
    model.facts.providers_total = providers.len();
    model.facts.providers_ready = ready.len();
    let models_path = davinci_ai::models_json_path(&crate::default_agent_dir());
    model.facts.catalog_path = home_label(&models_path);
    model.facts.catalog_refreshed = refreshed_label(&models_path);
    open_sheet(model, Screen::Models);
}

/// Keep only usable providers, with the active provider leading, and show
/// newer featured models first within each provider. Focus the top row; the
/// active model is marked independently by the view.
fn order_catalog(catalog: &mut Vec<CatalogRow>, provider: &str, _model_id: &str) -> usize {
    catalog.retain(|row| row.credential == Credential::Ready);
    catalog.sort_by_key(|row| {
        (
            row.credential != Credential::Ready,
            row.provider != provider,
            row.provider.clone(),
            davinci_tui::model_picker_rank(&row.id),
        )
    });
    0
}

fn apply_theme_setting(model: &mut Model, settings: &crate::settings::Settings) {
    model.theme = model
        .theme
        .with_name(settings.theme.as_deref().unwrap_or("dark"));
}

/// `3b` — the settings sheet, from the same list the legacy overlay builds,
/// so both surfaces offer the same keys with the same ramps. A row whose
/// merged value differs from the user file's was set by the project.
fn open_settings_sheet(agent: &Agent, model: &mut Model) {
    let dir = crate::default_agent_dir();
    let user = crate::settings::load_settings(&dir);
    let merged = crate::settings::load_merged_settings(&dir, &agent.cwd);
    let user_list = davinci_tui::interactive_settings_list(
        &crate::settings::to_interactive_config(&user, "dark"),
    );
    let merged_list = davinci_tui::interactive_settings_list(
        &crate::settings::to_interactive_config(&merged, "dark"),
    );
    apply_theme_setting(model, &merged);
    let prior_setting = if model.screen == Screen::Settings {
        model
            .settings_rows
            .get(model.settings_index)
            .map(|row| row.key.clone())
    } else {
        model.settings_tab = 1;
        model.settings_focus = davinci_tui::davinci::views::settings::Focus::Search;
        model.settings_details = false;
        model.settings_query.clear();
        None
    };
    model.settings_rows = merged_list
        .items
        .into_iter()
        .map(|item| {
            let project = user_list
                .items
                .iter()
                .find(|own| own.id == item.id)
                .map(|own| own.current_value != item.current_value)
                .unwrap_or(false);
            SettingRow {
                label: item.label,
                value: item.current_value,
                project,
                values: if item.id == "theme" {
                    vec!["dark".into(), "light".into(), "vox".into()]
                } else {
                    item.values
                },
                description: item.description.unwrap_or_default(),
                key: item.id,
                note: String::new(),
            }
        })
        .collect();
    model
        .settings_rows
        .sort_by_key(|row| davinci_tui::davinci::views::settings::group_rank(&row.key));
    model.settings_index = prior_setting
        .and_then(|key| model.settings_rows.iter().position(|row| row.key == key))
        .unwrap_or(0);
    model.facts.settings_keys = model.settings_rows.len();
    open_sheet(model, Screen::Settings);
}

/// `3c` — the thinking sheet: every level this model supports, as the budget
/// it actually sends, with its share of the 64k ceiling and a warning when a
/// level would take a third of the window before the turn starts.
fn refresh_thinking_sheet(agent: &Agent, model: &mut Model) {
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    let budgets = stored.thinking_budgets.clone();
    let levels = crate::current_runtime_model(agent)
        .map(|runtime| crate::get_supported_thinking_levels(&runtime))
        .unwrap_or_else(|| davinci_protocol::ThinkingLevel::all().to_vec());
    let window = agent.context_window.max(1) as f64;
    model.thinking_rows = levels
        .iter()
        .map(|level| {
            let budget = if *level == davinci_protocol::ThinkingLevel::Off {
                0u32
            } else {
                davinci_ai::thinking_budget_for_level(*level, budgets.as_ref())
            };
            let of_window = budget as f64 / window;
            let warn = of_window >= 1.0 / 3.0;
            let maps_to = if budget == 0 {
                "disabled → none".to_string()
            } else if warn {
                format!("! {:.0}% of the window", of_window * 100.0)
            } else {
                let sent = davinci_ai::clamp_reasoning(*level)
                    .map(|resolved| resolved.as_str().to_string())
                    .unwrap_or_else(|| "none".into());
                format!("{budget} → {sent}")
            };
            ThinkingRow {
                level: level.as_str().to_string(),
                budget: if budget == 0 {
                    "0".into()
                } else {
                    format!("{:.1}k", budget as f64 / 1000.0)
                },
                fraction: budget as f64 / 65_536.0,
                maps_to,
                warn,
            }
        })
        .collect();
    model.thinking_index = model
        .thinking_rows
        .iter()
        .position(|row| row.level == agent.thinking_level.as_str())
        .unwrap_or(0);
    let entries = agent
        .session
        .as_ref()
        .map(|store| store.entries.as_slice())
        .unwrap_or(&[]);
    let (last_turn, share) = thinking_facts(entries);
    model.facts.thinking_reserve = String::new();
    model.facts.thinking_last_turn = last_turn;
    model.facts.thinking_output_share = share;
}

/// `3d` — provider credentials: every provider `/login` offers, with where
/// its credential came from, from the same auth resolution `/model` uses.
fn open_login_sheet(parsed: &crate::args::Args, model: &mut Model) {
    let snapshot = crate::load_model_runtime(parsed);
    let names = crate::interactive_login_providers(parsed);
    model.providers = names
        .iter()
        .map(|name| match snapshot.auth.get(name) {
            Some(check) => ProviderRow {
                name: name.clone(),
                method: check.kind.clone(),
                source: check.source.clone(),
                state: Credential::Ready,
            },
            None => ProviderRow {
                name: name.clone(),
                method: "api key or oauth".into(),
                source: "never configured".into(),
                state: Credential::Absent,
            },
        })
        .collect();
    model.login_index = 0;
    model.device_code = None;
    model.facts.auth_path = home_label(&davinci_ai::default_auth_path());
    model.facts.auth_mode = if cfg!(unix) {
        "0600".into()
    } else {
        String::new()
    };
    open_sheet(model, Screen::Login);
}

/// A binding's action id, said in words: `cursorWordLeft` → `cursor word left`.
fn humanize_action(action: &str) -> String {
    let name = action.rsplit('.').next().unwrap_or(action);
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `3e` — the keymap, grouped by the surface a key belongs to, from the real
/// binding table plus every extension shortcut.
fn open_keys_sheet(model: &mut Model) {
    let mut bindings: Vec<_> = model
        .keybindings
        .bindings()
        .filter(|(action, _)| *action != "app.thinking.cycle")
        .collect();
    bindings.sort_by_key(|(action, _)| *action);
    let mut instruments: Vec<(String, String)> = Vec::new();
    let mut composer: Vec<(String, String)> = Vec::new();
    let mut lists: Vec<(String, String)> = Vec::new();
    let mut other: Vec<(String, String)> = Vec::new();
    for (action, keys) in &bindings {
        let row = (keys.join(", "), humanize_action(action));
        if action.starts_with("davinci.") {
            instruments.push(row);
        } else if action.starts_with("tui.editor.") || action.starts_with("tui.input.") {
            composer.push(row);
        } else if action.starts_with("tui.select.") {
            lists.push(row);
        } else {
            other.push(row);
        }
    }
    let mut groups = vec![
        KeymapGroup {
            title: "INSTRUMENTS".into(),
            note: "over the transcript".into(),
            rows: instruments,
        },
        KeymapGroup {
            title: "COMPOSER".into(),
            note: String::new(),
            rows: composer,
        },
        KeymapGroup {
            title: "LISTS".into(),
            note: "inside a panel".into(),
            rows: lists,
        },
    ];
    if !other.is_empty() {
        groups.push(KeymapGroup {
            title: "SESSION".into(),
            note: String::new(),
            rows: other,
        });
    }
    if !model.extension_shortcuts.is_empty() {
        groups.push(KeymapGroup {
            title: "EXTENSIONS".into(),
            note: "registered by extensions".into(),
            rows: model
                .extension_shortcuts
                .iter()
                .map(|(key, path)| {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    (key.clone(), name)
                })
                .collect(),
        });
    }
    model.facts.keys_count = bindings.len() + model.extension_shortcuts.len();
    model.facts.keys_surfaces = groups.len();
    model.keymap = groups;
    model.keys_offset = 0;
    open_sheet(model, Screen::Keys);
}

/// `4a` — the session list, with what resuming each one would carry, from the
/// real store. Token counts are an estimate and say so.
fn open_resume_sheet(parsed: &crate::args::Args, agent: &Agent, model: &mut Model) {
    let session_dir = crate::resolved_session_dir(parsed, &agent.cwd);
    let mut found = davinci_session::discover_sessions(&session_dir, None).unwrap_or_default();
    found.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    model.session_count = found.len();
    let now = davinci_session::now_ms();
    model.resume_sessions = found
        .iter()
        .map(|summary| {
            let last = summary
                .all_messages_text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let last = if last.chars().count() > 60 {
                let tail: String = last
                    .chars()
                    .rev()
                    .take(57)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                format!("…{tail}")
            } else {
                last
            };
            ResumeRow {
                name: crate::davinci_sources::session_name(summary),
                branch: String::new(),
                turns: summary.message_count.to_string(),
                tokens: format!(
                    "~{}",
                    davinci_tui::davinci::views::chrome::thousands(
                        (summary.all_messages_text.len() / 4) as u64
                    )
                ),
                model: String::new(),
                touched: crate::davinci_sources::humanise(
                    now.saturating_sub(summary.modified_at) / 1_000,
                ),
                named: summary
                    .name
                    .as_deref()
                    .is_some_and(|name| !name.trim().is_empty()),
                warning: None,
                note: summary
                    .parent_session_id
                    .as_ref()
                    .map(|parent| {
                        format!("forked from {}", parent.chars().take(8).collect::<String>())
                    })
                    .unwrap_or_default(),
                last,
                path: summary.path.display().to_string(),
                size: String::new(),
                commit: String::new(),
            }
        })
        .collect();
    model.resume_index = 0;
    let paths: Vec<std::path::PathBuf> = found.iter().map(|summary| summary.path.clone()).collect();
    model.facts.sessions_disk = sessions_disk(&paths);
    open_sheet(model, Screen::Resume);
}

/// `4b` — the session tree, from the current session's entry graph: one node
/// per user turn, spacers carrying the trunk between them.
fn open_tree_sheet(agent: &Agent, model: &mut Model) -> bool {
    let Some(store) = agent.session.as_ref() else {
        return false;
    };
    let turn_text = |entry: &davinci_session::SessionEntry| -> Option<String> {
        let message = entry.message.as_ref()?;
        if message.get("role").and_then(serde_json::Value::as_str) != Some("user") {
            return None;
        }
        let text = davinci_tui::CustomMessage::text_content(message);
        let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
        Some(clip(line, 48))
    };
    let turns: Vec<(&davinci_session::SessionEntry, String)> = store
        .entries
        .iter()
        .filter(|entry| entry.entry_type == "message")
        .filter_map(|entry| turn_text(entry).map(|text| (entry, text)))
        .collect();
    if turns.is_empty() {
        return false;
    }
    let on_path: std::collections::BTreeSet<&str> = {
        // The chain from the leaf back to the root is the trunk in hand.
        let by_id: std::collections::BTreeMap<&str, &davinci_session::SessionEntry> = store
            .entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut path = std::collections::BTreeSet::new();
        let mut cursor = store.leaf_id.as_deref();
        while let Some(id) = cursor {
            path.insert(id);
            cursor = by_id.get(id).and_then(|entry| entry.parent_id.as_deref());
        }
        path
    };
    let stamp = |ms: u64| -> String {
        let seconds = ms / 1_000;
        format!("{:02}:{:02}", (seconds / 3_600) % 24, (seconds / 60) % 60)
    };
    let mut rows: Vec<TreeNode> = Vec::new();
    let count = turns.len();
    for (index, (entry, text)) in turns.iter().enumerate() {
        if index > 0 {
            rows.push(TreeNode {
                trunk: "│".into(),
                ..TreeNode::default()
            });
        }
        let last = index + 1 == count;
        let active = on_path.contains(entry.id.as_str())
            && store.leaf_id.as_deref() == Some(entry.id.as_str());
        rows.push(TreeNode {
            trunk: if index == 0 {
                String::new()
            } else if last {
                "└── ".into()
            } else {
                "├── ".into()
            },
            state: Some(if active {
                State::Active
            } else if on_path.contains(entry.id.as_str()) {
                State::Done
            } else {
                State::Queued
            }),
            id: Some(format!("{:02}", index + 1)),
            label: Some(text.clone()),
            meta: Some(stamp(entry.timestamp)),
            entry_id: entry.id.clone(),
            detail: None,
        });
    }
    model.tree_index = rows
        .iter()
        .position(|row| row.state == Some(State::Active))
        .or_else(|| rows.iter().rposition(|row| row.id.is_some()))
        .unwrap_or(0);
    model.session_tree = rows;
    model.facts.session_name = store
        .display_name()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| store.header.id.chars().take(8).collect());
    model.facts.session_turns = count;
    let stats = davinci_session::session_usage_stats(&store.entries);
    model.facts.session_cost = if stats.cost > 0.0 {
        format!("${:.2}", stats.cost)
    } else {
        String::new()
    };
    open_sheet(model, Screen::Tree);
    true
}

/// `6d` — the Δ review, from the real working tree: every changed file with
/// its counts and its own first hunk. `/diff` is davinci's own command — the
/// legacy chrome has no such screen.
fn open_diff_sheet(shell: &mut Shell<'_>) -> Next {
    use std::process::Command;
    let cwd = shell.cwd;
    let changes = crate::davinci_sources::git_changes(cwd);
    if changes.is_empty() {
        shell.note("nothing to review — the working tree is clean");
        return Next::Go;
    }
    let numstat: Vec<(String, u32, u32)> = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["diff", "--numstat", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\t');
                    let adds = fields.next()?.parse().ok()?;
                    let dels = fields.next()?.parse().ok()?;
                    Some((fields.next()?.to_string(), adds, dels))
                })
                .collect()
        })
        .unwrap_or_default();
    let hunk_of = |path: &str| -> (Vec<Hunk>, String, String) {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["diff", "--unified=2", "HEAD", "--", path])
            .output();
        let Ok(output) = output else {
            return (Vec::new(), String::new(), String::new());
        };
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let hunk_count = text.lines().filter(|line| line.starts_with("@@")).count();
        let mut rows = Vec::new();
        let mut header = String::new();
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with("@@") {
                if inside {
                    break; // the first hunk is enough for the sheet
                }
                inside = true;
                // git's `@@ -214,7 +214,18 @@`, without the minus the sheet
                // already says with its colour.
                header = clip(&line.replacen("@@ -", "@@ ", 1), 90);
                continue;
            }
            if !inside {
                continue;
            }
            if rows.len() >= 12 {
                break;
            }
            let (kind, body) = match line.chars().next() {
                Some('+') => (HunkKind::Add, &line[1..]),
                Some('-') => (HunkKind::Del, &line[1..]),
                _ => (HunkKind::Context, line.trim_start_matches(' ')),
            };
            rows.push(Hunk::new(kind, &clip(body, 90)));
        }
        let note = match hunk_count {
            0 => String::new(),
            1 => "hunk 1 of 1".to_string(),
            n => format!("hunk 1 of {n}"),
        };
        (rows, note, header)
    };
    let mut total_adds = 0u32;
    let mut total_dels = 0u32;
    let files: Vec<ReviewFile> = changes
        .iter()
        .take(24)
        .map(|change| {
            let normalized = change.path.replace('\\', "/");
            let counted = numstat.iter().find(|(path, _, _)| path == &normalized);
            let (adds, dels) = counted
                .map(|(_, adds, dels)| (Some(*adds), Some(*dels)))
                .unwrap_or((None, None));
            total_adds += adds.unwrap_or(0);
            total_dels += dels.unwrap_or(0);
            let untracked = change.status == "?";
            let (hunk, hunk_note, hunk_header) = if untracked {
                (
                    Vec::new(),
                    "new file · untracked".to_string(),
                    String::new(),
                )
            } else {
                hunk_of(&normalized)
            };
            ReviewFile {
                state: match change.status.as_str() {
                    "D" => State::Failed,
                    "A" | "?" => State::Done,
                    _ => State::Delta,
                },
                path: change.path.clone(),
                adds,
                dels,
                tests: "not run".into(),
                test_state: State::Queued,
                hunk_note,
                hunk_header,
                hunk,
            }
        })
        .collect();
    let behind = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-list", "--count", "HEAD..@{upstream}"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u32>()
                .ok()
        })
        .filter(|count| *count > 0)
        .map(|count| format!("{count} commits behind"))
        .unwrap_or_default();
    shell.model.review = Some(ReviewSheet {
        files,
        adds: total_adds,
        dels: total_dels,
        branch: shell.model.branch.clone(),
        behind,
        warning: String::new(),
        tests: "run the tests before trusting the diff · !cargo test".into(),
    });
    shell.model.diff_index = 0;
    open_sheet(shell.model, Screen::Diff);
    Next::Go
}

/// A JSON string field, or empty.
fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `5b` — the vector index sheet, from the real `memory-status` payload.
fn vectors_sheet(value: &serde_json::Value) -> VectorIndex {
    let records = value
        .get("records")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let enabled = value
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let automatic = value
        .get("automaticRetrieval")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut health = vec![(
        if enabled {
            State::Done
        } else {
            State::Attention
        },
        if enabled {
            "index enabled".to_string()
        } else {
            "index disabled — memory tools answer empty".to_string()
        },
    )];
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_u64);
    let inserted_this_session = number("lastIndexed").unwrap_or(0);
    if let Some(at) = number("lastIndexedAt") {
        health.push((
            State::Done,
            format!(
                "indexed {inserted_this_session} this session · last {}",
                sheet_time_ago(at)
            ),
        ));
    } else if enabled {
        health.push((State::Queued, "nothing indexed yet this session".into()));
    }
    let embedded = number("embedded").unwrap_or(0);
    let dense_available = value
        .get("denseAvailable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    if records > 0 {
        health.push((
            if embedded == records {
                State::Done
            } else {
                State::Attention
            },
            format!("{embedded} of {records} records carry a vector"),
        ));
    }
    if !dense_available {
        health.push((
            State::Attention,
            "dense retrieval paused — the embedding host did not answer; lexical only for 2m"
                .into(),
        ));
    }
    health.push((
        if automatic {
            State::Done
        } else {
            State::Queued
        },
        if automatic {
            "automatic retrieval before each turn".to_string()
        } else {
            "retrieval on demand only — /memory-search".to_string()
        },
    ));
    let kinds = value
        .get("kinds")
        .and_then(serde_json::Value::as_object)
        .map(|kinds| {
            let mut rows: Vec<(String, u64)> = kinds
                .iter()
                .map(|(name, count)| (name.replace('_', " "), count.as_u64().unwrap_or(0)))
                .collect();
            rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
            rows.into_iter()
                .map(|(name, count)| {
                    let fraction = if records == 0 {
                        0.0
                    } else {
                        count as f64 / records as f64
                    };
                    let note = match name.as_str() {
                        "task" => "what was asked",
                        "decision" => "what was concluded",
                        "constraint" => "never evicted",
                        "compaction" => "one per compaction",
                        "conversation" => "first to go",
                        _ => "promoted · importance 0.9",
                    };
                    (name, sheet_thousands(count), fraction, note.to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let model = json_str(value, "embeddingModel");
    let dims = number("embeddingDimensions").unwrap_or(0);
    let model_dims = if model.is_empty() {
        String::new()
    } else {
        format!("{model} {dims}d")
    };
    let embed_host = json_str(value, "ollama")
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();
    let result_limit = number("resultLimit").unwrap_or(0);
    let retrieval_note = if records == 0 {
        String::new()
    } else if embedded > 0 && dense_available {
        format!(
            "k={result_limit} from {} records · hybrid dense + lexical",
            sheet_thousands(records)
        )
    } else {
        format!(
            "k={result_limit} from {} records · lexical only",
            sheet_thousands(records)
        )
    };
    VectorIndex {
        retrieval_mode: if !enabled {
            "off"
        } else if automatic {
            "automatic"
        } else {
            "on demand"
        }
        .into(),
        repo: json_str(value, "repoId").chars().take(12).collect(),
        repo_records: sheet_thousands(records),
        total_records: sheet_thousands(records),
        injection_cap: number("maxInjectedTokens")
            .map(|tokens| format!("{} tokens", sheet_compact(tokens)))
            .unwrap_or_default(),
        floor: value
            .get("minimumScore")
            .and_then(serde_json::Value::as_f64)
            .map(|score| format!("{score:.2}"))
            .unwrap_or_default(),
        kinds,
        embeddings: "ollama".into(),
        embed_host: if model_dims.is_empty() {
            embed_host
        } else {
            format!("{embed_host} · {model_dims}")
        },
        store: "qdrant".into(),
        collection: json_str(value, "collection"),
        extraction: json_str(value, "extractionModel"),
        config: crate::default_agent_dir()
            .join("vector-memory.json")
            .display()
            .to_string(),
        health,
        model_dims,
        retrieval_note,
        ..Default::default()
    }
}

/// `1,482` — design.md §9: a count carries its magnitude at a glance.
fn sheet_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `1.5k`, `96.2k`, `2.8M` — the short form the counters use.
fn sheet_compact(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// `4m ago`, `just now`, `2h ago` for a unix-millisecond stamp.
fn sheet_time_ago(at_ms: u64) -> String {
    let now = davinci_session::now_ms();
    let elapsed = now.saturating_sub(at_ms) / 1_000;
    if elapsed < 60 {
        "just now".into()
    } else if elapsed < 3_600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{}h ago", elapsed / 3_600)
    } else {
        format!("{}d ago", elapsed / 86_400)
    }
}

/// `5c` — the governor's ledger, from the real `governor-status` payload plus
/// a look into its store directory.
fn governor_sheet(value: &serde_json::Value) -> GovernorSheet {
    let count_in = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let count = |key: &str| count_in(value, key);
    let store_dir = json_str(value, "store");
    let size_label = |bytes: u64| {
        if bytes >= 1_000 {
            format!("{} KB", bytes / 1_000)
        } else {
            format!("{bytes} B")
        }
    };
    // The governor's own manifest names the tool and call behind each id;
    // the directory listing only covers ids left by an earlier process.
    let manifest: Vec<GovernorStored> = value
        .get("stored")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(8)
                .map(|entry| GovernorStored {
                    id: json_str(entry, "id"),
                    tool: json_str(entry, "tool"),
                    call: json_str(entry, "call"),
                    size: format!(
                        "{} ln · {}",
                        sheet_thousands(count_in(entry, "lines")),
                        size_label(count_in(entry, "bytes"))
                    ),
                    stale: false,
                })
                .collect()
        })
        .unwrap_or_default();
    let stored: Vec<GovernorStored> = if manifest.is_empty() {
        std::fs::read_dir(&store_dir)
            .into_iter()
            .flatten()
            .flatten()
            .take(8)
            .map(|entry| GovernorStored {
                id: entry
                    .file_name()
                    .to_string_lossy()
                    .trim_end_matches(".txt")
                    .to_string(),
                tool: String::new(),
                call: "from an earlier process".into(),
                size: entry
                    .metadata()
                    .map(|meta| size_label(meta.len()))
                    .unwrap_or_default(),
                stale: true,
            })
            .collect()
    } else {
        manifest
    };
    let (on_disk, on_disk_bytes) = std::fs::read_dir(&store_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .fold((0u64, 0u64), |(n, bytes), meta| (n + 1, bytes + meta.len()));
    let outputs_note = if on_disk == 0 {
        String::new()
    } else {
        format!(
            "{on_disk} outputs · {} · {} newest shown · swept 14 days after the session's last write",
            size_label(on_disk_bytes),
            stored.len()
        )
    };
    let tokens_withheld = count("bytesWithheld") / 4;
    let policy = value
        .get("thresholds")
        .map(|thresholds| {
            let t = |key: &str| count_in(thresholds, key);
            format!(
                "compresses above {} KB or {} lines · keeps {} head, {} tail, {} lines it judges important",
                t("compressBytes") / 1024,
                t("compressLines"),
                t("keepHeadLines"),
                t("keepTailLines"),
                t("maxImportantLines")
            )
        })
        .unwrap_or_default();
    GovernorSheet {
        enabled: value.get("enabled").and_then(serde_json::Value::as_bool),
        session_id: json_str(value, "sessionKey"),
        counters: vec![
            GovernorCounter {
                number: count("compressedOutputs").to_string(),
                of: "results".into(),
                verb: "compressed".into(),
                note: "head and tail kept · rest on disk".into(),
                tone: Tone::Primary,
            },
            GovernorCounter {
                number: count("deduplicatedReads").to_string(),
                of: "reads".into(),
                verb: "deduplicated".into(),
                note: "same file, same state hash".into(),
                tone: Tone::Secondary,
            },
            GovernorCounter {
                number: count("blockedCalls").to_string(),
                of: "calls".into(),
                verb: "blocked".into(),
                note: "anti-loop · no new state".into(),
                tone: Tone::Warning,
            },
            GovernorCounter {
                number: sheet_compact(tokens_withheld),
                of: format!("of {} calls", count("toolCalls")),
                verb: "est. tokens saved".into(),
                note: "approximation: withheld bytes / 4".into(),
                tone: Tone::Success,
            },
        ],
        stored,
        store_dir,
        outputs_note,
        policy,
        ..Default::default()
    }
}

/// `5d` — the security scan sheet, from the structured `sec-status` payload.
fn security_sheet(value: &serde_json::Value) -> SecurityScan {
    if value["schemaVersion"] == 2 {
        let empty = Vec::new();
        let raw = value["findings"].as_array().unwrap_or(&empty);
        let findings = raw
            .iter()
            .map(|finding| {
                let assessment = &finding["assessment"];
                let location = &finding["claim"]["locations"][0];
                let count = finding["occurrences"].as_array().map_or(1, Vec::len);
                Finding {
                    message: format!(
                        "{}{}: {}{}",
                        if finding["scope"] == "supporting" {
                            "Outside target · "
                        } else {
                            ""
                        },
                        assessment["classification"]
                            .as_str()
                            .unwrap_or_else(|| assessment["disposition"]
                                .as_str()
                                .unwrap_or("deferred")),
                        json_str(&finding["claim"], "title"),
                        if count > 1 {
                            format!(" ({count} occurrences)")
                        } else {
                            String::new()
                        }
                    ),
                    location: format!("{}:{}", json_str(location, "path"), location["startLine"]),
                    severity: match assessment["severity"].as_str() {
                        Some("critical") => Severity::Critical,
                        Some("high") => Severity::High,
                        Some("medium") => Severity::Medium,
                        Some("low") => Severity::Low,
                        _ => Severity::Dismissed,
                    },
                    rule: "native-security-review".into(),
                    evidence:
                        crate::native_extensions::security_scan::report::render_finding_details(
                            finding,
                        ),
                    path: String::new(),
                }
            })
            .collect();
        let reviewed = value["coverage"]["reviewedPaths"]
            .as_array()
            .map_or(0, Vec::len);
        let eligible = value["coverage"]["eligibleFiles"].as_u64().unwrap_or(0);
        let status = value["status"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown");
        return SecurityScan {
            validated: raw
                .iter()
                .filter(|finding| finding["assessment"]["classification"] == "confirmed")
                .count() as u32,
            candidates: raw.len() as u32,
            fraction: if eligible == 0 {
                0.0
            } else {
                reviewed as f64 / eligible as f64
            },
            files: format!("{reviewed}/{eligible}"),
            skipped: value["coverage"]["skipped"]
                .as_array()
                .map_or(0, Vec::len)
                .to_string(),
            findings,
            seal: String::new(),
            id: json_str(value, "scanId"),
            state: status.to_string(),
            report: format!(
                "Experimental · {} · coverage {} · scan {} · /sec-status /sec-report /sec-abort",
                status,
                if value["coverageComplete"] == true {
                    "complete"
                } else {
                    "incomplete"
                },
                json_str(value, "scanId")
            ),
            ..Default::default()
        };
    }
    let severity_of = |name: &str| match name {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Dismissed,
    };
    let empty = Vec::new();
    let raw_findings = value
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    let findings: Vec<Finding> = raw_findings
        .iter()
        .map(|finding| {
            let severity = if finding
                .get("falsePositive")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                Severity::Dismissed
            } else {
                severity_of(&json_str(finding, "severity"))
            };
            Finding {
                message: json_str(finding, "message"),
                location: format!(
                    "{}:{}",
                    json_str(finding, "file"),
                    finding
                        .get("line")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                ),
                severity,
                rule: json_str(finding, "ruleId"),
                evidence: json_str(finding, "evidence"),
                path: String::new(),
            }
        })
        .collect();
    let validated = raw_findings
        .iter()
        .filter(|finding| {
            finding
                .get("validated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .count() as u32;
    let dismissed = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Dismissed)
        .count() as u32;
    let candidates = value
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .map(|list| list.len() as u32)
        .unwrap_or(0);
    let mut severities: Vec<(String, u32, Severity)> = Vec::new();
    for (name, severity) in [
        ("critical", Severity::Critical),
        ("high", Severity::High),
        ("medium", Severity::Medium),
        ("low", Severity::Low),
        ("dismissed", Severity::Dismissed),
    ] {
        let count = findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count() as u32;
        if count > 0 {
            severities.push((name.to_string(), count, severity));
        }
    }
    let coverage = value.get("coverage").cloned().unwrap_or_default();
    let read_only = value["readOnly"] == true || value["status"] == "legacy-readonly";
    SecurityScan {
        validated,
        candidates: candidates.max(findings.len() as u32),
        fraction: if value["coverageComplete"] == true {
            if candidates == 0 {
                1.0
            } else {
                validated as f64 / candidates as f64
            }
        } else {
            0.0
        },
        files: coverage
            .get("filesScanned")
            .and_then(serde_json::Value::as_u64)
            .map(|count| count.to_string())
            .unwrap_or_default(),
        skipped: coverage
            .get("filesSkipped")
            .and_then(serde_json::Value::as_u64)
            .map(|count| count.to_string())
            .unwrap_or_default(),
        bytes: String::new(),
        severities,
        dismissed,
        findings,
        seal: json_str(
            value.get("manifest").unwrap_or(&serde_json::Value::Null),
            "scanId",
        ),
        id: json_str(value, "scanId"),
        state: if read_only {
            "legacy-readonly".into()
        } else {
            json_str(value, "status")
        },
        report: if read_only {
            "Legacy v1 · read-only · not v2 confirmation · coverage incomplete".into()
        } else {
            "report.md in the scan artifact · /sec-report".into()
        },
        ..Default::default()
    }
}

/// `5a` — the graph run sheet, from the real `graph-status` payload
/// (`native_extensions::graph::mod::status`: `run` is the persisted
/// `GraphRun`, camelCase).
fn graph_sheet(value: &serde_json::Value) -> Option<GraphRunSheet> {
    let run = value.get("run").filter(|run| !run.is_null())?;
    let empty = Vec::new();
    let tasks_json = run
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    let phase = json_str(run, "phase");
    let stopped = matches!(phase.as_str(), "blocked" | "cancelled");
    let number = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let now = davinci_session::now_ms();

    // The phase ribbon. A stopped run marks the phase it stopped in as
    // failed; the phase order is the controller's.
    const ORDER: [&str; 7] = [
        "classify",
        "investigate",
        "plan",
        "implement",
        "verify",
        "review",
        "done",
    ];
    let phase_of_role = |role: &str| match role {
        "classifier" => 0,
        "researcher" | "test-analyzer" | "historian" => 1,
        "planner" => 2,
        "writer" => 3,
        "reviewer" => 5,
        _ => 0,
    };
    let verification_failed = run
        .get("verification")
        .and_then(|verification| verification.get("passed"))
        .and_then(serde_json::Value::as_bool)
        == Some(false);
    let at = if phase == "done" {
        6
    } else if let Some(index) = ORDER.iter().position(|name| *name == phase) {
        index
    } else if verification_failed {
        4
    } else {
        tasks_json
            .last()
            .map(|task| phase_of_role(&json_str(task, "role")))
            .unwrap_or(0)
    };
    let phases: Vec<(String, State)> = ORDER
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let state = match index.cmp(&at) {
                std::cmp::Ordering::Less => State::Done,
                std::cmp::Ordering::Equal if phase == "done" => State::Done,
                std::cmp::Ordering::Equal if stopped => State::Failed,
                std::cmp::Ordering::Equal => State::Active,
                std::cmp::Ordering::Greater => State::Queued,
            };
            (name.to_string(), state)
        })
        .collect();

    // Presentation-only dependency propagation. Do not change the persisted
    // status or infer blockage from a missing dependency.
    let mut unavailable = std::collections::BTreeSet::new();
    let mut waiting = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    let mut frontier = std::collections::VecDeque::new();
    for task in tasks_json {
        let Some(id) = task.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        match task.get("status").and_then(serde_json::Value::as_str) {
            Some("failed" | "cancelled") => {
                if unavailable.insert(id) {
                    frontier.push_back(id);
                }
            }
            Some("pending" | "ready") => {
                if let Some(deps) = task.get("dependsOn").and_then(serde_json::Value::as_array) {
                    for dep in deps.iter().filter_map(serde_json::Value::as_str) {
                        waiting.entry(dep).or_default().push(id);
                    }
                }
            }
            _ => {}
        }
    }
    while let Some(id) = frontier.pop_front() {
        for &dependent in waiting.get(id).into_iter().flatten() {
            if unavailable.insert(dependent) {
                frontier.push_back(dependent);
            }
        }
    }

    // The rows: one per worker the run has spawned so far.
    let policy_of_role = |role: &str| match role {
        "test-analyzer" | "reviewer" => "read-and-test",
        "writer" => "write-no-git-mutation",
        _ => "read-only",
    };
    let clip = |text: &str, max: usize| {
        let text = text.lines().next().unwrap_or_default().trim();
        if text.chars().count() > max {
            format!(
                "{}…",
                text.chars().take(max.saturating_sub(1)).collect::<String>()
            )
        } else {
            text.to_string()
        }
    };
    let classification = run.get("classification");
    let live = !stopped && phase != "done";
    let mode = json_str(run, "forced");
    let mode = if mode.is_empty() {
        classification
            .map(|c| json_str(c, "complexity"))
            .unwrap_or_default()
    } else {
        mode
    };
    // A trivial (or forced simple) run goes classify → implement → verify:
    // it never investigates, plans or reviews, so those are not drawn as
    // pending. A finished run shows only what ran.
    let full_pipeline = !matches!(mode.as_str(), "trivial" | "simple");
    let tasks: Vec<GraphTask> = tasks_json
        .iter()
        .map(|task| {
            let id = json_str(task, "id");
            let role = json_str(task, "role");
            let status = json_str(task, "status");
            let expect = json_str(task, "expect");
            let focus = json_str(task, "focus");
            let deps = task
                .get("dependsOn")
                .and_then(serde_json::Value::as_array)
                .map(|deps| {
                    deps.iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let blocked =
                matches!(status.as_str(), "pending" | "ready") && unavailable.contains(id.as_str());
            let state = match status.as_str() {
                "succeeded" => State::Done,
                "running" => State::Active,
                "failed" | "cancelled" => State::Failed,
                _ if blocked => State::Attention,
                _ => State::Queued,
            };
            let artifact = match status.as_str() {
                "succeeded" => match expect.as_str() {
                    "classification" => classification
                        .map(|c| {
                            format!(
                                "{} · {}",
                                json_str(c, "taskClass"),
                                json_str(c, "complexity")
                            )
                        })
                        .unwrap_or_else(|| "classification".into()),
                    _ if !focus.is_empty() => format!("{expect} · {}", clip(&focus, 40)),
                    _ => expect.clone(),
                },
                "running" => {
                    let activity = json_str(task, "lastActivity");
                    if activity.is_empty() {
                        format!("{expect} · working")
                    } else {
                        activity
                    }
                }
                "failed" => format!("failed · {}", clip(&json_str(task, "error"), 44)),
                "cancelled" => "cancelled".into(),
                _ if blocked => format!("blocked · dependency unavailable: {deps}"),
                "ready" => "ready".into(),
                _ if !deps.is_empty() => format!("pending · waits on {deps}"),
                _ => "pending".into(),
            };
            let usage = task.get("usage");
            let input = usage.map(|u| number(u, "input")).unwrap_or(0);
            let output = usage.map(|u| number(u, "output")).unwrap_or(0);
            let cost = usage
                .and_then(|u| u.get("costUsd"))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let started = number(task, "startedAt");
            let ended = number(task, "endedAt");
            let time = if started == 0 {
                String::new()
            } else if ended >= started && ended > 0 {
                sheet_duration(ended - started)
            } else {
                sheet_duration(now.saturating_sub(started))
            };
            let usage = if started == 0 && input == 0 {
                "—".into()
            } else {
                format!(
                    "{}↑ {}↓ ${cost:.2} {time}",
                    sheet_compact(input),
                    sheet_compact(output)
                )
                .trim_end()
                .to_string()
            };
            let owner = json_str(task, "owner");
            let attempts = number(task, "attempts") as u32;
            let error = task
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(String::from);
            let public_contract = task
                .get("publicContract")
                .and_then(serde_json::Value::as_str)
                .map(String::from);
            let recent_tools = task
                .get("recentTools")
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let dependencies: Vec<String> = task
                .get("dependsOn")
                .and_then(serde_json::Value::as_array)
                .map(|deps| {
                    deps.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            GraphTask {
                title: graph_public_text(&focus),
                branch: task
                    .get("branch")
                    .and_then(serde_json::Value::as_str)
                    .map(graph_public_text),
                worktree: task
                    .get("worktree")
                    .and_then(serde_json::Value::as_str)
                    .map(graph_public_text),
                id,
                policy: policy_of_role(&role).into(),
                artifact: graph_public_text(&artifact),
                usage,
                state,
                dependencies,
                owner,
                attempts,
                error,
                recent_tools,
                public_contract,
                phase: match role.as_str() {
                    "classifier" => "classify",
                    "researcher" | "test-analyzer" | "historian" => "investigate",
                    "planner" => "plan",
                    "writer" => "implement",
                    "reviewer" => "review",
                    _ => "",
                }
                .into(),
                role,
                status,
                artifact_file: task
                    .get("artifactFile")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from),
            }
        })
        .collect();

    // The worker graph: classify fans out to the research workers, which
    // join into plan → implement → review. Phases the run has not reached
    // are drawn as pending nodes so the shape reads whole from the start.
    let glyph = |state: State| match state {
        State::Done => "✓",
        State::Active => "◉",
        State::Failed => "×",
        _ => "○",
    };
    let node = |task: &GraphTask| format!("{} {}", task.id, glyph(task.state));
    let latest = |prefix: &str| {
        tasks
            .iter()
            .filter(|task| task.id == prefix || task.id.starts_with(&format!("{prefix}-")))
            .last()
            .map(node)
            .unwrap_or_else(|| format!("{prefix} ○"))
    };
    let research: Vec<String> = tasks
        .iter()
        .filter(|task| task.id.starts_with("research-"))
        .map(node)
        .collect();
    let has = |prefix: &str| {
        tasks
            .iter()
            .any(|task| task.id == prefix || task.id.starts_with(&format!("{prefix}-")))
    };
    let mut chain: Vec<String> = Vec::new();
    if has("plan") || (live && full_pipeline) {
        chain.push(latest("plan"));
    }
    if has("implement") || live {
        chain.push(latest("implement"));
    }
    if has("review") || (live && full_pipeline) {
        chain.push(latest("review"));
    }
    let tail = chain
        .iter()
        .map(|node| format!("─ {node}"))
        .collect::<Vec<_>>()
        .join(" ");
    let head = latest("classify");
    let shape: Vec<String> = if research.is_empty() {
        if live && full_pipeline {
            vec![format!("{head} ─ investigate ○ {tail}")]
        } else if tail.is_empty() {
            vec![head]
        } else {
            vec![format!("{head} {tail}")]
        }
    } else if research.len() == 1 {
        vec![format!("{head} ─ {} {tail}", research[0])]
    } else {
        let width = research
            .iter()
            .map(|r| r.chars().count())
            .max()
            .unwrap_or(0);
        let pad = " ".repeat(head.chars().count() + 1);
        let last = research.len() - 1;
        let join_row = research.len() / 2;
        research
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let (left, right) = match index {
                    0 => (format!("{head} ─┬─"), "─┐"),
                    i if i == last => (format!("{pad}└─"), "─┘"),
                    _ => (format!("{pad}├─"), "─┤"),
                };
                let right = if index == join_row {
                    if index == 0 {
                        "─┬"
                    } else if index == last {
                        "─┴"
                    } else {
                        "─┼"
                    }
                } else {
                    right
                };
                let mut row = format!("{left} {name:<width$} {right}");
                if index == join_row {
                    row.push_str(&tail);
                }
                row
            })
            .collect()
    };

    let counters = run.get("counters").cloned().unwrap_or_default();
    let budgets = run.get("budgets").cloned().unwrap_or_default();
    let cost = counters
        .get("costUsd")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let cost_cap = budgets
        .get("maxCostUsd")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let capped = |used: u64, cap: u64| {
        if cap == 0 {
            format!("{used} of no cap")
        } else {
            format!("{used} of {cap}")
        }
    };
    let started_at = number(&counters, "startedAt");
    let elapsed_until = if live {
        now
    } else {
        number(run, "updatedAt").max(started_at)
    };
    let milestone = match (
        run.get("currentMilestone")
            .and_then(serde_json::Value::as_u64),
        run.get("milestones")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
    ) {
        (Some(current), Some(total)) if total > 1 => format!("{current} of {total}"),
        _ => String::new(),
    };
    Some(GraphRunSheet {
        goal: graph_public_text(&json_str(run, "goal")),
        phases,
        shape,
        tasks,
        cost: format!("${cost:.2}"),
        cost_cap: if cost_cap > 0.0 {
            format!("${cost_cap:.2}")
        } else {
            "no cap".into()
        },
        cost_fraction: if cost_cap > 0.0 {
            (cost / cost_cap).clamp(0.0, 1.0)
        } else {
            0.0
        },
        workers: capped(
            number(&counters, "workersSpawned"),
            number(&budgets, "maxWorkers"),
        ),
        parallel: number(&budgets, "maxParallelWorkers").to_string(),
        cycles: format!(
            "{} total (limit {}/milestone)",
            number(&counters, "revisionCycles"),
            number(&budgets, "maxRevisionCycles"),
        ),
        replans: capped(number(&counters, "replans"), number(&budgets, "maxReplans")),
        artifacts: match (
            run.get("cwd").and_then(serde_json::Value::as_str),
            run.get("runId").and_then(serde_json::Value::as_str),
        ) {
            (Some(cwd), Some(id)) => {
                crate::native_extensions::graph::store::run_dir(std::path::Path::new(cwd), id)
                    .display()
                    .to_string()
            }
            _ => String::new(),
        },
        id: json_str(run, "runId"),
        mode,
        milestone,
        elapsed: if started_at == 0 {
            String::new()
        } else {
            sheet_duration(elapsed_until.saturating_sub(started_at))
        },
        ecosystem: run
            .get("ecosystemStats")
            .and_then(|v| {
                serde_json::from_value::<crate::native_extensions::ecosystem::EcosystemStats>(
                    v.clone(),
                )
                .ok()
            })
            .map(|stats| stats.render_compact_lines())
            .unwrap_or_default(),
        lifecycle: if matches!(phase.as_str(), "done" | "blocked" | "cancelled") {
            "stopped"
        } else {
            run.get("lifecycle")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("running")
        }
        .into(),
        phase,
        blocked_reason: run
            .get("blockedReason")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
        verification: graph_verification_facts(run.get("verification")),
        control_status: run
            .get("controlStatus")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
        ..Default::default()
    })
}

fn graph_verification_facts(verification: Option<&serde_json::Value>) -> Vec<String> {
    let Some(verification) = verification else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    if let Some(progress) = verification
        .get("progress")
        .filter(|value| value.is_object())
    {
        let index = progress
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let total = progress
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let started = progress
            .get("startedAt")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let elapsed =
            crate::native_extensions::graph::store::now_ms().saturating_sub(started) / 1000;
        facts.push(format!(
            "Verification running · {index}/{total} · {elapsed}s elapsed"
        ));
        facts.push(format!(
            "Running: {} · {}",
            json_str(progress, "name"),
            json_str(progress, "command")
        ));
    } else if let Some(passed) = verification
        .get("passed")
        .and_then(serde_json::Value::as_bool)
    {
        facts.push(format!(
            "Verification {}",
            if passed { "passed" } else { "failed" }
        ));
    }
    if let Some(commands) = verification
        .get("commands")
        .and_then(serde_json::Value::as_array)
    {
        for command in commands {
            let mut parts = vec![json_str(command, "name"), json_str(command, "command")];
            if command.get("skipped").and_then(serde_json::Value::as_bool) == Some(true) {
                parts.push("skipped".into());
            } else if let Some(exit) = command.get("exitCode").and_then(serde_json::Value::as_i64) {
                parts.push(format!("exit {exit}"));
            }
            if let Some(ms) = command
                .get("durationMs")
                .and_then(serde_json::Value::as_u64)
            {
                parts.push(format!("{:.1}s", ms as f64 / 1000.0));
            }
            let fact = parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            if !fact.is_empty() {
                facts.push(fact);
            }
        }
    }
    facts
}

#[cfg(test)]
mod graph_canvas_fact_tests {
    use super::*;
    use crate::native_extensions::graph::types::*;
    use serde_json::json;

    #[test]
    fn verification_progress_does_not_report_a_running_attempt_as_failed() {
        let value = json!({"passed": false, "commands": [], "progress": {
            "name": "test", "command": "cargo test --workspace",
            "index": 3, "total": 4, "startedAt": 0
        }});
        let facts = graph_verification_facts(Some(&value)).join("\n");
        assert!(facts.contains("Verification running"), "{facts}");
        assert!(facts.contains("3/4"), "{facts}");
        assert!(facts.contains("cargo test --workspace"), "{facts}");
        assert!(!facts.contains("Verification failed"), "{facts}");
    }

    #[test]
    fn graph_activity_is_bounded_and_matches_the_selected_run_and_worker() {
        let mut sheet = graph_sheet(&json!({"run": snapshot()})).unwrap();
        let view = json!({"runId": sheet.id, "taskId": "writer", "transcript": [
            "read src/main.rs", "thinking: PRIVATE_SENTINEL", "cargo test complete"
        ]});
        apply_graph_activity(&mut sheet, "review", &view);
        assert!(sheet.tasks[1].recent_tools.is_empty());
        apply_graph_activity(&mut sheet, "writer", &view);
        assert_eq!(sheet.tasks[0].recent_tools.len(), 3);
        assert!(!sheet.tasks[0]
            .recent_tools
            .join(" ")
            .contains("PRIVATE_SENTINEL"));
        assert!(sheet.tasks[0].recent_tools[2].contains("cargo test"));
        let previous = sheet.tasks[0].recent_tools.clone();
        let mut wrong = view.clone();
        wrong["runId"] = json!("different-run");
        apply_graph_activity(&mut sheet, "writer", &wrong);
        assert_eq!(sheet.tasks[0].recent_tools, previous);
    }

    #[test]
    fn graph_goal_and_activity_redact_credentials_before_display() {
        let mut run = snapshot();
        run.goal = "Fix login using apikey_fixture_not_a_real_key and Bearer fake-token".into();
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert!(sheet.goal.contains("Fix login"));
        assert!(!sheet.goal.contains("fixture_not_a_real_key"));
        assert!(!sheet.goal.contains("fake-token"));
        assert_eq!(
            graph_public_text("password=fixture-password"),
            "password=[REDACTED]"
        );
    }

    fn snapshot() -> GraphRun {
        let mut run: GraphRun = serde_json::from_value(json!({
            "version": 1, "runId": "canvas-facts", "goal": "fixture", "cwd": ".",
            "phase": "blocked", "dryRun": true, "budgets": GraphBudgets::default(),
            "counters": {"workersSpawned": 2, "revisionCycles": 0, "replans": 0, "costUsd": 0.2, "startedAt": 1000},
            "updatedAt": 2000
        })).unwrap();
        let mut writer = GraphTaskState::new(
            "writer",
            Role::Writer,
            ArtifactKind::PatchReport,
            vec![],
            None,
        );
        writer.status = TaskStatus::Failed;
        writer.attempts = 2;
        writer.artifact_file = Some("artifacts/writer.json".into());
        writer.error = Some("assertion failed".into());
        writer.started_at = Some(1000);
        writer.ended_at = Some(2000);
        writer.usage.input = 1200;
        writer.usage.output = 300;
        writer.usage.cost_usd = 0.2;
        writer.context_fingerprint = Some("PRIVATE_CONTEXT_SENTINEL".into());
        run.tasks = vec![
            writer,
            GraphTaskState::new(
                "review",
                Role::Reviewer,
                ArtifactKind::Review,
                vec!["writer".into()],
                None,
            ),
        ];
        run.blocked_reason = Some("required checks failed".into());
        run.verification = Some(VerificationResult {
            progress: None,
            passed: false,
            commands: vec![VerificationCommandResult {
                name: "tests".into(),
                command: "cargo test".into(),
                exit_code: 1,
                duration_ms: 1200,
                output_tail: "PRIVATE_OUTPUT_SENTINEL".into(),
                skipped: false,
            }],
        });
        run
    }

    #[test]
    fn graph_canvas_facts_match_typed_snapshot_without_private_payloads() {
        let run = snapshot();
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.tasks[0].artifact_file, run.tasks[0].artifact_file);
        assert_eq!(sheet.tasks[0].attempts, run.tasks[0].attempts);
        assert_eq!(sheet.tasks[0].status, "failed");
        assert_eq!(sheet.tasks[0].phase, "implement");
        assert_eq!(sheet.tasks[0].error, run.tasks[0].error);
        assert_eq!(sheet.tasks[1].dependencies, run.tasks[1].depends_on);
        assert_eq!(sheet.tasks[1].state, State::Attention);
        assert!(sheet.tasks[1].artifact.contains("writer"));
        assert!(sheet.tasks[0].usage.contains("1.2k↑ 300↓ $0.20 1s"));
        assert_eq!(sheet.blocked_reason, run.blocked_reason);
        assert_eq!(sheet.phase, "blocked");
        assert_eq!(
            sheet.artifacts,
            crate::native_extensions::graph::store::run_dir(
                std::path::Path::new(&run.cwd),
                &run.run_id
            )
            .display()
            .to_string()
        );
        assert_eq!(sheet.lifecycle, "stopped");
        assert!(sheet
            .verification
            .iter()
            .any(|v| v.contains("cargo test") && v.contains("exit 1") && v.contains("1.2s")));
        assert!(sheet.verification.iter().any(|v| v.contains("failed")));
        assert!(sheet.tasks[0].owner.is_empty() && sheet.tasks[0].recent_tools.is_empty());
        assert!(sheet.tasks[0].public_contract.is_none());
        assert!(!format!("{sheet:?}").contains("PRIVATE_"));
    }

    #[test]
    fn graph_revision_counter_distinguishes_lifetime_total_from_delivery_limit() {
        let mut run = snapshot();
        run.counters.revision_cycles = 4;
        run.budgets.max_revision_cycles = 3;
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.cycles, "4 total (limit 3/milestone)");
        run.budgets.max_revision_cycles = 0;
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.cycles, "4 total (limit 0/milestone)");
    }

    #[test]
    fn graph_canvas_facts_keep_cancelled_ready_legacy_lifecycle_and_unknowns_honest() {
        let mut run = snapshot();
        run.phase = Phase::Done;
        run.tasks[0].status = TaskStatus::Cancelled;
        run.tasks[1].status = TaskStatus::Ready;
        run.tasks[1].depends_on.clear();
        run.verification = None;
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.lifecycle, "stopped");
        assert_eq!(sheet.tasks[0].status, "cancelled");
        assert_eq!(sheet.tasks[1].status, "ready");
        assert!(sheet.verification.is_empty());
        assert!(sheet.tasks[1].artifact_file.is_none());
    }

    #[test]
    fn graph_canvas_facts_propagate_only_declared_failed_dependencies() {
        let mut run = snapshot();
        run.tasks.push(GraphTaskState::new(
            "dependent",
            Role::Reviewer,
            ArtifactKind::Review,
            vec!["review".into()],
            None,
        ));
        run.tasks.push(GraphTaskState::new(
            "missing",
            Role::Reviewer,
            ArtifactKind::Review,
            vec!["absent".into()],
            None,
        ));
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.tasks[2].state, State::Attention);
        assert_eq!(sheet.tasks[2].status, "pending");
        assert_eq!(sheet.tasks[3].state, State::Queued);
        let mut run = snapshot();
        run.tasks[0].status = TaskStatus::Succeeded;
        let sheet = graph_sheet(&json!({"run": run})).unwrap();
        assert_eq!(sheet.tasks[1].state, State::Queued);
    }
}

/// `4s`, `1m52s`, `1h03m` for a span in milliseconds.
fn sheet_duration(ms: u64) -> String {
    let seconds = ms / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{:02}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

/// Re-read `graph-status` and rebuild the `5a` sheet from it. `false` when
/// there is no run to show.
fn refresh_graph_sheet(model: &mut Model, host: &Arc<Mutex<ExtensionHost>>) -> bool {
    let status = host
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .execute_native_command("graph-status", "");
    match status {
        Ok(Some(value)) => match graph_sheet(&value) {
            Some(sheet) => {
                graph_feedback::refresh(model, sheet);
                if let Some(selected) = model
                    .graph_run
                    .as_ref()
                    .and_then(|run| run.selected_node_id.clone())
                {
                    let view = host
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .execute_native_command("graph-view", &selected);
                    if let Ok(Some(value)) = view {
                        if let Some(run) = model.graph_run.as_mut() {
                            apply_graph_activity(run, &selected, &value);
                        }
                    }
                }
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Never attach a transcript from a different run or a fallback worker.
fn apply_graph_activity(run: &mut GraphRunSheet, selected: &str, value: &serde_json::Value) {
    if value.get("runId").and_then(serde_json::Value::as_str) != Some(run.id.as_str())
        || value.get("taskId").and_then(serde_json::Value::as_str) != Some(selected)
    {
        return;
    }
    if let Some(task) = run.tasks.iter_mut().find(|task| task.id == selected) {
        task.recent_tools = value
            .get("transcript")
            .and_then(serde_json::Value::as_array)
            .map(|rows| {
                rows.iter()
                    .rev()
                    .take(40)
                    .rev()
                    .filter_map(serde_json::Value::as_str)
                    .map(graph_public_text)
                    .collect()
            })
            .unwrap_or_default();
    }
}

fn graph_public_text(value: &str) -> String {
    static DIRECT_KEY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let direct_key =
        DIRECT_KEY.get_or_init(|| regex::Regex::new(r"apikey_[A-Za-z0-9_-]+").unwrap());
    let value = direct_key.replace_all(value, "[REDACTED]");
    let value = crate::native_extensions::vector_memory::redact_secrets(&value);
    davinci_tui::davinci::views::graph_inspector::public_text(&value)
}

/// Everything a composer line or a chosen row may need. Bundled because the
/// borrow checker will not let the loop hand out eight `&mut` pieces at once.
struct Shell<'a> {
    voice: &'a mut crate::voice_input::VoiceInput,
    parsed: &'a crate::args::Args,
    agent: &'a mut Agent,
    model: &'a mut Model,
    terminal: &'a mut davinci_tui::davinci::runtime::Session,
    host: &'a Arc<Mutex<ExtensionHost>>,
    pending: &'a mut Option<Question>,
    cwd: &'a std::path::Path,
    dresser: &'a crate::davinci_sources::WorkspaceDresser,
    /// Images pasted from the clipboard, waiting for the next prompt.
    images: &'a mut Vec<davinci_ai::MessageContent>,
}

/// What the loop should do after handling one key's worth of consequence.
enum Next {
    Go,
    Leave,
    Fail(String),
}

impl Shell<'_> {
    /// A block of prose in the transcript, preceded by the one blank row that
    /// separates blocks (design.md §3).
    fn say(&mut self, text: &str) {
        self.model.running = false;
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.model.transcript.push(Entry::Gap);
        for paragraph in text.split("\n\n") {
            self.model.transcript.push(Entry::prose(paragraph.trim()));
        }
    }

    /// A line that needs the attention glyph. Colour is never the only signal
    /// (design.md §4), so this reads as a warning under `NO_COLOR` too.
    fn note(&mut self, text: &str) {
        self.model.running = false;
        attach_section_notice(self.model, text);
        self.model.transcript.push(Entry::Gap);
        self.model
            .transcript
            .push(Entry::tool(State::Attention, "instrumenta", text, None));
    }

    /// Re-read everything the workspace owns: git, the tree, the session
    /// list. The git and filesystem half runs on the dresser's thread — four
    /// subprocesses and a tree walk have no place on the key-handling path —
    /// and lands on the model when the loop next looks.
    fn redress(&mut self) {
        refresh_context(self.model, self.agent);
        self.dresser.request();
        crate::davinci_surfaces::dress_from_extensions(self.model, self.cwd, self.agent);
        let items = corpus(self.agent, &self.model.slash_commands, &self.model.sessions);
        self.model.corpus_total = items.len();
        self.model.corpus = items;
        // Shortcuts an extension registered since the last look — a reload
        // may have added or removed one.
        let host = self.host.lock().unwrap_or_else(|err| err.into_inner());
        let (shortcuts, _) = host.resolve_shortcuts(&self.model.keybindings);
        drop(host);
        self.model.extension_shortcuts = shortcuts;
    }

    /// Run the handler behind an extension's registered shortcut, then apply
    /// everything it queued. Mirrors `host_invoke_shortcut` in main.rs.
    fn run_shortcut(&mut self, key: &str, path: &str) -> Next {
        let outcome = {
            let mut host = self.host.lock().unwrap_or_else(|err| err.into_inner());
            host.runtime_flag_values = crate::flag_values_json(self.parsed);
            if host.js.iter().any(|ext| ext.path == path) {
                host.invoke_shortcut(path, key)
            } else {
                ExtensionHost::default().invoke_shortcut(path, key)
            }
        };
        if let Err(err) = outcome {
            self.note(&format!("shortcut {key}: {err}"));
            return Next::Go;
        }
        apply_host_effects(self)
    }

    fn finish(&mut self, done: Done) -> Next {
        match done {
            Done::Said(text) => self.say(&text),
            Done::Note(text) => self.note(&text),
            Done::Opened => self.model.running = false,
            Done::Ask(question) => {
                self.model.running = false;
                self.model.ask = question.ask(self.agent);
                open_ask_overlay(self.model);
                *self.pending = Some(question);
            }
            Done::Detach(detached) => return self.detach(detached),
        }
        Next::Go
    }

    /// Hand the terminal back, run something that owns a console of its own,
    /// then take it again. A browser handshake prints and prompts; it cannot
    /// do either underneath an alternate screen.
    fn detach(&mut self, detached: Detached) -> Next {
        self.voice.terminal_handoff(self.model);
        self.model.composer_epoch = self.model.composer_epoch.saturating_add(1);
        if let Err(err) = self.terminal.close() {
            return Next::Fail(err.to_string());
        }
        // The screen is the user's again, so a browser handshake may print and
        // prompt on it directly.
        crate::set_hosted_tui_active(false);
        let outcome = match &detached {
            Detached::Login { provider, key } => {
                if provider.is_empty() {
                    Err("usage: /login <provider> [key]".to_string())
                } else {
                    // `stored` is false when the handshake only printed its
                    // URL. Calling that "signed in" is what left the next
                    // request with no credential and no explanation.
                    crate::login_provider_with_wait(provider, key.as_deref(), true)
                        .and_then(|stored| detached_login_message(provider, !stored))
                }
            }
        };
        match davinci_tui::davinci::runtime::Session::open() {
            Ok(session) => *self.terminal = session,
            Err(err) => return Next::Fail(err.to_string()),
        }
        crate::set_hosted_tui_active(true);
        if let Ok((width, height)) = self.terminal.size() {
            self.model.width = width;
            self.model.height = height;
        }
        match outcome {
            Ok(text) => self.say(&text),
            Err(err) => self.note(&err),
        }
        Next::Go
    }

    /// Open a session file and make it the one in hand.
    fn resume(&mut self, path: &str) -> Next {
        if path.is_empty() {
            self.note("that session has no file on disk");
            return Next::Go;
        }
        match davinci_session::JsonlSession::open(std::path::Path::new(path)) {
            Ok(store) => {
                if let Err(error) = self.agent.load_from_session(store) {
                    self.note(&error);
                    return Next::Go;
                }
                self.voice.cancel(self.model);
                self.model.composer_epoch = self.model.composer_epoch.saturating_add(1);
                self.model.transcript = transcript_from(&self.agent.messages);
                self.model.running = false;
                self.redress();
                Next::Go
            }
            Err(err) => {
                self.note(&format!("could not open that session: {err}"));
                Next::Go
            }
        }
    }
}

/// Run `!command` in the shell without a model turn, exactly as the legacy
/// chrome's `SessionAction::RunBash` does: the extension event first (an
/// extension may run it itself), then the built-in bash tool, with the result
/// recorded into the session unless `!!` asked to keep it out of context.
fn run_user_bash(shell: &mut Shell<'_>, line: &str) -> Next {
    use crate::extension_host::ExtensionEvent;

    let trimmed = line.trim_start();
    let Some(stripped) = trimmed.strip_prefix('!') else {
        return Next::Go;
    };
    let exclude_from_context = trimmed.starts_with("!!");
    let command = if exclude_from_context {
        stripped.strip_prefix('!').unwrap_or(stripped).trim()
    } else {
        stripped.trim()
    };
    if command.is_empty() {
        shell.note("usage: !<command> — !! keeps the output out of context");
        return Next::Go;
    }

    {
        let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        host.runtime_flag_values = crate::flag_values_json(shell.parsed);
        host.emit(ExtensionEvent::UserBash {
            command: command.to_string(),
            exclude_from_context,
            cwd: shell.agent.cwd.display().to_string(),
        });
    }
    match apply_host_effects(shell) {
        Next::Go => {}
        other => return other,
    }

    let say_output = |shell: &mut Shell<'_>, output: &str, failed: bool| {
        shell.model.running = false;
        shell.model.transcript.push(Entry::Gap);
        let lines: Vec<&str> = output
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect();
        let summary = if lines.len() == 1 {
            "1 line".to_string()
        } else {
            format!("{} lines", lines.len())
        };
        shell.model.transcript.push(
            Entry::tool(state_of("bash", failed), "manus", &clip(command, 60), None)
                .summarised(&summary),
        );
        let shown = lines.len().min(20);
        for line in &lines[..shown] {
            shell.model.transcript.push(Entry::detail(&clip(line, 100)));
        }
        if lines.len() > shown {
            shell
                .model
                .transcript
                .push(Entry::detail(&format!("… {} more", lines.len() - shown)));
        }
    };

    // An extension may have run the command itself.
    let handled = {
        let host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        host.last_user_bash_result()
    };
    if let Some(result) = handled {
        shell
            .agent
            .record_bash_result(command, &result, exclude_from_context);
        let output = result
            .get("output")
            .or_else(|| result.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        say_output(shell, output, false);
        shell.redress();
        return Next::Go;
    }

    match davinci_agent::execute_tool(
        &shell.agent.cwd,
        "bash",
        &serde_json::json!({ "command": command }),
    ) {
        Ok(result) => {
            let value = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
            shell
                .agent
                .record_bash_result(command, &value, exclude_from_context);
            say_output(shell, &result.content, result.is_error);
        }
        Err(err) => say_output(shell, &err.to_string(), true),
    }
    shell.redress();
    Next::Go
}

/// One composer line, carried out.
fn on_line(shell: &mut Shell<'_>, line: &str) -> Next {
    // Mode changes are host-owned; an extension cannot shadow their safety state.
    if let Some(rest) = line.trim().strip_prefix("/permissions") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return permissions_command(shell, rest.trim());
        }
    }
    if let Some(result) = crate::permissions::handle_mode_command(shell.agent, line) {
        sync_permission_state(shell.agent, shell.model);
        match result {
            Ok(message) => shell.say(&message),
            Err(error) => shell.note(&error),
        }
        return Next::Go;
    }
    // `!command` runs in the shell, never in the model (the legacy chrome's
    // `SessionAction::RunBash`); without this it was sent as prose.
    if line.trim_start().starts_with('!') {
        return run_user_bash(shell, line);
    }
    // An extension owns its `/command` before the model ever sees the line.
    // The legacy chrome does this in `prepare_user_input`; davinci ran without
    // it, so `/graph-view` and every other extension command was sent to the
    // provider as literal text and came back as "the model returned no text".
    if line.trim_start().starts_with('/') {
        if line.trim_start().starts_with("/learn") {
            let (name, args) = crate::parse_extension_command(line);
            if name == "learn" {
                match crate::native_extensions::learning::parse_learn_args(&args) {
                    Ok(req) => {
                        let prompt = crate::native_extensions::learning::build_learn_prompt(&req);
                        return submit_prompt(shell, &prompt, &[]);
                    }
                    Err(err) => {
                        shell.note(&err);
                        return Next::Go;
                    }
                }
            }
        }
        if let Some(next) = run_extension_command(shell, line) {
            return next;
        }
        // `/diff` is davinci's own: the Δ review sheet (`6d`) over the real
        // working tree. Checked after extensions so one may still claim it.
        if line.trim() == "/diff" {
            return open_diff_sheet(shell);
        }
        // `/todo` — the model's ledger; `/jobs` — the background jobs.
        if let Some(rest) = line.trim().strip_prefix("/todo") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return todo_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/jobs") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return jobs_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/workflow-stop") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return workflow_stop_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/workflow-resume") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return workflow_resume_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/workflows") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return workflows_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/tasks") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return tasks_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/agents") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return agents_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/context") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return context_inspector_command(shell, rest.trim());
            }
        }
        if let Some(rest) = line.trim().strip_prefix("/workflow") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return workflow_command(shell, rest.trim());
            }
        }
    }
    match classify(line) {
        Sent::Quit => {
            run_stop_hooks(shell);
            Next::Leave
        }
        Sent::Say(text) => {
            shell.say(&text);
            Next::Go
        }
        Sent::Command(action) => match perform(shell.parsed, shell.agent, shell.model, action) {
            Ok(done) => {
                let next = shell.finish(done);
                shell.redress();
                next
            }
            Err(err) => {
                shell.note(&err);
                Next::Go
            }
        },
        Sent::Prompt(text) => submit_prompt(shell, &text, &[]),
    }
}

/// One prompt, from the composer or the initial `pi "…"` message, through the
/// same gauntlet the legacy chrome's `prepare_user_input` runs: extensions
/// see it first and may swallow or transform it, then skills and templates
/// expand it, then the turn runs, then whatever was queued behind it.
fn submit_prompt(shell: &mut Shell<'_>, text: &str, images: &[davinci_ai::MessageContent]) -> Next {
    // Whatever ctrl+v attached goes with this prompt.
    let mut images = images.to_vec();
    images.append(shell.images);
    shell.model.extensions.set_status("§images", None);
    let images = images.as_slice();
    // Extensions with an input handler get the line before the model does.
    let input = {
        let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        host.runtime_flag_values = crate::flag_values_json(shell.parsed);
        host.editor_text = shell.model.composer.to_string();
        host.emit_input(text, images, "interactive")
    };
    match apply_host_effects(shell) {
        Next::Go => {}
        other => return other,
    }
    if input.action == "handled" {
        shell.model.running = false;
        return Next::Go;
    }
    let (text, images) = if input.action == "transform" {
        (input.text, input.images)
    } else {
        (text.to_string(), images.to_vec())
    };

    // `prompt` is what writes the user turn to the session file;
    // pushing onto `messages` directly would lose it on restart.
    let expanded =
        davinci_agent::expand_user_text(&text, &shell.agent.skills, &shell.agent.templates);
    // A line that is nothing but a `/word` nobody claims — not a
    // command, not an extension's, not a skill or a template, since
    // expansion left it alone — has nothing the model can do with it.
    // TS sends it anyway; sending it is what produced a turn that
    // answered "the model returned no text" and named no cause.
    if expanded == text {
        if let Some(note) = unknown_command(shell.model, &text) {
            shell.note(&note);
            return Next::Go;
        }
    }
    // Phase 1 is shadow-only: the provider sees the bounded, redacted
    // decision contract, while the deterministic agent remains the sole
    // authority for this turn. Provider failure is deliberately ignored here
    // because the normal coding path must never depend on Jev availability.
    let recent_paths = shell
        .model
        .changes_list
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    let capability_names = shell
        .agent
        .runtime
        .as_ref()
        .map(|runtime| {
            runtime
                .capability_registry
                .list()
                .into_iter()
                .map(|capability| capability.name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut metadata = davinci_coding_agent::decision_state::DecisionMetadata::from_workspace(
        shell.cwd,
        &recent_paths,
        &capability_names,
    );
    let snapshots = shell
        .host
        .try_lock()
        .ok()
        .and_then(|host| host.engineering_snapshots());
    if let Some(runtime) = shell.agent.decision_runtime() {
        let cwd = shell.cwd.to_path_buf();
        let task = expanded.clone();
        let _ = runtime.enqueue_shadow_with(move || {
            // A prior turn's observation is reusable only after freshness/root
            // validation. New-turn invalidation or a concurrent scan yields
            // unknown facts; neither causes work on the submit thread.
            if let Some(snapshot) = snapshots.and_then(|facts| facts.peek_current(&cwd)) {
                metadata.apply_snapshot(
                    snapshot.workspace_dirty,
                    snapshot.index.files.keys().map(String::as_str),
                    snapshot
                        .metadata
                        .packages
                        .iter()
                        .flat_map(|p| p.dependencies.iter().map(String::as_str)),
                );
            }
            davinci_coding_agent::decision_state::build_request_with_metadata(
                davinci_agent::new_message_id(),
                &task,
                davinci_agent::decision::risk::DecisionRisk::Planning,
                metadata,
            )
        });
    }
    if let Some(runtime) = shell.agent.decision_runtime() {
        if let Some(health) = runtime.take_health_transition() {
            if let Some(notice) = decision_health_notice(health) {
                shell.note(notice);
            }
        }
    }
    shell.agent.prompt_user_with(&expanded, &images);
    let next = run_turns(shell);
    shell.redress();
    next
}

fn decision_health_notice(
    health: davinci_agent::decision::provider::DecisionProviderHealth,
) -> Option<&'static str> {
    use davinci_agent::decision::provider::DecisionProviderHealth;
    match health {
        DecisionProviderHealth::CredentialInvalid => Some(
            "TypeSafe decision intelligence credential is invalid; deterministic routing remains active. Reconfigure it in /settings.",
        ),
        DecisionProviderHealth::RateLimited => Some(
            "TypeSafe decision intelligence is rate limited; deterministic routing remains active.",
        ),
        DecisionProviderHealth::Overloaded => Some(
            "TypeSafe decision intelligence is overloaded; deterministic routing remains active.",
        ),
        DecisionProviderHealth::Unavailable => Some(
            "TypeSafe decision intelligence is unavailable; deterministic routing remains active.",
        ),
        DecisionProviderHealth::SchemaMismatch => Some(
            "TypeSafe decision intelligence returned an incompatible response; deterministic routing remains active.",
        ),
        DecisionProviderHealth::Disabled | DecisionProviderHealth::Ready => None,
    }
}

/// One row of an open instrument, chosen with enter.
fn on_choice(shell: &mut Shell<'_>, choice: Choice) -> Next {
    match choice {
        Choice::Command { name, kind } => match kind.as_str() {
            "voice" => {
                shell.model.close();
                shell.voice.open_setup(shell.model);
                Next::Go
            }
            "command" => on_line(shell, &name),
            "session" => {
                let label = name.trim_start_matches("memoria: ").to_string();
                let path = shell
                    .model
                    .sessions
                    .iter()
                    .find(|item| item.name == label)
                    .map(|item| item.path.clone());
                match path {
                    Some(path) => shell.resume(&path),
                    None => {
                        shell.note("that session is no longer on disk");
                        Next::Go
                    }
                }
            }
            // A tool is the agent's to reach for, not the user's to run. The
            // palette row hands its name to the composer instead.
            _ => {
                shell.model.composer.push_str(&name);
                shell.model.mark_caret_moved();
                Next::Go
            }
        },
        Choice::Session(index) => {
            let path = shell
                .model
                .sessions
                .get(index)
                .map(|item| item.path.clone())
                .unwrap_or_default();
            shell.resume(&path)
        }
        Choice::Model(index) => {
            let Some(item) = shell.model.models.get(index).cloned() else {
                return Next::Go;
            };
            shell.agent.provider = item.provider.clone();
            shell.agent.model_id = item.id.clone();
            crate::loaded_extension_host(shell.parsed).emit(
                crate::extension_host::ExtensionEvent::ModelSelect {
                    provider: item.provider.clone(),
                    model: item.id.clone(),
                },
            );
            adopt_model(shell.parsed, shell.agent, shell.model);
            match persist_model_choice(shell.agent) {
                Ok(()) => shell.say(&format!("model {} / {}", item.provider, item.id)),
                Err(err) => shell.say(&format!(
                    "model {} / {} · this run only ({err})",
                    item.provider, item.id
                )),
            }
            Next::Go
        }
        Choice::Ask(index) => {
            let Some(question) = shell.pending.take() else {
                return Next::Go;
            };
            if let Question::GraphSetup(setup) = question {
                return graph_setup::choose(shell, setup, index);
            }
            match answer(shell, &question, index) {
                Ok(text) => shell.say(&text),
                Err(err) => shell.note(&err),
            }
            Next::Go
        }
        // `3a` — a catalog row: switch, unless nothing stands behind it.
        Choice::Catalog(index) => {
            let Some(row) = shell.model.catalog.get(index).cloned() else {
                return Next::Go;
            };
            if row.credential == Credential::Absent {
                shell.note(&format!(
                    "no credential for {} — /login {} adds one",
                    row.name, row.provider
                ));
                return Next::Go;
            }
            let session_only = shell.model.catalog_session_only;
            shell.model.close();
            shell.agent.provider = row.provider.clone();
            shell.agent.model_id = row.id.clone();
            crate::loaded_extension_host(shell.parsed).emit(
                crate::extension_host::ExtensionEvent::ModelSelect {
                    provider: row.provider.clone(),
                    model: row.id.clone(),
                },
            );
            adopt_model(shell.parsed, shell.agent, shell.model);
            if let Some(level) = row
                .reasoning_levels
                .get(row.reasoning_index)
                .filter(|level| shell.model.thinking_levels.contains(level))
                .and_then(|level| davinci_protocol::ThinkingLevel::parse(level))
            {
                shell.agent.thinking_level = level;
                crate::loaded_extension_host(shell.parsed).emit(
                    crate::extension_host::ExtensionEvent::ThinkingLevelSelect {
                        level: level.as_str().to_string(),
                    },
                );
                sync_thinking_state(shell.agent, shell.model);
            }
            if session_only {
                shell.say(&format!(
                    "model {} / {} · this session only",
                    row.provider, row.id
                ));
            } else {
                match persist_model_choice(shell.agent) {
                    Ok(()) => shell.say(&format!("model {} / {}", row.provider, row.id)),
                    Err(err) => shell.say(&format!(
                        "model {} / {} · this run only ({err})",
                        row.provider, row.id
                    )),
                }
            }
            refresh_thinking_sheet(shell.agent, shell.model);
            Next::Go
        }
        // `3b` — advance the setting to its next value and persist it.
        Choice::Setting(index) => cycle_setting(shell, index),
        // `3c` — a thinking level.
        Choice::ThinkingLevel(index) => {
            let Some(level) = shell
                .model
                .thinking_rows
                .get(index)
                .map(|row| row.level.clone())
            else {
                return Next::Go;
            };
            shell.model.thinking_index = index;
            let action = crate::slash::SlashAction::SetThinking(level);
            match perform(shell.parsed, shell.agent, shell.model, action) {
                Ok(Done::Said(text)) => {
                    shell.model.close();
                    shell.say(&text);
                }
                Ok(Done::Note(text)) | Err(text) => shell.note(&text),
                Ok(_) => {}
            }
            Next::Go
        }
        // `3d` — sign in to the chosen provider, then rebuild the ledger so
        // the fresh credential is on it.
        Choice::Provider(index) => {
            let Some(name) = shell.model.providers.get(index).map(|row| row.name.clone()) else {
                return Next::Go;
            };
            shell.model.close();
            let next = shell.detach(Detached::Login {
                provider: name,
                key: None,
            });
            open_login_sheet(shell.parsed, shell.model);
            next
        }
        // `4a` — open the chosen session.
        Choice::ResumeSession(index) => {
            let Some(path) = shell
                .model
                .resume_sessions
                .get(index)
                .map(|row| row.path.clone())
            else {
                return Next::Go;
            };
            shell.model.close();
            shell.resume(&path)
        }
        // `4b` — move the session to the chosen turn.
        Choice::TreeEntry(index) => {
            let Some(node) = shell.model.session_tree.get(index).cloned() else {
                return Next::Go;
            };
            let target = node.entry_id;
            if target.is_empty() {
                return Next::Go;
            }
            {
                let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
                host.emit(crate::extension_host::ExtensionEvent::SessionBeforeTree);
                if host.last_result_cancelled() {
                    drop(host);
                    shell.note("tree navigation cancelled");
                    return Next::Go;
                }
            }
            match shell
                .agent
                .navigate_tree_entry(&target, false, None, false, 16_384)
            {
                Ok(_) => {
                    shell.voice.cancel(shell.model);
                    shell.model.composer_epoch = shell.model.composer_epoch.saturating_add(1);
                    shell.model.close();
                    shell.model.transcript = transcript_from(&shell.agent.messages);
                    shell.say(&format!(
                        "moved to turn {}",
                        node.id.unwrap_or_else(|| target.clone())
                    ));
                    shell.redress();
                }
                Err(err) => shell.note(&format!("could not move there: {err}")),
            }
            Next::Go
        }
        // `6a` — the sheet was read; put the actual decision.
        Choice::TrustDecide => {
            shell.model.close();
            let question = Question::Trust {
                path: shell.agent.cwd.display().to_string(),
                options: crate::trust::get_project_trust_options(&shell.agent.cwd, false),
            };
            shell.finish(Done::Ask(question))
        }
        Choice::Permission(index) => apply_permission_row(shell, index),
        Choice::AgentAction { action, index } => apply_agent_action(shell, action, index),
        Choice::ContextInspectorAction { action, index } => {
            apply_context_inspector_action(shell, action, index)
        }
        Choice::GraphAction { action, index } => apply_graph_action(shell, action, index),
    }
}

fn apply_permission_row(shell: &mut Shell<'_>, index: usize) -> Next {
    let Some(row) = shell.model.permission_rows.get(index).cloned() else {
        return Next::Go;
    };
    if row.kind == "mode" {
        if let Some(mode) = PermissionMode::parse(&row.key) {
            change_permission_mode(shell.agent, shell.model, mode);
            open_permissions_sheet(shell);
        }
        return Next::Go;
    }
    match row.source.as_str() {
        "session" => {
            shell
                .agent
                .permissions
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .session_allow
                .retain(|rule| rule.to_string() != row.key);
        }
        "user" | "project" => {
            let list = if row.detail.starts_with("deny") {
                "deny"
            } else {
                "allow"
            };
            let path = if row.source == "user" {
                crate::settings::settings_path(&crate::default_agent_dir())
            } else {
                crate::permissions::project_settings_path(shell.cwd)
            };
            if let Err(err) = crate::permissions::forget_file_rule(&path, list, &row.key) {
                shell.note(&format!("could not drop `{}`: {err}", row.key));
            }
            // The file changed; the policy in force has to follow it, or the
            // sheet shows a rule gone that the gate still applies.
            reload_file_rules(shell);
        }
        _ => {}
    }
    open_permissions_sheet(shell);
    Next::Go
}

/// Re-read the user and project rule lists into the live policy, keeping
/// what is session-only: the mode, session grants, plan mode, the MCP
/// read-only set.
fn reload_file_rules(shell: &mut Shell<'_>) {
    let fresh = crate::permissions::PermissionSources::load(
        &crate::default_agent_dir(),
        shell.cwd,
        shell.parsed.project_trust_override,
    )
    .policy(None);
    let mut policy = shell
        .agent
        .permissions
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    policy.allow = fresh.allow;
    policy.deny = fresh.deny;
}

fn persist_setting_row<F>(row: &mut SettingRow, persist: F) -> Result<String, String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    if row.values.is_empty() {
        return Ok(row.value.clone());
    }
    let at = row
        .values
        .iter()
        .position(|value| value == &row.value)
        .unwrap_or(0);
    let next = row.values[(at + 1) % row.values.len()].clone();
    persist(&format!("{}={next}", row.key))?;
    row.value = next.clone();
    row.project = false;
    Ok(next)
}

/// `3b` — advance a setting to the next value on its ramp, write it through
/// the same store the legacy overlay writes, and re-honour it at once where
/// davinci reads it live.
fn cycle_setting(shell: &mut Shell<'_>, index: usize) -> Next {
    let Some(row) = shell.model.settings_rows.get(index) else {
        return Next::Go;
    };
    if row.values.is_empty() {
        return Next::Go;
    }
    let key = row.key.clone();
    let current = row.value.clone();
    let project = row.project;
    if key == "typesafe-api-key" {
        if let Some(reason) = typesafe_key_replacement_blocker(
            &shell.model.settings_rows,
            std::env::var_os("TYPESAFE_API_KEY").is_some(),
        ) {
            shell.note(reason);
            return Next::Go;
        }
        shell.model.settings_index = index;
        open_typesafe_key_input(shell.model);
        return Next::Go;
    }
    if key == "decision-intelligence" {
        return cycle_decision_intelligence(shell, index, current == "on", project);
    }
    let Some(row) = shell.model.settings_rows.get_mut(index) else {
        return Next::Go;
    };
    if let Err(err) = persist_setting_row(row, crate::persist_interactive_setting) {
        shell.note(&err);
        return Next::Go;
    }

    crate::sync_agent_from_settings(shell.agent);
    open_settings_sheet(shell.agent, shell.model);
    shell.model.settings_index = index.min(shell.model.settings_rows.len().saturating_sub(1));
    let effective = shell
        .model
        .settings_rows
        .iter()
        .find(|row| row.key == key)
        .map(|row| row.value.clone())
        .unwrap_or_default();
    match key.as_str() {
        "autocomplete-max-visible" => {
            if let Ok(rows) = effective.parse::<usize>() {
                shell.model.suggestion_rows = rows.clamp(3, 20);
            }
        }
        "terminal-progress" => shell.model.terminal_progress = effective == "true",
        "double-escape-action" => shell.model.double_escape_action = effective,
        "show-tool-output" => shell.model.show_tool_output = effective == "true",
        _ => {}
    }
    Next::Go
}

fn typesafe_key_replacement_blocker(
    rows: &[SettingRow],
    environment_override: bool,
) -> Option<&'static str> {
    if environment_override {
        return Some(
            "TYPESAFE_API_KEY controls the current TypeSafe credential. Unset or change that environment variable, then restart DaVinci.",
        );
    }
    if rows
        .iter()
        .any(|row| row.key == "decision-intelligence" && row.project && row.value == "off")
    {
        return Some("TypeSafe / Jev decision intelligence is disabled by project settings");
    }
    None
}

fn paste_secret_clipboard(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    read_text: impl FnOnce() -> Option<String>,
) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    if model.overlay != Some(Overlay::SecretInput)
        || key.kind == KeyEventKind::Release
        || key.code != KeyCode::Char('v')
        || !key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return false;
    }
    if let Some(text) = read_text() {
        let text = zeroize::Zeroizing::new(text);
        model.paste(&text);
    }
    true
}

fn open_typesafe_key_input(model: &mut Model) {
    model.secret_input = Some(davinci_tui::davinci::views::secret_input::SecretInputState::new());
    model.overlay = Some(Overlay::SecretInput);
}

fn cycle_decision_intelligence(
    shell: &mut Shell<'_>,
    index: usize,
    enabled: bool,
    project_disabled: bool,
) -> Next {
    if enabled {
        if let Err(error) = crate::persist_interactive_setting("decision-intelligence=off") {
            shell.note(&error);
            return Next::Go;
        }
        shell.agent.disable_decision_runtime();
        crate::sync_agent_from_settings(shell.agent);
        open_settings_sheet(shell.agent, shell.model);
        shell.model.settings_index = index.min(shell.model.settings_rows.len().saturating_sub(1));
        return Next::Go;
    }

    if project_disabled {
        shell.note("TypeSafe / Jev decision intelligence is disabled by project settings");
        return Next::Go;
    }

    let auth = match davinci_ai::AuthStorage::create() {
        Ok(auth) => auth,
        Err(error) => {
            shell.note(&format!("TypeSafe credential storage unavailable: {error}"));
            return Next::Go;
        }
    };
    let key = match davinci_coding_agent::decision_providers::typesafe::resolve_api_key(&auth) {
        Ok(Some(key)) => key,
        Ok(None) => {
            open_typesafe_key_input(shell.model);
            return Next::Go;
        }
        Err(error) => {
            shell.note(&error.to_string());
            return Next::Go;
        }
    };
    enable_typesafe_with_key(shell, index, key, false)
}

fn on_secret_input(shell: &mut Shell<'_>, candidate: String) -> Next {
    enable_typesafe_with_key(shell, shell.model.settings_index, candidate, true)
}

fn enable_typesafe_with_key(
    shell: &mut Shell<'_>,
    index: usize,
    candidate: String,
    persist_credential: bool,
) -> Next {
    let raw_candidate = zeroize::Zeroizing::new(candidate);
    let candidate = match davinci_coding_agent::decision_providers::typesafe::normalize_api_key(
        raw_candidate.as_str(),
    ) {
        Some(candidate) => zeroize::Zeroizing::new(candidate.to_owned()),
        None => {
            shell.note("TypeSafe credential validation failed: TypeSafe credential is empty");
            return Next::Go;
        }
    };
    drop(raw_candidate);
    if let Err(error) =
        davinci_coding_agent::decision_providers::typesafe::TypeSafeProvider::validate_api_key(
            &candidate,
        )
    {
        shell.note(&format!("TypeSafe credential validation failed: {error}"));
        return Next::Go;
    }

    let mut auth = match davinci_ai::AuthStorage::create() {
        Ok(auth) => auth,
        Err(error) => {
            shell.note(&format!("TypeSafe credential storage unavailable: {error}"));
            return Next::Go;
        }
    };
    let previous = zeroize::Zeroizing::new(
        auth.get("typesafe")
            .and_then(|credential| credential.key.clone()),
    );
    if persist_credential {
        if let Err(error) = auth.login_api_key("typesafe", candidate.to_string()) {
            shell.note(&format!("TypeSafe credential was not saved: {error}"));
            return Next::Go;
        }
    }

    let dir = crate::default_agent_dir();
    let mut settings = crate::settings::load_settings(&dir);
    settings.decision_intelligence =
        Some(crate::settings::DecisionIntelligenceSettings { enabled: true });
    if let Err(error) = crate::settings::save_settings(&dir, &settings) {
        if persist_credential {
            let rollback = match previous.as_ref() {
                Some(previous) => auth.login_api_key("typesafe", previous.clone()),
                None => auth.remove("typesafe"),
            };
            if let Err(rollback_error) = rollback {
                shell.note(&format!(
                    "TypeSafe enable failed and credential rollback failed: {rollback_error}"
                ));
                return Next::Go;
            }
        }
        shell.note(&format!("TypeSafe decision setting was not saved: {error}"));
        return Next::Go;
    }

    let provider = Arc::new(
        davinci_coding_agent::decision_providers::typesafe::TypeSafeProvider::new(
            candidate.to_string(),
        ),
    );
    if let Some(runtime) = shell.agent.decision_runtime() {
        runtime.replace_provider(provider);
        runtime.enable();
    } else {
        let runtime = Arc::new(davinci_agent::decision::DecisionRuntime::new(provider));
        runtime.enable();
        shell.agent.set_decision_runtime(runtime);
    }
    crate::sync_agent_from_settings(shell.agent);
    open_settings_sheet(shell.agent, shell.model);
    shell.model.settings_index = index.min(shell.model.settings_rows.len().saturating_sub(1));
    if persist_credential {
        shell.say("TypeSafe / Jev API key saved and decision intelligence enabled");
    } else {
        shell.say("TypeSafe / Jev decision intelligence enabled");
    }
    Next::Go
}

/// Carry out the row chosen from a question.
fn answer(shell: &mut Shell<'_>, question: &Question, index: usize) -> Result<String, String> {
    match question {
        Question::GraphSetup(_) => Err("Graph setup must be handled by the launch flow".into()),
        Question::Trust { options, .. } => {
            let Some(option) = options.get(index) else {
                return Err("that trust option is gone".into());
            };
            let store = crate::trust::ProjectTrustStore::open(&crate::default_agent_dir());
            store.set_many(&option.updates)?;
            Ok(format!(
                "saved: {}. it takes effect the next time pi starts.",
                option.label
            ))
        }
        Question::FirstRun => {
            let dir = crate::default_agent_dir();
            let mut stored = crate::settings::load_settings(&dir);
            let share = index == 0;
            crate::settings::set_enable_analytics(&mut stored, share);
            crate::settings::save_settings(&dir, &stored)?;
            let _ = shell;
            Ok(if share {
                "sharing anonymous usage data. change it any time in settings.".into()
            } else {
                "nothing leaves this machine. change it any time in settings.".into()
            })
        }
        Question::Logout { providers } => {
            let Some(provider) = providers.get(index) else {
                return Err("that credential is gone".into());
            };
            let mut storage = davinci_ai::AuthStorage::create().map_err(|err| err.to_string())?;
            storage.remove(provider).map_err(|err| err.to_string())?;
            Ok(format!("removed {provider}"))
        }
    }
}

/// What recall searches for: what is being typed, else the last thing the
/// user asked. Opening recall with nothing in hand should still recall
/// something about the work in progress.
pub fn recall_query(model: &Model, agent: &Agent) -> String {
    let typed = model.composer.trim();
    if !typed.is_empty() {
        return typed.to_string();
    }
    agent
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| {
            let text: String = message
                .content
                .iter()
                .filter_map(|part| match part {
                    davinci_ai::MessageContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            clip(text.trim(), 120)
        })
        .unwrap_or_default()
}

/// `/todo` — the model's ledger as a STUDIO box between turns; `/todo
/// clear` empties it (and the session's record of it).
fn todo_command(shell: &mut Shell<'_>, arg: &str) -> Next {
    if arg == "clear" {
        *shell
            .agent
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = davinci_agent::TodoList::default();
        shell.agent.persist_todos();
        shell.model.plan.clear();
        shell.model.running = false;
        shell.model.transcript.push(Entry::Gap);
        shell.model.transcript.push(Entry::tool(
            State::Done,
            "instrumenta",
            "ledger cleared",
            None,
        ));
        return Next::Go;
    }
    if !arg.is_empty() {
        shell.note("usage: /todo — the model's ledger · /todo clear");
        return Next::Go;
    }
    let list = shell
        .agent
        .tool_context
        .todos
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    shell.model.running = false;
    shell.model.transcript.push(Entry::Gap);
    if list.is_empty() {
        shell.model.transcript.push(Entry::tool(
            State::Queued,
            "instrumenta",
            "no ledger · the model keeps one with the todo tool on longer tasks",
            None,
        ));
        return Next::Go;
    }
    shell.model.plan = plan_from_todos(&list);
    shell
        .model
        .transcript
        .push(Entry::Studio(steps_from_todos(&list)));
    Next::Go
}

/// `/jobs` — every background job of the session, one row each; `/jobs
/// kill <id>` stops one.
fn jobs_command(shell: &mut Shell<'_>, arg: &str) -> Next {
    let jobs = shell.agent.tool_context.jobs.clone();
    if let Some(id) = arg.strip_prefix("kill") {
        let Ok(id) = id.trim().parse::<u32>() else {
            shell.note("usage: /jobs kill <id>");
            return Next::Go;
        };
        let killed = jobs.lock().unwrap_or_else(|err| err.into_inner()).kill(id);
        shell.model.running = false;
        shell.model.transcript.push(Entry::Gap);
        shell.model.transcript.push(match killed {
            Some(status) => Entry::tool(
                State::Done,
                "manus",
                &format!("job {id} · {}", status.describe()),
                None,
            ),
            None => Entry::tool(State::Attention, "manus", &format!("no job {id}"), None),
        });
        return Next::Go;
    }
    if !arg.is_empty() {
        shell.note("usage: /jobs — the background jobs · /jobs kill <id>");
        return Next::Go;
    }
    let summaries = jobs
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .summaries();
    shell.model.running = false;
    shell.model.transcript.push(Entry::Gap);
    if summaries.is_empty() {
        shell.model.transcript.push(Entry::tool(
            State::Queued,
            "manus",
            "no background jobs · bash with background: true starts one",
            None,
        ));
        return Next::Go;
    }
    for job in summaries {
        let state = match job.status {
            davinci_agent::JobStatus::Running => State::Active,
            davinci_agent::JobStatus::Exited(0) => State::Done,
            davinci_agent::JobStatus::Exited(_) => State::Failed,
            davinci_agent::JobStatus::Killed => State::Skipped,
        };
        shell.model.transcript.push(
            Entry::tool(
                state,
                "manus",
                &format!("job {} · {}", job.id, clip(&job.command, 50)),
                Some(&davinci_agent::jobs::format_elapsed(job.elapsed)),
            )
            .summarised(&job.status.describe()),
        );
    }
    Next::Go
}

fn open_workflows_sheet(agent: &Agent, model: &mut Model) {
    let workflows = if let Some(runtime) = &agent.runtime {
        if let Some(exec) = &runtime.workflow_executor {
            let list = exec.list_workflows();
            list.into_iter()
                .map(|w| {
                    let mut phases = Vec::new();
                    for (p_id, p_st) in &w.phases {
                        phases.push((p_id.clone(), format!("{:?}", p_st.status).to_lowercase()));
                    }
                    WorkflowRow {
                        id: w.id.to_string(),
                        name: w.name,
                        status: format!("{:?}", w.status).to_lowercase(),
                        phases,
                        started_ms: w.started_ms,
                        elapsed: if let Some(fin) = w.finished_ms {
                            format!("{}ms", fin.saturating_sub(w.started_ms))
                        } else {
                            "running".into()
                        },
                        error: w.error,
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    model.workflows = Some(WorkflowsSheet {
        workflows,
        selected_index: 0,
    });
    open_sheet(model, Screen::Workflows);
}

fn workflows_command(shell: &mut Shell<'_>, _arg: &str) -> Next {
    open_workflows_sheet(shell.agent, shell.model);
    Next::Go
}

fn open_task_board_sheet(agent: &Agent, model: &mut Model) {
    let tasks = if let Some(runtime) = &agent.runtime {
        crate::davinci_surfaces::task_board(runtime)
    } else {
        Vec::new()
    };

    model.task_board = Some(davinci_tui::davinci::model::TaskBoardSheet {
        tasks,
        selected_index: 0,
    });
    open_sheet(model, Screen::TaskBoard);
}

fn tasks_command(shell: &mut Shell<'_>, _arg: &str) -> Next {
    open_task_board_sheet(shell.agent, shell.model);
    Next::Go
}

fn open_agents_sheet(agent: &Agent, model: &mut Model) {
    let agents = if let Some(runtime) = &agent.runtime {
        crate::davinci_surfaces::agents_sheet(runtime)
    } else {
        Vec::new()
    };

    model.agents = Some(davinci_tui::davinci::model::AgentsSheet {
        agents,
        selected_index: 0,
    });
    open_sheet(model, Screen::Agents);
}

fn agents_command(shell: &mut Shell<'_>, _arg: &str) -> Next {
    open_agents_sheet(shell.agent, shell.model);
    Next::Go
}

fn apply_agent_action(shell: &mut Shell<'_>, action: &str, index: usize) -> Next {
    let Some(sheet) = shell.model.agents.as_ref() else {
        return Next::Go;
    };
    let Some(agent_row) = sheet.agents.get(index).cloned() else {
        return Next::Go;
    };
    let Ok(agent_id) = agent_row.id.parse::<davinci_agent::runtime::AgentId>() else {
        return Next::Go;
    };

    match action {
        "inspect" => {
            let owned_str = if agent_row.owned_paths.is_empty() {
                "none".to_string()
            } else {
                agent_row.owned_paths.join(", ")
            };
            shell.say(&format!(
                "Worker '{}' (id: {}):\n  Role: {}\n  Status: {}\n  Elapsed: {}\n  Tools: {}\n  Owned paths: {}\n  Waiting on: {}",
                agent_row.name,
                agent_row.id,
                agent_row.role,
                agent_row.status,
                agent_row.elapsed,
                agent_row.tool_count,
                owned_str,
                agent_row.waiting_on.as_deref().unwrap_or("none")
            ));
        }
        "steer" => {
            if let Some(runtime) = &shell.agent.runtime {
                let _ = runtime.mailbox.send_steer(
                    agent_id,
                    0,
                    "focus on primary objective".into(),
                    true,
                );
                shell.note(&format!(
                    "Steering message queued for worker '{}'",
                    agent_row.name
                ));
            }
        }
        "stop" => {
            if let Some(runtime) = &shell.agent.runtime {
                let controller =
                    davinci_agent::runtime::WorkerController::new(runtime.registry.clone());
                let cmd = davinci_agent::runtime::WorkerControlCommand {
                    id: uuid::Uuid::new_v4(),
                    root_run_id: runtime.run_id,
                    agent_id,
                    generation: 0,
                    task_id: None,
                    expected_revision: 0,
                    action: davinci_agent::runtime::WorkerControlAction::Stop {
                        reason: Some("stopped via live agent panel".into()),
                    },
                };
                let receipt = controller.execute_command(cmd, true);
                shell.note(&format!(
                    "Stop requested for worker '{}': status {:?}",
                    agent_row.name, receipt.status
                ));
            }
            open_agents_sheet(shell.agent, shell.model);
        }
        "retry" => {
            if let Some(runtime) = &shell.agent.runtime {
                let controller =
                    davinci_agent::runtime::WorkerController::new(runtime.registry.clone());
                let cmd = davinci_agent::runtime::WorkerControlCommand {
                    id: uuid::Uuid::new_v4(),
                    root_run_id: runtime.run_id,
                    agent_id,
                    generation: 0,
                    task_id: None,
                    expected_revision: 0,
                    action: davinci_agent::runtime::WorkerControlAction::Retry {
                        reason: Some("retry via live agent panel".into()),
                    },
                };
                let receipt = controller.execute_command(cmd, true);
                shell.note(&format!(
                    "Retry requested for worker '{}': status {:?}",
                    agent_row.name, receipt.status
                ));
            }
            open_agents_sheet(shell.agent, shell.model);
        }
        "diff" => {
            if let Ok(baseline) =
                crate::native_extensions::graph::mutation::capture_baseline(&shell.agent.cwd)
            {
                if let Ok(report) = crate::native_extensions::graph::mutation::compute_owned_diff(
                    &shell.agent.cwd,
                    &baseline,
                    &[],
                ) {
                    if report.owned_diff.is_empty() && report.unattributed_diff.is_none() {
                        shell.say(&format!(
                            "Worker '{}': no working tree changes.",
                            agent_row.name
                        ));
                    } else {
                        shell.say(&format!(
                            "Worker '{}' owned diff:\n{}\n{}",
                            agent_row.name,
                            report.owned_diff,
                            report.unattributed_diff.as_deref().unwrap_or("")
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Next::Go
}

fn open_context_inspector_sheet(agent: &davinci_agent::Agent, model: &mut Model) {
    if let Some(manifest) = &agent.last_prepared_manifest {
        model.context_inspector = Some(
            crate::davinci_surfaces::context_inspector_sheet_from_manifest(
                manifest, None, false, false,
            ),
        );
    } else {
        model.context_inspector =
            Some(davinci_tui::davinci::model::ContextInspectorSheet::default());
    }
    open_sheet(model, Screen::ContextInspector);
}

fn context_inspector_command(shell: &mut Shell<'_>, _arg: &str) -> Next {
    open_context_inspector_sheet(shell.agent, shell.model);
    Next::Go
}

fn apply_context_inspector_action(shell: &mut Shell<'_>, action: &str, index: usize) -> Next {
    let Some(sheet) = shell.model.context_inspector.as_mut() else {
        return Next::Go;
    };
    let Some(row) = sheet.rows.get_mut(index) else {
        return Next::Go;
    };

    match action {
        "preview" => {
            sheet.preview_active = !sheet.preview_active;
        }
        "pin" => {
            if !davinci_agent::runtime::overlay_change_allowed(row.mandatory, "pin", true) {
                sheet.confirmation_dialog =
                    Some("Cannot pin or exclude mandatory policy items".into());
            } else {
                row.pinned = !row.pinned;
                if row.pinned {
                    row.selected = true;
                }
                sheet.overlay_revision += 1;
                sheet.confirmation_dialog = None;
            }
        }
        "exclude" => {
            if !davinci_agent::runtime::overlay_change_allowed(row.mandatory, "exclude", true) {
                sheet.confirmation_dialog =
                    Some("Cannot exclude mandatory policy items: exclusion rejected".into());
            } else {
                row.selected = !row.selected;
                if !row.selected {
                    row.pinned = false;
                }
                sheet.overlay_revision += 1;
                sheet.confirmation_dialog = None;
            }
        }
        "refresh" => {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            row.last_refreshed_at = Some(format!("{now_secs}s"));
            row.freshness = "fresh".into();
            sheet.overlay_revision += 1;
            sheet.confirmation_dialog =
                Some(format!("Refreshed authorized evidence for {}", row.item_id));
        }
        "toggle_pending" => {
            sheet.show_pending = !sheet.show_pending;
        }
        _ => {}
    }
    Next::Go
}

fn apply_graph_action(shell: &mut Shell<'_>, action: &str, index: usize) -> Next {
    if action == "resume"
        || (action == "pause_resume"
            && shell
                .model
                .graph_run
                .as_ref()
                .is_some_and(|run| run.can_resume()))
    {
        let Some(run) = shell.model.graph_run.as_ref() else {
            return Next::Go;
        };
        if !run.can_resume() {
            shell.note("This graph cannot be resumed. Start a new /graph goal.");
            return Next::Go;
        }
        let command = format!("/graph-resume {}", run.id);
        return run_extension_command(shell, &command).unwrap_or(Next::Go);
    }
    let Some(sheet) = shell.model.graph_run.as_mut() else {
        return Next::Go;
    };
    let node_id = sheet.tasks.get(index).map(|t| t.id.clone());
    match action {
        "inspect" => {
            sheet.inspecting_node = !sheet.inspecting_node;
        }
        "diff" => {
            sheet.showing_diff = !sheet.showing_diff;
        }
        "pause_resume" => {
            let is_paused = sheet.lifecycle == "paused" || sheet.lifecycle == "pause_requested";
            let cmd = if is_paused {
                "graph-resume"
            } else {
                "graph-pause"
            };
            let _ = shell
                .host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .execute_native_command(cmd, &sheet.id);
            sheet.control_status = Some(if is_paused {
                "Resume requested".into()
            } else {
                "Pause requested".into()
            });
        }
        "stop" => {
            let arg = if let Some(ref nid) = node_id {
                format!("{} {}", sheet.id, nid)
            } else {
                sheet.id.clone()
            };
            let _ = shell
                .host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .execute_native_command("graph-stop", &arg);
            sheet.control_status = Some(format!(
                "Stop requested for {}",
                node_id.as_deref().unwrap_or("graph")
            ));
        }
        "retry" => {
            if let Some(ref nid) = node_id {
                let arg = format!("{} {}", sheet.id, nid);
                let _ = shell
                    .host
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .execute_native_command("graph-retry", &arg);
                sheet.control_status = Some(format!("Retry requested for {}", nid));
            }
        }
        _ => {}
    }
    Next::Go
}

fn workflow_command(shell: &mut Shell<'_>, goal: &str) -> Next {
    let goal = goal.trim();
    if goal.is_empty() {
        open_workflows_sheet(shell.agent, shell.model);
        return Next::Go;
    }

    let trusted = shell
        .agent
        .runtime
        .as_ref()
        .is_some_and(|r| r.project_trusted);
    match davinci_agent::find_saved_workflow(&shell.agent.cwd, goal, trusted) {
        Ok(spec) => {
            if let Some(runtime) = &shell.agent.runtime {
                if let Some(exec) = &runtime.workflow_executor {
                    match exec.execute_background(spec) {
                        Ok(wf_id) => {
                            shell.model.running = false;
                            shell.model.transcript.push(Entry::Gap);
                            shell.model.transcript.push(Entry::tool(
                                State::Done,
                                "opus",
                                &format!("workflow '{}' ({}) started in background", goal, wf_id),
                                None,
                            ));
                            return Next::Go;
                        }
                        Err(err) => {
                            shell.note(&format!("failed to start workflow: {err}"));
                            return Next::Go;
                        }
                    }
                }
            }
            shell.note("runtime workflow executor not available");
            Next::Go
        }
        Err(err) => {
            if err.contains("requires project trust") {
                shell.note(&err);
                return Next::Go;
            }
            submit_prompt(
                shell,
                &format!(
                    "Create and execute a deterministic workflow using workflow_run for the following goal:\n{}",
                    goal
                ),
                &[],
            )
        }
    }
}

fn workflow_stop_command(shell: &mut Shell<'_>, id_str: &str) -> Next {
    let id_str = id_str.trim();
    if id_str.is_empty() {
        shell.note("usage: /workflow-stop <id>");
        return Next::Go;
    }

    let Some(runtime) = &shell.agent.runtime else {
        shell.note("runtime subsystem not available");
        return Next::Go;
    };
    let Some(exec) = &runtime.workflow_executor else {
        shell.note("workflow executor not initialized");
        return Next::Go;
    };

    let list = exec.list_workflows();
    let target = list
        .iter()
        .find(|w| w.id.to_string().starts_with(id_str))
        .map(|w| w.id);

    match target {
        Some(wf_id) => match exec.cancel(&wf_id) {
            Ok(()) => {
                shell.model.running = false;
                shell.model.transcript.push(Entry::Gap);
                shell.model.transcript.push(Entry::tool(
                    State::Done,
                    "opus",
                    &format!("workflow {} stopped", wf_id),
                    None,
                ));
            }
            Err(err) => {
                shell.note(&format!("error stopping workflow: {err}"));
            }
        },
        None => {
            shell.note(&format!("no workflow matching '{}'", id_str));
        }
    }
    Next::Go
}

fn workflow_resume_command(shell: &mut Shell<'_>, id_str: &str) -> Next {
    let id_str = id_str.trim();
    if id_str.is_empty() {
        shell.note("usage: /workflow-resume <id>");
        return Next::Go;
    }

    let Some(runtime) = &shell.agent.runtime else {
        shell.note("runtime subsystem not available");
        return Next::Go;
    };
    let Some(exec) = &runtime.workflow_executor else {
        shell.note("workflow executor not initialized");
        return Next::Go;
    };

    let list = exec.list_workflows();
    let target = list
        .iter()
        .find(|w| w.id.to_string().starts_with(id_str))
        .map(|w| w.id);

    match target {
        Some(wf_id) => match exec.resume(&wf_id) {
            Ok(()) => {
                shell.model.running = false;
                shell.model.transcript.push(Entry::Gap);
                shell.model.transcript.push(Entry::tool(
                    State::Done,
                    "opus",
                    &format!("workflow {} resumed", wf_id),
                    None,
                ));
            }
            Err(err) => {
                shell.note(&format!("error resuming workflow: {err}"));
            }
        },
        None => {
            shell.note(&format!("no workflow matching '{}'", id_str));
        }
    }
    Next::Go
}

fn run_stop_hooks(shell: &mut Shell<'_>) {
    let settings = crate::settings::load_merged_settings(&crate::default_agent_dir(), shell.cwd);
    let trusted =
        crate::settings::is_trusted(&settings, shell.cwd, shell.parsed.project_trust_override);
    crate::hooks::run_stop(&crate::hooks::load(
        &crate::default_agent_dir(),
        shell.cwd,
        trusted,
    ));
}

/// `/permissions` — the mode and every rule in force, by source; or, with a
/// mode named, that mode for the rest of the session. Rules are drawn as
/// tool rows rather than prose: `bash(git *)` is not markdown emphasis.
fn sync_permission_state(agent: &Agent, model: &mut Model) {
    model.permission_mode = agent.permission_mode().as_str().to_string();
}

fn change_permission_mode(agent: &mut Agent, model: &mut Model, mode: PermissionMode) {
    agent.set_permission_mode(mode);
    sync_permission_state(agent, model);
}

fn cycle_permission_mode(agent: &mut Agent, model: &mut Model) {
    let next = agent.permission_mode().next();
    change_permission_mode(agent, model, next);
}

fn permissions_command(shell: &mut Shell<'_>, arg: &str) -> Next {
    if !arg.is_empty() {
        match PermissionMode::parse(arg) {
            Some(mode) => {
                change_permission_mode(shell.agent, shell.model, mode);
                shell.say(&format!(
                    "{} · {} · this session",
                    mode.label(),
                    mode.describe()
                ));
            }
            None => shell.note(&format!(
                "no permission mode {arg} — Manual, Accept Edits, Plan Mode, Auto Mode or Always Approve"
            )),
        }
        return Next::Go;
    }
    open_permissions_sheet(shell);
    Next::Go
}

fn open_permissions_sheet(shell: &mut Shell<'_>) {
    let sources = crate::permissions::PermissionSources::load(
        &crate::default_agent_dir(),
        shell.cwd,
        shell.parsed.project_trust_override,
    );
    let policy = shell
        .agent
        .permissions
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let mut rows = Vec::new();
    for mode in PermissionMode::ALL {
        rows.push(PermissionRow {
            label: mode.label().into(),
            detail: mode.describe().into(),
            current: policy.mode == mode,
            kind: "mode".into(),
            key: mode.as_str().into(),
            source: String::new(),
        });
    }
    let push_rules = |rows: &mut Vec<PermissionRow>, source: &str, list: &str, rules: &[String]| {
        for rule in rules {
            rows.push(PermissionRow {
                label: rule.clone(),
                detail: format!("{list} · {source}"),
                current: false,
                kind: "rule".into(),
                key: rule.clone(),
                source: source.into(),
            });
        }
    };
    push_rules(&mut rows, "user", "allow", &sources.user.allow);
    push_rules(&mut rows, "user", "deny", &sources.user.deny);
    if let Some(project) = &sources.project {
        push_rules(&mut rows, "project", "allow", &project.allow);
        push_rules(&mut rows, "project", "deny", &project.deny);
    }
    let session: Vec<String> = policy
        .session_allow
        .iter()
        .map(ToString::to_string)
        .collect();
    push_rules(&mut rows, "session", "allow", &session);
    let current = rows.iter().position(|row| row.current).unwrap_or(0);
    shell.model.permission_rows = rows;
    shell.model.permission_index = current;
    open_sheet(shell.model, Screen::Permissions);
}

fn detached_login_message(provider: &str, oauth_pending: bool) -> Result<String, String> {
    if oauth_pending {
        if davinci_ai::PROVIDER_SPECS
            .iter()
            .find(|spec| spec.id == provider)
            .is_some_and(|spec| !spec.oauth)
        {
            return Err(format!(
                "API key required for {provider}. Run /login {provider} <api-key>."
            ));
        }
        Err(format!("authorization required to sign in to {provider}"))
    } else {
        Ok(format!("signed in to {provider}"))
    }
}

fn refresh_context(model: &mut Model, agent: &Agent) {
    // Session restoration and fail-closed tool failures can change the mode
    // without a composer event; always redraw from the authoritative policy.
    sync_permission_state(agent, model);
    model.context = (
        davinci_agent::estimate_context_tokens(&agent.messages),
        agent.context_window,
    );
    model.model_name = agent.model_id.clone();
    model.active_provider = agent.provider.clone();
}

#[cfg(test)]
mod tests {
    #[test]
    fn security_tui_maps_cancelled_failed_and_admission_without_success() {
        use davinci_tui::davinci::{
            model::Model,
            theme::{ColorDepth, Theme},
            views::securitas,
        };
        fn drawn(sheet: davinci_tui::davinci::model::SecurityScan) -> String {
            let mut model =
                Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
            model.security = Some(sheet);
            securitas::lines(&model)
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        for status in ["cancelled", "failed", "interrupted"] {
            let sheet = super::security_sheet(&serde_json::json!({
                "schemaVersion": 2,
                "scanId": "11111111-1111-1111-1111-111111111111",
                "status": status,
                "coverageComplete": false,
                "findings": [],
                "coverage": {"eligibleFiles": 1, "reviewedPaths": [], "skipped": []}
            }));
            assert_eq!(sheet.state, status);
            assert_eq!(sheet.id, "11111111-1111-1111-1111-111111111111");
            let text = drawn(sheet);
            assert!(text.contains(&format!("Status: {status}")), "{text}");
            assert!(text.contains("coverage incomplete"), "{text}");
            for claim in [
                "complete for captured scope",
                "report sealed",
                "never left",
                "coverage complete",
            ] {
                assert!(!text.contains(claim), "{status}: {text}");
            }
        }
        let missing = super::security_sheet(&serde_json::json!({"schemaVersion": 2}));
        assert_eq!(missing.state, "unknown");
        assert!(!drawn(missing).contains("Status: completed"));
        let mut empty = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        empty.security = None;
        let text = securitas::lines(&empty)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("No security scan available"));
        assert!(!text.contains("Status: completed"));
        let view = crate::native_extensions::security_scan::report::project_legacy_v1(
            br#"{"schemaVersion":1,"validated":true,"findings":[{"severity":"critical","title":"old"}]}"#,
        )
        .unwrap();
        let sheet = super::security_sheet(&view);
        assert_eq!(sheet.state, "legacy-readonly");
        assert_eq!(sheet.fraction, 0.0);
        let text = drawn(sheet);
        assert!(text.contains("Status: legacy-readonly"), "{text}");
        assert!(text.contains("not v2 confirmation"), "{text}");
        assert!(!text.contains("Status: completed"), "{text}");
        assert!(!text.contains("report sealed"), "{text}");
    }

    #[test]
    fn security_sheet_preserves_grouped_occurrence_assessments() {
        let report = serde_json::json!({"schemaVersion":2,"findings":[{
            "claim":{"title":"Shared control","locations":[{"path":"control.rs","startLine":1}]},
            "assessment":{"classification":"confirmed","severity":"high","reason":"First reason"},
            "occurrences":[
                {"claim":{"entrypoint":"First caller"},"assessment":{"classification":"confirmed","severity":"high","reason":"First reason"}},
                {"claim":{"entrypoint":"Second caller","prerequisites":["Second prerequisite"],"locations":[{"path":"second.rs","startLine":2,"endLine":3}]},"assessment":{"classification":"likely","severity":"critical","reason":"Second reason","proofGaps":["Second unknown condition"]}}]}]});
        let sheet = super::security_sheet(&report);
        assert_eq!(sheet.findings.len(), 1);
        assert!(sheet.findings[0].message.contains("2 occurrences"));
        assert!(sheet.findings[0].evidence.contains("First caller"));
        assert!(sheet.findings[0].evidence.contains("likely / critical"));
        assert!(sheet.findings[0].evidence.contains("Second caller"));
        assert!(sheet.findings[0]
            .evidence
            .contains("Second unknown condition"));
        assert!(sheet.findings[0].evidence.contains("Second prerequisite"));
        assert!(sheet.findings[0].evidence.contains("second.rs:2-3"));
    }
    use super::*;
    use serde_json::json;

    #[test]
    fn a_home_path_is_said_with_the_home_variable() {
        let home = davinci_session::home_dir().expect("a home directory");
        let label = home_label(&home.join(".pi").join("agent").join("auth.json"));
        if cfg!(windows) {
            assert!(label.starts_with("%USERPROFILE%"), "{label}");
        } else {
            assert!(label.starts_with("~/"), "{label}");
        }
        assert!(label.ends_with("auth.json"), "{label}");
        let elsewhere = std::path::Path::new("/srv/pi/auth.json");
        assert_eq!(home_label(elsewhere), elsewhere.display().to_string());
    }

    #[test]
    fn a_missing_models_file_has_no_refreshed_label() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(refreshed_label(&dir.path().join("models.json")), "");
        std::fs::write(dir.path().join("models.json"), "{}").unwrap();
        assert_eq!(refreshed_label(&dir.path().join("models.json")), "just now");
        assert_eq!(age_label(7_200), "2h ago");
    }

    #[test]
    fn thinking_facts_come_from_the_reported_usage() {
        assert_eq!(thinking_facts(&[]), (String::new(), 0.0));
        let entry = |usage: serde_json::Value| {
            let mut entry = davinci_session::SessionEntry::message("assistant", json!([]));
            entry.message = Some(json!({"role": "assistant", "usage": usage}));
            entry
        };
        let entries = vec![
            entry(json!({"output": 1000, "reasoning": 400})),
            entry(json!({"output": 1000})),
            entry(json!({"output": 2000, "reasoning": 1200})),
        ];
        let (last, share) = thinking_facts(&entries);
        assert_eq!(last, "1.2k tokens");
        assert!((share - 0.4).abs() < 1e-9, "{share}");
    }

    #[test]
    fn the_session_disk_figure_sums_the_files_and_leaves_the_cap_unknown() {
        assert_eq!(sessions_disk(&[]), None);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        std::fs::write(&a, "12345").unwrap();
        std::fs::write(&b, "12").unwrap();
        assert_eq!(
            sessions_disk(&[a, b, dir.path().join("gone")]),
            Some((7, 0))
        );
    }

    #[test]
    fn the_keys_sheet_counts_its_bindings_and_surfaces() {
        let mut m = model();
        open_keys_sheet(&mut m);
        assert_eq!(m.screen, Screen::Keys);
        assert_eq!(
            m.facts.keys_count,
            m.keymap.iter().map(|group| group.rows.len()).sum::<usize>()
        );
        assert!(
            !m.keymap
                .iter()
                .flat_map(|group| &group.rows)
                .any(|(_, label)| label.contains("thinking cycle")
                    || label.contains("cycle thinking"))
        );
        assert_eq!(m.facts.keys_surfaces, m.keymap.len());
        assert!(m.facts.keys_surfaces >= 3);
    }

    #[test]
    fn the_login_sheet_names_the_auth_file() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let mut m = model();
        open_login_sheet(&crate::args::Args::default(), &mut m);
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        assert_eq!(m.screen, Screen::Login);
        assert!(
            m.facts.auth_path.ends_with("auth.json"),
            "{}",
            m.facts.auth_path
        );
        assert_eq!(m.facts.auth_mode.is_empty(), !cfg!(unix));
    }

    #[test]
    fn detached_login_does_not_report_pending_oauth_as_signed_in() {
        let outcome = detached_login_message("anthropic", true);
        assert_eq!(
            outcome,
            Err("authorization required to sign in to anthropic".to_string())
        );
    }

    #[test]
    fn detached_openai_without_key_reports_api_key_requirement() {
        assert_eq!(
            detached_login_message("openai", true),
            Err("API key required for openai. Run /login openai <api-key>.".to_string())
        );
    }

    #[test]
    fn detached_login_reports_completed_login_as_signed_in() {
        assert_eq!(
            detached_login_message("anthropic", false),
            Ok("signed in to anthropic".to_string())
        );
    }

    fn assistant(text: &str) -> davinci_ai::ChatMessage {
        davinci_ai::ChatMessage {
            role: "assistant".into(),
            content: vec![davinci_ai::MessageContent::Text { text: text.into() }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn a_second_interrupt_while_one_is_pending_asks_for_the_way_out() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut m = model();
        let abort = Arc::new(AtomicBool::new(false));
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        // The first esc requests the abort and is not a force-quit.
        assert!(!mid_turn_key(&mut m, esc, &abort));
        assert!(abort.load(Ordering::Relaxed));
        // The second, with the flag already up, is: the worker is not
        // answering, and the only way left restores the terminal first.
        assert!(mid_turn_key(&mut m, esc, &abort));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(mid_turn_key(&mut m, ctrl_c, &abort));
    }

    #[test]
    fn an_interrupted_turn_opens_the_recovery_sheet_with_what_it_ran() {
        // The `6c` sheet is populated from the turn's own log; this pins the
        // log's shape so `end_tool` keeps feeding it.
        let mut m = model();
        let mut turn = Turn::default();
        turn.start_tool(&mut m, "call-1", "read", &json!({"path": "src/lib.rs"}));
        turn.end_tool(
            &mut m,
            "call-1",
            "read",
            &serde_json::Value::String("line".into()),
            false,
            None,
        );
        assert_eq!(turn.log.len(), 1);
        assert_eq!(turn.log[0].0, State::Read);
        assert!(turn.log[0].1.contains("read src/lib.rs"));
    }

    #[test]
    fn the_diff_command_is_findable_in_the_palette() {
        let agent = davinci_agent::Agent::new("test");
        let items = corpus(&agent, &[], &[]);
        assert!(items
            .iter()
            .any(|item| item.name == "/diff" && item.kind == "command"));
    }

    #[test]
    fn switching_models_resolves_unsupported_thinking_before_optional_confirmation() {
        use davinci_protocol::ThinkingLevel::*;
        assert_eq!(
            supported_thinking_choice(Max, &[Off, Low, Medium, High]),
            High
        );
        assert_eq!(
            supported_thinking_choice(Low, &[Off, Low, Medium, High]),
            Low
        );
        assert_eq!(supported_thinking_choice(High, &[Off]), Off);
        assert_eq!(supported_thinking_choice(Off, &[Low, Medium, High]), Low);
    }

    #[test]
    fn theme_setting_persists_and_recolors_the_current_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = model();
        m.composer = "keep this draft".into();
        let dark = m.theme;
        let mut row = SettingRow {
            key: "theme".into(),
            value: "dark".into(),
            values: vec!["dark".into(), "light".into()],
            ..SettingRow::default()
        };
        persist_setting_row(&mut row, |assignment| {
            assert_eq!(assignment, "theme=light");
            let stored = crate::settings::Settings {
                theme: Some("light".into()),
                ..Default::default()
            };
            crate::settings::save_settings(dir.path(), &stored)
        })
        .unwrap();
        let stored = crate::settings::load_settings(dir.path());
        apply_theme_setting(&mut m, &stored);
        assert_ne!(m.theme.background, dark.background);
        assert_eq!(m.composer, "keep this draft");
        let mut restarted = model();
        apply_theme_setting(&mut restarted, &stored);
        assert_eq!(restarted.theme, m.theme);
        assert_eq!(row.value, "light");
    }

    #[test]
    fn failed_setting_persistence_does_not_change_the_displayed_row() {
        let mut row = SettingRow {
            label: "Auto compact".into(),
            value: "on".into(),
            values: vec!["on".into(), "off".into()],
            project: true,
            key: "auto-compact".into(),
            ..SettingRow::default()
        };
        let result = persist_setting_row(&mut row, |_| Err("disk full".into()));
        assert_eq!(result.unwrap_err(), "disk full");
        assert_eq!(row.value, "on");
        assert!(row.project);
    }

    #[test]
    fn typesafe_key_clipboard_paste_stays_in_secret_input() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use davinci_tui::davinci::app::{self, Flow};
        let mut model = model();
        open_typesafe_key_input(&mut model);
        assert!(paste_secret_clipboard(
            &mut model,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            || Some("test_KEY".into()),
        ));
        assert_eq!(model.composer.to_string(), "");
        let Flow::SecretInputSubmitted(value) = app::handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ) else {
            panic!("credential was not submitted")
        };
        assert_eq!(value.into_inner(), "test_KEY");
        assert!(!paste_secret_clipboard(
            &mut model,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            || panic!("clipboard must not be read without secret overlay"),
        ));
    }

    #[test]
    fn opening_typesafe_key_replacement_uses_the_masked_secret_overlay() {
        let mut model = model();

        open_typesafe_key_input(&mut model);

        assert_eq!(model.overlay, Some(Overlay::SecretInput));
        assert!(model.secret_input.is_some());
        assert_eq!(model.composer.to_string(), "");
    }

    #[test]
    fn typesafe_key_replacement_respects_environment_and_project_overrides() {
        let enabled = SettingRow {
            key: "decision-intelligence".into(),
            value: "on".into(),
            ..SettingRow::default()
        };
        assert_eq!(typesafe_key_replacement_blocker(&[enabled], false), None);

        let project_disabled = SettingRow {
            key: "decision-intelligence".into(),
            value: "off".into(),
            project: true,
            ..SettingRow::default()
        };
        assert!(typesafe_key_replacement_blocker(&[project_disabled], false)
            .unwrap()
            .contains("project settings"));
        assert!(typesafe_key_replacement_blocker(&[], true)
            .unwrap()
            .contains("TYPESAFE_API_KEY"));
    }

    #[test]
    fn section_notices_are_only_attached_to_open_sheets() {
        let mut sheet = model();
        sheet.screen = Screen::Settings;
        attach_section_notice(&mut sheet, "could not save");
        assert_eq!(sheet.section_notice.as_deref(), Some("could not save"));

        let mut agent = model();
        agent.screen = Screen::Agent;
        attach_section_notice(&mut agent, "conversation warning");
        assert!(agent.section_notice.is_none());
    }

    #[test]
    fn opening_an_ask_overlay_resets_only_overlay_reading_state() {
        let mut m = model();
        m.section_offset = Some(5);
        m.overlay_offset = Some(8);
        m.ask_index = 3;
        open_ask_overlay(&mut m);
        assert_eq!(m.section_offset, Some(5));
        assert!(m.overlay_offset.is_none());
        assert_eq!(m.ask_index, 0);
        assert_eq!(m.overlay, Some(Overlay::Ask));
    }

    #[test]
    fn opening_a_sheet_clears_section_reading_state() {
        let mut m = model();
        m.section_offset = Some(9);
        m.overlay_offset = Some(7);
        m.section_notice = Some("stale notice".into());
        open_sheet(&mut m, Screen::Settings);
        assert!(m.section_offset.is_none());
        assert!(m.overlay_offset.is_none());
        assert!(m.section_notice.is_none());
    }

    fn model() -> Model {
        Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            40,
            true,
        )
    }

    #[test]
    fn shift_tab_cycle_commits_policy_prompt_and_status_without_submitting_the_draft() {
        let mut agent = Agent::new("base prompt");
        let mut m = model();
        change_permission_mode(&mut agent, &mut m, PermissionMode::Ask);
        m.composer.set_text("unfinished request");
        for expected in [
            PermissionMode::Edits,
            PermissionMode::ReadOnly,
            PermissionMode::Auto,
            PermissionMode::AlwaysApprove,
            PermissionMode::Ask,
        ] {
            let flow = davinci_tui::davinci::app::handle_key(
                &mut m,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::BackTab,
                    crossterm::event::KeyModifiers::SHIFT,
                ),
            );
            assert_eq!(flow, davinci_tui::davinci::app::Flow::CyclePermissionMode);
            cycle_permission_mode(&mut agent, &mut m);
            assert_eq!(agent.permission_mode(), expected);
            assert_eq!(m.permission_mode, expected.as_str());
            assert_eq!(agent.is_plan_mode(), expected == PermissionMode::ReadOnly);
            assert_eq!(
                agent
                    .system_prompt
                    .contains(davinci_agent::PLAN_MODE_APPENDIX),
                agent.is_plan_mode()
            );
            assert_eq!(m.composer.to_string(), "unfinished request");
        }
    }

    #[test]
    fn context_refresh_resynchronizes_mode_after_session_restore() {
        let mut agent = Agent::new("base prompt");
        let mut m = model();
        change_permission_mode(&mut agent, &mut m, PermissionMode::AlwaysApprove);
        // Session loading can choose Plan Mode without a keyboard event.
        agent.set_permission_mode(PermissionMode::ReadOnly);
        refresh_context(&mut m, &agent);
        assert_eq!(m.permission_label(), "Plan Mode");
        assert!(m.is_plan_mode());
    }

    #[test]
    fn switching_to_plan_from_always_approve_blocks_real_tool_permissions() {
        let mut agent = Agent::new("base prompt");
        let mut m = model();
        change_permission_mode(&mut agent, &mut m, PermissionMode::AlwaysApprove);
        change_permission_mode(&mut agent, &mut m, PermissionMode::ReadOnly);
        let policy = agent.permissions.lock().unwrap();
        let verdict = policy.decide(
            "plan-transition",
            "write",
            &serde_json::json!({"path":"src/new.rs", "content":"mutation"}),
            &agent.cwd,
        );
        assert!(matches!(
            verdict,
            davinci_agent::PermissionVerdict::Deny { .. }
        ));
        assert_eq!(m.permission_label(), "Plan Mode");
    }

    fn partial(text: &str) -> davinci_ai::AssistantMessage {
        davinci_ai::AssistantMessage {
            id: "m".into(),
            role: "assistant".into(),
            content: vec![davinci_ai::ContentBlock::Text { text: text.into() }],
            model: "fixture".into(),
            usage: None,
            stop_reason: None,
            error_message: None,
        }
    }

    fn update(event: davinci_ai::AssistantMessageEvent) -> AgentEvent {
        AgentEvent::MessageUpdate {
            message: Arc::new(davinci_ai::assistant_to_chat(event.message())),
            assistant_message_event: event,
        }
    }

    #[test]
    fn text_deltas_stream_into_one_prose_entry_that_message_end_keeps() {
        use davinci_ai::AssistantMessageEvent as Ev;
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::MessageStart {
                message: davinci_ai::ChatMessage::text("assistant", ""),
            },
        );
        apply(
            &mut m,
            &mut turn,
            &update(Ev::TextStart {
                content_index: 0,
                partial: partial(""),
            }),
        );
        apply(
            &mut m,
            &mut turn,
            &update(Ev::TextDelta {
                content_index: 0,
                delta: "Hel".into(),
                partial: partial("Hel"),
            }),
        );
        apply(
            &mut m,
            &mut turn,
            &update(Ev::TextDelta {
                content_index: 0,
                delta: "lo".into(),
                partial: partial("Hello"),
            }),
        );
        let prose: Vec<&Entry> = m
            .transcript
            .iter()
            .filter(|entry| matches!(entry, Entry::Prose(_)))
            .collect();
        assert_eq!(prose.len(), 1, "{:?}", m.transcript);
        assert!(matches!(prose[0], Entry::Prose(text) if text == "Hello"));

        apply(
            &mut m,
            &mut turn,
            &AgentEvent::MessageEnd {
                message: davinci_ai::ChatMessage::text("assistant", "Hello"),
            },
        );
        let prose: Vec<&Entry> = m
            .transcript
            .iter()
            .filter(|entry| matches!(entry, Entry::Prose(_)))
            .collect();
        assert_eq!(
            prose.len(),
            1,
            "message end must not repeat the streamed text"
        );
        assert!(turn.said_something);
    }

    #[test]
    fn a_message_that_never_streamed_still_lands_at_message_end() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::MessageEnd {
                message: davinci_ai::ChatMessage::text("assistant", "whole reply"),
            },
        );
        assert!(matches!(
            m.transcript.last(),
            Some(Entry::Prose(text)) if text == "whole reply"
        ));
    }

    #[test]
    fn reasoning_streams_live_and_collapses_when_the_text_starts() {
        use davinci_ai::AssistantMessageEvent as Ev;
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &update(Ev::ThinkingDelta {
                content_index: 0,
                delta: "Need the file first.".into(),
                partial: partial(""),
            }),
        );
        assert!(matches!(
            m.transcript.last(),
            Some(Entry::Thinking { text, live: true, .. }) if text == "Need the file first."
        ));
        apply(
            &mut m,
            &mut turn,
            &update(Ev::TextDelta {
                content_index: 1,
                delta: "Reading.".into(),
                partial: partial("Reading."),
            }),
        );
        let thinking = m
            .transcript
            .iter()
            .find(|entry| matches!(entry, Entry::Thinking { .. }))
            .expect("thinking row");
        assert!(matches!(thinking, Entry::Thinking { live: false, .. }));
        assert!(matches!(m.transcript.last(), Some(Entry::Prose(text)) if text == "Reading."));
    }

    #[test]
    fn hidden_reasoning_never_reaches_the_transcript() {
        use davinci_ai::AssistantMessageEvent as Ev;
        let mut m = model();
        let mut turn = Turn {
            hide_thinking: true,
            ..Turn::default()
        };
        apply(
            &mut m,
            &mut turn,
            &update(Ev::ThinkingDelta {
                content_index: 0,
                delta: "secret".into(),
                partial: partial(""),
            }),
        );
        assert!(!m
            .transcript
            .iter()
            .any(|entry| matches!(entry, Entry::Thinking { .. })));
    }

    #[test]
    fn the_catalogue_opens_with_newer_models_before_the_current_older_model() {
        let row = |provider: &str, id: &str, ready: bool| CatalogRow {
            name: format!("{provider}/{id}"),
            detail: String::new(),
            window: "200k".into(),
            thinking: "none".into(),
            price: "1.00 · 2.00".into(),
            credential: if ready {
                Credential::Ready
            } else {
                Credential::Absent
            },
            note: String::new(),
            ring: false,
            provider: provider.into(),
            id: id.into(),
            ..Default::default()
        };
        let mut catalog = vec![
            row("amazon-bedrock", "nova", false),
            row("anthropic", "claude", true),
            row("openai-codex", "gpt-5", true),
            row("openai-codex", "gpt-5-mini", true),
            row("openai-codex", "gpt-5.6-luna", true),
            row("openai-codex", "gpt-6-astra", true),
            row("xai", "grok", false),
        ];
        let index = order_catalog(&mut catalog, "openai-codex", "gpt-5-mini");
        assert_eq!(index, 0);
        let names: Vec<&str> = catalog.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "openai-codex/gpt-6-astra",
                "openai-codex/gpt-5.6-luna",
                "openai-codex/gpt-5",
                "openai-codex/gpt-5-mini",
                "anthropic/claude",
            ]
        );
    }

    #[test]
    fn what_the_session_found_is_said_under_the_mark_not_in_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let mut agent = davinci_agent::Agent::new("test");
        agent.cwd = dir.path().to_path_buf();
        agent.context_files.push(davinci_agent::ContextFile {
            path: dir.path().join("AGENTS.md"),
            name: "AGENTS.md".into(),
            body: "be kind".into(),
        });
        let found = opening_found(&crate::args::Args::default(), &agent);
        assert_eq!(found, ["loaded 1 context file"]);
        let block = opening_block(&crate::args::Args::default(), &agent, &[]);
        assert!(!block
            .iter()
            .any(|entry| matches!(entry, Entry::Prose(text) if text.starts_with("loaded"))));
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn f01_native_denial_editor_requires_confirmation_and_keeps_draft() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        for cancel in [false, true] {
            let request = approval("write", "ordinary.txt", false);
            let choices = request.host_choices(true);
            let mut m = model();
            m.width = 40;
            m.height = 12;
            m.composer.set_text("original cafe\u{301}");
            m.composer.editor_mut().move_left();
            let draft = m.composer.to_string();
            let cursor = m.composer.editor().get_cursor();
            m.ask = permission_ask(&request, true);
            open_ask_overlay(&mut m);
            let abort = Arc::new(AtomicBool::new(false));
            let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
            assert!(approval_answer_key(
                &mut m,
                press(KeyCode::Char('5')),
                &choices,
                &request,
                &abort
            )
            .is_none());
            assert!(m.approval_instructions.is_none());
            assert!(
                approval_answer_key(&mut m, press(KeyCode::Enter), &choices, &request, &abort)
                    .is_none()
            );
            assert!(m.approval_instructions.is_some());
            assert!(
                approval_answer_key(&mut m, press(KeyCode::Enter), &choices, &request, &abort)
                    .is_none()
            );
            for ch in "try cafe\u{301}".chars() {
                assert!(approval_answer_key(
                    &mut m,
                    press(KeyCode::Char(ch)),
                    &choices,
                    &request,
                    &abort
                )
                .is_none());
            }
            m.paste("must not enter either buffer");
            assert!(!m.insert_dictation("late voice", m.composer_epoch).unwrap());
            assert_eq!(
                m.approval_instructions.as_ref().unwrap().to_string(),
                "try cafe\u{301}"
            );
            let mut repeat = press(KeyCode::Enter);
            repeat.kind = KeyEventKind::Repeat;
            assert!(approval_answer_key(&mut m, repeat, &choices, &request, &abort).is_none());
            m.width = 120;
            m.height = 40;
            let rendered = davinci_tui::davinci::views::ask::lines(&m);
            assert!(rendered
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("try cafe")));
            let answer = approval_answer_key(
                &mut m,
                press(if cancel { KeyCode::Esc } else { KeyCode::Enter }),
                &choices,
                &request,
                &abort,
            )
            .unwrap();
            assert_eq!(answer.decision, ToolApprovalDecision::Deny);
            assert_eq!(
                answer.instructions.as_deref(),
                if cancel {
                    None
                } else {
                    Some("try cafe\u{301}")
                }
            );
            assert!(m.approval_instructions.is_none());
            assert!(m.overlay.is_none());
            assert_eq!(m.composer.to_string(), draft);
            assert_eq!(m.composer.editor().get_cursor(), cursor);
        }
    }

    #[test]
    fn f01_native_denial_editor_bounds_utf8_and_cancels() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let request = approval("write", "ordinary.txt", false);
        let choices = request.host_choices(true);
        let mut m = model();
        m.ask = permission_ask(&request, true);
        open_ask_overlay(&mut m);
        m.approval_instructions = Some("x".repeat(4095).into());
        let abort = Arc::new(AtomicBool::new(false));
        assert!(approval_answer_key(
            &mut m,
            KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
            &choices,
            &request,
            &abort
        )
        .is_none());
        assert_eq!(m.approval_instructions.as_ref().unwrap().len(), 4095);
        assert!(approval_answer_key(
            &mut m,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            &choices,
            &request,
            &abort
        )
        .is_none());
        assert_eq!(m.approval_instructions.as_ref().unwrap().len(), 4096);
        let answer = approval_answer_key(
            &mut m,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &choices,
            &request,
            &abort,
        )
        .unwrap();
        assert_eq!(answer, ToolApprovalDecision::Deny.into());
        assert!(abort.load(Ordering::Relaxed));
        assert!(m.approval_instructions.is_none());
        assert!(m.overlay.is_none());
    }

    #[test]
    fn f01_native_offers_policy_owned_denial_instructions() {
        let mut request = approval("write", "ordinary.txt", false);
        assert_eq!(
            permission_ask(&request, true).items.last().unwrap().label,
            "deny with instructions"
        );
        request.legal_choices.retain(|choice| {
            choice.scope != davinci_agent::approval::GrantScope::DenyWithInstructions
        });
        assert_eq!(
            permission_ask(&request, true).items.last().unwrap().label,
            "deny"
        );
    }

    #[test]
    fn f01_modal_numeric_focus_preserves_draft_until_enter() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for (width, height) in [(40, 12), (120, 40)] {
            let mut m = model();
            m.width = width;
            m.height = height;
            m.composer.set_text("keep cafe\u{301} draft");
            m.composer.editor_mut().move_left();
            let cursor = m.composer.editor().get_cursor();
            let original = m.composer.to_string();
            let permission = m.permission_mode.clone();
            let request = approval("write", "ordinary.txt", false);
            let choices = request.host_choices(true);
            m.ask = permission_ask(&request, true);
            open_ask_overlay(&mut m);
            let abort = Arc::new(AtomicBool::new(false));
            for key in [
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            ] {
                assert_eq!(approval_key(&mut m, key, &choices, &abort), None);
            }
            assert_eq!(m.ask_index, 1);
            m.keybindings = davinci_tui::Keybindings::from_json(r#"{"tui.select.confirm":"x"}"#);
            assert_eq!(
                approval_key(
                    &mut m,
                    KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                    &choices,
                    &abort
                ),
                None
            );
            m.paste("\r\npasted instruction");
            assert!(!m
                .insert_dictation("voice instruction", m.composer_epoch)
                .unwrap());
            assert_eq!(m.composer.to_string(), original);
            assert_eq!(m.composer.editor().get_cursor(), cursor);
            assert_eq!(m.permission_mode, permission);
            assert_eq!(
                approval_key(
                    &mut m,
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    &choices,
                    &abort
                ),
                Some(ToolApprovalDecision::AllowForSession)
            );
            assert_eq!(m.composer.to_string(), original);
            assert_eq!(m.composer.editor().get_cursor(), cursor);
            open_ask_overlay(&mut m);
            assert_eq!(
                approval_key(
                    &mut m,
                    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                    &choices,
                    &abort
                ),
                Some(ToolApprovalDecision::Deny)
            );
            assert_eq!(m.composer.to_string(), original);
            assert_eq!(m.composer.editor().get_cursor(), cursor);
        }
    }

    #[test]
    fn f01_native_approval_wait_releases_on_cancel_expiry_and_disconnect() {
        for case in [
            "allow",
            "instructions",
            "cancel",
            "expired",
            "disconnect",
            "ui_error",
            "expires_waiting",
        ] {
            let (tx, rx) = mpsc::channel();
            let abort = Arc::new(AtomicBool::new(case == "cancel"));
            let expires = if case == "expired" {
                0
            } else if case == "expires_waiting" {
                davinci_session::now_ms() + 250
            } else {
                davinci_session::now_ms() + 60_000
            };
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| {
                    wait_native_approval(
                        &approval("write", "ordinary.txt", false),
                        expires,
                        &tx,
                        &abort,
                    )
                });
                if matches!(
                    case,
                    "allow" | "instructions" | "ui_error" | "expires_waiting"
                ) {
                    let pending = rx.recv_timeout(Duration::from_secs(1)).unwrap();
                    if case == "allow" {
                        pending
                            .reply
                            .send(ToolApprovalDecision::AllowOnce.into())
                            .unwrap();
                    } else if case == "instructions" {
                        pending
                            .reply
                            .send(NativeApprovalAnswer {
                                decision: ToolApprovalDecision::Deny,
                                instructions: Some("read the docs first".into()),
                            })
                            .unwrap();
                    } else if case == "ui_error" {
                        let _exit = ApprovalUiExit::new(abort.clone());
                    }
                    let start = Instant::now();
                    while !worker.is_finished() && start.elapsed() < Duration::from_secs(1) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(worker.is_finished());
                    assert!(!pending.live.load(Ordering::Relaxed));
                }
                drop(rx);
                let answer = worker.join().unwrap();
                assert_eq!(
                    answer.decision,
                    if case == "allow" {
                        ToolApprovalDecision::AllowOnce
                    } else {
                        ToolApprovalDecision::Deny
                    }
                );
                let challenge: davinci_agent::approval::ApprovalChallenge = serde_json::from_value(json!({
                    "schema_version": 1, "id": "00000000-0000-0000-0000-000000000001", "call_id": "fixture",
                    "action_digest": "fixture", "policy_revision": 0, "contract_revision": null,
                    "mode": "ask", "action_label": "write", "display_target": "ordinary.txt", "reason": "fixture",
                    "legal_choices": [], "expires_at_ms": expires
                })).unwrap();
                let reply = answer.into_reply(&challenge);
                assert_eq!(reply.challenge_id, challenge.id);
                assert_eq!(
                    reply.choice_id,
                    if case == "instructions" {
                        "deny_with_instructions"
                    } else if case == "allow" {
                        "once"
                    } else {
                        "deny"
                    }
                );
                assert_eq!(
                    reply.instructions.as_deref(),
                    if case == "instructions" {
                        Some("read the docs first")
                    } else {
                        None
                    }
                );
            });
        }
    }

    fn approval(tool: &str, subject: &str, outside: bool) -> ToolApprovalRequest {
        ToolApprovalRequest {
            legal_choices: davinci_agent::approval::offer_scopes(false, true, true),
            tool_call_id: "call_1".into(),
            tool: tool.into(),
            args: json!({}),
            subject: subject.into(),
            summary: davinci_agent::summary_of(tool, subject),
            session_rule: davinci_agent::session_rule_for(tool, subject).to_string(),
            outside_project: outside,
            mode: PermissionMode::Ask,
        }
    }

    #[test]
    fn the_permission_panel_offers_always_only_in_a_trusted_project() {
        let ask = permission_ask(&approval("bash", "git status --short", false), true);
        assert_eq!(ask.title, "Permission");
        assert_eq!(ask.name, "PERMISSION");
        assert_eq!(ask.key, "/permissions");
        assert_eq!(ask.note, "bash · git status --short");
        let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "allow once",
                "allow for this session",
                "always allow here",
                "deny",
                "deny with instructions"
            ]
        );
        assert_eq!(ask.items[1].detail, "bash(git status *) until pi exits");
        assert_eq!(
            ask.items[2].detail,
            "bash(git status *) saved to .pi/settings.json"
        );

        let ask = permission_ask(&approval("write", "../out.txt", true), false);
        let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "allow once",
                "allow for this session",
                "deny",
                "deny with instructions"
            ]
        );
        assert_eq!(ask.note, "write · ../out.txt · outside the project");
    }

    #[test]
    fn f05_scope_expansion_panel_has_only_host_authority_choices() {
        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-panel",
            7,
            davinci_agent::TaskId::new(),
            3,
            vec!["src/".into()],
            vec!["secrets/".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = contract
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        let ask = scope_expansion_ask(&preview);
        assert_eq!(ask.title, "Scope expansion required");
        assert_eq!(ask.key, "/scope-expansion");
        let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "approve expansion",
                "request in-scope approach",
                "deny with instructions"
            ]
        );
        assert!(ask.note.contains("migrations/001.sql"));
        assert!(ask.note.contains("revision 7 → 8"));
        assert!(ask.note.contains(&preview.preview_digest));
        assert_eq!(
            scope_expansion_choice(0),
            Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::Approve)
        );
        assert_eq!(
            scope_expansion_choice(1),
            Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::RequestInScopeApproach)
        );
        assert_eq!(
            scope_expansion_choice(2),
            Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny)
        );
        assert_eq!(scope_expansion_choice(3), None);
    }

    #[test]
    fn f05_scope_expansion_deny_collects_bounded_host_instructions() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-keys",
            1,
            davinci_agent::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = contract
            .preview_scope_expansion("outside.rs", "blocked write")
            .unwrap();
        let mut m = model();
        m.ask = scope_expansion_ask(&preview);
        open_ask_overlay(&mut m);
        m.ask_index = 2;
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);

        assert!(scope_expansion_answer_key(&mut m, press(KeyCode::Enter)).is_none());
        assert!(m.approval_instructions.is_some());
        for ch in "stay in src".chars() {
            assert!(scope_expansion_answer_key(&mut m, press(KeyCode::Char(ch))).is_none());
        }
        let answer = scope_expansion_answer_key(&mut m, press(KeyCode::Enter)).unwrap();
        assert_eq!(
            answer.0,
            davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny
        );
        assert_eq!(answer.1.as_deref(), Some("stay in src"));
        assert!(m.overlay.is_none());
    }

    #[test]
    fn f05_scope_expansion_commit_is_durable_exact_and_invalidates_session_grants() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        agent.session =
            Some(davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap());
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-commit",
            4,
            davinci_agent::TaskId::new(),
            2,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        agent.set_active_contract(current.clone());
        agent.permissions.lock().unwrap().remember("write:**");
        assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());

        commit_scope_expansion(&mut agent, &preview, None).unwrap();

        assert_eq!(agent.active_contract(), Some(preview.proposed.clone()));
        assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
        let session = agent.session.as_ref().unwrap();
        let entry = session.entries.last().expect("durable audit entry");
        assert_eq!(
            entry.custom_type.as_deref(),
            Some("task_contract_scope_expansion")
        );
        assert_eq!(
            entry.extra["data"]["preview_digest"],
            preview.preview_digest
        );
        assert_eq!(
            entry.extra["data"]["proposed"]["digest"],
            preview.proposed.digest
        );
    }

    #[test]
    fn f05_scope_expansion_commit_updates_bound_task_digest_before_live_swap() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        agent
            .load_from_session(
                davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
            )
            .unwrap();
        let runtime = agent.runtime_for_session().unwrap().clone();
        let task_id = davinci_agent::TaskId::new();
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-bound",
            5,
            task_id,
            2,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
            .with_assigned(runtime.agent_id)
            .with_contract_digest(current.digest.clone());
        task.id = task_id;
        runtime.task_registry.create_task(task).unwrap();
        agent.set_active_contract(current.clone());
        let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();

        commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap();

        let rebound = runtime.task_registry.get_task(&task_id).unwrap();
        assert_eq!(
            rebound.contract_digest.as_deref(),
            Some(preview.proposed.digest.as_str())
        );
        assert_eq!(agent.active_contract(), Some(preview.proposed));
    }

    #[test]
    fn f05_scope_expansion_rejects_task_revision_changed_after_preview() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        agent
            .load_from_session(
                davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
            )
            .unwrap();
        let runtime = agent.runtime_for_session().unwrap().clone();
        let task_id = davinci_agent::TaskId::new();
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-bound-stale",
            5,
            task_id,
            2,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
            .with_assigned(runtime.agent_id)
            .with_contract_digest(current.digest.clone());
        task.id = task_id;
        runtime.task_registry.create_task(task).unwrap();
        agent.set_active_contract(current.clone());
        agent.permissions.lock().unwrap().remember("write:**");
        let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();
        runtime
            .task_registry
            .assign_task(task_id, runtime.agent_id)
            .unwrap();
        let changed = runtime.task_registry.get_task(&task_id).unwrap();

        let error = commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap_err();

        assert!(error.contains("bound task contract digest"));
        assert_eq!(agent.active_contract(), Some(current));
        assert_eq!(
            runtime
                .task_registry
                .get_task(&task_id)
                .unwrap()
                .contract_digest,
            changed.contract_digest
        );
        assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());
    }

    #[test]
    fn f05_scope_expansion_bound_task_update_failure_keeps_live_contract_and_grants() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        agent
            .load_from_session(
                davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
            )
            .unwrap();
        let runtime = agent.runtime_for_session().unwrap().clone();
        let task_id = davinci_agent::TaskId::new();
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-bound-fail",
            5,
            task_id,
            2,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        let other_agent = davinci_agent::AgentId::new();
        let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
            .with_assigned(other_agent)
            .with_contract_digest(current.digest.clone());
        task.id = task_id;
        runtime.task_registry.create_task(task).unwrap();
        agent.set_active_contract(current.clone());
        agent.permissions.lock().unwrap().remember("write:**");
        let before = runtime.task_registry.get_task(&task_id).unwrap();
        let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();

        let error = commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap_err();

        assert!(error.contains("bound task contract digest"));
        assert_eq!(agent.active_contract(), Some(current));
        assert_eq!(runtime.task_registry.get_task(&task_id), Some(before));
        assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());
        let audit = agent.session.as_ref().unwrap().entries.last().unwrap();
        assert_eq!(
            audit.custom_type.as_deref(),
            Some("task_contract_scope_expansion")
        );
    }

    #[test]
    fn f05_scope_expansion_storage_failure_and_stale_preview_keep_current_contract() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        let mut session =
            davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        session.path = dir.path().join("missing-parent/session.jsonl");
        agent.session = Some(session);
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-fail",
            9,
            davinci_agent::TaskId::new(),
            2,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("migrations/001.sql", "write migration")
            .unwrap();
        agent.set_active_contract(current.clone());
        let prior_leaf = agent.session.as_ref().unwrap().leaf_id.clone();

        assert!(commit_scope_expansion(&mut agent, &preview, None).is_err());
        assert_eq!(agent.active_contract(), Some(current.clone()));
        assert_eq!(agent.session.as_ref().unwrap().leaf_id, prior_leaf);

        let newer = current.expand_scope(vec!["docs/".into()], vec![]).unwrap();
        agent.set_active_contract(newer.clone());
        assert!(commit_scope_expansion(&mut agent, &preview, None).is_err());
        assert_eq!(agent.active_contract(), Some(newer));
    }

    #[test]
    fn f05_scope_expansion_reject_and_in_scope_guidance_keep_contract_unchanged() {
        let current = davinci_agent::runtime::contracts::TaskContract::new(
            "scope-reject",
            3,
            davinci_agent::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = current
            .preview_scope_expansion("outside/file.rs", "requested write")
            .unwrap();

        let mut always_agent = Agent::new("fixture");
        always_agent.set_active_contract(current.clone());
        always_agent.set_permission_mode(PermissionMode::AlwaysApprove);
        assert_eq!(always_agent.active_contract(), Some(current.clone()));

        for (decision, instructions) in [
            (
                davinci_agent::runtime::contracts::ScopeExpansionDecision::RequestInScopeApproach,
                None,
            ),
            (
                davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny,
                Some("do not modify generated files".to_string()),
            ),
        ] {
            let mut agent = Agent::new("fixture");
            agent.set_active_contract(current.clone());
            let guidance = apply_scope_expansion_decision(
                &mut agent,
                &preview,
                None,
                decision,
                instructions.as_deref(),
            )
            .unwrap()
            .expect("non-approval guidance");
            assert_eq!(agent.active_contract(), Some(current.clone()));
            assert!(guidance.contains("outside/file.rs"));
            if let Some(instructions) = instructions.as_deref() {
                assert!(guidance.contains(instructions));
            }
        }
    }

    #[test]
    fn f09_watchdog_ask_and_choice_navigation() {
        use davinci_agent::runtime::progress_watchdog::LoopSignal;
        let signal = LoopSignal::RepeatedTestFailureWithoutNewDiagnosis {
            signature: "test_foo failed: assertion failed".into(),
            count: 3,
        };
        let ask = watchdog_ask(&signal);
        assert_eq!(ask.title, "Progress watchdog");
        assert_eq!(ask.name, "WATCHDOG");
        assert_eq!(ask.key, "/watchdog");
        assert_eq!(ask.items.len(), 3);
        assert_eq!(watchdog_choice(0), Some("continue"));
        assert_eq!(watchdog_choice(1), Some("plan"));
        assert_eq!(watchdog_choice(2), Some("stop"));
        assert_eq!(watchdog_choice(3), None);

        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            80,
            24,
            false,
        );
        model.ask = ask;
        model.ask_index = 0;
        model.overlay = Some(Overlay::Ask);

        // Press '1' -> continue
        assert_eq!(
            watchdog_answer_key(
                &mut model,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('1'),
                    crossterm::event::KeyModifiers::empty(),
                ),
            ),
            Some("continue")
        );

        // Press '2' -> plan
        assert_eq!(
            watchdog_answer_key(
                &mut model,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('2'),
                    crossterm::event::KeyModifiers::empty(),
                ),
            ),
            Some("plan")
        );

        // Press '3' -> stop
        assert_eq!(
            watchdog_answer_key(
                &mut model,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('3'),
                    crossterm::event::KeyModifiers::empty(),
                ),
            ),
            Some("stop")
        );

        // Press Esc -> stop
        assert_eq!(
            watchdog_answer_key(
                &mut model,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Esc,
                    crossterm::event::KeyModifiers::empty(),
                ),
            ),
            Some("stop")
        );

        // Test apply_watchdog_choice transitions
        let mut agent = Agent::new("test_agent");
        let res_continue = apply_watchdog_choice(&mut agent, "continue", &signal);
        assert!(res_continue.is_ok());

        let res_plan = apply_watchdog_choice(&mut agent, "plan", &signal);
        assert!(res_plan.is_ok());
        assert_eq!(
            agent.permission_mode(),
            davinci_agent::PermissionMode::ReadOnly
        );

        let res_stop = apply_watchdog_choice(&mut agent, "stop", &signal);
        assert!(res_stop.is_ok());
    }

    #[test]
    fn interrupted_recovery_copy_avoids_unverified_guarantees() {
        let rows = interrupted_recovery_aftermath();
        let copy = rows
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(copy.contains("interrupted"));
        assert!(!copy.contains("nothing to recover"));
        assert!(!copy.contains("transcript written"));
        assert!(!copy.contains("abort was delivered"));
        assert!(!copy.contains("esc esc"));
    }

    #[test]
    fn permission_panel_uses_policy_choices_even_in_a_trusted_project() {
        let dir = tempfile::tempdir().unwrap();
        let policy = davinci_agent::PermissionPolicy::new(PermissionMode::Ask);
        let davinci_agent::PermissionVerdict::Ask(request) = policy.decide(
            "risky",
            "write",
            &json!({"path":".env", "content":"fixture"}),
            dir.path(),
        ) else {
            panic!("expected Ask")
        };
        let mut m = model();
        m.ask = permission_ask(&request, true);
        assert_eq!(
            m.ask
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["allow once", "deny", "deny with instructions"]
        );
        open_ask_overlay(&mut m);
        m.ask_index = 1;
        let abort = Arc::new(AtomicBool::new(false));
        assert_eq!(
            approval_key(
                &mut m,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE
                ),
                &request.host_choices(true),
                &abort
            ),
            Some(ToolApprovalDecision::Deny)
        );
        let mut trusted = true;
        assert_eq!(
            persist_permission_choice(
                &mut m,
                &request,
                ToolApprovalDecision::AllowAlways,
                dir.path(),
                &mut trusted
            ),
            Some(ToolApprovalDecision::Deny)
        );
        assert!(!dir.path().join(".davinci/settings.json").exists());
    }

    #[test]
    fn permission_save_failure_reopens_panel_without_granting() {
        use ToolApprovalDecision::*;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".davinci"), "not a directory").unwrap();
        let request = approval("write", "file.txt", false);
        let mut m = model();
        let mut project_allowed = true;
        assert_eq!(
            persist_permission_choice(
                &mut m,
                &request,
                AllowAlways,
                dir.path(),
                &mut project_allowed
            ),
            None
        );
        assert!(!project_allowed);
        assert!(m.ask.note.contains("could not be saved"));
        assert_eq!(m.ask.items.len(), 4);
        let abort = Arc::new(AtomicBool::new(false));
        let decision = approval_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
            &request.host_choices(project_allowed),
            &abort,
        );
        assert_eq!(decision, Some(Deny));
        assert_eq!(
            persist_permission_choice(&mut m, &request, Deny, dir.path(), &mut project_allowed),
            Some(Deny)
        );
        for (index, expected) in [(0, AllowOnce), (1, AllowForSession), (2, Deny)] {
            project_allowed = true;
            assert_eq!(
                persist_permission_choice(
                    &mut m,
                    &request,
                    AllowAlways,
                    dir.path(),
                    &mut project_allowed
                ),
                None
            );
            assert!(matches!(m.overlay, Some(Overlay::Ask)));
            m.ask_index = index;
            let choice = approval_key(
                &mut m,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                &request.host_choices(project_allowed),
                &abort,
            )
            .unwrap();
            assert_eq!(
                persist_permission_choice(
                    &mut m,
                    &request,
                    choice,
                    dir.path(),
                    &mut project_allowed
                ),
                Some(expected)
            );
        }
        assert_eq!(
            persist_permission_choice(
                &mut m,
                &request,
                AllowAlways,
                dir.path(),
                &mut project_allowed
            ),
            Some(Deny)
        );
        let saved = tempfile::tempdir().unwrap();
        project_allowed = true;
        assert_eq!(
            persist_permission_choice(
                &mut m,
                &request,
                AllowAlways,
                saved.path(),
                &mut project_allowed
            ),
            Some(AllowAlways)
        );
        let settings: serde_json::Value = serde_json::from_slice(
            &std::fs::read(saved.path().join(".davinci/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings["permissions"]["allow"],
            json!([request.session_rule])
        );
    }

    #[test]
    fn permission_rows_map_to_decisions_in_both_shapes() {
        use ToolApprovalDecision::*;
        let request = approval("bash", "git status", false);
        let trusted = request.host_choices(true);
        let untrusted = request.host_choices(false);
        assert_eq!(permission_choice(0, &trusted), Some(AllowOnce));
        assert_eq!(permission_choice(1, &trusted), Some(AllowForSession));
        assert_eq!(permission_choice(2, &trusted), Some(AllowAlways));
        assert_eq!(permission_choice(3, &trusted), Some(Deny));
        assert_eq!(permission_choice(4, &trusted), None);
        assert_eq!(permission_choice(2, &untrusted), Some(Deny));
        assert_eq!(permission_choice(3, &untrusted), None);
    }

    #[test]
    fn under_the_permission_panel_esc_denies_and_enter_chooses_the_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
        let abort = Arc::new(AtomicBool::new(false));
        let choices = approval("bash", "git status", false).host_choices(true);
        let open = || {
            let mut m = model();
            m.running = true;
            m.ask = permission_ask(&approval("bash", "git status", false), true);
            m.ask_index = 0;
            m.overlay = Some(Overlay::Ask);
            m
        };

        let mut m = open();
        assert_eq!(
            approval_key(&mut m, key(KeyCode::Esc), &choices, &abort),
            Some(ToolApprovalDecision::Deny)
        );
        assert_eq!(m.overlay, None);
        assert!(
            !abort.load(Ordering::Relaxed),
            "esc refuses; it does not interrupt"
        );

        let mut m = open();
        assert_eq!(
            approval_key(&mut m, key(KeyCode::Down), &choices, &abort),
            None
        );
        assert_eq!(
            m.overlay,
            Some(Overlay::Ask),
            "moving keeps the question up"
        );
        assert_eq!(
            approval_key(&mut m, key(KeyCode::Enter), &choices, &abort),
            Some(ToolApprovalDecision::AllowForSession)
        );
        assert_eq!(m.overlay, None);

        let mut m = open();
        assert_eq!(
            approval_key(
                &mut m,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &choices,
                &abort
            ),
            Some(ToolApprovalDecision::Deny)
        );
        assert!(
            abort.load(Ordering::Relaxed),
            "ctrl+c interrupts the turn as well"
        );
    }

    #[test]
    fn a_waiting_call_says_so_on_its_ledger_row_and_is_quiet_again_once_answered() {
        let mut m = model();
        let mut turn = Turn::default();
        turn.start_tool(&mut m, "call_1", "bash", &json!({"command": "git status"}));
        let request = approval("bash", "git status", false);
        turn.await_approval(&mut m, &request);
        let studio_target = |m: &Model| {
            m.transcript
                .iter()
                .find_map(|entry| match entry {
                    Entry::Studio(steps) => steps.last().and_then(|step| step.target.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        let tool_summary = |m: &Model| {
            m.transcript
                .iter()
                .find_map(|entry| match entry {
                    Entry::Tool { summary, .. } => Some(summary.clone()),
                    _ => None,
                })
                .flatten()
        };
        assert!(
            studio_target(&m).ends_with(" · awaiting approval"),
            "{}",
            studio_target(&m)
        );
        assert_eq!(tool_summary(&m).as_deref(), Some("awaiting approval"));

        turn.settle_approval(&mut m, &request, Some("bash(git status *)"));
        assert!(
            !studio_target(&m).contains("awaiting"),
            "{}",
            studio_target(&m)
        );
        assert_eq!(tool_summary(&m), None);
        assert!(matches!(
            m.transcript.last(),
            Some(Entry::Tool { target, .. }) if target == "remembered bash(git status *) · .pi/settings.json"
        ));
    }

    #[test]
    fn shell_is_manus_and_everything_else_is_instrumenta() {
        assert_eq!(instrument_of("bash"), "manus");
        assert_eq!(instrument_of("powershell"), "manus");
        assert_eq!(instrument_of("read"), "instrumenta");
        assert_eq!(instrument_of("grep"), "instrumenta");
        assert_eq!(instrument_of("memory_search"), "memoria");
        assert_eq!(instrument_of("graph_impact"), "grafo");
    }

    #[test]
    fn every_tool_gets_a_glyph_and_a_failure_overrides_it() {
        assert_eq!(state_of("read", false), State::Read);
        assert_eq!(state_of("grep", false), State::Search);
        assert_eq!(state_of("edit", false), State::Delta);
        assert_eq!(state_of("bash", false), State::Done);
        for tool in ["read", "grep", "edit", "bash"] {
            assert_eq!(state_of(tool, true), State::Failed, "{tool}");
        }
    }

    #[test]
    fn targets_read_as_the_mockups_do() {
        assert_eq!(
            target_of("read", &json!({"path": "crates/davinci-tui/src/lib.rs"})),
            "read crates/davinci-tui/src/lib.rs"
        );
        assert_eq!(
            target_of("grep", &json!({"pattern": "SessionManager"})),
            "search \"SessionManager\""
        );
        assert_eq!(
            target_of("bash", &json!({"command": "cargo test -p pi-session"})),
            "cargo test -p pi-session"
        );
        assert_eq!(target_of("read", &json!({})), "read ");
    }

    #[test]
    fn a_long_target_is_clipped_and_marked() {
        let long = "x".repeat(200);
        let drawn = target_of("bash", &json!({ "command": long }));
        assert!(drawn.chars().count() <= 61, "{}", drawn.chars().count());
        assert!(drawn.ends_with('…'));
    }

    #[test]
    fn only_the_first_line_of_a_multiline_command_is_shown() {
        let drawn = target_of("bash", &json!({"command": "cargo fmt\ncargo clippy"}));
        assert_eq!(drawn, "cargo fmt");
    }

    #[test]
    fn a_failure_keeps_at_most_four_lines() {
        let body = (0..12)
            .map(|i| format!("frame {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = failure_lines(&json!(body));
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "frame 0");

        let structured = json!({"output": "error[E0308] mismatched types\nstore.rs:118"});
        assert_eq!(
            failure_lines(&structured),
            vec![
                "error[E0308] mismatched types".to_string(),
                "store.rs:118".to_string()
            ]
        );
    }

    #[test]
    fn a_tool_call_becomes_one_line_and_gains_its_duration() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                args: json!({"command": "cargo fmt"}),
            },
        );
        match &m.transcript[0] {
            Entry::Tool {
                state,
                instrument,
                target,
                duration,
                ..
            } => {
                assert_eq!(*state, State::Done);
                assert_eq!(instrument, "manus");
                assert_eq!(target, "cargo fmt");
                assert!(duration.is_none(), "no duration while it runs");
            }
            other => panic!("{other:?}"),
        }

        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionEnd {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                result: json!("ok"),
                is_error: false,
                details: None,
            },
        );
        match &m.transcript[0] {
            Entry::Tool { duration, .. } => {
                assert!(duration.as_deref().unwrap().ends_with('s'))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_denied_call_keeps_the_done_glyph() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                args: json!({"command": "rm -rf /"}),
            },
        );
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionEnd {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                result: json!("Permission denied: `bash · rm -rf /` matches the deny rule `bash`."),
                is_error: true,
                details: None,
            },
        );
        match &m.transcript[0] {
            Entry::Tool {
                state: State::Done,
                summary,
                ..
            } => assert_eq!(summary.as_deref(), Some("denied")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_failing_tool_keeps_its_glyph_and_its_detail_directly_beneath() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                args: json!({"command": "cargo test"}),
            },
        );
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionEnd {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                result: json!("error[E0308] mismatched types\nstore.rs:118"),
                is_error: true,
                details: None,
            },
        );

        match &m.transcript[0] {
            Entry::Tool {
                state: State::Failed,
                output,
                ..
            } => {
                assert!(
                    output.iter().any(|line| line.contains("E0308")),
                    "{output:?}"
                );
                assert!(
                    output.iter().any(|line| line.contains("store.rs:118")),
                    "{output:?}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            !m.transcript
                .iter()
                .any(|entry| matches!(entry, Entry::Detail(_))),
            "the failure rides on the tool line, not as Detail siblings: {:?}",
            m.transcript
        );
    }

    #[test]
    fn concurrent_tools_each_find_their_own_line() {
        let mut m = model();
        let mut turn = Turn::default();
        for (id, command) in [("1", "cargo fmt"), ("2", "cargo clippy")] {
            apply(
                &mut m,
                &mut turn,
                &AgentEvent::ToolExecutionStart {
                    tool_call_id: id.into(),
                    tool_name: "bash".into(),
                    args: json!({ "command": command }),
                },
            );
        }
        // The second one finishes first.
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionEnd {
                tool_call_id: "2".into(),
                tool_name: "bash".into(),
                result: json!("ok"),
                is_error: false,
                details: None,
            },
        );

        let durations: Vec<bool> = m
            .transcript
            .iter()
            .filter_map(|entry| match entry {
                Entry::Tool { duration, .. } => Some(duration.is_some()),
                _ => None,
            })
            .collect();
        assert_eq!(durations, vec![false, true], "the right line was closed");
    }

    #[test]
    fn the_studio_ledger_tracks_the_step_in_hand() {
        let mut m = model();
        let mut turn = Turn::default();
        for (id, tool, args) in [
            ("1", "read", json!({"path": "store.rs"})),
            ("2", "bash", json!({"command": "cargo test"})),
        ] {
            apply(
                &mut m,
                &mut turn,
                &AgentEvent::ToolExecutionStart {
                    tool_call_id: id.into(),
                    tool_name: tool.into(),
                    args,
                },
            );
        }

        let studio = m
            .transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Studio(steps) => Some(steps.clone()),
                _ => None,
            })
            .expect("a ledger");
        assert_eq!(studio.len(), 2);
        assert_eq!(studio[0].state, State::Done, "the earlier step is settled");
        assert_eq!(studio[1].state, State::Active);
        assert_eq!(studio[0].verb, "studying");
        assert_eq!(studio[1].verb, "testing");
    }

    #[test]
    fn an_interrupted_turn_marks_its_open_step_skipped_rather_than_done() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "bash".into(),
                args: json!({"command": "cargo test"}),
            },
        );
        turn.close(&mut m, true);

        let studio = m
            .transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Studio(steps) => Some(steps.clone()),
                _ => None,
            })
            .expect("a ledger");
        assert_eq!(studio[0].state, State::Skipped);
    }

    #[test]
    fn an_assistant_reply_becomes_prose_after_a_gap() {
        let mut m = model();
        let mut turn = Turn::default();
        let message = assistant("A request enters the agent as a Turn.");
        apply(&mut m, &mut turn, &AgentEvent::MessageEnd { message });

        assert!(matches!(m.transcript[0], Entry::Gap));
        assert!(matches!(&m.transcript[1], Entry::Prose(text) if text.starts_with("A request")));
        assert!(turn.said_something);
    }

    #[test]
    fn an_empty_reply_is_not_pushed() {
        let mut m = model();
        let mut turn = Turn::default();
        let message = assistant("   \n ");
        apply(&mut m, &mut turn, &AgentEvent::MessageEnd { message });
        assert!(m.transcript.is_empty());
        assert!(!turn.said_something);
    }

    #[test]
    fn a_plain_line_is_a_prompt_and_slash_quit_leaves() {
        assert!(
            matches!(classify("explain the runtime"), Sent::Prompt(text) if text == "explain the runtime")
        );
        assert!(matches!(classify("/quit"), Sent::Quit));
        assert!(matches!(classify("/exit"), Sent::Quit));
    }

    #[test]
    fn commands_with_a_home_reach_perform_which_opens_their_sheet() {
        use crate::slash::SlashAction;
        // Each of these opens its designed sheet (screens 3a–4a) from
        // `perform`, with live data behind it, rather than a bare overlay.
        assert!(matches!(
            classify("/model"),
            Sent::Command(SlashAction::OpenModel)
        ));
        assert!(matches!(
            classify("/resume"),
            Sent::Command(SlashAction::Resume)
        ));
        assert!(matches!(
            classify("/settings"),
            Sent::Command(SlashAction::Settings)
        ));
        assert!(matches!(
            classify("/hotkeys"),
            Sent::Command(SlashAction::Hotkeys)
        ));
    }

    #[test]
    fn help_answers_without_asking_the_model() {
        assert!(matches!(classify("/help"), Sent::Say(text) if text.contains('/')));
    }

    #[test]
    fn init_starts_a_repository_analysis_prompt() {
        assert!(
            matches!(classify("/init"), Sent::Prompt(text) if text.contains("AGENTS.md") && !text.starts_with('/'))
        );
    }

    #[test]
    fn thinking_command_reaches_the_agent_for_every_supported_level() {
        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            let command = format!("/thinking {level}");
            assert!(matches!(
                classify(&command),
                Sent::Command(crate::slash::SlashAction::SetThinking(value)) if value == level
            ));
        }
        assert!(crate::slash::builtin_slash_commands()
            .iter()
            .any(|command| command.name == "thinking"));
    }

    #[test]
    fn every_builtin_command_reaches_the_agent_rather_than_the_model() {
        use crate::slash::SlashAction;
        // Nothing a `/` line can parse to may fall through to the model as
        // prose: either an instrument opens, or the agent carries it out.
        for line in [
            "/compact",
            "/new",
            "/export report.html",
            "/name work",
            "/fork",
            "/clone",
            "/copy",
            "/reload",
            "/import a.jsonl",
            "/share",
            "/session",
            "/thinking high",
            "/mcp",
            "/cost",
            "/status",
            "/agents",
            "/logout",
            "/login openai",
            "/tree",
        ] {
            match classify(line) {
                Sent::Command(_) => {}
                _ => panic!("{line} did not reach the agent"),
            }
        }
        assert!(matches!(
            classify("/export report.html"),
            Sent::Command(SlashAction::Export(Some(path))) if path == "report.html"
        ));
    }

    #[test]
    fn recall_searches_for_what_is_typed_and_falls_back_to_the_last_ask() {
        let mut m = model();
        let mut agent = davinci_agent::Agent::new("test");
        assert_eq!(recall_query(&m, &agent), "", "nothing typed, nothing asked");

        agent.messages.push(davinci_ai::ChatMessage {
            role: "user".into(),
            content: vec![davinci_ai::MessageContent::Text {
                text: "how does the session store work".into(),
            }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            extra: Default::default(),
        });
        agent.messages.push(assistant("it appends to a jsonl file"));
        assert_eq!(recall_query(&m, &agent), "how does the session store work");

        m.composer = "  branch cache  ".into();
        assert_eq!(recall_query(&m, &agent), "branch cache");
    }

    #[test]
    fn extensions_get_rows_and_a_title_but_never_the_palette() {
        let mut m = model();
        let title = apply_ui_calls(
            &mut m,
            &[
                json!({"op": "setHeader", "lines": ["branch: rust-rewrite"]}),
                json!({"op": "setFooter", "lines": ["2 checks pending"]}),
                json!({"op": "setWidget", "key": "todo", "lines": ["3 open todos"]}),
                json!({"op": "setWidget", "key": "hint", "content": "one\ntwo", "placement": "belowEditor"}),
                json!({"op": "setStatus", "key": "sync", "text": "synced"}),
                json!({"op": "notify", "message": "the index is stale"}),
                json!({"op": "setTitle", "title": "pi · rust-rewrite"}),
                // Ignored: davinci has one palette and two animations.
                json!({"op": "setTheme", "theme": "solarized"}),
                json!({"op": "setWorkingIndicator", "frames": ["-", "\\"]}),
            ],
        );

        assert_eq!(title.as_deref(), Some("pi · rust-rewrite"));
        assert_eq!(
            m.extensions.header,
            vec!["branch: rust-rewrite".to_string()]
        );
        assert_eq!(m.extensions.footer, vec!["2 checks pending".to_string()]);
        assert_eq!(m.extensions.above(), vec!["3 open todos", "synced"]);
        assert_eq!(m.extensions.below(), vec!["one", "two"]);
        assert!(m.transcript.iter().any(|entry| {
            matches!(entry, Entry::Tool { state, target, .. }
                if *state == State::Attention && target == "the index is stale")
        }));

        // A widget with no lines is a removal, keyed as the extension keyed it.
        apply_ui_calls(&mut m, &[json!({"op": "setWidget", "key": "todo"})]);
        assert_eq!(m.extensions.above(), vec!["synced"]);
        apply_ui_calls(&mut m, &[json!({"op": "setStatus", "key": "sync"})]);
        assert!(m.extensions.above().is_empty());
    }

    #[test]
    fn an_extension_can_fill_the_composer_and_add_to_it() {
        let mut m = model();
        apply_ui_calls(&mut m, &[json!({"op": "setEditorText", "text": "review "})]);
        assert_eq!(m.composer, "review ");
        apply_ui_calls(
            &mut m,
            &[json!({"op": "pasteToEditor", "text": "the diff"})],
        );
        assert_eq!(m.composer, "review the diff");
    }

    #[test]
    fn a_question_wears_a_named_panel_and_one_row_per_answer() {
        let mut agent = davinci_agent::Agent::new("test");
        agent.thinking_level = davinci_protocol::ThinkingLevel::Medium;

        let trust = Question::Trust {
            path: "C:\\work\\pi-rust".into(),
            options: crate::trust::get_project_trust_options(std::path::Path::new("."), false),
        };
        let panel = trust.ask(&agent);
        assert_eq!(panel.title, "Trust");
        assert!(panel.note.contains("C:\\work\\pi-rust"), "{}", panel.note);
        assert!(!panel.items.is_empty());

        let credentials = Question::Logout {
            providers: vec!["anthropic".into(), "openai".into()],
        }
        .ask(&agent);
        assert_eq!(credentials.title, "Credentials");
        assert_eq!(credentials.items.len(), 2);
        assert_eq!(credentials.items[0].label, "anthropic");
    }

    #[test]
    fn first_run_asks_only_what_davinci_cannot_decide_for_itself() {
        let agent = davinci_agent::Agent::new("test");
        let panel = Question::FirstRun.ask(&agent);
        // The old setup asked for a theme too; there is one palette here, so
        // the only question left is the one about the user, not the terminal.
        assert_eq!(panel.title, "Welcome");
        assert_eq!(panel.items.len(), 2);
        assert!(panel.items[0].label.contains("share"));
        assert!(panel.note.contains("never required"), "{}", panel.note);
    }

    #[test]
    fn an_untrusted_project_with_pi_resources_is_warned_about_by_glyph() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join(".pi").join("skills")).unwrap();

        let mut agent = davinci_agent::Agent::new("test");
        agent.cwd = project.clone();
        let block = opening_block(&crate::args::Args::default(), &agent, &[]);

        let warned = block.iter().any(|entry| {
            matches!(entry, Entry::Tool { state, target, .. }
                if *state == State::Attention && target.contains("not trusted"))
        });
        assert!(warned, "an untrusted project must say so");

        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn a_trusted_project_opens_without_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let project = dir.path().join("plain");
        std::fs::create_dir_all(&project).unwrap();

        let mut agent = davinci_agent::Agent::new("test");
        agent.cwd = project;
        let block = opening_block(&crate::args::Args::default(), &agent, &[]);
        assert!(!block.iter().any(|entry| {
            matches!(entry, Entry::Tool { target, .. } if target.contains("not trusted"))
        }));

        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn a_question_with_nothing_in_it_still_produces_a_panel() {
        let agent = davinci_agent::Agent::new("test");
        let panel = Question::Logout { providers: vec![] }.ask(&agent);
        assert!(panel.items.is_empty());
        assert_eq!(panel.key, "/logout");
    }

    #[test]
    fn an_unknown_slash_command_is_still_a_prompt() {
        // Skills and templates arrive this way; the agent expands them.
        // Extension commands never reach here — `on_line` runs them first.
        assert!(matches!(classify("/skill:review"), Sent::Prompt(_)));
    }

    #[test]
    fn a_native_command_is_run_by_the_shell_not_sent_to_the_model() {
        // `/graph-view` used to fall through `classify` as prose, reach the
        // provider verbatim, and come back as "the model returned no text".
        let host = crate::extension_host::ExtensionHost::default();
        let result = host
            .execute_native_command("graph-status", "")
            .expect("graph-status is a native command");
        assert!(result.is_some(), "the host must claim it");

        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            44,
            true,
        );
        push_command_result(&mut model, "graph-status", &result.unwrap());
        let said: Vec<String> = model
            .transcript
            .iter()
            .filter_map(|entry| match entry {
                Entry::Tool { target, .. } => Some(target.clone()),
                Entry::Detail(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|line| line == "/graph-status"),
            "the command names itself: {said:?}"
        );
    }

    #[test]
    fn a_failed_request_says_why_instead_of_blaming_the_model() {
        let target = |entries: &[Entry]| -> Vec<String> {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Tool { target, .. } => Some(target.clone()),
                    Entry::Detail(text) => Some(text.clone()),
                    Entry::Prose(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };

        // The provider's own words, not "the model returned no text". This is
        // the whole point: the reason used to be dropped with the worker's
        // return value, and every failure read as an empty answer.
        let out = turn_outcome(
            false,
            false,
            false,
            Some("No credential for openai-codex. Run /login openai-codex.".into()),
            "",
        );
        assert!(target(&out).iter().any(|line| line == "the request failed"));
        assert!(target(&out)
            .iter()
            .any(|line| line.contains("Run /login openai-codex")));
        assert!(out
            .iter()
            .any(|entry| matches!(entry, Entry::Tool { state, .. } if *state == State::Failed)));

        // A turn that genuinely said nothing still says so.
        let out = turn_outcome(false, false, false, None, "");
        assert!(target(&out)
            .iter()
            .any(|line| line.contains("the model returned no text")));

        // Text the events never carried is said rather than dropped.
        let out = turn_outcome(false, false, false, None, "  late answer  ");
        assert_eq!(target(&out), vec!["late answer".to_string()]);

        // A turn that spoke owes nothing more.
        assert!(turn_outcome(false, false, true, None, "").is_empty());

        // A crash and an interrupt outrank a failure message.
        let out = turn_outcome(true, false, false, Some("ignored".into()), "");
        assert!(target(&out).iter().any(|line| line.contains("crashed")));
        let out = turn_outcome(false, true, false, Some("ignored".into()), "");
        assert!(target(&out).iter().any(|line| line.contains("interrupted")));

        // An empty failure string is not a failure.
        let out = turn_outcome(false, false, false, Some("   ".into()), "");
        assert!(target(&out)
            .iter()
            .any(|line| line.contains("the model returned no text")));
    }

    #[test]
    fn a_slash_nobody_owns_is_named_rather_than_sent_to_the_model() {
        let mut m = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            44,
            true,
        );
        m.slash_commands = ["graph-view", "graph-status", "compact"]
            .into_iter()
            .map(|name| davinci_tui::SlashCommandSpec {
                name: name.into(),
                description: String::new(),
                argument_hint: None,
                argument_items: Vec::new(),
            })
            .collect();

        // A transposition must not be answered with the shorter command it
        // happens to begin with.
        let note = unknown_command(&m, "/graph-veiw").expect("not a command");
        assert!(note.contains("did you mean /graph-view?"), "{note}");

        // The typo in the screenshot: a trailing full stop.
        let note = unknown_command(&m, "/graph-view.").expect("not a command");
        assert!(note.contains("did you mean /graph-view?"), "{note}");

        // A prefix offers the shortest name it could still become.
        let note = unknown_command(&m, "/graph-s").expect("not a command");
        assert!(note.contains("did you mean /graph-status?"), "{note}");

        // A real command is left alone.
        assert!(unknown_command(&m, "/graph-view").is_none());
        assert!(unknown_command(&m, "/compact").is_none());

        // A slash line with arguments is a prompt, not a mistyped command.
        assert!(unknown_command(&m, "/explain this file").is_none());
        assert!(unknown_command(&m, "plain prose").is_none());

        // Nothing close: still say what to press.
        let note = unknown_command(&m, "/zzzz").expect("not a command");
        assert!(note.contains("ctrl+p"), "{note}");
    }

    #[test]
    fn a_command_result_becomes_rows_rather_than_a_json_dump() {
        let value = serde_json::json!({
            "runId": "run-7",
            "tasks": [{"id": "review-1", "status": "running"}],
            "pending": [],
        });
        let rows = command_result_rows(&value);
        assert!(rows.iter().any(|row| row == "run id · run-7"), "{rows:?}");
        assert!(rows.iter().any(|row| row == "tasks · 1"), "{rows:?}");
        assert!(
            rows.iter().any(|row| row == "  review-1 · running"),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row == "pending · none"), "{rows:?}");
        assert!(
            !rows.iter().any(|row| row.contains('{')),
            "no raw JSON: {rows:?}"
        );
    }

    #[test]
    fn an_error_result_carries_the_attention_glyph() {
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            44,
            true,
        );
        push_command_result(
            &mut model,
            "graph-view",
            &serde_json::json!({"error": "No graph runs in this project."}),
        );
        let failed = model.transcript.iter().any(|entry| {
            matches!(
                entry,
                Entry::Tool { state, instrument, .. }
                    if *state == State::Attention && instrument == "grafo"
            )
        });
        assert!(failed, "{:?}", model.transcript);
    }

    #[test]
    fn the_palette_lists_every_command_the_composer_completes() {
        let agent = Agent::new("x");
        let commands = vec![
            davinci_tui::SlashCommandSpec {
                name: "graph-view".into(),
                description: "Tail a graph worker's live transcript.".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
            davinci_tui::SlashCommandSpec {
                name: "quit".into(),
                description: "Leave".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
        ];
        let items = corpus(&agent, &commands, &[]);
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert!(names.contains(&"/graph-view"), "{names:?}");
        assert!(names.contains(&"/quit"), "{names:?}");
        assert!(names.contains(&"/todo"), "{names:?}");
        assert!(names.contains(&"/jobs"), "{names:?}");
        assert!(names.contains(&"/mcp"), "{names:?}");
        assert!(names.contains(&"/plan"), "{names:?}");
        assert!(names.contains(&"/act"), "{names:?}");
        assert!(names.contains(&"/cost"), "{names:?}");
        assert!(names.contains(&"/status"), "{names:?}");
        assert!(names.contains(&"/agents"), "{names:?}");
        assert!(names.contains(&"/workflow"), "{names:?}");
        assert!(names.contains(&"/workflows"), "{names:?}");
        assert!(names.contains(&"/workflow-stop"), "{names:?}");
        assert!(names.contains(&"/workflow-resume"), "{names:?}");
    }

    #[test]
    fn a_tool_row_says_what_the_tool_does_not_which_instrument_ran_it() {
        // design.md §3: `instrumenta` is the default instrument and is never
        // named. It used to fill the middle column of nearly every tool row,
        // which made the column say nothing at all.
        let mut agent = Agent::new("x");
        agent.tools = vec!["bash".into(), "read".into()];
        let items = corpus(&agent, &[], &[]);
        for item in &items {
            assert_ne!(item.description, "instrumenta", "{item:?}");
        }
        let bash = items.iter().find(|item| item.name == "bash").unwrap();
        assert!(!bash.description.is_empty(), "{bash:?}");
    }

    #[test]
    fn extension_ui_calls_are_taken_rather_than_replayed_each_turn() {
        let host = Arc::new(Mutex::new(crate::extension_host::ExtensionHost::default()));
        host.lock().unwrap().ui_calls = vec![serde_json::json!({
            "op": "notify",
            "message": "index rebuilt",
        })];
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            44,
            true,
        );
        drain_ui_calls(&mut model, &host);
        let notices = |model: &Model| {
            model
                .transcript
                .iter()
                .filter(|entry| matches!(entry, Entry::Tool { target, .. } if target == "index rebuilt"))
                .count()
        };
        assert_eq!(notices(&model), 1);

        // The host is shared across every turn, so reading the queue without
        // taking it said the same thing again on each one.
        drain_ui_calls(&mut model, &host);
        assert_eq!(notices(&model), 1);
    }

    #[test]
    fn a_resumed_session_opens_where_it_left_off() {
        let messages = vec![
            assistant("first reply"),
            davinci_ai::ChatMessage {
                role: "user".into(),
                content: vec![davinci_ai::MessageContent::Text {
                    text: "and then?".into(),
                }],
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                extra: Default::default(),
            },
        ];
        let entries = transcript_from(&messages);
        assert!(matches!(&entries[0], Entry::Agent(name) if name == "davinci"));
        assert!(matches!(&entries[2], Entry::Prose(text) if text == "first reply"));
        assert!(matches!(&entries[4], Entry::User(text) if text == "and then?"));
    }

    #[test]
    fn a_resumed_session_keeps_the_calls_the_turn_made() {
        let messages = vec![
            davinci_ai::ChatMessage {
                role: "assistant".into(),
                content: vec![
                    davinci_ai::MessageContent::Text {
                        text: "looking".into(),
                    },
                    davinci_ai::MessageContent::ToolCall {
                        id: "call-1".into(),
                        name: "read".into(),
                        arguments: json!({"path": "src/lib.rs"}),
                    },
                    davinci_ai::MessageContent::ToolCall {
                        id: "call-2".into(),
                        name: "bash".into(),
                        arguments: json!({"command": "cargo test"}),
                    },
                ],
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                extra: Default::default(),
            },
            davinci_ai::ChatMessage {
                role: "tool".into(),
                content: vec![davinci_ai::MessageContent::Text {
                    text: "one\ntwo\nthree".into(),
                }],
                tool_call_id: Some("call-1".into()),
                tool_name: Some("read".into()),
                is_error: None,
                extra: Default::default(),
            },
            davinci_ai::ChatMessage {
                role: "tool".into(),
                content: vec![davinci_ai::MessageContent::Text {
                    text: "error[E0308] mismatched types".into(),
                }],
                tool_call_id: Some("call-2".into()),
                tool_name: Some("bash".into()),
                is_error: Some(true),
                extra: Default::default(),
            },
        ];

        let entries = transcript_from(&messages);
        let read = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Tool {
                    state,
                    target,
                    summary,
                    ..
                } if target.starts_with("read ") => Some((*state, summary.clone())),
                _ => None,
            })
            .expect("the read is drawn");
        assert_eq!(read, (State::Read, Some("3 lines".to_string())));

        let failed = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Tool {
                    state,
                    target,
                    summary,
                    output,
                    ..
                } if target == "cargo test" => Some((*state, summary.clone(), output.clone())),
                _ => None,
            })
            .expect("the command is drawn");
        assert_eq!(failed.0, State::Failed);
        assert_eq!(failed.1, None, "a failure states no outcome");
        assert!(
            failed.2.iter().any(|line| line.contains("E0308")),
            "the failure rides on the tool line: {:?}",
            failed.2
        );
        assert!(
            !entries
                .iter()
                .any(|entry| matches!(entry, Entry::Detail(_))),
            "a resumed failure is not a Detail sibling"
        );
    }

    #[test]
    fn an_outcome_is_stated_in_the_fewest_words_that_say_it() {
        assert_eq!(
            summary_of("grep", &json!({"pattern": "x"}), &json!("a\nb")),
            Some("2 matches".into())
        );
        assert_eq!(
            summary_of("grep", &json!({"pattern": "x"}), &json!("")),
            Some("0 matches".into())
        );
        assert_eq!(
            summary_of("read", &json!({"path": "a"}), &json!("only")),
            Some("1 line".into())
        );
        assert_eq!(
            summary_of(
                "edit",
                &json!({"edits": [{"oldText": "a\nb", "newText": "a\nb\nc\nd"}]}),
                &json!("Edited a")
            ),
            Some("+4 -2".into())
        );
        assert_eq!(summary_of("ask", &json!({}), &json!("whatever")), None);
        assert_eq!(
            summary_of(
                "todo",
                &json!({"items": [
                    {"text": "survey", "status": "completed"},
                    {"text": "edit", "status": "in_progress"}
                ]}),
                &json!("ok")
            ),
            Some("1 of 2 done".into())
        );
        assert_eq!(
            summary_of(
                "web_search",
                &json!({"query": "ratatui"}),
                &json!("1. ratatui.rs\n2. docs.rs/ratatui")
            ),
            Some("2 results".into())
        );
    }

    #[test]
    fn an_empty_session_opens_on_the_empty_state() {
        assert!(transcript_from(&[]).is_empty());
        let blank = assistant("   ");
        assert!(transcript_from(&[blank]).is_empty());
    }

    #[test]
    fn durations_read_in_seconds() {
        assert_eq!(duration_of(Duration::from_millis(1_840)), "1.84s");
        assert_eq!(duration_of(Duration::from_millis(420)), "0.42s");
    }

    #[test]
    fn an_edit_draws_its_delta_from_the_details_the_tool_returned() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "edit".into(),
                args: json!({"path": "src/lib.rs"}),
            },
        );
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionEnd {
                tool_call_id: "1".into(),
                tool_name: "edit".into(),
                result: json!("Edited src/lib.rs"),
                is_error: false,
                details: Some(json!({
                    "path": "src/lib.rs",
                    "diff": "+ 1 pub fn foo() {\n- 1 pub fn bar() {\n"
                })),
            },
        );
        assert!(matches!(
            &m.transcript[0],
            Entry::Tool {
                state: State::Delta,
                ..
            }
        ));
        assert!(matches!(m.transcript[1], Entry::Gap));
        match &m.transcript[2] {
            Entry::Delta {
                path,
                adds,
                dels,
                hunks,
            } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!((*adds, *dels), (1, 1));
                assert_eq!(hunks[0].kind, HunkKind::Add);
                assert!(hunks[0].text.contains("foo"));
                assert_eq!(hunks[1].kind, HunkKind::Del);
                assert!(hunks[1].text.contains("bar"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_todo_call_becomes_the_studio_ledger_and_the_plan_sheet() {
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "1".into(),
                tool_name: "todo".into(),
                args: json!({"items": [
                    {"text": "survey", "status": "completed"},
                    {"text": "edit", "status": "in_progress"},
                    {"text": "test", "status": "pending"}
                ]}),
            },
        );
        assert!(
            m.transcript
                .iter()
                .any(|entry| matches!(entry, Entry::Studio(steps) if steps.len() == 3)),
            "{:?}",
            m.transcript
        );
        assert_eq!(m.plan.len(), 3);
        assert_eq!(m.plan[0].state, State::Done);
        assert_eq!(m.plan[1].state, State::Active);
        assert_eq!(m.plan[2].state, State::Queued);
    }

    #[test]
    fn a_finished_job_is_a_manus_row_with_its_tail_behind_the_line() {
        let ok = davinci_agent::JobNotice {
            id: 1,
            command: "cargo build".into(),
            status: davinci_agent::JobStatus::Exited(0),
            elapsed: Duration::from_millis(31_200),
            tail: vec!["Compiling pi".into(), "Finished".into()],
        };
        match job_row(&ok) {
            Entry::Tool {
                state: State::Done,
                instrument,
                target,
                summary,
                output,
                duration,
            } => {
                assert_eq!(instrument, "manus");
                assert!(target.contains("job 1 finished"), "{target}");
                assert!(target.contains("cargo build"), "{target}");
                assert_eq!(summary.as_deref(), Some("exit 0"));
                assert_eq!(duration.as_deref(), Some("31.2s"));
                assert!(output.iter().any(|line| line.contains("Compiling pi")));
            }
            other => panic!("{other:?}"),
        }
        let fail = davinci_agent::JobNotice {
            id: 2,
            command: "cargo test".into(),
            status: davinci_agent::JobStatus::Exited(1),
            elapsed: Duration::from_millis(400),
            tail: vec!["FAILED".into()],
        };
        match job_row(&fail) {
            Entry::Tool {
                state: State::Failed,
                summary,
                ..
            } => assert_eq!(summary.as_deref(), Some("exit 1")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn new_tools_name_their_instrument_state_verb_and_target() {
        assert_eq!(instrument_of("web_fetch"), "instrumenta");
        assert_eq!(instrument_of("job_output"), "manus");
        assert_eq!(state_of("web_fetch", false), State::Search);
        assert_eq!(state_of("web_search", false), State::Search);
        assert_eq!(state_of("todo", false), State::Done);
        assert_eq!(state_of("job_output", false), State::Read);
        assert_eq!(state_of("notebook_edit", false), State::Delta);
        assert_eq!(verb_of("todo"), "planning");
        assert_eq!(verb_of("web_search"), "surveying");
        assert_eq!(verb_of("notebook_edit"), "constructing");
        assert_eq!(
            target_of("web_fetch", &json!({"url": "https://docs.rs/ratatui/"})),
            "fetch docs.rs/ratatui"
        );
        assert_eq!(
            target_of("web_search", &json!({"query": "myers diff"})),
            "search web \"myers diff\""
        );
        assert_eq!(
            target_of("todo", &json!({"items": [{"text": "a"}, {"text": "b"}]})),
            "plan · 2 items"
        );
        assert_eq!(
            target_of("job_output", &json!({"jobId": 3})),
            "job 3 output"
        );
        assert_eq!(
            target_of("notebook_edit", &json!({"path": "n.ipynb", "cell": 2})),
            "edit n.ipynb · cell 2"
        );
        assert_eq!(instrument_of("mcp_read"), "instrumenta");
        assert_eq!(instrument_of("mcp__memory__echo"), "instrumenta");
        assert_eq!(state_of("mcp_read", false), State::Read);
        assert_eq!(state_of("mcp__memory__echo", false), State::Done);
        assert_eq!(verb_of("mcp_read"), "studying");
        assert_eq!(
            target_of(
                "mcp_read",
                &json!({"server": "memory", "uri": "fixture://note"})
            ),
            "mcp memory fixture://note"
        );
        assert_eq!(
            target_of("mcp__memory__echo", &json!({"text": "hi"})),
            "mcp memory echo hi"
        );
        assert_eq!(verb_of("agent"), "delegating");
        assert_eq!(
            target_of(
                "agent",
                &json!({"prompt": "scan the crate", "description": "survey"})
            ),
            "agent survey"
        );
    }

    #[test]
    fn a_persisted_background_job_replays_as_a_tool_row_not_a_user_echo() {
        let mut extra = serde_json::Map::new();
        extra.insert("customType".into(), json!(davinci_agent::JOB_NOTICE_TYPE));
        extra.insert("jobId".into(), json!(1));
        let notice = davinci_agent::JobNotice {
            id: 1,
            command: "cargo build".into(),
            status: davinci_agent::JobStatus::Exited(0),
            elapsed: Duration::from_millis(31_200),
            tail: vec!["Finished".into()],
        };
        let message = davinci_ai::ChatMessage {
            role: "user".into(),
            content: vec![davinci_ai::MessageContent::Text {
                text: notice.message_text(),
            }],
            extra,
            ..davinci_ai::ChatMessage::default()
        };
        let entries = transcript_from(&[message]);
        match &entries[0] {
            Entry::Tool {
                state: State::Done,
                instrument,
                target,
                summary,
                output,
                ..
            } => {
                assert_eq!(instrument, "manus");
                assert!(target.contains("job 1 finished"), "{target}");
                assert_eq!(summary.as_deref(), Some("exit 0"));
                assert!(output.iter().any(|line| line.contains("Finished")));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            !entries.iter().any(|entry| matches!(entry, Entry::User(_))),
            "{entries:?}"
        );
    }

    #[test]
    fn hunks_from_diff_strips_the_number_column_and_counts() {
        let (adds, dels, hunks) = hunks_from_diff("+12 added\n- 8 removed\n  4 kept\n    ...\n");
        assert_eq!((adds, dels), (1, 1));
        assert_eq!(hunks[0].kind, HunkKind::Add);
        assert_eq!(hunks[0].text, "added");
        assert_eq!(hunks[1].kind, HunkKind::Del);
        assert_eq!(hunks[1].text, "removed");
        assert_eq!(hunks[2].kind, HunkKind::Context);
        assert_eq!(hunks[2].text, "kept");
        assert_eq!(hunks[3].text, "…");
    }

    #[test]
    fn open_resume_sheet_shows_all_sessions_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("sessions");
        let work_dir = dir.path().join("work");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&work_dir).unwrap();

        // Create 35 sessions (exceeding the old 30-item limit)
        for i in 0..35 {
            davinci_session::JsonlSession::create(
                &session_dir,
                &work_dir.to_string_lossy(),
                Some(&format!("session-{i}")),
            )
            .unwrap();
        }

        let parsed = crate::args::Args {
            session_dir: Some(session_dir.to_string_lossy().into_owned()),
            ..crate::args::Args::default()
        };
        let mut agent = davinci_agent::Agent::new("test");
        agent.cwd = work_dir;

        let mut m = model();
        open_resume_sheet(&parsed, &agent, &mut m);

        assert_eq!(m.screen, Screen::Resume);
        assert_eq!(m.session_count, 35);
        assert_eq!(
            m.resume_sessions.len(),
            35,
            "all 35 sessions must be present without a 30-item truncation limit"
        );
    }

    #[test]
    fn session_commands_classify_to_resume_or_stats() {
        use crate::slash::SlashAction;
        assert!(matches!(
            classify("/session"),
            Sent::Command(SlashAction::Resume)
        ));
        assert!(matches!(
            classify("/sessions"),
            Sent::Command(SlashAction::Resume)
        ));
        assert!(matches!(
            classify("/resume"),
            Sent::Command(SlashAction::Resume)
        ));
        assert!(matches!(
            classify("/session info"),
            Sent::Command(SlashAction::SessionInfo)
        ));
        assert!(matches!(
            classify("/session stats"),
            Sent::Command(SlashAction::SessionInfo)
        ));
    }

    #[test]
    fn test_workflow_commands_and_sheet() {
        let mut agent = davinci_agent::Agent::new("test");
        let bus = davinci_agent::RuntimeBus::new();
        let runtime = davinci_agent::RuntimeHandle::new(
            davinci_agent::RunId::new(),
            davinci_agent::AgentId::new(),
            bus,
        );
        let store = davinci_agent::WorkflowStateStore::new();
        let executor = std::sync::Arc::new(davinci_agent::WorkflowExecutor::new(
            runtime.clone(),
            store,
            None,
        ));
        let runtime = runtime.with_workflow_executor(executor.clone());
        agent.runtime = Some(runtime);

        let mut m = model();
        open_workflows_sheet(&agent, &mut m);
        assert_eq!(m.screen, Screen::Workflows);
        assert!(m.workflows.is_some());
        assert_eq!(m.workflows.unwrap().workflows.len(), 0);
    }

    #[test]
    fn f02_wait_native_decision_timeout() {
        let (tx, rx) = mpsc::channel();
        let abort = AtomicBool::new(false);
        let q = davinci_agent::decisions::DecisionQuestion {
            id: "q-timeout".into(),
            kind: davinci_agent::decisions::DecisionKind::Architecture,
            title: "Timeout Test".into(),
            question: "Will it time out?".into(),
            materiality: "High".into(),
            evidence_refs: vec![],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![],
            allow_custom: true,
            custom_only: true,
            plan_revision: 1,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
        };
        let req = davinci_agent::DecisionHostRequest { question: q };
        // Expired deadline
        let expired_ms = davinci_session::now_ms().saturating_sub(10);
        let resp = wait_native_decision(req, expired_ms, &tx, &abort);
        assert_eq!(resp, davinci_agent::DecisionHostResponse::Cancelled);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn f02_wait_native_decision_abort_cancelled() {
        let (tx, _rx) = mpsc::channel();
        let abort = AtomicBool::new(true);
        let q = davinci_agent::decisions::DecisionQuestion {
            id: "q-abort".into(),
            kind: davinci_agent::decisions::DecisionKind::Architecture,
            title: "Abort Test".into(),
            question: "Will it cancel?".into(),
            materiality: "High".into(),
            evidence_refs: vec![],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![],
            allow_custom: true,
            custom_only: true,
            plan_revision: 1,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
        };
        let req = davinci_agent::DecisionHostRequest { question: q };
        let resp = wait_native_decision(req, davinci_session::now_ms() + 10_000, &tx, &abort);
        assert_eq!(resp, davinci_agent::DecisionHostResponse::Cancelled);
    }

    #[test]
    fn f02_open_decision_modal_mapping() {
        let mut m = model();
        let q = davinci_agent::decisions::DecisionQuestion {
            id: "q-map".into(),
            kind: davinci_agent::decisions::DecisionKind::Behavior,
            title: "Map Test".into(),
            question: "Map Question?".into(),
            materiality: "Medium".into(),
            evidence_refs: vec!["ref1".into()],
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: vec![davinci_agent::decisions::DecisionOptionInput {
                id: "opt1".into(),
                label: "Option 1".into(),
                explanation: "Expl 1".into(),
                recommended: true,
            }],
            allow_custom: true,
            custom_only: false,
            plan_revision: 3,
            state: davinci_agent::decisions::DecisionState::Open,
            answer: None,
        };
        open_decision_modal(&mut m, &q);
        assert_eq!(m.overlay, Some(Overlay::Ask));
        assert!(m.decision_modal.is_some());
        let modal = m.decision_modal.as_ref().unwrap();
        assert_eq!(modal.question_id, "q-map");
        assert_eq!(modal.title, "Map Test");
        assert_eq!(modal.options.len(), 1);
        assert_eq!(modal.options[0].id, "opt1");
        assert!(modal.options[0].recommended);
        assert_eq!(modal.plan_revision, 3);
    }

    #[test]
    fn f04_open_rewind_modal_mapping() {
        let mut m = model();
        let preview = davinci_agent::runtime::rewind::RewindPreview {
            checkpoint_id: "cp-123".into(),
            preview_digest: "digest-abc".into(),
            files: vec![davinci_agent::runtime::rewind::FileRewindPlan {
                path: "src/main.rs".into(),
                classification: "inverse".into(),
                resolved_content: Some(b"fn main() {}".to_vec()),
                is_conflict: false,
                conflict_reason: None,
                pre_rewind_hash: None,
            }],
            conflict_count: 0,
            irreversible_effects: vec![davinci_agent::runtime::effects::ExternalEffectReceipt {
                receipt_id: "rcpt-1".into(),
                task_id: davinci_agent::TaskId::new(),
                operation_id: "op-1".into(),
                kind: "deploy".into(),
                details: serde_json::json!({
                    "secret_token": "secret123",
                    "target": "staging"
                }),
                timestamp_ms: 1000,
                reversible: false,
            }],
        };
        open_rewind_modal(&mut m, &preview, "Pre-Task 1", "12:00:00");
        assert_eq!(m.overlay, Some(Overlay::Ask));
        assert!(m.rewind_modal.is_some());
        let modal = m.rewind_modal.as_ref().unwrap();
        assert_eq!(modal.checkpoint_id, "cp-123");
        assert_eq!(modal.checkpoint_name, "Pre-Task 1");
        assert_eq!(modal.checkpoint_time, "12:00:00");
        assert_eq!(modal.preview_digest, "digest-abc");
        assert_eq!(modal.files.len(), 1);
        assert_eq!(modal.files[0].path, "src/main.rs");
        assert_eq!(modal.files[0].classification, "inverse");
        assert_eq!(modal.conflict_count, 0);
        assert_eq!(modal.irreversible_effects.len(), 1);
        assert_eq!(modal.irreversible_effects[0].operation_id, "op-1");
        assert!(modal.irreversible_effects[0].details.contains("[REDACTED]"));
        assert!(!modal.irreversible_effects[0].details.contains("secret123"));
    }
}

#[cfg(test)]
mod terminal_rebuild_host_contracts {
    use super::*;
    #[test]
    fn starting_a_graph_does_not_force_the_canvas_open() {
        assert!(!graph_command_opens_view(
            "graph",
            "Inspect the parser and implement recovery"
        ));
        assert!(!graph_command_opens_view("graph", "run saved-example"));
        assert!(graph_command_opens_view("graph-status", ""));
        assert!(graph_command_opens_view("graph-view", ""));
        assert!(!graph_command_opens_view("model", ""));
    }
}
