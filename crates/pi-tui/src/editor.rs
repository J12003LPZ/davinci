use std::cell::Cell;
use std::collections::BTreeMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::kill_ring::KillRing;
use crate::render::{visible_width, visible_width_stripped, Component};
use crate::undo_stack::UndoStack;
use crate::word_wrap::{grapheme_segments, word_wrap_line, Segment};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSnapshot {
    state: EditorState,
    pastes: BTreeMap<usize, String>,
    paste_counter: usize,
}

#[derive(Debug, Clone, Copy)]
struct VisualLine {
    logical_line: usize,
    start_col: usize,
    length: usize,
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
    undo_stack: UndoStack<EditorSnapshot>,
    history_index: i32,
    history_draft: Option<EditorState>,
    pastes: BTreeMap<usize, String>,
    paste_counter: usize,
    preferred_col: Option<usize>,
    snapped_from_cursor_col: Option<usize>,
    padding_x: usize,
    last_width: Cell<usize>,
    scroll_offset: Cell<usize>,
    terminal_rows: usize,
    focused: bool,
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
            snapped_from_cursor_col: None,
            padding_x: 0,
            last_width: Cell::new(80),
            scroll_offset: Cell::new(0),
            terminal_rows: 24,
            focused: true,
        }
    }

    pub fn set_padding_x(&mut self, padding: usize) {
        self.padding_x = padding;
    }

    pub fn set_terminal_rows(&mut self, rows: usize) {
        self.terminal_rows = rows.max(1);
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
        if self.is_on_first_visual_line()
            && (self.buffer.is_empty() || self.history_index > -1 || self.get_cursor().1 == 0)
        {
            self.navigate_history(-1);
        } else if self.is_on_first_visual_line() {
            self.move_line_start();
        } else {
            self.move_visual(-1);
        }
    }

    pub fn cursor_down(&mut self) {
        if self.history_index > -1 && self.is_on_last_visual_line() {
            self.navigate_history(1);
        } else if self.is_on_last_visual_line() {
            self.move_line_end();
        } else {
            self.move_visual(1);
        }
    }

    pub fn page_up(&mut self) {
        self.page_scroll(-1);
    }

    pub fn page_down(&mut self) {
        self.page_scroll(1);
    }

    fn is_on_first_visual_line(&self) -> bool {
        let lines = self.build_visual_line_map(self.last_width.get());
        self.find_current_visual_line(&lines) == 0
    }

    fn is_on_last_visual_line(&self) -> bool {
        let lines = self.build_visual_line_map(self.last_width.get());
        self.find_current_visual_line(&lines) + 1 == lines.len()
    }

    fn move_visual(&mut self, delta: isize) {
        self.last_action = None;
        let visual_lines = self.build_visual_line_map(self.last_width.get());
        let current = self.find_current_visual_line(&visual_lines);
        let target = current as isize + delta;
        if target >= 0 && (target as usize) < visual_lines.len() {
            self.move_to_visual_line(&visual_lines, current, target as usize);
        }
    }

    fn page_scroll(&mut self, direction: isize) {
        self.last_action = None;
        let page_size = (self.terminal_rows as f64 * 0.3).floor() as usize;
        let page_size = page_size.max(5);
        let visual_lines = self.build_visual_line_map(self.last_width.get());
        if visual_lines.is_empty() {
            return;
        }
        let current = self.find_current_visual_line(&visual_lines);
        let target = current
            .saturating_add_signed(direction * page_size as isize)
            .min(visual_lines.len() - 1);
        self.move_to_visual_line(&visual_lines, current, target);
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

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            state: EditorState {
                buffer: self.buffer.clone(),
                cursor: self.cursor,
            },
            pastes: self.pastes.clone(),
            paste_counter: self.paste_counter,
        }
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
    }

    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        self.buffer = snapshot.state.buffer;
        self.cursor = snapshot.state.cursor;
        self.pastes = snapshot.pastes;
        self.paste_counter = snapshot.paste_counter;
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
        self.exit_history_browsing();
        self.clear_last_action();
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        if let Some((start, end)) = marker_ending_at(&self.buffer, self.cursor, &self.pastes) {
            if let Some(id) = parse_marker_id(&self.buffer[start..end]) {
                self.remove_paste_id(id);
            }
            self.buffer.drain(start..end);
            self.cursor = start;
            return;
        }
        let prev = prev_grapheme_len(&self.buffer[..self.cursor]);
        self.cursor -= prev;
        self.buffer.drain(self.cursor..self.cursor + prev);
    }

    pub fn delete_forward(&mut self) {
        self.exit_history_browsing();
        self.clear_last_action();
        if self.cursor >= self.buffer.len() {
            return;
        }
        self.push_undo();
        if let Some((start, end)) = marker_starting_at(&self.buffer, self.cursor, &self.pastes) {
            if let Some(id) = parse_marker_id(&self.buffer[start..end]) {
                self.remove_paste_id(id);
            }
            self.buffer.drain(start..end);
            return;
        }
        let next = next_grapheme_len(&self.buffer[self.cursor..]);
        self.buffer.drain(self.cursor..self.cursor + next);
    }

    fn remove_paste_id(&mut self, target_id: usize) {
        self.pastes.remove(&target_id);
        self.paste_counter = self.paste_counter.saturating_sub(1);
        let mut higher: Vec<usize> = self
            .pastes
            .keys()
            .copied()
            .filter(|id| *id > target_id)
            .collect();
        higher.sort_unstable();
        for id in higher {
            if let Some(content) = self.pastes.remove(&id) {
                self.pastes.insert(id - 1, content);
            }
        }
        self.buffer = renumber_markers(&self.buffer, target_id);
    }

    pub fn move_left(&mut self) {
        self.clear_last_action();
        self.preferred_col = None;
        if self.cursor == 0 {
            return;
        }
        if let Some((start, _)) = marker_ending_at(&self.buffer, self.cursor, &self.pastes) {
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
        if let Some((_, end)) = marker_starting_at(&self.buffer, self.cursor, &self.pastes) {
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
        let normalized = normalize_editor_text(&text.into());
        if self.buffer != normalized {
            self.push_undo();
        }
        self.pastes.clear();
        self.paste_counter = 0;
        self.preferred_col = None;
        self.snapped_from_cursor_col = None;
        self.buffer = normalized;
        self.cursor = self.buffer.len();
        self.last_action = None;
    }

    pub fn insert_text_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.exit_history_browsing();
        self.push_undo();
        self.last_action = None;
        self.insert_str_internal(&normalize_editor_text(text));
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
        self.snapped_from_cursor_col = None;
        result
    }

    fn segment_line(&self, line: &str) -> Vec<Segment> {
        merge_marker_segments(line, &self.pastes)
    }

    fn build_visual_line_map(&self, width: usize) -> Vec<VisualLine> {
        let mut visual_lines = Vec::new();
        let logical: Vec<&str> = if self.buffer.is_empty() {
            vec![""]
        } else {
            self.buffer.split('\n').collect()
        };
        for (i, line) in logical.iter().enumerate() {
            if line.is_empty() {
                visual_lines.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: 0,
                });
            } else if visible_width(line) <= width {
                visual_lines.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: line.len(),
                });
            } else {
                let segs = self.segment_line(line);
                for chunk in word_wrap_line(line, width, Some(&segs)) {
                    visual_lines.push(VisualLine {
                        logical_line: i,
                        start_col: chunk.start_index,
                        length: chunk.end_index - chunk.start_index,
                    });
                }
            }
        }
        visual_lines
    }

    fn find_visual_line_at(&self, visual_lines: &[VisualLine], line: usize, col: usize) -> usize {
        for (i, vl) in visual_lines.iter().enumerate() {
            if vl.logical_line != line {
                continue;
            }
            let offset = col as isize - vl.start_col as isize;
            let is_last =
                i + 1 == visual_lines.len() || visual_lines[i + 1].logical_line != vl.logical_line;
            if offset >= 0
                && (offset < vl.length as isize || (is_last && offset == vl.length as isize))
            {
                return i;
            }
        }
        visual_lines.len().saturating_sub(1)
    }

    fn find_current_visual_line(&self, visual_lines: &[VisualLine]) -> usize {
        let (line, col) = self.get_cursor();
        self.find_visual_line_at(visual_lines, line, col)
    }

    fn move_to_visual_line(
        &mut self,
        visual_lines: &[VisualLine],
        current_visual_line: usize,
        target_visual_line: usize,
    ) {
        let Some(current_vl) = visual_lines.get(current_visual_line) else {
            return;
        };
        let Some(target_vl) = visual_lines.get(target_visual_line) else {
            return;
        };
        let current_visual_col = if let Some(snapped) = self.snapped_from_cursor_col {
            let vl_index = self.find_visual_line_at(visual_lines, current_vl.logical_line, snapped);
            snapped.saturating_sub(visual_lines[vl_index].start_col)
        } else {
            self.get_cursor().1.saturating_sub(current_vl.start_col)
        };

        let is_last_source = current_visual_line + 1 == visual_lines.len()
            || visual_lines[current_visual_line + 1].logical_line != current_vl.logical_line;
        let source_max = if is_last_source {
            current_vl.length
        } else {
            current_vl.length.saturating_sub(1)
        };
        let is_last_target = target_visual_line + 1 == visual_lines.len()
            || visual_lines[target_visual_line + 1].logical_line != target_vl.logical_line;
        let target_max = if is_last_target {
            target_vl.length
        } else {
            target_vl.length.saturating_sub(1)
        };
        let move_to = self.compute_vertical_move_column(current_visual_col, source_max, target_max);
        let logical = self
            .buffer
            .split('\n')
            .nth(target_vl.logical_line)
            .unwrap_or("");
        let target_col = (target_vl.start_col + move_to).min(logical.len());
        set_cursor_line_col(
            &self.buffer,
            &mut self.cursor,
            target_vl.logical_line,
            target_col,
        );

        let segments = self.segment_line(logical);
        for seg in &segments {
            if seg.index > target_col {
                break;
            }
            if seg.text.len() <= 1 {
                continue;
            }
            if target_col < seg.index + seg.text.len() {
                let is_continuation = seg.index < target_vl.start_col;
                let is_moving_down = target_visual_line > current_visual_line;
                if is_continuation && is_moving_down {
                    let seg_end = seg.index + seg.text.len();
                    let mut next = target_visual_line + 1;
                    while next < visual_lines.len()
                        && visual_lines[next].logical_line == target_vl.logical_line
                        && visual_lines[next].start_col < seg_end
                    {
                        next += 1;
                    }
                    if next < visual_lines.len() {
                        self.move_to_visual_line(visual_lines, current_visual_line, next);
                        return;
                    }
                }
                self.snapped_from_cursor_col = Some(target_col);
                set_cursor_line_col(
                    &self.buffer,
                    &mut self.cursor,
                    target_vl.logical_line,
                    seg.index,
                );
                return;
            }
        }
        self.snapped_from_cursor_col = None;
    }

    fn compute_vertical_move_column(
        &mut self,
        current_visual_col: usize,
        source_max: usize,
        target_max: usize,
    ) -> usize {
        let has_preferred = self.preferred_col.is_some();
        let cursor_in_middle = current_visual_col < source_max;
        let target_too_short = target_max < current_visual_col;
        if !has_preferred || cursor_in_middle {
            if target_too_short {
                self.preferred_col = Some(current_visual_col);
                return target_max;
            }
            self.preferred_col = None;
            return current_visual_col;
        }
        let preferred = self.preferred_col.unwrap_or(current_visual_col);
        if target_too_short || target_max < preferred {
            return target_max;
        }
        self.preferred_col = None;
        preferred
    }

    fn layout_width(&self, width: usize) -> (usize, usize, usize) {
        let max_padding = (width.saturating_sub(1)) / 2;
        let padding_x = self.padding_x.min(max_padding);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        let layout_width = if padding_x > 0 {
            content_width
        } else {
            content_width.saturating_sub(1).max(1)
        };
        (padding_x, content_width, layout_width)
    }

    fn layout_text(&self, content_width: usize) -> Vec<LayoutLine> {
        let logical: Vec<&str> = if self.buffer.is_empty() {
            vec![""]
        } else {
            self.buffer.split('\n').collect()
        };
        let (cursor_line, cursor_col) = self.get_cursor();
        let mut layout = Vec::new();
        for (i, line) in logical.iter().enumerate() {
            let is_current = i == cursor_line;
            if visible_width(line) <= content_width {
                layout.push(LayoutLine {
                    text: (*line).to_string(),
                    has_cursor: is_current,
                    cursor_pos: if is_current { cursor_col } else { 0 },
                });
                continue;
            }
            let segs = self.segment_line(line);
            let chunks = word_wrap_line(line, content_width, Some(&segs));
            for (chunk_index, chunk) in chunks.iter().enumerate() {
                let last = chunk_index + 1 == chunks.len();
                let mut has_cursor = false;
                let mut cursor_pos = 0;
                if is_current {
                    if last {
                        has_cursor = cursor_col >= chunk.start_index;
                        cursor_pos = cursor_col.saturating_sub(chunk.start_index);
                    } else {
                        has_cursor =
                            cursor_col >= chunk.start_index && cursor_col < chunk.end_index;
                        if has_cursor {
                            cursor_pos = cursor_col.saturating_sub(chunk.start_index);
                            if cursor_pos > chunk.text.len() {
                                cursor_pos = chunk.text.len();
                            }
                        }
                    }
                }
                layout.push(LayoutLine {
                    text: chunk.text.clone(),
                    has_cursor,
                    cursor_pos,
                });
            }
        }
        layout
    }
}

#[derive(Debug, Clone)]
struct LayoutLine {
    text: String,
    has_cursor: bool,
    cursor_pos: usize,
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

fn valid_markers(text: &str, pastes: &BTreeMap<usize, String>) -> Vec<(usize, usize)> {
    paste_markers(text)
        .into_iter()
        .filter(|(start, end)| {
            parse_marker_id(&text[*start..*end]).is_some_and(|id| pastes.contains_key(&id))
        })
        .collect()
}

fn marker_starting_at(
    text: &str,
    cursor: usize,
    pastes: &BTreeMap<usize, String>,
) -> Option<(usize, usize)> {
    valid_markers(text, pastes)
        .into_iter()
        .find(|(start, _)| *start == cursor)
}

fn marker_ending_at(
    text: &str,
    cursor: usize,
    pastes: &BTreeMap<usize, String>,
) -> Option<(usize, usize)> {
    valid_markers(text, pastes)
        .into_iter()
        .find(|(_, end)| *end == cursor)
}

fn merge_marker_segments(line: &str, pastes: &BTreeMap<usize, String>) -> Vec<Segment> {
    let markers = valid_markers(line, pastes);
    if markers.is_empty() {
        return grapheme_segments(line);
    }
    let mut out = Vec::new();
    let mut marker_idx = 0;
    for seg in grapheme_segments(line) {
        while marker_idx < markers.len() && markers[marker_idx].1 <= seg.index {
            marker_idx += 1;
        }
        if let Some(&(start, end)) = markers.get(marker_idx) {
            if seg.index >= start && seg.index < end {
                if seg.index == start {
                    out.push(Segment {
                        text: line[start..end].to_string(),
                        index: start,
                    });
                }
                continue;
            }
        }
        out.push(seg);
    }
    out
}

fn renumber_markers(text: &str, target_id: usize) -> String {
    let markers = paste_markers(text);
    let mut result = String::new();
    let mut last = 0;
    for (start, end) in markers {
        result.push_str(&text[last..start]);
        let marker = &text[start..end];
        if let Some(id) = parse_marker_id(marker) {
            if id > target_id {
                result.push_str(&marker.replacen(&format!("#{id}"), &format!("#{}", id - 1), 1));
            } else {
                result.push_str(marker);
            }
        } else {
            result.push_str(marker);
        }
        last = end;
    }
    result.push_str(&text[last..]);
    result
}

fn normalize_editor_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

fn prev_grapheme_len(text: &str) -> usize {
    text.graphemes(true).next_back().map(str::len).unwrap_or(0)
}

fn next_grapheme_len(text: &str) -> usize {
    text.graphemes(true).next().map(str::len).unwrap_or(0)
}

fn create_scroll_border(direction: &str, hidden: usize, width: usize) -> String {
    let indicator = format!("─── {direction} {hidden} more ");
    let remaining = width as isize - visible_width(&indicator) as isize;
    if remaining >= 0 {
        return format!("{indicator}{}", "─".repeat(remaining as usize));
    }
    let ellipsis = &"..."[..width.min(3)];
    let keep = width.saturating_sub(visible_width(ellipsis));
    let mut clipped = String::new();
    let mut used = 0;
    for ch in indicator.chars() {
        let w = visible_width(&ch.to_string());
        if used + w > keep {
            break;
        }
        clipped.push(ch);
        used += w;
    }
    format!("{clipped}{ellipsis}")
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
        let width = width.max(1);
        let (padding_x, content_width, layout_width) = self.layout_width(width);
        self.last_width.set(layout_width);

        let layout = self.layout_text(layout_width);
        let max_visible = ((self.terminal_rows as f64) * 0.3).floor() as usize;
        let max_visible = max_visible.max(5);
        let cursor_line = layout.iter().position(|line| line.has_cursor).unwrap_or(0);
        let mut scroll = self.scroll_offset.get();
        if cursor_line < scroll {
            scroll = cursor_line;
        } else if cursor_line >= scroll + max_visible {
            scroll = cursor_line + 1 - max_visible;
        }
        let max_scroll = layout.len().saturating_sub(max_visible);
        scroll = scroll.min(max_scroll);
        self.scroll_offset.set(scroll);
        let visible = &layout[scroll..(scroll + max_visible).min(layout.len())];

        let left = " ".repeat(padding_x);
        let right = left.clone();
        let mut result = Vec::new();
        if scroll > 0 {
            result.push(create_scroll_border("↑", scroll, width));
        } else {
            result.push("─".repeat(width));
        }

        for line in visible {
            let mut display = line.text.clone();
            let mut cursor_in_padding = false;
            if line.has_cursor {
                let pos = line.cursor_pos.min(display.len());
                let before = display[..pos].to_string();
                let after = display[pos..].to_string();
                let marker = if self.focused { CURSOR_MARKER } else { "" };
                if after.is_empty() {
                    display = format!("{before}{marker}\x1b[7m \x1b[0m");
                    if visible_width_stripped(&display) > content_width && padding_x > 0 {
                        cursor_in_padding = true;
                    }
                } else {
                    let g_len = next_grapheme_len(&after);
                    let first = &after[..g_len];
                    let rest = &after[g_len..];
                    display = format!("{before}{marker}\x1b[7m{first}\x1b[0m{rest}");
                }
            }
            let line_vis = visible_width_stripped(&display);
            let pad = " ".repeat(content_width.saturating_sub(line_vis));
            let line_right = if cursor_in_padding && !right.is_empty() {
                right[1..].to_string()
            } else {
                right.clone()
            };
            result.push(format!("{left}{display}{pad}{line_right}"));
        }

        let below = layout.len().saturating_sub(scroll + visible.len());
        if below > 0 {
            result.push(create_scroll_border("↓", below, width));
        } else {
            result.push("─".repeat(width));
        }
        result
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
        let lines = editor.render(20);
        assert!(lines.iter().any(|line| line.contains(CURSOR_MARKER)));
        assert!(lines
            .iter()
            .all(|line| crate::render::visible_width_stripped(line) <= 20));
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

    #[test]
    fn paste_marker_delete_renumbers_and_undo_restores_registry() {
        let mut editor = Editor::new();
        let paste_a = (0..12)
            .map(|i| format!("alpha{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let paste_b = (0..12)
            .map(|i| format!("beta{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let paste_c = (0..12)
            .map(|i| format!("gamma{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.handle_paste(&paste_a);
        editor.backspace();
        editor.undo();
        assert_eq!(editor.submit(), paste_a);

        let mut editor = Editor::new();
        editor.handle_paste(&paste_a);
        editor.handle_paste(&paste_b);
        editor.move_line_start();
        editor.move_right();
        editor.backspace();
        editor.undo();
        assert_eq!(editor.submit(), format!("{paste_a}{paste_b}"));

        let mut editor = Editor::new();
        editor.handle_paste(&paste_a);
        editor.move_line_start();
        editor.handle_paste(&paste_b);
        editor.move_line_start();
        editor.handle_paste(&paste_c);
        editor.move_line_end();
        editor.backspace();
        assert_eq!(editor.submit(), format!("{paste_c}{paste_b}"));

        let mut editor = Editor::new();
        editor.handle_paste(&paste_a);
        editor.set_text("replacement");
        editor.undo();
        assert_eq!(editor.submit(), paste_a);

        let mut editor = Editor::new();
        editor.handle_paste(&paste_a);
        editor.insert('B');
        editor.move_line_start();
        editor.move_right();
        editor.backspace();
        assert_eq!(editor.get_text(), "B");
        editor.undo();
        assert!(editor.get_text().contains("[paste #1"));
    }

    #[test]
    fn typed_marker_like_text_is_not_atomic() {
        let mut editor = Editor::new();
        editor.set_text("[paste #99 +5 lines]");
        editor.move_line_start();
        editor.move_right();
        assert_eq!(editor.cursor, 1);
    }

    #[test]
    fn wrap_and_page_match_ts_editor_tests() {
        let mut editor = Editor::new();
        editor.set_text("日本語テスト");
        let lines = editor.render(11);
        let content: Vec<String> = lines[1..lines.len() - 1]
            .iter()
            .map(|line| {
                crate::render::strip_terminal_sequences(line)
                    .trim()
                    .to_string()
            })
            .collect();
        assert_eq!(content, ["日本語テス", "ト"]);
        assert!(lines
            .iter()
            .all(|line| crate::render::visible_width_stripped(line) == 11));

        let mut editor = Editor::new();
        editor.set_text("✅✅✅✅✅✅");
        let lines = editor.render(10);
        assert!(lines
            .iter()
            .all(|line| crate::render::visible_width_stripped(line) <= 10));

        let mut editor = Editor::new();
        editor.set_text("Hello world this is a test of word wrapping functionality");
        let lines = editor.render(40);
        let content: Vec<String> = lines[1..lines.len() - 1]
            .iter()
            .map(|line| {
                crate::render::strip_terminal_sequences(line)
                    .trim()
                    .to_string()
            })
            .filter(|line| !line.is_empty())
            .collect();
        assert!(
            content[0].ends_with("wrapping")
                || content[0].ends_with("of")
                || !content[0].contains("funct")
        );
        assert!(content.iter().skip(1).all(|line| !line.starts_with(' ')));

        let mut editor = Editor::new();
        let big = "line\n".repeat(47).trim_end().to_string();
        editor.handle_paste(&big);
        let lines = editor.render(8);
        assert!(lines
            .iter()
            .all(|line| crate::render::visible_width_stripped(line) <= 8));

        let mut editor = Editor::new();
        editor.set_text(
            (0..20)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        editor.render(10);
        for _ in 0..10 {
            editor.cursor_up();
        }
        let lines = editor.render(10);
        let top = crate::render::strip_terminal_sequences(&lines[0]);
        let bottom = crate::render::strip_terminal_sequences(lines.last().unwrap());
        assert!(top.starts_with("─── ↑"));
        assert!(bottom.starts_with("─── ↓"));
        assert!(lines
            .iter()
            .all(|line| crate::render::visible_width_stripped(line) == 10));

        let mut editor = Editor::new();
        editor.set_terminal_rows(24);
        editor.set_text(
            (0..40)
                .map(|i| format!("row{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        editor.render(20);
        let before = editor.get_cursor();
        editor.page_up();
        let after = editor.get_cursor();
        assert!(after.0 < before.0);
        editor.page_down();
        assert_eq!(editor.get_cursor().0, before.0);
    }

    #[test]
    fn snap_to_paste_marker_when_navigating_down() {
        let mut editor = Editor::new();
        editor.set_text("12345678901234567890\n\nhello ");
        editor.handle_paste(&"x".repeat(2000));
        editor.render(80);
        editor.cursor_up();
        editor.cursor_up();
        editor.move_line_start();
        for _ in 0..10 {
            editor.move_right();
        }
        assert_eq!(editor.get_cursor(), (0, 10));
        editor.cursor_down();
        assert_eq!(editor.get_cursor(), (1, 0));
        editor.cursor_down();
        assert_eq!(editor.get_cursor(), (2, 6));
    }

    #[test]
    fn get_expanded_text_is_literal_and_insert_normalizes() {
        let mut editor = Editor::new();
        let pasted = (1..=11)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.handle_paste(&pasted);
        assert!(editor.get_text().contains("[paste #1"));
        assert_eq!(editor.get_expanded_text(), pasted);

        let mut editor = Editor::new();
        editor.insert_text_at_cursor("a\r\nb\rc");
        assert_eq!(editor.get_text(), "a\nb\nc");
        editor.undo();
        assert_eq!(editor.get_text(), "");
    }
}
