//! Differential line renderer matching TypeScript `TuiMainScreen`.

use unicode_width::UnicodeWidthStr;

use crate::widgets::CURSOR_MARKER;

pub const SYNC_BEGIN: &str = "\u{1b}[?2026h";
pub const SYNC_END: &str = "\u{1b}[?2026l";
pub const SEGMENT_RESET: &str = "\u{1b}[0m\u{1b}]8;;\u{07}";

fn skip_escape(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) {
    match chars.peek() {
        Some('[') => {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        Some(']') | Some('_') | Some('^') | Some('P') => {
            chars.next();
            for next in chars.by_ref() {
                if next == '\u{07}' {
                    break;
                }
            }
        }
        _ => {}
    }
}

pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            skip_escape(&mut chars);
            continue;
        }
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
}

/// TypeScript `extractCursorPosition`: scan the bottom `height` lines, strip
/// `CURSOR_MARKER`, and return `{row, col}` where `col` is `visibleWidth` before
/// the marker.
pub fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
    if lines.is_empty() || height == 0 {
        return None;
    }
    let viewport_top = lines.len().saturating_sub(height);
    for row in (viewport_top..lines.len()).rev() {
        if let Some(marker_index) = lines[row].find(CURSOR_MARKER) {
            let col = visible_width(&lines[row][..marker_index]);
            let line = &lines[row];
            lines[row] = format!(
                "{}{}",
                &line[..marker_index],
                &line[marker_index + CURSOR_MARKER.len()..]
            );
            return Some((row, col));
        }
    }
    None
}

pub fn hardware_cursor_sequence(row: usize, col: usize) -> String {
    format!("\u{1b}[{};{}H", row + 1, col + 1)
}

pub fn is_image_line(line: &str) -> bool {
    crate::terminal_image::is_image_line(line)
}

/// TypeScript `normalizeTerminalOutput`: decompose Thai/Lao AM and expand
/// visible tabs to 3 spaces without touching tabs inside escape sequences.
pub fn normalize_terminal_output(text: &str) -> String {
    let normalized = text
        .replace('\u{0e33}', "\u{0e4d}\u{0e32}")
        .replace('\u{0eb3}', "\u{0ecd}\u{0eb2}");
    if !normalized.contains('\t') {
        return normalized;
    }
    let mut out = String::new();
    let mut chars = normalized.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            out.push(ch);
            match chars.peek() {
                Some('[') => {
                    out.push('[');
                    chars.next();
                    for next in chars.by_ref() {
                        out.push(next);
                        if matches!(next, 'm' | 'G' | 'K' | 'H' | 'J') {
                            break;
                        }
                    }
                }
                Some(kind @ (']' | '_')) => {
                    out.push(*kind);
                    chars.next();
                    let mut prev_esc = false;
                    for next in chars.by_ref() {
                        out.push(next);
                        if next == '\u{07}' || (prev_esc && next == '\\') {
                            break;
                        }
                        prev_esc = next == '\u{1b}';
                    }
                }
                _ => {}
            }
            continue;
        }
        if ch == '\t' {
            out.push_str("   ");
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn apply_line_resets(lines: &mut [String]) {
    for line in lines {
        if !is_image_line(line) {
            *line = format!("{}{SEGMENT_RESET}", normalize_terminal_output(line));
        }
    }
}

pub fn pad_to_width(text: &str, width: usize) -> String {
    let vis = visible_width(text);
    if vis >= width {
        truncate_visible(text, width)
    } else {
        format!("{text}{}", " ".repeat(width - vis))
    }
}

pub fn truncate_visible(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut acc = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            out.push(ch);
            match chars.peek() {
                Some('[') => {
                    out.push('[');
                    chars.next();
                    for next in chars.by_ref() {
                        out.push(next);
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(kind @ (']' | '_' | '^' | 'P')) => {
                    out.push(*kind);
                    chars.next();
                    for next in chars.by_ref() {
                        out.push(next);
                        if next == '\u{07}' {
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + w > width {
            break;
        }
        out.push(ch);
        acc += w;
    }
    out
}

/// TypeScript `truncateToWidth` with default ellipsis `...`.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str) -> String {
    if max_width == 0 {
        return String::new();
    }
    if visible_width(text) <= max_width {
        return text.to_string();
    }
    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        return truncate_visible(ellipsis, max_width);
    }
    let target = max_width - ellipsis_width;
    let prefix = truncate_visible(text, target);
    if text.contains('\u{1b}') {
        format!("{prefix}\u{1b}[0m{ellipsis}\u{1b}[0m")
    } else {
        format!("{prefix}{ellipsis}")
    }
}

/// Composite overlay content into a terminal line at a fixed column (TS `compositeTuiLine`).
pub fn composite_tui_line(
    base_line: &str,
    overlay_line: &str,
    start_col: usize,
    overlay_width: usize,
    total_width: usize,
) -> String {
    let before = pad_to_width("", start_col.min(total_width));
    let overlay = pad_to_width(
        overlay_line,
        overlay_width.min(total_width.saturating_sub(start_col)),
    );
    let used = visible_width(&before) + visible_width(&overlay);
    let after_width = total_width.saturating_sub(used);
    let after = if after_width == 0 {
        String::new()
    } else {
        let base_tail = {
            let mut skipped = 0;
            let mut rest = String::new();
            let mut chars = base_line.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '\u{1b}' {
                    if chars.peek() == Some(&'[') {
                        chars.next();
                        for next in chars.by_ref() {
                            if next.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    continue;
                }
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if skipped >= start_col + overlay_width {
                    rest.push(ch);
                }
                skipped += w;
            }
            rest
        };
        pad_to_width(&base_tail, after_width)
    };
    let result = format!("{before}{SEGMENT_RESET}{overlay}{SEGMENT_RESET}{after}");
    if visible_width(&result) <= total_width {
        result
    } else {
        truncate_visible(&result, total_width)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiffScreen {
    pub previous_lines: Vec<String>,
    pub columns: usize,
    pub rows: usize,
    pub full_redraws: u32,
}

impl DiffScreen {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            previous_lines: Vec::new(),
            columns,
            rows,
            full_redraws: 0,
        }
    }

    /// Emit synchronized output for a new frame. Only rewritten lines are included.
    pub fn render(&mut self, new_lines: &[String], force: bool) -> String {
        let width_changed = self.columns > 0
            && new_lines
                .iter()
                .any(|l| UnicodeWidthStr::width(l.as_str()) > self.columns && self.columns != 0);
        let _ = width_changed;
        if force || self.previous_lines.is_empty() {
            self.full_redraws += 1;
            let mut out = String::from(SYNC_BEGIN);
            if force && !self.previous_lines.is_empty() {
                out.push_str("\u{1b}[H\u{1b}[2J");
            }
            let mut i = 0;
            while i < new_lines.len() {
                if i > 0 {
                    out.push_str("\r\n");
                }
                let reserved = crate::terminal_image::kitty_image_reserved_rows(
                    new_lines,
                    i,
                    Some(new_lines.len().saturating_sub(1)),
                );
                if reserved > 1 && reserved <= self.rows.max(1) {
                    out.push_str(&crate::terminal_image::emit_reserved_image_block(
                        &new_lines[i],
                        reserved,
                    ));
                    i += reserved;
                    continue;
                }
                out.push_str(&new_lines[i]);
                i += 1;
            }
            out.push_str(SYNC_END);
            self.previous_lines = new_lines.to_vec();
            return out;
        }

        let mut first = None;
        let mut last = 0usize;
        let max = new_lines.len().max(self.previous_lines.len());
        for i in 0..max {
            let old = self.previous_lines.get(i).map(String::as_str).unwrap_or("");
            let new = new_lines.get(i).map(String::as_str).unwrap_or("");
            if old != new {
                if first.is_none() {
                    first = Some(i);
                }
                last = i;
            }
        }
        let Some(first_changed) = first else {
            return String::new();
        };
        let mut out = String::from(SYNC_BEGIN);
        for i in first_changed..=last.min(new_lines.len().saturating_sub(1)) {
            if i >= new_lines.len() {
                continue;
            }
            out.push_str(&format!("\u{1b}[{};1H\u{1b}[2K", i + 1));
            out.push_str(&new_lines[i]);
        }
        if self.previous_lines.len() > new_lines.len() {
            for i in new_lines.len()..self.previous_lines.len() {
                out.push_str(&format!("\u{1b}[{};1H\u{1b}[2K", i + 1));
            }
        }
        out.push_str(SYNC_END);
        self.previous_lines = new_lines.to_vec();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_truncates_to_declared_width() {
        let line = composite_tui_line("", &"X".repeat(100), 0, 20, 80);
        assert!(visible_width(&line) <= 80);
        assert!(visible_width(&line) >= 20);
    }

    #[test]
    fn diff_rewrites_only_changed_lines() {
        let mut screen = DiffScreen::new(80, 24);
        let first = screen.render(&["a".into(), "b".into()], false);
        assert!(first.contains(SYNC_BEGIN));
        assert_eq!(screen.full_redraws, 1);
        let second = screen.render(&["a".into(), "c".into()], false);
        assert!(second.contains("c"));
        assert!(!second.contains("\u{1b}[1;1H"));
        assert!(second.contains("\u{1b}[2;1H"));
        let third = screen.render(&["a".into(), "c".into()], false);
        assert!(third.is_empty());
    }

    #[test]
    fn extract_cursor_strips_marker_and_uses_visible_width() {
        let marker = crate::widgets::CURSOR_MARKER;
        let mut lines = vec![
            "status".into(),
            format!("hello{marker}\x1b[7m \x1b[27mworld"),
            "footer".into(),
        ];
        let pos = extract_cursor_position(&mut lines, 3).expect("marker");
        assert_eq!(pos, (1, 5));
        assert!(!lines[1].contains(marker));
        assert!(lines[1].contains("hello"));
        assert_eq!(visible_width(marker), 0);
        let seq = hardware_cursor_sequence(pos.0, pos.1);
        assert_eq!(seq, "\u{1b}[2;6H");
        let mut above = vec![format!("{marker}hidden"), "visible".into()];
        assert!(extract_cursor_position(&mut above, 1).is_none());
        assert!(above[0].contains(marker));
        assert_eq!(normalize_terminal_output("a\tb"), "a   b");
        assert_eq!(normalize_terminal_output("\u{0e33}"), "\u{0e4d}\u{0e32}");
        assert!(!is_image_line("hello"));
        assert!(is_image_line("\u{1b}_Gabc"));
        let mut reset = vec!["hi".into()];
        apply_line_resets(&mut reset);
        assert!(reset[0].ends_with(SEGMENT_RESET));
        assert!(reset[0].starts_with("hi"));
    }
}
