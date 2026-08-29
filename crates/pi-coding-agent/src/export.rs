use std::fs;
use std::path::Path;

use pi_session::JsonlSession;

pub fn export_html(session: &JsonlSession, output: &Path) -> Result<String, String> {
    let mut html = String::from(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>pi session</title></head><body>",
    );
    html.push_str(&format!("<h1>Session {}</h1>", session.header.id));
    if let Some(name) = session.display_name() {
        html.push_str(&format!("<h2>{name}</h2>"));
    }
    html.push_str(&format!("<p>cwd: {}</p>", session.header.cwd));
    html.push_str("<ol>");
    for entry in &session.entries {
        let payload = serde_json::to_string(&entry.message).unwrap_or_default();
        html.push_str(&format!(
            "<li><code>{}</code> {}</li>",
            entry.entry_type,
            html_escape(&payload)
        ));
    }
    html.push_str("</ol></body></html>");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(output, &html).map_err(|err| err.to_string())?;
    Ok(output.display().to_string())
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
