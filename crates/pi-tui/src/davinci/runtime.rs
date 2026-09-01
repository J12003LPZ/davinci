//! Terminal ownership and the event loop.
//!
//! One clock drives everything: a 250ms tick advances the caret blink and the
//! single Studio spinner, and nothing else animates (design.md §8). Panels open
//! and close in one frame.

use std::collections::VecDeque;
use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
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
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

use super::app::{self, Flow};
use super::model::Model;

/// One frame of the clock. Both animations are derived from it.
pub const TICK: Duration = Duration::from_millis(250);

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
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            previous(info);
        }));
    });
}

/// Coalesce a bracketed paste that arrives as key events rather than as
/// [`Event::Paste`].
///
/// Unix terminals hand crossterm the `ESC[200~ … ESC[201~` byte stream and
/// crossterm parses it into one `Event::Paste`. The Windows console API has no
/// such notion: once `?2004h` is written, Windows Terminal still wraps a paste
/// in the markers, but ConPTY converts every byte into an individual key
/// event, so the markers and the pasted block arrive as a burst of keys — and
/// every newline in the block used to read as a submit. This filter watches
/// the key stream for the markers and reassembles the block, exactly as the
/// legacy chrome's `StdinBuffer` does at the byte level.
#[derive(Debug, Default)]
struct PasteFilter {
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
/// How long a partial marker may sit before it is flushed as real keys. A
/// paste burst arrives in one batch; a human typing `ESC [` cannot beat this.
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
                self.ready.push_back(other);
                return;
            }
        };

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
            self.held.push(key);
            self.start_matched = 1;
            return;
        }
        self.ready.push_back(Event::Key(key));
    }

    fn flush_held(&mut self) {
        for key in self.held.drain(..) {
            self.ready.push_back(Event::Key(key));
        }
        self.start_matched = 0;
    }

    /// Nothing more is immediately available. A partial start marker older
    /// than the patience window was a real escape: hand it back. A paste
    /// whose end marker never came is delivered as it stands rather than
    /// freezing the interface waiting for it.
    fn idle(&mut self) {
        let stale = self
            .last_fed
            .is_none_or(|at| at.elapsed() >= PASTE_MARKER_PATIENCE);
        if !stale {
            return;
        }
        if self.start_matched > 0 {
            self.flush_held();
        }
        if let Some(mut block) = self.pasting.take() {
            if self.end_matched > 0 {
                block.push('\u{1b}');
                for matched in &PASTE_END[..self.end_matched - 1] {
                    block.push(*matched);
                }
                self.end_matched = 0;
            }
            self.ready.push_back(Event::Paste(block));
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
        self.start_matched > 0
    }
}

/// The terminal, for as long as the TUI owns it.
pub struct Session {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    keyboard: Keyboard,
    paste: PasteFilter,
}

impl Session {
    /// Take the terminal: raw mode, alternate screen, and the kitty keyboard
    /// protocol when the terminal supports it.
    pub fn open() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        // Held from here, not after the screen is taken: if entering the
        // alternate screen fails, `restore()` still has to turn raw mode back
        // off, and it short-circuits on `HELD`.
        HELD.store(true, Ordering::SeqCst);
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        // A multi-line paste arrives as one `Event::Paste` rather than as a
        // burst of keys with newlines in it, which the composer would have
        // read as one submit per line.
        let _ = execute!(out, EnableBracketedPaste);

        let disambiguated = supports_keyboard_enhancement();
        if disambiguated {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
            DISAMBIGUATED.store(true, Ordering::SeqCst);
        }

        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self {
            terminal,
            keyboard: Keyboard { disambiguated },
            paste: PasteFilter::default(),
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
                return Ok(Some(ready));
            }
            // While a partial marker is held, wait only briefly: the rest of
            // a real marker is already in the queue, and a real escape should
            // not sit swallowed for a full tick.
            let wait = if self.paste.holding() {
                timeout.min(PASTE_MARKER_PATIENCE)
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
        execute!(io::stdout(), SetTitle(title))
    }

    pub fn size(&self) -> io::Result<(u16, u16)> {
        let area = self.terminal.size()?;
        Ok((area.width, area.height))
    }

    /// Paint one frame.
    pub fn draw(&mut self, model: &Model) -> io::Result<()> {
        let background = Style::default().bg(model.theme.background);
        self.terminal.draw(|frame| {
            let area: Rect = frame.area();
            let rows = app::compose(model, area.height);
            frame.render_widget(Paragraph::new(rows).style(background), area);
        })?;
        Ok(())
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
        restore()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.close();
    }
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
                        // The fixture runner has no agent to act on a
                        // choice; the live shell in `davinci_session` does.
                        Flow::Choose(_)
                        | Flow::Continue
                        | Flow::Interrupt
                        | Flow::CycleThinking => {}
                    }
                }
                Event::Resize(width, height) => {
                    model.width = width.max(20);
                    model.height = height.max(4);
                }
                Event::Paste(text) => model.paste(&text),
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
    fn the_clock_is_two_hundred_and_fifty_milliseconds() {
        assert_eq!(TICK, Duration::from_millis(250));
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
        // What ConPTY hands crossterm when Windows Terminal brackets a paste:
        // every byte of the markers and the block as an individual key event.
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
    fn a_paste_missing_its_end_marker_is_still_delivered_on_idle() {
        let mut filter = PasteFilter::default();
        feed_str(&mut filter, "\u{1b}[200~orphan");
        filter.last_fed = Some(Instant::now() - Duration::from_millis(200));
        filter.idle();
        assert_eq!(drain(&mut filter), vec![Event::Paste("orphan".to_string())]);
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
