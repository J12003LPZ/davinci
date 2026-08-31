//! The app shell: compose a window's worth of rows, route a key, paint.
//!
//! Layout is assembled as a flat list of rows rather than through nested
//! ratatui `Layout` splits, so the composer can be anchored to the bottom of
//! the window at any height and an overlay can dim the transcript behind it by
//! rendering it with the dropped ramp (design.md §1, §2).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/app.ex`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

use super::model::{Model, Overlay, Screen};
use super::ui::{blank, pad_to, tail};
use super::views::chrome::{self, Hint};
use super::views::transcript;

/// What the runtime should do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    /// Keep going.
    Continue,
    /// Leave the TUI. `ctrl+c` never produces this — it interrupts the run.
    Quit,
    /// The user asked to interrupt the run in progress.
    Interrupt,
    /// The composer was sent; the caller owns what happens next.
    Submit(String),
}

/// Compose exactly `height` rows: header, body, composer, status bar.
pub fn compose(model: &Model, height: u16) -> Vec<Line<'static>> {
    let height = height.max(4) as usize;
    let composer_rows = composer_rows(model);
    let reserved = 1 + composer_rows.len() + 1;
    let body_height = height.saturating_sub(reserved);

    let mut rows = Vec::with_capacity(height);
    rows.push(chrome::header(model));
    rows.extend(body(model, body_height));
    rows.extend(composer_rows);
    rows.push(chrome::status(model));
    rows.truncate(height);
    rows
}

fn composer_rows(model: &Model) -> Vec<Line<'static>> {
    let hint = match (model.screen, model.overlay) {
        (Screen::Memoria, _) => Hint::Recall,
        (Screen::Agent, None) => Hint::Default,
        _ => Hint::Closable,
    };
    chrome::composer(model, None, hint)
}

/// The transcript, bottom-anchored: older rows fall off the top like a
/// scrollback, and a short transcript is padded so the composer stays put.
fn body(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let width = model.width;
    let mut rows = transcript::lines(model, &model.transcript, width);
    if rows.len() > height {
        rows = tail(rows, height);
    } else {
        let lead = height - rows.len();
        let mut padded = vec![blank(); lead];
        padded.extend(rows);
        rows = padded;
    }
    pad_to(rows, height)
}

/// Route one key. `esc` closes the instrument in hand, `ctrl+c` interrupts the
/// run and never the app (design.md §6).
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if ctrl {
        match key.code {
            KeyCode::Char('c') => {
                model.interrupt();
                return Flow::Interrupt;
            }
            KeyCode::Char('d') => return Flow::Quit,
            KeyCode::Char('p') => {
                model.toggle_overlay(Overlay::Instrumenta);
                return Flow::Continue;
            }
            KeyCode::Char('s') => {
                model.toggle_overlay(Overlay::Sessions);
                return Flow::Continue;
            }
            KeyCode::Char('o') => {
                model.toggle_overlay(Overlay::Cogitator);
                return Flow::Continue;
            }
            KeyCode::Char('l') => {
                model.toggle_screen(Screen::Plan);
                return Flow::Continue;
            }
            KeyCode::Char('g') => {
                model.toggle_screen(Screen::Grafo);
                return Flow::Continue;
            }
            KeyCode::Char('u') => {
                model.toggle_screen(Screen::Mensura);
                return Flow::Continue;
            }
            KeyCode::Char('e') => {
                model.toggle_codex();
                return Flow::Continue;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => model.close(),
        KeyCode::Backspace => model.backspace(),
        KeyCode::Enter => {
            if model.overlay.is_some() {
                model.overlay = None;
            } else {
                let sent = model.composer.clone();
                model.submit();
                if !sent.trim().is_empty() {
                    return Flow::Submit(sent);
                }
            }
        }
        KeyCode::Char(ch) => model.type_char(&ch.to_string()),
        _ => {}
    }
    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Entry;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16, height: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            height,
            true,
        );
        model.cwd = "C:\\dev\\oss\\davinci-rust".into();
        model.branch = "main".into();
        model.model_name = "sonnet".into();
        model.context = (47_000, 200_000);
        model
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_window_is_filled_exactly_at_every_height() {
        for height in [10u16, 24, 44, 60] {
            let rows = compose(&model(100, height), height);
            assert_eq!(rows.len(), height as usize, "at height {height}");
        }
    }

    #[test]
    fn the_composer_is_anchored_to_the_bottom_above_the_status_bar() {
        let m = model(100, 24);
        let rows = compose(&m, 24);
        assert_eq!(text(&rows[19]).chars().next(), Some('╭'));
        assert!(text(&rows[20]).contains("› ask davinci…"));
        assert_eq!(text(&rows[21]).chars().next(), Some('╰'));
        assert!(text(&rows[22]).contains("enter send"));
        assert!(text(&rows[23]).starts_with("agent · main"));
    }

    #[test]
    fn the_header_and_status_bar_are_one_row_each_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width, 30);
            let rows = compose(&m, 30);
            assert_eq!(run_width(&rows[0].spans), width);
            assert_eq!(run_width(&rows[29].spans), width);
        }
    }

    #[test]
    fn a_long_transcript_scrolls_off_the_top_like_a_scrollback() {
        let mut m = model(100, 12);
        m.transcript = (0..40).map(|i| Entry::user(&format!("turn {i}"))).collect();
        let rows = compose(&m, 12);
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("turn 0")));
        assert!(drawn.iter().any(|row| row.contains("turn 39")));
    }

    #[test]
    fn a_short_transcript_sits_above_the_composer_not_under_the_header() {
        let mut m = model(100, 20);
        m.transcript = vec![Entry::user("run the tests")];
        let rows = compose(&m, 20);
        assert!(text(&rows[1]).is_empty(), "the gap is above the turn");
        assert!(
            text(&rows[14]).contains("> run the tests"),
            "the turn sits directly above the composer, not under the header"
        );
        assert_eq!(text(&rows[15]).chars().next(), Some('╭'));
    }

    #[test]
    fn ctrl_c_interrupts_the_run_and_never_the_app() {
        let mut m = model(100, 24);
        m.type_char("cargo test");
        m.submit();
        assert!(m.running);
        assert_eq!(handle_key(&mut m, ctrl('c')), Flow::Interrupt);
        assert!(!m.running);
        assert!(!m.transcript.is_empty());
    }

    #[test]
    fn every_instrument_has_a_key_and_esc_closes_it() {
        let mut m = model(160, 44);
        for (ch, expected) in [
            ('l', Screen::Plan),
            ('g', Screen::Grafo),
            ('u', Screen::Mensura),
        ] {
            handle_key(&mut m, ctrl(ch));
            assert_eq!(m.screen, expected, "ctrl+{ch}");
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.screen, Screen::Agent);
        }
        for (ch, expected) in [
            ('p', Overlay::Instrumenta),
            ('s', Overlay::Sessions),
            ('o', Overlay::Cogitator),
        ] {
            handle_key(&mut m, ctrl(ch));
            assert_eq!(m.overlay, Some(expected), "ctrl+{ch}");
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.overlay, None);
        }
        handle_key(&mut m, ctrl('e'));
        assert!(m.codex_open());
    }

    #[test]
    fn typing_and_sending_a_turn() {
        let mut m = model(100, 24);
        for ch in "run the tests".chars() {
            handle_key(&mut m, key(KeyCode::Char(ch)));
        }
        assert_eq!(m.composer, "run the tests");
        handle_key(&mut m, key(KeyCode::Backspace));
        assert_eq!(m.composer, "run the test");

        let flow = handle_key(&mut m, key(KeyCode::Enter));
        assert_eq!(flow, Flow::Submit("run the test".to_string()));
        assert_eq!(m.composer, "");
    }

    #[test]
    fn enter_on_an_empty_composer_does_nothing() {
        let mut m = model(100, 24);
        assert_eq!(handle_key(&mut m, key(KeyCode::Enter)), Flow::Continue);
        assert!(m.transcript.is_empty());
    }

    #[test]
    fn a_key_that_belongs_to_the_composer_is_never_a_quit() {
        let mut m = model(100, 24);
        for ch in ['q', 'x', 'Q'] {
            assert_eq!(handle_key(&mut m, key(KeyCode::Char(ch))), Flow::Continue);
        }
        assert_eq!(m.composer, "qxQ");
        assert_eq!(handle_key(&mut m, ctrl('d')), Flow::Quit);
    }

    #[test]
    fn nothing_breaks_below_eighty_columns() {
        let mut m = model(60, 18);
        m.transcript = vec![
            Entry::user("run the tests"),
            Entry::Gap,
            Entry::agent("davinci"),
        ];
        let rows = compose(&m, 18);
        assert_eq!(rows.len(), 18);
        for row in &rows {
            assert!(
                run_width(&row.spans) <= 60,
                "row overflows 60 columns: {:?}",
                text(row)
            );
        }
    }
}
