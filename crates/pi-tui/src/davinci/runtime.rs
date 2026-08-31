//! Terminal ownership and the event loop.
//!
//! One clock drives everything: a 250ms tick advances the caret blink and the
//! single Studio spinner, and nothing else animates (design.md §8). Panels open
//! and close in one frame.

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
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

/// The terminal, for as long as the TUI owns it.
pub struct Session {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    keyboard: Keyboard,
}

impl Session {
    /// Take the terminal: raw mode, alternate screen, and the kitty keyboard
    /// protocol when the terminal supports it.
    pub fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;

        let disambiguated = supports_keyboard_enhancement();
        if disambiguated {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }

        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self {
            terminal,
            keyboard: Keyboard { disambiguated },
        })
    }

    pub fn keyboard(&self) -> Keyboard {
        self.keyboard
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

    /// Give the terminal back. Safe to call twice.
    pub fn close(&mut self) -> io::Result<()> {
        if self.keyboard.disambiguated {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        disable_raw_mode()?;
        self.terminal.show_cursor()?;
        io::stdout().flush()?;
        Ok(())
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
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    match app::handle_key(model, key) {
                        Flow::Quit => break,
                        Flow::Submit(text) => on_submit(model, text),
                        Flow::Continue | Flow::Interrupt => {}
                    }
                }
                Event::Resize(width, height) => {
                    model.width = width;
                    model.height = height;
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
    fn the_clock_is_two_hundred_and_fifty_milliseconds() {
        assert_eq!(TICK, Duration::from_millis(250));
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
