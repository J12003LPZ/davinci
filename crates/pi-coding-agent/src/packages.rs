use std::fs;
use std::path::Path;

use crate::settings::{load_settings, save_settings, Settings};

pub fn handle_package_command(
    command: &str,
    args: &[String],
    agent_dir: &Path,
) -> Result<String, String> {
    let local = args.iter().any(|a| a == "-l" || a == "--local");
    let source = args.iter().find(|a| !a.starts_with('-')).cloned();
    let mut settings = load_settings(agent_dir);
    match command {
        "install" => {
            let source = source.ok_or("install <source> [-l]")?;
            if !settings.extensions.contains(&source) {
                settings.extensions.push(source.clone());
            }
            save_settings(agent_dir, &settings)?;
            Ok(format!("Installed {source}{}", scope(local)))
        }
        "remove" | "uninstall" => {
            let source = source.ok_or("remove <source> [-l]")?;
            settings.extensions.retain(|item| item != &source);
            save_settings(agent_dir, &settings)?;
            Ok(format!("Removed {source}{}", scope(local)))
        }
        "update" => {
            let target = source.unwrap_or_else(|| "all".into());
            let catalogs = agent_dir.join("models");
            fs::create_dir_all(&catalogs).map_err(|err| err.to_string())?;
            for provider in pi_ai::builtin_provider_ids() {
                if let Some(json) = pi_ai::builtin_catalog_json(provider) {
                    fs::write(catalogs.join(format!("{provider}.json")), json)
                        .map_err(|err| err.to_string())?;
                }
            }
            Ok(format!(
                "Updated {target}: wrote {} catalogs to {}",
                pi_ai::builtin_provider_ids().len(),
                catalogs.display()
            ))
        }
        "list" => Ok(render_list(&settings)),
        "config" => Ok(render_config(&settings, local)),
        _ => Err(format!("Unknown command {command}")),
    }
}

fn scope(local: bool) -> &'static str {
    if local {
        " (local)"
    } else {
        ""
    }
}

fn render_list(settings: &Settings) -> String {
    if settings.extensions.is_empty() {
        "No extensions installed.".into()
    } else {
        settings.extensions.join("\n")
    }
}

fn render_config(settings: &Settings, local: bool) -> String {
    format!(
        "scope: {}\nextensions: {}\ntheme: {}\n",
        if local { "local" } else { "user" },
        settings.extensions.join(", "),
        settings.theme.clone().unwrap_or_else(|| "dark".into())
    )
}

pub fn ensure_agent_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| err.to_string())
}
