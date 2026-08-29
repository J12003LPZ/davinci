//! TypeScript `export-html/tool-renderer.ts` + `preRenderCustomTools`.

use crate::ansi_to_html::{ansi_lines_to_html, trim_rendered_result_lines};
use crate::extensions::{self, RegisteredToolMeta};
use crate::theme;
use pi_tui::wrap_text_with_ansi;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tools rendered directly by TypeScript `template.js`.
const TEMPLATE_RENDERED_TOOLS: &[&str] = &["bash", "read", "write", "edit", "ls"];
const DEFAULT_MAX_BYTES: u64 = 50 * 1024;
const RENDER_WIDTH: usize = 100;

#[derive(Debug, Clone, Default)]
pub struct RenderedToolHtml {
    pub call_html: Option<String>,
    pub result_html_collapsed: Option<String>,
    pub result_html_expanded: Option<String>,
}

impl RenderedToolHtml {
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        if let Some(html) = &self.call_html {
            obj.insert("callHtml".into(), json!(html));
        }
        if let Some(html) = &self.result_html_collapsed {
            obj.insert("resultHtmlCollapsed".into(), json!(html));
        }
        if let Some(html) = &self.result_html_expanded {
            obj.insert("resultHtmlExpanded".into(), json!(html));
        }
        Value::Object(obj)
    }
}

#[derive(Clone)]
pub struct ToolHtmlRenderer {
    theme: String,
    cwd: PathBuf,
    tools: Vec<RegisteredToolMeta>,
}

impl ToolHtmlRenderer {
    pub fn new(theme: impl Into<String>, cwd: PathBuf, tools: Vec<RegisteredToolMeta>) -> Self {
        Self {
            theme: theme.into(),
            cwd,
            tools,
        }
    }

    pub fn render_call(&self, tool_call_id: &str, tool_name: &str, args: &Value) -> Option<String> {
        let _ = tool_call_id;
        if let Some(lines) = builtin_render_call(tool_name, args, &self.theme, &self.cwd) {
            return Some(ansi_lines_to_html(&lines));
        }
        self.extension_render_call(tool_name, args)
    }

    pub fn render_result(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        result: &Value,
        details: &Value,
        is_error: bool,
    ) -> Option<(Option<String>, Option<String>)> {
        let _ = tool_call_id;
        if let Some((collapsed, expanded)) =
            builtin_render_result(tool_name, result, details, is_error, &self.theme)
        {
            return Some(html_pair(collapsed, expanded));
        }
        self.extension_render_result(tool_name, result, details, is_error)
    }

    fn extension_render_call(&self, tool_name: &str, args: &Value) -> Option<String> {
        let tool = self.tools.iter().find(|t| t.name == tool_name)?;
        if !tool.has_render_call {
            return None;
        }
        let colors = theme_color_map(&self.theme);
        let payload = json!({
            "args": args,
            "themeColors": colors,
            "width": RENDER_WIDTH,
            "cwd": self.cwd.to_string_lossy(),
        });
        let lines =
            extensions::invoke_extension_render(&tool.path, "render_call", tool_name, &payload)
                .ok()??;
        Some(ansi_lines_to_html(&lines))
    }

    fn extension_render_result(
        &self,
        tool_name: &str,
        result: &Value,
        details: &Value,
        is_error: bool,
    ) -> Option<(Option<String>, Option<String>)> {
        let tool = self.tools.iter().find(|t| t.name == tool_name)?;
        if !tool.has_render_result {
            return None;
        }
        let colors = theme_color_map(&self.theme);
        let collapsed_lines =
            render_extension_result(tool, &colors, &self.cwd, result, details, is_error, false)?;
        let expanded_lines =
            render_extension_result(tool, &colors, &self.cwd, result, details, is_error, true)?;
        Some(html_pair(collapsed_lines, expanded_lines))
    }
}

fn render_extension_result(
    tool: &RegisteredToolMeta,
    colors: &Value,
    cwd: &std::path::Path,
    result: &Value,
    details: &Value,
    is_error: bool,
    expanded: bool,
) -> Option<Vec<String>> {
    let payload = json!({
        "result": {
            "content": result,
            "details": details,
            "isError": is_error,
        },
        "options": { "expanded": expanded, "isPartial": false },
        "themeColors": colors,
        "width": RENDER_WIDTH,
        "cwd": cwd.to_string_lossy(),
        "isError": is_error,
    });
    extensions::invoke_extension_render(&tool.path, "render_result", &tool.name, &payload).ok()?
}

fn html_pair(collapsed: Vec<String>, expanded: Vec<String>) -> (Option<String>, Option<String>) {
    let collapsed_html = ansi_lines_to_html(&trim_rendered_result_lines(&collapsed));
    let expanded_html = ansi_lines_to_html(&trim_rendered_result_lines(&expanded));
    let collapsed = if collapsed_html.is_empty() || collapsed_html == expanded_html {
        None
    } else {
        Some(collapsed_html)
    };
    (
        collapsed,
        if expanded_html.is_empty() {
            None
        } else {
            Some(expanded_html)
        },
    )
}

fn theme_color_map(theme: &str) -> Value {
    Value::Object(theme::get_resolved_theme_colors(theme))
}

fn builtin_render_call(
    tool_name: &str,
    args: &Value,
    theme: &str,
    cwd: &std::path::Path,
) -> Option<Vec<String>> {
    let _ = cwd;
    let text = match tool_name {
        "grep" => format_grep_call(args, theme),
        "find" => format_find_call(args, theme),
        _ => return None,
    };
    Some(wrap_text_with_ansi(&text, RENDER_WIDTH))
}

fn builtin_render_result(
    tool_name: &str,
    result: &Value,
    details: &Value,
    _is_error: bool,
    theme: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    match tool_name {
        "grep" => Some((
            wrap_text_with_ansi(
                &format_grep_result(result, details, false, theme),
                RENDER_WIDTH,
            ),
            wrap_text_with_ansi(
                &format_grep_result(result, details, true, theme),
                RENDER_WIDTH,
            ),
        )),
        "find" => Some((
            wrap_text_with_ansi(
                &format_find_result(result, details, false, theme),
                RENDER_WIDTH,
            ),
            wrap_text_with_ansi(
                &format_find_result(result, details, true, theme),
                RENDER_WIDTH,
            ),
        )),
        _ => None,
    }
}

fn js_str(value: Option<&Value>) -> Option<String> {
    match value {
        None => Some(String::new()),
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => None,
    }
}

fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(home.as_ref()) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

fn invalid_arg(theme: &str) -> String {
    theme_fg(theme, "error", "[invalid arg]")
}

fn theme_fg(theme: &str, color: &str, text: &str) -> String {
    let colors = theme::get_resolved_theme_colors(theme);
    let hex = colors
        .get(color)
        .and_then(|v| v.as_str())
        .unwrap_or("#d4d4d4");
    match theme::parse_color(hex) {
        Some((r, g, b)) => format!("\u{1b}[38;2;{r};{g};{b}m{text}\u{1b}[39m"),
        None => text.to_string(),
    }
}

fn theme_bold(text: &str) -> String {
    format!("\u{1b}[1m{text}\u{1b}[22m")
}

fn expand_hint(theme: &str) -> String {
    format!(
        "{}{}",
        theme_fg(theme, "dim", "ctrl+o"),
        theme_fg(theme, "muted", " to expand")
    )
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn get_text_output(result: &Value) -> String {
    let content = if result.is_array() {
        result.as_array()
    } else {
        result.get("content").and_then(|v| v.as_array())
    };
    let Some(parts) = content else {
        return String::new();
    };
    parts
        .iter()
        .filter(|part| part.get("type").and_then(|v| v.as_str()) == Some("text"))
        .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
        .replace('\r', "")
}

fn format_grep_call(args: &Value, theme: &str) -> String {
    let pattern = js_str(args.get("pattern"));
    let raw_path = js_str(args.get("path"));
    let path = raw_path
        .as_ref()
        .map(|p| shorten_path(if p.is_empty() { "." } else { p }));
    let glob = js_str(args.get("glob")).filter(|g| !g.is_empty());
    let limit = args.get("limit");
    let mut text = format!(
        "{} {}{}",
        theme_fg(theme, "toolTitle", &theme_bold("grep")),
        match pattern.as_deref() {
            None => invalid_arg(theme),
            Some(p) => theme_fg(theme, "accent", &format!("/{p}/")),
        },
        theme_fg(
            theme,
            "toolOutput",
            &format!(
                " in {}",
                path.as_deref()
                    .map(str::to_string)
                    .unwrap_or_else(|| invalid_arg(theme))
            )
        )
    );
    if let Some(glob) = glob {
        text.push_str(&theme_fg(theme, "toolOutput", &format!(" ({glob})")));
    }
    if let Some(limit) = limit {
        text.push_str(&theme_fg(theme, "toolOutput", &format!(" limit {limit}")));
    }
    text
}

fn format_find_call(args: &Value, theme: &str) -> String {
    let pattern = js_str(args.get("pattern"));
    let raw_path = js_str(args.get("path"));
    let path = raw_path
        .as_ref()
        .map(|p| shorten_path(if p.is_empty() { "." } else { p }));
    let limit = args.get("limit");
    let mut text = format!(
        "{} {}{}",
        theme_fg(theme, "toolTitle", &theme_bold("find")),
        match pattern.as_deref() {
            None => invalid_arg(theme),
            Some(p) => theme_fg(theme, "accent", p),
        },
        theme_fg(
            theme,
            "toolOutput",
            &format!(
                " in {}",
                path.as_deref()
                    .map(str::to_string)
                    .unwrap_or_else(|| invalid_arg(theme))
            )
        )
    );
    if let Some(limit) = limit {
        text.push_str(&theme_fg(theme, "toolOutput", &format!(" (limit {limit})")));
    }
    text
}

fn format_grep_result(result: &Value, details: &Value, expanded: bool, theme: &str) -> String {
    let output = get_text_output(result).trim().to_string();
    let mut text = String::new();
    if !output.is_empty() {
        let lines: Vec<&str> = output.split('\n').collect();
        let max_lines = if expanded { lines.len() } else { 15 };
        let display = &lines[..max_lines.min(lines.len())];
        let remaining = lines.len().saturating_sub(max_lines);
        text.push('\n');
        text.push_str(
            &display
                .iter()
                .map(|line| theme_fg(theme, "toolOutput", line))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if remaining > 0 {
            text.push_str(&theme_fg(
                theme,
                "muted",
                &format!("\n... ({remaining} more lines,"),
            ));
            text.push(' ');
            text.push_str(&expand_hint(theme));
            text.push_str(&theme_fg(theme, "muted", ")"));
        }
    }
    let match_limit = details.get("matchLimitReached");
    let truncation = details.get("truncation");
    let lines_truncated = details
        .get("linesTruncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let truncated = truncation
        .and_then(|t| t.get("truncated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if match_limit.is_some_and(|v| !v.is_null()) || truncated || lines_truncated {
        let mut warnings = Vec::new();
        if let Some(limit) = match_limit {
            if !limit.is_null() {
                warnings.push(format!("{limit} matches limit"));
            }
        }
        if truncated {
            let max_bytes = truncation
                .and_then(|t| t.get("maxBytes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_MAX_BYTES);
            warnings.push(format!("{} limit", format_size(max_bytes)));
        }
        if lines_truncated {
            warnings.push("some lines truncated".into());
        }
        text.push('\n');
        text.push_str(&theme_fg(
            theme,
            "warning",
            &format!("[Truncated: {}]", warnings.join(", ")),
        ));
    }
    text
}

fn format_find_result(result: &Value, details: &Value, expanded: bool, theme: &str) -> String {
    let output = get_text_output(result).trim().to_string();
    let mut text = String::new();
    if !output.is_empty() {
        let lines: Vec<&str> = output.split('\n').collect();
        let max_lines = if expanded { lines.len() } else { 20 };
        let display = &lines[..max_lines.min(lines.len())];
        let remaining = lines.len().saturating_sub(max_lines);
        text.push('\n');
        text.push_str(
            &display
                .iter()
                .map(|line| theme_fg(theme, "toolOutput", line))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if remaining > 0 {
            text.push_str(&theme_fg(
                theme,
                "muted",
                &format!("\n... ({remaining} more lines,"),
            ));
            text.push(' ');
            text.push_str(&expand_hint(theme));
            text.push_str(&theme_fg(theme, "muted", ")"));
        }
    }
    let result_limit = details.get("resultLimitReached");
    let truncation = details.get("truncation");
    let truncated = truncation
        .and_then(|t| t.get("truncated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if result_limit.is_some_and(|v| !v.is_null()) || truncated {
        let mut warnings = Vec::new();
        if let Some(limit) = result_limit {
            if !limit.is_null() {
                warnings.push(format!("{limit} results limit"));
            }
        }
        if truncated {
            let max_bytes = truncation
                .and_then(|t| t.get("maxBytes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_MAX_BYTES);
            warnings.push(format!("{} limit", format_size(max_bytes)));
        }
        text.push('\n');
        text.push_str(&theme_fg(
            theme,
            "warning",
            &format!("[Truncated: {}]", warnings.join(", ")),
        ));
    }
    text
}

pub fn is_template_rendered(name: &str) -> bool {
    TEMPLATE_RENDERED_TOOLS.contains(&name)
}

/// TypeScript `preRenderCustomTools`.
pub fn pre_render_custom_tools(entries: &[Value], renderer: &ToolHtmlRenderer) -> Option<Value> {
    let mut rendered: serde_json::Map<String, Value> = serde_json::Map::new();
    for entry in entries {
        if entry.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let msg = entry.get("message").unwrap_or(entry);
        if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
            if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) != Some("toolCall") {
                        continue;
                    }
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if is_template_rendered(name) {
                        continue;
                    }
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let args = block
                        .get("arguments")
                        .or_else(|| block.get("args"))
                        .cloned()
                        .unwrap_or(json!({}));
                    if let Some(call_html) = renderer.render_call(id, name, &args) {
                        rendered.insert(
                            id.to_string(),
                            RenderedToolHtml {
                                call_html: Some(call_html),
                                ..Default::default()
                            }
                            .to_json(),
                        );
                    }
                }
            }
        }
        if msg.get("role").and_then(|v| v.as_str()) == Some("toolResult") {
            let Some(tool_call_id) = msg
                .get("toolCallId")
                .or_else(|| entry.get("toolCallId"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let tool_name = msg
                .get("toolName")
                .or_else(|| entry.get("toolName"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let existing = rendered.get(tool_call_id).cloned();
            if existing.is_none() && is_template_rendered(tool_name) {
                continue;
            }
            let content = msg
                .get("content")
                .or_else(|| entry.get("content"))
                .cloned()
                .unwrap_or(json!([]));
            let details = msg
                .get("details")
                .or_else(|| entry.get("details"))
                .cloned()
                .unwrap_or(json!({}));
            let is_error = msg
                .get("isError")
                .or_else(|| entry.get("isError"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some((collapsed, expanded)) =
                renderer.render_result(tool_call_id, tool_name, &content, &details, is_error)
            {
                let mut html = existing
                    .as_ref()
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                if let Some(collapsed) = collapsed {
                    html.insert("resultHtmlCollapsed".into(), json!(collapsed));
                }
                if let Some(expanded) = expanded {
                    html.insert("resultHtmlExpanded".into(), json!(expanded));
                }
                rendered.insert(tool_call_id.to_string(), Value::Object(html));
            }
        }
    }
    if rendered.is_empty() {
        None
    } else {
        Some(Value::Object(rendered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi_to_html::ansi_to_html;

    #[test]
    fn grep_and_find_are_pre_rendered_not_template_tools() {
        assert!(!is_template_rendered("grep"));
        assert!(!is_template_rendered("find"));
        assert!(is_template_rendered("bash"));
        let renderer = ToolHtmlRenderer::new("dark", PathBuf::from("."), vec![]);
        let call = renderer
            .render_call(
                "c1",
                "grep",
                &json!({"pattern": "TODO", "path": ".", "glob": "*.rs", "limit": 10}),
            )
            .unwrap();
        assert!(call.contains("ansi-line"));
        assert!(call.contains("TODO") || call.contains("/TODO/"));
        let result = renderer
            .render_result(
                "c1",
                "grep",
                &json!([{"type":"text","text":"src/main.rs:1:TODO"}]),
                &json!({"matchLimitReached": 100, "truncation": {"truncated": true, "maxBytes": 51200}}),
                false,
            )
            .unwrap();
        let expanded = result.1.unwrap();
        assert!(expanded.contains("TODO"));
        assert!(expanded.contains("Truncated") || expanded.contains("matches limit"));
        assert_eq!(format_size(51200), "50.0KB");
        assert!(ansi_to_html("\u{1b}[31mx").contains("#800000"));
    }

    #[test]
    fn pre_render_walks_session_entries() {
        let renderer = ToolHtmlRenderer::new("dark", PathBuf::from("."), vec![]);
        let entries = vec![
            json!({
                "type": "message",
                "id": "a",
                "message": {
                    "role": "assistant",
                    "content": [{
                        "type": "toolCall",
                        "id": "call-1",
                        "name": "grep",
                        "arguments": {"pattern": "fn"}
                    }]
                }
            }),
            json!({
                "type": "message",
                "id": "b",
                "message": {
                    "role": "toolResult",
                    "toolCallId": "call-1",
                    "toolName": "grep",
                    "content": [{"type":"text","text":"lib.rs:1:fn"}],
                    "isError": false
                }
            }),
            json!({
                "type": "message",
                "id": "c",
                "message": {
                    "role": "assistant",
                    "content": [{
                        "type": "toolCall",
                        "id": "call-2",
                        "name": "bash",
                        "arguments": {"command": "ls"}
                    }]
                }
            }),
        ];
        let rendered = pre_render_custom_tools(&entries, &renderer).unwrap();
        assert!(rendered.get("call-1").is_some());
        assert!(rendered.get("call-1").unwrap().get("callHtml").is_some());
        assert!(rendered
            .get("call-1")
            .unwrap()
            .get("resultHtmlExpanded")
            .is_some());
        assert!(rendered.get("call-2").is_none());
    }
}
