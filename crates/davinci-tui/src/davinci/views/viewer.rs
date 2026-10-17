//! The transcript viewer (Ctrl+O): the whole conversation, every tool call
//! expanded, in a read-only pager.
//!
//! `j`/`k` or the arrows step a row; space/`b`, `d`/`u` and PgDn/PgUp move
//! by page and half page; `g`/`G` reach the ends; `/` searches with `n`/`N`;
//! `{`/`}` jump between prompts; `[` writes the transcript to the terminal's
//! own scrollback and `v` opens it in `$VISUAL`/`$EDITOR`; `q` or esc leaves.
//!
//! The viewer reads a snapshot taken when it opens (and again on resize), so
//! rows do not move under the reader while a turn streams in.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::alt_screen_search::find_alt_screen_search_matches;
use crate::davinci::model::{Entry, Model};
use crate::davinci::ui::{self, blank};
use crate::davinci::views::transcript;

#[derive(Debug, Clone, Default)]
pub struct TranscriptViewer {
    pub lines: Vec<Line<'static>>,
    pub plain: Vec<String>,
    /// Row each prompt starts on.
    pub prompts: Vec<usize>,
    pub width: u16,
    pub top: usize,
    /// The search being typed, before enter.
    pub typing: Option<String>,
    pub query: String,
    pub matches: Vec<usize>,
    pub current: usize,
}

/// What the terminal must do outside the alternate screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerTask {
    /// Print the text to the main screen and wait for esc or q.
    Scrollback(String),
    /// Open the text in `$VISUAL`/`$EDITOR`.
    Editor(String),
}

/// Rows the header and footer take.
const CHROME_ROWS: usize = 2;

impl TranscriptViewer {
    /// Snapshot `model`'s conversation at its width, with tool output shown.
    pub fn open(model: &Model) -> Self {
        let width = model.width.saturating_sub(1).max(1);
        let mut expanded = model.clone();
        expanded.show_tool_output = true;
        let rows = transcript::conversation_rows(&expanded, width);
        let lines: Vec<Line<'static>> = rows.slice(0..rows.len());
        let prompts = model
            .transcript
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, Entry::User(_)))
            .map(|(index, _)| transcript::rows_before(&expanded, index, width))
            .collect();
        let plain = lines
            .iter()
            .map(|line| {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                text.trim_end().to_owned()
            })
            .collect();
        let mut viewer = Self {
            lines,
            plain,
            prompts,
            width: model.width,
            ..Self::default()
        };
        viewer.top = viewer.max_top(usize::from(model.height));
        viewer
    }

    fn body(height: usize) -> usize {
        height.saturating_sub(CHROME_ROWS).max(1)
    }

    fn max_top(&self, height: usize) -> usize {
        self.lines.len().saturating_sub(Self::body(height))
    }

    pub fn text(&self) -> String {
        let mut text = self.plain.join("\n");
        text.push('\n');
        text
    }

    fn scroll(&mut self, delta: isize, height: usize) {
        self.top = self
            .top
            .saturating_add_signed(delta)
            .min(self.max_top(height));
    }

    fn search(&mut self) {
        self.matches.clear();
        if !self.query.is_empty() {
            for found in find_alt_screen_search_matches(&self.plain, &self.query) {
                if let Some(segment) = found.segments.first() {
                    if self.matches.last() != Some(&segment.row) {
                        self.matches.push(segment.row);
                    }
                }
            }
        }
        // Start from the first match at or below the top of the view.
        self.current = self
            .matches
            .iter()
            .position(|&row| row >= self.top)
            .unwrap_or(0);
    }

    fn show_match(&mut self, height: usize) {
        if let Some(&row) = self.matches.get(self.current) {
            self.top = row.saturating_sub(2).min(self.max_top(height));
        }
    }

    fn step_match(&mut self, forward: bool, height: usize) {
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len();
        self.current = if forward {
            (self.current + 1) % count
        } else {
            (self.current + count - 1) % count
        };
        self.show_match(height);
    }

    fn jump_prompt(&mut self, forward: bool, height: usize) {
        let target = if forward {
            self.prompts.iter().copied().find(|&row| row > self.top)
        } else {
            self.prompts
                .iter()
                .rev()
                .copied()
                .find(|&row| row < self.top)
        };
        if let Some(row) = target {
            self.top = row.min(self.max_top(height));
        }
    }
}

/// What a key did to the viewer.
pub enum Outcome {
    Stay,
    Close,
    Task(ViewerTask),
}

/// Give a key to the open viewer.
pub fn handle_key(viewer: &mut TranscriptViewer, key: KeyEvent, height: usize) -> Outcome {
    let page = TranscriptViewer::body(height) as isize;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if let Some(typed) = &mut viewer.typing {
        match key.code {
            KeyCode::Esc => viewer.typing = None,
            KeyCode::Enter => {
                viewer.query = std::mem::take(typed);
                viewer.typing = None;
                viewer.search();
                viewer.show_match(height);
            }
            KeyCode::Backspace => {
                typed.pop();
            }
            KeyCode::Char(ch) if !ctrl => typed.push(ch),
            _ => {}
        }
        return Outcome::Stay;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Outcome::Close,
        KeyCode::Char('c' | 'o') if ctrl => return Outcome::Close,
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => viewer.scroll(1, height),
        KeyCode::Char('k') | KeyCode::Up => viewer.scroll(-1, height),
        KeyCode::Char(' ') | KeyCode::PageDown => viewer.scroll(page, height),
        KeyCode::Char('f') if ctrl => viewer.scroll(page, height),
        KeyCode::Char('b') | KeyCode::PageUp => viewer.scroll(-page, height),
        KeyCode::Char('d') => viewer.scroll(page / 2, height),
        KeyCode::Char('u') => viewer.scroll(-(page / 2), height),
        KeyCode::Char('g') | KeyCode::Home => viewer.top = 0,
        KeyCode::Char('G') | KeyCode::End => viewer.top = viewer.max_top(height),
        KeyCode::Char('/') => viewer.typing = Some(String::new()),
        KeyCode::Char('n') => viewer.step_match(true, height),
        KeyCode::Char('N') => viewer.step_match(false, height),
        KeyCode::Char('}') => viewer.jump_prompt(true, height),
        KeyCode::Char('{') => viewer.jump_prompt(false, height),
        KeyCode::Char('[') => return Outcome::Task(ViewerTask::Scrollback(viewer.text())),
        KeyCode::Char('v') => return Outcome::Task(ViewerTask::Editor(viewer.text())),
        _ => {}
    }
    Outcome::Stay
}

/// The viewer as a full screen of `height` rows.
pub fn render(model: &Model, viewer: &TranscriptViewer, height: usize) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let width = model.width;
    let body = TranscriptViewer::body(height);
    let last = (viewer.top + body).min(viewer.lines.len());
    let mut out = Vec::with_capacity(height);
    out.push(Line::from(vec![
        ui::span("  Transcript", cc.permission),
        ui::span(
            format!(
                "  rows {}–{} of {}",
                if viewer.lines.is_empty() {
                    0
                } else {
                    viewer.top + 1
                },
                last,
                viewer.lines.len()
            ),
            cc.inactive,
        ),
    ]));
    for (offset, line) in viewer.lines[viewer.top.min(viewer.lines.len())..last]
        .iter()
        .enumerate()
    {
        let row = viewer.top + offset;
        let mut spans: Vec<Span<'static>> = line.spans.clone();
        if viewer.matches.binary_search(&row).is_ok() {
            let modifier = if viewer.matches.get(viewer.current) == Some(&row) {
                Modifier::REVERSED
            } else {
                Modifier::UNDERLINED
            };
            for span in &mut spans {
                span.style = span.style.add_modifier(modifier);
            }
        }
        out.push(Line::from(ui::truncate_run(spans, width)));
    }
    while out.len() < body + 1 {
        out.push(blank());
    }
    out.push(Line::from(match &viewer.typing {
        Some(typed) => vec![
            ui::span("  /", cc.permission),
            ui::span(typed.clone(), cc.permission),
            ui::span("▏", cc.permission),
        ],
        None if !viewer.query.is_empty() => vec![ui::span(
            if viewer.matches.is_empty() {
                format!("  /{}  no match", viewer.query)
            } else {
                format!(
                    "  /{}  {} of {} · n/N next/prev",
                    viewer.query,
                    viewer.current + 1,
                    viewer.matches.len()
                )
            },
            cc.inactive,
        )],
        None => vec![ui::span(
            "  q close · j/k space/b scroll · / search · { } prompts · [ scrollback · v editor",
            cc.inactive,
        )],
    }));
    out.truncate(height);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(lines: &[&str]) -> TranscriptViewer {
        TranscriptViewer {
            lines: lines.iter().map(|l| Line::from(l.to_string())).collect(),
            plain: lines.iter().map(|l| l.to_string()).collect(),
            ..TranscriptViewer::default()
        }
    }

    fn press(viewer: &mut TranscriptViewer, code: KeyCode, height: usize) -> Outcome {
        handle_key(viewer, KeyEvent::new(code, KeyModifiers::NONE), height)
    }

    #[test]
    fn paging_stays_inside_the_transcript() {
        let rows: Vec<String> = (0..50).map(|n| format!("row {n}")).collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let mut v = viewer(&refs);
        press(&mut v, KeyCode::Char('g'), 12);
        assert_eq!(v.top, 0);
        press(&mut v, KeyCode::Char(' '), 12);
        assert_eq!(v.top, 10);
        press(&mut v, KeyCode::Char('G'), 12);
        assert_eq!(v.top, 40);
        press(&mut v, KeyCode::Char('j'), 12);
        assert_eq!(v.top, 40);
        press(&mut v, KeyCode::Char('k'), 12);
        assert_eq!(v.top, 39);
    }

    #[test]
    fn search_finds_rows_and_n_walks_them() {
        let mut v = viewer(&["alpha", "beta", "gamma alpha", "delta", "alpha again"]);
        v.top = 0;
        press(&mut v, KeyCode::Char('/'), 4);
        for ch in "alpha".chars() {
            press(&mut v, KeyCode::Char(ch), 4);
        }
        press(&mut v, KeyCode::Enter, 4);
        assert_eq!(v.matches, [0, 2, 4]);
        assert_eq!(v.current, 0);
        press(&mut v, KeyCode::Char('n'), 4);
        assert_eq!(v.current, 1);
        press(&mut v, KeyCode::Char('N'), 4);
        press(&mut v, KeyCode::Char('N'), 4);
        assert_eq!(v.current, 2, "n and N wrap around");
    }

    #[test]
    fn braces_jump_between_prompts() {
        let rows: Vec<String> = (0..40).map(|n| format!("row {n}")).collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let mut v = viewer(&refs);
        v.prompts = vec![0, 10, 20];
        v.top = 5;
        press(&mut v, KeyCode::Char('}'), 12);
        assert_eq!(v.top, 10);
        press(&mut v, KeyCode::Char('}'), 12);
        assert_eq!(v.top, 20);
        press(&mut v, KeyCode::Char('{'), 12);
        assert_eq!(v.top, 10);
    }

    #[test]
    fn q_closes_and_the_dump_keys_hand_back_the_text() {
        let mut v = viewer(&["one", "two"]);
        assert!(matches!(
            press(&mut v, KeyCode::Char('q'), 8),
            Outcome::Close
        ));
        assert!(matches!(
            press(&mut v, KeyCode::Char('['), 8),
            Outcome::Task(ViewerTask::Scrollback(text)) if text == "one\ntwo\n"
        ));
        assert!(matches!(
            press(&mut v, KeyCode::Char('v'), 8),
            Outcome::Task(ViewerTask::Editor(_))
        ));
    }

    #[test]
    fn typing_a_query_does_not_trigger_pager_keys() {
        let mut v = viewer(&["q", "j"]);
        press(&mut v, KeyCode::Char('/'), 8);
        assert!(matches!(
            press(&mut v, KeyCode::Char('q'), 8),
            Outcome::Stay
        ));
        assert_eq!(v.typing.as_deref(), Some("q"));
        assert!(matches!(press(&mut v, KeyCode::Esc, 8), Outcome::Stay));
        assert!(v.typing.is_none());
    }
}
