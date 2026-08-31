//! Live tool execution cards.
//!
//! Collapsed form is one line per the davinci design spec §6 ToolCall:
//! `glyph  instrument · verb  target  duration`, no box. Failures expand to
//! at most four indented lines. `ctrl+o` (tools expanded) shows the full
//! call/output dump that the TS `formatToolExecution` produced.

use crate::render::{visible_width, Component};
use serde_json::Value;
use std::time::Instant;

pub const FALLBACK_PREVIEW_LINES: usize = 10;
const ERROR_DETAIL_LINES: usize = 4;

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
    pub call_lines: Vec<String>,
    pub result_lines: Vec<String>,
    pub render_shell: Option<String>,
    pub started_at: Instant,
    pub duration_ms: Option<u64>,
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
            call_lines: Vec::new(),
            result_lines: Vec::new(),
            render_shell: None,
            started_at: Instant::now(),
            duration_ms: None,
        }
    }

    pub fn finish(&mut self, result: &Value, is_error: bool) {
        self.output = text_output(result);
        self.duration_ms = Some(self.started_at.elapsed().as_millis() as u64);
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
    /// Extension `renderCall` / `renderResult` lines replace the default dump.
    pub fn format_tool_execution(&self) -> String {
        if !self.call_lines.is_empty() || !self.result_lines.is_empty() {
            let mut text = if self.call_lines.is_empty() {
                self.tool_name.clone()
            } else {
                self.call_lines.join("\n")
            };
            if !self.result_lines.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&self.result_lines.join("\n"));
            } else if !self.output.is_empty() {
                text.push('\n');
                text.push_str(&self.output);
            }
            return text;
        }
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

    /// Plain glyph-prefixed transcript block: one summary line, then failure
    /// detail. Styling happens at render time from the glyph (theme-safe).
    pub fn summary_block(&self) -> String {
        let mut lines = vec![summary_line(
            &self.tool_name,
            &self.args,
            &self.state,
            self.duration_ms,
        )];
        if self.state == ToolCardState::Error {
            for detail in self
                .output
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(ERROR_DETAIL_LINES)
            {
                // By character, not by byte: `String::truncate` panics when
                // the index lands inside a multi-byte character, and tool
                // failures carry whatever the tool printed.
                let detail: String = detail.trim_end().chars().take(200).collect();
                lines.push(format!("  ! {detail}"));
            }
        }
        lines.join("\n")
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

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

fn format_duration(duration_ms: Option<u64>) -> String {
    match duration_ms {
        Some(ms) if ms >= 100 => format!("  {:.2}s", ms as f64 / 1000.0),
        _ => String::new(),
    }
}

fn state_glyph(state: &ToolCardState) -> &'static str {
    match state {
        ToolCardState::Pending | ToolCardState::Running => "◉",
        ToolCardState::Done => "✓",
        ToolCardState::Error => "×",
    }
}

/// `glyph instrument · verb target duration` (plain text; styled at render).
pub fn summary_line(
    tool_name: &str,
    args: &Value,
    state: &ToolCardState,
    duration_ms: Option<u64>,
) -> String {
    let glyph = state_glyph(state);
    let duration = format_duration(duration_ms);
    match tool_name {
        "read" => {
            let path = arg_str(args, "path").unwrap_or("");
            format!("↳ read {path}")
        }
        "ls" => {
            let path = arg_str(args, "path").unwrap_or(".");
            format!("↳ ls {path}")
        }
        "grep" | "find" => {
            let pattern = arg_str(args, "pattern")
                .or_else(|| arg_str(args, "glob"))
                .unwrap_or("");
            format!("⌕ {tool_name} \"{}\"", first_line(pattern, 48))
        }
        "bash" | "powershell" => {
            let command = arg_str(args, "command").unwrap_or("");
            format!("{glyph} manus · {}{duration}", first_line(command, 64))
        }
        "write" => {
            let path = arg_str(args, "path").unwrap_or("");
            let added = arg_str(args, "content")
                .map(|content| content.lines().count())
                .unwrap_or(0);
            format!("Δ {path} +{added}")
        }
        "edit" => {
            let path = arg_str(args, "path").unwrap_or("");
            let removed = arg_str(args, "oldText")
                .map(|text| text.lines().count())
                .unwrap_or(0);
            let added = arg_str(args, "newText")
                .map(|text| text.lines().count())
                .unwrap_or(0);
            format!("Δ {path} +{added} -{removed}")
        }
        "memory_search" => {
            let query = arg_str(args, "query").unwrap_or("");
            format!("⌕ memoria · {}", first_line(query, 56))
        }
        name if name.starts_with("graph") => {
            let detail = arg_str(args, "goal")
                .or_else(|| arg_str(args, "run_id"))
                .unwrap_or("");
            format!(
                "{glyph} grafo · {name} {}{duration}",
                first_line(detail, 40)
            )
        }
        name if name.starts_with("sec_") => {
            format!("{glyph} scan · {name}{duration}")
        }
        name => {
            let detail = args
                .as_object()
                .and_then(|map| map.values().find_map(Value::as_str))
                .map(|value| first_line(value, 40))
                .unwrap_or_default();
            if detail.is_empty() {
                format!("{glyph} {name}{duration}")
            } else {
                format!("{glyph} {name} · {detail}{duration}")
            }
        }
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
        let mut lines: Vec<String> = self
            .summary_block()
            .lines()
            .map(|line| {
                let mut clipped = line.to_string();
                while visible_width(&clipped) > width {
                    clipped.pop();
                }
                clipped
            })
            .collect();
        if self.expanded {
            let body = self.format_tool_execution();
            let mut body_lines: Vec<String> = body.lines().map(str::to_string).collect();
            if body_lines.len() > FALLBACK_PREVIEW_LINES * 4 {
                body_lines.truncate(FALLBACK_PREVIEW_LINES * 4);
                body_lines.push("…".into());
            }
            for line in body_lines {
                let mut clipped = format!("    {line}");
                while visible_width(&clipped) > width {
                    clipped.pop();
                }
                lines.push(clipped);
            }
        }
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
    fn summary_lines_use_instrument_glyphs() {
        let card = ToolCard::start("bash", "c1", serde_json::json!({"command": "cargo test"}));
        assert_eq!(card.summary_block(), "◉ manus · cargo test");
        let mut done = card.clone();
        done.finish(&serde_json::json!({"content": "ok"}), false);
        done.duration_ms = Some(420);
        assert_eq!(done.summary_block(), "✓ manus · cargo test  0.42s");
        let read = ToolCard::start("read", "c2", serde_json::json!({"path": "src/lib.rs"}));
        assert_eq!(read.summary_block(), "↳ read src/lib.rs");
        let edit = ToolCard::start(
            "edit",
            "c3",
            serde_json::json!({"path": "a.rs", "oldText": "x\ny", "newText": "z"}),
        );
        assert_eq!(edit.summary_block(), "Δ a.rs +1 -2");
    }

    #[test]
    fn failures_keep_at_most_four_detail_lines() {
        let mut card = ToolCard::start("bash", "c1", serde_json::json!({"command": "cargo test"}));
        card.finish(
            &serde_json::json!({"content": "e1\ne2\ne3\ne4\ne5\ne6"}),
            true,
        );
        card.duration_ms = None;
        let block = card.summary_block();
        let detail_lines = block.lines().filter(|l| l.starts_with("  !")).count();
        assert_eq!(detail_lines, 4, "{block}");
        assert!(block.starts_with("× manus · cargo test"));
    }

    #[test]
    fn a_long_failure_line_of_multibyte_text_is_clipped_not_panicked_on() {
        // A tool prints whatever it prints. Clipping this by byte index lands
        // inside a character and panics the render.
        let mut card = ToolCard::start("bash", "c1", serde_json::json!({"command": "build"}));
        card.finish(&serde_json::json!({"content": "é".repeat(400)}), true);
        let block = card.summary_block();
        let detail = block
            .lines()
            .find(|line| line.starts_with("  !"))
            .expect("a detail line");
        assert_eq!(detail.chars().filter(|ch| *ch == 'é').count(), 200);
    }

    #[test]
    fn expanded_render_appends_full_dump() {
        let mut card = ToolCard::start("bash", "call-1", serde_json::json!({"command": "ls"}));
        card.finish(&serde_json::json!({"content": "ok"}), false);
        card.expanded = true;
        let rendered = card.render(60).join("\n");
        assert!(rendered.contains("command"), "{rendered}");
        assert!(rendered.contains("ok"), "{rendered}");
        card.expanded = false;
        let collapsed = card.render(60).join("\n");
        assert!(!collapsed.contains("\"command\""), "{collapsed}");
    }
}
