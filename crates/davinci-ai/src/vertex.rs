//! Google Vertex ADC / API-key resolution matching `api/google-vertex.ts`.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub const GCP_VERTEX_CREDENTIALS_MARKER: &str = "gcp-vertex-credentials";

pub fn is_placeholder_api_key(api_key: &str) -> bool {
    api_key.len() >= 3 && api_key.starts_with('<') && api_key.ends_with('>')
}

pub fn resolve_vertex_api_key(api_key: Option<&str>) -> Option<String> {
    let key = api_key?.trim();
    if key.is_empty() || key == GCP_VERTEX_CREDENTIALS_MARKER || is_placeholder_api_key(key) {
        None
    } else {
        Some(key.to_string())
    }
}

pub fn resolve_vertex_project(explicit: Option<&str>) -> Result<String, String> {
    if let Some(project) = explicit.filter(|s| !s.is_empty()) {
        return Ok(project.to_string());
    }
    std::env::var("GOOGLE_CLOUD_PROJECT")
        .or_else(|_| std::env::var("GCLOUD_PROJECT"))
        .map_err(|_| {
            "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT/GCLOUD_PROJECT or pass project in options."
                .to_string()
        })
}

pub fn resolve_vertex_location(explicit: Option<&str>) -> Result<String, String> {
    if let Some(location) = explicit.filter(|s| !s.is_empty()) {
        return Ok(location.to_string());
    }
    std::env::var("GOOGLE_CLOUD_LOCATION").map_err(|_| {
        "Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION or pass location in options."
            .to_string()
    })
}

pub fn application_default_credentials_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        return Some(PathBuf::from(path));
    }
    dirs::home_dir().map(|home| home.join(".config/gcloud/application_default_credentials.json"))
}

/// Fixture-friendly ADC: use `token` / `access_token` from the credentials JSON.
pub fn load_adc_access_token() -> Option<String> {
    let path = application_default_credentials_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value
        .get("token")
        .or_else(|| value.get("access_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn resolve_vertex_auth(api_key: Option<&str>) -> Result<VertexAuth, String> {
    if let Some(key) = resolve_vertex_api_key(api_key) {
        return Ok(VertexAuth::ApiKey(key));
    }
    let project = resolve_vertex_project(None)?;
    let location = resolve_vertex_location(None)?;
    let token = load_adc_access_token();
    Ok(VertexAuth::Adc {
        project,
        location,
        token,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexAuth {
    ApiKey(String),
    Adc {
        project: String,
        location: String,
        token: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn marker_falls_back_to_adc() {
        let _guard = ENV_LOCK.lock().unwrap();
        assert_eq!(
            resolve_vertex_api_key(Some(GCP_VERTEX_CREDENTIALS_MARKER)),
            None
        );
        assert_eq!(resolve_vertex_api_key(Some("<authenticated>")), None);
        assert_eq!(
            resolve_vertex_api_key(Some("vertex-key")),
            Some("vertex-key".into())
        );
    }

    #[test]
    fn project_and_location_match_ts_errors() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("GCLOUD_PROJECT");
        std::env::remove_var("GOOGLE_CLOUD_LOCATION");
        assert!(resolve_vertex_project(None)
            .unwrap_err()
            .contains("Vertex AI requires a project ID"));
        assert!(resolve_vertex_location(None)
            .unwrap_err()
            .contains("Vertex AI requires a location"));
        assert_eq!(resolve_vertex_project(Some("p")).unwrap(), "p");
        assert_eq!(
            resolve_vertex_location(Some("us-central1")).unwrap(),
            "us-central1"
        );
    }

    #[test]
    fn adc_reads_fixture_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let path = dir.path().join("adc.json");
        fs::write(&path, r#"{"access_token":"ya29.fixture"}"#).unwrap();
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &path);
        assert_eq!(load_adc_access_token().as_deref(), Some("ya29.fixture"));
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
    }
}
