//! Differential line renderer matching TypeScript `TuiMainScreen`.

use unicode_width::UnicodeWidthStr;

pub const SYNC_BEGIN: &str = "\u{1b}[?2026h";
pub const SYNC_END: &str = "\u{1b}[?2026l";
pub const SEGMENT_RESET: &str = "\u{1b}[0m\u{1b}]8;;\u{07}";

pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            if chars.peek() == Some(&']') {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\u{07}' {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
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
            if chars.peek() == Some(&'[') {
                out.push('[');
                chars.next();
                for next in chars.by_ref() {
                    out.push(next);
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
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
            for (i, line) in new_lines.iter().enumerate() {
                if i > 0 {
                    out.push_str("\r\n");
                }
                out.push_str(line);
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
}
