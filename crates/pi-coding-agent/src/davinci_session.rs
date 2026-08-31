//! The davinci TUI, driven by the real agent.
//!
//! The shell in `pi_tui::davinci` knows nothing about agents; this module owns
//! the loop that turns a sent composer line into an agent turn and the agent's
//! events back into transcript blocks (`docs/ui/design.md` §6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pi_agent::{Agent, AgentEvent, EventSink};
use pi_tui::davinci::model::{CorpusItem, Entry, Model, ModelItem, Step};
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
        let worker =
            scope.spawn(|| crate::complete_prompt_with_host(parsed, agent, Some(host), false));

        loop {
            while let Ok(event) = event_rx.try_recv() {
                apply(model, &mut turn, &event);
            }
            session.draw(model)?;

            if worker.is_finished() {
                break;
            }
            if crossterm::event::poll(Duration::from_millis(40))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
                    if key.kind != KeyEventKind::Release {
                        let stop = key.code == KeyCode::Esc
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL));
                        if stop {
                            abort.store(true, Ordering::Relaxed);
                            model.interrupt();
                        }
                    }
                } else if let crossterm::event::Event::Resize(width, height) =
                    crossterm::event::read()?
                {
                    model.width = width;
                    model.height = height;
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
    Ok(())
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
    /// A slash command this shell does not own yet, named so the user is told
    /// rather than ignored.
    Unsupported(String),
}

pub fn classify(line: &str) -> Sent {
    use crate::slash::SlashAction;
    use pi_tui::davinci::model::Overlay;

    match crate::slash::parse_line(line) {
        SlashAction::Prompt(text) => Sent::Prompt(text),
        SlashAction::Quit => Sent::Quit,
        SlashAction::Status(text) => Sent::Say(text),
        // The instruments the design already gives these commands a home in.
        SlashAction::OpenModel | SlashAction::SetModel(_) => Sent::Open(Overlay::Cogitator),
        SlashAction::Resume | SlashAction::Tree => Sent::Open(Overlay::Sessions),
        SlashAction::Settings | SlashAction::Hotkeys => Sent::Open(Overlay::Instrumenta),
        _ => Sent::Unsupported(command_name(line)),
    }
}

/// The command as the user typed it, for the line that says it is not wired in
/// yet. Naming it from the input rather than from the parsed variant means the
/// message always matches what was on screen.
fn command_name(line: &str) -> String {
    line.split_whitespace()
        .next()
        .unwrap_or("/command")
        .to_string()
}

/// Run the davinci TUI against a live agent until the user leaves.
pub fn run(
    parsed: &crate::args::Args,
    agent: &mut Agent,
    raw: &[String],
    host: Arc<Mutex<ExtensionHost>>,
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
        })
        .collect();
    model.model_index = model
        .models
        .iter()
        .position(|item| item.name.ends_with(&agent.model_id))
        .unwrap_or(0);
    refresh_context(&mut model, agent);

    let mut terminal = Session::open().map_err(|err| err.to_string())?;
    let (width, height) = terminal.size().map_err(|err| err.to_string())?;
    model.width = width;
    model.height = height;

    let mut last_tick = Instant::now();
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
                    match app::handle_key(&mut model, key) {
                        Flow::Quit => break Ok(0),
                        Flow::Submit(line) => match classify(&line) {
                            Sent::Quit => break Ok(0),
                            Sent::Say(text) => {
                                model.running = false;
                                model.transcript.push(Entry::Gap);
                                model.transcript.push(Entry::prose(text.trim()));
                            }
                            Sent::Open(overlay) => {
                                model.running = false;
                                model.toggle_overlay(overlay);
                            }
                            Sent::Unsupported(what) => {
                                model.running = false;
                                model.transcript.push(Entry::Gap);
                                model.transcript.push(Entry::tool(
                                    State::Attention,
                                    "instrumenta",
                                    &format!("{what} is not wired into this shell yet"),
                                    None,
                                ));
                            }
                            Sent::Prompt(text) => {
                                // `prompt` is what writes the user turn to the
                                // session file; pushing onto `messages`
                                // directly would lose it on restart.
                                let text = pi_agent::expand_user_text(
                                    &text,
                                    &agent.skills,
                                    &agent.templates,
                                );
                                agent.prompt(&text);
                                if let Err(err) =
                                    run_turn(parsed, agent, &mut model, &mut terminal, host.clone())
                                {
                                    break Err(err.to_string());
                                }
                                refresh_context(&mut model, agent);
                                crate::davinci_sources::dress_from_workspace(
                                    &mut model,
                                    &cwd,
                                    &session_dir,
                                );
                            }
                        },
                        Flow::Continue | Flow::Interrupt => {}
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
        assert!(matches!(classify("/tree"), Sent::Open(Overlay::Sessions)));
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
    fn a_command_with_no_home_yet_is_named_rather_than_swallowed() {
        match classify("/compact") {
            Sent::Unsupported(name) => assert_eq!(name, "/compact"),
            other => panic!("{}", matches!(other, Sent::Prompt(_))),
        }
        match classify("/new") {
            Sent::Unsupported(name) => assert_eq!(name, "/new"),
            _ => panic!("expected an unsupported command"),
        }
        match classify("/export report.html") {
            Sent::Unsupported(name) => assert_eq!(name, "/export"),
            _ => panic!("expected an unsupported command"),
        }
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
