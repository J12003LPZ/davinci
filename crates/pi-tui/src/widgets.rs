//! Remaining TypeScript TUI primitives: box, stacks, scroll, input, settings.

use crate::component::{wrap, Component};
use crate::keys::Key;

/// TypeScript `CURSOR_MARKER` from `packages/tui/src/tui.ts` — IME cursor position.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

#[derive(Debug, Clone)]
pub struct BoxWidget {
    pub title: Option<String>,
    pub body: Vec<String>,
}

impl Component for BoxWidget {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2).max(1);
        let mut lines = vec![format!("┌{}┐", "─".repeat(inner))];
        if let Some(title) = &self.title {
            for line in wrap(title, inner) {
                lines.push(format!("│{line:<inner$}│"));
            }
        }
        for line in &self.body {
            for wrapped in wrap(line, inner) {
                lines.push(format!("│{wrapped:<inner$}│"));
            }
        }
        lines.push(format!("└{}┘", "─".repeat(inner)));
        lines
    }
}

#[derive(Debug, Clone, Default)]
pub struct VStack {
    pub children: Vec<Vec<String>>,
}

impl Component for VStack {
    fn render(&self, _width: usize) -> Vec<String> {
        self.children.iter().flatten().cloned().collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HStack {
    pub children: Vec<String>,
    pub gap: usize,
}

impl Component for HStack {
    fn render(&self, width: usize) -> Vec<String> {
        let gap = " ".repeat(self.gap);
        let line = self.children.join(&gap);
        wrap(&line, width)
    }
}

#[derive(Debug, Clone)]
pub struct ScrollView {
    pub lines: Vec<String>,
    pub offset: usize,
    pub height: usize,
}

impl Component for ScrollView {
    fn render(&self, width: usize) -> Vec<String> {
        self.lines
            .iter()
            .skip(self.offset)
            .take(self.height)
            .flat_map(|line| wrap(line, width))
            .take(self.height)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    Continue,
    Submit,
    Escape,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub value: String,
    pub placeholder: String,
    pub cursor: usize,
    pub focused: bool,
}

impl Input {
    pub fn get_value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.len());
    }

    pub fn handle_key(&mut self, key: &Key) -> InputAction {
        match key {
            Key::Enter => InputAction::Submit,
            Key::Escape => InputAction::Escape,
            Key::Char(ch) if !ch.is_control() => {
                self.insert_str(&ch.to_string());
                InputAction::Continue
            }
            Key::Backspace => {
                self.delete_backward();
                InputAction::Continue
            }
            Key::Left => {
                self.move_left();
                InputAction::Continue
            }
            Key::Right => {
                self.move_right();
                InputAction::Continue
            }
            Key::Ctrl('a') => {
                self.cursor = 0;
                InputAction::Continue
            }
            Key::Ctrl('e') => {
                self.cursor = self.value.len();
                InputAction::Continue
            }
            Key::Ctrl('u') => {
                self.value.replace_range(..self.cursor, "");
                self.cursor = 0;
                InputAction::Continue
            }
            Key::Ctrl('k') => {
                self.value.truncate(self.cursor);
                InputAction::Continue
            }
            Key::Ctrl('w') => {
                self.delete_word_backward();
                InputAction::Continue
            }
            _ => InputAction::Continue,
        }
    }

    pub fn handle_input(&mut self, data: &str) -> InputAction {
        if data == "\x1b" || data == "escape" {
            return InputAction::Escape;
        }
        if data == "\r" || data == "\n" || data == "enter" {
            return InputAction::Submit;
        }
        self.handle_key(&crate::keys::parse_key(data))
    }

    fn insert_str(&mut self, text: &str) {
        self.value.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.value[..self.cursor]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor -= prev;
        self.value.drain(self.cursor..self.cursor + prev);
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.value[..self.cursor]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor -= prev;
    }

    fn move_right(&mut self) {
        if let Some(ch) = self.value[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.value[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace());
        let word_end = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| {
                i + trimmed[i..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        self.value.replace_range(word_end..self.cursor, "");
        self.cursor = word_end;
    }
}

impl Component for Input {
    fn render(&self, width: usize) -> Vec<String> {
        let text = if self.value.is_empty() && !self.focused {
            self.placeholder.as_str()
        } else {
            self.value.as_str()
        };
        if !self.focused {
            return wrap(text, width);
        }
        let cursor = self.cursor.min(text.len());
        let before = &text[..cursor];
        let rest = &text[cursor..];
        let at = rest.chars().next();
        let at_len = at.map(|c| c.len_utf8()).unwrap_or(0);
        let at_text = at.map(|c| c.to_string()).unwrap_or_else(|| " ".into());
        let after = if at_len == 0 { "" } else { &rest[at_len..] };
        let line = format!("{before}{CURSOR_MARKER}\x1b[7m{at_text}\x1b[27m{after}");
        wrap(&line, width.max(line.len()))
    }
}

#[derive(Debug, Clone)]
pub struct SettingsList {
    pub items: Vec<(String, bool)>,
    pub selected: usize,
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, (name, on))| {
                let mark = if *on { "[x]" } else { "[ ]" };
                let prefix = if i == self.selected { ">" } else { " " };
                let mut line = format!("{prefix} {mark} {name}");
                if line.len() > width {
                    line.truncate(width);
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_render() {
        let boxw = BoxWidget {
            title: Some("settings".into()),
            body: vec!["theme".into()],
        };
        assert!(boxw.render(20).iter().any(|l| l.contains("settings")));
        let stack = VStack {
            children: vec![vec!["a".into()], vec!["b".into()]],
        };
        assert_eq!(stack.render(10), vec!["a", "b"]);
        let scroll = ScrollView {
            lines: vec!["1".into(), "2".into(), "3".into()],
            offset: 1,
            height: 1,
        };
        assert_eq!(scroll.render(10), vec!["2"]);
        let mut input = Input {
            focused: true,
            ..Input::default()
        };
        assert_eq!(input.handle_input("h"), InputAction::Continue);
        assert_eq!(input.handle_input("i"), InputAction::Continue);
        assert_eq!(input.get_value(), "hi");
        assert_eq!(input.handle_key(&Key::Enter), InputAction::Submit);
        assert_eq!(input.handle_key(&Key::Escape), InputAction::Escape);
        let rendered = input.render(20);
        assert!(rendered.iter().any(|line| line.contains(CURSOR_MARKER)));
        assert!(rendered.iter().any(|line| line.contains("\x1b[7m")));
    }
}
