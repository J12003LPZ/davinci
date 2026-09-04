//! Startup migrations matching `vendor/pi/packages/coding-agent/src/migrations.ts`.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use davinci_session::default_agent_dir;

const MIGRATION_GUIDE_URL: &str =
    "https://github.com/earendil-works/pi-mono/blob/main/packages/coding-agent/CHANGELOG.md#extensions-migration";
const EXTENSIONS_DOC_URL: &str =
    "https://github.com/earendil-works/pi-mono/blob/main/packages/coding-agent/docs/extensions.md";
const CONFIG_DIR_NAME: &str = ".pi";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationResult {
    pub migrated_auth_providers: Vec<String>,
    pub deprecation_warnings: Vec<String>,
    pub notes: Vec<String>,
}

pub fn project_config_dir(cwd: &Path) -> PathBuf {
    cwd.join(CONFIG_DIR_NAME)
}

pub fn run_migrations(cwd: &Path, agent_dir: &Path) -> MigrationResult {
    let mut result = MigrationResult {
        migrated_auth_providers: migrate_auth_to_auth_json(agent_dir),
        ..MigrationResult::default()
    };
    migrate_sessions_from_agent_root(agent_dir);
    migrate_tools_to_bin(agent_dir, &mut result);
    migrate_keybindings_config_file(agent_dir);
    let warnings = migrate_extension_system(cwd, agent_dir, &mut result);
    result.deprecation_warnings.extend(warnings);
    result
}

fn migrate_auth_to_auth_json(agent_dir: &Path) -> Vec<String> {
    let auth_path = agent_dir.join("auth.json");
    if auth_path.exists() {
        return Vec::new();
    }
    let mut migrated = serde_json::Map::new();
    let mut providers = Vec::new();
    let oauth_path = agent_dir.join("oauth.json");
    if oauth_path.exists() {
        if let Ok(raw) = fs::read_to_string(&oauth_path) {
            if let Ok(serde_json::Value::Object(oauth)) =
                serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}'))
            {
                for (provider, cred) in oauth {
                    let mut entry = serde_json::Map::new();
                    entry.insert("type".into(), serde_json::json!("oauth"));
                    if let serde_json::Value::Object(fields) = cred {
                        for (key, value) in fields {
                            entry.insert(key, value);
                        }
                    }
                    migrated.insert(provider.clone(), serde_json::Value::Object(entry));
                    providers.push(provider);
                }
                let _ = fs::rename(&oauth_path, agent_dir.join("oauth.json.migrated"));
            }
        }
    }
    let settings_path = agent_dir.join("settings.json");
    if settings_path.exists() {
        if let Ok(raw) = fs::read_to_string(&settings_path) {
            if let Ok(mut settings) =
                serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}'))
            {
                if let Some(keys) = settings
                    .get("apiKeys")
                    .and_then(serde_json::Value::as_object)
                {
                    for (provider, key) in keys {
                        if !migrated.contains_key(provider) {
                            if let Some(key) = key.as_str() {
                                migrated.insert(
                                    provider.clone(),
                                    serde_json::json!({ "type": "api_key", "key": key }),
                                );
                                providers.push(provider.clone());
                            }
                        }
                    }
                }
                if let Some(object) = settings.as_object_mut() {
                    if object.remove("apiKeys").is_some() {
                        let _ = fs::write(
                            &settings_path,
                            serde_json::to_string_pretty(&settings).unwrap_or_default(),
                        );
                    }
                }
            }
        }
    }
    if !migrated.is_empty() {
        let _ = fs::create_dir_all(agent_dir);
        let _ = fs::write(
            auth_path,
            serde_json::to_string_pretty(&serde_json::Value::Object(migrated)).unwrap_or_default(),
        );
    }
    providers
}

fn migrate_sessions_from_agent_root(agent_dir: &Path) {
    let Ok(entries) = fs::read_dir(agent_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(first) = content.lines().next() else {
            continue;
        };
        let Ok(header) = serde_json::from_str::<serde_json::Value>(first) else {
            continue;
        };
        if header.get("type").and_then(serde_json::Value::as_str) != Some("session") {
            continue;
        }
        let Some(cwd) = header.get("cwd").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let safe_path = format!(
            "--{}--",
            cwd.trim_start_matches(['/', '\\'])
                .replace(['/', '\\', ':'], "-")
        );
        let dest_dir = agent_dir.join("sessions").join(safe_path);
        let Some(name) = path.file_name() else {
            continue;
        };
        let dest = dest_dir.join(name);
        if dest.exists() {
            continue;
        }
        let _ = fs::create_dir_all(&dest_dir);
        let _ = fs::rename(&path, dest);
    }
}

fn migrate_tools_to_bin(agent_dir: &Path, result: &mut MigrationResult) {
    let tools_dir = agent_dir.join("tools");
    if !tools_dir.exists() {
        return;
    }
    let bin_dir = agent_dir.join("bin");
    let binaries = ["fd", "rg", "fd.exe", "rg.exe"];
    let mut moved_any = false;
    for bin in binaries {
        let old_path = tools_dir.join(bin);
        if !old_path.exists() {
            continue;
        }
        let new_path = bin_dir.join(bin);
        let _ = fs::create_dir_all(&bin_dir);
        if new_path.exists() {
            let _ = fs::remove_file(&old_path);
        } else if fs::rename(&old_path, &new_path).is_ok() {
            moved_any = true;
        }
    }
    if moved_any {
        result
            .notes
            .push("Migrated managed binaries tools/ → bin/".into());
    }
}

fn migrate_keybindings_config_file(agent_dir: &Path) {
    let path = agent_dir.join("keybindings.json");
    if !path.exists() {
        return;
    }
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}'))
    else {
        return;
    };
    if !parsed.is_object() {
        return;
    }
    let _ = parsed;
}

fn migrate_commands_to_prompts(base_dir: &Path, label: &str, result: &mut MigrationResult) {
    let commands = base_dir.join("commands");
    let prompts = base_dir.join("prompts");
    if commands.exists() && !prompts.exists() {
        match fs::rename(&commands, &prompts) {
            Ok(()) => result
                .notes
                .push(format!("Migrated {label} commands/ → prompts/")),
            Err(err) => result.notes.push(format!(
                "Warning: Could not migrate {label} commands/ to prompts/: {err}"
            )),
        }
    }
}

fn check_deprecated_extension_dirs(base_dir: &Path, label: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if base_dir.join("hooks").exists() {
        warnings.push(format!(
            "{label} hooks/ directory found. Hooks have been renamed to extensions."
        ));
    }
    let tools_dir = base_dir.join("tools");
    if tools_dir.exists() {
        if let Ok(entries) = fs::read_dir(&tools_dir) {
            let custom = entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let lower = name.to_lowercase();
                lower != "fd"
                    && lower != "rg"
                    && lower != "fd.exe"
                    && lower != "rg.exe"
                    && !name.starts_with('.')
            });
            if custom {
                warnings.push(format!(
                    "{label} tools/ directory contains custom tools. Custom tools have been merged into extensions."
                ));
            }
        }
    }
    warnings
}

fn migrate_extension_system(
    cwd: &Path,
    agent_dir: &Path,
    result: &mut MigrationResult,
) -> Vec<String> {
    let project_dir = project_config_dir(cwd);
    migrate_commands_to_prompts(agent_dir, "Global", result);
    migrate_commands_to_prompts(&project_dir, "Project", result);
    let mut warnings = check_deprecated_extension_dirs(agent_dir, "Global");
    warnings.extend(check_deprecated_extension_dirs(&project_dir, "Project"));
    warnings
}

pub fn format_deprecation_warnings(warnings: &[String]) -> String {
    let mut lines = Vec::new();
    for warning in warnings {
        lines.push(format!("Warning: {warning}"));
    }
    lines.push(String::new());
    lines.push("Move your extensions to the extensions/ directory.".into());
    lines.push(format!("Migration guide: {MIGRATION_GUIDE_URL}"));
    lines.push(format!("Documentation: {EXTENSIONS_DOC_URL}"));
    lines.push(String::new());
    lines.push("Press any key to continue...".into());
    lines.join("\n")
}

pub fn show_deprecation_warnings(warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    println!("{}", format_deprecation_warnings(warnings));
    if std::env::var("PI_DEPRECATION_SKIP").is_ok() || !stdin_is_interactive() {
        return;
    }
    wait_for_keypress();
    println!();
}

fn stdin_is_interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal()
}

fn wait_for_keypress() {
    if let Ok(raw) = std::env::var("PI_DEPRECATION_REPLY") {
        let _ = raw;
        return;
    }
    let mut buf = [0u8; 1];
    let _ = io::stdin().read(&mut buf);
}

pub fn maybe_run_startup_migrations(cwd: &Path) -> MigrationResult {
    let agent_dir = if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        PathBuf::from(dir)
    } else {
        default_agent_dir()
    };
    run_migrations(cwd, &agent_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn deprecation_warnings_lock_ts_strings() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project").join(".pi");
        fs::create_dir_all(agent.join("hooks")).unwrap();
        fs::create_dir_all(agent.join("tools")).unwrap();
        fs::write(agent.join("tools").join("fd"), "").unwrap();
        fs::create_dir_all(project.join("tools")).unwrap();
        fs::write(project.join("tools").join("custom.js"), "").unwrap();
        fs::create_dir_all(agent.join("commands")).unwrap();
        let result = run_migrations(&dir.path().join("project"), &agent);
        assert!(result.deprecation_warnings.contains(
            &"Global hooks/ directory found. Hooks have been renamed to extensions.".into()
        ));
        assert!(result.deprecation_warnings.contains(
            &"Project tools/ directory contains custom tools. Custom tools have been merged into extensions.".into()
        ));
        assert!(!result
            .deprecation_warnings
            .iter()
            .any(|item| item.contains("Global tools/")));
        assert!(agent.join("prompts").exists());
        assert!(agent.join("bin").join("fd").exists());
        assert!(!agent.join("tools").join("fd").exists());
        let text = format_deprecation_warnings(&result.deprecation_warnings);
        assert!(text.contains("Warning: Global hooks/ directory found."));
        assert!(text.contains("Move your extensions to the extensions/ directory."));
        assert!(text.contains(MIGRATION_GUIDE_URL));
        assert!(text.contains("Press any key to continue..."));
    }
}
