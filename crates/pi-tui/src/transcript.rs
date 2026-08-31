//! Conversation transcript.
//!
//! The transcript is the interface (davinci design spec §1): user turns are
//! `> text` in muted, agent turns open with a copper `◆` mark, tool lines are
//! one glyph-prefixed line each, and there is one blank line between blocks.

use crate::custom_message::CustomMessage;
use crate::markdown::{render_markdown_with, DEFAULT_CODE_BLOCK_INDENT};
use crate::mermaid::{transform_mermaid, MermaidContext, MermaidMode};
use crate::render::{visible_width, Component};
use crate::themes::{glyphs, Theme};

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
    theme_name: String,
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
    pub theme: Theme,
    /// Name shown on the agent turn mark (`◆ davinci`).
    pub agent_label: String,
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
            theme: Theme::default(),
            agent_label: "davinci".into(),
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

    /// Style a plain glyph-prefixed tool line (`✓ manus · cargo test  0.42s`).
    /// Text stays plain in storage so theme switches and NO_COLOR re-render.
    fn style_tool_line(&self, line: &str) -> String {
        style_glyph_line(&self.theme, line)
    }
}

/// Color a glyph-prefixed status line by its leading state glyph (spec §4).
pub fn style_glyph_line(theme: &Theme, line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let mut chars = trimmed.chars();
    let glyph = chars.next().unwrap_or(' ');
    let rest = chars.as_str();
    let styled_glyph = match glyph {
        '✓' => theme.fg("success", "✓"),
        '×' => theme.fg("error", "×"),
        '!' => theme.fg("warning", "!"),
        'Δ' => theme.fg("primary", "Δ"),
        '↳' | '⌕' => theme.fg("secondary", &glyph.to_string()),
        '◉' => theme.fg("primary", "◉"),
        '○' => theme.fg("border", "○"),
        '·' => theme.fg("border", "·"),
        _ => return format!("{indent}{}", theme.fg("muted", trimmed)),
    };
    format!("{indent}{styled_glyph}{}", theme.fg("muted", rest))
}

impl Component for Transcript {
    fn render(&self, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        let theme = &self.theme;
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
                        && entry.theme_name == theme.name
                    {
                        out.extend(entry.rendered.iter().cloned());
                        continue;
                    }
                }
            }
            let block_start = out.len();
            match line.role.as_str() {
                "image" => out.push(line.text.clone()),
                // Pre-rendered instrument panels (Grafo, Memoria, Mensura):
                // lines pass through untouched.
                "panel" => out.extend(line.text.lines().map(str::to_string)),
                "tool" => {
                    for tool_line in line.text.lines() {
                        out.push(truncate(&self.style_tool_line(tool_line), width));
                    }
                }
                "custom" => {
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
                }
                "assistant" => {
                    out.push(format!(
                        "{} {}",
                        theme.fg("primary", glyphs::AGENT),
                        theme.fg("primary", &self.agent_label)
                    ));
                    let transformed = transform_mermaid(
                        &line.text,
                        MermaidContext {
                            available_width: width,
                            is_streaming: false,
                            message_type: "assistant",
                            mode: self.mermaid_mode,
                        },
                        Some(theme),
                    );
                    let transformed = apply_extra_transformers(
                        &transformed,
                        &self.extra_transformers,
                        "assistant",
                        width,
                    );
                    // Prose measure never exceeds 74 columns (design spec §6).
                    let measure = width.min(74);
                    out.extend(render_markdown_with(
                        &transformed,
                        measure,
                        &self.code_block_indent,
                    ));
                }
                "user" => {
                    let transformed = apply_extra_transformers(
                        &line.text,
                        &self.extra_transformers,
                        "user",
                        width,
                    );
                    let wrapped = crate::render::wrap_text(&transformed, width.saturating_sub(2));
                    for (index, text) in wrapped.into_iter().enumerate() {
                        if index == 0 {
                            out.push(format!(
                                "{} {}",
                                theme.fg("primary", ">"),
                                theme.fg("muted", &text)
                            ));
                        } else {
                            out.push(format!("  {}", theme.fg("muted", &text)));
                        }
                    }
                }
                "error" => {
                    for (index, text) in
                        crate::render::wrap_text(&line.text, width.saturating_sub(2))
                            .into_iter()
                            .enumerate()
                    {
                        if index == 0 {
                            out.push(format!(
                                "{} {}",
                                theme.fg("error", glyphs::FAILED),
                                theme.fg("error", &text)
                            ));
                        } else {
                            out.push(format!("  {}", theme.fg("error", &text)));
                        }
                    }
                }
                "thinking" | "assistant-thinking" => {
                    for text in crate::render::wrap_text(&line.text, width.saturating_sub(2)) {
                        out.push(format!("  {}", theme.fg("dim", &text)));
                    }
                }
                _ => {
                    // system, notice, session, title, changelog, help, …
                    for (index, text) in
                        crate::render::wrap_text(&line.text, width.saturating_sub(2))
                            .into_iter()
                            .enumerate()
                    {
                        if index == 0 {
                            out.push(format!(
                                "{} {}",
                                theme.fg("border", glyphs::TICK),
                                theme.fg("muted", &text)
                            ));
                        } else {
                            out.push(format!("  {}", theme.fg("muted", &text)));
                        }
                    }
                }
            }
            out.push(String::new());
            *line.cache.lock().unwrap_or_else(|error| error.into_inner()) =
                Some(CachedLineRender {
                    width,
                    tools_expanded: self.tools_expanded,
                    mermaid_mode: self.mermaid_mode,
                    code_block_indent: self.code_block_indent.clone(),
                    theme_name: theme.name.clone(),
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
        crate::ansi::truncate_to_width(line, width, "…", false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_render_tracks_width_and_setting_changes() {
        let _guard = crate::themes::NO_COLOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("NO_COLOR");
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

    #[test]
    fn roles_render_with_glyphs_not_headings() {
        let _guard = crate::themes::NO_COLOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("NO_COLOR", "1");
        let mut transcript = Transcript::default();
        transcript.push("user", "run the tests");
        transcript.push("assistant", "All green.");
        transcript.push("tool", "✓ manus · cargo test  0.42s");
        transcript.push("error", "boom");
        transcript.push("system", "model set");
        let rendered = transcript.render(80).join("\n");
        std::env::remove_var("NO_COLOR");
        assert!(rendered.contains("> run the tests"), "{rendered}");
        assert!(rendered.contains("◆ davinci"), "{rendered}");
        assert!(rendered.contains("✓ manus · cargo test"), "{rendered}");
        assert!(rendered.contains("× boom"), "{rendered}");
        assert!(rendered.contains("· model set"), "{rendered}");
        assert!(!rendered.contains("user:"), "{rendered}");
        assert!(!rendered.contains("assistant:"), "{rendered}");
    }
}
