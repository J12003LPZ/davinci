//! Text matching `vendor/pi/packages/tui/src/components/text.ts`.

use crate::ansi::{visible_width, wrap_text_with_ansi};
use crate::render::Component;

type BackgroundFn = Box<dyn Fn(&str) -> String>;

pub fn apply_background_to_line(
    line: &str,
    width: usize,
    bg_fn: impl Fn(&str) -> String,
) -> String {
    let padding_needed = width.saturating_sub(visible_width(line));
    let with_padding = format!("{line}{}", " ".repeat(padding_needed));
    bg_fn(&with_padding)
}

pub struct TuiText {
    text: String,
    padding_x: usize,
    padding_y: usize,
    custom_bg: Option<BackgroundFn>,
}

impl TuiText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            custom_bg: None,
        }
    }

    pub fn with_background(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        bg_fn: impl Fn(&str) -> String + 'static,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            custom_bg: Some(Box::new(bg_fn)),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn set_custom_bg_fn(&mut self, bg_fn: Option<BackgroundFn>) {
        self.custom_bg = bg_fn;
    }
}

impl Default for TuiText {
    fn default() -> Self {
        Self::new("", 1, 1)
    }
}

impl Component for TuiText {
    fn render(&self, width: usize) -> Vec<String> {
        if self.text.trim().is_empty() {
            return Vec::new();
        }
        let normalized = self.text.replace('\t', "   ");
        let padding_x = self.padding_x.min(width.saturating_sub(1) / 2);
        let content_width = width.saturating_sub(padding_x.saturating_mul(2)).max(1);
        let wrapped = wrap_text_with_ansi(&normalized, content_width);
        let left = " ".repeat(padding_x);
        let right = " ".repeat(padding_x);
        let mut content_lines = Vec::new();
        for line in wrapped {
            let line_with_margins = format!("{left}{line}{right}");
            if let Some(bg_fn) = &self.custom_bg {
                content_lines.push(apply_background_to_line(&line_with_margins, width, bg_fn));
            } else {
                let padding_needed = width.saturating_sub(visible_width(&line_with_margins));
                content_lines.push(format!("{line_with_margins}{}", " ".repeat(padding_needed)));
            }
        }
        let empty_line = " ".repeat(width);
        let empty = if let Some(bg_fn) = &self.custom_bg {
            apply_background_to_line(&empty_line, width, bg_fn)
        } else {
            empty_line
        };
        let mut result = Vec::new();
        for _ in 0..self.padding_y {
            result.push(empty.clone());
        }
        result.extend(content_lines);
        for _ in 0..self.padding_y {
            result.push(empty.clone());
        }
        if result.is_empty() {
            vec![String::new()]
        } else {
            result
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::visible_width;

    #[test]
    fn tui_text_padding_wrap_and_empty_match_ts() {
        assert!(TuiText::new("   ", 1, 1).render(20).is_empty());
        let lines = TuiText::new("hello", 1, 1).render(20);
        assert_eq!(lines.len(), 3);
        assert_eq!(visible_width(&lines[1]), 20);
        assert!(lines[1].contains("hello"));
        let tabbed = TuiText::new("a\tb", 0, 0).render(10);
        assert!(tabbed[0].contains("a   b"));
        let bg = TuiText::with_background("hi", 0, 0, |line| format!("[{line}]"));
        assert_eq!(bg.render(4)[0], "[hi  ]");
    }
}
