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
use super::views::{
    codex, cogitator, disegno, grafo, instrumenta, memoria, mensura, startup, transcript,
};

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
/// An empty transcript is the empty state instead, centred in the body (`1a`).
fn body(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if let Some(overlay) = model.overlay {
        return overlay_body(model, overlay, height);
    }
    let screen_rows = match model.screen {
        Screen::Plan => Some(disegno::lines(model)),
        Screen::Grafo => Some(grafo::lines(model)),
        Screen::Memoria => Some(memoria::recall(model)),
        Screen::Mensura => Some(mensura::lines(model)),
        Screen::Agent => None,
    };
    if let Some(rows) = screen_rows {
        return panel(model, rows, height);
    }
    if model.codex_open() {
        return codex::lines(model, height);
    }
    if model.transcript.is_empty() {
        return empty_state(model, height);
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

/// The identity mark and what the session found, vertically centred. If the
/// window is too short for the mark, the mark goes rather than the words.
fn empty_state(model: &Model, height: usize) -> Vec<Line<'static>> {
    let mut rows = startup::lines(model, &model.startup);
    if rows.len() > height {
        rows = rows.split_off(rows.len() - height);
    }
    let lead = (height - rows.len()) / 2;
    let mut out = vec![blank(); lead];
    out.extend(rows);
    pad_to(out, height)
}

/// A screen that takes over the body: the turn that produced it stays visible
/// above, and the panel is anchored above the composer, as the mockups show
/// (`1c`, `2a`, `2b`, `2c`).
fn panel(model: &Model, rows: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    let panel = tail(rows, height);
    let room = height - panel.len();
    let mut above = tail(
        transcript::lines(model, &model.transcript, model.width),
        room,
    );
    while above.len() < room {
        above.insert(0, blank());
    }
    above.extend(panel);
    above
}

/// An instrument in hand: the transcript stays visible behind it with the ramp
/// dropped, and the panel is drawn over it, anchored above the composer
/// (design.md §2, screens `1d` and `1f`).
fn overlay_body(model: &Model, overlay: Overlay, height: usize) -> Vec<Line<'static>> {
    let dimmed = Model {
        theme: model.theme.dim(),
        overlay: None,
        ..model.clone()
    };
    let mut behind = body(&dimmed, height);

    let panel = match overlay {
        Overlay::Instrumenta => instrumenta::lines(model),
        Overlay::Sessions => memoria::sessions(model),
        Overlay::Cogitator => cogitator::lines(model, &model.config_path),
    };

    let panel = if panel.len() > height {
        tail(panel, height)
    } else {
        panel
    };
    behind.truncate(height - panel.len());
    behind.extend(panel);
    behind
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
            // design.md §5 asks for ctrl+m here. At the ANSI and Windows
            // console layers ctrl+m *is* enter (0x0D), and crossterm can only
            // tell them apart through the kitty keyboard protocol, which the
            // Windows console does not implement. Binding it would cost the
            // composer its enter key, so recall lives on ctrl+r.
            KeyCode::Char('r') => {
                model.toggle_screen(Screen::Memoria);
                return Flow::Continue;
            }
            KeyCode::Char('e') => {
                model.toggle_codex();
                return Flow::Continue;
            }
            // An unbound control chord is not text; it must never reach the
            // composer.
            KeyCode::Char(_) => return Flow::Continue,
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
        KeyCode::Up => model.move_selection(-1),
        KeyCode::Down => model.move_selection(1),
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
            // ctrl+m in the spec; see the note in `handle_key`.
            ('r', Screen::Memoria),
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
    fn ctrl_m_stays_enter_so_the_composer_keeps_its_send_key() {
        let mut m = model(120, 30);
        m.type_char("run the tests");
        // The terminal delivers ctrl+m as enter; recall must not steal it.
        let flow = handle_key(&mut m, ctrl('m'));
        assert_eq!(flow, Flow::Continue);
        assert_eq!(m.screen, Screen::Agent, "ctrl+m did not open a screen");

        let flow = handle_key(&mut m, key(KeyCode::Enter));
        assert_eq!(flow, Flow::Submit("run the tests".to_string()));
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

    /// Every state the shell can be in, so the responsive and NO_COLOR audits
    /// can walk all of them.
    fn every_surface(width: u16, height: u16) -> Vec<(String, Model)> {
        use crate::davinci::fixtures;

        let base = |screen: &str| {
            let mut model = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, false),
                width,
                height,
                true,
            );
            fixtures::dress_screen(&mut model, screen);
            model.config_path = "%USERPROFILE%\\.pi\\config.json".into();
            model
        };

        let mut all: Vec<(String, Model)> = ["1a", "1b", "1c", "1d", "1e", "1f", "2a", "2b", "2c"]
            .iter()
            .map(|screen| (screen.to_string(), base(screen)))
            .collect();
        all.push(("1f-cogitator".into(), base("1f-cogitator")));
        all
    }

    #[test]
    fn no_screen_overflows_its_window_at_any_breakpoint() {
        for (width, height) in [(60u16, 20u16), (80, 24), (100, 30), (120, 40), (160, 44)] {
            for (screen, model) in every_surface(width, height) {
                let rows = compose(&model, height);
                assert_eq!(rows.len(), height as usize, "{screen} at {width}");
                for row in &rows {
                    assert!(
                        run_width(&row.spans) <= width,
                        "{screen} overflows {width}: {:?}",
                        text(row)
                    );
                }
            }
        }
    }

    #[test]
    fn below_eighty_columns_there_is_only_a_transcript_and_a_composer() {
        for (screen, model) in every_surface(60, 20) {
            assert!(!model.codex_open(), "{screen} opened a split below 80");
            assert_eq!(
                model.overlay_inset(),
                0,
                "{screen} inset a panel below 80 instead of filling the window"
            );
        }
    }

    #[test]
    fn the_codex_split_never_opens_below_a_hundred_and_twenty_columns() {
        for width in [60u16, 80, 100, 119] {
            let mut m = model(width, 30);
            m.toggle_codex();
            assert!(!m.codex_open(), "a split opened at {width}");
        }
        let mut m = model(120, 30);
        m.toggle_codex();
        assert!(m.codex_open());
    }

    #[test]
    fn no_color_leaves_every_state_readable_by_glyph_alone() {
        use crate::davinci::fixtures;

        for screen in ["1a", "1b", "1c", "1d", "1e", "1f", "2a", "2b", "2c"] {
            let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 120, 40, true);
            fixtures::dress_screen(&mut m, screen);
            let rows = compose(&m, 40);

            for row in &rows {
                for span in &row.spans {
                    if let Some(ratatui::style::Color::Rgb(r, g, b)) = span.style.fg {
                        assert!(
                            r == g && g == b,
                            "{screen} drew a colored run under NO_COLOR: {:?}",
                            span.content
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_transcript_still_states_every_outcome_under_no_color() {
        use crate::davinci::fixtures;

        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 100, 40, true);
        fixtures::dress_screen(&mut m, "1b");
        let drawn: String = compose(&m, 40).iter().map(|row| text(row)).collect();

        // Screen 1h: done, failed, in progress, queued, change and read all
        // read from the glyph alone.
        for glyph in ['✓', '×', '○', 'Δ', '↳', '⌕'] {
            assert!(drawn.contains(glyph), "{glyph} is missing under NO_COLOR");
        }
    }

    #[test]
    fn animations_stop_under_no_animation() {
        use crate::davinci::fixtures;

        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            120,
            40,
            false,
        );
        fixtures::dress_screen(&mut m, "1b");

        let frames: Vec<String> = (0..8)
            .map(|tick| {
                m.tick = tick;
                compose(&m, 40).iter().map(|row| text(row)).collect()
            })
            .collect();
        assert!(
            frames.windows(2).all(|pair| pair[0] == pair[1]),
            "something moved with --no-animation"
        );
    }

    #[test]
    fn exactly_two_things_move_when_animation_is_on() {
        use crate::davinci::fixtures;

        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, true);
        fixtures::dress_screen(&mut m, "1b");

        let mut moving: Vec<char> = Vec::new();
        let base: Vec<String> = {
            m.tick = 0;
            compose(&m, 40).iter().map(|row| text(row)).collect()
        };
        for tick in 1..8u64 {
            m.tick = tick;
            let frame: Vec<String> = compose(&m, 40).iter().map(|row| text(row)).collect();
            for (before, after) in base.iter().zip(&frame) {
                if before == after {
                    continue;
                }
                for (a, b) in before.chars().zip(after.chars()) {
                    if a != b {
                        moving.push(a);
                        moving.push(b);
                    }
                }
            }
        }
        moving.sort_unstable();
        moving.dedup();
        // The spinner's four frames and the caret's two states, nothing else.
        for ch in &moving {
            assert!("◜◝◞◟ ".contains(*ch), "{ch:?} animates, and it should not");
        }
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
