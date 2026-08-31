//! The davinci TUI, driven by the real agent.
//!
//! The shell in `pi_tui::davinci` knows nothing about agents; this module owns
//! the loop that turns a sent composer line into an agent turn and the agent's
//! events back into transcript blocks (`docs/ui/design.md` §6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pi_agent::{Agent, AgentEvent, EventSink};
use pi_tui::davinci::model::{
    Ask, Choice, CorpusItem, Entry, Model, ModelItem, Overlay, PickerItem, Step,
};
use pi_tui::davinci::theme::State;

use crate::extension_host::ExtensionHost;

/// Which instrument a tool belongs to (design.md §5). Shell execution is
/// Manus; everything else the agent reaches for is Instrumenta.
pub fn instrument_of(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" | "powershell" => "manus",
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
        "read" | "ls" => State::Read,
        "grep" | "find" => State::Search,
        name if name.starts_with("memory") => State::Search,
        "edit" | "write" => State::Delta,
        _ => State::Done,
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
    match tool_name {
        "read" => format!("read {}", field("path")),
        "ls" => format!("list {}", field("path")),
        "write" => format!("write {}", field("path")),
        "edit" => format!("edit {}", field("path")),
        "grep" => format!("search \"{}\"", field("pattern")),
        "find" => format!("find \"{}\"", field("pattern")),
        "bash" | "powershell" => field("command"),
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
        "read" | "ls" => "studying",
        "grep" | "find" => "surveying",
        "bash" | "powershell" => "testing",
        "edit" | "write" => "constructing",
        name if name.starts_with("memory") => "recalling",
        name if name.starts_with("graph") => "tracing",
        _ => "working",
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
/// code (design.md §6).
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

/// The transcript state a turn builds up, so events can find the block they
/// belong to without searching the transcript.
#[derive(Default)]
struct Turn {
    /// `tool_call_id` -> (index in the transcript, when it started).
    open: Vec<(String, usize, Instant)>,
    studio: Option<usize>,
    said_something: bool,
}

impl Turn {
    fn start_tool(
        &mut self,
        model: &mut Model,
        tool_call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        let index = model.transcript.len();
        model.transcript.push(Entry::tool(
            state_of(tool_name, false),
            instrument_of(tool_name),
            &target_of(tool_name, args),
            None,
        ));
        self.open
            .push((tool_call_id.to_string(), index, Instant::now()));
        self.push_step(model, tool_name, args);
    }

    fn end_tool(
        &mut self,
        model: &mut Model,
        tool_call_id: &str,
        tool_name: &str,
        result: &serde_json::Value,
        is_error: bool,
    ) {
        let Some(position) = self.open.iter().position(|(id, _, _)| id == tool_call_id) else {
            return;
        };
        let (_, index, started) = self.open.remove(position);
        if let Some(Entry::Tool {
            state, duration, ..
        }) = model.transcript.get_mut(index)
        {
            *state = state_of(tool_name, is_error);
            *duration = Some(duration_of(started.elapsed()));
        }
        if is_error {
            // The detail belongs directly under the line that failed.
            let mut at = index + 1;
            for line in failure_lines(result) {
                model.transcript.insert(at, Entry::detail(&line));
                at += 1;
                self.shift(index, 1);
            }
        }
        self.finish_step(model);
    }

    /// Keep the recorded indices valid when detail rows are spliced in.
    fn shift(&mut self, after: usize, by: usize) {
        for (_, index, _) in self.open.iter_mut() {
            if *index > after {
                *index += by;
            }
        }
        if let Some(studio) = self.studio.as_mut() {
            if *studio > after {
                *studio += by;
            }
        }
    }

    fn push_step(&mut self, model: &mut Model, tool_name: &str, args: &serde_json::Value) {
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
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            if let Some(step) = steps.last_mut() {
                step.state = State::Done;
            }
        }
    }

    fn close(&mut self, model: &mut Model, interrupted: bool) {
        if let Some(Entry::Studio(steps)) = self.studio.and_then(|i| model.transcript.get_mut(i)) {
            for step in steps.iter_mut() {
                if step.state == State::Active {
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
    }
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
        } => turn.end_tool(model, tool_call_id, tool_name, result, *is_error),

        AgentEvent::MessageEnd { message } if message.role == "assistant" => {
            let text = pi_ai::content_text(&message.content);
            if !text.trim().is_empty() {
                model.transcript.push(Entry::Gap);
                model.transcript.push(Entry::prose(text.trim()));
                turn.said_something = true;
            }
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
fn mid_turn_key(model: &mut Model, key: crossterm::event::KeyEvent, abort: &Arc<AtomicBool>) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            abort.store(true, Ordering::Relaxed);
            model.interrupt();
        }
        KeyCode::Char('c') if ctrl => {
            abort.store(true, Ordering::Relaxed);
            model.interrupt();
        }
        KeyCode::Char('j') if ctrl => model.newline(),
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
}

/// A turn and everything queued behind it, in the order it was typed. Each
/// follow-up opens its own block, exactly as if it had been sent by hand.
fn run_turns(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    model: &mut Model,
    session: &mut pi_tui::davinci::runtime::Session,
    host: Arc<Mutex<ExtensionHost>>,
) -> std::io::Result<()> {
    loop {
        run_turn(parsed, agent, model, session, host.clone())?;
        if model.queued.is_empty() {
            return Ok(());
        }
        let text = model.queued.remove(0);
        let expanded = pi_agent::expand_user_text(&text, &agent.skills, &agent.templates);
        agent.prompt(&expanded);
        model.transcript.push(Entry::Gap);
        model.transcript.push(Entry::user(&text));
        model.transcript.push(Entry::Gap);
        model.transcript.push(Entry::agent("davinci"));
        model.running = true;
    }
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

    let mut turn = Turn::default();
    let mut last_tick = Instant::now();

    std::thread::scope(|scope| -> std::io::Result<()> {
        let worker = scope
            .spawn(|| crate::complete_prompt_with_host(parsed, agent, Some(host.clone()), false));

        loop {
            while let Ok(event) = event_rx.try_recv() {
                apply(model, &mut turn, &event);
            }
            session.draw(model)?;

            if worker.is_finished() {
                break;
            }
            if crossterm::event::poll(Duration::from_millis(40))? {
                match crossterm::event::read()? {
                    crossterm::event::Event::Key(key)
                        if key.kind != crossterm::event::KeyEventKind::Release =>
                    {
                        mid_turn_key(model, key, &abort);
                    }
                    crossterm::event::Event::Resize(width, height) => {
                        model.width = width;
                        model.height = height;
                    }
                    _ => {}
                }
            }
            if last_tick.elapsed() >= pi_tui::davinci::runtime::TICK {
                model.tick = model.tick.wrapping_add(1);
                last_tick = Instant::now();
            }
        }

        let _ = worker.join();
        Ok(())
    })?;

    while let Ok(event) = event_rx.try_recv() {
        apply(model, &mut turn, &event);
    }
    let interrupted = abort.load(Ordering::Relaxed);
    turn.close(model, interrupted);

    if interrupted {
        model.transcript.push(Entry::Gap);
        model.transcript.push(Entry::tool(
            State::Skipped,
            "manus",
            "interrupted · the transcript is kept",
            None,
        ));
    } else if !turn.said_something {
        model.transcript.push(Entry::Gap);
        model.transcript.push(Entry::tool(
            State::Attention,
            "manus",
            "the model returned no text",
            None,
        ));
    }

    agent.abort_signal = None;
    agent.event_sink = None;
    model.running = false;

    // Extensions may have asked for rows while the turn ran.
    if let Ok(host) = host.lock() {
        apply_ui_calls(model, &host.ui_calls);
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
            pi_tui::davinci::views::chrome::thousands(waste.missed_tokens as u64)
        ),
        None,
    ));
}

/// Everything already in the session, as transcript blocks, so a resumed
/// session opens where it left off rather than empty.
pub fn transcript_from(messages: &[pi_ai::ChatMessage]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for message in messages {
        let text = pi_ai::content_text(&message.content);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if !entries.is_empty() {
            entries.push(Entry::Gap);
        }
        match message.role.as_str() {
            "user" => entries.push(Entry::user(text)),
            "assistant" => {
                entries.push(Entry::agent("davinci"));
                entries.push(Entry::Gap);
                entries.push(Entry::prose(text));
            }
            _ => {
                entries.pop();
            }
        }
    }
    entries
}

/// Everything Instrumenta can reach: the real slash commands, the real tools,
/// and the sessions already on disk (`1d`).
pub fn corpus(agent: &Agent, sessions: &[pi_tui::davinci::model::SessionItem]) -> Vec<CorpusItem> {
    let mut items: Vec<CorpusItem> = crate::slash::builtin_slash_commands()
        .into_iter()
        .map(|command| {
            CorpusItem::new(
                &format!("/{}", command.name),
                &command.description,
                "command",
            )
        })
        .collect();

    for tool in &agent.tools {
        items.push(CorpusItem::new(tool, instrument_of(tool), "tool"));
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

/// What the composer line means. A `/` line is a command, everything else is a
/// prompt (design.md §6: the composer is the only input).
pub enum Sent {
    Prompt(String),
    Quit,
    /// Say something back without asking the model.
    Say(String),
    /// Summon one of the instruments the shell already owns.
    Open(pi_tui::davinci::model::Overlay),
    /// A command that needs the agent and the session to carry it out.
    Command(crate::slash::SlashAction),
}

pub fn classify(line: &str) -> Sent {
    use crate::slash::SlashAction;
    use pi_tui::davinci::model::Overlay;

    match crate::slash::parse_line(line) {
        SlashAction::Prompt(text) => Sent::Prompt(text),
        SlashAction::Quit => Sent::Quit,
        SlashAction::Status(text) => Sent::Say(text),
        // The instruments the design already gives these commands a home in.
        SlashAction::OpenModel => Sent::Open(Overlay::Cogitator),
        SlashAction::Resume => Sent::Open(Overlay::Sessions),
        SlashAction::Settings | SlashAction::Hotkeys => Sent::Open(Overlay::Instrumenta),
        other => Sent::Command(other),
    }
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
    /// Summon an instrument instead of printing anything.
    Open(pi_tui::davinci::model::Overlay),
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
    /// How hard the model should think.
    Thinking,
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
    pub fn ask(&self, agent: &Agent) -> Ask {
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
            Question::Thinking => Ask {
                title: "MEDITATIO".into(),
                name: "THINKING".into(),
                key: "/thinking".into(),
                note: format!("in hand: {}", agent.thinking_level.as_str()),
                items: THINKING_LEVELS
                    .iter()
                    .map(|level| {
                        PickerItem::new(
                            level,
                            if *level == agent.thinking_level.as_str() {
                                "in hand"
                            } else {
                                ""
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
        SlashAction::OpenModel => Ok(Done::Open(Overlay::Cogitator)),
        SlashAction::Resume => Ok(Done::Open(Overlay::Sessions)),
        SlashAction::Settings | SlashAction::Hotkeys => Ok(Done::Open(Overlay::Instrumenta)),

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
            let result = agent.compact(instructions.as_deref());
            if result.compacted {
                host.emit(ExtensionEvent::SessionCompact);
            } else {
                host.emit(ExtensionEvent::SessionCompactFailed {
                    error: result.summary.clone(),
                });
            }
            model.transcript = transcript_from(&agent.messages);
            Ok(Done::Said(result.summary))
        }
        SlashAction::Export(path) => {
            let Some(store) = agent.session.as_ref() else {
                return Ok(Done::Note("no session to export".into()));
            };
            let output = PathBuf::from(path.unwrap_or_else(|| "session.html".into()));
            Ok(Done::Said(crate::export::export_session(store, &output)?))
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
            agent.provider = provider;
            agent.model_id = model_id;
            crate::loaded_extension_host(parsed).emit(ExtensionEvent::ModelSelect {
                provider: agent.provider.clone(),
                model: agent.model_id.clone(),
            });
            adopt_model(parsed, agent, model);
            Ok(Done::Said(format!(
                "model {} / {}",
                agent.provider, agent.model_id
            )))
        }
        SlashAction::SetThinking(level) => {
            let Some(parsed_level) = pi_protocol::ThinkingLevel::parse(&level) else {
                return Ok(Done::Note(format!("unknown thinking level {level}")));
            };
            agent.thinking_level = parsed_level;
            crate::loaded_extension_host(parsed)
                .emit(ExtensionEvent::ThinkingLevelSelect { level });
            Ok(Done::Said(format!(
                "thinking level {}",
                agent.thinking_level.as_str()
            )))
        }
        SlashAction::OpenThinking => Ok(Done::Ask(Question::Thinking)),
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
            Ok(Done::Open(Overlay::Sessions))
        }
        SlashAction::Reload => {
            crate::apply_discovered_resources(parsed, agent);
            let mut host = crate::loaded_extension_host(parsed);
            host.runtime_flag_values = crate::flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionStart);
            model.corpus = corpus(agent, &model.sessions);
            model.corpus_total = model.corpus.len();
            Ok(Done::Said(
                "reloaded extensions, skills, prompts and context files".into(),
            ))
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
            Ok(Done::Ask(Question::Trust {
                path: agent.cwd.display().to_string(),
                options: crate::trust::get_project_trust_options(&agent.cwd, false),
            }))
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
        SlashAction::Llama => Ok(Done::Said(format!(
            "llama.cpp server {}",
            std::env::var("LLAMA_BASE_URL")
                .unwrap_or_else(|_| crate::llama::DEFAULT_LLAMA_SERVER_URL.into())
        ))),
    }
}

/// The thinking levels the protocol accepts, in the order the old selector
/// listed them.
pub const THINKING_LEVELS: [&str; 4] = ["off", "low", "medium", "high"];

fn session_id(agent: &Agent) -> String {
    agent
        .session
        .as_ref()
        .map(|store| store.header.id.clone())
        .unwrap_or_else(|| "in-memory".into())
}

/// After a model switch: the name in the header, the cap on the context meter,
/// and the row Cogitator marks as the one in hand all move together.
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
    model.model_name = agent.model_id.clone();
    model.config_path = crate::default_agent_dir()
        .join("config.json")
        .display()
        .to_string();
    model.transcript = transcript_from(&agent.messages);
    model.corpus = corpus(agent, &model.sessions);
    model.corpus_total = model.corpus.len();
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
        apply_ui_calls(&mut model, &host.ui_calls);
    }
    crate::start_catalog_refresh_async(parsed);
    for entry in opening_block(parsed, agent, migrated_auth_providers) {
        model.transcript.push(entry);
    }

    let mut terminal = Session::open().map_err(|err| err.to_string())?;
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
    let mut queued: Vec<String> = Vec::new();
    if let Some(text) = prepared.text.clone() {
        let expanded = pi_agent::expand_user_text(&text, &agent.skills, &agent.templates);
        agent.prompt_with(&expanded, &prepared.images);
        if let Err(err) = run_turns(parsed, agent, &mut model, &mut terminal, host.clone()) {
            return Err(err.to_string());
        }
        refresh_context(&mut model, agent);
        crate::davinci_sources::dress_from_workspace(&mut model, &cwd, &session_dir);
    }
    queued.extend(prepared.remaining_messages.iter().cloned());
    for text in queued {
        let expanded = pi_agent::expand_user_text(&text, &agent.skills, &agent.templates);
        agent.prompt(&expanded);
        if let Err(err) = run_turns(parsed, agent, &mut model, &mut terminal, host.clone()) {
            return Err(err.to_string());
        }
        refresh_context(&mut model, agent);
        crate::davinci_sources::dress_from_workspace(&mut model, &cwd, &session_dir);
    }

    let result = loop {
        if let Err(err) = terminal.draw(&model) {
            break Err(err.to_string());
        }

        let timeout = pi_tui::davinci::runtime::TICK.saturating_sub(last_tick.elapsed());
        match crossterm::event::poll(timeout) {
            Ok(true) => match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key))
                    if key.kind != crossterm::event::KeyEventKind::Release =>
                {
                    let was = model.screen;
                    let next = match app::handle_key(&mut model, key) {
                        Flow::Quit => Next::Leave,
                        Flow::Submit(line) => on_line(
                            &mut Shell {
                                parsed,
                                agent,
                                model: &mut model,
                                terminal: &mut terminal,
                                host: &host,
                                pending: &mut pending,
                                cwd: &cwd,
                                session_dir: &session_dir,
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
                                session_dir: &session_dir,
                            },
                            choice,
                        ),
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
                Ok(crossterm::event::Event::Resize(width, height)) => {
                    model.width = width;
                    model.height = height;
                }
                Ok(_) => {}
                Err(err) => break Err(err.to_string()),
            },
            Ok(false) => {}
            Err(err) => break Err(err.to_string()),
        }

        if last_tick.elapsed() >= pi_tui::davinci::runtime::TICK {
            model.tick = model.tick.wrapping_add(1);
            last_tick = Instant::now();
        }
    };

    terminal.close().map_err(|err| err.to_string())?;
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
    let quiet = stored.quiet_startup && !parsed.verbose;
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

    if !quiet && !parsed.models.is_empty() {
        out.push(Entry::Gap);
        out.push(Entry::prose(&format!(
            "models scoped to {}",
            parsed.models.join(", ")
        )));
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

    if !quiet {
        let mut loaded: Vec<String> = Vec::new();
        if !agent.context_files.is_empty() {
            loaded.push(format!("{} context files", agent.context_files.len()));
        }
        if !agent.skills.is_empty() {
            loaded.push(format!("{} skills", agent.skills.len()));
        }
        if !agent.templates.is_empty() {
            loaded.push(format!("{} prompts", agent.templates.len()));
        }
        if !loaded.is_empty() {
            out.push(Entry::Gap);
            out.push(Entry::prose(&format!("loaded {}", loaded.join(" · "))));
        }
    }

    out.extend(custom_messages(agent));
    out
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

/// Apply the UI calls the loaded extensions have made, returning a window
/// title if one was asked for.
///
/// Davinci honours the calls that are rows or text — widgets, header, footer,
/// status, notifications, the composer and the title. It ignores the ones that
/// would take over the design itself: `setTheme` (one palette, negotiated from
/// the terminal, §2), `setEditorComponent`, `setWorkingIndicator` and
/// `setWorkingVisible` (exactly two things animate, §8), and
/// `setToolsExpanded`.
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
            Some("setEditorText") => model.composer = field(call, "text"),
            Some("pasteToEditor") => model.composer.push_str(&field(call, "text")),
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
    session_dir: &'a std::path::Path,
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

    /// Re-read everything the workspace owns: git, the tree, the session list.
    fn redress(&mut self) {
        refresh_context(self.model, self.agent);
        crate::davinci_sources::dress_from_workspace(self.model, self.cwd, self.session_dir);
        crate::davinci_surfaces::dress_from_extensions(self.model, self.cwd);
        self.model.corpus = corpus(self.agent, &self.model.sessions);
        self.model.corpus_total = self.model.corpus.len();
    }

    fn finish(&mut self, done: Done) -> Next {
        match done {
            Done::Said(text) => self.say(&text),
            Done::Note(text) => self.note(&text),
            Done::Open(overlay) => {
                self.model.running = false;
                self.model.overlay = Some(overlay);
            }
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
        let outcome = match &detached {
            Detached::Login { provider, key } => {
                if provider.is_empty() {
                    Err("usage: /login <provider> [key]".to_string())
                } else {
                    crate::login_provider(provider, key.as_deref())
                        .map(|()| format!("signed in to {provider}"))
                }
            }
        };
        match pi_tui::davinci::runtime::Session::open() {
            Ok(session) => *self.terminal = session,
            Err(err) => return Next::Fail(err.to_string()),
        }
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

/// One composer line, carried out.
fn on_line(shell: &mut Shell<'_>, line: &str) -> Next {
    match classify(line) {
        Sent::Quit => Next::Leave,
        Sent::Say(text) => {
            shell.say(&text);
            Next::Go
        }
        Sent::Open(overlay) => {
            shell.model.running = false;
            shell.model.toggle_overlay(overlay);
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
        Sent::Prompt(text) => {
            // `prompt` is what writes the user turn to the session file;
            // pushing onto `messages` directly would lose it on restart.
            let text =
                pi_agent::expand_user_text(&text, &shell.agent.skills, &shell.agent.templates);
            shell.agent.prompt(&text);
            let host = shell.host.clone();
            if let Err(err) =
                run_turns(shell.parsed, shell.agent, shell.model, shell.terminal, host)
            {
                return Next::Fail(err.to_string());
            }
            shell.redress();
            Next::Go
        }
    }
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
            shell.say(&format!("model {} / {}", item.provider, item.id));
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
    }
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
        Question::Thinking => {
            let Some(level) = THINKING_LEVELS.get(index) else {
                return Err("that thinking level is gone".into());
            };
            let action = crate::slash::SlashAction::SetThinking((*level).to_string());
            match perform(shell.parsed, shell.agent, shell.model, action)? {
                Done::Said(text) => Ok(text),
                Done::Note(text) => Err(text),
                _ => Ok(format!("thinking level {level}")),
            }
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
            },
        );

        assert!(matches!(
            &m.transcript[0],
            Entry::Tool {
                state: State::Failed,
                ..
            }
        ));
        assert!(matches!(&m.transcript[1], Entry::Detail(text) if text.contains("E0308")));
        assert!(matches!(&m.transcript[2], Entry::Detail(text) if text.contains("store.rs:118")));
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
    fn commands_with_a_home_open_their_instrument() {
        use pi_tui::davinci::model::Overlay;
        assert!(matches!(classify("/model"), Sent::Open(Overlay::Cogitator)));
        assert!(matches!(classify("/resume"), Sent::Open(Overlay::Sessions)));
        assert!(matches!(
            classify("/settings"),
            Sent::Open(Overlay::Instrumenta)
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
            "/thinking",
            "/thinking high",
            "/logout",
            "/login openai",
            "/tree",
        ] {
            match classify(line) {
                Sent::Command(_) | Sent::Open(_) => {}
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

        let thinking = Question::Thinking.ask(&agent);
        assert_eq!(thinking.title, "MEDITATIO");
        assert_eq!(thinking.name, "THINKING");
        assert_eq!(thinking.items.len(), THINKING_LEVELS.len());
        // The level in hand is named on its own row, not only highlighted:
        // colour is never the only signal (design.md §4).
        let marked: Vec<&PickerItem> = thinking
            .items
            .iter()
            .filter(|item| item.detail == "in hand")
            .collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].label, "medium");

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
        assert!(matches!(classify("/skill:review"), Sent::Prompt(_)));
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
}
