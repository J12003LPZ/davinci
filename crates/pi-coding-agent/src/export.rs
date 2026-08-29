use crate::theme;
use crate::tool_html::{self, ToolHtmlRenderer};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Default)]
pub struct HtmlExportState {
    pub system_prompt: Option<String>,
    pub tools: Vec<Value>,
}

pub fn export_from_file(input: &str, output: Option<&str>) -> Result<PathBuf, ExportError> {
    write_export(input, output, None, None, None)
}

pub fn export_from_file_with_theme(
    input: &str,
    output: Option<&str>,
    theme: &str,
) -> Result<PathBuf, ExportError> {
    write_export(input, output, Some(theme), None, None)
}

pub fn export_from_file_with_renderer(
    input: &str,
    output: Option<&str>,
    theme: &str,
    renderer: &ToolHtmlRenderer,
    state: Option<&HtmlExportState>,
) -> Result<PathBuf, ExportError> {
    write_export(input, output, Some(theme), Some(renderer), state)
}

fn write_export(
    input: &str,
    output: Option<&str>,
    theme: Option<&str>,
    renderer: Option<&ToolHtmlRenderer>,
    state: Option<&HtmlExportState>,
) -> Result<PathBuf, ExportError> {
    let input_path = expand_tilde(input);
    let raw = fs::read_to_string(&input_path)
        .map_err(|e| ExportError::Message(format!("Failed to export session: {e}")))?;
    let dest = match output {
        Some(path) => expand_tilde(path),
        None => {
            let mut dest = input_path.clone();
            dest.set_extension("html");
            dest
        }
    };
    let title = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    let html = match (theme, renderer) {
        (Some(theme), Some(renderer)) => {
            session_to_html_document(&raw, title, theme, Some(renderer), state)
        }
        (Some(theme), None) => session_to_html_with_theme(&raw, title, theme),
        (None, Some(renderer)) => {
            session_to_html_document(&raw, title, "dark", Some(renderer), state)
        }
        (None, None) => session_to_html(&raw, title),
    };
    fs::write(&dest, html).map_err(|e| ExportError::Message(e.to_string()))?;
    Ok(dest)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// TypeScript `sanitizeMarkdownUrl`: strip C0 controls, allow http(s)|mailto|tel|ftp.
pub fn sanitize_markdown_url(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            let n = *c as u32;
            n > 0x1f && n != 0x7f
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https:")
        || lower.starts_with("http:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("ftp:")
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

const TEMPLATE_HTML: &str =
    include_str!("../../../vendor/pi/packages/coding-agent/src/core/export-html/template.html");
const TEMPLATE_CSS: &str =
    include_str!("../../../vendor/pi/packages/coding-agent/src/core/export-html/template.css");
const TEMPLATE_JS: &str =
    include_str!("../../../vendor/pi/packages/coding-agent/src/core/export-html/template.js");
const MARKED_JS: &str = include_str!(
    "../../../vendor/pi/packages/coding-agent/src/core/export-html/vendor/marked.min.js"
);
const HIGHLIGHT_JS: &str = include_str!(
    "../../../vendor/pi/packages/coding-agent/src/core/export-html/vendor/highlight.min.js"
);

pub fn session_to_html(raw: &str, title: &str) -> String {
    session_to_html_with_theme(raw, title, "dark")
}

pub fn session_to_html_with_theme(raw: &str, title: &str, theme: &str) -> String {
    session_to_html_document(raw, title, theme, None, None)
}

pub fn session_to_html_document(
    raw: &str,
    title: &str,
    theme: &str,
    renderer: Option<&ToolHtmlRenderer>,
    state: Option<&HtmlExportState>,
) -> String {
    let title = escape_html(title);
    let _ = sanitize_markdown_url("https://pi.dev");
    let (header, entries) = parse_session_document(raw, &title);
    let leaf_id = entries
        .iter()
        .rev()
        .find_map(|entry| entry.get("id").cloned());
    let rendered_tools = renderer.and_then(|r| tool_html::pre_render_custom_tools(&entries, r));
    let mut session_json = serde_json::json!({
        "header": header,
        "entries": entries,
        "leafId": leaf_id,
    });
    if let Some(obj) = session_json.as_object_mut() {
        if let Some(rendered) = rendered_tools {
            obj.insert("renderedTools".into(), rendered);
        }
        if let Some(state) = state {
            if let Some(prompt) = &state.system_prompt {
                obj.insert("systemPrompt".into(), json!(prompt));
            }
            if !state.tools.is_empty() {
                obj.insert("tools".into(), json!(state.tools));
            }
        }
    }
    let session_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        session_json.to_string().as_bytes(),
    );
    let (theme_vars, page_bg, card_bg, info_bg) = theme::generate_theme_vars(theme);
    let css = TEMPLATE_CSS
        .replace("{{THEME_VARS}}", &theme_vars)
        .replace("{{BODY_BG}}", &page_bg)
        .replace("{{CONTAINER_BG}}", &card_bg)
        .replace("{{INFO_BG}}", &info_bg);
    TEMPLATE_HTML
        .replace(
            "<title>Session Export</title>",
            &format!("<title>{title}</title>"),
        )
        .replace("{{CSS}}", &css)
        .replace("{{JS}}", TEMPLATE_JS)
        .replace("{{SESSION_DATA}}", &session_b64)
        .replace("{{MARKED_JS}}", MARKED_JS)
        .replace("{{HIGHLIGHT_JS}}", HIGHLIGHT_JS)
}

fn parse_session_document(raw: &str, title: &str) -> (Value, Vec<Value>) {
    let mut header = serde_json::json!({
        "type": "session",
        "id": title,
        "cwd": ".",
        "timestamp": "",
    });
    let mut entries = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = value
            .get("kind")
            .or_else(|| value.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if kind == "header" || kind == "session" {
            header = value;
            continue;
        }
        entries.push(normalize_entry(value));
    }
    (header, entries)
}

fn normalize_entry(value: Value) -> Value {
    if value.get("message").is_some() {
        return value;
    }
    if value.get("type").and_then(|v| v.as_str()) != Some("message") {
        return value;
    }
    let mut message = serde_json::Map::new();
    for key in [
        "role",
        "content",
        "timestamp",
        "toolCallId",
        "toolName",
        "details",
        "isError",
        "name",
        "arguments",
    ] {
        if let Some(v) = value.get(key) {
            message.insert(key.to_string(), v.clone());
        }
    }
    let mut entry = value.clone();
    if let Some(obj) = entry.as_object_mut() {
        obj.insert("message".into(), Value::Object(message));
    }
    entry
}

#[allow(dead_code)]
pub fn export_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn session_payload(html: &str) -> Value {
        let marker = "id=\"session-data\"";
        let start = html.find(marker).expect("session-data");
        let after = &html[start..];
        let open = after.find('>').expect("open");
        let close = after.find("</script>").expect("close");
        let b64 = after[open + 1..close].trim();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[test]
    fn escapes_xss_and_sanitizes_urls() {
        assert_eq!(escape_html("<img>"), "&lt;img&gt;");
        assert_eq!(sanitize_markdown_url("javascript:alert(1)"), None);
        assert_eq!(
            sanitize_markdown_url("https://example.com"),
            Some("https://example.com".into())
        );
        let raw = r#"{"type":"message","id":"e<script>","role":"user","content":"hi [x](javascript:alert(1)) <img>"}"#;
        let html = session_to_html(raw, "title<script>");
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(html.contains("sanitizeMarkdownUrl(token.href)"));
        assert!(html.contains("^(https?|mailto|tel|ftp)"));
        assert!(html.contains("replace(/[\\x00-\\x1f\\x7f]/g, '')"));
        assert!(html.contains("escapeHtml(href)"));
        assert!(html.contains("entry-${escapeHtml(entry.id)}"));
        assert!(html.contains("data-entry-id=\"${escapeHtml(entryId)}\""));
    }

    #[test]
    fn keeps_http_links() {
        let raw = r#"{"type":"message","id":"1","role":"assistant","content":"see [docs](https://example.com)"}"#;
        let html = session_to_html(raw, "s");
        let data = session_payload(&html);
        let content = data["entries"][0]["message"]["content"].as_str().unwrap();
        assert!(content.contains("https://example.com"));
        assert!(html.contains("function sanitizeMarkdownUrl"));
    }

    #[test]
    fn embeds_typescript_template_and_session_data() {
        let raw = r#"{"type":"message","id":"root","parentId":null,"role":"user","content":"hi"}
{"type":"message","id":"child","parentId":"root","role":"assistant","content":"ok"}"#;
        let html = session_to_html(raw, "s");
        assert!(html.contains("id=\"tree-container\""));
        assert!(html.contains("id=\"session-data\""));
        assert!(html.contains("function buildTree()"));
        assert!(html.contains("id=\"tree-search\""));
        assert!(html.contains("data-filter=\"default\""));
        assert!(html.contains("id=\"hamburger\""));
        assert!(html.contains("marked v18"));
        assert!(html.contains("hljs") || html.contains("highlight.js"));
        let data = session_payload(&html);
        assert_eq!(data["entries"][0]["id"], "root");
        assert_eq!(data["entries"][1]["id"], "child");
        assert_eq!(data["leafId"], "child");
        assert_eq!(data["entries"][0]["message"]["role"], "user");
        assert!(html.contains("#18181e"));
        let light = session_to_html_with_theme(raw, "s", "light");
        assert!(light.contains("#f8f8f8"));
        assert!(light.contains("#ffffff"));
    }

    #[test]
    fn session_html_includes_custom_tool_pre_render() {
        let raw = r#"{"type":"message","id":"a","role":"assistant","content":[{"type":"toolCall","id":"call-1","name":"grep","arguments":{"pattern":"TODO"}}]}
{"type":"message","id":"b","role":"toolResult","toolCallId":"call-1","toolName":"grep","content":[{"type":"text","text":"main.rs:1:TODO"}]}
{"type":"message","id":"c","role":"assistant","content":[{"type":"toolCall","id":"call-2","name":"bash","arguments":{"command":"ls"}}]}"#;
        let renderer = ToolHtmlRenderer::new("dark", PathBuf::from("."), vec![]);
        let html = session_to_html_document(raw, "s", "dark", Some(&renderer), None);
        let data = session_payload(&html);
        assert!(data["renderedTools"]["call-1"]["callHtml"]
            .as_str()
            .unwrap()
            .contains("ansi-line"));
        assert!(data["renderedTools"]["call-1"]["resultHtmlExpanded"]
            .as_str()
            .unwrap()
            .contains("TODO"));
        assert!(data["renderedTools"].get("call-2").is_none());
    }

    #[test]
    fn custom_message_entries_reach_html_session_data() {
        let raw = r#"{"type":"custom_message","id":"cm1","customType":"note","content":"hello custom","display":true}"#;
        let html = session_to_html(raw, "s");
        let data = session_payload(&html);
        assert_eq!(data["entries"][0]["type"], "custom_message");
        assert_eq!(data["entries"][0]["customType"], "note");
        assert!(html.contains("custom_message") || html.contains("hook-type"));
    }
}
