use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{0}")]
    Message(String),
}

pub fn export_from_file(input: &str, output: Option<&str>) -> Result<PathBuf, ExportError> {
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
    let html = session_to_html(
        &raw,
        input_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("session"),
    );
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

const DEFAULT_THEME_VARS: &str = "\
--text: #d4d4d4;
--accent: #8abeb7;
--border: #5f87ff;
--muted: #808080;
--dim: #666666;
--userMessageBg: #343541;
--userMessageText: #d4d4d4;
--toolPendingBg: #282832;
--toolSuccessBg: #283228;
--toolErrorBg: #3c2828;
--mdHeading: #f0c674;
--mdLink: #81a2be;
--mdCode: #8abeb7;
--exportPageBg: rgb(24, 24, 30);
--exportCardBg: rgb(30, 30, 36);
--exportInfoBg: rgb(60, 55, 40);";

pub fn session_to_html(raw: &str, title: &str) -> String {
    let title = escape_html(title);
    let _ = sanitize_markdown_url("https://pi.dev");
    let (header, entries) = parse_session_document(raw, &title);
    let leaf_id = entries
        .iter()
        .rev()
        .find_map(|entry| entry.get("id").cloned());
    let session_json = serde_json::json!({
        "header": header,
        "entries": entries,
        "leafId": leaf_id,
    });
    let session_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        session_json.to_string().as_bytes(),
    );
    let css = TEMPLATE_CSS
        .replace("{{THEME_VARS}}", DEFAULT_THEME_VARS)
        .replace("{{BODY_BG}}", "rgb(24, 24, 30)")
        .replace("{{CONTAINER_BG}}", "rgb(30, 30, 36)")
        .replace("{{INFO_BG}}", "rgb(60, 55, 40)");
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
    let mut entry = value.clone();
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(
            "message".into(),
            serde_json::json!({
                "role": value.get("role"),
                "content": value.get("content"),
                "timestamp": value.get("timestamp"),
            }),
        );
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
    }
}
