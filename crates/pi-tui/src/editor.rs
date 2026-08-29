use crate::render::{visible_width, Component};
use crate::CURSOR_MARKER;

#[derive(Debug, Clone)]
pub struct Editor {
    pub buffer: String,
    pub cursor: usize,
    pub history: Vec<String>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
        }
    }

    pub fn insert_str(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.buffer[..self.cursor]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor -= prev;
        self.buffer.drain(self.cursor..self.cursor + prev);
    }

    pub fn submit(&mut self) -> String {
        let value = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        if !value.is_empty() {
            self.history.push(value.clone());
        }
        value
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        let mut line = format!("> {}", self.buffer);
        if visible_width(&line) > width.saturating_sub(1) {
            line.truncate(width.saturating_sub(1));
        }
        line.push_str(CURSOR_MARKER);
        vec![line]
    }

    fn handle_input(&mut self, data: &str) {
        for ch in data.chars() {
            if ch == '\u{8}' || ch == '\u{7f}' {
                self.backspace();
            } else if !ch.is_control() {
                self.insert(ch);
            }
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_history_and_render() {
        let mut editor = Editor::new();
        editor.handle_input("hi");
        assert_eq!(editor.submit(), "hi");
        assert_eq!(editor.history, ["hi"]);
        let line = editor.render(20);
        assert!(line[0].contains(CURSOR_MARKER));
    }
}
