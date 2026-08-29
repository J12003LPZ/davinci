//! Alt-screen search matching `vendor/pi/packages/tui/src/alt-screen-search.ts`.

use crate::ansi::{strip_terminal_sequences, truncate_to_width, visible_width};
use crate::input::Input;
use crate::render::Component;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AltScreenSearchSegment {
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AltScreenSearchMatch {
    pub segments: Vec<AltScreenSearchSegment>,
}

#[derive(Debug, Clone, Copy)]
struct SearchSourceSpan {
    row: usize,
    start_col: usize,
    end_col: usize,
}

fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_regexp(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if matches!(
            ch,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn build_search_corpus(lines: &[String]) -> (String, Vec<Option<SearchSourceSpan>>) {
    let mut text = String::new();
    let mut source = Vec::new();
    let mut pending_separator = false;
    for (row, line) in lines.iter().enumerate() {
        let stripped = strip_terminal_sequences(line);
        let mut column = 0usize;
        for grapheme in stripped.graphemes(true) {
            let width = visible_width(grapheme);
            if grapheme.chars().all(char::is_whitespace) {
                if !text.is_empty() {
                    pending_separator = true;
                }
                column += width;
                continue;
            }
            if pending_separator {
                text.push(' ');
                source.push(None);
                pending_separator = false;
            }
            let span = SearchSourceSpan {
                row,
                start_col: column,
                end_col: column + width,
            };
            for ch in grapheme.chars() {
                text.push(ch);
                source.push(Some(span));
            }
            column += width;
        }
        if !text.is_empty() {
            pending_separator = true;
        }
    }
    (text, source)
}

/// TS `findAltScreenSearchMatches`.
pub fn find_alt_screen_search_matches(lines: &[String], query: &str) -> Vec<AltScreenSearchMatch> {
    let normalized = normalize_query(query);
    if normalized.is_empty() {
        return Vec::new();
    }
    let (corpus, source) = build_search_corpus(lines);
    let pattern = escape_regexp(&normalized);
    let Ok(regex) = fancy_regex::Regex::new(&format!("(?iu){pattern}")) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for found in regex.find_iter(&corpus).flatten() {
        let start_char = corpus[..found.start()].chars().count();
        let end_char = corpus[..found.end()].chars().count();
        let mut segments: Vec<AltScreenSearchSegment> = Vec::new();
        for index in start_char..end_char {
            let Some(span) = source.get(index).and_then(|item| *item) else {
                continue;
            };
            if let Some(previous) = segments.last_mut() {
                if previous.row == span.row && span.start_col <= previous.end_col {
                    previous.end_col = previous.end_col.max(span.end_col);
                    continue;
                }
            }
            segments.push(AltScreenSearchSegment {
                row: span.row,
                start_col: span.start_col,
                end_col: span.end_col,
            });
        }
        if !segments.is_empty() {
            matches.push(AltScreenSearchMatch { segments });
        }
    }
    matches
}

/// TS `getAltScreenSearchMatchKey`.
pub fn get_alt_screen_search_match_key(match_: &AltScreenSearchMatch) -> String {
    let Some(first) = match_.segments.first() else {
        return String::new();
    };
    let Some(last) = match_.segments.last() else {
        return String::new();
    };
    format!(
        "{}:{}:{}:{}",
        first.row, first.start_col, last.row, last.end_col
    )
}

pub struct AltScreenSearchComponent {
    input: Input,
    result_count: usize,
    result_index: isize,
    pub focused: bool,
    on_query_change: Box<dyn Fn(&str)>,
}

impl AltScreenSearchComponent {
    pub fn new(on_query_change: impl Fn(&str) + 'static) -> Self {
        Self {
            input: Input::new(),
            result_count: 0,
            result_index: -1,
            focused: true,
            on_query_change: Box::new(on_query_change),
        }
    }

    pub fn set_results(&mut self, count: usize, index: isize) {
        self.result_count = count;
        self.result_index = index;
    }

    pub fn query(&self) -> String {
        self.input.value().to_string()
    }
}

impl Component for AltScreenSearchComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let status = if self.result_count == 0 {
            "  0/0".to_string()
        } else {
            format!("  {}/{}", self.result_index.max(0) + 1, self.result_count)
        };
        let status_width = visible_width(&status);
        let query_width = width.saturating_sub(status_width + 2).max(1);
        let query = truncate_to_width(&format!("/{}", self.input.value()), query_width, "", false);
        let pad = width.saturating_sub(visible_width(&query) + status_width);
        vec![format!("{query}{}{status}", " ".repeat(pad))]
    }

    fn handle_input(&mut self, data: &str) {
        self.input.handle_input(data);
        (self.on_query_change)(self.input.value());
    }

    fn invalidate(&mut self) {}

    fn wants_key_release(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_insensitive_spans_across_whitespace() {
        let lines = vec!["Hello   world".into()];
        let matches = find_alt_screen_search_matches(&lines, "hello world");
        assert_eq!(matches.len(), 1);
        assert_eq!(get_alt_screen_search_match_key(&matches[0]), "0:0:0:13");
    }
}
