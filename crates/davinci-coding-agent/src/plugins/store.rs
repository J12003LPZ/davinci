//! `<agent_dir>/plugins/installed.json`: which plugins DaVinci loads.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Installed by DaVinci into its own cache.
    Davinci,
    /// Adopted from Claude Code; the path is resolved on every load.
    Claude,
    /// Adopted from Codex; the path is resolved on every load.
    Codex,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Davinci => "davinci",
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub origin: Origin,
    /// Set for `davinci` installs only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    /// Digest of the approved hook set; hooks run only while it matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks_approved: Option<String>,
    #[serde(default)]
    pub installed_at: u64,
}

fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    #[serde(default)]
    pub version: u32,
    /// Keyed by `<plugin>@<marketplace>`.
    #[serde(default)]
    pub plugins: BTreeMap<String, InstalledPlugin>,
}

impl InstalledFile {
    /// The key a user meant: an exact `name@marketplace`, or a bare name
    /// that matches exactly one installed plugin.
    pub fn resolve_key(&self, wanted: &str) -> Result<String, String> {
        if self.plugins.contains_key(wanted) {
            return Ok(wanted.to_string());
        }
        let matches: Vec<&String> = self
            .plugins
            .keys()
            .filter(|key| split_key(key).0 == wanted)
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(format!("no installed plugin named {wanted}")),
            many => Err(format!(
                "{wanted} is ambiguous: {}",
                many.iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

pub fn split_key(key: &str) -> (&str, &str) {
    key.split_once('@').unwrap_or((key, ""))
}

pub fn plugins_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join("plugins")
}

fn installed_path(agent_dir: &Path) -> PathBuf {
    plugins_dir(agent_dir).join("installed.json")
}

pub fn load(agent_dir: &Path) -> InstalledFile {
    std::fs::read_to_string(installed_path(agent_dir))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

pub fn save(agent_dir: &Path, file: &InstalledFile) -> Result<(), String> {
    let mut file = file.clone();
    file.version = 1;
    write_json_atomic(&installed_path(agent_dir), &file)
}

pub fn update<T>(
    agent_dir: &Path,
    change: impl FnOnce(&mut InstalledFile) -> Result<T, String>,
) -> Result<T, String> {
    let mut file = load(agent_dir);
    let value = change(&mut file)?;
    save(agent_dir, &file)?;
    Ok(value)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let body = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id()
    ));
    std::fs::write(&tmp, body).map_err(|err| err.to_string())?;
    std::fs::rename(&tmp, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        err.to_string()
    })
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_resolves_bare_names() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), |file| {
            for key in ["alpha@one", "beta@one", "beta@two"] {
                file.plugins.insert(
                    key.into(),
                    InstalledPlugin {
                        origin: Origin::Davinci,
                        install_path: None,
                        version: None,
                        enabled: true,
                        hooks_approved: None,
                        installed_at: 1,
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let file = load(dir.path());
        assert_eq!(file.version, 1);
        assert_eq!(file.resolve_key("alpha").unwrap(), "alpha@one");
        assert_eq!(file.resolve_key("beta@two").unwrap(), "beta@two");
        assert!(file.resolve_key("beta").unwrap_err().contains("ambiguous"));
        assert!(file.resolve_key("gamma").is_err());
    }
}
