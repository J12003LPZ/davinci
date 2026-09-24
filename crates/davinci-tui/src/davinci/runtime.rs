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

use crossterm::cursor::Show;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use unicode_width::UnicodeWidthChar;

use super::app::{self, Flow};
use super::model::Model;

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
        let _ = execute!(io::stdout(), event::DisableMouseCapture);
    }
    let _ = execute!(io::stdout(), DisableBracketedPaste);
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
    mic_rect: Option<Rect>,
    rendered_lines: Vec<String>,
    selection_anchor: Option<(u16, u16)>,
    selection_focus: Option<(u16, u16)>,
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
            let _ = execute!(out, EnableBracketedPaste);
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
            mic_rect: None,
            rendered_lines: Vec::new(),
            selection_anchor: None,
            selection_focus: None,
        })
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
        let _ = execute!(out, EnableBracketedPaste);
        if self.keyboard.disambiguated {
            let _ = execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
            DISAMBIGUATED.store(true, Ordering::SeqCst);
        }
        self.terminal.clear()
    }

    /// The next input event, or `None` when `timeout` passes without one.
    ///
    /// All davinci input comes through here so a paste that the platform
    /// delivers as a burst of keys (Windows; see [`PasteFilter`]) is
    /// reassembled into the one [`Event::Paste`] the model expects.
    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        loop {
            if let Some(ready) = self.paste.next_ready() {
                if matches!(ready, Event::Resize(..)) {
                    self.mic_rect = None;
                }
                return Ok(Some(ready));
            }
            // While a partial marker is held, wait only briefly: the rest of
            // a real marker is already in the queue, and a real escape should
            // not sit swallowed for a full tick.
            let wait = if self.paste.holding() {
                timeout.min(super::paste_burst::TYPING_WAIT)
            } else {
                timeout
            };
            if !event::poll(wait)? {
                self.paste.idle();
                return Ok(self.paste.next_ready());
            }
            self.paste.feed(event::read()?);
            // Feed everything already queued before answering, so a whole
            // paste burst is reassembled in one call.
            while !self.paste.has_ready() && event::poll(Duration::ZERO)? {
                self.paste.feed(event::read()?);
            }
            if let Some(ready) = self.paste.next_ready() {
                if matches!(ready, Event::Resize(..)) {
                    self.mic_rect = None;
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
    pub fn draw(&mut self, model: &Model) -> io::Result<()> {
        if !HELD.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.mic_rect = None;
        let mouse = true;
        if mouse != MOUSE.load(Ordering::SeqCst) {
            if mouse {
                execute!(io::stdout(), event::EnableMouseCapture)?;
            } else {
                execute!(io::stdout(), event::DisableMouseCapture)?;
            }
            MOUSE.store(mouse, Ordering::SeqCst);
        }
        let background = Style::default().bg(model.theme.background);
        let mut mic_rect = None;
        let mut rendered_lines = Vec::new();
        let selection = self.selection_range();
        self.terminal.draw(|frame| {
            let area: Rect = frame.area();
            let composed = app::compose_frame(model, area.height);
            mic_rect = composed
                .mic_rect
                .filter(|r| r.right() <= area.width && r.bottom() <= area.height);
            rendered_lines = composed.lines.iter().map(ToString::to_string).collect();
            frame.render_widget(Paragraph::new(composed.lines).style(background), area);
            if let Some((start, end)) = selection {
                for row in start.1..=end.1.min(area.height.saturating_sub(1)) {
                    let left = if row == start.1 { start.0 } else { 0 };
                    let right = if row == end.1 {
                        end.0.saturating_add(1).min(area.width)
                    } else {
                        area.width
                    };
                    if right > left {
                        frame.buffer_mut().set_style(
                            Rect::new(left, row, right.saturating_sub(left), 1),
                            Style::default().add_modifier(Modifier::REVERSED),
                        );
                    }
                }
            }
        })?;
        self.mic_rect = mic_rect;
        self.rendered_lines = rendered_lines;
        Ok(())
    }

    fn selection_range(&self) -> Option<((u16, u16), (u16, u16))> {
        let anchor = self.selection_anchor?;
        let focus = self.selection_focus?;
        Some(if (anchor.1, anchor.0) <= (focus.1, focus.0) {
            (anchor, focus)
        } else {
            (focus, anchor)
        })
    }

    /// Own left-drag selection while mouse reporting is enabled. Releasing the
    /// button copies immediately, matching the upstream fullscreen terminal.
    /// Returns true only when the microphone button was activated.
    pub fn handle_mouse(&mut self, mouse: event::MouseEvent) -> bool {
        use event::{MouseButton::Left, MouseEventKind};
        let point = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(Left)
                if self.mic_rect.is_some_and(|r| r.contains(point.into())) =>
            {
                self.selection_anchor = None;
                self.selection_focus = None;
                true
            }
            MouseEventKind::Down(Left) => {
                self.selection_anchor = Some(point);
                self.selection_focus = Some(point);
                false
            }
            MouseEventKind::Drag(Left) if self.selection_anchor.is_some() => {
                self.selection_focus = Some(point);
                false
            }
            MouseEventKind::Up(Left) if self.selection_anchor.is_some() => {
                self.selection_focus = Some(point);
                if let Some(text) = self.selected_text() {
                    crate::open_browser::copy_text(&text);
                }
                false
            }
            _ => false,
        }
    }

    /// Graph interaction shares the composed geometry. Microphone activation
    /// keeps first refusal; other surfaces retain native text selection.
    pub fn handle_model_mouse(&mut self, model: &mut Model, mouse: event::MouseEvent) -> bool {
        if self.mic_clicked(mouse) {
            return self.handle_mouse(mouse);
        }
        if route_graph_mouse(model, mouse, self.mic_rect) {
            self.selection_anchor = None;
            self.selection_focus = None;
            return false;
        }
        self.handle_mouse(mouse)
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        if start == end {
            return None;
        }
        let mut rows = Vec::new();
        for row in start.1..=end.1 {
            let line = self
                .rendered_lines
                .get(row as usize)
                .map(String::as_str)
                .unwrap_or("");
            let from = if row == start.1 { start.0 as usize } else { 0 };
            let to = if row == end.1 {
                end.0.saturating_add(1) as usize
            } else {
                usize::MAX
            };
            rows.push(display_column_slice(line, from, to).trim_end().to_string());
        }
        let text = rows.join("\n");
        (!text.is_empty()).then_some(text)
    }

    pub fn mic_clicked(&self, mouse: event::MouseEvent) -> bool {
        mouse.kind == event::MouseEventKind::Down(event::MouseButton::Left)
            && self
                .mic_rect
                .is_some_and(|r| r.contains((mouse.column, mouse.row).into()))
    }

    pub fn mic_visible(&self) -> bool {
        self.mic_rect.is_some()
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

fn route_graph_mouse(model: &mut Model, mouse: event::MouseEvent, mic: Option<Rect>) -> bool {
    if model.screen != super::model::Screen::GraphRun
        || model.overlay.is_some()
        || model.voice.setup
        || (mouse.kind == event::MouseEventKind::Down(event::MouseButton::Left)
            && mic.is_some_and(|r| r.contains((mouse.column, mouse.row).into())))
    {
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
        session.draw(model)?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if let Some(event) = session.poll_event(timeout)? {
            match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    match app::handle_key(model, key) {
                        Flow::Quit => break,
                        Flow::Submit(text) => on_submit(model, text),
                        // The fixture runner has no agent to confirm a mode
                        // change or choice. Never advance UI permission state
                        // without the live host's authoritative policy update.
                        Flow::CyclePermissionMode
                        | Flow::SecretInputSubmitted(_)
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
            model.tick = model.tick.wrapping_add(1);
            last_tick = Instant::now();
        }
    }

    session.close()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_mouse_preserves_microphone_and_other_surfaces() {
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
        assert!(!route_graph_mouse(
            &mut model,
            click,
            Some(Rect::new(column, row, 1, 1))
        ));
        assert!(model.graph_canvas.follow_live);
        assert!(route_graph_mouse(&mut model, click, None));
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
        assert!(route_graph_mouse(&mut model, wheel, None));
        assert!(!model.graph_canvas.follow_live);
        model.screen = super::super::model::Screen::Agent;
        assert!(!route_graph_mouse(&mut model, click, None));
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
