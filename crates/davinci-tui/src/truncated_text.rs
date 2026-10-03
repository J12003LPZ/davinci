//! TruncatedText matching `vendor/pi/packages/tui/src/components/truncated-text.ts`.

use crate::ansi::{truncate_to_width, visible_width};
use crate::render::Component;

pub struct TruncatedText {
    text: String,
    padding_x: usize,
    padding_y: usize,
}

impl TruncatedText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
        }
    }
}

impl Component for TruncatedText {
    fn render(&self, width: usize) -> Vec<String> {
        let mut result = Vec::new();
        let empty_line = " ".repeat(width);
        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }
        let available_width = width
            .saturating_sub(self.padding_x.saturating_mul(2))
            .max(1);
        let single_line = self
            .text
            .split_once('\n')
            .map_or(self.text.as_str(), |(first, _)| first);
        let display = truncate_to_width(single_line, available_width, "...", false);
        let left = " ".repeat(self.padding_x);
        let right = " ".repeat(self.padding_x);
        let line_with_padding = format!("{left}{display}{right}");
        let padding_needed = width.saturating_sub(visible_width(&line_with_padding));
        result.push(format!("{line_with_padding}{}", " ".repeat(padding_needed)));
        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }
        result
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::visible_width;

    fn strip_ansi(line: &str) -> String {
        line.replace('\u{1b}', "")
            .chars()
            .filter(|ch| *ch != '[')
            .collect()
    }

    #[test]
    fn truncated_text_matches_ts() {
        let lines = TruncatedText::new("Hello world", 1, 0).render(50);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 50);

        let lines = TruncatedText::new("Hello", 0, 2).render(40);
        assert_eq!(lines.len(), 5);
        for line in &lines {
            assert_eq!(visible_width(line), 40);
        }

        let long =
            "This is a very long piece of text that will definitely exceed the available width";
        let lines = TruncatedText::new(long, 1, 0).render(30);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 30);
        assert!(lines[0].contains("..."));

        let styled = "\x1b[31mHello\x1b[39m \x1b[34mworld\x1b[39m";
        let lines = TruncatedText::new(styled, 1, 0).render(40);
        assert_eq!(visible_width(&lines[0]), 40);
        assert!(lines[0].contains('\u{1b}'));

        let long_styled = format!(
            "\x1b[31m{}\x1b[39m",
            "This is a very long red text that will be truncated"
        );
        let lines = TruncatedText::new(long_styled, 1, 0).render(20);
        assert_eq!(visible_width(&lines[0]), 20);
        assert!(lines[0].contains("\x1b[0m..."));

        let lines = TruncatedText::new("Hello world", 1, 0).render(30);
        assert_eq!(visible_width(&lines[0]), 30);
        assert!(!strip_ansi(&lines[0]).contains("..."));

        let lines = TruncatedText::new("", 1, 0).render(30);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 30);

        let lines = TruncatedText::new("First line\nSecond line\nThird line", 1, 0).render(40);
        assert_eq!(visible_width(&lines[0]), 40);
        assert!(lines[0].contains("First line"));
        assert!(!lines[0].contains("Second line"));
        assert!(!lines[0].contains("Third line"));

        let lines = TruncatedText::new(
            "This is a very long first line that needs truncation\nSecond line",
            1,
            0,
        )
        .render(25);
        assert_eq!(visible_width(&lines[0]), 25);
        assert!(lines[0].contains("..."));
        assert!(!lines[0].contains("Second line"));
    }
}
