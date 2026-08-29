//! Live tool execution cards matching
//! `vendor/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts`.

use crate::render::{visible_width, Component};
use serde_json::Value;

pub const FALLBACK_PREVIEW_LINES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCardState {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone)]
pub struct ToolCard {
    pub tool_name: String,
    pub tool_call_id: String,
    pub args: Value,
    pub output: String,
    pub state: ToolCardState,
    pub expanded: bool,
}

impl ToolCard {
    pub fn start(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        args: Value,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            args,
            output: String::new(),
            state: ToolCardState::Pending,
            expanded: false,
        }
    }

    pub fn finish(&mut self, result: &Value, is_error: bool) {
        self.output = text_output(result);
        self.state = if is_error {
            ToolCardState::Error
        } else {
            ToolCardState::Done
        };
    }

    pub fn update_partial(&mut self, partial: &Value) {
        self.output = text_output(partial);
        self.state = ToolCardState::Running;
    }

    /// TS `formatToolExecution`: bold tool name, pretty-printed args, then output.
    pub fn format_tool_execution(&self) -> String {
        let mut text = self.tool_name.clone();
        let content = serde_json::to_string_pretty(&self.args).unwrap_or_default();
        if !content.is_empty() && content != "null" {
            text.push_str("\n\n");
            text.push_str(&content);
        }
        if !self.output.is_empty() {
            text.push('\n');
            text.push_str(&self.output);
        }
        text
    }

    pub fn status_label(&self) -> &'static str {
        match self.state {
            ToolCardState::Pending => "pending",
            ToolCardState::Running => "running",
            ToolCardState::Done => "done",
            ToolCardState::Error => "error",
        }
    }

    pub fn image_payloads(&self) -> Vec<(String, String)> {
        image_payloads_from_value(&self.args)
            .into_iter()
            .chain(image_payloads_from_json_text(&self.output))
            .collect()
    }
}

fn text_output(value: &Value) -> String {
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(items) = value.get("content").and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
    }
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn image_payloads_from_json_text(text: &str) -> Vec<(String, String)> {
    serde_json::from_str(text)
        .ok()
        .map(|value| image_payloads_from_value(&value))
        .unwrap_or_default()
}

fn image_payloads_from_value(value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_images(value, &mut out);
    out
}

fn collect_images(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            if let (Some(data), Some(mime)) = (
                map.get("data").and_then(Value::as_str),
                map.get("mimeType")
                    .or_else(|| map.get("mime_type"))
                    .and_then(Value::as_str),
            ) {
                if mime.starts_with("image/") {
                    out.push((data.to_string(), mime.to_string()));
                }
            }
            for nested in map.values() {
                collect_images(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_images(item, out);
            }
        }
        _ => {}
    }
}

impl Component for ToolCard {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2);
        let header = format!(" {} [{}] ", self.tool_name, self.status_label());
        let mut lines = vec![
            format!("┌{}┐", "─".repeat(inner)),
            format!("│{header:<inner$}│"),
        ];
        let body = self.format_tool_execution();
        let mut body_lines: Vec<String> = body.lines().map(str::to_string).collect();
        if !self.expanded && body_lines.len() > FALLBACK_PREVIEW_LINES {
            body_lines.truncate(FALLBACK_PREVIEW_LINES);
            body_lines.push("…".into());
        }
        for line in body_lines {
            let mut clipped = line;
            while visible_width(&clipped) > inner.saturating_sub(2) {
                clipped.pop();
            }
            let mut padded = format!("│ {clipped}");
            while visible_width(&padded) < width.saturating_sub(1) {
                padded.push(' ');
            }
            padded.push('│');
            lines.push(padded);
        }
        lines.push(format!("└{}┘", "─".repeat(inner)));
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if data == " " {
            self.expanded = !self.expanded;
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_ts_and_previews_ten_lines() {
        let mut card = ToolCard::start("bash", "call-1", serde_json::json!({"command": "ls"}));
        assert!(card.format_tool_execution().contains("bash"));
        assert!(card.format_tool_execution().contains("command"));
        card.finish(&serde_json::json!({"content": "ok"}), false);
        assert_eq!(card.state, ToolCardState::Done);
        assert!(card.render(40).iter().any(|line| line.contains("bash")));
        let mut long = ToolCard::start("read", "c2", serde_json::json!({}));
        long.output = (0..20)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = long.render(40);
        assert!(rendered.iter().any(|line| line.contains('…')));
        long.expanded = true;
        assert!(!long.render(40).iter().any(|line| line.contains('…')));
    }
}
