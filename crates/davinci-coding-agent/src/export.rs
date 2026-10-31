use std::fs;
use std::path::Path;

use base64::Engine;
use davinci_session::JsonlSession;
use davinci_tui::builtin_themes;
use davinci_tui::export_css;

const TEMPLATE_HTML: &str = include_str!("../export-html/template.html");
const TEMPLATE_CSS: &str = include_str!("../export-html/template.css");
const TEMPLATE_JS: &str = include_str!("../export-html/template.js");
const MARKED_JS: &str = include_str!("../export-html/vendor/marked.min.js");
const HIGHLIGHT_JS: &str = include_str!("../export-html/vendor/highlight.min.js");

pub fn export_session(session: &JsonlSession, output: &Path) -> Result<String, String> {
    if output
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
    {
        export_jsonl(session, output)
    } else {
        export_html(session, output)
    }
}

pub fn export_jsonl(session: &JsonlSession, output: &Path) -> Result<String, String> {
    davinci_session::export_session_jsonl(session, output).map_err(|err| err.to_string())
}

pub fn export_html(session: &JsonlSession, output: &Path) -> Result<String, String> {
    let html = generate_html(session, None)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(output, html).map_err(|err| err.to_string())?;
    Ok(output.display().to_string())
}

pub fn generate_html(session: &JsonlSession, theme_name: Option<&str>) -> Result<String, String> {
    let theme = builtin_themes()
        .into_iter()
        .find(|theme| theme_name == Some(theme.name.as_str()))
        .or_else(|| builtin_themes().into_iter().next())
        .expect("theme");
    let entries: Vec<_> = session
        .entries
        .iter()
        .filter(|entry| {
            !(entry.custom_type.is_some()
                && entry.extra.get("display") == Some(&serde_json::Value::Bool(false)))
        })
        .collect();
    let session_data = serde_json::json!({
        "header": {
            "type": "session",
            "version": session.header.version,
            "id": session.header.id,
            "cwd": session.header.cwd,
            "timestamp": session.header.created_at,
        },
        "entries": entries,
        "leafId": session.leaf_id,
        "systemPrompt": null,
        "tools": [],
        "renderedTools": {},
    });
    let session_data_b64 = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&session_data).map_err(|err| err.to_string())?);
    let (theme_vars, background) = export_css(&theme);
    let css = TEMPLATE_CSS
        .replace("{{THEME_VARS}}", &theme_vars)
        .replace("{{BODY_BG}}", &background)
        .replace("{{CONTAINER_BG}}", &background)
        .replace("{{INFO_BG}}", &background);
    Ok(TEMPLATE_HTML
        .replace("{{CSS}}", &css)
        .replace("{{JS}}", TEMPLATE_JS)
        .replace("{{SESSION_DATA}}", &session_data_b64)
        .replace("{{MARKED_JS}}", MARKED_JS)
        .replace("{{HIGHLIGHT_JS}}", HIGHLIGHT_JS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn export_embeds_session_data_and_template_js() {
        let dir = tempdir().unwrap();
        let mut session =
            davinci_session::JsonlSession::create(dir.path(), "/tmp", Some("demo")).unwrap();
        session
            .append_entry(davinci_session::SessionEntry::message(
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
        assert!(html.contains("id=\"session-data\""));
        assert!(html.contains("function buildTree"));
        assert!(!html.contains("<script>alert(1)</script>"));
        let jsonl = dir.path().join("session.jsonl");
        export_session(&session, &jsonl).unwrap();
        let raw = std::fs::read_to_string(&jsonl).unwrap();
        assert!(raw.starts_with("{\"cwd\"") || raw.contains("\"type\":\"session\""));
        assert!(raw.contains("hello") || raw.contains("<script>"));
    }

    fn embedded_session_data(html: &str) -> String {
        let marker = "type=\"application/json\">";
        let start = html.find(marker).unwrap() + marker.len();
        let end = html[start..].find("</script>").unwrap() + start;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(html[start..end].trim())
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn audit_wp11_b106_hidden_custom_entries_are_not_exported() {
        let dir = tempdir().unwrap();
        let mut session =
            davinci_session::JsonlSession::create(dir.path(), "/tmp", Some("demo")).unwrap();
        let mut hidden =
            davinci_session::SessionEntry::message("user", serde_json::json!("hidden-marker-abc"));
        hidden.custom_type = Some("note".into());
        hidden
            .extra
            .insert("display".into(), serde_json::Value::Bool(false));
        session.append_entry(hidden).unwrap();
        let mut visible =
            davinci_session::SessionEntry::message("user", serde_json::json!("visible-marker-xyz"));
        visible.custom_type = Some("note".into());
        visible
            .extra
            .insert("display".into(), serde_json::Value::Bool(true));
        session.append_entry(visible).unwrap();
        let html = generate_html(&session, None).unwrap();
        let data = embedded_session_data(&html);
        assert!(!data.contains("hidden-marker-abc"), "{data}");
        assert!(data.contains("visible-marker-xyz"), "{data}");
    }

    #[test]
    fn audit_wp11_b271_export_defines_foreground_theme_variables() {
        let dir = tempdir().unwrap();
        let session =
            davinci_session::JsonlSession::create(dir.path(), "/tmp", Some("theme")).unwrap();
        let html = generate_html(&session, None).unwrap();
        assert!(!html.contains("{{THEME_VARS}}"));
        assert!(html.contains("--text:#"), "no text colour in {html}");
        assert!(html.contains("--accent:#"), "no accent colour in {html}");
        assert!(html.contains("--mdLink:#"), "no markdown link colour");
    }
}
