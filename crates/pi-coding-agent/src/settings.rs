use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn agent_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
}

pub fn settings_path(local: bool) -> PathBuf {
    if local {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".pi")
            .join("settings.json")
    } else {
        agent_dir().join("settings.json")
    }
}

pub fn package_source_string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string).or_else(|| {
        value
            .get("source")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

#[derive(Debug, Clone)]
pub struct SettingsDocument {
    value: Value,
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self { value: json!({}) }
    }
}

impl SettingsDocument {
    pub fn load(path: &Path) -> Self {
        let value = fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({}));
        Self { value }
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            path,
            serde_json::to_string_pretty(&self.value).unwrap_or_else(|_| "{}".into()),
        );
    }

    pub fn packages(&self) -> Vec<Value> {
        self.value
            .get("packages")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default()
    }

    pub fn packages_mut(&mut self) -> &mut Vec<Value> {
        let obj = self
            .value
            .as_object_mut()
            .expect("settings document is an object");
        if !obj.contains_key("packages") || !obj["packages"].is_array() {
            obj.insert("packages".into(), json!([]));
        }
        obj.get_mut("packages")
            .and_then(|v| v.as_array_mut())
            .expect("packages is an array")
    }

    pub fn set_packages(&mut self, packages: Vec<Value>) {
        if let Some(obj) = self.value.as_object_mut() {
            obj.insert("packages".into(), json!(packages));
        }
    }

    pub fn resource_paths(&self, resource_type: &str) -> Vec<String> {
        self.value
            .get(resource_type)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_resource_paths(&mut self, resource_type: &str, paths: Vec<String>) {
        if let Some(obj) = self.value.as_object_mut() {
            obj.insert(resource_type.into(), json!(paths));
        }
    }

    pub fn npm_command(&self) -> Option<Vec<String>> {
        let args = self
            .value
            .get("npmCommand")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })?;
        if args.is_empty() || args.iter().all(|s| s.is_empty()) {
            None
        } else {
            Some(args)
        }
    }

    pub fn default_project_trust(&self) -> &'static str {
        match self
            .value
            .get("defaultProjectTrust")
            .and_then(|v| v.as_str())
        {
            Some("always") => "always",
            Some("never") => "never",
            _ => "ask",
        }
    }
}

const TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES: [&str; 7] = [
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

pub fn canonicalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

pub fn cwd_relative_path(file_path: &Path, cwd: &Path) -> Option<PathBuf> {
    let resolved_cwd = canonicalize_path(cwd);
    let resolved_path = canonicalize_path(file_path);
    if resolved_path == resolved_cwd {
        return Some(PathBuf::from("."));
    }
    resolved_path
        .strip_prefix(&resolved_cwd)
        .ok()
        .map(Path::to_path_buf)
}

pub fn has_trust_requiring_project_resources(cwd: &Path) -> bool {
    let current = canonicalize_path(cwd);
    let config_dir = current.join(".pi");
    if TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES
        .iter()
        .any(|entry| config_dir.join(entry).exists())
    {
        return true;
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let user_agents = canonicalize_path(&home.join(".agents").join("skills"));
    let mut dir = current;
    loop {
        let agents = canonicalize_path(&dir.join(".agents").join("skills"));
        if agents != user_agents && agents.exists() {
            return true;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return false,
        }
    }
}

pub fn trust_store_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("trust.json")
}

fn read_trust_file(agent_dir: &Path) -> Value {
    let path = trust_store_path(agent_dir);
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(|value: &Value| value.is_object())
        .unwrap_or_else(|| json!({}))
}

pub fn trust_decision(agent_dir: &Path, cwd: &Path) -> Option<bool> {
    let data = read_trust_file(agent_dir);
    let obj = data.as_object()?;
    let mut dir = canonicalize_path(cwd);
    loop {
        let key = dir.to_string_lossy();
        if let Some(value) = obj.get(key.as_ref()) {
            if let Some(decision) = value.as_bool() {
                return Some(decision);
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

pub fn is_trusted(agent_dir: &Path, cwd: &Path) -> bool {
    trust_decision(agent_dir, cwd) == Some(true)
}

#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn save_trust(agent_dir: &Path, cwd: &Path) {
    set_trust(agent_dir, cwd, Some(true));
}

pub fn set_trust(agent_dir: &Path, cwd: &Path, decision: Option<bool>) {
    let path = trust_store_path(agent_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut data = read_trust_file(agent_dir);
    let key = canonicalize_path(cwd).to_string_lossy().into_owned();
    if let Some(obj) = data.as_object_mut() {
        match decision {
            Some(value) => {
                obj.insert(key, json!(value));
            }
            None => {
                obj.remove(&key);
            }
        }
    }
    let _ = fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".into())
        ),
    );
}
