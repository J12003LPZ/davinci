use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;

use unicode_width::UnicodeWidthStr;

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Dense or sparse line buffer matching JS `string[]` holes used by huge transcripts.
#[derive(Clone)]
pub struct RenderedLines {
    inner: Rc<RenderedLinesInner>,
}

enum RenderedLinesInner {
    Dense(Vec<String>),
    Sparse {
        count: usize,
        lines: BTreeMap<usize, String>,
    },
}

impl RenderedLines {
    pub fn dense(lines: Vec<String>) -> Self {
        Self {
            inner: Rc::new(RenderedLinesInner::Dense(lines)),
        }
    }

    pub fn sparse(count: usize, lines: BTreeMap<usize, String>) -> Self {
        Self {
            inner: Rc::new(RenderedLinesInner::Sparse { count, lines }),
        }
    }

    pub fn empty() -> Self {
        Self::dense(Vec::new())
    }

    pub fn len(&self) -> usize {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines.len(),
            RenderedLinesInner::Sparse { count, .. } => *count,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines.get(index).map(String::as_str),
            RenderedLinesInner::Sparse { lines, .. } => lines.get(&index).map(String::as_str),
        }
    }

    pub fn defined(&self) -> Vec<(usize, &str)> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines
                .iter()
                .enumerate()
                .map(|(index, line)| (index, line.as_str()))
                .collect(),
            RenderedLinesInner::Sparse { lines, .. } => lines
                .iter()
                .map(|(index, line)| (*index, line.as_str()))
                .collect(),
        }
    }

    pub fn max_visible_width(&self) -> usize {
        self.defined()
            .into_iter()
            .map(|(_, line)| visible_width(line))
            .max()
            .unwrap_or(0)
    }

    pub fn find_containing(&self, needle: &str) -> Option<usize> {
        self.defined()
            .into_iter()
            .find(|(_, line)| line.contains(needle))
            .map(|(index, _)| index)
    }

    pub fn to_dense_vec(&self) -> Option<Vec<String>> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => Some(lines.clone()),
            RenderedLinesInner::Sparse { .. } => None,
        }
    }
}

impl Default for RenderedLines {
    fn default() -> Self {
        Self::dense(Vec::new())
    }
}

pub trait Component: AsAny {
    fn render(&self, width: usize) -> Vec<String>;
    fn rendered_lines(&self, width: usize) -> RenderedLines {
        RenderedLines::dense(self.render(width))
    }
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self);
    fn wants_key_release(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Text {
    pub value: String,
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        wrap_text(&self.value, width)
    }

    fn invalidate(&mut self) {}
}

/// Non-allocating huge transcript source (JS sparse `string[]`).
pub struct SparseLines {
    pub count: usize,
    pub lines: BTreeMap<usize, String>,
}

impl SparseLines {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            lines: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, index: usize, line: impl Into<String>) {
        self.lines.insert(index, line.into());
    }
}

impl Component for SparseLines {
    fn render(&self, _width: usize) -> Vec<String> {
        Vec::new()
    }

    fn rendered_lines(&self, _width: usize) -> RenderedLines {
        RenderedLines::sparse(self.count, self.lines.clone())
    }

    fn invalidate(&mut self) {}
}

pub fn visible_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// TS `visibleWidth`: strip ANSI/OSC/APC then measure columns.
pub fn visible_width_stripped(text: &str) -> usize {
    // Most lines carry no escape sequences; skip the stripped copy for them.
    if !text.contains('\x1b') {
        return visible_width(text);
    }
    visible_width(&strip_terminal_sequences(text))
}

pub fn strip_terminal_sequences(text: &str) -> String {
    if !text.contains('\x1b') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
                continue;
            }
            if i + 1 < bytes.len()
                && (bytes[i + 1] == b']' || bytes[i + 1] == b'_' || bytes[i + 1] == b'^')
            {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        // Running accumulator: recomputing `visible_width(&current)` per word
        // made wrapping quadratic in line length.
        let mut current_width = 0usize;
        for word in raw.split(' ') {
            let word_width = visible_width(word);
            if current.is_empty() {
                current = word.to_string();
                current_width = word_width;
                continue;
            }
            if current_width + 1 + word_width <= width {
                current.push(' ');
                current.push_str(word);
                current_width += 1 + word_width;
            } else {
                lines.push(current);
                current = word.to_string();
                current_width = word_width;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

/// Shared terminal-native section formatting for the reusable ANSI components.
/// Uses their existing theme (or inherits the caller's ink), and the same focus
/// gutter as the native renderer. Input/state remain owned by each component.
pub(crate) struct CommandSection<'a> {
    width: usize,
    theme: Option<&'a crate::themes::Theme>,
    rows: Vec<String>,
}

pub(crate) fn selection_window(
    selected: usize,
    count: usize,
    limit: usize,
) -> std::ops::Range<usize> {
    let limit = limit.max(1);
    let selected = selected.min(count.saturating_sub(1));
    let start = selected
        .saturating_sub(limit / 2)
        .min(count.saturating_sub(limit));
    start..start.saturating_add(limit).min(count)
}

impl<'a> CommandSection<'a> {
    pub(crate) fn new(width: usize, title: &str, theme: Option<&'a crate::themes::Theme>) -> Self {
        let mut section = Self {
            width,
            theme,
            rows: Vec::new(),
        };
        section.heading(title);
        section
    }
    fn paint(&self, role: &str, text: &str) -> String {
        self.theme
            .map(|theme| theme.fg(role, text))
            .unwrap_or_else(|| text.to_string())
    }
    pub(crate) fn heading(&mut self, text: &str) {
        let text = self
            .theme
            .map(|theme| theme.bold(text))
            .unwrap_or_else(|| text.to_string());
        self.wrapped(&self.paint("text", &text), 2);
    }
    pub(crate) fn detail(&mut self, text: &str) {
        self.message("muted", text);
    }
    pub(crate) fn message(&mut self, role: &str, text: &str) {
        if !text.is_empty() {
            self.wrapped(&self.paint(role, text), 3);
        }
    }
    fn wrapped(&mut self, text: &str, indent: usize) {
        if self.width == 0 {
            return;
        }
        let indent = indent.min(self.width.saturating_sub(1));
        let room = self.width - indent;
        for line in crate::ansi::wrap_text_with_ansi(text, room) {
            self.rows.push(crate::ansi::truncate_to_width(
                &format!("{}{line}", " ".repeat(indent)),
                self.width,
                "",
                false,
            ));
        }
    }
    /// Read-only query preview; the component still handles query editing.
    pub(crate) fn search(&mut self, query: &str) {
        let room = self.width.saturating_sub(11);
        let cells = crate::ansi::visible_width(query);
        let query = if cells > room {
            format!(
                "…{}",
                crate::ansi::slice_by_column(
                    query,
                    cells.saturating_sub(room.saturating_sub(1)),
                    room.saturating_sub(1),
                    true
                )
            )
        } else {
            query.to_string()
        };
        self.detail(&format!("Search: {query}"));
    }
    pub(crate) fn input(&mut self, rows: Vec<String>) {
        self.rows.extend(
            rows.into_iter()
                .map(|row| crate::ansi::truncate_to_width(&row, self.width, "", false)),
        );
    }
    pub(crate) fn item(&mut self, selected: bool, label: &str, value: &str) {
        use crate::ansi::{strip_terminal_sequences, truncate_to_width, visible_width};
        let prefix = if selected {
            crate::davinci::ui::SELECTION_BAR
        } else {
            "   "
        };
        let prefix_width = visible_width(prefix);
        let available = self.width.saturating_sub(prefix_width);
        let value_width = if self.width < 32 {
            0
        } else {
            visible_width(value).min(available / 3).min(28)
        };
        let gap = usize::from(value_width > 0) * 2;
        let name_width = available.saturating_sub(value_width + gap);
        let name = truncate_to_width(label, name_width, "…", false);
        let mut row = format!(
            "{}{}",
            self.paint(if selected { "accent" } else { "muted" }, prefix),
            self.paint(if selected { "accent" } else { "text" }, &name)
        );
        if value_width > 0 {
            row.push_str(&" ".repeat(available.saturating_sub(visible_width(&name) + value_width)));
            row.push_str(&self.paint("muted", &truncate_to_width(value, value_width, "…", false)));
        }
        self.rows
            .push(truncate_to_width(&row, self.width, "", false));
        if selected {
            if strip_terminal_sequences(&name) != strip_terminal_sequences(label) {
                self.detail(label);
            }
            if !value.is_empty() && (value_width == 0 || visible_width(value) > value_width) {
                self.detail(value);
            }
        }
    }
    pub(crate) fn position(&mut self, selected: usize, count: usize, limit: usize) {
        if count > limit.max(1) {
            self.detail(&format!(
                "{} of {count}",
                selected.saturating_add(1).min(count)
            ));
        }
    }
    pub(crate) fn hint(&mut self, text: &str) {
        self.message("dim", text);
    }
    pub(crate) fn finish(self) -> Vec<String> {
        self.rows
    }
}

#[cfg(test)]
mod command_component_regressions {
    use super::*;
    use crate::{
        ansi::{strip_terminal_sequences, visible_width},
        config_selector::{ConfigResource, ConfigResourceKind, ConfigScope, ConfigSelector},
        oauth_selector::{AuthSelectorMode, OAuthSelector},
        settings::{SettingItem, SettingsList},
        thinking_selector::ThinkingSelector,
    };
    #[test]
    fn unicode_resource_names_cannot_panic_or_overflow_a_narrow_view() {
        let selector = ConfigSelector::new(vec![ConfigResource {
            kind: ConfigResourceKind::Skills,
            name: "你好吗 café 🦀".into(),
            source: "项目/source".into(),
            enabled: true,
            scope: ConfigScope::User,
        }]);
        for width in [0, 1, 7, 20, 32, 40] {
            for row in selector.render(width) {
                assert!(visible_width(&row) <= width, "{width}: {row}");
            }
        }
    }
    #[test]
    fn settings_window_follows_focus_beyond_the_first_page() {
        let mut list = SettingsList::new(
            (0..40)
                .map(|i| SettingItem {
                    id: i.to_string(),
                    label: format!("setting-{i}"),
                    current_value: "on".into(),
                    values: vec!["on".into(), "off".into()],
                    description: Some("Help for this setting".into()),
                })
                .collect(),
            8,
        );
        list.selected = 30;
        let drawn = strip_terminal_sequences(&list.render(40).join("\n"));
        assert!(drawn.contains("setting-30"), "{drawn}");
        assert!(drawn.contains("Help for this setting"));
    }
    #[test]
    fn empty_auth_lists_keep_a_visible_cancel_hint() {
        let selector = OAuthSelector::new(AuthSelectorMode::Logout, Vec::new(), None);
        let drawn = strip_terminal_sequences(&selector.render(40).join("\n"));
        assert!(drawn.to_lowercase().contains("esc"), "{drawn}");
    }
    #[test]
    fn thinking_names_survive_ansi_styling_and_narrow_widths() {
        let selector = ThinkingSelector::new("low", vec!["low".into(), "high".into()], "high");
        let drawn = strip_terminal_sequences(&selector.render(20).join("\n"));
        assert!(drawn.contains("low"), "{drawn}");
        for width in [0, 1, 20, 32, 40] {
            for row in selector.render(width) {
                assert!(visible_width(&row) <= width, "{width}: {row}");
            }
        }
    }
}
