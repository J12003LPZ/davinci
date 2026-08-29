//! Session share matching TypeScript `session-share.ts`.

use std::path::Path;
use std::process::Command;

const DEFAULT_SHARE_VIEWER_URL: &str = "https://pi.dev/session/";

pub fn radius_artifact_url(gateway: &str) -> String {
    format!(
        "{}/v1/artifacts?visibility=organization&title=Pi%20session",
        pi_ai::normalize_radius_gateway_url(gateway)
    )
}

pub fn parse_radius_artifact_response(status: u16, body: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if let Some(url) = json
        .pointer("/artifact/canonical_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if (200..300).contains(&status) {
            return Ok(url.to_string());
        }
    }
    let err = json
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error");
    Err(format!("Failed to upload Radius artifact: {err}"))
}

/// Try Radius first (when a bearer token is present), then a secret gist.
pub fn share_session(
    jsonl_path: &Path,
    html_path: &Path,
    radius_token: Option<&str>,
    gateway: Option<&str>,
) -> Result<ShareResult, String> {
    if let Some(token) = radius_token.filter(|s| !s.is_empty()) {
        match share_via_radius(jsonl_path, token, gateway) {
            Ok(result) => return Ok(result),
            Err(err) if err.contains("Network disabled") => {}
            Err(err) => return Err(err),
        }
    }
    share_via_gist(html_path)
}

pub fn share_via_radius(
    jsonl_path: &Path,
    token: &str,
    gateway: Option<&str>,
) -> Result<ShareResult, String> {
    if std::env::var("PI_DISABLE_NETWORK").ok().as_deref() == Some("1") {
        return Err("Network disabled (PI_DISABLE_NETWORK=1)".into());
    }
    let gateway = gateway.unwrap_or(pi_ai::DEFAULT_RADIUS_GATEWAY);
    let url = radius_artifact_url(gateway);
    let body = std::fs::read(jsonl_path).map_err(|e| format!("Failed to export session: {e}"))?;
    let response = ureq::post(&url)
        .set("authorization", &format!("Bearer {token}"))
        .set("content-type", "application/x-ndjson")
        .send_bytes(&body)
        .map_err(|e| format!("Failed to upload Radius artifact: {e}"))?;
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    let share_url = parse_radius_artifact_response(status, &text)?;
    Ok(ShareResult {
        gist_url: String::new(),
        preview_url: share_url,
    })
}

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

    #[test]
    fn radius_url_and_fixture_response_match_ts() {
        assert_eq!(
            radius_artifact_url("https://radius.pi.dev"),
            "https://radius.pi.dev/v1/artifacts?visibility=organization&title=Pi%20session"
        );
        assert_eq!(
            parse_radius_artifact_response(
                200,
                r#"{"artifact":{"canonical_url":"https://radius.pi.dev/a/1"}}"#
            )
            .unwrap(),
            "https://radius.pi.dev/a/1"
        );
        assert!(parse_radius_artifact_response(400, r#"{"error":"nope"}"#)
            .unwrap_err()
            .contains("Failed to upload Radius artifact: nope"));
    }
}
