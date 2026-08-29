//! Terminal UI matching `@earendil-works/pi-tui`.

mod box_comp;
mod chrome;
mod editor;
mod fuzzy;
mod keys;
mod markdown;
mod mouse;
mod render;
mod scroll;
mod themes;
mod transcript;

pub use box_comp::TuiBox;
pub use chrome::ChatChrome;
pub use editor::Editor;
pub use fuzzy::{fuzzy_filter, fuzzy_match, FuzzyMatch};
pub use keys::{parse_key, Key};
pub use markdown::render_markdown;
pub use mouse::{parse_mouse_sgr, MouseButton, MouseEvent, MouseKind, MOUSE_DISABLE, MOUSE_ENABLE};
pub use render::{visible_width, Component, Text};
pub use scroll::ScrollView;
pub use themes::{builtin_themes, Theme};
pub use transcript::{Transcript, TranscriptLine};

pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";
pub const ALT_BUFFER_ENTER: &str = "\x1b[?1049h";
pub const ALT_BUFFER_LEAVE: &str = "\x1b[?1049l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiMode {
    Regular,
    Fullscreen,
}

impl TuiMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "regular" => Some(Self::Regular),
            "fullscreen" => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keybinding {
    pub action: String,
    pub keys: Vec<String>,
}

pub const TUI_KEYBINDINGS: &[(&str, &str)] = &[
    ("submit", "enter"),
    ("newline", "shift+enter"),
    ("abort", "escape"),
    ("quit", "ctrl+c"),
    ("cycle-model", "ctrl+p"),
    ("cycle-thinking", "ctrl+t"),
    ("clear", "ctrl+l"),
];

pub fn get_keybindings() -> Vec<Keybinding> {
    TUI_KEYBINDINGS
        .iter()
        .map(|(action, key)| Keybinding {
            action: (*action).to_string(),
            keys: vec![(*key).to_string()],
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct SelectList {
    pub items: Vec<String>,
    pub selected: usize,
    pub query: String,
}

impl SelectList {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
        }
    }
}

impl Component for SelectList {
    fn render(&self, width: usize) -> Vec<String> {
        let filtered = fuzzy_filter(&self.query, &self.items);
        filtered
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == self.selected { "> " } else { "  " };
                truncate_line(&format!("{prefix}{item}"), width)
            })
            .collect()
    }

    fn invalidate(&mut self) {}
}

fn truncate_line(line: &str, width: usize) -> String {
    if visible_width(line) <= width {
        line.to_string()
    } else {
        let mut out = String::new();
        for ch in line.chars() {
            if visible_width(&out) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
                > width.saturating_sub(1)
            {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_render_width_and_fuzzy() {
        let list = SelectList::new(vec!["openai/gpt-4".into(), "anthropic/sonnet".into()]);
        let lines = list.render(10);
        assert!(lines.iter().all(|line| visible_width(line) <= 10));
        assert!(fuzzy_match("gpt", "openai/gpt-4").matches);
        assert!(!fuzzy_match("zzz", "openai/gpt-4").matches);
    }

    #[test]
    fn markdown_and_keybindings() {
        let lines = render_markdown("# Title\n\nHello **world**", 40);
        assert!(lines.iter().any(|line| line.contains("Title")));
        assert!(get_keybindings().iter().any(|b| b.action == "cycle-model"));
        let mut chrome = ChatChrome::new(builtin_themes()[0].clone(), "pi 0.84.4");
        chrome.transcript.push("user", "hi");
        chrome.transcript.push("assistant", "# hello");
        let rendered = chrome.render(40);
        assert!(rendered.iter().any(|line| line.contains("pi 0.84.4")));
        assert!(parse_mouse_sgr("\x1b[<0;2;2M").is_some());
    }
}
