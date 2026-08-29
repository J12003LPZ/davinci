use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default, rename = "sessionDir")]
    pub session_dir: Option<String>,
    #[serde(default)]
    pub trusted: bool,
}

pub fn load_settings(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn trust_store_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("trusted-projects.json")
}

pub fn is_trusted(agent_dir: &Path, cwd: &Path) -> bool {
    let path = trust_store_path(agent_dir);
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|v| v.as_str() == Some(&cwd.to_string_lossy()))
        })
        .unwrap_or(false)
}

pub fn save_trust(agent_dir: &Path, cwd: &Path) {
    let path = trust_store_path(agent_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut items = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default();
    let cwd = cwd.to_string_lossy().to_string();
    if !items.contains(&cwd) {
        items.push(cwd);
    }
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()),
    );
}
