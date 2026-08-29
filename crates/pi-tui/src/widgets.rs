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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputLastAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub value: String,
    pub placeholder: String,
    pub cursor: usize,
    pub focused: bool,
    paste_buffer: String,
    is_in_paste: bool,
    kill_ring: Vec<String>,
    last_action: Option<InputLastAction>,
    undo_stack: Vec<(String, usize)>,
}

impl Input {
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            placeholder: placeholder.into(),
            ..Self::default()
        }
    }

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
                self.insert_character(&ch.to_string());
                InputAction::Continue
            }
            Key::Backspace => {
                self.delete_backward();
                InputAction::Continue
            }
            Key::Left => {
                self.last_action = None;
                self.move_left();
                InputAction::Continue
            }
            Key::Right => {
                self.last_action = None;
                self.move_right();
                InputAction::Continue
            }
            Key::Ctrl('a') => {
                self.last_action = None;
                self.cursor = 0;
                InputAction::Continue
            }
            Key::Ctrl('e') => {
                self.last_action = None;
                self.cursor = self.value.len();
                InputAction::Continue
            }
            Key::Ctrl('d') => {
                self.delete_forward();
                InputAction::Continue
            }
            Key::Ctrl('u') => {
                self.delete_to_line_start();
                InputAction::Continue
            }
            Key::Ctrl('k') => {
                self.delete_to_line_end();
                InputAction::Continue
            }
            Key::Ctrl('w') => {
                self.delete_word_backward();
                InputAction::Continue
            }
            Key::Ctrl('y') => {
                self.yank();
                InputAction::Continue
            }
            _ => InputAction::Continue,
        }
    }

    pub fn handle_input(&mut self, data: &str) -> InputAction {
        let mut data = data.to_string();
        if data.contains("\x1b[200~") {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            data = data.replace("\x1b[200~", "");
        }
        if self.is_in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end) = self.paste_buffer.find("\x1b[201~") {
                let pasted = self.paste_buffer[..end].to_string();
                let remaining = self.paste_buffer[end + 6..].to_string();
                self.is_in_paste = false;
                self.paste_buffer.clear();
                self.handle_paste(&pasted);
                if !remaining.is_empty() {
                    return self.handle_input(&remaining);
                }
            }
            return InputAction::Continue;
        }
        if data == "\x1b" || data == "escape" {
            return InputAction::Escape;
        }
        if data == "\r" || data == "\n" || data == "enter" {
            return InputAction::Submit;
        }
        if data == "\x19" {
            self.yank();
            return InputAction::Continue;
        }
        if data == "\x1by" {
            self.yank_pop();
            return InputAction::Continue;
        }
        if data == "\x1bd" {
            self.delete_word_forward();
            return InputAction::Continue;
        }
        if data == "\x1bb" || data == "\x1b[1;3D" || data == "\x1b[1;5D" {
            self.move_word_backward();
            return InputAction::Continue;
        }
        if data == "\x1bf" || data == "\x1b[1;3C" || data == "\x1b[1;5C" {
            self.move_word_forward();
            return InputAction::Continue;
        }
        if data == "\x1b[3~" {
            self.delete_forward();
            return InputAction::Continue;
        }
        if data == "\x1b[45;5u" {
            self.undo();
            return InputAction::Continue;
        }
        if is_printable_input(&data) {
            self.insert_character(&data);
            return InputAction::Continue;
        }
        self.handle_key(&crate::keys::parse_key(&data))
    }

    fn push_undo(&mut self) {
        self.undo_stack.push((self.value.clone(), self.cursor));
    }

    fn undo(&mut self) {
        if let Some((value, cursor)) = self.undo_stack.pop() {
            self.value = value;
            self.cursor = cursor;
            self.last_action = None;
        }
    }

    fn push_kill(&mut self, text: String, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate {
            if let Some(last) = self.kill_ring.last_mut() {
                if prepend {
                    *last = format!("{text}{last}");
                } else {
                    last.push_str(&text);
                }
                self.last_action = Some(InputLastAction::Kill);
                return;
            }
        }
        self.kill_ring.push(text);
        self.last_action = Some(InputLastAction::Kill);
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.last().cloned() else {
            return;
        };
        self.push_undo();
        self.insert_str(&text);
        self.last_action = Some(InputLastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(InputLastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        let prev = self.kill_ring.last().cloned().unwrap_or_default();
        if self.cursor >= prev.len() && self.value[self.cursor - prev.len()..self.cursor] == *prev {
            self.value
                .replace_range(self.cursor - prev.len()..self.cursor, "");
            self.cursor -= prev.len();
        }
        if self.kill_ring.len() > 1 {
            let last = self.kill_ring.pop().expect("len > 1");
            self.kill_ring.insert(0, last);
        }
        if let Some(text) = self.kill_ring.last().cloned() {
            self.insert_str(&text);
        }
        self.last_action = Some(InputLastAction::Yank);
    }

    fn insert_character(&mut self, text: &str) {
        let whitespace = text.chars().next().is_some_and(char::is_whitespace);
        if whitespace || self.last_action != Some(InputLastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(InputLastAction::TypeWord);
        self.insert_str(text);
    }

    fn insert_str(&mut self, text: &str) {
        self.value.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn handle_paste(&mut self, pasted: &str) {
        self.last_action = None;
        self.push_undo();
        let clean = pasted
            .replace("\r\n", "")
            .replace('\r', "")
            .replace('\n', "")
            .replace('\t', "    ");
        self.insert_str(&clean);
    }

    fn delete_backward(&mut self) {
        self.last_action = None;
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let prev = self.value[..self.cursor]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor -= prev;
        self.value.drain(self.cursor..self.cursor + prev);
    }

    fn delete_forward(&mut self) {
        self.last_action = None;
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let next = self.value[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.value.drain(self.cursor..self.cursor + next);
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let deleted = self.value[..self.cursor].to_string();
        let accumulate = self.last_action == Some(InputLastAction::Kill);
        self.push_kill(deleted, true, accumulate);
        self.value.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let deleted = self.value[self.cursor..].to_string();
        let accumulate = self.last_action == Some(InputLastAction::Kill);
        self.push_kill(deleted, false, accumulate);
        self.value.truncate(self.cursor);
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

    fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.last_action = None;
        self.cursor = crate::word_nav::find_word_backward(&self.value, self.cursor);
    }

    fn move_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.last_action = None;
        self.cursor = crate::word_nav::find_word_forward(&self.value, self.cursor);
    }

    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let was_kill = self.last_action == Some(InputLastAction::Kill);
        self.push_undo();
        let from = crate::word_nav::find_word_backward(&self.value, self.cursor);
        let deleted = self.value[from..self.cursor].to_string();
        self.push_kill(deleted, true, was_kill);
        self.value.replace_range(from..self.cursor, "");
        self.cursor = from;
    }

    fn delete_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let was_kill = self.last_action == Some(InputLastAction::Kill);
        self.push_undo();
        let to = crate::word_nav::find_word_forward(&self.value, self.cursor);
        let deleted = self.value[self.cursor..to].to_string();
        self.push_kill(deleted, false, was_kill);
        self.value.replace_range(self.cursor..to, "");
    }
}

fn is_printable_input(data: &str) -> bool {
    !data.is_empty()
        && !data.starts_with('\u{1b}')
        && data.chars().all(|ch| {
            let code = ch as u32;
            code >= 32 && code != 0x7f && !(0x80..=0x9f).contains(&code)
        })
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
        let mut paste = Input::default();
        assert_eq!(
            paste.handle_input("\x1b[200~beep boop\x1b[201~"),
            InputAction::Continue
        );
        assert_eq!(paste.get_value(), "beep boop");
        paste.handle_key(&Key::Ctrl('w'));
        assert_eq!(paste.get_value(), "beep ");
        paste.handle_key(&Key::Ctrl('y'));
        assert_eq!(paste.get_value(), "beep boop");
        paste.handle_input("\x1b[45;5u");
        assert_eq!(paste.get_value(), "beep ");
    }

    #[test]
    fn input_kill_ring_yank_pop_matches_typescript() {
        let mut input = Input::default();
        input.set_value("first");
        input.handle_input("\x05");
        input.handle_input("\x17");
        input.set_value("second");
        input.handle_input("\x05");
        input.handle_input("\x17");
        input.set_value("third");
        input.handle_input("\x05");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "");
        input.handle_input("\x19");
        assert_eq!(input.get_value(), "third");
        input.handle_input("\x1by");
        assert_eq!(input.get_value(), "second");
        input.handle_input("\x1by");
        assert_eq!(input.get_value(), "first");
        input.handle_input("\x1by");
        assert_eq!(input.get_value(), "third");

        let mut one = Input::default();
        one.set_value("only");
        one.handle_input("\x05");
        one.handle_input("\x17");
        one.handle_input("\x19");
        one.handle_input("\x1by");
        assert_eq!(one.get_value(), "only");

        let mut broken = Input::default();
        broken.set_value("test");
        broken.handle_input("\x05");
        broken.handle_input("\x17");
        broken.set_value("other");
        broken.handle_input("\x05");
        broken.handle_input("x");
        broken.handle_input("\x1by");
        assert_eq!(broken.get_value(), "otherx");
    }

    #[test]
    fn input_word_kill_and_punctuation_match_typescript() {
        let mut input = Input::default();
        input.set_value("foo.bar");
        input.handle_input("\x05");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "foo.");

        input.set_value("foo:bar");
        input.handle_input("\x05");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "foo:");

        input.set_value("你好世界。你好，世界");
        input.handle_input("\x05");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "你好世界。你好，");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "你好世界。你好");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "你好世界。");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "你好世界");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "你好");
        input.handle_input("\x17");
        assert_eq!(input.get_value(), "");

        let mut forward = Input::default();
        forward.set_value("hello world test");
        forward.handle_input("\x01");
        forward.handle_input("\x1bd");
        assert_eq!(forward.get_value(), " world test");
        forward.handle_input("\x1bd");
        assert_eq!(forward.get_value(), " test");
        forward.handle_input("\x19");
        assert_eq!(forward.get_value(), "hello world test");
    }

    #[test]
    fn input_undo_coalescing_and_paste_cleanup_match_typescript() {
        let mut input = Input::default();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            input.handle_input(ch);
        }
        assert_eq!(input.get_value(), "hello world");
        input.handle_input("\x1b[45;5u");
        assert_eq!(input.get_value(), "hello");
        input.handle_input("\x1b[45;5u");
        assert_eq!(input.get_value(), "");

        let mut spaces = Input::default();
        for ch in ["h", "e", "l", "l", "o", " ", " "] {
            spaces.handle_input(ch);
        }
        spaces.handle_input("\x1b[45;5u");
        assert_eq!(spaces.get_value(), "hello ");
        spaces.handle_input("\x1b[45;5u");
        assert_eq!(spaces.get_value(), "hello");

        let mut paste = Input::default();
        paste.set_value("hello world");
        paste.handle_input("\x01");
        for _ in 0..5 {
            paste.handle_input("\x1b[C");
        }
        paste.handle_input("\x1b[200~beep\n\tboop\x1b[201~");
        assert_eq!(paste.get_value(), "hellobeep    boop world");
        paste.handle_input("\x1b[45;5u");
        assert_eq!(paste.get_value(), "hello world");
    }
}
