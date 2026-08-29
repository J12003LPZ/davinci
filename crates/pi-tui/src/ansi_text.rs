//! ANSI/grapheme helpers matching TypeScript `packages/tui/src/utils.ts`.

use crate::diff::visible_width;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy)]
pub struct AnsiCode<'a> {
    pub code: &'a str,
    pub length: usize,
}

pub fn extract_ansi_code(text: &str, pos: usize) -> Option<AnsiCode<'_>> {
    let bytes = text.as_bytes();
    if pos >= bytes.len() || bytes[pos] != 0x1b {
        return None;
    }
    let rest = &text[pos..];
    let mut chars = rest.char_indices().skip(1);
    let (_, next) = chars.next()?;
    match next {
        '[' => {
            for (idx, ch) in chars {
                if ch.is_ascii_alphabetic() {
                    return Some(AnsiCode {
                        code: &rest[..=idx],
                        length: idx + ch.len_utf8(),
                    });
                }
            }
            None
        }
        ']' => {
            for (idx, ch) in chars {
                if ch == '\u{07}' {
                    return Some(AnsiCode {
                        code: &rest[..=idx],
                        length: idx + 1,
                    });
                }
                if ch == '\\' && idx >= 1 && rest.as_bytes().get(idx - 1) == Some(&0x1b) {
                    return Some(AnsiCode {
                        code: &rest[..=idx],
                        length: idx + 1,
                    });
                }
            }
            None
        }
        '_' | '^' | 'P' => {
            for (idx, ch) in chars {
                if ch == '\u{07}' {
                    return Some(AnsiCode {
                        code: &rest[..=idx],
                        length: idx + 1,
                    });
                }
                if ch == '\\' && idx >= 1 && rest.as_bytes().get(idx - 1) == Some(&0x1b) {
                    return Some(AnsiCode {
                        code: &rest[..=idx],
                        length: idx + 1,
                    });
                }
            }
            None
        }
        _ => None,
    }
}

pub fn strip_terminal_sequences(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_string();
    }
    let mut result = String::new();
    let mut i = 0;
    while i < text.len() {
        if let Some(ansi) = extract_ansi_code(text, i) {
            i += ansi.length;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

fn is_combining(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{FE0F}'
            | '\u{200D}'
    )
}

pub fn next_grapheme(text: &str) -> Option<(&str, &str)> {
    let mut chars = text.char_indices();
    let (start, first) = chars.next()?;
    let mut end = start + first.len_utf8();
    for (idx, ch) in chars {
        if is_combining(ch) {
            end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    Some((&text[..end], &text[end..]))
}

pub fn grapheme_width(segment: &str) -> usize {
    if segment == "\t" {
        return 3;
    }
    segment
        .chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphemeCellRange {
    pub start: usize,
    pub end: usize,
}

pub fn get_grapheme_cell_range(line: &str, column: usize) -> Option<GraphemeCellRange> {
    let mut current_col = 0usize;
    let mut i = 0usize;
    while i < line.len() {
        if let Some(ansi) = extract_ansi_code(line, i) {
            i += ansi.length;
            continue;
        }
        let mut text_end = i;
        while text_end < line.len() && extract_ansi_code(line, text_end).is_none() {
            let ch = line[text_end..].chars().next().unwrap();
            text_end += ch.len_utf8();
        }
        let mut rest = &line[i..text_end];
        while !rest.is_empty() {
            let (segment, next) = next_grapheme(rest)?;
            let width = grapheme_width(segment);
            if width > 0 && column >= current_col && column < current_col + width {
                return Some(GraphemeCellRange {
                    start: current_col,
                    end: current_col + width,
                });
            }
            current_col += width;
            rest = next;
        }
        i = text_end;
    }
    None
}

pub fn get_osc8_link_at_column(line: &str, column: usize) -> Option<String> {
    let mut active_url: Option<String> = None;
    let mut current_col = 0usize;
    let mut i = 0usize;
    while i < line.len() {
        if let Some(ansi) = extract_ansi_code(line, i) {
            if let Some(url) = parse_osc8(ansi.code) {
                active_url = if url.is_empty() { None } else { Some(url) };
            }
            i += ansi.length;
            continue;
        }
        let mut text_end = i;
        while text_end < line.len() && extract_ansi_code(line, text_end).is_none() {
            let ch = line[text_end..].chars().next().unwrap();
            text_end += ch.len_utf8();
        }
        let mut rest = &line[i..text_end];
        while !rest.is_empty() {
            let (segment, next) = next_grapheme(rest)?;
            let width = if segment == "\t" {
                3
            } else {
                grapheme_width(segment)
            };
            if column >= current_col && column < current_col + width {
                return active_url;
            }
            current_col += width;
            rest = next;
        }
        i = text_end;
    }
    None
}

fn parse_osc8(code: &str) -> Option<String> {
    let rest = code.strip_prefix("\u{1b}]8;")?;
    let rest = rest
        .strip_suffix('\u{07}')
        .or_else(|| rest.strip_suffix("\u{1b}\\"))?;
    let url = rest.split_once(';')?.1;
    Some(url.to_string())
}

pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    if length == 0 {
        return String::new();
    }
    let end_col = start_col + length;
    let mut result = String::new();
    let mut current_col = 0usize;
    let mut i = 0usize;
    let mut pending_ansi = String::new();
    while i < line.len() {
        if let Some(ansi) = extract_ansi_code(line, i) {
            if current_col >= start_col && current_col < end_col {
                result.push_str(ansi.code);
            } else if current_col < start_col {
                pending_ansi.push_str(ansi.code);
            }
            i += ansi.length;
            continue;
        }
        let mut text_end = i;
        while text_end < line.len() && extract_ansi_code(line, text_end).is_none() {
            let ch = line[text_end..].chars().next().unwrap();
            text_end += ch.len_utf8();
        }
        let mut rest = &line[i..text_end];
        while !rest.is_empty() {
            let Some((segment, next)) = next_grapheme(rest) else {
                break;
            };
            let width = grapheme_width(segment);
            let in_range = current_col >= start_col && current_col < end_col;
            let fits = !strict || current_col + width <= end_col;
            if in_range && fits {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push_str(segment);
            }
            current_col += width;
            rest = next;
            if current_col >= end_col {
                break;
            }
        }
        i = text_end;
        if current_col >= end_col {
            break;
        }
    }
    let _ = visible_width(&result);
    result
}

pub fn strip_osc133_zone_prefix(line: &str) -> String {
    let mut rest = line;
    loop {
        let Some(after) = rest.strip_prefix("\u{1b}]133;") else {
            break;
        };
        if after.is_empty() || !matches!(after.as_bytes()[0], b'A' | b'B' | b'C') {
            break;
        }
        if let Some(idx) = after.find('\u{07}') {
            rest = &after[idx + 1..];
            continue;
        }
        if let Some(idx) = after.find("\u{1b}\\") {
            rest = &after[idx + 2..];
            continue;
        }
        break;
    }
    rest.to_string()
}

pub fn is_osc133_prompt_start(line: &str) -> bool {
    line.starts_with("\u{1b}]133;A\u{07}") || line.starts_with("\u{1b}]133;A\u{1b}\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_segment_reset() {
        let line = format!("line 7{}", crate::diff::SEGMENT_RESET);
        assert_eq!(strip_terminal_sequences(&line), "line 7");
    }
}
