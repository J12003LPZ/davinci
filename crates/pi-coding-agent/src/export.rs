use std::fs;
use std::path::Path;

use pi_session::JsonlSession;
use pi_tui::builtin_themes;

pub fn export_html(session: &JsonlSession, output: &Path) -> Result<String, String> {
    let theme = builtin_themes().into_iter().next().expect("builtin theme");
    let mut messages = String::new();
    for entry in &session.entries {
        let role = entry
            .message
            .as_ref()
            .and_then(|value| value.get("role"))
            .and_then(|value| value.as_str())
            .unwrap_or(&entry.entry_type);
        let text = entry
            .message
            .as_ref()
            .and_then(message_text)
            .unwrap_or_default();
        messages.push_str(&format!(
            "<article class=\"message {role}\" data-id=\"{}\"><header>{}</header><pre>{}</pre></article>",
            html_escape(&entry.id),
            html_escape(role),
            html_escape(&text)
        ));
    }
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>pi session {id}</title>
<style>
:root {{
  --text: {fg};
  --body-bg: {bg};
  --container-bg: {bg};
  --accent: {accent};
  --dim: #666;
}}
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
  font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace;
  font-size: 12px;
  color: var(--text);
  background: var(--body-bg);
}}
#app {{ display: flex; min-height: 100vh; }}
#sidebar {{
  width: 280px;
  background: var(--container-bg);
  border-right: 1px solid var(--dim);
  padding: 16px;
}}
#messages {{ flex: 1; padding: 16px; }}
.message {{ margin-bottom: 16px; }}
.message header {{ color: var(--accent); margin-bottom: 4px; }}
.message pre {{ white-space: pre-wrap; }}
</style>
</head>
<body>
<div id="app">
<aside id="sidebar">
  <h1>Session</h1>
  <p class="id">{id}</p>
  <p class="cwd">{cwd}</p>
  <p class="name">{name}</p>
</aside>
<main id="messages">{messages}</main>
</div>
</body>
</html>
"#,
        id = html_escape(&session.header.id),
        cwd = html_escape(&session.header.cwd),
        name = html_escape(&session.display_name().unwrap_or_default()),
        bg = theme.background,
        fg = theme.foreground,
        accent = theme.accent,
        messages = messages,
    );
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(output, &html).map_err(|err| err.to_string())?;
    Ok(output.display().to_string())
}

fn message_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("content").and_then(|content| {
        if let Some(text) = content.as_str() {
            return Some(text.to_string());
        }
        content.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
                .join("")
        })
    }) {
        return Some(text);
    }
    Some(value.to_string())
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn export_escapes_and_uses_ts_layout_ids() {
        let dir = tempdir().unwrap();
        let mut session =
            pi_session::JsonlSession::create(dir.path(), "/tmp", Some("demo")).unwrap();
        session
            .append_entry(pi_session::SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"<script>alert(1)</script>"}]),
            ))
            .unwrap();
        let out = dir.path().join("session.html");
        export_html(&session, &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains("id=\"app\""));
        assert!(html.contains("id=\"sidebar\""));
        assert!(html.contains("id=\"messages\""));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
