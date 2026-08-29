//! Single-line Input matching TS `components/input.ts`.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::keybindings::Keybindings;
use crate::keys::decode_kitty_printable;
use crate::kill_ring::KillRing;
use crate::render::Component;
use crate::undo_stack::UndoStack;
use crate::word_nav::{find_word_backward_default, find_word_forward_default};
use crate::CURSOR_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputState {
    value: String,
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    None,
    Submit(String),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct Input {
    value: String,
    cursor: usize,
    pub focused: bool,
    paste_buffer: String,
    is_in_paste: bool,
    kill_ring: KillRing,
    last_action: Option<LastAction>,
    undo_stack: UndoStack<InputState>,
    keybindings: Keybindings,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            kill_ring: KillRing::new(),
            last_action: None,
            undo_stack: UndoStack::new(),
            keybindings: Keybindings::defaults(),
        }
    }

    pub fn get_value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.len());
    }

    pub fn handle_key(&mut self, data: &str) -> InputAction {
        if data.contains("\x1b[200~") {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            let stripped = data.replacen("\x1b[200~", "", 1);
            if stripped.is_empty() && !data.ends_with("\x1b[201~") {
                return InputAction::None;
            }
            return self.handle_key(&stripped);
        }

        if self.is_in_paste {
            self.paste_buffer.push_str(data);
            if let Some(end_index) = self.paste_buffer.find("\x1b[201~") {
                let paste_content = self.paste_buffer[..end_index].to_string();
                let remaining = self.paste_buffer[end_index + "\x1b[201~".len()..].to_string();
                self.is_in_paste = false;
                self.paste_buffer.clear();
                self.handle_paste(&paste_content);
                if remaining.is_empty() {
                    return InputAction::None;
                }
                return self.handle_key(&remaining);
            }
            return InputAction::None;
        }

        let kb = &self.keybindings;
        if kb.matches(data, "tui.select.cancel") {
            return InputAction::Cancel;
        }
        if kb.matches(data, "tui.editor.undo") {
            self.undo();
            return InputAction::None;
        }
        if kb.matches(data, "tui.input.submit") || data == "\n" {
            return InputAction::Submit(self.value.clone());
        }
        if kb.matches(data, "tui.editor.deleteCharBackward") {
            self.handle_backspace();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.deleteCharForward") {
            self.handle_forward_delete();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.deleteWordBackward") {
            self.delete_word_backwards();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.deleteWordForward") {
            self.delete_word_forward();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.deleteToLineStart") {
            self.delete_to_line_start();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.deleteToLineEnd") {
            self.delete_to_line_end();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.yank") {
            self.yank();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.yankPop") {
            self.yank_pop();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorLeft") {
            self.last_action = None;
            if self.cursor > 0 {
                let len = last_grapheme_len(&self.value[..self.cursor]);
                self.cursor -= len;
            }
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorRight") {
            self.last_action = None;
            if self.cursor < self.value.len() {
                let len = first_grapheme_len(&self.value[self.cursor..]);
                self.cursor += len;
            }
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorLineStart") {
            self.last_action = None;
            self.cursor = 0;
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorLineEnd") {
            self.last_action = None;
            self.cursor = self.value.len();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorWordLeft") {
            self.move_word_backwards();
            return InputAction::None;
        }
        if kb.matches(data, "tui.editor.cursorWordRight") {
            self.move_word_forwards();
            return InputAction::None;
        }
        if let Some(printable) = decode_kitty_printable(data) {
            self.insert_character(&printable);
            return InputAction::None;
        }
        if !has_control_chars(data) {
            self.insert_character(data);
        }
        InputAction::None
    }

    fn insert_character(&mut self, char: &str) {
        if is_whitespace_char(char) || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);
        self.value.insert_str(self.cursor, char);
        self.cursor += char.len();
    }

    fn handle_backspace(&mut self) {
        self.last_action = None;
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let len = last_grapheme_len(&self.value[..self.cursor]);
        let start = self.cursor - len;
        self.value.drain(start..self.cursor);
        self.cursor = start;
    }

    fn handle_forward_delete(&mut self) {
        self.last_action = None;
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let len = first_grapheme_len(&self.value[self.cursor..]);
        self.value.drain(self.cursor..self.cursor + len);
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let deleted = self.value[..self.cursor].to_string();
        self.kill_ring
            .push(&deleted, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value.drain(..self.cursor);
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let deleted = self.value[self.cursor..].to_string();
        self.kill_ring
            .push(&deleted, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value.drain(self.cursor..);
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let delete_from = find_word_backward_default(&self.value, self.cursor);
        let deleted = self.value[delete_from..self.cursor].to_string();
        self.kill_ring.push(&deleted, true, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.drain(delete_from..self.cursor);
        self.cursor = delete_from;
    }

    fn delete_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let delete_to = find_word_forward_default(&self.value, self.cursor);
        let deleted = self.value[self.cursor..delete_to].to_string();
        self.kill_ring.push(&deleted, false, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.drain(self.cursor..delete_to);
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(str::to_string) else {
            return;
        };
        self.push_undo();
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        let prev = self.kill_ring.peek().unwrap_or("").to_string();
        let start = self.cursor.saturating_sub(prev.len());
        if self.value.get(start..self.cursor) == Some(prev.as_str()) {
            self.value.drain(start..self.cursor);
            self.cursor = start;
        }
        self.kill_ring.rotate();
        let text = self.kill_ring.peek().unwrap_or("").to_string();
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(InputState {
            value: self.value.clone(),
            cursor: self.cursor,
        });
    }

    fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        self.value = snapshot.value;
        self.cursor = snapshot.cursor;
        self.last_action = None;
    }

    fn move_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_backward_default(&self.value, self.cursor);
    }

    fn move_word_forwards(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_forward_default(&self.value, self.cursor);
    }

    fn handle_paste(&mut self, pasted_text: &str) {
        self.last_action = None;
        self.push_undo();
        let clean = pasted_text
            .replace("\r\n", "")
            .replace(['\r', '\n'], "")
            .replace('\t', "    ");
        self.value.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }
}

impl Component for Input {
    fn render(&self, width: usize) -> Vec<String> {
        let prompt = "> ";
        let available_width = width.saturating_sub(prompt.len());
        if available_width == 0 {
            return vec![prompt.to_string()];
        }

        let total_width = UnicodeWidthStr::width(self.value.as_str());
        let (visible_text, cursor_display) = if total_width < available_width {
            (self.value.clone(), self.cursor)
        } else {
            let scroll_width = if self.cursor == self.value.len() {
                available_width.saturating_sub(1)
            } else {
                available_width
            };
            let cursor_col = UnicodeWidthStr::width(&self.value[..self.cursor]);
            if scroll_width == 0 {
                (String::new(), 0)
            } else {
                let half_width = scroll_width / 2;
                let start_col = if cursor_col < half_width {
                    0
                } else if cursor_col > total_width.saturating_sub(half_width) {
                    total_width.saturating_sub(scroll_width)
                } else {
                    cursor_col.saturating_sub(half_width)
                };
                let visible = slice_by_column(&self.value, start_col, scroll_width, true);
                let before = slice_by_column(
                    &self.value,
                    start_col,
                    cursor_col.saturating_sub(start_col),
                    true,
                );
                (visible, before.len())
            }
        };

        let at_cursor = first_grapheme(&visible_text[cursor_display.min(visible_text.len())..])
            .unwrap_or(" ")
            .to_string();
        let before_cursor = &visible_text[..cursor_display.min(visible_text.len())];
        let after_start = cursor_display + at_cursor.len();
        let after_cursor = if after_start <= visible_text.len() {
            &visible_text[after_start..]
        } else {
            ""
        };
        let marker = if self.focused { CURSOR_MARKER } else { "" };
        let cursor_char = format!("\x1b[7m{at_cursor}\x1b[27m");
        let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");
        let visual_length = visible_width_ansi(&text_with_cursor);
        let padding = " ".repeat(available_width.saturating_sub(visual_length));
        vec![format!("{prompt}{text_with_cursor}{padding}")]
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

fn is_whitespace_char(char: &str) -> bool {
    !char.is_empty() && char.chars().all(char::is_whitespace)
}

fn has_control_chars(data: &str) -> bool {
    data.chars().any(|ch| {
        let code = ch as u32;
        code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
    })
}

fn last_grapheme_len(text: &str) -> usize {
    text.graphemes(true).next_back().map(str::len).unwrap_or(1)
}

fn first_grapheme_len(text: &str) -> usize {
    text.graphemes(true).next().map(str::len).unwrap_or(1)
}

fn first_grapheme(text: &str) -> Option<&str> {
    text.graphemes(true).next()
}

fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    if length == 0 {
        return String::new();
    }
    let end_col = start_col + length;
    let mut current = 0;
    let mut result = String::new();
    for grapheme in line.graphemes(true) {
        let width = UnicodeWidthStr::width(grapheme);
        let in_range = current >= start_col && current < end_col;
        let fits = !strict || current + width <= end_col;
        if in_range && fits {
            result.push_str(grapheme);
        }
        current += width;
        if current >= end_col {
            break;
        }
    }
    result
}

fn visible_width_ansi(text: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(text).as_str())
}

fn strip_ansi(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
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
            if i + 1 < bytes.len() && bytes[i + 1] == b'_' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if b == 0x07 {
                        break;
                    }
                }
                continue;
            }
            i += 1;
            continue;
        }
        let rest = &text[i..];
        let ch = rest.chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(input: &mut Input, data: &str) -> InputAction {
        input.handle_key(data)
    }

    #[test]
    fn submits_value_including_backslash_on_enter() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", "\\"] {
            feed(&mut input, ch);
        }
        assert_eq!(
            feed(&mut input, "\r"),
            InputAction::Submit("hello\\".into())
        );
    }

    #[test]
    fn inserts_backslash_as_regular_character() {
        let mut input = Input::new();
        feed(&mut input, "\\");
        feed(&mut input, "x");
        assert_eq!(input.get_value(), "\\x");
    }

    #[test]
    fn render_does_not_overflow_wide_cjk_and_fullwidth() {
        let width = 93;
        let cases = [
            "가나다라마바사아자차카타파하 한글 텍스트가 터미널 너비를 초과하면 크래시가 발생합니다 이것은 재현용 테스트입니다",
            "これはテスト文章です。日本語のテキストが正しく表示されるかどうかを確認するためのサンプルテキストです。あいうえお",
            "这是一段测试文本，用于验证中文字符在终端中的显示宽度是否被正确计算，如果不正确就会导致用户界面崩溃的问题",
            "ＡＢＣＤＥＦＧＨＩＪＫＬＭＮＯＰＱＲＳＴＵＶＷＸＹＺ０１２３４５６７８９ａｂｃｄｅｆｇｈｉｊｋｌｍ",
        ];
        for text in cases {
            for steps in [0usize, 10, usize::MAX] {
                let mut input = Input::new();
                input.set_value(text);
                input.focused = true;
                if steps == 0 {
                } else if steps == usize::MAX {
                    feed(&mut input, "\x05");
                } else {
                    for _ in 0..steps {
                        feed(&mut input, "\x1b[C");
                    }
                }
                let line = &input.render(width)[0];
                assert!(
                    visible_width_ansi(line) <= width,
                    "rendered line overflowed"
                );
            }
        }
    }

    #[test]
    fn keeps_cursor_visible_when_scrolling_wide_text() {
        let mut input = Input::new();
        input.set_value("가나다라마바사아자차카타파하");
        input.focused = true;
        feed(&mut input, "\x01");
        for _ in 0..5 {
            feed(&mut input, "\x1b[C");
        }
        let line = &input.render(20)[0];
        assert!(visible_width_ansi(line) <= 20);
    }

    #[test]
    fn ctrl_w_saves_to_kill_ring_and_ctrl_y_yanks() {
        let mut input = Input::new();
        input.set_value("foo bar baz");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "foo bar ");
        feed(&mut input, "\x01");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "bazfoo bar ");
    }

    #[test]
    fn ctrl_w_preserves_ascii_punctuation_boundaries() {
        let mut input = Input::new();
        input.set_value("foo.bar");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "foo.");
        input.set_value("foo:bar");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "foo:");
    }

    #[test]
    fn ctrl_w_handles_unicode_word_boundaries() {
        let mut input = Input::new();
        input.set_value("你好世界。你好，世界");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "你好世界。你好，");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "你好世界。你好");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "你好世界。");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "你好世界");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "你好");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn ctrl_u_and_ctrl_k_save_to_kill_ring() {
        let mut input = Input::new();
        input.set_value("hello world");
        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x15");
        assert_eq!(input.get_value(), "world");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello world");

        input.set_value("hello world");
        feed(&mut input, "\x01");
        feed(&mut input, "\x0b");
        assert_eq!(input.get_value(), "");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn ctrl_y_does_nothing_when_kill_ring_empty() {
        let mut input = Input::new();
        input.set_value("test");
        feed(&mut input, "\x05");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "test");
    }

    #[test]
    fn alt_y_cycles_kill_ring_after_ctrl_y() {
        let mut input = Input::new();
        input.set_value("first");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("second");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("third");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "third");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "second");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "first");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "third");
    }

    #[test]
    fn alt_y_does_nothing_if_not_preceded_by_yank() {
        let mut input = Input::new();
        input.set_value("test");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("other");
        feed(&mut input, "\x05");
        feed(&mut input, "x");
        assert_eq!(input.get_value(), "otherx");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "otherx");
    }

    #[test]
    fn alt_y_does_nothing_if_kill_ring_has_one_entry() {
        let mut input = Input::new();
        input.set_value("only");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "only");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "only");
    }

    #[test]
    fn consecutive_ctrl_w_accumulates() {
        let mut input = Input::new();
        input.set_value("one two three");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        feed(&mut input, "\x17");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "one two three");
    }

    #[test]
    fn non_delete_actions_break_kill_accumulation() {
        let mut input = Input::new();
        input.set_value("foo bar baz");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "foo bar ");
        feed(&mut input, "x");
        assert_eq!(input.get_value(), "foo bar x");
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "foo bar ");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "foo bar x");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "foo bar baz");
    }

    #[test]
    fn non_yank_actions_break_alt_y_chain() {
        let mut input = Input::new();
        input.set_value("first");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("second");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "second");
        feed(&mut input, "x");
        assert_eq!(input.get_value(), "secondx");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "secondx");
    }

    #[test]
    fn kill_ring_rotation_persists_after_cycling() {
        let mut input = Input::new();
        input.set_value("first");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("second");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("third");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("");
        feed(&mut input, "\x19");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "second");
        feed(&mut input, "x");
        input.set_value("");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "second");
    }

    #[test]
    fn backward_deletions_prepend_forward_append() {
        let mut input = Input::new();
        input.set_value("prefix|suffix");
        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x0b");
        assert_eq!(input.get_value(), "prefix");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "prefix|suffix");
    }

    #[test]
    fn alt_d_deletes_word_forward_and_saves() {
        let mut input = Input::new();
        input.set_value("hello world test");
        feed(&mut input, "\x01");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), " world test");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), " test");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello world test");
    }

    #[test]
    fn alt_d_preserves_ascii_punctuation_and_unicode() {
        let mut input = Input::new();
        input.set_value("foo.bar baz");
        feed(&mut input, "\x01");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), ".bar baz");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "bar baz");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), " baz");

        input.set_value("你好世界。你好，世界");
        feed(&mut input, "\x01");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "世界。你好，世界");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "。你好，世界");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "你好，世界");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "，世界");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "世界");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn yank_and_yank_pop_in_middle_of_text() {
        let mut input = Input::new();
        input.set_value("word");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("hello world");
        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello wordworld");

        input.set_value("FIRST");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("SECOND");
        feed(&mut input, "\x05");
        feed(&mut input, "\x17");
        input.set_value("hello world");
        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello SECONDworld");
        feed(&mut input, "\x1by");
        assert_eq!(input.get_value(), "hello FIRSTworld");
    }

    #[test]
    fn undo_coalesces_and_restores_ts_cases() {
        let mut input = Input::new();
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "");

        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            feed(&mut input, ch);
        }
        assert_eq!(input.get_value(), "hello world");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "");

        for ch in ["h", "e", "l", "l", "o", " ", " "] {
            feed(&mut input, ch);
        }
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello ");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "");

        for ch in ["h", "e", "l", "l", "o"] {
            feed(&mut input, ch);
        }
        feed(&mut input, "\x7f");
        assert_eq!(input.get_value(), "hell");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello");

        feed(&mut input, "\x01");
        feed(&mut input, "\x1b[C");
        feed(&mut input, "\x1b[3~");
        assert_eq!(input.get_value(), "hllo");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello");
    }

    #[test]
    fn undo_kills_yank_and_paste() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            feed(&mut input, ch);
        }
        feed(&mut input, "\x17");
        assert_eq!(input.get_value(), "hello ");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");

        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x0b");
        assert_eq!(input.get_value(), "hello ");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");

        feed(&mut input, "\x01");
        for _ in 0..6 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x15");
        assert_eq!(input.get_value(), "world");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");

        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " "] {
            feed(&mut input, ch);
        }
        feed(&mut input, "\x17");
        feed(&mut input, "\x19");
        assert_eq!(input.get_value(), "hello ");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "");

        input.set_value("hello world");
        feed(&mut input, "\x01");
        for _ in 0..5 {
            feed(&mut input, "\x1b[C");
        }
        feed(&mut input, "\x1b[200~beep boop\x1b[201~");
        assert_eq!(input.get_value(), "hellobeep boop world");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");

        input.set_value("hello world");
        feed(&mut input, "\x01");
        feed(&mut input, "\x1bd");
        assert_eq!(input.get_value(), " world");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn cursor_movement_starts_new_undo_unit() {
        let mut input = Input::new();
        feed(&mut input, "a");
        feed(&mut input, "b");
        feed(&mut input, "c");
        feed(&mut input, "\x01");
        feed(&mut input, "\x05");
        feed(&mut input, "d");
        feed(&mut input, "e");
        assert_eq!(input.get_value(), "abcde");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "abc");
        feed(&mut input, "\x1b[45;5u");
        assert_eq!(input.get_value(), "");
    }
}
