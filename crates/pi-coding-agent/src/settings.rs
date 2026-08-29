use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub quiet_startup: bool,
    #[serde(default)]
    pub trusted_projects: Vec<String>,
    #[serde(default)]
    pub double_escape_action: Option<String>,
    #[serde(default)]
    pub autocomplete_max_visible: Option<u32>,
    #[serde(default, rename = "treeFilterMode")]
    pub tree_filter_mode: Option<String>,
    #[serde(default)]
    pub markdown: MarkdownSettings,
    #[serde(default, rename = "enableAnalytics")]
    pub enable_analytics: Option<bool>,
    #[serde(default, rename = "trackingId")]
    pub tracking_id: Option<String>,
    #[serde(default, rename = "enabledModels")]
    pub enabled_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownSettings {
    #[serde(default)]
    pub mermaid: Option<String>,
}

pub fn settings_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("settings.json")
}

pub fn load_settings(agent_dir: &Path) -> Settings {
    fs::read_to_string(settings_path(agent_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings(agent_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    fs::write(
        settings_path(agent_dir),
        serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|err| err.to_string())
}

pub fn should_run_first_time_setup(settings_path: &Path) -> bool {
    if std::env::var("PI_EXPERIMENTAL").ok().as_deref() != Some("1") {
        return false;
    }
    if std::env::var("PI_CODING_AGENT_DIR").is_ok() {
        return false;
    }
    !settings_path.exists()
}

pub fn set_enable_analytics(settings: &mut Settings, enabled: bool) {
    settings.enable_analytics = Some(enabled);
    if enabled && settings.tracking_id.is_none() {
        settings.tracking_id = Some(Uuid::new_v4().to_string());
    }
}

pub fn is_trusted(settings: &Settings, cwd: &Path, override_trust: Option<bool>) -> bool {
    if let Some(value) = override_trust {
        return value;
    }
    let canonical = cwd.to_string_lossy().into_owned();
    settings.trusted_projects.iter().any(|p| p == &canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_time_setup_gate_matches_ts() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("settings.json");
        std::env::set_var("PI_EXPERIMENTAL", "1");
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert!(should_run_first_time_setup(&path));
        std::env::remove_var("PI_EXPERIMENTAL");
        assert!(!should_run_first_time_setup(&path));
        std::env::set_var("PI_EXPERIMENTAL", "1");
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        assert!(!should_run_first_time_setup(&path));
        std::env::remove_var("PI_CODING_AGENT_DIR");
        std::fs::write(&path, "{}").expect("write");
        assert!(!should_run_first_time_setup(&path));
        std::env::remove_var("PI_EXPERIMENTAL");
    }

    #[test]
    fn analytics_generates_tracking_id_on_opt_in() {
        let mut settings = Settings::default();
        set_enable_analytics(&mut settings, false);
        assert_eq!(settings.enable_analytics, Some(false));
        assert!(settings.tracking_id.is_none());
        set_enable_analytics(&mut settings, true);
        assert_eq!(settings.enable_analytics, Some(true));
        let id = settings.tracking_id.clone().expect("tracking id");
        assert_eq!(id.len(), 36);
        set_enable_analytics(&mut settings, false);
        set_enable_analytics(&mut settings, true);
        assert_eq!(settings.tracking_id.as_deref(), Some(id.as_str()));
    }
}
