use crate::custom_message::CustomMessage;
use crate::markdown::render_markdown;
use crate::mermaid::{transform_mermaid, MermaidContext, MermaidMode};
use crate::render::{visible_width, Component};
use crate::themes::Theme;

#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub role: String,
    pub text: String,
    pub custom_lines: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub lines: Vec<TranscriptLine>,
    pub scroll: usize,
    pub mermaid_mode: MermaidMode,
    pub hide_thinking_block: bool,
    pub renderers: crate::custom_message::MessageRendererRegistry,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            scroll: 0,
            mermaid_mode: MermaidMode::Streaming,
            hide_thinking_block: false,
            renderers: crate::custom_message::MessageRendererRegistry::default(),
        }
    }
}

impl Transcript {
    pub fn push(&mut self, role: impl Into<String>, text: impl Into<String>) {
        self.lines.push(TranscriptLine {
            role: role.into(),
            text: text.into(),
            custom_lines: None,
        });
    }

    pub fn push_custom(
        &mut self,
        custom_type: impl Into<String>,
        content: impl Into<String>,
        custom_lines: Option<Vec<String>>,
    ) {
        let custom_type = custom_type.into();
        let content = content.into();
        self.lines.push(TranscriptLine {
            role: "custom".into(),
            text: format!("{custom_type}\n{content}"),
            custom_lines,
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
            if self.hide_thinking_block
                && (line.role == "thinking" || line.role == "assistant-thinking")
            {
                continue;
            }
            let heading = format!("{}:", line.role);
            out.push(truncate(&heading, width));
            if line.role == "image" {
                out.push(line.text.clone());
            } else if line.role == "tool" {
                out.extend(crate::render::wrap_text(&line.text, width));
            } else if line.role == "custom" {
                let (custom_type, content) = line
                    .text
                    .split_once('\n')
                    .unwrap_or((line.text.as_str(), ""));
                let mut message = CustomMessage::new(custom_type, content);
                message.renderer_lines = line.custom_lines.clone();
                if message.renderer_lines.is_none() {
                    message.renderer = self.renderers.get(custom_type);
                }
                out.extend(message.render(width));
            } else if line.role == "assistant" {
                let transformed = transform_mermaid(
                    &line.text,
                    MermaidContext {
                        available_width: width,
                        is_streaming: false,
                        message_type: "assistant",
                        mode: self.mermaid_mode,
                    },
                    Some(&Theme {
                        name: "dark".into(),
                        background: String::new(),
                        foreground: String::new(),
                        accent: String::new(),
                    }),
                );
                out.extend(render_markdown(&transformed, width));
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
