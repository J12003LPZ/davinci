use crate::custom_message::CustomMessage;
use crate::markdown::{render_markdown_with, DEFAULT_CODE_BLOCK_INDENT};
use crate::mermaid::{transform_mermaid, MermaidContext, MermaidMode};
use crate::render::{visible_width, Component};
use crate::themes::Theme;

#[derive(Debug)]
pub struct TranscriptLine {
    pub role: String,
    pub text: String,
    pub custom_lines: Option<Vec<String>>,
    /// Rendered-lines memo. Line text is immutable after push, so a cached
    /// render stays valid until width or a display setting changes.
    cache: std::sync::Mutex<Option<CachedLineRender>>,
}

impl TranscriptLine {
    fn new(role: String, text: String, custom_lines: Option<Vec<String>>) -> Self {
        Self {
            role,
            text,
            custom_lines,
            cache: std::sync::Mutex::new(None),
        }
    }
}

impl Clone for TranscriptLine {
    fn clone(&self) -> Self {
        Self::new(
            self.role.clone(),
            self.text.clone(),
            self.custom_lines.clone(),
        )
    }
}

#[derive(Debug, Clone)]
struct CachedLineRender {
    width: usize,
    tools_expanded: bool,
    mermaid_mode: MermaidMode,
    code_block_indent: String,
    rendered: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub lines: Vec<TranscriptLine>,
    pub scroll: usize,
    pub mermaid_mode: MermaidMode,
    pub hide_thinking_block: bool,
    pub tools_expanded: bool,
    pub renderers: crate::custom_message::MessageRendererRegistry,
    pub extra_transformers: Vec<fn(&str, &str, usize) -> String>,
    pub code_block_indent: String,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            scroll: 0,
            mermaid_mode: MermaidMode::Streaming,
            hide_thinking_block: false,
            tools_expanded: false,
            renderers: crate::custom_message::MessageRendererRegistry::default(),
            extra_transformers: Vec::new(),
            code_block_indent: DEFAULT_CODE_BLOCK_INDENT.into(),
        }
    }
}

impl Transcript {
    pub fn push(&mut self, role: impl Into<String>, text: impl Into<String>) {
        self.lines
            .push(TranscriptLine::new(role.into(), text.into(), None));
    }

    pub fn push_custom(
        &mut self,
        custom_type: impl Into<String>,
        content: impl Into<String>,
        custom_lines: Option<Vec<String>>,
    ) {
        let custom_type = custom_type.into();
        let content = content.into();
        self.lines.push(TranscriptLine::new(
            "custom".into(),
            format!("{custom_type}\n{content}"),
            custom_lines,
        ));
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
            // Re-rendering the whole history each frame made every keystroke
            // pay for a full markdown parse and re-wrap of the session; a
            // line's text never changes after push, so its render is memoized
            // per (width, display settings). `extra_transformers` are fn
            // pointers registered once at startup, so they stay out of the key.
            {
                let cached = line.cache.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(entry) = cached.as_ref() {
                    if entry.width == width
                        && entry.tools_expanded == self.tools_expanded
                        && entry.mermaid_mode == self.mermaid_mode
                        && entry.code_block_indent == self.code_block_indent
                    {
                        out.extend(entry.rendered.iter().cloned());
                        continue;
                    }
                }
            }
            let block_start = out.len();
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
                message.set_expanded(self.tools_expanded);
                message.code_block_indent = self.code_block_indent.clone();
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
                let transformed = apply_extra_transformers(
                    &transformed,
                    &self.extra_transformers,
                    "assistant",
                    width,
                );
                out.extend(render_markdown_with(
                    &transformed,
                    width,
                    &self.code_block_indent,
                ));
            } else if line.role == "user" {
                let transformed =
                    apply_extra_transformers(&line.text, &self.extra_transformers, "user", width);
                out.extend(crate::render::wrap_text(&transformed, width));
            } else {
                out.extend(crate::render::wrap_text(&line.text, width));
            }
            out.push(String::new());
            *line.cache.lock().unwrap_or_else(|error| error.into_inner()) =
                Some(CachedLineRender {
                    width,
                    tools_expanded: self.tools_expanded,
                    mermaid_mode: self.mermaid_mode,
                    code_block_indent: self.code_block_indent.clone(),
                    rendered: out[block_start..].to_vec(),
                });
        }
        out
    }

    fn invalidate(&mut self) {}
}

fn apply_extra_transformers(
    text: &str,
    transformers: &[fn(&str, &str, usize) -> String],
    message_type: &str,
    width: usize,
) -> String {
    transformers
        .iter()
        .fold(text.to_string(), |acc, transform| {
            transform(&acc, message_type, width)
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_render_tracks_width_and_setting_changes() {
        let mut transcript = Transcript::default();
        transcript.push(
            "assistant",
            "# title\n\none two three four five six seven eight nine ten eleven twelve",
        );
        let first = transcript.render(40);
        // Second render is served from the memo and must be identical.
        assert_eq!(first, transcript.render(40));
        // A width change misses the memo and produces a different wrap.
        assert_ne!(first, transcript.render(20));
        // …and going back to the original width matches the first render.
        assert_eq!(first, transcript.render(40));
        // A clone starts with a cold cache and still renders the same lines.
        let cloned = transcript.clone();
        assert_eq!(transcript.render(40), cloned.render(40));
    }
}
