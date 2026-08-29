use crate::kill_ring::KillRing;
use crate::render::{visible_width, Component};
use crate::CURSOR_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Kill,
    Yank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpMode {
    Forward,
    Backward,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub buffer: String,
    pub cursor: usize,
    pub history: Vec<String>,
    kill_ring: KillRing,
    last_action: Option<LastAction>,
    jump_mode: Option<JumpMode>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            kill_ring: KillRing::new(),
            last_action: None,
            jump_mode: None,
        }
    }

    pub fn jump_mode(&self) -> Option<bool> {
        match self.jump_mode {
            Some(JumpMode::Forward) => Some(true),
            Some(JumpMode::Backward) => Some(false),
            None => None,
        }
    }

    pub fn cancel_jump(&mut self) {
        self.jump_mode = None;
    }

    fn clear_last_action(&mut self) {
        self.last_action = None;
    }

    pub fn insert_str(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.clear_last_action();
    }

    pub fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.clear_last_action();
    }

    pub fn backspace(&mut self) {
        self.clear_last_action();
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
        self.clear_last_action();
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
        self.clear_last_action();
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
        self.clear_last_action();
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
        self.clear_last_action();
        let line_start = self.buffer[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.cursor = line_start;
    }

    pub fn move_line_end(&mut self) {
        self.clear_last_action();
        let line_end = self.buffer[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.buffer.len());
        self.cursor = line_end;
    }

    pub fn move_word_backwards(&mut self) {
        self.clear_last_action();
        self.cursor = crate::word_nav::find_word_backward_default(&self.buffer, self.cursor);
    }

    pub fn move_word_forwards(&mut self) {
        self.clear_last_action();
        self.cursor = crate::word_nav::find_word_forward_default(&self.buffer, self.cursor);
    }

    pub fn delete_word_backwards(&mut self) {
        let start = crate::word_nav::find_word_backward_default(&self.buffer, self.cursor);
        let deleted = self.buffer[start..self.cursor].to_string();
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
        self.kill_ring
            .push(&deleted, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn delete_word_forwards(&mut self) {
        let end = crate::word_nav::find_word_forward_default(&self.buffer, self.cursor);
        let deleted = self.buffer[self.cursor..end].to_string();
        self.buffer.drain(self.cursor..end);
        self.kill_ring
            .push(&deleted, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn delete_to_line_start(&mut self) {
        let start = self.buffer[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let deleted = self.buffer[start..self.cursor].to_string();
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
        self.kill_ring
            .push(&deleted, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn delete_to_line_end(&mut self) {
        let end = self.buffer[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.buffer.len());
        let deleted = self.buffer[self.cursor..end].to_string();
        self.buffer.drain(self.cursor..end);
        self.kill_ring
            .push(&deleted, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(str::to_string) else {
            return;
        };
        self.buffer.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    pub fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        let prev = self.kill_ring.peek().unwrap_or("").to_string();
        let start = self.cursor.saturating_sub(prev.len());
        if self.buffer.get(start..self.cursor) == Some(prev.as_str()) {
            self.buffer.drain(start..self.cursor);
            self.cursor = start;
        }
        self.kill_ring.rotate();
        self.yank();
    }

    pub fn begin_jump_forward(&mut self) {
        self.jump_mode = Some(JumpMode::Forward);
        self.last_action = None;
    }

    pub fn begin_jump_backward(&mut self) {
        self.jump_mode = Some(JumpMode::Backward);
        self.last_action = None;
    }

    pub fn jump_to_char(&mut self, ch: char, forward: bool) {
        self.last_action = None;
        self.jump_mode = None;
        if forward {
            let start = if self.cursor < self.buffer.len() {
                self.cursor
                    + self.buffer[self.cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0)
            } else {
                self.cursor
            };
            if start <= self.buffer.len() {
                if let Some(index) = self.buffer[start..].find(ch) {
                    self.cursor = start + index;
                }
            }
        } else if self.cursor > 0 {
            if let Some(index) = self.buffer[..self.cursor].rfind(ch) {
                self.cursor = index;
            }
        }
    }

    pub fn take_jump_mode(&mut self) -> Option<bool> {
        match self.jump_mode.take() {
            Some(JumpMode::Forward) => Some(true),
            Some(JumpMode::Backward) => Some(false),
            None => None,
        }
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

    #[test]
    fn kill_ring_yank_and_jump_match_ts() {
        let mut editor = Editor::new();
        editor.set_text("hello world");
        editor.cursor = 11;
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "hello ");
        editor.yank();
        assert_eq!(editor.buffer, "hello world");
        editor.set_text("one two three");
        editor.cursor = editor.buffer.len();
        editor.delete_word_backwards();
        editor.move_line_start();
        editor.delete_word_forwards();
        assert_eq!(editor.buffer, " two ");
        editor.yank();
        assert_eq!(editor.buffer, "one two ");
        editor.yank_pop();
        assert_eq!(editor.buffer, "three two ");
        editor.set_text("jump-to-char");
        editor.cursor = 0;
        editor.jump_to_char('t', true);
        assert_eq!(editor.cursor, 5);
        editor.jump_to_char('j', false);
        assert_eq!(editor.cursor, 0);
        editor.begin_jump_forward();
        assert_eq!(editor.jump_mode(), Some(true));
        editor.cancel_jump();
        assert_eq!(editor.jump_mode(), None);
    }
}
