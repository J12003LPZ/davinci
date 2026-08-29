use std::fs;
use std::path::Path;

use base64::Engine;
use pi_session::JsonlSession;
use pi_tui::builtin_themes;

const TEMPLATE_HTML: &str = include_str!("../export-html/template.html");
const TEMPLATE_CSS: &str = include_str!("../export-html/template.css");
const TEMPLATE_JS: &str = include_str!("../export-html/template.js");
const MARKED_JS: &str = include_str!("../export-html/vendor/marked.min.js");
const HIGHLIGHT_JS: &str = include_str!("../export-html/vendor/highlight.min.js");

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
    let session_data = serde_json::json!({
        "header": {
            "type": "session",
            "version": session.header.version,
            "id": session.header.id,
            "cwd": session.header.cwd,
            "timestamp": session.header.created_at,
        },
        "entries": session.entries,
        "leafId": session.leaf_id,
        "systemPrompt": null,
        "tools": [],
        "renderedTools": {},
    });
    let session_data_b64 = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&session_data).map_err(|err| err.to_string())?);
    let css = TEMPLATE_CSS
        .replace("{{THEME_VARS}}", "")
        .replace("{{BODY_BG}}", &theme.background)
        .replace("{{CONTAINER_BG}}", &theme.background)
        .replace("{{INFO_BG}}", &theme.background);
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
        assert!(html.contains("id=\"session-data\""));
        assert!(html.contains("function buildTree"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }
}
