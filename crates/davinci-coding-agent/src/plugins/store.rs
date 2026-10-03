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

pub fn load(agent_dir: &Path) -> Result<InstalledFile, String> {
    read_json(&installed_path(agent_dir))
}

fn save(agent_dir: &Path, file: &InstalledFile) -> Result<(), String> {
    let mut file = file.clone();
    file.version = 1;
    write_json_atomic(&installed_path(agent_dir), &file)
}

pub fn update<T>(
    agent_dir: &Path,
    change: impl FnOnce(&mut InstalledFile) -> Result<T, String>,
) -> Result<T, String> {
    let _lock = lock_registry(&installed_path(agent_dir))?;
    let mut file = load(agent_dir)?;
    let value = change(&mut file)?;
    save(agent_dir, &file)?;
    Ok(value)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    davinci_sys::fs::atomic_write(path, body.as_bytes())
        .map_err(|err| format!("could not save {}: {err}", path.display()))
}

/// Only absence means an empty registry. Corrupt or unreadable state must
/// never be replaced with defaults by a subsequent write.
pub(super) fn read_json<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
) -> Result<T, String> {
    let body = match std::fs::read(path) {
        Ok(body) => body,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(err) => return Err(format!("could not read {}: {err}", path.display())),
    };
    let parse = || {
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|err| err.to_string())?;
        if !value.is_object() {
            return Err("expected a JSON object".to_string());
        }
        serde_json::from_value(value).map_err(|err| err.to_string())
    };
    parse().map_err(|err| {
        format!(
            "invalid registry {}: {err}; restore or repair this file before retrying",
            path.display()
        )
    })
}

pub(super) fn lock_registry(path: &Path) -> Result<davinci_sys::lock::ExclusiveFileLock, String> {
    davinci_sys::lock::ExclusiveFileLock::acquire(
        &davinci_sys::lock::lock_path_for(path),
        std::time::Duration::from_secs(10),
    )
    .map_err(|err| format!("could not lock {}: {err}", path.display()))
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
    fn audit_corrupt_registry_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = installed_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in ["{broken", "[]", "{\"plugins\":false}"] {
            std::fs::write(&path, body).unwrap();
            let result = update(dir.path(), |_| Ok(()));
            assert!(result.is_err(), "corrupt registry was accepted: {body}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(update(dir.path(), |_| Ok(())).is_err());
    }

    #[test]
    fn audit_concurrent_registry_changes_survive() {
        let dir = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let barrier = barrier.clone();
                    let path = dir.path();
                    scope.spawn(move || {
                        barrier.wait();
                        update(path, |file| {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                            file.plugins.insert(
                                format!("plugin{index}@market"),
                                InstalledPlugin {
                                    origin: Origin::Davinci,
                                    install_path: None,
                                    version: None,
                                    enabled: false,
                                    hooks_approved: None,
                                    installed_at: 0,
                                },
                            );
                            Ok(())
                        })
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        });
        assert_eq!(load(dir.path()).unwrap().plugins.len(), 8);
    }

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
        let file = load(dir.path()).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.resolve_key("alpha").unwrap(), "alpha@one");
        assert_eq!(file.resolve_key("beta@two").unwrap(), "beta@two");
        assert!(file.resolve_key("beta").unwrap_err().contains("ambiguous"));
        assert!(file.resolve_key("gamma").is_err());
    }

    #[test]
    fn audit_registry_process_worker() {
        let Some(root) = std::env::var_os("DAVINCI_TEST_PLUGIN_TRANSACTION") else {
            return;
        };
        let root = PathBuf::from(root);
        let key = std::env::var("DAVINCI_TEST_PLUGIN_KEY").unwrap();
        std::fs::write(root.join(format!("ready-{key}")), "").unwrap();
        let start = std::time::Instant::now();
        while !root.join("go").exists() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        update(&root, |file| {
            // Both children reach the transaction together; this amplifies
            // a missing lock without a barrier inside the critical section.
            std::thread::sleep(std::time::Duration::from_millis(100));
            let plugin = file.plugins.get_mut(&key).unwrap();
            plugin.enabled = false;
            plugin.hooks_approved = None;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn audit_two_processes_preserve_disables_and_revocations() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), |file| {
            for key in ["first", "second"] {
                file.plugins.insert(
                    key.into(),
                    InstalledPlugin {
                        origin: Origin::Davinci,
                        install_path: None,
                        version: None,
                        enabled: true,
                        hooks_approved: Some("approved".into()),
                        installed_at: 0,
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let mut children: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|key| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "plugins::store::tests::audit_registry_process_worker",
                        "--nocapture",
                    ])
                    .env("DAVINCI_TEST_PLUGIN_TRANSACTION", dir.path())
                    .env("DAVINCI_TEST_PLUGIN_KEY", key)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        let start = std::time::Instant::now();
        while !["first", "second"]
            .iter()
            .all(|key| dir.path().join(format!("ready-{key}")).exists())
        {
            if start.elapsed() >= std::time::Duration::from_secs(10) {
                for child in &mut children {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                panic!("registry workers did not become ready");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::write(dir.path().join("go"), "").unwrap();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let file = load(dir.path()).unwrap();
        assert_eq!(file.plugins.len(), 2);
        assert!(file
            .plugins
            .values()
            .all(|plugin| !plugin.enabled && plugin.hooks_approved.is_none()));
    }
}
