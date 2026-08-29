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

pub fn session_to_html(raw: &str, title: &str) -> String {
    let mut body = String::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let escaped = line
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        body.push_str(&format!("<pre>{escaped}</pre>\n"));
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font-family:ui-monospace,monospace;background:#111;color:#eee;padding:1.5rem}}pre{{white-space:pre-wrap;border-bottom:1px solid #333;padding:.5rem 0}} .role{{color:#8cf}}</style></head><body><h1>{title}</h1>{body}</body></html>"
    )
}

#[allow(dead_code)]
pub fn export_exists(path: &Path) -> bool {
    path.exists()
}
