//! Custom-message widget matching TS `custom-message.ts`.

use crate::markdown::render_markdown;
use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone)]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: String,
    pub expanded: bool,
    pub output_pad: usize,
}

impl CustomMessage {
    pub fn new(custom_type: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            custom_type: custom_type.into(),
            content: content.into(),
            expanded: false,
            output_pad: 1,
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
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
}

impl Component for CustomMessage {
    fn render(&self, width: usize) -> Vec<String> {
        let theme = Theme {
            name: "dark".into(),
            background: String::new(),
            foreground: String::new(),
            accent: String::new(),
        };
        let pad = " ".repeat(self.output_pad);
        let label = theme.fg(
            "customMessageLabel",
            &format!("\x1b[1m[{}]\x1b[22m", self.custom_type),
        );
        let mut lines = vec![format!("{pad}{}", theme.bg("customMessageBg", &label))];
        let body = if self.expanded {
            self.content.clone()
        } else {
            self.content.lines().next().unwrap_or("").to_string()
        };
        for line in render_markdown(
            &body,
            width.saturating_sub(self.output_pad.saturating_mul(2)),
        ) {
            lines.push(format!(
                "{pad}{}",
                theme.bg("customMessageBg", &theme.fg("customMessageText", &line))
            ));
        }
        lines
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
