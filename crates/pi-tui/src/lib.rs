//! Differential terminal renderer matching pi-tui line-diff semantics.

use pi_core::{Message, Role};
use unicode_width::UnicodeWidthStr;

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redraw {
    Full,
    Differential {
        first_changed: usize,
        last_changed: usize,
    },
}

#[derive(Debug, Default)]
pub struct DifferentialRenderer {
    previous: Vec<String>,
    width: usize,
    first_frame: bool,
}

impl DifferentialRenderer {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
            width: 0,
            first_frame: true,
        }
    }

    pub fn frame(&mut self, lines: Vec<String>, width: usize) -> Redraw {
        let width_changed = self.width != width && !self.first_frame;
        let redraw = if self.first_frame || width_changed {
            Redraw::Full
        } else {
            let max = self.previous.len().max(lines.len());
            let mut first = None;
            let mut last = 0;
            for i in 0..max {
                let prev = self.previous.get(i).map(String::as_str).unwrap_or("");
                let next = lines.get(i).map(String::as_str).unwrap_or("");
                if prev != next {
                    if first.is_none() {
                        first = Some(i);
                    }
                    last = i;
                }
            }
            match first {
                None => Redraw::Differential {
                    first_changed: 0,
                    last_changed: 0,
                },
                Some(first_changed) => Redraw::Differential {
                    first_changed,
                    last_changed: last,
                },
            }
        };
        self.previous = lines;
        self.width = width;
        self.first_frame = false;
        redraw
    }

    pub fn previous_lines(&self) -> &[String] {
        &self.previous
    }
}

pub fn visible_width(text: &str) -> usize {
    text.width()
}

pub fn truncate_to_width(text: &str, width: usize) -> String {
    if visible_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

pub struct ConversationView {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub input: String,
}

impl ConversationView {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            messages: Vec::new(),
            input: String::new(),
        }
    }
}

impl Component for ConversationView {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![truncate_to_width(
            &format!("Pi Agent — session {}", self.session_id),
            width,
        )];
        lines.push("-".repeat(width.min(40)));
        for message in &self.messages {
            let role = match message.role {
                Role::User => "User",
                Role::Assistant => "Pi",
                Role::System => "System",
                Role::Tool => "Tool",
            };
            lines.push(truncate_to_width(
                &format!("[{role}] {}", message.content),
                width,
            ));
        }
        lines.push(truncate_to_width(&format!("> {}", self.input), width));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_is_full_redraw() {
        let mut r = DifferentialRenderer::new();
        assert_eq!(r.frame(vec!["a".into()], 80), Redraw::Full);
        assert_eq!(
            r.frame(vec!["a".into(), "b".into()], 80),
            Redraw::Differential {
                first_changed: 1,
                last_changed: 1
            }
        );
    }

    #[test]
    fn width_change_forces_full_redraw() {
        let mut r = DifferentialRenderer::new();
        r.frame(vec!["hello".into()], 80);
        assert_eq!(r.frame(vec!["hello".into()], 40), Redraw::Full);
    }

    #[test]
    fn truncate_respects_cjk_width() {
        assert_eq!(visible_width("水"), 2);
        assert_eq!(truncate_to_width("水水水", 2), "水");
    }
}
