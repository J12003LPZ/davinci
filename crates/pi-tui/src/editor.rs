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

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.buffer[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.buffer.drain(self.cursor..self.cursor + next);
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= self.buffer[..self.cursor]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        self.cursor += self.buffer[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
    }

    pub fn move_line_start(&mut self) {
        let line_start = self.buffer[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.cursor = line_start;
    }

    pub fn move_line_end(&mut self) {
        let line_end = self.buffer[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.buffer.len());
        self.cursor = line_end;
    }

    pub fn move_word_backwards(&mut self) {
        self.cursor = crate::word_nav::find_word_backward_default(&self.buffer, self.cursor);
    }

    pub fn move_word_forwards(&mut self) {
        self.cursor = crate::word_nav::find_word_forward_default(&self.buffer, self.cursor);
    }

    pub fn delete_word_backwards(&mut self) {
        let start = crate::word_nav::find_word_backward_default(&self.buffer, self.cursor);
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn delete_word_forwards(&mut self) {
        let end = crate::word_nav::find_word_forward_default(&self.buffer, self.cursor);
        self.buffer.drain(self.cursor..end);
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer = text.into();
        self.cursor = self.buffer.len();
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

    #[test]
    fn word_nav_and_delete_match_ts_boundaries() {
        let mut editor = Editor::new();
        editor.set_text("hello world");
        editor.move_word_backwards();
        assert_eq!(editor.cursor, 6);
        editor.delete_word_forwards();
        assert_eq!(editor.buffer, "hello ");
        editor.move_line_start();
        editor.delete_word_forwards();
        assert_eq!(editor.buffer, " ");
        editor.set_text("foo.bar");
        editor.cursor = 7;
        editor.move_word_backwards();
        assert_eq!(&editor.buffer[editor.cursor..], "bar");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "foobar");
        editor.move_left();
        editor.delete_forward();
        assert_eq!(editor.buffer, "fobar");
    }
}
