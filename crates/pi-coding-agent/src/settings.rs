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

    pub fn trusted(&self) -> bool {
        self.value
            .get("trusted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
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
}

pub fn trust_store_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("trusted-projects.json")
}

pub fn is_trusted(agent_dir: &Path, cwd: &Path) -> bool {
    let path = trust_store_path(agent_dir);
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
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

#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
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
