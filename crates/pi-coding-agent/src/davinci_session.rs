//! The davinci TUI, driven by the real agent.
//!
//! The shell in `pi_tui::davinci` knows nothing about agents; this module owns
//! the loop that turns a sent composer line into an agent turn and the agent's
//! events back into transcript blocks (`docs/ui/design.md` §6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pi_agent::{
    Agent, AgentEvent, EventSink, PermissionMode, ToolApprovalDecision, ToolApprovalRequest,
    ToolApprover,
};
use pi_tui::davinci::model::{
    Ask, CatalogRow, Choice, Compaction, CorpusItem, Credential, Entry, ExportLedger, FailedRun,
    Finding, GovernorCounter, GovernorSheet, GovernorStored, GraphRunSheet, GraphTask, Hunk,
    HunkKind, KeymapGroup, McpServerRow, McpSheet, Model, ModelItem, Overlay, PermissionRow,
    PickerItem, PlanStep, ProjectTrustSheet, ProviderRow, ResumeRow, ReviewFile, ReviewSheet,
    Screen, SecurityScan, SettingRow, Severity, Step, ThinkingRow, Tone, TreeNode, TrustFile,
    VectorIndex, Working, WorkshopSheet,
};
use pi_tui::davinci::theme::State;

use crate::extension_host::ExtensionHost;

/// Which instrument a tool belongs to (design.md §5). Shell execution is
/// Manus; everything else the agent reaches for is Instrumenta.
pub fn instrument_of(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" | "powershell" | "job_output" | "job_kill" => "manus",
        name if name.starts_with("memory") => "memoria",
        name if name.starts_with("graph") => "grafo",
        _ => "instrumenta",
    }
}

/// The glyph a tool call carries while it runs and once it is done.
pub fn state_of(tool_name: &str, failed: bool) -> State {
    if failed {
        return State::Failed;
    }
    match tool_name {
        "read" | "ls" | "job_output" | "mcp_read" => State::Read,
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
    let text = result_text(result);
    text.contains("Permission denied") || text.starts_with("plan mode:")
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
                        .and_then(pi_agent::TodoStatus::parse)
                        == Some(pi_agent::TodoStatus::Done)
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
            if let Ok(list) = pi_agent::TodoList::from_args(args) {
                self.take_ledger(model, &list);
                return;
            }
        }
        self.push_step(model, tool_name, args);
    }

    /// The model's list becomes the ledger: the STUDIO box and the plan
    /// sheet both show it, and tools stop adding steps of their own.
    fn take_ledger(&mut self, model: &mut Model, list: &pi_agent::TodoList) {
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
            *output = pi_tui::davinci::model::tool_output_rows(&result_text(result));
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
pub fn steps_from_todos(list: &pi_agent::TodoList) -> Vec<Step> {
    list.items
        .iter()
        .map(|item| Step::new(todo_state(item.status), &item.text, None))
        .collect()
}

/// The same ledger on the `1c` plan sheet, numbered.
pub fn plan_from_todos(list: &pi_agent::TodoList) -> Vec<PlanStep> {
    list.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            PlanStep::new(
                &pi_tui::davinci::views::disegno::roman(index + 1),
                todo_state(item.status),
                &item.text,
                None,
            )
        })
        .collect()
}

fn todo_state(status: pi_agent::TodoStatus) -> State {
    match status {
        pi_agent::TodoStatus::Done => State::Done,
        pi_agent::TodoStatus::Active => State::Active,
        pi_agent::TodoStatus::Pending => State::Queued,
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
pub fn job_row(notice: &pi_agent::JobNotice) -> Entry {
    let state = if notice.status.succeeded() {
        State::Done
    } else {
        State::Failed
    };
    Entry::tool(
        state,
        "manus",
        &format!("job {} finished · {}", notice.id, clip(&notice.command, 50)),
        Some(&pi_agent::jobs::format_elapsed(notice.elapsed)),
    )
    .summarised(&notice.status.describe())
    .with_output(&notice.tail.join("\n"))
}

/// Finished jobs the user has not seen become rows; the count of running
/// ones feeds the status bar. Safe mid-turn: rows go at the end.
pub fn poll_jobs(jobs: &Arc<Mutex<pi_agent::JobBook>>, model: &mut Model) {
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
        } => turn.end_tool(
            model,
            tool_call_id,
            tool_name,
            result,
            *is_error,
            details.as_ref(),
        ),

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
            use pi_ai::AssistantMessageEvent as Ev;
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
                let estimated = estimate_tokens(&pi_ai::content_text(&message.content));
                working.tokens = turn.streamed + reported.max(estimated);
            }
        }

        AgentEvent::MessageEnd { message } if message.role == "assistant" => {
            let text = pi_ai::content_text(&message.content);
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
        KeyCode::Char('t') if ctrl => model.show_tool_output = !model.show_tool_output,
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
    if let Err(err) = run_turn(shell.parsed, shell.agent, shell.model, shell.terminal, host) {
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

fn run_turn(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    model: &mut Model,
    session: &mut pi_tui::davinci::runtime::Session,
    host: Arc<Mutex<ExtensionHost>>,
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
    let cwd = agent.cwd.clone();
    let (approval_tx, approval_rx) =
        mpsc::channel::<(ToolApprovalRequest, mpsc::Sender<ToolApprovalDecision>)>();
    agent.approver = Some(ToolApprover(Arc::new(move |request| {
        let (reply_tx, reply_rx) = mpsc::channel();
        if approval_tx.send((request.clone(), reply_tx)).is_err() {
            return ToolApprovalDecision::Deny;
        }
        reply_rx.recv().unwrap_or(ToolApprovalDecision::Deny)
    })));
    let mut approval: Option<(ToolApprovalRequest, mpsc::Sender<ToolApprovalDecision>)> = None;
    let mut last_tick = Instant::now();
    // The job book, read on every tick while the worker holds the agent.
    let jobs = agent.tool_context.jobs.clone();
    // The working line above the composer, for as long as the turn runs.
    let started = Instant::now();
    model.working = Some(Working {
        seconds: 0,
        tokens: 0,
        thinking: thinking_effort(agent),
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

    std::thread::scope(|scope| -> std::io::Result<()> {
        let worker = scope
            .spawn(|| crate::complete_prompt_with_host(parsed, agent, Some(host.clone()), false));

        loop {
            while let Ok(event) = event_rx.try_recv() {
                apply(model, &mut turn, &event);
            }
            if approval.is_none() {
                if let Ok((request, reply)) = approval_rx.try_recv() {
                    turn.await_approval(model, &request);
                    model.ask = permission_ask(&request, trusted);
                    model.ask_index = 0;
                    model.overlay = Some(Overlay::Ask);
                    approval = Some((request, reply));
                }
            }
            if let Some(working) = model.working.as_mut() {
                working.seconds = started.elapsed().as_secs();
            }
            session.draw(model)?;

            if worker.is_finished() {
                break;
            }
            if let Some(event) = session.poll_event(Duration::from_millis(40))? {
                match event {
                    crossterm::event::Event::Key(key)
                        if key.kind != crossterm::event::KeyEventKind::Release
                            && approval.is_some() =>
                    {
                        let decision = approval_key(model, key, trusted, &abort);
                        if pi_ai::trace::enabled() {
                            pi_ai::trace::log(&format!(
                                "davinci permission panel: key {:?} {:?} -> {decision:?}",
                                key.code, key.modifiers
                            ));
                        }
                        if let Some(decision) = decision {
                            let (request, reply) = approval.take().expect("checked above");
                            let decision = match decision {
                                ToolApprovalDecision::AllowAlways => {
                                    match crate::permissions::remember_project_rule(
                                        &cwd,
                                        &request.session_rule,
                                    ) {
                                        Ok(_) => {
                                            turn.settle_approval(
                                                model,
                                                &request,
                                                Some(&request.session_rule),
                                            );
                                            ToolApprovalDecision::AllowAlways
                                        }
                                        // The file could not be written: the
                                        // grant still holds for this run, and
                                        // the user is told why it is no more.
                                        Err(err) => {
                                            turn.settle_approval(model, &request, None);
                                            model.transcript.push(Entry::tool(
                                                State::Attention,
                                                "instrumenta",
                                                &format!(
                                                    "could not save {} · {err}",
                                                    request.session_rule
                                                ),
                                                None,
                                            ));
                                            ToolApprovalDecision::AllowForSession
                                        }
                                    }
                                }
                                other => {
                                    turn.settle_approval(model, &request, None);
                                    other
                                }
                            };
                            let _ = reply.send(decision);
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
                            let _ = pi_tui::davinci::runtime::restore();
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
                    crossterm::event::Event::Paste(text) => model.paste(&text),
                    _ => {}
                }
            }
            if last_tick.elapsed() >= pi_tui::davinci::runtime::TICK {
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
                reply = text;
            }
        }
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
            .map(|message| clip(pi_ai::content_text(&message.content).trim(), 60))
            .unwrap_or_default();
        let kept = pi_agent::estimate_context_tokens(&agent.messages);
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
                pi_tui::davinci::views::chrome::thousands(kept)
            ),
            billed: "in the next /session stats".into(),
            aftermath: vec![
                (
                    State::Done,
                    "transcript written to the session file · nothing to recover on restart".into(),
                ),
                (
                    State::Done,
                    "the abort was delivered; a running tool stops at its next check".into(),
                ),
                (
                    State::Attention,
                    "queued follow-ups were dropped with the interrupt".into(),
                ),
                (
                    State::Skipped,
                    "esc esc opens the session tree · ctrl+d quits".into(),
                ),
            ],
        });
        open_sheet(model, Screen::Recovery);
    }

    agent.abort_signal = None;
    agent.event_sink = None;
    agent.approver = None;
    // A question the worker never got an answer to (it was interrupted under
    // the panel) is closed with it.
    if approval.take().is_some() && model.overlay == Some(Overlay::Ask) {
        model.overlay = None;
    }
    model.running = false;
    if model.terminal_progress {
        let _ = session.set_progress(false);
    }

    // Extensions may have asked for rows while the turn ran. Taking the queue
    // rather than reading it is what keeps a `notify` from being replayed into
    // the transcript on every later turn.
    drain_ui_calls(model, &host);
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
            pi_tui::davinci::views::chrome::thousands(waste.missed_tokens as u64)
        ),
        None,
    ));
}

/// Everything already in the session, as transcript blocks, so a resumed
/// session opens where it left off rather than empty.
pub fn transcript_from(messages: &[pi_ai::ChatMessage]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    // `tool_call_id` -> where its line sits, and what it was asked to do, so a
    // resumed session states its outcomes the way the live one did.
    let mut open: Vec<(String, usize, String, serde_json::Value)> = Vec::new();

    for message in messages {
        let text = pi_ai::content_text(&message.content);
        let text = text.trim();
        match message.role.as_str() {
            "user" if !text.is_empty() => {
                if !entries.is_empty() {
                    entries.push(Entry::Gap);
                }
                if message.extra.get("customType")
                    == Some(&serde_json::Value::String(
                        pi_agent::JOB_NOTICE_TYPE.to_string(),
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
                        pi_ai::MessageContent::ToolCall {
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
                let outcome = (!failed)
                    .then(|| summary_of(&name, &arguments, &result))
                    .flatten();
                if let Some(Entry::Tool {
                    state,
                    summary,
                    output,
                    ..
                }) = entries.get_mut(index)
                {
                    *state = state_of(&name, failed);
                    *summary = outcome;
                    // The result rides on the line, as it does live: a
                    // failure shows its first rows, `ctrl+t` shows any.
                    *output = pi_tui::davinci::model::tool_output_rows(text);
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
fn job_entry_from_notice(message: &pi_ai::ChatMessage, text: &str) -> Entry {
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
    commands: &[pi_tui::SlashCommandSpec],
    sessions: &[pi_tui::davinci::model::SessionItem],
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
        "/diff",
        "review every change in the working tree",
        "command",
    ));
    items.push(CorpusItem::new(
        "/permissions",
        "what runs without asking · read-only, ask, edits, auto",
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
    let described = pi_agent::tool_specs()
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.description)
        .or_else(|| {
            crate::native_extensions::NativeExtensionHost::tool_specs()
                .into_iter()
                .find(|tool| tool.name == name)
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
    let (name, args) = crate::parse_extension_command(line);
    if name.is_empty() {
        return None;
    }

    let outcome = {
        let mut host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        crate::apply_graph_session_context(shell.parsed, shell.agent, &host);
        match host.execute_native_command(&name, &args) {
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
            "sec-status" => {
                shell.model.security = Some(security_sheet(&value));
                shell.model.security_index = 0;
                open_sheet(shell.model, Screen::Securitas);
            }
            "sec-report" => {
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
            "graph-status" | "graph-view" => match graph_sheet(&value) {
                Some(sheet) => {
                    shell.model.graph_run = Some(sheet);
                    open_sheet(shell.model, Screen::GraphRun);
                }
                None => push_command_result(shell.model, &name, &value),
            },
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
    /// Whether this project's `.pi` resources may be used.
    Trust {
        path: String,
        options: Vec<crate::trust::ProjectTrustOption>,
    },
    /// Which stored credential to remove.
    Logout { providers: Vec<String> },
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
            Question::Trust { path, options } => Ask {
                title: "FIDES".into(),
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
                title: "SALVE".into(),
                name: "WELCOME".into(),
                key: "first run".into(),
                note: "anonymous usage data helps pi improve; it is never required".into(),
                items: vec![
                    PickerItem::new("share anonymous usage data", "recommended"),
                    PickerItem::new("keep it to this machine", ""),
                ],
            },
            Question::Logout { providers } => Ask {
                title: "CLAVES".into(),
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

/// The `LICENTIA · PERMISSION` panel for one tool call the policy could not
/// decide on its own (spec: trust-and-control, *davinci*). Four rows; three
/// when the project is not trusted, because a rule written to
/// `.pi/settings.json` would never be read back from an untrusted checkout.
pub fn permission_ask(request: &ToolApprovalRequest, trusted: bool) -> Ask {
    let rule = &request.session_rule;
    let mut items = vec![
        PickerItem::new("allow once", "runs this call only"),
        PickerItem::new("allow for this session", &format!("{rule} until pi exits")),
    ];
    if trusted {
        items.push(PickerItem::new(
            "always allow here",
            &format!("{rule} saved to .pi/settings.json"),
        ));
    }
    items.push(PickerItem::new("deny", "the model is told no"));
    let mut note = request.summary.clone();
    if request.outside_project {
        note.push_str(" · outside the project");
    }
    Ask {
        title: "LICENTIA".into(),
        name: "PERMISSION".into(),
        key: "/permissions".into(),
        note,
        items,
    }
}

/// What the chosen row of `permission_ask` means; `None` for a row that the
/// panel did not offer.
pub fn permission_choice(index: usize, trusted: bool) -> Option<ToolApprovalDecision> {
    match (index, trusted) {
        (0, _) => Some(ToolApprovalDecision::AllowOnce),
        (1, _) => Some(ToolApprovalDecision::AllowForSession),
        (2, true) => Some(ToolApprovalDecision::AllowAlways),
        (2, false) | (3, true) => Some(ToolApprovalDecision::Deny),
        _ => None,
    }
}

/// One key while the permission panel is up. The panel takes the keys every
/// other panel takes — ↑↓ move, enter chooses, esc closes — and esc closing
/// an unanswered question is a `deny`, not a shrug. `ctrl+c` raises the
/// abort flag as it does anywhere mid-turn, and denies so the worker sees
/// the flag promptly rather than after a call the user was refusing.
fn approval_key(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    trusted: bool,
    abort: &Arc<AtomicBool>,
) -> Option<ToolApprovalDecision> {
    use pi_tui::davinci::app::{handle_key, Flow};
    match handle_key(model, key) {
        Flow::Choose(Choice::Ask(index)) => {
            Some(permission_choice(index, trusted).unwrap_or(ToolApprovalDecision::Deny))
        }
        Flow::Interrupt | Flow::Quit => {
            abort.store(true, Ordering::Relaxed);
            model.interrupt();
            model.overlay = None;
            Some(ToolApprovalDecision::Deny)
        }
        Flow::Continue if model.overlay.is_none() => Some(ToolApprovalDecision::Deny),
        Flow::Continue | Flow::Choose(_) | Flow::Submit(_) | Flow::CycleThinking => None,
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
    use pi_session::JsonlSession;
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
            agent.messages.clear();
            agent.session = Some(store);
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
            let after = pi_agent::estimate_context_tokens(&agent.messages);
            let folded_messages = messages_before.saturating_sub(agent.messages.len());
            let thousands = pi_tui::davinci::views::chrome::thousands;
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
                .filter(|part| matches!(part, pi_ai::MessageContent::ToolCall { .. }))
                .count();
            let images = agent
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .filter(|part| matches!(part, pi_ai::MessageContent::Image { .. }))
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
            agent.load_from_session(next);
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
            agent.load_from_session(next);
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(format!("cloned to {}", session_id(agent))))
        }
        SlashAction::Import(path) => {
            if path.is_empty() {
                return Ok(Done::Note("usage: /import <path.jsonl>".into()));
            }
            let expanded = pi_session::expand_tilde(&path);
            let next = JsonlSession::open(&expanded).map_err(|err| err.to_string())?;
            agent.load_from_session(next);
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(format!("imported {}", session_id(agent))))
        }
        SlashAction::Copy => match agent.last_assistant_text() {
            Some(text) => {
                pi_tui::copy_text(&text);
                Ok(Done::Said("copied the last reply to the clipboard".into()))
            }
            None => Ok(Done::Note("no agent messages to copy yet".into())),
        },
        SlashAction::Share => Ok(Done::Said(crate::share_current_session(agent)?)),
        SlashAction::Changelog => {
            let entries = crate::changelog::parse_changelog(&crate::changelog::changelog_path());
            let stored = crate::settings::load_merged_settings(&agent_dir, &agent.cwd);
            let text = match stored.last_changelog_version.as_deref() {
                Some(since) => crate::changelog::format_changelog_since(&entries, Some(since)),
                None => crate::changelog::format_changelog(&entries),
            };
            Ok(Done::Said(text))
        }
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
            let remembered = persist_model_choice(&agent.provider, &agent.model_id);
            Ok(Done::Said(match remembered {
                Ok(()) => format!("model {} / {}", agent.provider, agent.model_id),
                Err(err) => format!(
                    "model {} / {} · this run only ({err})",
                    agent.provider, agent.model_id
                ),
            }))
        }
        SlashAction::SetThinking(level) => {
            let Some(parsed_level) = pi_protocol::ThinkingLevel::parse(&level) else {
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
        SlashAction::OpenThinking => {
            open_thinking_sheet(agent, model);
            Ok(Done::Opened)
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
            model.keybindings = pi_tui::Keybindings::load(&agent_dir);
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
                    format!("keybindings · {} bindings", pi_tui::get_keybindings().len()),
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
                crate::native_extensions::NATIVE_COMMANDS
                    .iter()
                    .filter(|name| name.starts_with(prefix))
                    .count()
            };
            let native_tools = |prefix: &str| {
                crate::native_extensions::NATIVE_TOOLS
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
            let schema_tokens = serde_json::to_string(&pi_agent::tool_specs())
                .map(|text| text.len() / 4)
                .unwrap_or(0)
                + serde_json::to_string(
                    &crate::native_extensions::NativeExtensionHost::tool_specs(),
                )
                .map(|text| text.len() / 4)
                .unwrap_or(0);
            let window = agent.context_window.max(1) as f64;
            let builtin = pi_agent::tool_specs().len();
            let native_count = crate::native_extensions::NATIVE_TOOLS.len();
            let extension_count: usize = host.js.iter().map(|ext| ext.tools.len()).sum();
            let total = (builtin + native_count + extension_count).max(1) as f64;
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
        SlashAction::Trust => {
            let mut host = crate::loaded_extension_host(parsed);
            host.emit(ExtensionEvent::UiPromptStart {
                kind: "trust".into(),
            });
            host.emit(ExtensionEvent::ProjectTrust {
                path: agent.cwd.display().to_string(),
            });
            host.emit(ExtensionEvent::UiPromptEnd {
                kind: "trust".into(),
            });
            // The sheet says what the project would load (`6a`); enter moves
            // on to the decision itself.
            open_trust_sheet(agent, model);
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
            let mut storage = pi_ai::AuthStorage::create().map_err(|err| err.to_string())?;
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
        SlashAction::ScopedModels => Ok(Done::Said(scoped_models_summary(parsed, agent))),
        SlashAction::Mcp => {
            open_mcp_sheet(agent, model);
            Ok(Done::Opened)
        }
        SlashAction::Plan => {
            agent.set_plan_mode(true);
            model.plan_mode = true;
            Ok(Done::Said(
                "plan mode · mutations are off until /act".into(),
            ))
        }
        SlashAction::Act => {
            agent.set_plan_mode(false);
            model.plan_mode = false;
            Ok(Done::Said(
                "act · edits and shell commands may run again".into(),
            ))
        }
        SlashAction::ShowCost => Ok(Done::Said(crate::format_session_cost(parsed, agent))),
        SlashAction::ShowStatus => Ok(Done::Said(crate::format_session_status(parsed, agent))),
        SlashAction::Llama => Ok(Done::Said(format!(
            "llama.cpp server {}",
            std::env::var("LLAMA_BASE_URL")
                .unwrap_or_else(|_| crate::llama::DEFAULT_LLAMA_SERVER_URL.into())
        ))),
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
fn persist_model_choice(provider: &str, model_id: &str) -> Result<(), String> {
    let dir = crate::default_agent_dir();
    let mut stored = crate::settings::load_settings(&dir);
    stored.default_provider = Some(provider.to_string());
    stored.default_model = Some(model_id.to_string());
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

fn cycle_thinking(agent: &mut Agent, model: &mut Model) -> Option<String> {
    let current = agent.thinking_level.as_str();
    let next_index = model
        .thinking_levels
        .iter()
        .position(|level| level == current)
        .map(|index| (index + 1) % model.thinking_levels.len())
        .unwrap_or(0);
    let next = model.thinking_levels.get(next_index)?.clone();
    let parsed = pi_protocol::ThinkingLevel::parse(&next)?;
    agent.thinking_level = parsed;
    model.thinking_level = next.clone();
    Some(next)
}

fn adopt_model(parsed: &crate::args::Args, agent: &mut Agent, model: &mut Model) {
    if let Some(found) = crate::available_models(parsed)
        .into_iter()
        .find(|item| item.provider == agent.provider && item.id == agent.model_id)
    {
        agent.context_window = found.context_window;
    }
    model.model_index = model
        .models
        .iter()
        .position(|item| item.provider == agent.provider && item.id == agent.model_id)
        .unwrap_or(model.model_index);
    model.model_name = agent.model_id.clone();
    model.context.1 = agent.context_window;
    sync_thinking_state(agent, model);
}

/// The models `--models` pinned this run to, and the one actually in hand.
fn scoped_models_summary(parsed: &crate::args::Args, agent: &Agent) -> String {
    let mut rows = vec![format!("in hand  {} / {}", agent.provider, agent.model_id)];
    for spec in &parsed.models {
        rows.push(format!("scoped   {spec}"));
    }
    if rows.len() == 1 {
        rows.push("no --models scope for this run".into());
    }
    rows.join("\n")
}

/// Run the davinci TUI against a live agent until the user leaves.
pub fn run(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    raw: &[String],
    host: Arc<Mutex<ExtensionHost>>,
    migrated_auth_providers: &[String],
) -> Result<i32, String> {
    use pi_tui::davinci::app::{self, Flow};
    use pi_tui::davinci::runtime::Session;

    let cwd = agent.cwd.clone();
    let mut model = pi_tui::davinci::boot(raw, 100, 44);
    let session_dir = pi_session::default_session_dir();
    crate::davinci_sources::dress_from_workspace(&mut model, &cwd, &session_dir);
    // Every later re-read runs on this worker rather than the drawing thread.
    let dresser = crate::davinci_sources::WorkspaceDresser::start(cwd.clone(), session_dir.clone());
    crate::davinci_surfaces::dress_from_extensions(&mut model, &cwd, agent);
    model.model_name = agent.model_id.clone();
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
                &pi_tui::davinci::views::chrome::thousands(entry.context_window),
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
    model.permission_mode = agent
        .permissions
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .mode
        .as_str()
        .to_string();
    model.plan_mode = agent.plan_mode;
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
    model.keybindings = pi_tui::Keybindings::load(&crate::default_agent_dir());
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
    // Images pasted with ctrl+v, sent with the next prompt so a vision model
    // is reachable from this interface.
    let mut attached_images: Vec<pi_ai::MessageContent> = Vec::new();

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
    // The question the `Ask` instrument is currently putting, if any. It lives
    // here rather than in the model because only this module knows what the
    // rows mean.
    let mut pending: Option<Question> = None;
    if crate::settings::should_run_first_time_setup(&crate::settings::settings_path(
        &crate::default_agent_dir(),
    )) {
        pending = Some(Question::FirstRun);
        model.ask = Question::FirstRun.ask(agent);
        model.overlay = Some(Overlay::Ask);
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
    let mut openers: Vec<(String, Vec<pi_ai::MessageContent>)> = Vec::new();
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
        dresser.apply_ready(&mut model);
        if let Err(err) = terminal.draw(&model) {
            break Err(err.to_string());
        }

        let timeout = pi_tui::davinci::runtime::TICK.saturating_sub(last_tick.elapsed());
        match terminal.poll_event(timeout) {
            Ok(Some(event)) => match event {
                crossterm::event::Event::Key(key)
                    if key.kind != crossterm::event::KeyEventKind::Release =>
                {
                    // An extension's registered shortcut gets the chord before
                    // the shell's own keys, exactly as the legacy loop gives
                    // it. Resolution already refused the reserved chords.
                    let claimed = pi_tui::key_event_bytes(&key).and_then(|data| {
                        model
                            .extension_shortcuts
                            .iter()
                            .find(|(chord, _)| pi_tui::key_to_bytes(chord) == data)
                            .cloned()
                    });
                    if let Some((chord, path)) = claimed {
                        let mut shell = Shell {
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
                        let taken = pi_tui::key_event_bytes(&key).is_some_and(|data| {
                            let mut locked = host.lock().unwrap_or_else(|err| err.into_inner());
                            locked.dispatch_terminal_input(&data)
                        });
                        if taken {
                            let mut shell = Shell {
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
                            attached_images.push(pi_ai::MessageContent::Image {
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
                        && model.screen == pi_tui::davinci::model::Screen::Agent
                        && !model.codex_open()
                        && model.composer.trim().is_empty()
                        && model.double_escape_action != "none"
                    {
                        let now = Instant::now();
                        let doubled = last_escape.is_some_and(|prev| {
                            now.duration_since(prev)
                                < Duration::from_millis(pi_tui::DOUBLE_ESCAPE_MS)
                        });
                        if doubled {
                            last_escape = None;
                            match pi_tui::DoubleEscapeAction::parse(&model.double_escape_action) {
                                pi_tui::DoubleEscapeAction::Fork => {
                                    let mut shell = Shell {
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
                    let was = model.screen;
                    let next = match app::handle_key(&mut model, key) {
                        Flow::Quit => {
                            run_stop_hooks(&mut Shell {
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
                        Flow::CycleThinking => {
                            let cycled = cycle_thinking(agent, &mut model);
                            if let Some(level) = cycled.clone() {
                                let mut extension_host =
                                    host.lock().unwrap_or_else(|err| err.into_inner());
                                extension_host.runtime_thinking_level = level.clone();
                                extension_host.emit(
                                    crate::extension_host::ExtensionEvent::ThinkingLevelSelect {
                                        level,
                                    },
                                );
                            }
                            let mut shell = Shell {
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
                            match cycled {
                                // The header carries the new level, but a chord
                                // that only moves a token in the top-right row
                                // reads as a key that did nothing.
                                Some(level) => {
                                    let provider = shell.agent.provider.clone();
                                    let model_id = shell.agent.model_id.clone();
                                    match persist_thinking_choice(&provider, &model_id, &level) {
                                        Ok(()) => shell.say(&format!("thinking level {level}")),
                                        Err(err) => shell.say(&format!(
                                            "thinking level {level} · this run only ({err})"
                                        )),
                                    }
                                }
                                // Refusing in silence is what made shift+tab
                                // look broken rather than inapplicable.
                                None => {
                                    let why = if shell.model.thinking_levels.is_empty() {
                                        "no model in hand — /model picks one before thinking has levels"
                                    } else {
                                        "this model has one thinking level"
                                    };
                                    shell.note(why);
                                }
                            }
                            Next::Go
                        }
                        Flow::Continue | Flow::Interrupt => Next::Go,
                    };
                    // Recall is a search, so it runs when the instrument is
                    // summoned rather than being kept warm behind it.
                    if model.screen == pi_tui::davinci::model::Screen::Memoria
                        && was != pi_tui::davinci::model::Screen::Memoria
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
                crossterm::event::Event::Paste(text) => model.paste(&text),
                _ => {}
            },
            Ok(None) => {}
            Err(err) => break Err(err.to_string()),
        }

        if last_tick.elapsed() >= pi_tui::davinci::runtime::TICK {
            model.tick = model.tick.wrapping_add(1);
            last_tick = Instant::now();
            // A job that finishes between turns is news at once, not at
            // the next prompt.
            poll_jobs(&agent.tool_context.jobs, &mut model);
        }
    };

    crate::set_hosted_tui_active(false);
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
    let notices = crate::startup::collect_startup_notices(
        crate::VERSION,
        &stored,
        models_json_error,
        migrated_auth_providers.to_vec(),
    );
    for (kind, line) in crate::startup::format_notices(&notices) {
        out.push(Entry::Gap);
        // A warning wears the attention glyph; anything else is prose.
        if kind == "warning" || kind == "error" {
            out.push(Entry::tool(State::Attention, "instrumenta", &line, None));
        } else {
            out.push(Entry::prose(&line));
        }
    }

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
            .map(pi_tui::CustomMessage::text_content)
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
    crate::apply_session_calls(
        Some(shell.parsed),
        shell.agent,
        crate::SessionCallUi::Davinci(shell.model),
        &calls,
        false,
    );

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

    if wants_turn {
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
            Some("setEditorText") => model.composer = field(call, "text").into(),
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
    model.running = false;
    model.overlay = None;
    model.screen = screen;
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

/// `3a` — the full model catalog, from the live runtime snapshot. Every
/// catalog model stays listed; a row without a credential draws dimmed with
/// its note saying so, exactly as the legacy `/model` lists them locked.
fn open_models_sheet(parsed: &crate::args::Args, agent: &Agent, model: &mut Model) {
    let snapshot = crate::load_model_runtime(parsed);
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
            CatalogRow {
                name: key.clone(),
                detail: String::new(),
                window: pi_tui::davinci::views::chrome::thousands(entry.context_window),
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
    open_sheet(model, Screen::Models);
}

/// The model in hand first, then everything a credential unlocks (the
/// current provider's models ahead of the others), then the rest of the
/// catalogue — the row the user can act on is never buried under a thousand
/// providers they have not signed in to. Returns the current model's row.
fn order_catalog(catalog: &mut [CatalogRow], provider: &str, model_id: &str) -> usize {
    let current = |row: &CatalogRow| row.provider == provider && row.id == model_id;
    catalog.sort_by_key(|row| {
        (
            !current(row),
            row.credential != Credential::Ready,
            row.provider != provider,
        )
    });
    catalog.iter().position(current).unwrap_or(0)
}

/// `3b` — the settings sheet, from the same list the legacy overlay builds,
/// so both surfaces offer the same keys with the same ramps. A row whose
/// merged value differs from the user file's was set by the project.
fn open_settings_sheet(agent: &Agent, model: &mut Model) {
    let dir = crate::default_agent_dir();
    let user = crate::settings::load_settings(&dir);
    let merged = crate::settings::load_merged_settings(&dir, &agent.cwd);
    let user_list = pi_tui::interactive_settings_list(&crate::settings::to_interactive_config(
        &user, "davinci",
    ));
    let merged_list = pi_tui::interactive_settings_list(&crate::settings::to_interactive_config(
        &merged, "davinci",
    ));
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
                values: item.values,
                description: item.description.unwrap_or_default(),
                key: item.id,
                note: String::new(),
            }
        })
        .collect();
    model.settings_index = 0;
    open_sheet(model, Screen::Settings);
}

/// `3c` — the thinking sheet: every level this model supports, as the budget
/// it actually sends, with its share of the 64k ceiling and a warning when a
/// level would take a third of the window before the turn starts.
fn open_thinking_sheet(agent: &Agent, model: &mut Model) {
    let stored = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    let budgets = stored.thinking_budgets.clone();
    let levels = crate::current_runtime_model(agent)
        .map(|runtime| crate::get_supported_thinking_levels(&runtime))
        .unwrap_or_else(|| pi_protocol::ThinkingLevel::all().to_vec());
    let window = agent.context_window.max(1) as f64;
    model.thinking_rows = levels
        .iter()
        .map(|level| {
            let budget = if *level == pi_protocol::ThinkingLevel::Off {
                0u32
            } else {
                pi_ai::thinking_budget_for_level(*level, budgets.as_ref())
            };
            let of_window = budget as f64 / window;
            let warn = of_window >= 1.0 / 3.0;
            let maps_to = if budget == 0 {
                "disabled → none".to_string()
            } else if warn {
                format!("! {:.0}% of the window", of_window * 100.0)
            } else {
                let sent = pi_ai::clamp_reasoning(*level)
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
    open_sheet(model, Screen::Thinking);
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
    let bindings = pi_tui::get_keybindings();
    let mut instruments: Vec<(String, String)> = Vec::new();
    let mut composer: Vec<(String, String)> = Vec::new();
    let mut lists: Vec<(String, String)> = Vec::new();
    let mut other: Vec<(String, String)> = Vec::new();
    for binding in &bindings {
        let row = (binding.keys.join(", "), humanize_action(&binding.action));
        if binding.action.starts_with("davinci.") {
            instruments.push(row);
        } else if binding.action.starts_with("tui.editor.")
            || binding.action.starts_with("tui.input.")
        {
            composer.push(row);
        } else if binding.action.starts_with("tui.select.") {
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
    model.keymap = groups;
    model.keys_offset = 0;
    open_sheet(model, Screen::Keys);
}

/// `4a` — the session list, with what resuming each one would carry, from the
/// real store. Token counts are an estimate and say so.
fn open_resume_sheet(parsed: &crate::args::Args, agent: &Agent, model: &mut Model) {
    let session_dir = crate::resolved_session_dir(parsed, &agent.cwd);
    let mut found = pi_session::discover_sessions(&session_dir, None).unwrap_or_default();
    found.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    model.session_count = found.len();
    let now = pi_session::now_ms();
    model.resume_sessions = found
        .iter()
        .take(30)
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
                    pi_tui::davinci::views::chrome::thousands(
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
    open_sheet(model, Screen::Resume);
}

/// `4b` — the session tree, from the current session's entry graph: one node
/// per user turn, spacers carrying the trunk between them.
fn open_tree_sheet(agent: &Agent, model: &mut Model) -> bool {
    let Some(store) = agent.session.as_ref() else {
        return false;
    };
    let turn_text = |entry: &pi_session::SessionEntry| -> Option<String> {
        let message = entry.message.as_ref()?;
        if message.get("role").and_then(serde_json::Value::as_str) != Some("user") {
            return None;
        }
        let text = pi_tui::CustomMessage::text_content(message);
        let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
        Some(clip(line, 48))
    };
    let turns: Vec<(&pi_session::SessionEntry, String)> = store
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
        let by_id: std::collections::BTreeMap<&str, &pi_session::SessionEntry> = store
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
    open_sheet(model, Screen::Tree);
    true
}

/// `6a` — what this project would load if trusted, walked from the real
/// `.pi` directory and context files.
fn open_trust_sheet(agent: &Agent, model: &mut Model) {
    let cwd = &agent.cwd;
    let mut files: Vec<TrustFile> = Vec::new();
    let count_in = |dir: &std::path::Path| -> usize {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
    };
    let extensions = cwd.join(".pi").join("extensions");
    if extensions.is_dir() {
        for entry in std::fs::read_dir(&extensions)
            .into_iter()
            .flatten()
            .flatten()
        {
            files.push(TrustFile {
                state: State::Attention,
                path: format!(".pi\\extensions\\{}", entry.file_name().to_string_lossy()),
                detail: "runs as node, no sandbox".into(),
                risk_label: "executes code".into(),
            });
        }
    }
    let settings = cwd.join(".pi").join("settings.json");
    if settings.is_file() {
        let keys = std::fs::read_to_string(&settings)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value.as_object().map(|map| map.len()))
            .unwrap_or(0);
        files.push(TrustFile {
            state: State::Attention,
            path: ".pi\\settings.json".into(),
            detail: format!("{keys} keys, incl. limits and allowlists"),
            risk_label: "changes limits".into(),
        });
    }
    let skills = cwd.join(".pi").join("skills");
    if skills.is_dir() {
        files.push(TrustFile {
            state: State::Read,
            path: format!(".pi\\skills\\ ({})", count_in(&skills)),
            detail: "instructions loaded on demand".into(),
            risk_label: "prompt text".into(),
        });
    }
    let prompts = cwd.join(".pi").join("prompts");
    if prompts.is_dir() {
        files.push(TrustFile {
            state: State::Read,
            path: format!(".pi\\prompts\\ ({})", count_in(&prompts)),
            detail: "slash commands that expand to prompts".into(),
            risk_label: "prompt text".into(),
        });
    }
    let mut context_lines = 0usize;
    let mut context_names: Vec<&str> = Vec::new();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(text) = std::fs::read_to_string(cwd.join(name)) {
            context_lines += text.lines().count();
            context_names.push(name);
        }
    }
    if !context_names.is_empty() {
        files.push(TrustFile {
            state: State::Read,
            path: context_names.join(" · "),
            detail: format!("{context_lines} lines, prepended to every turn"),
            risk_label: "prompt text".into(),
        });
    }
    let store = crate::trust::ProjectTrustStore::open(&crate::default_agent_dir());
    let (trusted, ignored) = store.counts();
    let first_visit = store.get(cwd).is_none();
    model.project_trust = Some(ProjectTrustSheet {
        files,
        first_visit,
        path: cwd.display().to_string(),
        trusted: format!("{trusted} projects"),
        ignored: ignored.to_string(),
        store: crate::default_agent_dir()
            .join("trust.json")
            .display()
            .to_string(),
    });
    open_sheet(model, Screen::Trust);
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
    let hunk_of = |path: &str| -> (Vec<Hunk>, String) {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["diff", "--unified=2", "HEAD", "--", path])
            .output();
        let Ok(output) = output else {
            return (Vec::new(), String::new());
        };
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let hunk_count = text.lines().filter(|line| line.starts_with("@@")).count();
        let mut rows = Vec::new();
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with("@@") {
                if inside {
                    break; // the first hunk is enough for the sheet
                }
                inside = true;
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
        (rows, note)
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
            let (hunk, hunk_note) = if untracked {
                (Vec::new(), "new file · untracked".to_string())
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
    let last_indexed = json_str(value, "lastIndexed");
    if !last_indexed.is_empty() {
        health.push((State::Done, format!("last indexed {last_indexed}")));
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
    VectorIndex {
        repo: json_str(value, "repoId"),
        repo_records: records.to_string(),
        total_records: records.to_string(),
        injection_cap: String::new(),
        floor: String::new(),
        kinds: Vec::new(),
        embeddings: "ollama".into(),
        embed_host: json_str(value, "ollama"),
        store: "qdrant".into(),
        collection: format!("collection {}", json_str(value, "collection")),
        extraction: String::new(),
        config: crate::default_agent_dir()
            .join("vector-memory.json")
            .display()
            .to_string(),
        health,
        ..Default::default()
    }
}

/// `5c` — the governor's ledger, from the real `governor-status` payload plus
/// a look into its store directory.
fn governor_sheet(value: &serde_json::Value) -> GovernorSheet {
    let count = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let store_dir = json_str(value, "store");
    let stored: Vec<GovernorStored> = std::fs::read_dir(&store_dir)
        .into_iter()
        .flatten()
        .flatten()
        .take(8)
        .map(|entry| {
            let size = entry
                .metadata()
                .map(|meta| {
                    let bytes = meta.len();
                    if bytes >= 1_000 {
                        format!("{} KB", bytes / 1_000)
                    } else {
                        format!("{bytes} B")
                    }
                })
                .unwrap_or_default();
            GovernorStored {
                id: entry.file_name().to_string_lossy().to_string(),
                tool: String::new(),
                call: String::new(),
                size,
                stale: false,
            }
        })
        .collect();
    GovernorSheet {
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
        ],
        stored,
        store_dir: format!("{store_dir} · dropped when the session ends"),
        ..Default::default()
    }
}

/// `5d` — the security scan sheet, from the structured `sec-status` payload.
fn security_sheet(value: &serde_json::Value) -> SecurityScan {
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
    SecurityScan {
        validated,
        candidates: candidates.max(findings.len() as u32),
        fraction: if candidates == 0 {
            1.0
        } else {
            validated as f64 / candidates as f64
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
        report: "report.md in the scan artifact · /sec-report".into(),
        ..Default::default()
    }
}

/// `5a` — the graph run sheet, from the real `graph-status` payload.
fn graph_sheet(value: &serde_json::Value) -> Option<GraphRunSheet> {
    let run = value.get("run").filter(|run| !run.is_null())?;
    let current_phase = json_str(run, "phase");
    let phases: Vec<(String, State)> = {
        let order = [
            "classify",
            "investigate",
            "plan",
            "implement",
            "verify",
            "review",
            "done",
        ];
        let at = order
            .iter()
            .position(|phase| *phase == current_phase)
            .unwrap_or(0);
        order
            .iter()
            .enumerate()
            .map(|(index, phase)| {
                let state = match index.cmp(&at) {
                    std::cmp::Ordering::Less => State::Done,
                    std::cmp::Ordering::Equal => {
                        if current_phase == "done" {
                            State::Done
                        } else {
                            State::Active
                        }
                    }
                    std::cmp::Ordering::Greater => State::Queued,
                };
                (phase.to_string(), state)
            })
            .collect()
    };
    let empty = Vec::new();
    let tasks: Vec<GraphTask> = run
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .map(|task| {
            let status = json_str(task, "status");
            let usage = task
                .get("usage")
                .map(|usage| {
                    let field = |key: &str| {
                        usage
                            .get(key)
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0)
                    };
                    format!(
                        "{}↑ {}↓",
                        pi_tui::davinci::views::chrome::thousands(field("inputTokens")),
                        pi_tui::davinci::views::chrome::thousands(field("outputTokens")),
                    )
                })
                .unwrap_or_default();
            GraphTask {
                id: format!("{} {}", json_str(task, "id"), json_str(task, "role")),
                policy: json_str(task, "role"),
                artifact: task
                    .get("artifactFile")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending")
                    .to_string(),
                usage,
                state: match status.as_str() {
                    "done" | "succeeded" => State::Done,
                    "running" | "started" => State::Active,
                    "failed" => State::Failed,
                    _ => State::Queued,
                },
            }
        })
        .collect();
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
    let number = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    Some(GraphRunSheet {
        goal: json_str(run, "goal"),
        phases,
        shape: Vec::new(),
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
        workers: format!(
            "{} of {}",
            number(&counters, "workersSpawned"),
            number(&budgets, "maxWorkers")
        ),
        parallel: number(&budgets, "maxParallelWorkers").to_string(),
        cycles: format!(
            "{} of {}",
            number(&counters, "revisionCycles"),
            number(&budgets, "maxRevisionCycles")
        ),
        replans: format!(
            "{} of {}",
            number(&counters, "replans"),
            number(&budgets, "maxReplans")
        ),
        artifacts: format!(".pi\\graph\\{}\\", json_str(run, "runId")),
        ..Default::default()
    })
}

/// Everything a composer line or a chosen row may need. Bundled because the
/// borrow checker will not let the loop hand out eight `&mut` pieces at once.
struct Shell<'a> {
    parsed: &'a crate::args::Args,
    agent: &'a mut Agent,
    model: &'a mut Model,
    terminal: &'a mut pi_tui::davinci::runtime::Session,
    host: &'a Arc<Mutex<ExtensionHost>>,
    pending: &'a mut Option<Question>,
    cwd: &'a std::path::Path,
    dresser: &'a crate::davinci_sources::WorkspaceDresser,
    /// Images pasted from the clipboard, waiting for the next prompt.
    images: &'a mut Vec<pi_ai::MessageContent>,
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
                self.model.ask_index = 0;
                self.model.overlay = Some(Overlay::Ask);
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
        match pi_tui::davinci::runtime::Session::open() {
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
        match pi_session::JsonlSession::open(std::path::Path::new(path)) {
            Ok(store) => {
                self.agent.load_from_session(store);
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

    match pi_agent::execute_tool(
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
        if let Some(next) = run_extension_command(shell, line) {
            return next;
        }
        // `/diff` is davinci's own: the Δ review sheet (`6d`) over the real
        // working tree. Checked after extensions so one may still claim it.
        if line.trim() == "/diff" {
            return open_diff_sheet(shell);
        }
        // `/permissions` likewise: the mode and rules in force, or a new
        // mode for the rest of the session.
        if let Some(rest) = line.trim().strip_prefix("/permissions") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return permissions_command(shell, rest.trim());
            }
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
fn submit_prompt(shell: &mut Shell<'_>, text: &str, images: &[pi_ai::MessageContent]) -> Next {
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
    let expanded = pi_agent::expand_user_text(&text, &shell.agent.skills, &shell.agent.templates);
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
    shell.agent.prompt_with(&expanded, &images);
    let next = run_turns(shell);
    shell.redress();
    next
}

/// One row of an open instrument, chosen with enter.
fn on_choice(shell: &mut Shell<'_>, choice: Choice) -> Next {
    match choice {
        Choice::Command { name, kind } => match kind.as_str() {
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
            match persist_model_choice(&item.provider, &item.id) {
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
            match persist_model_choice(&row.provider, &row.id) {
                Ok(()) => shell.say(&format!("model {} / {}", row.provider, row.id)),
                Err(err) => shell.say(&format!(
                    "model {} / {} · this run only ({err})",
                    row.provider, row.id
                )),
            }
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
                Ok(Done::Said(text)) => shell.say(&text),
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
    }
}

fn apply_permission_row(shell: &mut Shell<'_>, index: usize) -> Next {
    let Some(row) = shell.model.permission_rows.get(index).cloned() else {
        return Next::Go;
    };
    if row.kind == "mode" {
        if let Some(mode) = PermissionMode::parse(&row.key) {
            shell
                .agent
                .permissions
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .mode = mode;
            shell.model.permission_mode = mode.as_str().to_string();
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
        "user" => {
            let list = if row.detail.starts_with("deny") {
                "deny"
            } else {
                "allow"
            };
            let path = crate::settings::settings_path(&crate::default_agent_dir());
            let _ = crate::permissions::forget_file_rule(&path, list, &row.key);
        }
        "project" => {
            let list = if row.detail.starts_with("deny") {
                "deny"
            } else {
                "allow"
            };
            let path = crate::permissions::project_settings_path(shell.cwd);
            let _ = crate::permissions::forget_file_rule(&path, list, &row.key);
        }
        _ => {}
    }
    open_permissions_sheet(shell);
    Next::Go
}

/// `3b` — advance a setting to the next value on its ramp, write it through
/// the same store the legacy overlay writes, and re-honour it at once where
/// davinci reads it live.
fn cycle_setting(shell: &mut Shell<'_>, index: usize) -> Next {
    let Some(row) = shell.model.settings_rows.get_mut(index) else {
        return Next::Go;
    };
    if row.values.is_empty() {
        return Next::Go;
    }
    let at = row
        .values
        .iter()
        .position(|value| value == &row.value)
        .unwrap_or(0);
    let next = row.values[(at + 1) % row.values.len()].clone();
    let key = row.key.clone();
    row.value = next.clone();
    row.project = false;
    if let Err(err) = crate::persist_interactive_setting(&format!("{key}={next}")) {
        shell.note(&err);
        return Next::Go;
    }
    crate::sync_agent_from_settings(shell.agent);
    match key.as_str() {
        "autocomplete-max-visible" => {
            if let Ok(rows) = next.parse::<usize>() {
                shell.model.suggestion_rows = rows.clamp(3, 20);
            }
        }
        "terminal-progress" => shell.model.terminal_progress = next == "true",
        "double-escape-action" => shell.model.double_escape_action = next.clone(),
        "show-tool-output" => shell.model.show_tool_output = next == "true",
        _ => {}
    }
    Next::Go
}

/// Carry out the row chosen from a question.
fn answer(shell: &mut Shell<'_>, question: &Question, index: usize) -> Result<String, String> {
    match question {
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
            let mut storage = pi_ai::AuthStorage::create().map_err(|err| err.to_string())?;
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
                    pi_ai::MessageContent::Text { text } => Some(text.as_str()),
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
            .unwrap_or_else(|err| err.into_inner()) = pi_agent::TodoList::default();
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
            pi_agent::JobStatus::Running => State::Active,
            pi_agent::JobStatus::Exited(0) => State::Done,
            pi_agent::JobStatus::Exited(_) => State::Failed,
            pi_agent::JobStatus::Killed => State::Skipped,
        };
        shell.model.transcript.push(
            Entry::tool(
                state,
                "manus",
                &format!("job {} · {}", job.id, clip(&job.command, 50)),
                Some(&pi_agent::jobs::format_elapsed(job.elapsed)),
            )
            .summarised(&job.status.describe()),
        );
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
fn permissions_command(shell: &mut Shell<'_>, arg: &str) -> Next {
    if !arg.is_empty() {
        match PermissionMode::parse(arg) {
            Some(mode) => {
                shell
                    .agent
                    .permissions
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .mode = mode;
                shell.model.permission_mode = mode.as_str().to_string();
                shell.say(&format!(
                    "permission mode {} · {} · this session",
                    mode.as_str(),
                    mode.describe()
                ));
            }
            None => shell.note(&format!(
                "no permission mode {arg} — read-only, ask, edits or auto"
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
            label: mode.as_str().into(),
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
        if pi_ai::PROVIDER_SPECS
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
    model.context = (
        pi_agent::estimate_context_tokens(&agent.messages),
        agent.context_window,
    );
    model.model_name = agent.model_id.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    fn assistant(text: &str) -> pi_ai::ChatMessage {
        pi_ai::ChatMessage {
            role: "assistant".into(),
            content: vec![pi_ai::MessageContent::Text { text: text.into() }],
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
        let agent = pi_agent::Agent::new("test");
        let items = corpus(&agent, &[], &[]);
        assert!(items
            .iter()
            .any(|item| item.name == "/diff" && item.kind == "command"));
    }

    #[test]
    fn cycling_thinking_advances_supported_levels_and_syncs_chrome() {
        let mut agent = pi_agent::Agent::new("test");
        agent.thinking_level = pi_protocol::ThinkingLevel::Low;
        let mut m = model();
        m.thinking_levels = vec!["off".into(), "low".into(), "medium".into(), "high".into()];
        m.thinking_level = "low".into();

        assert_eq!(
            cycle_thinking(&mut agent, &mut m).as_deref(),
            Some("medium")
        );
        assert_eq!(agent.thinking_level.as_str(), "medium");
        assert_eq!(m.thinking_level, "medium");
    }

    fn model() -> Model {
        Model::new(
            pi_tui::davinci::theme::Theme::da_vinci(
                pi_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            40,
            true,
        )
    }

    fn partial(text: &str) -> pi_ai::AssistantMessage {
        pi_ai::AssistantMessage {
            id: "m".into(),
            role: "assistant".into(),
            content: vec![pi_ai::ContentBlock::Text { text: text.into() }],
            model: "fixture".into(),
            usage: None,
            stop_reason: None,
            error_message: None,
        }
    }

    fn update(event: pi_ai::AssistantMessageEvent) -> AgentEvent {
        AgentEvent::MessageUpdate {
            message: Arc::new(pi_ai::assistant_to_chat(event.message())),
            assistant_message_event: event,
        }
    }

    #[test]
    fn text_deltas_stream_into_one_prose_entry_that_message_end_keeps() {
        use pi_ai::AssistantMessageEvent as Ev;
        let mut m = model();
        let mut turn = Turn::default();
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::MessageStart {
                message: pi_ai::ChatMessage::text("assistant", ""),
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
                message: pi_ai::ChatMessage::text("assistant", "Hello"),
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
                message: pi_ai::ChatMessage::text("assistant", "whole reply"),
            },
        );
        assert!(matches!(
            m.transcript.last(),
            Some(Entry::Prose(text)) if text == "whole reply"
        ));
    }

    #[test]
    fn reasoning_streams_live_and_collapses_when_the_text_starts() {
        use pi_ai::AssistantMessageEvent as Ev;
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
        use pi_ai::AssistantMessageEvent as Ev;
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
    fn the_catalogue_opens_on_the_model_in_hand_with_usable_rows_first() {
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
        };
        let mut catalog = vec![
            row("amazon-bedrock", "nova", false),
            row("anthropic", "claude", true),
            row("openai-codex", "gpt-5", true),
            row("openai-codex", "gpt-5-mini", true),
            row("xai", "grok", false),
        ];
        let index = order_catalog(&mut catalog, "openai-codex", "gpt-5-mini");
        assert_eq!(index, 0);
        let names: Vec<&str> = catalog.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "openai-codex/gpt-5-mini",
                "openai-codex/gpt-5",
                "anthropic/claude",
                "amazon-bedrock/nova",
                "xai/grok",
            ]
        );
    }

    #[test]
    fn what_the_session_found_is_said_under_the_mark_not_in_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let mut agent = pi_agent::Agent::new("test");
        agent.cwd = dir.path().to_path_buf();
        agent.context_files.push(pi_agent::ContextFile {
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

    fn approval(tool: &str, subject: &str, outside: bool) -> ToolApprovalRequest {
        ToolApprovalRequest {
            tool_call_id: "call_1".into(),
            tool: tool.into(),
            args: json!({}),
            subject: subject.into(),
            summary: pi_agent::summary_of(tool, subject),
            session_rule: pi_agent::session_rule_for(tool, subject).to_string(),
            outside_project: outside,
            mode: PermissionMode::Ask,
        }
    }

    #[test]
    fn the_permission_panel_offers_always_only_in_a_trusted_project() {
        let ask = permission_ask(&approval("bash", "git status --short", false), true);
        assert_eq!(ask.title, "LICENTIA");
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
                "deny"
            ]
        );
        assert_eq!(ask.items[1].detail, "bash(git status *) until pi exits");
        assert_eq!(
            ask.items[2].detail,
            "bash(git status *) saved to .pi/settings.json"
        );

        let ask = permission_ask(&approval("write", "../out.txt", true), false);
        let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, ["allow once", "allow for this session", "deny"]);
        assert_eq!(ask.note, "write · ../out.txt · outside the project");
    }

    #[test]
    fn permission_rows_map_to_decisions_in_both_shapes() {
        use ToolApprovalDecision::*;
        assert_eq!(permission_choice(0, true), Some(AllowOnce));
        assert_eq!(permission_choice(1, true), Some(AllowForSession));
        assert_eq!(permission_choice(2, true), Some(AllowAlways));
        assert_eq!(permission_choice(3, true), Some(Deny));
        assert_eq!(permission_choice(4, true), None);
        assert_eq!(permission_choice(2, false), Some(Deny));
        assert_eq!(permission_choice(3, false), None);
    }

    #[test]
    fn under_the_permission_panel_esc_denies_and_enter_chooses_the_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
        let abort = Arc::new(AtomicBool::new(false));
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
            approval_key(&mut m, key(KeyCode::Esc), true, &abort),
            Some(ToolApprovalDecision::Deny)
        );
        assert_eq!(m.overlay, None);
        assert!(
            !abort.load(Ordering::Relaxed),
            "esc refuses; it does not interrupt"
        );

        let mut m = open();
        assert_eq!(approval_key(&mut m, key(KeyCode::Down), true, &abort), None);
        assert_eq!(
            m.overlay,
            Some(Overlay::Ask),
            "moving keeps the question up"
        );
        assert_eq!(
            approval_key(&mut m, key(KeyCode::Enter), true, &abort),
            Some(ToolApprovalDecision::AllowForSession)
        );
        assert_eq!(m.overlay, None);

        let mut m = open();
        assert_eq!(
            approval_key(
                &mut m,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                true,
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
            target_of("read", &json!({"path": "crates/pi-tui/src/lib.rs"})),
            "read crates/pi-tui/src/lib.rs"
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
            "/trust",
            "/reload",
            "/import a.jsonl",
            "/share",
            "/changelog",
            "/session",
            "/scoped-models",
            "/llama",
            "/mcp",
            "/plan",
            "/act",
            "/cost",
            "/status",
            "/thinking",
            "/thinking high",
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
        let mut agent = pi_agent::Agent::new("test");
        assert_eq!(recall_query(&m, &agent), "", "nothing typed, nothing asked");

        agent.messages.push(pi_ai::ChatMessage {
            role: "user".into(),
            content: vec![pi_ai::MessageContent::Text {
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
        let mut agent = pi_agent::Agent::new("test");
        agent.thinking_level = pi_protocol::ThinkingLevel::Medium;

        let trust = Question::Trust {
            path: "C:\\work\\pi-rust".into(),
            options: crate::trust::get_project_trust_options(std::path::Path::new("."), false),
        };
        let panel = trust.ask(&agent);
        assert_eq!(panel.title, "FIDES");
        assert!(panel.note.contains("C:\\work\\pi-rust"), "{}", panel.note);
        assert!(!panel.items.is_empty());

        let credentials = Question::Logout {
            providers: vec!["anthropic".into(), "openai".into()],
        }
        .ask(&agent);
        assert_eq!(credentials.title, "CLAVES");
        assert_eq!(credentials.items.len(), 2);
        assert_eq!(credentials.items[0].label, "anthropic");
    }

    #[test]
    fn first_run_asks_only_what_davinci_cannot_decide_for_itself() {
        let agent = pi_agent::Agent::new("test");
        let panel = Question::FirstRun.ask(&agent);
        // The old setup asked for a theme too; there is one palette here, so
        // the only question left is the one about the user, not the terminal.
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

        let mut agent = pi_agent::Agent::new("test");
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

        let mut agent = pi_agent::Agent::new("test");
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
        let agent = pi_agent::Agent::new("test");
        let panel = Question::Logout { providers: vec![] }.ask(&agent);
        assert!(panel.items.is_empty());
        assert_eq!(panel.key, "/logout");
    }

    #[test]
    fn the_model_scope_names_the_model_in_hand_even_with_no_scope_set() {
        let mut agent = pi_agent::Agent::new("test");
        agent.provider = "anthropic".into();
        agent.model_id = "claude-opus-5".into();
        let summary = scoped_models_summary(&crate::args::Args::default(), &agent);
        assert!(summary.contains("anthropic / claude-opus-5"), "{summary}");
        assert!(summary.contains("no --models scope"), "{summary}");
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
            pi_tui::davinci::theme::Theme::da_vinci(
                pi_tui::davinci::theme::ColorDepth::TrueColor,
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
            pi_tui::davinci::theme::Theme::da_vinci(
                pi_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            44,
            true,
        );
        m.slash_commands = ["graph-view", "graph-status", "compact"]
            .into_iter()
            .map(|name| pi_tui::SlashCommandSpec {
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
            pi_tui::davinci::theme::Theme::da_vinci(
                pi_tui::davinci::theme::ColorDepth::TrueColor,
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
            pi_tui::SlashCommandSpec {
                name: "graph-view".into(),
                description: "Tail a graph worker's live transcript.".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
            pi_tui::SlashCommandSpec {
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
            pi_tui::davinci::theme::Theme::da_vinci(
                pi_tui::davinci::theme::ColorDepth::TrueColor,
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
            pi_ai::ChatMessage {
                role: "user".into(),
                content: vec![pi_ai::MessageContent::Text {
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
            pi_ai::ChatMessage {
                role: "assistant".into(),
                content: vec![
                    pi_ai::MessageContent::Text {
                        text: "looking".into(),
                    },
                    pi_ai::MessageContent::ToolCall {
                        id: "call-1".into(),
                        name: "read".into(),
                        arguments: json!({"path": "src/lib.rs"}),
                    },
                    pi_ai::MessageContent::ToolCall {
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
            pi_ai::ChatMessage {
                role: "tool".into(),
                content: vec![pi_ai::MessageContent::Text {
                    text: "one\ntwo\nthree".into(),
                }],
                tool_call_id: Some("call-1".into()),
                tool_name: Some("read".into()),
                is_error: None,
                extra: Default::default(),
            },
            pi_ai::ChatMessage {
                role: "tool".into(),
                content: vec![pi_ai::MessageContent::Text {
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
        let ok = pi_agent::JobNotice {
            id: 1,
            command: "cargo build".into(),
            status: pi_agent::JobStatus::Exited(0),
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
        let fail = pi_agent::JobNotice {
            id: 2,
            command: "cargo test".into(),
            status: pi_agent::JobStatus::Exited(1),
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
        extra.insert("customType".into(), json!(pi_agent::JOB_NOTICE_TYPE));
        extra.insert("jobId".into(), json!(1));
        let notice = pi_agent::JobNotice {
            id: 1,
            command: "cargo build".into(),
            status: pi_agent::JobStatus::Exited(0),
            elapsed: Duration::from_millis(31_200),
            tail: vec!["Finished".into()],
        };
        let message = pi_ai::ChatMessage {
            role: "user".into(),
            content: vec![pi_ai::MessageContent::Text {
                text: notice.message_text(),
            }],
            extra,
            ..pi_ai::ChatMessage::default()
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
}
