//! Plugins and marketplaces that Claude Code and Codex already installed.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.
//!
//! Everything here is read-only. Claude Code keeps
//! `$CLAUDE_CONFIG_DIR/plugins/installed_plugins.json` (default `~/.claude`);
//! Codex keeps `$CODEX_HOME/plugins/cache/<marketplace>/<plugin>/<version>/`
//! and enables plugins in `config.toml` (default `~/.codex`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::store::Origin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPlugin {
    pub key: String,
    pub origin: Origin,
    pub path: PathBuf,
    pub version: Option<String>,
    /// Whether the owning tool currently has it enabled.
    pub enabled_there: bool,
}

pub fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| davinci_session::home_dir().map(|home| home.join(".claude")))
}

pub fn codex_dir() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| davinci_session::home_dir().map(|home| home.join(".codex")))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Strip the `\\?\` verbatim prefix Codex writes into `config.toml`.
fn plain_path(text: &str) -> PathBuf {
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text))
}

pub fn claude_plugins() -> Vec<ExternalPlugin> {
    let Some(dir) = claude_dir() else {
        return Vec::new();
    };
    let Some(installed) = read_json(&dir.join("plugins").join("installed_plugins.json")) else {
        return Vec::new();
    };
    let enabled = read_json(&dir.join("settings.json"))
        .and_then(|settings| settings.get("enabledPlugins").cloned())
        .unwrap_or(Value::Null);
    let mut out = Vec::new();
    for (key, entry) in installed
        .get("plugins")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        // Version 2 keeps a list of installs (per scope); version 1 one object.
        let records: Vec<&Value> = match entry {
            Value::Array(items) => items.iter().collect(),
            object @ Value::Object(_) => vec![object],
            _ => continue,
        };
        let record = records
            .iter()
            .find(|record| record.get("scope").and_then(Value::as_str) == Some("user"))
            .or_else(|| records.first());
        let Some(record) = record else {
            continue;
        };
        let Some(path) = record.get("installPath").and_then(Value::as_str) else {
            continue;
        };
        let path = plain_path(path);
        if !path.is_dir() {
            continue;
        }
        out.push(ExternalPlugin {
            key: key.clone(),
            origin: Origin::Claude,
            path,
            version: record
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string),
            enabled_there: enabled.get(key).and_then(Value::as_bool).unwrap_or(true),
        });
    }
    out
}

fn codex_config() -> Option<toml_edit::DocumentMut> {
    let body = std::fs::read_to_string(codex_dir()?.join("config.toml")).ok()?;
    body.parse().ok()
}

pub fn codex_plugins() -> Vec<ExternalPlugin> {
    let Some(dir) = codex_dir() else {
        return Vec::new();
    };
    let config = codex_config();
    let enabled_in_config = |key: &str| -> bool {
        config
            .as_ref()
            .and_then(|doc| doc.get("plugins"))
            .and_then(|plugins| plugins.get(key))
            .and_then(|entry| entry.get("enabled"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    };
    let cache = dir.join("plugins").join("cache");
    let mut out = Vec::new();
    for marketplace in sorted_dirs(&cache) {
        let Some(mkt) = file_name(&marketplace) else {
            continue;
        };
        for plugin_dir in sorted_dirs(&marketplace) {
            let Some(name) = file_name(&plugin_dir) else {
                continue;
            };
            let Some(version_dir) = newest_dir(&plugin_dir) else {
                continue;
            };
            let key = format!("{name}@{mkt}");
            out.push(ExternalPlugin {
                enabled_there: enabled_in_config(&key),
                version: file_name(&version_dir),
                key,
                origin: Origin::Codex,
                path: version_dir,
            });
        }
    }
    out
}

/// All plugins importable from both tools.
pub fn all_plugins() -> Vec<ExternalPlugin> {
    let mut all = claude_plugins();
    all.extend(codex_plugins());
    all
}

/// The live location of an adopted plugin.
pub fn resolve(origin: Origin, key: &str) -> Option<ExternalPlugin> {
    let candidates = match origin {
        Origin::Claude => claude_plugins(),
        Origin::Codex => codex_plugins(),
        Origin::Davinci => return None,
    };
    candidates.into_iter().find(|plugin| plugin.key == key)
}

/// Marketplace directories Claude Code and Codex already keep on disk.
pub fn marketplace_dirs() -> Vec<(String, PathBuf, &'static str)> {
    let mut out = Vec::new();
    if let Some(known) =
        claude_dir().and_then(|dir| read_json(&dir.join("plugins").join("known_marketplaces.json")))
    {
        for (name, entry) in known.as_object().into_iter().flatten() {
            if let Some(location) = entry.get("installLocation").and_then(Value::as_str) {
                out.push((name.clone(), plain_path(location), "claude"));
            }
        }
    }
    if let Some(doc) = codex_config() {
        if let Some(table) = doc
            .get("marketplaces")
            .and_then(|item| item.as_table_like())
        {
            for (name, entry) in table.iter() {
                let local = entry.get("source_type").and_then(|v| v.as_str()) == Some("local");
                if let (true, Some(source)) = (local, entry.get("source").and_then(|v| v.as_str()))
                {
                    out.push((name.to_string(), plain_path(source), "codex"));
                }
            }
        }
    }
    if let Some(dir) = codex_dir() {
        for path in sorted_dirs(&dir.join(".tmp").join("marketplaces")) {
            if let Some(name) = file_name(&path) {
                out.push((name, path, "codex"));
            }
        }
    }
    out.retain(|(_, path, _)| path.is_dir());
    out
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| !file_name(path).is_some_and(|name| name.starts_with('.')))
        .collect();
    dirs.sort();
    dirs
}

/// The most recently modified version directory.
fn newest_dir(dir: &Path) -> Option<PathBuf> {
    sorted_dirs(dir).into_iter().max_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that point `CLAUDE_CONFIG_DIR` / `CODEX_HOME` at a
    /// temporary directory.
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct FakeHomes {
        pub claude: PathBuf,
        pub codex: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl FakeHomes {
        pub(crate) fn new(base: &Path) -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let claude = base.join("claude");
            let codex = base.join("codex");
            std::fs::create_dir_all(&claude).unwrap();
            std::fs::create_dir_all(&codex).unwrap();
            std::env::set_var("CLAUDE_CONFIG_DIR", &claude);
            std::env::set_var("CODEX_HOME", &codex);
            Self {
                claude,
                codex,
                _guard: guard,
            }
        }
    }

    impl Drop for FakeHomes {
        fn drop(&mut self) {
            std::env::remove_var("CLAUDE_CONFIG_DIR");
            std::env::remove_var("CODEX_HOME");
        }
    }

    pub(crate) fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn reads_claude_and_codex_installs() {
        let dir = tempfile::tempdir().unwrap();
        let homes = FakeHomes::new(dir.path());
        let claude_plugin = homes.claude.join("plugins/cache/mk/alpha/1.0.0");
        write(
            &claude_plugin.join(".claude-plugin/plugin.json"),
            r#"{"name":"alpha"}"#,
        );
        write(
            &homes.claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {
                "alpha@mk": [{"scope": "user", "installPath": claude_plugin, "version": "1.0.0"}],
                "gone@mk": [{"scope": "user", "installPath": dir.path().join("missing")}]
            }})
            .to_string(),
        );
        write(
            &homes.claude.join("settings.json"),
            r#"{"enabledPlugins":{"alpha@mk":false}}"#,
        );
        write(
            &homes
                .codex
                .join("plugins/cache/cmk/beta/2.0.0/.codex-plugin/plugin.json"),
            r#"{"name":"beta"}"#,
        );
        let local_mkt = dir.path().join("local-mkt");
        std::fs::create_dir_all(&local_mkt).unwrap();
        write(
            &homes.codex.join("config.toml"),
            &format!(
                "[plugins.\"beta@cmk\"]\nenabled = true\n\n[marketplaces.local-one]\nsource_type = \"local\"\nsource = '{}'\n",
                local_mkt.display()
            ),
        );

        let claude = claude_plugins();
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].key, "alpha@mk");
        assert!(!claude[0].enabled_there);
        let codex = codex_plugins();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].key, "beta@cmk");
        assert_eq!(codex[0].version.as_deref(), Some("2.0.0"));
        assert!(codex[0].enabled_there);
        assert_eq!(
            resolve(Origin::Codex, "beta@cmk").unwrap().path,
            codex[0].path
        );
        let markets = marketplace_dirs();
        assert!(markets
            .iter()
            .any(|(name, _, from)| name == "local-one" && *from == "codex"));
    }
}
