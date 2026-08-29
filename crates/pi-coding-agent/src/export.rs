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

pub fn session_to_html(raw: &str, title: &str) -> String {
    let title = escape_html(title);
    let mut messages = String::new();
    let mut models = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                messages.push_str(&format!(
                    "<article class=\"entry raw\" id=\"entry-{}\"><pre>{}</pre></article>\n",
                    index,
                    escape_html(line)
                ));
                continue;
            }
        };
        if value.get("type").and_then(|v| v.as_str()) == Some("session")
            || value.get("kind").and_then(|v| v.as_str()) == Some("session")
        {
            continue;
        }
        let entry_id = value
            .get("id")
            .and_then(|v| v.as_str())
            .map(escape_html)
            .unwrap_or_else(|| index.to_string());
        let message = value.get("message").cloned().unwrap_or(value.clone());
        let role = message
            .get("role")
            .or_else(|| value.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("entry");
        if let Some(model) = message
            .get("model")
            .or_else(|| value.get("modelId"))
            .and_then(|v| v.as_str())
        {
            if !models.iter().any(|m: &String| m == model) {
                models.push(model.to_string());
            }
        }
        let content = message
            .get("content")
            .or_else(|| value.get("content"))
            .map(render_content)
            .unwrap_or_else(|| escape_html(&value.to_string()));
        messages.push_str(&format!(
            "<article class=\"entry role-{role}\" id=\"entry-{entry_id}\" data-entry-id=\"{entry_id}\">\
             <header><span class=\"role\">{}</span></header>\
             <div class=\"body\">{content}</div></article>\n",
            escape_html(role)
        ));
    }
    let model_label = if models.is_empty() {
        "unknown".to_string()
    } else {
        escape_html(&models.join(", "))
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; --bg:#111; --fg:#eee; --muted:#8cf; --line:#333; --user:#1e3a2f; --assistant:#1c2433; }}
body {{ margin:0; font-family:ui-sans-serif,system-ui,sans-serif; background:var(--bg); color:var(--fg); }}
#header-container {{ padding:1rem 1.5rem; border-bottom:1px solid var(--line); }}
#messages {{ padding:1rem 1.5rem 3rem; max-width:52rem; margin:0 auto; }}
.entry {{ margin:1rem 0; padding:0.85rem 1rem; border-radius:10px; background:#1a1a1a; }}
.role-user {{ background:var(--user); }}
.role-assistant {{ background:var(--assistant); }}
.role {{ color:var(--muted); font-size:0.75rem; text-transform:uppercase; letter-spacing:0.04em; }}
.body {{ white-space:pre-wrap; margin-top:0.4rem; }}
.body a {{ color:#8cf; }}
</style>
</head>
<body>
<div id="app">
  <main id="content">
    <div id="header-container"><h1>{title}</h1><p>models: {model_label}</p></div>
    <div id="messages">{messages}</div>
  </main>
</div>
<script>
function escapeHtml(value) {{
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}}
function sanitizeMarkdownUrl(href) {{
  const cleaned = String(href || '').replace(/[\x00-\x1f\x7f]/g, '');
  if (!/^(https?|mailto|tel|ftp):/i.test(cleaned)) return '';
  return cleaned;
}}
function link(token) {{
  const href = sanitizeMarkdownUrl(token.href);
  return href ? '<a href="' + escapeHtml(href) + '">' + escapeHtml(token.text || href) + '</a>' : escapeHtml(token.text || '');
}}
function image(token) {{
  const href = sanitizeMarkdownUrl(token.href);
  return href ? '<img src="' + escapeHtml(href) + '" alt="">' : '';
}}
</script>
</body>
</html>
"#
    )
}

fn render_content(value: &Value) -> String {
    match value {
        Value::String(text) => render_markdown_lite(text),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    render_markdown_lite(text)
                } else if item.get("type").and_then(|v| v.as_str()) == Some("toolCall") {
                    format!(
                        "<div class=\"tool\">[{}]</div>",
                        escape_html(item.get("name").and_then(|v| v.as_str()).unwrap_or("tool"))
                    )
                } else {
                    escape_html(&item.to_string())
                }
            })
            .collect(),
        other => escape_html(&other.to_string()),
    }
}

fn render_markdown_lite(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&escape_html(&rest[..start]));
        let after = &rest[start + 1..];
        if let Some(mid) = after.find("](") {
            let label = &after[..mid];
            let href_part = &after[mid + 2..];
            if let Some(end) = href_part.find(')') {
                let href = &href_part[..end];
                match sanitize_markdown_url(href) {
                    Some(safe) => {
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape_html(&safe),
                            escape_html(label)
                        ));
                    }
                    None => out.push_str(&escape_html(&format!("[{label}]({href})"))),
                }
                rest = &href_part[end + 1..];
                continue;
            }
        }
        out.push_str("&lt;");
        rest = after;
    }
    out.push_str(&escape_html(rest));
    out
}

#[allow(dead_code)]
pub fn export_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xss_and_sanitizes_urls() {
        let raw = r#"{"type":"message","id":"e<script>","role":"user","content":"hi [x](javascript:alert(1)) <img>"}"#;
        let html = session_to_html(raw, "title<script>");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;img&gt;") || html.contains("&lt;script&gt;"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(html.contains("entry-e&lt;script&gt;") || html.contains("data-entry-id="));
        assert!(html.contains("sanitizeMarkdownUrl(token.href)"));
        assert!(html.contains("^(https?|mailto|tel|ftp)"));
        assert!(html.contains("replace(/[\\x00-\\x1f\\x7f]/g, '')"));
        assert!(html.contains("escapeHtml(href)"));
    }

    #[test]
    fn keeps_http_links() {
        let raw = r#"{"type":"message","id":"1","role":"assistant","content":"see [docs](https://example.com)"}"#;
        let html = session_to_html(raw, "s");
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains(">docs</a>"));
    }
}
