//! Terminal ownership and the event loop.
//!
//! One clock drives everything: a 250ms tick advances the caret blink and the
//! single Studio spinner, and nothing else animates (design.md §8). Panels open
//! and close in one frame.

use std::collections::VecDeque;
use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, BeginSynchronizedUpdate, EndSynchronizedUpdate,
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use unicode_width::UnicodeWidthChar;

use super::app::{self, Flow};
use super::model::Model;
use super::model::TerminalTask;
use super::views::scrollbar::Scrollbar;
use super::views::viewer::ViewerTask;

/// One frame of the clock. Both animations are derived from it.
pub const TICK: Duration = Duration::from_millis(250);

fn display_column_slice(text: &str, from: usize, to: usize) -> String {
    let mut column = 0;
    text.chars()
        .filter(|ch| {
            let width = UnicodeWidthChar::width(*ch).unwrap_or(0);
            let keep = column >= from && column < to;
            column = column.saturating_add(width);
            keep
        })
        .collect()
}

/// Whether the terminal can tell `ctrl+m` from `enter`.
///
/// Without the kitty keyboard protocol they are the same byte (0x0D), which is
/// why the Ratatouille reference had to move vector recall to `ctrl+r`. When
/// the protocol is available we keep the binding the design asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keyboard {
    pub disambiguated: bool,
}

impl Keyboard {
    /// The key that opens Memoria vector recall on this terminal.
    pub fn recall_key(self) -> &'static str {
        if self.disambiguated {
            "ctrl+m"
        } else {
            "ctrl+r"
        }
    }
}

/// crossterm only implements the kitty keyboard protocol query on unix; the
/// Windows console has no equivalent, so `ctrl+m` stays indistinguishable from
/// `enter` there and vector recall falls back to `ctrl+r`.
#[cfg(unix)]
fn supports_keyboard_enhancement() -> bool {
    crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
}

#[cfg(not(unix))]
fn supports_keyboard_enhancement() -> bool {
    false
}

/// Whether the keyboard enhancement flags are currently pushed. Held globally
/// so the terminal can be given back from a panic hook, which has no `Session`
/// to ask.
static DISAMBIGUATED: AtomicBool = AtomicBool::new(false);
/// Whether the alternate screen is currently ours.
static HELD: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static MAIN_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct Generation(u64);

impl Generation {
    fn begin() -> Self {
        Self(GENERATION.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn is_current(self) -> bool {
        GENERATION.load(Ordering::SeqCst) == self.0
    }
}

fn panic_restores_terminal() -> bool {
    MAIN_THREAD
        .get()
        .is_some_and(|main| *main == std::thread::current().id())
}
static MOUSE: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
#[cfg(unix)]
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

fn event_marks_dirty(event: &Event) -> bool {
    !matches!(event, Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved)
}

/// Undo everything [`Session::open`] did, from anywhere, at most once.
///
/// Idempotent and safe to call when the terminal was never taken.
pub fn restore() -> io::Result<()> {
    if !HELD.swap(false, Ordering::SeqCst) {
        return Ok(());
    }
    if DISAMBIGUATED.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    if MOUSE.swap(false, Ordering::SeqCst) {
        #[cfg(unix)]
        {
            let mut out = io::stdout();
            let _ = out.write_all(MOUSE_OFF.as_bytes());
            let _ = out.flush();
        }
        #[cfg(windows)]
        {
            let _ = execute!(io::stdout(), event::DisableMouseCapture);
        }
    }
    let _ = execute!(io::stdout(), DisableFocusChange, DisableBracketedPaste);
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    let _ = disable_raw_mode();
    io::stdout().flush()
}

/// Give the terminal back before a panic prints.
///
/// Without this a panic inside the alternate screen writes its message onto a
/// buffer that is discarded a moment later: the user is left with a terminal
/// in raw mode and no explanation of why. Installing the hook is idempotent,
/// and the previous hook still runs, so the message and any backtrace are
/// printed as usual — onto the real screen.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    MAIN_THREAD.get_or_init(|| std::thread::current().id());
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_restores_terminal() {
                let _ = restore();
            }
            previous(info);
        }));
    });
}

/// Coalesce a bracketed paste that arrives as key events rather than as
/// [`Event::Paste`].
///
/// Unix terminals hand crossterm the `ESC[200~ … ESC[201~` byte stream and
/// crossterm parses it into one `Event::Paste`. The Windows console API has no
/// such notion: pasted characters can arrive as individual key events, with
/// ConPTY stripping the surrounding markers. This filter reassembles markers
/// when preserved, and uses the Windows burst fallback when they are absent,
/// so pasted newlines are not treated as submit keys.
#[derive(Debug, Default)]
struct PasteFilter {
    burst: Option<super::paste_burst::PasteBurst>,
    /// Keys held while a start marker is being matched. If the match fails
    /// they are handed back untouched.
    held: Vec<KeyEvent>,
    /// How much of `[200~` (after the escape) has matched so far.
    start_matched: usize,
    /// `Some` while inside a paste; the block collected so far.
    pasting: Option<String>,
    /// How much of `ESC[201~` has matched inside a paste.
    end_matched: usize,
    /// Events ready to hand back to the caller.
    ready: VecDeque<Event>,
    /// When the last key arrived, so a marker abandoned mid-way (a real
    /// escape the user typed) is flushed rather than held forever.
    last_fed: Option<Instant>,
}

const PASTE_START: &[char] = &['[', '2', '0', '0', '~'];
const PASTE_END: &[char] = &['[', '2', '0', '1', '~'];
/// How long an ambiguous start marker may sit before it is flushed as keys.
/// Once a paste starts, gaps between chunks must never turn text into commands.
const PASTE_MARKER_PATIENCE: Duration = Duration::from_millis(60);

impl PasteFilter {
    fn plain_char(key: &KeyEvent) -> Option<char> {
        match key.code {
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Some(ch)
            }
            _ => None,
        }
    }

    fn feed(&mut self, event: Event) {
        self.last_fed = Some(Instant::now());
        let key = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => key,
            // Anything that is not a key press passes straight through; a
            // resize in the middle of a paste is delivered in order.
            other => {
                self.emit_unbracketed(other);
                return;
            }
        };

        // A lost closing marker must remain fail-closed for Enter. Ctrl+C is
        // an explicit recovery action, never a timeout that can submit text.
        if self.pasting.is_some()
            && key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            let mut block = self.pasting.take().unwrap_or_default();
            if self.end_matched > 0 {
                block.push('\u{1b}');
                block.extend(&PASTE_END[..self.end_matched - 1]);
            }
            self.end_matched = 0;
            self.ready.push_back(Event::Paste(block));
            self.ready.push_back(Event::Key(key));
            return;
        }
        if let Some(text) = self.pasting.as_mut() {
            // Matching the end marker. A stray escape inside the pasted block
            // is folded back into the text when the match fails.
            if self.end_matched > 0 {
                if self.end_matched < 1 + PASTE_END.len()
                    && Self::plain_char(&key) == Some(PASTE_END[self.end_matched - 1])
                {
                    self.end_matched += 1;
                    if self.end_matched == 1 + PASTE_END.len() {
                        let block = self.pasting.take().unwrap_or_default();
                        self.end_matched = 0;
                        self.ready.push_back(Event::Paste(block));
                    }
                    return;
                }
                text.push('\u{1b}');
                for matched in &PASTE_END[..self.end_matched - 1] {
                    text.push(*matched);
                }
                self.end_matched = 0;
                // The key that broke the match is part of the block; fall
                // through to collect it.
            }
            match key.code {
                KeyCode::Esc => self.end_matched = 1,
                KeyCode::Enter => text.push('\n'),
                KeyCode::Tab => text.push('\t'),
                KeyCode::Char(ch) => text.push(ch),
                _ => {}
            }
            return;
        }

        if self.start_matched > 0 {
            if Self::plain_char(&key) == Some(PASTE_START[self.start_matched - 1]) {
                self.held.push(key);
                self.start_matched += 1;
                if self.start_matched == 1 + PASTE_START.len() {
                    self.held.clear();
                    self.start_matched = 0;
                    self.pasting = Some(String::new());
                }
                return;
            }
            self.flush_held();
            // Not a marker after all; the current key is an ordinary key —
            // unless it is itself an escape starting a fresh attempt.
        }

        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            if let Some(burst) = &mut self.burst {
                self.ready.extend(burst.flush());
            }
            self.held.push(key);
            self.start_matched = 1;
            return;
        }
        self.emit_unbracketed(Event::Key(key));
    }

    fn emit_unbracketed(&mut self, event: Event) {
        if let Some(burst) = &mut self.burst {
            self.ready.extend(burst.feed(event, Instant::now()));
        } else {
            self.ready.push_back(event);
        }
    }

    fn flush_held(&mut self) {
        for key in self.held.drain(..) {
            self.ready.push_back(Event::Key(key));
        }
        self.start_matched = 0;
    }

    /// Nothing more is immediately available. A partial start marker older
    /// than the patience window was a real escape: hand it back. An active
    /// paste stays buffered until its closing marker, even across slow chunks.
    fn idle(&mut self) {
        if let Some(burst) = &mut self.burst {
            self.ready.extend(burst.idle(Instant::now()));
        }
        let stale = self
            .last_fed
            .is_none_or(|at| at.elapsed() >= PASTE_MARKER_PATIENCE);
        if !stale {
            return;
        }
        if self.start_matched > 0 {
            self.flush_held();
        }
    }

    fn next_ready(&mut self) -> Option<Event> {
        self.ready.pop_front()
    }

    fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Whether a partial marker is held, so the caller polls again quickly
    /// instead of sleeping a full tick on what may be a real escape.
    fn holding(&self) -> bool {
        self.start_matched > 0 || self.burst.as_ref().is_some_and(|burst| burst.pending())
    }
}

/// The terminal, for as long as the TUI owns it.
pub struct Session {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    generation: Generation,
    keyboard: Keyboard,
    paste: PasteFilter,
    rendered_lines: Vec<String>,
    selector: super::selection::Selector,
    /// While the conversation's scrollbar is held: the thumb row grabbed.
    scrollbar_grab: Option<u16>,
    /// The conversation's scrollbar as last drawn, for hit-testing the
    /// pointer against what is actually on screen.
    scrollbar: Option<Scrollbar>,
    /// What the terminal last said about focus (mode 1004). Terminals that
    /// never report focus leave it `true`.
    focused: bool,
    /// How frames reach the terminal, read once from the environment.
    output: Output,
    /// Whether a path named on screen exists, so a frame does not stat the
    /// same files again: when it was asked, and the answer.
    path_cache: std::collections::HashMap<std::path::PathBuf, (Instant, bool)>,
    /// Draw-time statistics: show them (`DAVINCI_FRAME_STATS=1`) and their
    /// moving average in microseconds.
    frame_stats: bool,
    frame_micros: f64,
    /// Keys the shell raises on the user's behalf (a click on an option is
    /// an enter), handed out ahead of the terminal's own events.
    injected: VecDeque<Event>,
}

/// Frame delivery choices, fixed for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    /// Wrap each frame in DEC mode 2026 so the terminal shows it whole.
    /// Terminals without the mode ignore it; `DAVINCI_SYNC_OUTPUT=0` stops it.
    pub synchronized: bool,
    /// Repaint every cell each frame, for ConPTY hosts that mis-coalesce
    /// positioned writes and leave stale fragments (`DAVINCI_FULL_REPAINT=1`).
    pub full_repaint: bool,
    /// Show the terminal's cursor at the caret instead of painting a cell
    /// (`DAVINCI_NATIVE_CURSOR=1`, or the `showHardwareCursor` setting).
    pub native_cursor: bool,
}

impl Output {
    /// Read from an environment lookup, so tests need not touch the process.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |name: &str| {
            get(name).map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "" | "0" | "false" | "no" | "off"
                )
            })
        };
        Self {
            synchronized: flag("DAVINCI_SYNC_OUTPUT").unwrap_or(true),
            full_repaint: flag("DAVINCI_FULL_REPAINT").unwrap_or(false),
            native_cursor: flag("DAVINCI_NATIVE_CURSOR").unwrap_or(false),
        }
    }
}

/// Open a frame: the terminal holds what follows until [`finish_frame`].
fn begin_frame(out: &mut impl Write, output: Output) -> io::Result<()> {
    if output.synchronized {
        crossterm::queue!(out, BeginSynchronizedUpdate)?;
    }
    Ok(())
}

/// Close a frame: park the cursor on the caret, where an input method's
/// candidate window and a magnifier will look for it. It stays hidden unless
/// the terminal's own cursor is standing in for the painted caret.
fn finish_frame(out: &mut impl Write, output: Output, caret: Option<(u16, u16)>) -> io::Result<()> {
    match caret {
        Some((column, row)) if output.native_cursor => {
            crossterm::queue!(out, MoveTo(column, row), Show)?
        }
        Some((column, row)) => crossterm::queue!(out, Hide, MoveTo(column, row))?,
        None => crossterm::queue!(out, Hide)?,
    }
    if output.synchronized {
        crossterm::queue!(out, EndSynchronizedUpdate)?;
    }
    out.flush()
}

impl Session {
    /// Take the terminal: raw mode, alternate screen, and the kitty keyboard
    /// protocol when the terminal supports it.
    pub fn open() -> io::Result<Self> {
        install_panic_hook();
        let generation = Generation::begin();
        let opened = (|| {
            enable_raw_mode()?;
            HELD.store(true, Ordering::SeqCst);
            let mut out = io::stdout();
            execute!(out, EnterAlternateScreen)?;
            let _ = execute!(out, EnableBracketedPaste, EnableFocusChange);
            let disambiguated = supports_keyboard_enhancement();
            if disambiguated {
                execute!(
                    out,
                    PushKeyboardEnhancementFlags(
                        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    )
                )?;
                DISAMBIGUATED.store(true, Ordering::SeqCst);
            }
            let terminal = Terminal::new(CrosstermBackend::new(out))?;
            Ok::<_, io::Error>((terminal, disambiguated))
        })();
        let (terminal, disambiguated) = match opened {
            Ok(parts) => parts,
            Err(err) => {
                let _ = restore();
                return Err(err);
            }
        };
        Ok(Self {
            terminal,
            generation,
            keyboard: Keyboard { disambiguated },
            paste: PasteFilter {
                burst: cfg!(windows).then(super::paste_burst::PasteBurst::default),
                ..PasteFilter::default()
            },
            rendered_lines: Vec::new(),
            selector: super::selection::Selector::default(),
            scrollbar_grab: None,
            path_cache: std::collections::HashMap::new(),
            frame_stats: env_flag("DAVINCI_FRAME_STATS"),
            frame_micros: 0.0,
            injected: VecDeque::new(),
            scrollbar: None,
            focused: true,
            output: Output::from_env(|name| std::env::var(name).ok()),
        })
    }

    /// Use the terminal's own cursor at the caret (the `showHardwareCursor`
    /// setting); the environment can turn it on but a setting cannot turn
    /// the environment's choice off.
    pub fn set_native_cursor(&mut self, native: bool) {
        self.output.native_cursor |= native;
    }

    /// Take the terminal again after something else gave it back — the panic
    /// hook, which runs [`restore`] from whichever thread panicked. A worker
    /// panic is caught and reported as a failed turn, but by then the
    /// alternate screen is gone; without this the loop kept drawing onto the
    /// primary screen with echoing input.
    pub fn reacquire(&mut self) -> io::Result<()> {
        if HELD.load(Ordering::SeqCst) {
            return Ok(());
        }
        enable_raw_mode()?;
        HELD.store(true, Ordering::SeqCst);
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        let _ = execute!(out, EnableBracketedPaste, EnableFocusChange);
        if self.keyboard.disambiguated {
            let _ = execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
            DISAMBIGUATED.store(true, Ordering::SeqCst);
        }
        self.terminal.clear()
    }

    /// ctrl+z: give the terminal back, stop until the shell continues the
    /// process, then take it again with every mode re-asserted.
    fn suspend(&mut self) -> io::Result<()> {
        restore()?;
        #[cfg(unix)]
        {
            let pid = std::process::id().to_string();
            // SIGSTOP returns control here once the job is continued.
            let _ = std::process::Command::new("kill")
                .args(["-STOP", &pid])
                .status();
        }
        self.reacquire()
    }

    /// Terminal work the transcript viewer asked for: give the screen back,
    /// do it, take the screen again.
    fn run_viewer_task(&mut self, task: ViewerTask, model: &mut Model) -> io::Result<()> {
        restore()?;
        let note = match task {
            ViewerTask::Scrollback(text) => {
                scrollback_dump(&text).map(|()| "transcript written to scrollback".to_owned())
            }
            ViewerTask::Editor(text) => {
                open_in_editor(&text).map(|editor| format!("closed {editor}"))
            }
        };
        self.reacquire()?;
        model.push_toast(note.unwrap_or_else(|err| format!("transcript: {err}")));
        Ok(())
    }

    /// Forget what is on screen so the next draw paints every cell: for when
    /// something wrote to the terminal behind the renderer's back.
    pub fn clear(&mut self) -> io::Result<()> {
        self.terminal.clear()
    }

    /// The next input event, or `None` when `timeout` passes without one.
    ///
    /// All davinci input comes through here so a paste that the platform
    /// delivers as a burst of keys (Windows; see [`PasteFilter`]) is
    /// reassembled into the one [`Event::Paste`] the model expects.
    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        let event = self.poll_event_inner(timeout)?;
        match event {
            Some(Event::FocusGained) => self.focused = true,
            Some(Event::FocusLost) => self.focused = false,
            _ => {}
        }
        Ok(event)
    }

    fn poll_event_inner(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if let Some(event) = self.injected.pop_front() {
            return Ok(Some(event));
        }
        // Dropped mouse-motion events must consume the caller's existing wait,
        // not start a fresh full timeout. Otherwise continuous motion on
        // Windows can prevent the turn loop from reaching its next tick.
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(ready) = self.paste.next_ready() {
                if matches!(ready, Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved)
                {
                    continue;
                }
                return Ok(Some(ready));
            }
            // While a partial marker is held, wait only briefly: the rest of
            // a real marker is already in the queue, and a real escape should
            // not sit swallowed for a full tick.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.paste.idle();
                let ready = self.paste.next_ready();
                return Ok(ready.filter(|event| {
                    !matches!(
                        event,
                        Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved
                    )
                }));
            }
            let wait = if self.paste.holding() {
                remaining.min(super::paste_burst::TYPING_WAIT)
            } else {
                remaining
            };
            if !event::poll(wait)? {
                self.paste.idle();
                let ready = self.paste.next_ready();
                return Ok(ready.filter(|event| {
                    !matches!(
                        event,
                        Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved
                    )
                }));
            }
            self.paste.feed(event::read()?);
            // Feed everything already queued before answering, so a whole
            // paste burst is reassembled in one call.
            while !self.paste.has_ready() && event::poll(Duration::ZERO)? {
                self.paste.feed(event::read()?);
            }
            if let Some(ready) = self.paste.next_ready() {
                if matches!(ready, Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved)
                {
                    continue;
                }
                return Ok(Some(ready));
            }
            // Everything read so far is inside a marker or a paste; poll
            // again rather than reporting an empty tick.
            if !self.paste.holding() && self.paste.pasting.is_none() {
                return Ok(None);
            }
        }
    }

    pub fn keyboard(&self) -> Keyboard {
        self.keyboard
    }

    /// Name the window. The session in hand and the folder it is in belong in
    /// the tab strip, where they are legible with the terminal in the
    /// background.
    pub fn set_title(&mut self, title: &str) -> io::Result<()> {
        let title = super::sanitize::terminal_safe(title);
        execute!(io::stdout(), SetTitle(title.as_ref()))
    }

    pub fn size(&self) -> io::Result<(u16, u16)> {
        let area = self.terminal.size()?;
        Ok((area.width, area.height))
    }

    /// Paint one frame.
    pub fn draw(&mut self, model: &mut Model) -> io::Result<()> {
        if !HELD.load(Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(task) = model.viewer_task.take() {
            self.run_viewer_task(task, model)?;
        }
        match model.terminal_task.take() {
            Some(TerminalTask::Redraw) => self.terminal.clear()?,
            Some(TerminalTask::Suspend) => self.suspend()?,
            None => {}
        }
        let mouse = !env_flag("DAVINCI_DISABLE_MOUSE");
        if mouse != MOUSE.load(Ordering::SeqCst) {
            #[cfg(unix)]
            {
                let mut out = io::stdout();
                out.write_all(if mouse { MOUSE_ON } else { MOUSE_OFF }.as_bytes())?;
                out.flush()?;
            }
            #[cfg(windows)]
            {
                if mouse {
                    execute!(io::stdout(), event::EnableMouseCapture)?;
                } else {
                    execute!(io::stdout(), event::DisableMouseCapture)?;
                }
            }
            MOUSE.store(mouse, Ordering::SeqCst);
        }
        model.terminal_focused = self.focused;
        model.native_cursor = self.output.native_cursor;
        let model: &Model = model;
        let output = self.output;
        if output.full_repaint {
            self.terminal.clear()?;
        }
        begin_frame(self.terminal.backend_mut(), output)?;
        let mut caret = None;
        let background = Style::default().bg(model.theme.background);
        let mut rendered_lines = Vec::new();
        let mut scrollbar = None;
        let selection_bg = model.theme.cc().selection_bg;
        let selector = &mut self.selector;
        let started = Instant::now();
        let mut hits = None;
        let show_stats = self.frame_stats;
        let average = self.frame_micros;
        let hyperlinks = !env_flag("DAVINCI_DISABLE_HYPERLINKS");
        let mut linked: Vec<LinkedCells> = Vec::new();
        let path_cache = &mut self.path_cache;
        self.terminal.draw(|frame| {
            let area: Rect = frame.area();
            let composed = app::compose_frame(model, area.height);
            rendered_lines = composed.lines.iter().map(ToString::to_string).collect();
            scrollbar = composed.scrollbar;
            caret = composed.caret;
            hits = Some(super::model::HitCache {
                targets: composed.targets.clone(),
                suggestion_rows: composed.suggestion_rows.clone(),
            });
            if let Some(bar) = &scrollbar {
                strip_scrollbar(&mut rendered_lines, bar);
            }
            selector.view = scrollbar
                .as_ref()
                .map(super::selection::View::from_scrollbar);
            frame.render_widget(Paragraph::new(composed.lines).style(background), area);
            if show_stats {
                let text = format!(" frame {:.1}ms ", average / 1000.0);
                let x = area.width.saturating_sub(text.len() as u16);
                frame.buffer_mut().set_string(
                    x,
                    0,
                    text,
                    Style::default().add_modifier(Modifier::REVERSED),
                );
            }
            // Selected cells keep their own ink over the selection ground;
            // with no colour at all they are drawn reversed.
            let style = if selection_bg == ratatui::style::Color::Reset {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().bg(selection_bg)
            };
            for (row, left, right) in selector.highlights(&rendered_lines, area.width) {
                if row < area.height {
                    frame
                        .buffer_mut()
                        .set_style(Rect::new(left, row, right - left, 1), style);
                }
            }
            if hyperlinks {
                for (row, text) in rendered_lines
                    .iter()
                    .enumerate()
                    .take(usize::from(area.height))
                {
                    for link in row_links(path_cache, text) {
                        let buffer = frame.buffer_mut();
                        let mut cells = Vec::new();
                        let mut column = link.start;
                        while column < link.end.min(area.width) {
                            let Some(cell) = buffer.cell((column, row as u16)) else {
                                break;
                            };
                            // A wide glyph owns the cell after it.
                            let width = unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1);
                            cells.push((column, row as u16, cell.clone()));
                            column += width as u16;
                        }
                        linked.push((link.url, cells));
                    }
                }
            }
        })?;
        for (url, cells) in &linked {
            write_hyperlink(self.terminal.backend_mut(), url, cells)?;
        }
        finish_frame(self.terminal.backend_mut(), output, caret)?;
        // A moving average of how long a frame takes to compose and write,
        // shown in the corner under DAVINCI_FRAME_STATS=1.
        let micros = started.elapsed().as_secs_f64() * 1e6;
        self.frame_micros = if self.frame_micros == 0.0 {
            micros
        } else {
            self.frame_micros * 0.9 + micros * 0.1
        };
        *model.hit_cache.borrow_mut() = hits;
        self.rendered_lines = rendered_lines;
        self.scrollbar = scrollbar;
        Ok(())
    }

    /// A click on an option of the open question focuses it, and chooses it
    /// as enter would — except a tool approval, which a stray click must
    /// never grant: that only moves the focus.
    fn click_ask_option(&mut self, model: &mut Model, mouse: event::MouseEvent) -> bool {
        use super::model::Overlay;
        use event::{MouseButton::Left, MouseEventKind};
        if mouse.kind != MouseEventKind::Down(Left)
            || !mouse.modifiers.is_empty()
            || model.overlay != Some(Overlay::Ask)
            || model.rewind_modal.is_some()
            || model.decision_modal.is_some()
            || model.approval_instructions.is_some()
            || env_flag("DAVINCI_DISABLE_MOUSE_CLICKS")
        {
            return false;
        }
        let Some(row) = self.rendered_lines.get(usize::from(mouse.row)) else {
            return false;
        };
        let Some(index) = ask_option_at(&model.ask.items, row) else {
            return false;
        };
        model.ask_index = index;
        model.overlay_offset = None;
        model.dirty = true;
        if model.ask.key != "/permissions" {
            self.injected.push_back(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
        }
        true
    }

    /// Open the link under a Ctrl- or Cmd-click. `true` when there was one.
    fn open_link_at(&mut self, model: &mut Model, mouse: event::MouseEvent) -> bool {
        use event::{MouseButton::Left, MouseEventKind};
        if mouse.kind != MouseEventKind::Down(Left)
            || !mouse
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
        {
            return false;
        }
        let Some(text) = self.rendered_lines.get(usize::from(mouse.row)) else {
            return false;
        };
        let links = row_links(&mut self.path_cache, text);
        let Some(link) = super::links::at(&links, mouse.column) else {
            return false;
        };
        let note = match crate::open_browser::try_open_browser(&link.url) {
            Ok(_) => format!("opened {}", link.url),
            Err(err) => format!("could not open {}: {err}", link.url),
        };
        model.push_toast(note);
        true
    }

    /// Copy on release, as the reference terminal does: text selected with
    /// the pointer is on the clipboard the moment the button comes up.
    pub fn set_copy_on_select(&mut self, enabled: bool) {
        self.selector.copy_on_select = enabled;
    }

    /// Graph and scrollbar interaction take the pointer first; everything
    /// else selects text.
    pub fn handle_model_mouse(&mut self, model: &mut Model, mouse: event::MouseEvent) -> bool {
        use event::MouseEventKind;
        if !env_flag("DAVINCI_DISABLE_HYPERLINKS") && self.open_link_at(model, mouse) {
            return false;
        }
        if self.click_ask_option(model, mouse) {
            self.selector.clear();
            return false;
        }
        if route_graph_mouse(model, mouse)
            || route_click_target(model, mouse)
            || route_scroll_mouse(model, &mut self.scrollbar_grab, self.scrollbar, mouse)
        {
            // A selection held in conversation rows survives the wheel; one
            // held in screen rows would now cover different text.
            let wheel = matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            );
            let anchored = self
                .selector
                .selection
                .as_ref()
                .is_some_and(|s| s.space == super::selection::Space::Conversation);
            if !(wheel && anchored) {
                self.selector.clear();
            }
            return false;
        }
        if let Some(text) = self
            .selector
            .mouse(model, &self.rendered_lines, mouse, Instant::now())
        {
            copy_to_clipboard(model, &text);
        }
        false
    }

    /// Give a key to the selection first. Copy keys and shifted motions are
    /// taken (`None`); any other key comes back for the shell, after clearing
    /// the selection unless it is one that keeps it.
    pub fn claim_selection_key(&mut self, model: &mut Model, event: Event) -> Option<Event> {
        let Event::Key(key) = &event else {
            return Some(event);
        };
        match self.selector.key(model, &self.rendered_lines, key) {
            super::selection::KeyOutcome::Pass => Some(event),
            super::selection::KeyOutcome::Taken(text) => {
                if let Some(text) = text {
                    copy_to_clipboard(model, &text);
                }
                model.dirty = true;
                None
            }
        }
    }

    pub fn input_pending(&self) -> bool {
        !self.paste.ready.is_empty()
            || self.paste.holding()
            || self.paste.pasting.is_some()
            || event::poll(Duration::ZERO).unwrap_or(true)
    }

    /// OSC 9;4 terminal progress, for tab strips that draw it (Windows
    /// Terminal, ConEmu). Only written when the stored setting asked for it.
    pub fn set_progress(&mut self, active: bool) -> io::Result<()> {
        let sequence = if active {
            crate::osc::TERMINAL_PROGRESS_ACTIVE_SEQUENCE
        } else {
            crate::osc::TERMINAL_PROGRESS_CLEAR_SEQUENCE
        };
        let mut out = io::stdout();
        out.write_all(sequence.as_bytes())?;
        out.flush()
    }

    /// Give the terminal back. Safe to call twice.
    pub fn close(&mut self) -> io::Result<()> {
        if self.generation.is_current() {
            restore()
        } else {
            Ok(())
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Put `text` on the clipboard and say, briefly, which way it went.
fn copy_to_clipboard(model: &mut Model, text: &str) {
    let note = match crate::open_browser::try_copy_text(text) {
        Ok(path) => {
            let chars = text.chars().count();
            let path = path.strip_prefix("copied via ").unwrap_or("dry run");
            format!(
                "copied {chars} {} · {path}",
                if chars == 1 { "char" } else { "chars" }
            )
        }
        Err(err) => format!("copy failed: {err}"),
    };
    model.push_toast(note);
}

/// The wheel scrolls the conversation, or steps a list like the arrow
/// keys; a press on the scrollbar jumps there and a drag carries the
/// thumb. `false` leaves the event to text selection.
pub fn route_scroll_mouse(
    model: &mut Model,
    grab_state: &mut Option<u16>,
    drawn: Option<Scrollbar>,
    mouse: event::MouseEvent,
) -> bool {
    let wheel_rows = wheel_rows();
    use event::{MouseButton::Left, MouseEventKind};
    match mouse.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let up = mouse.kind == MouseEventKind::ScrollUp;
            // The completion list floats over the screen, so the pointer
            // decides which surface the wheel belongs to: over the list it
            // steps the selection, anywhere else it scrolls the conversation.
            if let Some(viewer) = &mut model.viewer {
                let rows = if up { -wheel_rows } else { wheel_rows };
                let height = usize::from(model.height);
                let max = viewer
                    .lines
                    .len()
                    .saturating_sub(height.saturating_sub(2).max(1));
                viewer.top = viewer.top.saturating_add_signed(rows).min(max);
                return true;
            }
            if over_suggestions(model, mouse.row) {
                model.suggestion_move(if up { -1 } else { 1 });
                return true;
            }
            if app::scroll_transcript(model, if up { wheel_rows } else { -wheel_rows }) {
                return true;
            }
            if model.screen != super::model::Screen::Agent || model.overlay.is_some() {
                let code = if up { KeyCode::Up } else { KeyCode::Down };
                let _ = app::handle_key(model, KeyEvent::new(code, KeyModifiers::NONE));
            }
            true
        }
        MouseEventKind::Down(Left) => {
            // A release outside the window is often never reported; a new
            // press always starts clean.
            *grab_state = None;
            let Some(bar) = drawn.filter(|bar| bar.contains(mouse.column, mouse.row)) else {
                return false;
            };
            let (thumb, len) = bar.thumb();
            let at = mouse.row - bar.top;
            let grab = bar.grab(mouse.row);
            if at < thumb || at >= thumb + len {
                app::set_transcript_offset(model, &bar, bar.offset_at(mouse.row, grab));
            }
            *grab_state = Some(grab);
            true
        }
        MouseEventKind::Drag(Left) => {
            let Some(grab) = *grab_state else {
                return false;
            };
            if let Some(bar) = drawn {
                app::set_transcript_offset(model, &bar, bar.offset_at(mouse.row, grab));
            }
            true
        }
        MouseEventKind::Up(Left) => grab_state.take().is_some(),
        _ => false,
    }
}

/// Whether `row` falls on the open completion list. Nothing is composed
/// unless a list is open.
/// Print `text` onto the terminal's own screen, where its scrollback keeps
/// it, and wait for esc, q or enter before the viewer comes back.
fn scrollback_dump(text: &str) -> Result<(), String> {
    let mut out = io::stdout();
    out.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    out.write_all(b"\n-- esc or q to return to davinci --\n")
        .map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    enable_raw_mode().map_err(|e| e.to_string())?;
    let waited = loop {
        match event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => break Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break Ok(()),
                _ => {}
            },
            Ok(_) => {}
            Err(err) => break Err(err.to_string()),
        }
    };
    let _ = disable_raw_mode();
    waited
}

/// Open `text` in `$VISUAL`, then `$EDITOR`, then `vi`, from a private
/// temporary file that is removed afterwards. Returns the editor's name.
fn open_in_editor(text: &str) -> Result<String, String> {
    let command = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_owned());
    let mut parts = command.split_whitespace();
    let program = parts.next().ok_or("no editor")?.to_owned();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "davinci-transcript-{}-{nanos}.txt",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| e.to_string())?;
    let status = std::process::Command::new(&program)
        .args(parts)
        .arg(&path)
        .status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(_) => Ok(program),
        Err(err) => Err(format!("could not start {program}: {err}")),
    }
}

/// The links in one rendered row, asking the disk about each path at most
/// once every couple of seconds.
fn row_links(
    cache: &mut std::collections::HashMap<std::path::PathBuf, (Instant, bool)>,
    text: &str,
) -> Vec<super::links::Link> {
    if !text.contains('/') {
        return Vec::new();
    }
    if cache.len() > 4096 {
        cache.clear();
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    super::links::find(text, &cwd, &mut |path| {
        let fresh = cache
            .get(path)
            .filter(|(asked, _)| asked.elapsed() < Duration::from_secs(2));
        if let Some((_, found)) = fresh {
            return *found;
        }
        let found = path.exists();
        cache.insert(path.to_path_buf(), (Instant::now(), found));
        found
    })
}

/// A link's address and the screen cells it covers.
type LinkedCells = (String, Vec<(u16, u16, ratatui::buffer::Cell)>);

/// Print `cells` again inside an OSC 8 hyperlink to `url`. The terminal
/// already shows exactly these cells, so this changes only what a click on
/// them means.
fn write_hyperlink<W: Write>(
    backend: &mut CrosstermBackend<W>,
    url: &str,
    cells: &[(u16, u16, ratatui::buffer::Cell)],
) -> io::Result<()> {
    use ratatui::backend::Backend;
    if cells.is_empty() {
        return Ok(());
    }
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    write!(backend, "\x1b]8;;{url}\x1b\\")?;
    backend.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))?;
    write!(backend, "\x1b]8;;\x1b\\")
}

/// The option of an open question that a drawn row shows: the longest label
/// found on it, so "Yes" does not win over "Yes, and don't ask again".
fn ask_option_at(items: &[super::model::PickerItem], row: &str) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.label.trim().is_empty() && row.contains(item.label.trim()))
        .max_by_key(|(_, item)| item.label.trim().len())
        .map(|(index, _)| index)
}

/// A truthy environment flag (`1`, `true`, `yes`, `on`).
fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Rows one wheel notch scrolls: `DAVINCI_SCROLL_SPEED` (0.25–20) times the
/// default three.
fn wheel_rows() -> isize {
    use std::sync::OnceLock;
    static ROWS: OnceLock<isize> = OnceLock::new();
    *ROWS.get_or_init(|| {
        let speed = std::env::var("DAVINCI_SCROLL_SPEED")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|speed| speed.is_finite())
            .unwrap_or(1.0)
            .clamp(0.25, 20.0);
        ((super::views::scrollbar::WHEEL_ROWS as f64 * speed).round() as isize).max(1)
    })
}

/// A left click on a drawn target: jump back to the newest output, or take
/// the completion under the pointer. `DAVINCI_DISABLE_MOUSE_CLICKS` leaves
/// only the wheel.
pub(crate) fn route_click_target(model: &mut Model, mouse: event::MouseEvent) -> bool {
    use event::{MouseButton::Left, MouseEventKind};
    if mouse.kind != MouseEventKind::Down(Left)
        || model.overlay.is_some()
        || env_flag("DAVINCI_DISABLE_MOUSE_CLICKS")
    {
        return false;
    }
    let cached = model.hit_cache.borrow().clone();
    let targets = match cached {
        Some(cache) => cache.targets,
        None => app::compose_frame(model, model.height).targets,
    };
    let Some(hit) = targets.iter().find(|hit| hit.rows.contains(&mouse.row)) else {
        return false;
    };
    match hit.target {
        app::Target::JumpToBottom => {
            model.transcript_scroll.take();
        }
        app::Target::ComposerRow(row) => {
            // The text starts after the two-cell prompt.
            model.place_caret(row, usize::from(mouse.column).saturating_sub(2));
        }
        app::Target::Prompt(entry) => {
            app::scroll_transcript_to_entry(model, entry);
        }
        app::Target::HistoryMatch(index) => {
            model.history_search_accept(index);
        }
        app::Target::Suggestion(index) => {
            model.suggestion_index = index;
            model.suggestion_picked = true;
            model.accept_suggestion();
        }
    }
    true
}

fn over_suggestions(model: &Model, row: u16) -> bool {
    if model.overlay.is_some() || model.suggestions.is_none() {
        return false;
    }
    let cached = model.hit_cache.borrow().clone();
    let rows = match cached {
        Some(cache) => cache.suggestion_rows,
        None => app::compose_frame(model, model.height).suggestion_rows,
    };
    rows.is_some_and(|rows| rows.contains(&row))
}

/// Copied text is the conversation's, not the scrollbar's: drop the bar's
/// column from the rows it was drawn in.
pub fn strip_scrollbar(rows: &mut [String], bar: &Scrollbar) {
    if !bar.drawn {
        return;
    }
    let first = usize::from(bar.top);
    let last = first + usize::from(bar.height);
    for row in rows.iter_mut().take(last).skip(first) {
        *row = display_column_slice(row, 0, usize::from(bar.column)).to_string();
    }
}

fn route_graph_mouse(model: &mut Model, mouse: event::MouseEvent) -> bool {
    if model.screen != super::model::Screen::GraphRun || model.overlay.is_some() {
        return false;
    }
    app::compose_frame(model, model.height)
        .graph
        .is_some_and(|frame| super::views::graph_nav::handle_mouse(model, mouse, &frame))
}

/// Run the loop until the user leaves. `on_submit` is handed each sent turn.
pub fn run(model: &mut Model, mut on_submit: impl FnMut(&mut Model, String)) -> io::Result<()> {
    let mut session = Session::open()?;
    let (width, height) = session.size()?;
    model.width = width;
    model.height = height;

    let mut last_tick = Instant::now();
    loop {
        if model.dirty {
            session.draw(model)?;
            model.dirty = false;
        }

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if let Some(event) = session
            .poll_event(timeout)?
            .and_then(|event| session.claim_selection_key(model, event))
        {
            model.dirty |= event_marks_dirty(&event);
            match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    match app::handle_key(model, key) {
                        Flow::Quit => break,
                        Flow::Submit(text) => on_submit(model, text),
                        // The fixture runner has no agent to confirm a mode
                        // change or choice. Never advance UI permission state
                        // without the live host's authoritative policy update.
                        Flow::CyclePermissionMode
                        | Flow::Choose(_)
                        | Flow::Continue
                        | Flow::Interrupt => {}
                    }
                }
                Event::Resize(width, height) => {
                    model.width = width.max(20);
                    model.height = height.max(4);
                }
                Event::Paste(text) => model.paste(&text),
                Event::Mouse(mouse) => {
                    session.handle_model_mouse(model, mouse);
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK {
            model.advance_tick();
            model.dirty = true;
            last_tick = Instant::now();
        }
    }

    session.close()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_option_is_found_by_the_longest_label_on_its_row() {
        use super::super::model::PickerItem;
        let items = [
            PickerItem::new("Yes", ""),
            PickerItem::new("Yes, and don't ask again", ""),
            PickerItem::new("No", ""),
        ];
        assert_eq!(
            ask_option_at(&items, "│ ❯ Yes, and don't ask again │"),
            Some(1)
        );
        assert_eq!(
            ask_option_at(&items, "│   Yes                       │"),
            Some(0)
        );
        assert_eq!(
            ask_option_at(&items, "│   something else            │"),
            None
        );
    }

    #[test]
    fn a_link_is_reprinted_between_hyperlink_open_and_close() {
        #[derive(Clone, Default)]
        struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let shared = Shared::default();
        let mut backend = CrosstermBackend::new(shared.clone());
        let mut cell = ratatui::buffer::Cell::default();
        cell.set_symbol("h");
        write_hyperlink(&mut backend, "https://a.io/\x1b[31m", &[(3, 1, cell)]).unwrap();
        backend.flush().unwrap();
        let out = String::from_utf8(shared.0.borrow().clone()).unwrap();
        assert!(
            out.starts_with("\x1b]8;;https://a.io/[31m\x1b\\"),
            "{out:?}"
        );
        assert!(out.contains('h'));
        assert!(out.ends_with("\x1b]8;;\x1b\\"), "{out:?}");
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn output_defaults_to_synchronized_incremental_painted_caret() {
        let output = Output::from_env(env(&[]));
        assert_eq!(
            output,
            Output {
                synchronized: true,
                full_repaint: false,
                native_cursor: false,
            }
        );
        let output = Output::from_env(env(&[
            ("DAVINCI_SYNC_OUTPUT", "0"),
            ("DAVINCI_FULL_REPAINT", "1"),
            ("DAVINCI_NATIVE_CURSOR", "true"),
        ]));
        assert!(!output.synchronized && output.full_repaint && output.native_cursor);
    }

    #[test]
    fn a_synchronized_frame_is_bracketed_and_parks_the_hidden_cursor() {
        let output = Output::from_env(env(&[]));
        let mut out = Vec::new();
        begin_frame(&mut out, output).unwrap();
        out.extend_from_slice(b"cells");
        finish_frame(&mut out, output, Some((4, 9))).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "\x1b[?2026hcells\x1b[?25l\x1b[10;5H\x1b[?2026l");
    }

    #[test]
    fn an_unsynchronized_native_cursor_frame_shows_the_cursor_at_the_caret() {
        let output = Output::from_env(env(&[
            ("DAVINCI_SYNC_OUTPUT", "off"),
            ("DAVINCI_NATIVE_CURSOR", "1"),
        ]));
        let mut out = Vec::new();
        begin_frame(&mut out, output).unwrap();
        finish_frame(&mut out, output, Some((0, 0))).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[1;1H\x1b[?25h");
        let mut out = Vec::new();
        finish_frame(&mut out, output, None).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[?25l");
    }

    #[test]
    fn mouse_motion_does_not_mark_the_frame_dirty() {
        let moved = Event::Mouse(event::MouseEvent {
            kind: event::MouseEventKind::Moved,
            column: 3,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!event_marks_dirty(&moved));
        assert!(event_marks_dirty(&Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        ))));
    }

    #[test]
    fn graph_mouse_preserves_other_surfaces() {
        let mut model = Model::new(
            super::super::theme::Theme::da_vinci(super::super::theme::ColorDepth::TrueColor, true),
            120,
            40,
            false,
        );
        model.width = 120;
        model.height = 40;
        model.screen = super::super::model::Screen::GraphRun;
        model.graph_run = Some(super::super::fixtures::blueprint_graph());
        let frame = app::compose_frame(&model, model.height).graph.unwrap();
        let node = frame
            .layout
            .nodes
            .iter()
            .find(|n| n.id == "writer")
            .unwrap();
        let column = (node.rect.x as i32 - frame.offset.0 + 1) as u16;
        let row = (node.rect.y as i32 - frame.offset.1 + frame.origin_y as i32 + 1) as u16;
        let click = event::MouseEvent {
            kind: event::MouseEventKind::Down(event::MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert!(model.graph_canvas.follow_live);
        assert!(route_graph_mouse(&mut model, click));
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("writer")
        );
        model.graph_canvas.follow_live = true;
        let wheel = event::MouseEvent {
            kind: event::MouseEventKind::ScrollDown,
            ..click
        };
        assert!(route_graph_mouse(&mut model, wheel));
        assert!(!model.graph_canvas.follow_live);
        model.screen = super::super::model::Screen::Agent;
        assert!(!route_graph_mouse(&mut model, click));
    }

    #[test]
    fn the_clock_is_two_hundred_and_fifty_milliseconds() {
        assert_eq!(TICK, Duration::from_millis(250));
    }

    #[test]
    fn display_column_slice_respects_wide_characters() {
        assert_eq!(display_column_slice("ab界cd", 2, 4), "界");
        assert_eq!(display_column_slice("ab界cd", 4, usize::MAX), "cd");
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn feed_str(filter: &mut PasteFilter, text: &str) {
        for ch in text.chars() {
            match ch {
                '\u{1b}' => filter.feed(key(KeyCode::Esc)),
                '\n' => filter.feed(key(KeyCode::Enter)),
                '\t' => filter.feed(key(KeyCode::Tab)),
                other => filter.feed(key(KeyCode::Char(other))),
            }
        }
    }

    fn drain(filter: &mut PasteFilter) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(event) = filter.next_ready() {
            out.push(event);
        }
        out
    }

    #[test]
    fn a_marker_wrapped_burst_of_keys_reassembles_into_one_paste() {
        // A key stream from a console path that preserves paste markers.
        let mut filter = PasteFilter::default();
        feed_str(
            &mut filter,
            "\u{1b}[200~first line\nsecond\tline\u{1b}[201~",
        );
        let events = drain(&mut filter);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0],
            Event::Paste("first line\nsecond\tline".to_string())
        );
    }

    #[test]
    fn keys_outside_a_marker_pass_through_untouched() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "hi");
        assert_eq!(
            drain(&mut filter),
            vec![key(KeyCode::Char('h')), key(KeyCode::Char('i'))]
        );
    }

    #[test]
    fn a_real_escape_is_handed_back_once_the_marker_fails_to_appear() {
        let mut filter = PasteFilter::default();
        filter.feed(key(KeyCode::Esc));
        assert!(
            filter.holding(),
            "the escape is held while it could be a marker"
        );
        assert!(drain(&mut filter).is_empty());
        // The next key is not `[`: the escape was real.
        filter.feed(key(KeyCode::Char('x')));
        assert_eq!(
            drain(&mut filter),
            vec![key(KeyCode::Esc), key(KeyCode::Char('x'))]
        );
    }

    #[test]
    fn a_stale_partial_marker_is_flushed_on_idle() {
        let mut filter = PasteFilter::default();
        filter.feed(key(KeyCode::Esc));
        filter.feed(key(KeyCode::Char('[')));
        filter.last_fed = Some(Instant::now() - Duration::from_millis(200));
        filter.idle();
        assert_eq!(
            drain(&mut filter),
            vec![key(KeyCode::Esc), key(KeyCode::Char('['))]
        );
    }

    #[test]
    fn an_escape_inside_the_pasted_block_stays_in_the_block() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "\u{1b}[200~a\u{1b}[2J b\u{1b}[201~");
        let events = drain(&mut filter);
        assert_eq!(events, vec![Event::Paste("a\u{1b}[2J b".to_string())]);
    }

    #[test]
    fn a_slow_paste_waits_for_its_end_marker_and_preserves_newlines() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "\u{1b}[200~first");
        filter.last_fed = Some(Instant::now() - Duration::from_millis(200));
        filter.idle();
        assert!(drain(&mut filter).is_empty());
        feed_str(&mut filter, "\nsecond\n\u{1b}[20");
        filter.last_fed = Some(Instant::now() - Duration::from_secs(2));
        filter.idle();
        assert!(drain(&mut filter).is_empty());
        feed_str(&mut filter, "1~");
        assert_eq!(
            drain(&mut filter),
            vec![Event::Paste("first\nsecond\n".into())]
        );
        filter.feed(key(KeyCode::Enter));
        assert_eq!(drain(&mut filter), vec![key(KeyCode::Enter)]);
    }

    #[test]
    fn a_resize_in_the_middle_of_a_paste_is_delivered_in_order() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "\u{1b}[200~before");
        filter.feed(Event::Resize(80, 24));
        feed_str(&mut filter, "after\u{1b}[201~");
        assert_eq!(
            drain(&mut filter),
            vec![
                Event::Resize(80, 24),
                Event::Paste("beforeafter".to_string())
            ]
        );
    }

    #[test]
    fn interrupted_paste_can_be_recovered_with_control_c_without_submitting() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "\u{1b}[200~draft\n");
        let cancel = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        filter.feed(Event::Key(cancel));
        assert_eq!(
            drain(&mut filter),
            vec![Event::Paste("draft\n".into()), Event::Key(cancel)]
        );
        assert!(filter.pasting.is_none());
    }

    #[test]
    fn recall_falls_back_when_the_terminal_cannot_split_ctrl_m_from_enter() {
        assert_eq!(
            Keyboard {
                disambiguated: true
            }
            .recall_key(),
            "ctrl+m"
        );
        assert_eq!(
            Keyboard {
                disambiguated: false
            }
            .recall_key(),
            "ctrl+r"
        );
    }
}
