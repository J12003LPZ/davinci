//! Session share matching TypeScript `session-share.ts`.

use std::path::Path;
use std::process::Command;

const DEFAULT_SHARE_VIEWER_URL: &str = "https://pi.dev/session/";

pub fn share_viewer_url(gist_id: &str) -> String {
    let base = std::env::var("PI_SHARE_VIEWER_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SHARE_VIEWER_URL.to_string());
    format!("{base}#{gist_id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareResult {
    pub gist_url: String,
    pub preview_url: String,
}

pub fn gh_available() -> Result<(), String> {
    match Command::new("gh").args(["auth", "status"]).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err("GitHub CLI is not logged in. Run 'gh auth login' first.".into()),
        Err(_) => {
            Err("GitHub CLI (gh) is not installed. Install it from https://cli.github.com/".into())
        }
    }
}

/// Create a secret gist from an exported HTML file (`gh gist create --public=false`).
pub fn share_via_gist(html_path: &Path) -> Result<ShareResult, String> {
    gh_available()?;
    let output = Command::new("gh")
        .args([
            "gist",
            "create",
            "--public=false",
            &html_path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("Failed to create gist: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to create gist: {}",
            stderr.trim().if_empty("Unknown error")
        ));
    }
    let gist_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let gist_id = gist_url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Failed to parse gist ID from gh output".to_string())?;
    Ok(ShareResult {
        preview_url: share_viewer_url(gist_id),
        gist_url,
    })
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for &str {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_url_matches_typescript() {
        std::env::remove_var("PI_SHARE_VIEWER_URL");
        assert_eq!(share_viewer_url("abc123"), "https://pi.dev/session/#abc123");
        std::env::set_var("PI_SHARE_VIEWER_URL", "https://example.test/s/");
        assert_eq!(share_viewer_url("abc123"), "https://example.test/s/#abc123");
        std::env::remove_var("PI_SHARE_VIEWER_URL");
    }

    #[test]
    fn missing_gh_uses_ts_error() {
        if Command::new("gh").output().is_err() {
            assert!(gh_available()
                .unwrap_err()
                .contains("GitHub CLI (gh) is not installed"));
        }
    }
}
