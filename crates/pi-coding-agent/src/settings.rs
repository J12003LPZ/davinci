use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

pub fn is_trusted(settings: &Settings, cwd: &Path, override_trust: Option<bool>) -> bool {
    if let Some(value) = override_trust {
        return value;
    }
    let canonical = cwd.to_string_lossy().into_owned();
    settings.trusted_projects.iter().any(|p| p == &canonical)
}
