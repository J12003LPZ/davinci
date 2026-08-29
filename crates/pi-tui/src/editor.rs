use crate::component::{wrap, Component};
use crate::keys::Key;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub buffer: String,
    pub cursor: usize,
    pub history: Vec<String>,
}

impl Editor {
    pub fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn handle_key(&mut self, key: &Key) {
        match key {
            Key::Char(c) => self.insert(*c),
            Key::Backspace => {
                if self.cursor > 0 {
                    let prev = self.buffer[..self.cursor]
                        .chars()
                        .next_back()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                    self.buffer.drain(self.cursor..self.cursor + prev);
                }
            }
            Key::Enter => {
                self.history.push(self.buffer.clone());
            }
            Key::Left => {
                if self.cursor > 0 {
                    let prev = self.buffer[..self.cursor]
                        .chars()
                        .next_back()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                }
            }
            Key::Right => {
                if let Some(c) = self.buffer[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            _ => {}
        }
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        wrap(&self.buffer, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_insert_and_backspace() {
        let mut editor = Editor::default();
        editor.handle_key(&Key::Char('a'));
        editor.handle_key(&Key::Char('b'));
        editor.handle_key(&Key::Backspace);
        assert_eq!(editor.buffer, "a");
    }
}
