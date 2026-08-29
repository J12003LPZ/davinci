//! Custom-message widget matching TS `custom-message.ts`.

use std::collections::HashMap;

use crate::markdown::{render_markdown_with, DEFAULT_CODE_BLOCK_INDENT};
use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRenderOptions {
    pub expanded: bool,
    pub output_pad: usize,
}

pub type MessageRenderer = fn(&CustomMessage, &MessageRenderOptions, &Theme) -> Option<Vec<String>>;

#[derive(Debug, Clone, Default)]
pub struct MessageRendererRegistry {
    native: HashMap<String, MessageRenderer>,
    lines: HashMap<String, Vec<String>>,
}

impl MessageRendererRegistry {
    pub fn register(&mut self, custom_type: impl Into<String>, renderer: MessageRenderer) {
        self.native.insert(custom_type.into(), renderer);
    }

    pub fn register_lines(&mut self, custom_type: impl Into<String>, lines: Vec<String>) {
        self.lines.insert(custom_type.into(), lines);
    }

    pub fn get(&self, custom_type: &str) -> Option<MessageRenderer> {
        self.native.get(custom_type).copied()
    }

    pub fn render(
        &self,
        message: &CustomMessage,
        options: &MessageRenderOptions,
        theme: &Theme,
    ) -> Option<Vec<String>> {
        if let Some(renderer) = self.native.get(&message.custom_type) {
            return renderer(message, options, theme);
        }
        self.lines.get(&message.custom_type).cloned()
    }
}

#[derive(Debug, Clone)]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: String,
    pub expanded: bool,
    pub output_pad: usize,
    pub renderer: Option<MessageRenderer>,
    pub renderer_lines: Option<Vec<String>>,
    pub theme: Theme,
    pub code_block_indent: String,
}

impl CustomMessage {
    pub fn new(custom_type: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            custom_type: custom_type.into(),
            content: content.into(),
            expanded: false,
            output_pad: 1,
            renderer: None,
            renderer_lines: None,
            theme: Theme {
                name: "dark".into(),
                background: String::new(),
                foreground: String::new(),
                accent: String::new(),
            },
            code_block_indent: DEFAULT_CODE_BLOCK_INDENT.into(),
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    pub fn set_output_pad(&mut self, output_pad: usize) {
        self.output_pad = output_pad;
    }

    pub fn options(&self) -> MessageRenderOptions {
        MessageRenderOptions {
            expanded: self.expanded,
            output_pad: self.output_pad,
        }
    }

    pub fn text_content(content: &serde_json::Value) -> String {
        if let Some(text) = content.as_str() {
            return text.to_string();
        }
        if let Some(items) = content.as_array() {
            return items
                .iter()
                .filter_map(|item| {
                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                        item.get("text")
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        String::new()
    }

    fn default_lines(&self, width: usize) -> Vec<String> {
        let pad = " ".repeat(self.output_pad);
        let label = self.theme.fg(
            "customMessageLabel",
            &format!("\x1b[1m[{}]\x1b[22m", self.custom_type),
        );
        let mut lines = vec![format!("{pad}{}", self.theme.bg("customMessageBg", &label))];
        let body = if self.expanded {
            self.content.clone()
        } else {
            self.content.lines().next().unwrap_or("").to_string()
        };
        for line in render_markdown_with(
            &body,
            width.saturating_sub(self.output_pad.saturating_mul(2)),
            &self.code_block_indent,
        ) {
            lines.push(format!(
                "{pad}{}",
                self.theme.bg(
                    "customMessageBg",
                    &self.theme.fg("customMessageText", &line)
                )
            ));
        }
        lines
    }
}

impl Component for CustomMessage {
    fn render(&self, width: usize) -> Vec<String> {
        let options = self.options();
        if let Some(renderer) = self.renderer {
            if let Some(lines) = renderer(self, &options, &self.theme) {
                return lines;
            }
        }
        if let Some(lines) = &self.renderer_lines {
            return lines.clone();
        }
        self.default_lines(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn padded_renderer(
        _message: &CustomMessage,
        options: &MessageRenderOptions,
        _theme: &Theme,
    ) -> Option<Vec<String>> {
        let pad = " ".repeat(options.output_pad);
        Some(vec![format!("{pad}custom")])
    }

    #[test]
    fn renders_purple_box_label_and_collapse() {
        let mut message = CustomMessage::new("note", "hello **world**\nsecond line");
        let collapsed = message.render(40).join("\n");
        assert!(collapsed.contains("[note]"));
        assert!(collapsed.contains("hello"));
        assert!(!collapsed.contains("second line"));
        message.set_expanded(true);
        let expanded = message.render(40).join("\n");
        assert!(expanded.contains("second line"));
        assert_eq!(
            CustomMessage::text_content(&serde_json::json!([
                {"type": "text", "text": "a"},
                {"type": "image"},
                {"type": "text", "text": "b"}
            ])),
            "a\nb"
        );
    }

    #[test]
    fn custom_renderer_receives_output_pad_like_ts() {
        let mut message = CustomMessage::new("test", "custom");
        message.renderer = Some(padded_renderer);
        message.set_output_pad(1);
        assert!(message
            .render(40)
            .iter()
            .any(|line| line.starts_with(' ') && line.contains("custom")));
        message.set_output_pad(0);
        assert!(message
            .render(40)
            .iter()
            .any(|line| line.starts_with("custom")));
        let mut registry = MessageRendererRegistry::default();
        registry.register("status-update", padded_renderer);
        assert!(registry.get("status-update").is_some());
        assert!(registry.get("missing").is_none());
    }
}
