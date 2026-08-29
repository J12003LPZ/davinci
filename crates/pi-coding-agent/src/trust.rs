//! Project trust store matching `vendor/pi/packages/coding-agent/src/core/trust-manager.ts`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::settings::with_settings_lock;

const TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES: &[&str] = &[
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustStoreEntry {
    pub path: String,
    pub decision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustUpdate {
    pub path: String,
    pub decision: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustOption {
    pub label: String,
    pub trusted: bool,
    pub updates: Vec<ProjectTrustUpdate>,
    pub saved_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectTrustStore {
    trust_path: PathBuf,
}

impl ProjectTrustStore {
    pub fn open(agent_dir: &Path) -> Self {
        Self {
            trust_path: agent_dir.join("trust.json"),
        }
    }

    pub fn get(&self, cwd: &Path) -> Option<bool> {
        self.get_entry(cwd).map(|entry| entry.decision)
    }

    pub fn get_entry(&self, cwd: &Path) -> Option<ProjectTrustStoreEntry> {
        with_settings_lock(&self.trust_path, || {
            let data = read_trust_file(&self.trust_path)?;
            Ok(find_nearest_trust_entry(&data, cwd))
        })
        .ok()
        .flatten()
    }

    #[allow(dead_code)]
    pub fn set(&self, cwd: &Path, decision: Option<bool>) -> Result<(), String> {
        self.set_many(&[ProjectTrustUpdate {
            path: canonicalize_trust_path(cwd),
            decision,
        }])
    }

    pub fn set_many(&self, decisions: &[ProjectTrustUpdate]) -> Result<(), String> {
        if let Some(parent) = self.trust_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        with_settings_lock(&self.trust_path, || {
            let mut data = read_trust_file(&self.trust_path)?;
            for update in decisions {
                let key = canonicalize_trust_path(Path::new(&update.path));
                match update.decision {
                    None => {
                        data.remove(&key);
                    }
                    Some(value) => {
                        data.insert(key, Value::Bool(value));
                    }
                }
            }
            write_trust_file(&self.trust_path, &data)
        })
    }
}

/// TS `resolveProjectTrusted` without the interactive ask / extension hook.
pub fn resolve_project_trusted(
    agent_dir: &Path,
    cwd: &Path,
    override_trust: Option<bool>,
    default_project_trust: Option<&str>,
    trusted_projects: &[String],
) -> bool {
    if let Some(value) = override_trust {
        return value;
    }
    if !has_trust_requiring_project_resources(cwd) {
        return true;
    }
    let store = ProjectTrustStore::open(agent_dir);
    if let Some(decision) = store.get(cwd) {
        return decision;
    }
    let canonical = canonicalize_trust_path(cwd);
    let display = cwd.to_string_lossy();
    if trusted_projects
        .iter()
        .any(|path| path == &canonical || path.as_str() == display.as_ref())
    {
        return true;
    }
    matches!(default_project_trust.unwrap_or("ask"), "always")
}

pub fn canonicalize_trust_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub fn get_project_trust_parent_path(cwd: &Path) -> Option<String> {
    let trust_path = PathBuf::from(canonicalize_trust_path(cwd));
    let parent = trust_path.parent()?;
    if parent == trust_path {
        None
    } else {
        Some(parent.to_string_lossy().into_owned())
    }
}

pub fn get_project_trust_options(
    cwd: &Path,
    include_session_only: bool,
) -> Vec<ProjectTrustOption> {
    let trust_path = canonicalize_trust_path(cwd);
    let mut options = vec![ProjectTrustOption {
        label: "Trust".into(),
        trusted: true,
        updates: vec![ProjectTrustUpdate {
            path: trust_path.clone(),
            decision: Some(true),
        }],
        saved_path: Some(trust_path.clone()),
    }];
    if let Some(parent_path) = get_project_trust_parent_path(cwd) {
        options.push(ProjectTrustOption {
            label: format!("Trust parent folder ({parent_path})"),
            trusted: true,
            updates: vec![
                ProjectTrustUpdate {
                    path: parent_path.clone(),
                    decision: Some(true),
                },
                ProjectTrustUpdate {
                    path: trust_path.clone(),
                    decision: None,
                },
            ],
            saved_path: Some(parent_path),
        });
    }
    if include_session_only {
        options.push(ProjectTrustOption {
            label: "Trust (this session only)".into(),
            trusted: true,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options.push(ProjectTrustOption {
        label: "Do not trust".into(),
        trusted: false,
        updates: vec![ProjectTrustUpdate {
            path: trust_path.clone(),
            decision: Some(false),
        }],
        saved_path: Some(trust_path),
    });
    if include_session_only {
        options.push(ProjectTrustOption {
            label: "Do not trust (this session only)".into(),
            trusted: false,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options
}

/// TS `hasTrustRequiringProjectResources`.
pub fn has_trust_requiring_project_resources(cwd: &Path) -> bool {
    let home = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs_home)
        .map(|path| canonicalize_trust_path(&path))
        .unwrap_or_default();
    let user_agents_skills = PathBuf::from(&home).join(".agents").join("skills");
    let user_agents_skills = canonicalize_trust_path(&user_agents_skills);
    let mut current = PathBuf::from(canonicalize_trust_path(cwd));
    let config_dir = current.join(".pi");
    if TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES
        .iter()
        .any(|entry| config_dir.join(entry).exists())
    {
        return true;
    }
    loop {
        let agents_skills = current.join(".agents").join("skills");
        let agents_skills = canonicalize_trust_path(&agents_skills);
        if agents_skills != user_agents_skills && Path::new(&agents_skills).exists() {
            return true;
        }
        let Some(parent) = current.parent() else {
            return false;
        };
        if parent == current {
            return false;
        }
        current = parent.to_path_buf();
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn read_trust_file(path: &Path) -> Result<BTreeMap<String, Value>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read trust store {}: {err}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let parsed: Value = serde_json::from_str(text)
        .map_err(|err| format!("Failed to read trust store {}: {err}", path.display()))?;
    let Some(object) = parsed.as_object() else {
        return Err(format!(
            "Invalid trust store {}: expected an object",
            path.display()
        ));
    };
    let mut data = BTreeMap::new();
    for (key, value) in object {
        if !value.is_boolean() && !value.is_null() {
            return Err(format!(
                "Invalid trust store {}: value for {key:?} must be true, false, or null",
                path.display()
            ));
        }
        data.insert(key.clone(), value.clone());
    }
    Ok(data)
}

fn write_trust_file(path: &Path, data: &BTreeMap<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut sorted = serde_json::Map::new();
    for (key, value) in data {
        if value.is_boolean() || value.is_null() {
            sorted.insert(key.clone(), value.clone());
        }
    }
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(sorted)).map_err(|err| err.to_string())?
    );
    fs::write(path, body).map_err(|err| err.to_string())
}

fn find_nearest_trust_entry(
    data: &BTreeMap<String, Value>,
    cwd: &Path,
) -> Option<ProjectTrustStoreEntry> {
    let mut current = PathBuf::from(canonicalize_trust_path(cwd));
    loop {
        let key = current.to_string_lossy().into_owned();
        if let Some(Value::Bool(decision)) = data.get(&key) {
            return Some(ProjectTrustStoreEntry {
                path: key,
                decision: *decision,
            });
        }
        let parent = current.parent()?;
        if parent == current {
            return None;
        }
        current = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stores_decisions_and_inherits_from_parent_directories() {
        let dir = tempdir().unwrap();
        let store = ProjectTrustStore::open(&dir.path().join("agent"));
        let parent = dir.path().join("trusted-parent");
        let child = parent.join("project");
        fs::create_dir_all(&child).unwrap();
        assert_eq!(store.get(&child), None);
        store.set(&parent, Some(true)).unwrap();
        assert_eq!(store.get(&child), Some(true));
        store.set(&child, Some(false)).unwrap();
        assert_eq!(store.get(&child), Some(false));
        store.set(&child, None).unwrap();
        assert_eq!(store.get(&child), Some(true));
    }

    #[test]
    fn detects_trust_requiring_project_resources() {
        let dir = tempdir().unwrap();
        let previous = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());
        let cwd = dir.path().join("project");
        fs::create_dir_all(dir.path().join(".pi").join("agent")).unwrap();
        fs::create_dir_all(dir.path().join(".agents").join("skills")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        assert!(!has_trust_requiring_project_resources(dir.path()));
        assert!(!has_trust_requiring_project_resources(&cwd));
        fs::write(dir.path().join(".pi").join("settings.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(dir.path()));
        let _ = fs::remove_file(dir.path().join(".pi").join("settings.json"));
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("settings.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
        let _ = fs::remove_dir_all(cwd.join(".pi"));
        fs::create_dir_all(cwd.join(".agents").join("skills")).unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
        match previous {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn trust_options_include_parent_folder() {
        let options = get_project_trust_options(Path::new("/parent/project"), false);
        assert_eq!(options[0].label, "Trust");
        assert!(options
            .iter()
            .any(|option| option.label == "Trust parent folder (/parent)"));
        assert!(options.iter().any(|option| option.label == "Do not trust"));
        assert!(!options
            .iter()
            .any(|option| option.label.contains("session only")));
        let with_session = get_project_trust_options(Path::new("/parent/project"), true);
        assert!(with_session
            .iter()
            .any(|option| option.label == "Trust (this session only)"));
    }

    #[test]
    fn resolve_uses_store_override_and_default() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("ask"),
            &[]
        ));
        assert!(resolve_project_trusted(
            &agent,
            &project,
            Some(true),
            Some("never"),
            &[]
        ));
        assert!(resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
        ProjectTrustStore::open(&agent)
            .set(&project, Some(false))
            .unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
    }
}
