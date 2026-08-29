use std::collections::BTreeMap;

use crate::kill_ring::KillRing;
use crate::render::{visible_width, Component};
use crate::undo_stack::UndoStack;
use crate::CURSOR_MARKER;

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorState {
    buffer: String,
    cursor: usize,
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
    undo_stack: UndoStack<EditorState>,
    history_index: i32,
    history_draft: Option<EditorState>,
    pastes: BTreeMap<usize, String>,
    paste_counter: usize,
    preferred_col: Option<usize>,
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
            undo_stack: UndoStack::new(),
            history_index: -1,
            history_draft: None,
            pastes: BTreeMap::new(),
            paste_counter: 0,
            preferred_col: None,
        }
    }

    pub fn get_text(&self) -> &str {
        &self.buffer
    }

    pub fn get_expanded_text(&self) -> String {
        expand_paste_markers(&self.buffer, &self.pastes)
    }

    pub fn get_cursor(&self) -> (usize, usize) {
        cursor_line_col(&self.buffer, self.cursor)
    }

    pub fn add_to_history(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.history.first().is_some_and(|entry| entry == trimmed) {
            return;
        }
        self.history.insert(0, trimmed.to_string());
        if self.history.len() > HISTORY_LIMIT {
            self.history.pop();
        }
    }

    fn exit_history_browsing(&mut self) {
        self.history_index = -1;
        self.history_draft = None;
    }

    pub fn navigate_history(&mut self, direction: i32) {
        self.last_action = None;
        if self.history.is_empty() {
            return;
        }
        let new_index = self.history_index - direction;
        if new_index < -1 || new_index >= self.history.len() as i32 {
            return;
        }
        if self.history_index == -1 && new_index >= 0 {
            self.push_undo();
            self.history_draft = Some(EditorState {
                buffer: self.buffer.clone(),
                cursor: self.cursor,
            });
        }
        self.history_index = new_index;
        if self.history_index == -1 {
            if let Some(draft) = self.history_draft.take() {
                self.buffer = draft.buffer;
                self.cursor = draft.cursor;
            } else {
                self.buffer.clear();
                self.cursor = 0;
            }
            self.preferred_col = None;
        } else {
            let text = self.history[self.history_index as usize].clone();
            self.buffer = text;
            if direction < 0 {
                self.cursor = 0;
            } else {
                self.cursor = self.buffer.len();
            }
            self.preferred_col = None;
        }
    }

    pub fn cursor_up(&mut self) {
        let (line, col) = self.get_cursor();
        if line == 0 && (self.buffer.is_empty() || self.history_index > -1 || col == 0) {
            self.navigate_history(-1);
        } else if line == 0 {
            self.move_line_start();
        } else {
            self.move_vertical(-1);
        }
    }

    pub fn cursor_down(&mut self) {
        let (line, _) = self.get_cursor();
        let last = self.buffer.matches('\n').count();
        if self.history_index > -1 && line == last {
            self.navigate_history(1);
        } else if line == last {
            self.move_line_end();
        } else {
            self.move_vertical(1);
        }
    }

    fn move_vertical(&mut self, delta: isize) {
        let (line, col) = self.get_cursor();
        let preferred = self.preferred_col.unwrap_or(col);
        self.preferred_col = Some(preferred);
        let last = self.buffer.matches('\n').count();
        let new_line = (line as isize + delta).clamp(0, last as isize) as usize;
        set_cursor_line_col(&self.buffer, &mut self.cursor, new_line, preferred);
    }

    pub fn handle_paste(&mut self, pasted_text: &str) {
        self.exit_history_browsing();
        self.last_action = None;
        self.push_undo();
        let decoded = decode_paste_csi_u(pasted_text);
        let clean = decoded
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        let mut filtered: String = clean
            .chars()
            .filter(|ch| *ch == '\n' || (*ch as u32) >= 32)
            .collect();
        if filtered.starts_with(['/', '~', '.']) {
            if let Some(before) = self.buffer[..self.cursor].chars().next_back() {
                if before.is_ascii_alphanumeric() || before == '_' {
                    filtered.insert(0, ' ');
                }
            }
        }
        let lines = filtered.split('\n').count();
        let total_chars = filtered.chars().count();
        if lines > 10 || total_chars > 1000 {
            self.paste_counter += 1;
            let paste_id = self.paste_counter;
            self.pastes.insert(paste_id, filtered.clone());
            let marker = if lines > 10 {
                format!("[paste #{paste_id} +{lines} lines]")
            } else {
                format!("[paste #{paste_id} {total_chars} chars]")
            };
            self.insert_str_internal(&marker);
            return;
        }
        self.insert_str_internal(&filtered);
    }

    fn insert_str_internal(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(EditorState {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
        });
    }

    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        self.buffer = snapshot.buffer;
        self.cursor = snapshot.cursor;
        self.last_action = None;
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
        self.exit_history_browsing();
        self.preferred_col = None;
        self.push_undo();
        self.insert_str_internal(text);
        self.clear_last_action();
    }

    pub fn insert(&mut self, ch: char) {
        self.exit_history_browsing();
        self.preferred_col = None;
        if ch.is_whitespace() || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace(&mut self) {
        self.clear_last_action();
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
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
        self.push_undo();
        let next = self.buffer[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.buffer.drain(self.cursor..self.cursor + next);
    }

    pub fn move_left(&mut self) {
        self.clear_last_action();
        self.preferred_col = None;
        if self.cursor == 0 {
            return;
        }
        if let Some((start, _)) = marker_ending_at(&self.buffer, self.cursor) {
            self.cursor = start;
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
        self.preferred_col = None;
        if self.cursor >= self.buffer.len() {
            return;
        }
        if let Some((_, end)) = marker_starting_at(&self.buffer, self.cursor) {
            self.cursor = end;
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
        self.push_undo();
        let start = crate::word_nav::find_word_backward_default(&self.buffer, self.cursor);
        let deleted = self.buffer[start..self.cursor].to_string();
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
        self.kill_ring
            .push(&deleted, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn delete_word_forwards(&mut self) {
        self.push_undo();
        let end = crate::word_nav::find_word_forward_default(&self.buffer, self.cursor);
        let deleted = self.buffer[self.cursor..end].to_string();
        self.buffer.drain(self.cursor..end);
        self.kill_ring
            .push(&deleted, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
    }

    pub fn delete_to_line_start(&mut self) {
        self.push_undo();
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
        self.push_undo();
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
        self.push_undo();
        self.buffer.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    pub fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
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
        self.exit_history_browsing();
        self.pastes.clear();
        self.paste_counter = 0;
        self.preferred_col = None;
        self.buffer = text.into();
        self.cursor = self.buffer.len();
    }

    pub fn submit(&mut self) -> String {
        let value = self.get_expanded_text();
        let result = value.trim().to_string();
        self.add_to_history(&result);
        self.buffer.clear();
        self.cursor = 0;
        self.pastes.clear();
        self.paste_counter = 0;
        self.exit_history_browsing();
        self.undo_stack.clear();
        self.last_action = None;
        self.preferred_col = None;
        result
    }
}

fn cursor_line_col(buffer: &str, cursor: usize) -> (usize, usize) {
    let before = &buffer[..cursor.min(buffer.len())];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().map(str::len).unwrap_or(0);
    (line, col)
}

fn set_cursor_line_col(buffer: &str, cursor: &mut usize, line: usize, col: usize) {
    let mut pos = 0;
    for (index, part) in buffer.split('\n').enumerate() {
        if index == line {
            *cursor = pos + col.min(part.len());
            return;
        }
        pos += part.len() + 1;
    }
    *cursor = buffer.len();
}

fn expand_paste_markers(text: &str, pastes: &BTreeMap<usize, String>) -> String {
    let mut result = text.to_string();
    for (id, content) in pastes {
        let markers = paste_markers(&result);
        for (start, end) in markers.into_iter().rev() {
            if parse_marker_id(&result[start..end]) == Some(*id) {
                result.replace_range(start..end, content);
            }
        }
    }
    result
}

fn paste_markers(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut search = 0;
    while let Some(rel) = text[search..].find("[paste #") {
        let start = search + rel;
        let Some(end_rel) = text[start..].find(']') else {
            break;
        };
        let end = start + end_rel + 1;
        if parse_marker_id(&text[start..end]).is_some() {
            out.push((start, end));
        }
        search = end;
    }
    out
}

fn parse_marker_id(marker: &str) -> Option<usize> {
    let inner = marker.strip_prefix("[paste #")?.strip_suffix(']')?;
    if let Some((id, rest)) = inner.split_once(' ') {
        let id = id.parse().ok()?;
        if let Some(count) = rest
            .strip_prefix('+')
            .and_then(|rest| rest.strip_suffix(" lines"))
        {
            count.parse::<usize>().ok()?;
            return Some(id);
        }
        if let Some(count) = rest.strip_suffix(" chars") {
            count.parse::<usize>().ok()?;
            return Some(id);
        }
        return None;
    }
    inner.parse().ok()
}

fn marker_starting_at(text: &str, cursor: usize) -> Option<(usize, usize)> {
    paste_markers(text)
        .into_iter()
        .find(|(start, _)| *start == cursor)
}

fn marker_ending_at(text: &str, cursor: usize) -> Option<(usize, usize)> {
    paste_markers(text)
        .into_iter()
        .find(|(_, end)| *end == cursor)
}

fn decode_paste_csi_u(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("\x1b[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('u') {
            let seq = &after[..end];
            if let Some((code, "5")) = seq.split_once(";5") {
                if let Ok(cp) = code.parse::<u32>() {
                    let decoded = if (97..=122).contains(&cp) {
                        Some(char::from_u32(cp - 96))
                    } else if (65..=90).contains(&cp) {
                        Some(char::from_u32(cp - 64))
                    } else {
                        None
                    };
                    if let Some(ch) = decoded.flatten() {
                        out.push(ch);
                        rest = &after[end + 1..];
                        continue;
                    }
                }
            }
        }
        out.push_str("\x1b[");
        rest = after;
    }
    out.push_str(rest);
    out
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

        editor.set_text("你好，世界");
        editor.move_word_backwards();
        assert_eq!(editor.cursor, "你好，".len());
        editor.move_word_backwards();
        assert_eq!(editor.cursor, "你好".len());
        editor.move_word_backwards();
        assert_eq!(editor.cursor, 0);
        editor.move_word_forwards();
        assert_eq!(editor.cursor, "你好".len());
        editor.move_word_forwards();
        assert_eq!(editor.cursor, "你好，".len());
        editor.move_word_forwards();
        assert_eq!(editor.cursor, editor.buffer.len());

        editor.set_text("你好世界。你好，世界");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "你好世界。你好，");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "你好世界。你好");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "你好世界。");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "你好世界");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "你好");
        editor.delete_word_backwards();
        assert_eq!(editor.buffer, "");
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

        let mut editor = Editor::new();
        for ch in "hello world".chars() {
            editor.insert(ch);
        }
        assert_eq!(editor.buffer, "hello world");
        editor.undo();
        assert_eq!(editor.buffer, "hello");
        editor.undo();
        assert_eq!(editor.buffer, "");
        editor.begin_jump_forward();
        assert_eq!(editor.jump_mode(), Some(true));
        editor.cancel_jump();
        assert_eq!(editor.jump_mode(), None);
    }

    #[test]
    fn prompt_history_matches_ts_editor_tests() {
        let mut editor = Editor::new();
        editor.cursor_up();
        assert_eq!(editor.get_text(), "");

        editor.add_to_history("first prompt");
        editor.add_to_history("second prompt");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "second prompt");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("first");
        editor.add_to_history("second");
        editor.add_to_history("third");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "third");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "second");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "first");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "first");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("prompt");
        editor.set_text("draft");
        editor.cursor = "dr".len();
        editor.cursor_up();
        assert_eq!(editor.get_text(), "draft");
        assert_eq!(editor.get_cursor(), (0, 0));
        editor.cursor_up();
        assert_eq!(editor.get_text(), "prompt");
        editor.cursor_down();
        assert_eq!(editor.get_text(), "draft");
        assert_eq!(editor.get_cursor(), (0, 0));

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("");
        editor.add_to_history("   ");
        editor.add_to_history("valid");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "valid");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "valid");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("same");
        editor.add_to_history("same");
        editor.add_to_history("same");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "same");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "same");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("first");
        editor.add_to_history("second");
        editor.add_to_history("first");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "first");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "second");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "first");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("history item");
        editor.set_text("line1\nline2");
        editor.cursor_up();
        editor.insert('X');
        assert_eq!(editor.get_text(), "line1X\nline2");

        editor.set_text("");
        editor.history.clear();
        for i in 0..105 {
            editor.add_to_history(&format!("prompt {i}"));
        }
        for _ in 0..100 {
            editor.cursor_up();
        }
        assert_eq!(editor.get_text(), "prompt 5");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "prompt 5");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("older entry");
        editor.add_to_history("line1\nline2\nline3");
        editor.cursor_up();
        assert_eq!(editor.get_text(), "line1\nline2\nline3");
        assert_eq!(editor.get_cursor(), (0, 0));
        editor.cursor_up();
        assert_eq!(editor.get_text(), "older entry");
        assert_eq!(editor.get_cursor(), (0, 0));

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("older entry");
        editor.add_to_history("line1\nline2\nline3");
        editor.add_to_history("newer entry");
        editor.cursor_up();
        editor.cursor_up();
        editor.cursor_up();
        editor.cursor_down();
        assert_eq!(editor.get_text(), "line1\nline2\nline3");
        assert_eq!(editor.get_cursor(), (2, 5));
        editor.cursor_down();
        assert_eq!(editor.get_text(), "newer entry");

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("line1\nline2\nline3");
        editor.cursor_up();
        assert_eq!(editor.get_cursor(), (0, 0));
        editor.cursor_down();
        assert_eq!(editor.get_text(), "line1\nline2\nline3");
        assert_eq!(editor.get_cursor(), (1, 0));
        editor.cursor_up();
        assert_eq!(editor.get_cursor(), (0, 0));

        editor.set_text("");
        editor.history.clear();
        editor.add_to_history("old prompt");
        editor.cursor_up();
        editor.insert('x');
        assert_eq!(editor.get_text(), "xold prompt");
    }

    #[test]
    fn dedicated_history_keys_browse_without_moving_cursor_first() {
        let mut editor = Editor::new();
        editor.add_to_history("older prompt");
        editor.add_to_history("newer\nmultiline prompt");
        editor.set_text("draft");
        editor.cursor = "dr".len();
        editor.navigate_history(-1);
        assert_eq!(editor.get_text(), "newer\nmultiline prompt");
        assert_eq!(editor.get_cursor(), (0, 0));
        editor.navigate_history(-1);
        assert_eq!(editor.get_text(), "older prompt");
        editor.navigate_history(1);
        assert_eq!(editor.get_text(), "newer\nmultiline prompt");
        assert_eq!(editor.get_cursor(), (1, 16));
        editor.navigate_history(1);
        assert_eq!(editor.get_text(), "draft");
        assert_eq!(editor.get_cursor(), (0, 2));
    }

    #[test]
    fn large_paste_markers_expand_on_submit() {
        let mut editor = Editor::new();
        let big = (0..20)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.handle_paste(&big);
        assert!(editor.get_text().contains("[paste #1 +20 lines]"));
        editor.insert('A');
        editor.move_line_start();
        editor.move_right();
        assert_eq!(editor.cursor, editor.get_text().len() - 1);
        editor.move_left();
        assert_eq!(editor.cursor, 0);
        assert_eq!(editor.submit(), format!("{big}A"));

        let mut editor = Editor::new();
        editor.handle_paste(&"x".repeat(1001));
        assert!(editor.get_text().contains("[paste #1 1001 chars]"));
        assert_eq!(editor.submit(), "x".repeat(1001));
    }
}
