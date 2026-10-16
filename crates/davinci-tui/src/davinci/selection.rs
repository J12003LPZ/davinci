//! Text selection inside the alternate screen: what is selected, and the
//! text it covers.
//!
//! A selection that starts in the conversation is held in conversation
//! rows, not screen rows, so scrolling keeps the same text selected. One that
//! starts anywhere else (the composer, a panel) is held in screen rows.
//! Double-click widens to a word, triple-click to the row; dragging after a
//! multi-click extends by whole words or rows.

use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;

/// Two presses this close in time and place count as one multi-click.
pub const MULTI_CLICK: Duration = Duration::from_millis(500);
/// How far, in cells, the pointer may wander between presses of a multi-click.
pub const MULTI_CLICK_SLOP: u16 = 1;

/// Which rows a [`Point::line`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Rows of the conversation, banner first, independent of scrolling.
    Conversation,
    /// Rows of the screen.
    Screen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Cell,
    Word,
    Line,
}

/// A cell: a row in the selection's [`Space`] and a display column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: usize,
    pub column: u16,
}

impl Point {
    pub fn new(line: usize, column: u16) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub space: Space,
    pub granularity: Granularity,
    /// The cells the press itself covered: one cell, a word, or a row. A drag
    /// always keeps them selected.
    pub origin: (Point, Point),
    /// Where the pointer or keyboard has carried the far end.
    pub focus: Point,
    /// Whether the press has moved; a plain click selects nothing.
    pub extended: bool,
}

impl Selection {
    pub fn new(space: Space, granularity: Granularity, origin: (Point, Point)) -> Self {
        Self {
            space,
            granularity,
            origin,
            focus: origin.1,
            extended: granularity != Granularity::Cell,
        }
    }

    /// The first and last selected cells, both inclusive, or `None` while a
    /// plain click has not moved.
    pub fn range(&self) -> Option<(Point, Point)> {
        if !self.extended {
            return None;
        }
        let (start, end) = self.origin;
        Some(if self.focus < start {
            (self.focus, end)
        } else if self.focus > end {
            (start, self.focus)
        } else {
            (start, end)
        })
    }

    /// Move the far end to `point`, widened to the selection's granularity
    /// using `text_of` for the row under it.
    pub fn drag_to(&mut self, point: Point, text_of: impl Fn(usize) -> String) {
        let point = match self.granularity {
            Granularity::Cell => point,
            Granularity::Word => {
                let text = text_of(point.line);
                match word_bounds(&text, point.column) {
                    Some((start, _)) if point < self.origin.0 => Point::new(point.line, start),
                    Some((_, end)) if point > self.origin.1 => Point::new(point.line, end),
                    Some(_) => self.origin.1,
                    None => point,
                }
            }
            Granularity::Line => {
                if point < self.origin.0 {
                    Point::new(point.line, 0)
                } else {
                    Point::new(point.line, width(&text_of(point.line)).saturating_sub(1))
                }
            }
        };
        self.extended |= point != self.origin.0;
        self.focus = point;
    }

    /// Move the far end by whole cells or rows from the keyboard.
    pub fn extend_by(&mut self, lines: isize, columns: isize, text_of: impl Fn(usize) -> String) {
        let mut focus = self.focus;
        focus.line = focus.line.saturating_add_signed(lines);
        let row_width = width(&text_of(focus.line));
        if columns < 0 && focus.column == 0 && focus.line > 0 && lines == 0 {
            focus.line -= 1;
            focus.column = width(&text_of(focus.line));
        } else if columns > 0 && focus.column >= row_width && lines == 0 {
            focus.line += 1;
            focus.column = 0;
        } else {
            focus.column = focus.column.saturating_add_signed(columns as i16);
        }
        self.focus = focus;
        self.extended = true;
    }
}

/// Counts presses into single, double and triple clicks.
#[derive(Debug, Clone, Default)]
pub struct Clicks {
    last: Option<(Instant, u16, u16)>,
    count: u8,
}

impl Clicks {
    /// Record a press at screen cell `(column, row)`; returns 1, 2 or 3.
    pub fn press(&mut self, now: Instant, column: u16, row: u16) -> u8 {
        let near = self.last.is_some_and(|(at, last_column, last_row)| {
            now.saturating_duration_since(at) < MULTI_CLICK
                && column.abs_diff(last_column) <= MULTI_CLICK_SLOP
                && row.abs_diff(last_row) <= MULTI_CLICK_SLOP
        });
        self.count = if near { self.count % 3 + 1 } else { 1 };
        self.last = Some((now, column, row));
        self.count
    }

    pub fn reset(&mut self) {
        self.last = None;
        self.count = 0;
    }
}

/// Characters that belong to a word for double-click. Besides letters and
/// digits this keeps a path, a URL with its scheme and query, an e-mail
/// address or a `--flag=value` together, as terminals' own smart selection
/// does.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || "_-./~:@%+=?&#".contains(ch)
}

/// Display width of `text`.
pub fn width(text: &str) -> u16 {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0) as u16)
        .sum()
}

/// `(display column, char)` for each visible character of `text`.
fn cells(text: &str) -> Vec<(u16, char)> {
    let mut column = 0u16;
    let mut out = Vec::new();
    for ch in text.chars() {
        let cells = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if cells == 0 {
            continue;
        }
        out.push((column, ch));
        column += cells;
    }
    out
}

/// The first and last display columns of the word under `column`, or `None`
/// when the cell holds no word character. Trailing sentence punctuation is
/// not part of a word, so `see src/a.rs.` selects `src/a.rs`.
pub fn word_bounds(text: &str, column: u16) -> Option<(u16, u16)> {
    let cells = cells(text);
    let at = cells.iter().rposition(|(start, _)| *start <= column)?;
    if !is_word_char(cells[at].1) {
        return None;
    }
    let mut first = at;
    while first > 0 && is_word_char(cells[first - 1].1) {
        first -= 1;
    }
    let mut last = at;
    while last + 1 < cells.len() && is_word_char(cells[last + 1].1) {
        last += 1;
    }
    while last > first && ".,:;?".contains(cells[last].1) {
        last -= 1;
    }
    let end_width = UnicodeWidthChar::width(cells[last].1).unwrap_or(1) as u16;
    Some((cells[first].0, cells[last].0 + end_width - 1))
}

/// The characters of `text` whose cells fall in columns `from..=to`.
pub fn slice_columns(text: &str, from: u16, to: u16) -> String {
    let mut column = 0u16;
    let mut out = String::new();
    for ch in text.chars() {
        let cells = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if column >= from && column <= to {
            out.push(ch);
        }
        column = column.saturating_add(cells);
    }
    out
}

/// The text from `start` to `end` (both inclusive), rows joined by `\n` with
/// trailing blanks dropped, as a terminal copies it.
pub fn text_between(start: Point, end: Point, text_of: impl Fn(usize) -> String) -> String {
    let mut rows = Vec::new();
    for line in start.line..=end.line {
        let text = text_of(line);
        let from = if line == start.line { start.column } else { 0 };
        let to = if line == end.line {
            end.column
        } else {
            u16::MAX
        };
        rows.push(slice_columns(&text, from, to).trim_end().to_string());
    }
    rows.join("\n")
}

// ---- pointer and keyboard -------------------------------------------------

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::model::Model;
use super::views::scrollbar::Scrollbar;

/// Where the scrolling conversation sat in the last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// First screen row of the conversation.
    pub top: u16,
    pub height: u16,
    /// The conversation row drawn at `top`.
    pub first: usize,
    /// First screen column past the text (the scrollbar's column, or the
    /// window's width when none is drawn).
    pub right: u16,
}

impl View {
    pub fn from_scrollbar(bar: &Scrollbar) -> Self {
        Self {
            top: bar.top,
            height: bar.height,
            first: bar
                .total
                .saturating_sub(usize::from(bar.height) + bar.offset),
            right: if bar.drawn {
                bar.column
            } else {
                bar.column.saturating_add(1)
            },
        }
    }

    fn contains(&self, column: u16, row: u16) -> bool {
        row >= self.top && row < self.top.saturating_add(self.height) && column < self.right
    }

    fn line_at(&self, row: u16) -> usize {
        let last = self.top + self.height.saturating_sub(1);
        self.first + usize::from(row.clamp(self.top, last) - self.top)
    }

    fn row_of(&self, line: usize) -> Option<u16> {
        (line >= self.first && line < self.first + usize::from(self.height))
            .then(|| self.top + (line - self.first) as u16)
    }
}

/// What a key did to the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Not a selection key; the shell handles it as usual.
    Pass,
    /// The selection took the key. `Some` carries text to copy.
    Taken(Option<String>),
}

/// Selection driven by the pointer and the keyboard, kept apart from the
/// terminal so it can be tested without one.
#[derive(Debug, Clone)]
pub struct Selector {
    pub selection: Option<Selection>,
    clicks: Clicks,
    pressing: bool,
    /// The conversation's place in the last frame; `None` when it is not on
    /// screen or does not scroll.
    pub view: Option<View>,
    /// Copy as soon as the button is released (the `copyOnSelect` setting).
    pub copy_on_select: bool,
}

impl Default for Selector {
    fn default() -> Self {
        Self {
            selection: None,
            clicks: Clicks::default(),
            pressing: false,
            view: None,
            copy_on_select: true,
        }
    }
}

impl Selector {
    pub fn has_selection(&self) -> bool {
        self.selection.as_ref().is_some_and(|s| s.range().is_some())
    }

    pub fn clear(&mut self) {
        self.selection = None;
        self.pressing = false;
    }

    fn point(&self, space: Space, column: u16, row: u16) -> Point {
        match (space, self.view) {
            (Space::Conversation, Some(view)) => {
                Point::new(view.line_at(row), column.min(view.right.saturating_sub(1)))
            }
            _ => Point::new(usize::from(row), column),
        }
    }

    fn text_of(&self, model: &Model, rendered: &[String], space: Space, line: usize) -> String {
        match space {
            Space::Screen => rendered.get(line).cloned().unwrap_or_default(),
            Space::Conversation => {
                if let Some(row) = self.view.and_then(|view| view.row_of(line)) {
                    if let Some(text) = rendered.get(usize::from(row)) {
                        return text.clone();
                    }
                }
                super::app::conversation_text(model, line..line + 1)
                    .pop()
                    .unwrap_or_default()
            }
        }
    }

    /// The selected text, read from the conversation itself when the
    /// selection is held in conversation rows, so rows scrolled out of view
    /// and the `↓ … below` line drawn over the last row do not get in the way.
    pub fn selected_text(&self, model: &Model, rendered: &[String]) -> Option<String> {
        let selection = self.selection.as_ref()?;
        let (start, end) = selection.range()?;
        let text = match selection.space {
            Space::Screen => text_between(start, end, |line| {
                rendered.get(line).cloned().unwrap_or_default()
            }),
            Space::Conversation => {
                let rows = super::app::conversation_text(model, start.line..end.line + 1);
                text_between(start, end, |line| {
                    rows.get(line - start.line).cloned().unwrap_or_default()
                })
            }
        };
        (!text.trim().is_empty()).then_some(text)
    }

    /// Feed one pointer event. Returns text to copy when a selection is
    /// released and copy-on-select is on.
    pub fn mouse(
        &mut self,
        model: &mut Model,
        rendered: &[String],
        mouse: MouseEvent,
        now: Instant,
    ) -> Option<String> {
        let (column, row) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let space = if self.view.is_some_and(|view| view.contains(column, row)) {
                    Space::Conversation
                } else {
                    Space::Screen
                };
                let count = self.clicks.press(now, column, row);
                let point = self.point(space, column, row);
                let text = self.text_of(model, rendered, space, point.line);
                let (granularity, origin) = match count {
                    2 => match word_bounds(&text, point.column) {
                        Some((start, end)) => (
                            Granularity::Word,
                            (Point::new(point.line, start), Point::new(point.line, end)),
                        ),
                        None => (Granularity::Cell, (point, point)),
                    },
                    3 => (
                        Granularity::Line,
                        (
                            Point::new(point.line, 0),
                            Point::new(point.line, width(&text).saturating_sub(1)),
                        ),
                    ),
                    _ => (Granularity::Cell, (point, point)),
                };
                self.selection = Some(Selection::new(space, granularity, origin));
                self.pressing = true;
                None
            }
            MouseEventKind::Drag(MouseButton::Left) if self.pressing => {
                let space = self.selection.as_ref()?.space;
                let mut point = self.point(space, column, row);
                // Dragging past the conversation's edge scrolls it a row at a
                // time and keeps selecting.
                if let (Space::Conversation, Some(view)) = (space, self.view) {
                    if row < view.top && super::app::scroll_transcript(model, 1) {
                        point = Point::new(view.first.saturating_sub(1), 0);
                        self.view = Some(View {
                            first: view.first.saturating_sub(1),
                            ..view
                        });
                    } else if row >= view.top + view.height
                        && super::app::scroll_transcript(model, -1)
                    {
                        let line = view.first + usize::from(view.height);
                        point = Point::new(line, view.right.saturating_sub(1));
                        self.view = Some(View {
                            first: view.first + 1,
                            ..view
                        });
                    }
                }
                let texts: Vec<(usize, String)> = [point.line]
                    .into_iter()
                    .map(|line| (line, self.text_of(model, rendered, space, line)))
                    .collect();
                if let Some(selection) = self.selection.as_mut() {
                    selection.drag_to(point, |line| {
                        texts
                            .iter()
                            .find(|(at, _)| *at == line)
                            .map(|(_, text)| text.clone())
                            .unwrap_or_default()
                    });
                }
                self.clicks.reset_unless_still(column, row);
                None
            }
            MouseEventKind::Up(MouseButton::Left) if self.pressing => {
                self.pressing = false;
                if self.copy_on_select {
                    self.selected_text(model, rendered)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Feed one key while a selection exists. Copy keys copy, shifted
    /// motions extend, navigation keeps the selection, and anything else
    /// clears it and passes through.
    pub fn key(&mut self, model: &mut Model, rendered: &[String], key: &KeyEvent) -> KeyOutcome {
        if key.kind == KeyEventKind::Release || !self.has_selection() {
            return KeyOutcome::Pass;
        }
        let mods = key.modifiers;
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let shift = mods.contains(KeyModifiers::SHIFT);
        let is_c = matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'));
        if ctrl && is_c {
            let text = self.selected_text(model, rendered);
            if !shift {
                // `ctrl+c` copies rather than interrupts while text is
                // selected; the selection has done its job.
                self.clear();
            }
            return KeyOutcome::Taken(text);
        }
        let only_shift = (mods - KeyModifiers::SHIFT).is_empty() && shift;
        let step = match key.code {
            KeyCode::Left => Some((0, -1)),
            KeyCode::Right => Some((0, 1)),
            KeyCode::Up => Some((-1, 0)),
            KeyCode::Down => Some((1, 0)),
            _ => None,
        };
        if only_shift {
            let space = self.selection.as_ref().map(|s| s.space);
            if let (Some((lines, columns)), Some(space)) = (step, space) {
                self.extend(model, rendered, space, lines, columns);
                return KeyOutcome::Taken(None);
            }
            if let (KeyCode::Home | KeyCode::End, Some(space)) = (key.code, space) {
                let line = self.selection.as_ref().map_or(0, |s| s.focus.line);
                let text = self.text_of(model, rendered, space, line);
                if let Some(selection) = self.selection.as_mut() {
                    selection.focus.column = if key.code == KeyCode::Home {
                        0
                    } else {
                        width(&text).saturating_sub(1)
                    };
                }
                return KeyOutcome::Taken(None);
            }
        }
        let navigation = matches!(
            key.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
        );
        let keeps = matches!(
            key.code,
            KeyCode::Esc | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Modifier(_)
        ) || (navigation
            && mods.intersects(
                KeyModifiers::SHIFT
                    | KeyModifiers::ALT
                    | KeyModifiers::SUPER
                    | KeyModifiers::META
                    | KeyModifiers::CONTROL,
            ));
        if !keeps {
            self.clear();
        }
        KeyOutcome::Pass
    }

    fn extend(
        &mut self,
        model: &mut Model,
        rendered: &[String],
        space: Space,
        lines: isize,
        columns: isize,
    ) {
        // Reaching the top or bottom edge of the conversation scrolls it.
        if let (Space::Conversation, Some(view), Some(selection)) =
            (space, self.view, self.selection.as_ref())
        {
            let focus = selection.focus.line;
            if lines < 0 && focus <= view.first && super::app::scroll_transcript(model, 1) {
                self.view = Some(View {
                    first: view.first.saturating_sub(1),
                    ..view
                });
            } else if lines > 0
                && focus + 1 >= view.first + usize::from(view.height)
                && super::app::scroll_transcript(model, -1)
            {
                self.view = Some(View {
                    first: view.first + 1,
                    ..view
                });
            }
        }
        let focus = self.selection.as_ref().map_or(0, |s| s.focus.line);
        let texts: Vec<(usize, String)> = (focus.saturating_sub(1)..=focus + 1)
            .map(|line| (line, self.text_of(model, rendered, space, line)))
            .collect();
        if let Some(selection) = self.selection.as_mut() {
            selection.extend_by(lines, columns, |line| {
                texts
                    .iter()
                    .find(|(at, _)| *at == line)
                    .map(|(_, text)| text.clone())
                    .unwrap_or_default()
            });
        }
    }

    /// Screen cells to paint as selected: `(row, first column, end column
    /// exclusive)`, clipped to what is on screen.
    pub fn highlights(&self, rendered: &[String], screen_width: u16) -> Vec<(u16, u16, u16)> {
        let Some(selection) = self.selection.as_ref() else {
            return Vec::new();
        };
        let Some((start, end)) = selection.range() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in start.line..=end.line {
            let (row, right_edge) = match selection.space {
                Space::Screen => (Some(line as u16), screen_width),
                Space::Conversation => match self.view {
                    Some(view) => (view.row_of(line), view.right),
                    None => (None, 0),
                },
            };
            let Some(row) = row else { continue };
            let text_width = rendered
                .get(usize::from(row))
                .map_or(0, |text| width(text.trim_end()));
            let left = if line == start.line { start.column } else { 0 };
            let right = if line == end.line {
                end.column.saturating_add(1)
            } else {
                text_width.max(left + 1)
            };
            let right = right.min(right_edge);
            if right > left {
                out.push((row, left, right));
            }
        }
        out
    }
}

impl Clicks {
    /// Motion away from the press ends any multi-click in progress.
    fn reset_unless_still(&mut self, column: u16, row: u16) {
        if self.last.is_some_and(|(_, last_column, last_row)| {
            column.abs_diff(last_column) > MULTI_CLICK_SLOP
                || row.abs_diff(last_row) > MULTI_CLICK_SLOP
        }) {
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<'a>(lines: &'a [&'a str]) -> impl Fn(usize) -> String + 'a {
        move |line| lines.get(line).copied().unwrap_or_default().to_string()
    }

    #[test]
    fn double_click_takes_a_whole_path_and_a_whole_url() {
        let text = "see src/utils/foo.ts and https://example.com/a?b=1.";
        assert_eq!(word_bounds(text, 9), Some((4, 19)));
        assert_eq!(slice_columns(text, 4, 19), "src/utils/foo.ts");
        let (start, end) = word_bounds(text, 30).unwrap();
        assert_eq!(slice_columns(text, start, end), "https://example.com/a?b=1");
        assert_eq!(word_bounds(text, 3), None, "a space is not a word");
    }

    #[test]
    fn wide_characters_are_measured_in_cells() {
        let text = "日本 語";
        assert_eq!(word_bounds(text, 1), Some((0, 3)));
        assert_eq!(slice_columns(text, 0, 3), "日本");
        assert_eq!(width(text), 7);
    }

    #[test]
    fn presses_count_up_to_three_when_close_in_time_and_place() {
        let mut clicks = Clicks::default();
        let start = Instant::now();
        assert_eq!(clicks.press(start, 10, 5), 1);
        assert_eq!(clicks.press(start + Duration::from_millis(200), 11, 5), 2);
        assert_eq!(clicks.press(start + Duration::from_millis(400), 11, 5), 3);
        assert_eq!(
            clicks.press(start + Duration::from_millis(500), 11, 5),
            1,
            "wraps"
        );
        assert_eq!(
            clicks.press(start + Duration::from_millis(600), 20, 5),
            1,
            "too far"
        );
        assert_eq!(
            clicks.press(start + Duration::from_secs(2), 20, 5),
            1,
            "too late"
        );
    }

    #[test]
    fn a_plain_click_selects_nothing_until_it_moves() {
        let origin = Point::new(2, 4);
        let mut selection = Selection::new(Space::Screen, Granularity::Cell, (origin, origin));
        assert_eq!(selection.range(), None);
        selection.drag_to(Point::new(3, 1), rows(&[]));
        assert_eq!(selection.range(), Some((origin, Point::new(3, 1))));
        selection.drag_to(Point::new(1, 0), rows(&[]));
        assert_eq!(selection.range(), Some((Point::new(1, 0), origin)));
    }

    #[test]
    fn a_word_drag_extends_by_whole_words() {
        let lines = ["alpha beta gamma"];
        let origin = (Point::new(0, 6), Point::new(0, 9));
        let mut selection = Selection::new(Space::Screen, Granularity::Word, origin);
        selection.drag_to(Point::new(0, 12), rows(&lines));
        let (start, end) = selection.range().unwrap();
        assert_eq!(text_between(start, end, rows(&lines)), "beta gamma");
        selection.drag_to(Point::new(0, 2), rows(&lines));
        let (start, end) = selection.range().unwrap();
        assert_eq!(text_between(start, end, rows(&lines)), "alpha beta");
    }

    #[test]
    fn copied_text_drops_trailing_blanks_per_row() {
        let lines = ["one   ", "two", "three"];
        let text = text_between(Point::new(0, 0), Point::new(2, 2), rows(&lines));
        assert_eq!(text, "one\ntwo\nthr");
    }

    #[test]
    fn the_keyboard_extends_across_row_ends() {
        let lines = ["ab", "cd"];
        let origin = Point::new(0, 1);
        let mut selection = Selection::new(Space::Screen, Granularity::Cell, (origin, origin));
        selection.extend_by(0, 1, rows(&lines));
        selection.extend_by(0, 1, rows(&lines));
        assert_eq!(selection.focus, Point::new(1, 0));
        selection.extend_by(-1, 0, rows(&lines));
        assert_eq!(selection.focus, Point::new(0, 0));
        assert!(selection.range().is_some());
    }

    mod pointer {
        use super::super::*;
        use crate::davinci::app;
        use crate::davinci::model::Entry;
        use crate::davinci::theme::{ColorDepth, Theme};

        fn chat() -> Model {
            let mut m = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, false),
                100,
                16,
                false,
            );
            m.transcript = (0..40).map(|i| Entry::user(&format!("turn {i}"))).collect();
            m
        }

        /// The frame's rows as the runtime keeps them, and the view it
        /// hands the selector.
        fn frame(m: &Model, selector: &mut Selector) -> Vec<String> {
            let composed = app::compose_frame(m, m.height);
            let mut rows: Vec<String> = composed.lines.iter().map(ToString::to_string).collect();
            if let Some(bar) = &composed.scrollbar {
                crate::davinci::runtime::strip_scrollbar(&mut rows, bar);
            }
            selector.view = composed.scrollbar.as_ref().map(View::from_scrollbar);
            rows
        }

        fn at(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }
        }

        fn row_of(rows: &[String], needle: &str) -> (u16, u16) {
            let row = rows
                .iter()
                .position(|row| row.ends_with(needle) || row.contains(&format!("{needle} ")))
                .unwrap();
            let column = width(&rows[row][..rows[row].find(needle).unwrap()]);
            (column, row as u16)
        }

        fn drag_over(
            m: &mut Model,
            selector: &mut Selector,
            rows: &[String],
            needle: &str,
        ) -> Option<String> {
            let (column, row) = row_of(rows, needle);
            let now = Instant::now();
            let end = column + width(needle) - 1;
            selector.mouse(
                m,
                rows,
                at(MouseEventKind::Down(MouseButton::Left), column, row),
                now,
            );
            selector.mouse(
                m,
                rows,
                at(MouseEventKind::Drag(MouseButton::Left), end, row),
                now,
            );
            selector.mouse(
                m,
                rows,
                at(MouseEventKind::Up(MouseButton::Left), end, row),
                now,
            )
        }

        #[test]
        fn release_copies_and_scrolling_keeps_the_same_text_selected() {
            let mut m = chat();
            let mut selector = Selector::default();
            let rows = frame(&m, &mut selector);
            assert!(selector.view.is_some(), "the conversation scrolls");
            let copied = drag_over(&mut m, &mut selector, &rows, "turn 35");
            assert_eq!(copied.as_deref(), Some("turn 35"));
            let before = selector.highlights(&rows, m.width);
            assert_eq!(before.len(), 1);

            assert!(app::scroll_transcript(&mut m, 3));
            let rows = frame(&m, &mut selector);
            let after = selector.highlights(&rows, m.width);
            assert_eq!(after.len(), 1);
            assert!(
                after[0].0 > before[0].0,
                "the highlight moved with its text"
            );
            assert_eq!(
                selector.selected_text(&m, &rows).as_deref(),
                Some("turn 35")
            );
        }

        #[test]
        fn a_selection_scrolled_out_of_view_still_copies_its_text() {
            let mut m = chat();
            let mut selector = Selector::default();
            let rows = frame(&m, &mut selector);
            drag_over(&mut m, &mut selector, &rows, "turn 39");
            assert!(app::scroll_transcript(&mut m, 30));
            let rows = frame(&m, &mut selector);
            assert!(selector.highlights(&rows, m.width).is_empty());
            assert_eq!(
                selector.selected_text(&m, &rows).as_deref(),
                Some("turn 39")
            );
        }

        #[test]
        fn double_click_selects_a_word_and_triple_click_the_row() {
            let mut m = chat();
            let mut selector = Selector::default();
            let rows = frame(&m, &mut selector);
            let (column, row) = row_of(&rows, "turn 37");
            let now = Instant::now();
            let press = at(MouseEventKind::Down(MouseButton::Left), column + 1, row);
            let release = at(MouseEventKind::Up(MouseButton::Left), column + 1, row);
            selector.mouse(&mut m, &rows, press, now);
            assert_eq!(
                selector.mouse(&mut m, &rows, release, now),
                None,
                "a click copies nothing"
            );
            selector.mouse(&mut m, &rows, press, now + Duration::from_millis(100));
            let word = selector.mouse(&mut m, &rows, release, now + Duration::from_millis(100));
            assert_eq!(word.as_deref(), Some("turn"));
            selector.mouse(&mut m, &rows, press, now + Duration::from_millis(200));
            let line = selector.mouse(&mut m, &rows, release, now + Duration::from_millis(200));
            assert!(
                line.as_deref()
                    .is_some_and(|line| line.ends_with("turn 37")),
                "{line:?}"
            );
        }

        #[test]
        fn copy_keys_copy_navigation_keeps_and_typing_clears() {
            let mut m = chat();
            let mut selector = Selector {
                copy_on_select: false,
                ..Selector::default()
            };
            let rows = frame(&m, &mut selector);
            assert_eq!(drag_over(&mut m, &mut selector, &rows, "turn 36"), None);
            let key = |code, modifiers| KeyEvent::new(code, modifiers);

            let esc = key(KeyCode::Esc, KeyModifiers::NONE);
            assert_eq!(selector.key(&mut m, &rows, &esc), KeyOutcome::Pass);
            assert!(selector.has_selection(), "esc keeps the selection");

            let shift_left = key(KeyCode::Left, KeyModifiers::SHIFT);
            assert_eq!(
                selector.key(&mut m, &rows, &shift_left),
                KeyOutcome::Taken(None)
            );
            assert_eq!(selector.selected_text(&m, &rows).as_deref(), Some("turn 3"));

            let copy = key(
                KeyCode::Char('C'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            );
            assert_eq!(
                selector.key(&mut m, &rows, &copy),
                KeyOutcome::Taken(Some("turn 3".into()))
            );
            assert!(selector.has_selection(), "ctrl+shift+c keeps it");

            let ctrl_c = key(KeyCode::Char('c'), KeyModifiers::CONTROL);
            assert_eq!(
                selector.key(&mut m, &rows, &ctrl_c),
                KeyOutcome::Taken(Some("turn 3".into()))
            );
            assert!(
                !selector.has_selection(),
                "ctrl+c copies instead of interrupting"
            );

            drag_over(&mut m, &mut selector, &rows, "turn 36");
            let typed = key(KeyCode::Char('x'), KeyModifiers::NONE);
            assert_eq!(selector.key(&mut m, &rows, &typed), KeyOutcome::Pass);
            assert!(!selector.has_selection(), "typing clears it");
        }

        #[test]
        fn a_selection_outside_the_conversation_is_held_in_screen_rows() {
            let mut m = chat();
            let mut selector = Selector::default();
            let rows = frame(&m, &mut selector);
            let last = rows.len() as u16 - 1;
            let now = Instant::now();
            selector.mouse(
                &mut m,
                &rows,
                at(MouseEventKind::Down(MouseButton::Left), 2, last),
                now,
            );
            selector.mouse(
                &mut m,
                &rows,
                at(MouseEventKind::Drag(MouseButton::Left), 8, last),
                now,
            );
            assert_eq!(
                selector.selection.as_ref().map(|s| s.space),
                Some(Space::Screen)
            );
            assert_eq!(selector.highlights(&rows, m.width), vec![(last, 2, 9)]);
        }
    }
}
