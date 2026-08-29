use crate::markdown::render_markdown;
use crate::render::{visible_width, Component};

#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub lines: Vec<TranscriptLine>,
    pub scroll: usize,
}

impl Transcript {
    pub fn push(&mut self, role: impl Into<String>, text: impl Into<String>) {
        self.lines.push(TranscriptLine {
            role: role.into(),
            text: text.into(),
        });
    }

    pub fn scroll_by(&mut self, delta: i32, viewport: usize) {
        let max = self.lines.len().saturating_sub(viewport.max(1));
        if delta < 0 {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.scroll = (self.scroll + delta as usize).min(max);
        }
    }
}

impl Component for Transcript {
    fn render(&self, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        for line in self.lines.iter().skip(self.scroll) {
            let heading = format!("{}:", line.role);
            out.push(truncate(&heading, width));
            if line.role == "assistant" {
                out.extend(render_markdown(&line.text, width));
            } else {
                out.extend(crate::render::wrap_text(&line.text, width));
            }
            out.push(String::new());
        }
        out
    }

    fn invalidate(&mut self) {}
}

fn truncate(line: &str, width: usize) -> String {
    if visible_width(line) <= width {
        line.to_string()
    } else {
        let mut out = String::new();
        for ch in line.chars() {
            if visible_width(&out) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
                > width.saturating_sub(1)
            {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }
}
