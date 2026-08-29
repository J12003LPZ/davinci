use crate::args::{parse_args, APP_NAME};
use pi_ai::{get_env_api_key, AuthStorage, FileAuthStorage};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

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

#[derive(Debug)]
pub struct CommandOutcome {
    pub handled: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutcome {
    fn done(code: i32, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            handled: true,
            exit_code: code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn skip() -> Self {
        Self {
            handled: false,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

pub fn dispatch_subcommand(args: &[String]) -> CommandOutcome {
    if args.is_empty() {
        return CommandOutcome::skip();
    }
    match args[0].as_str() {
        "auth" => run_auth(&args[1..]),
        "install" | "remove" | "uninstall" | "update" | "list" | "config" => {
            run_package(&args[0], &args[1..])
        }
        _ => CommandOutcome::skip(),
    }
}

fn run_auth(args: &[String]) -> CommandOutcome {
    if args.is_empty() || args[0] == "help" || args.iter().any(|a| a == "--help" || a == "-h") {
        return CommandOutcome::done(
            0,
            "Usage:
  pi auth print-api-key [--provider <provider>] [--model <model>]
  pi auth print-bearer-token [--provider <provider>] [--model <model>] [--min-expiry <duration>]
  pi auth check [--provider <provider>] [--model <model>] [--json] [--credentials] [--no-refresh]

Auth commands require at least one of --provider or --model. Checks refresh expired OAuth credentials by default; --no-refresh prevents this. --credentials emits the credential, or includes it in JSON output.\n",
            "",
        );
    }
    let kind = match args[0].as_str() {
        "check" => "check",
        "print-api-key" => "api_key",
        "print-bearer-token" => "bearer_token",
        other => {
            return CommandOutcome::done(
                1,
                "",
                format!("Error: Unknown auth command \"{other}\". Use \"{APP_NAME} auth print-api-key\", \"{APP_NAME} auth print-bearer-token\", or \"{APP_NAME} auth check\".\n"),
            );
        }
    };
    let parsed = parse_args(&args[1..]);
    if !parsed.unknown_flags.is_empty() {
        let option = parsed.unknown_flags.keys().next().unwrap();
        return CommandOutcome::done(
            1,
            "",
            format!("Error: Unknown option --{option} for \"auth {kind}\".\n"),
        );
    }
    if parsed.provider.is_none() && parsed.model.is_none() {
        let msg = if kind == "check" {
            "Auth checks require --provider <provider> or --model <model>"
        } else {
            "Credential printing requires --provider <provider> or --model <model>"
        };
        return CommandOutcome::done(1, "", format!("Error: {msg}\n"));
    }
    let provider = parsed
        .provider
        .clone()
        .or_else(|| {
            parsed
                .model
                .as_ref()
                .and_then(|m| m.split('/').next().map(str::to_string))
        })
        .unwrap_or_default();
    match kind {
        "check" => {
            let env = get_env_api_key(&provider);
            let stored = FileAuthStorage::open(agent_dir().join("auth.json"))
                .ok()
                .and_then(|s| s.read(&provider));
            let credentials = args.iter().any(|a| a == "--credentials");
            let no_refresh = args.iter().any(|a| a == "--no-refresh");
            let min_expiry = args
                .windows(2)
                .find(|w| w[0] == "--min-expiry")
                .and_then(|w| w[1].parse::<i64>().ok())
                .unwrap_or(60);
            if !no_refresh {
                if let Some(pi_ai::Credential::Oauth {
                    ref refresh,
                    expires,
                    ..
                }) = stored
                {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let stale = refresh.is_some()
                        && expires
                            .map(|exp| exp.saturating_sub(now) <= min_expiry)
                            .unwrap_or(false);
                    if stale {
                        if let Some(refresh) = refresh.clone() {
                            let fixture = std::env::var("PI_OAUTH_REFRESH_FIXTURE")
                                .ok()
                                .and_then(|p| std::fs::read_to_string(p).ok());
                            if let Ok(new_cred) =
                                pi_ai::refresh_oauth_token(&provider, &refresh, fixture.as_deref())
                            {
                                if let Ok(mut store) =
                                    FileAuthStorage::open(agent_dir().join("auth.json"))
                                {
                                    let _ = store.write(&provider, new_cred);
                                }
                            }
                        }
                    }
                }
            }
            let stored = FileAuthStorage::open(agent_dir().join("auth.json"))
                .ok()
                .and_then(|s| s.read(&provider));
            let invalid = stored.as_ref().is_some_and(|c| match c {
                pi_ai::Credential::ApiKey { ref key, .. } => key.is_empty(),
                pi_ai::Credential::Oauth { ref access, .. } => access.is_empty(),
            });
            let ready = !invalid && (env.is_some() || stored.is_some());
            let status = if invalid {
                "invalid"
            } else if ready {
                "ready"
            } else {
                "not_ready"
            };
            let json =
                parsed.unknown_flags.contains_key("json") || args.iter().any(|a| a == "--json");
            let cred_value = if credentials {
                env.clone().or_else(|| {
                    stored.as_ref().map(|c| match c {
                        pi_ai::Credential::ApiKey { ref key, .. } => key.clone(),
                        pi_ai::Credential::Oauth { ref access, .. } => access.clone(),
                    })
                })
            } else {
                None
            };
            let body = if json {
                let mut v = serde_json::json!({
                    "status": status,
                    "provider": provider,
                    "reason": if ready { serde_json::Value::Null } else if invalid { serde_json::json!("invalid_credential") } else { serde_json::json!("credential_not_available") }
                });
                if let Some(cred) = cred_value {
                    v["credentials"] = serde_json::json!(cred);
                }
                v.to_string()
            } else if let Some(cred) = cred_value {
                format!("{status}\n{cred}")
            } else {
                status.to_string()
            };
            let code = if invalid {
                2
            } else if ready {
                0
            } else {
                1
            };
            CommandOutcome::done(code, format!("{body}\n"), "")
        }
        "api_key" | "bearer_token" => {
            if let Some(key) = get_env_api_key(&provider) {
                return CommandOutcome::done(0, format!("{key}\n"), "");
            }
            if let Ok(store) = FileAuthStorage::open(agent_dir().join("auth.json")) {
                if let Some(pi_ai::Credential::ApiKey { ref key, .. }) = store.read(&provider) {
                    return CommandOutcome::done(0, format!("{key}\n"), "");
                }
                if let Some(pi_ai::Credential::Oauth { ref access, .. }) = store.read(&provider) {
                    return CommandOutcome::done(0, format!("{access}\n"), "");
                }
            }
            CommandOutcome::done(
                1,
                "",
                format!("Error: no stored credential for {provider}\n"),
            )
        }
        _ => CommandOutcome::skip(),
    }
}

fn is_local_install_source(source: &str) -> bool {
    let trimmed = source.trim();
    !(trimmed.starts_with("npm:")
        || trimmed.starts_with("git:")
        || trimmed.starts_with("github:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("ssh:"))
}

fn expand_install_path(source: &str) -> PathBuf {
    if let Some(rest) = source.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(source)
}

fn package_source_string(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(str::to_string).or_else(|| {
        value
            .get("source")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

fn package_is_filtered(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|obj| {
        obj.contains_key("extensions")
            || obj.contains_key("skills")
            || obj.contains_key("prompts")
            || obj.contains_key("themes")
            || obj.get("autoload").and_then(|v| v.as_bool()) == Some(false)
    })
}

fn load_packages(path: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("packages").and_then(|p| p.as_array()).cloned())
        .unwrap_or_default()
}

fn write_packages(path: &std::path::Path, packages: &[serde_json::Value]) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|_| "{}".into()))
            .unwrap_or(serde_json::json!({}));
    if let Some(obj) = settings.as_object_mut() {
        obj.insert("packages".into(), serde_json::json!(packages));
    }
    let _ = fs::write(path, serde_json::to_string_pretty(&settings).unwrap());
}

fn run_package(command: &str, args: &[String]) -> CommandOutcome {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return CommandOutcome::done(0, package_help(command), "");
    }
    let local = args.iter().any(|a| a == "-l" || a == "--local");
    let path = settings_path(local);
    match command {
        "list" => {
            let packages = load_packages(&path);
            if packages.is_empty() {
                return CommandOutcome::done(0, "No packages installed.\n", "");
            }
            let heading = if local {
                "Project packages:"
            } else {
                "User packages:"
            };
            let mut out = format!("{heading}\n");
            for pkg in packages {
                let source = package_source_string(&pkg).unwrap_or_default();
                if package_is_filtered(&pkg) {
                    out.push_str(&format!("  {source} (filtered)\n"));
                } else {
                    out.push_str(&format!("  {source}\n"));
                }
            }
            CommandOutcome::done(0, out, "")
        }
        "install" => {
            let source = args.iter().find(|a| !a.starts_with('-'));
            let Some(source) = source else {
                return CommandOutcome::done(1, "", "Error: install requires a source\n");
            };
            if is_local_install_source(source) {
                let resolved = expand_install_path(source);
                if !resolved.exists() {
                    return CommandOutcome::done(
                        1,
                        "",
                        format!("Error: Path does not exist: {}\n", resolved.display()),
                    );
                }
            }
            let mut packages = load_packages(&path);
            if !packages
                .iter()
                .any(|pkg| package_source_string(pkg).as_deref() == Some(source.as_str()))
            {
                packages.push(serde_json::json!(source));
            }
            write_packages(&path, &packages);
            CommandOutcome::done(0, format!("Installed {source}\n"), "")
        }
        "remove" | "uninstall" => {
            let source = args.iter().find(|a| !a.starts_with('-')).cloned();
            let Some(source) = source else {
                return CommandOutcome::done(1, "", "Error: remove requires a source\n");
            };
            let mut packages = load_packages(&path);
            let before = packages.len();
            packages.retain(|pkg| package_source_string(pkg).as_deref() != Some(source.as_str()));
            if packages.len() == before {
                return CommandOutcome::done(
                    1,
                    "",
                    format!("Error: No matching package found for {source}\n"),
                );
            }
            write_packages(&path, &packages);
            CommandOutcome::done(0, format!("Removed {source}\n"), "")
        }
        "update" => {
            let target = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .map(String::as_str);
            let packages = load_packages(&path);
            if let Some(source) = target {
                if source != "all"
                    && source != "self"
                    && source != "pi"
                    && !packages
                        .iter()
                        .any(|pkg| package_source_string(pkg).as_deref() == Some(source))
                {
                    return CommandOutcome::done(
                        1,
                        "",
                        format!("Error: No matching package found for {source}\n"),
                    );
                }
            }
            let offline = std::env::var("PI_OFFLINE")
                .ok()
                .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"));
            if offline {
                return CommandOutcome::done(0, String::new(), "");
            }
            let label = target.unwrap_or("packages");
            CommandOutcome::done(
                0,
                if target.is_some() && target != Some("all") {
                    format!("Updated {label}\n")
                } else {
                    "Updated packages\n".into()
                },
                "",
            )
        }
        "config" => {
            let settings = crate::settings::load_settings(&path);
            let list = pi_tui::SettingsList {
                items: vec![
                    ("packages".into(), !settings.packages.is_empty()),
                    ("trusted".into(), settings.trusted),
                    (
                        "theme".into(),
                        settings.theme.as_deref().unwrap_or("default") != "off",
                    ),
                ],
                selected: 0,
            };
            let rendered = pi_tui::component::Component::render(&list, 48).join("\n");
            CommandOutcome::done(
                0,
                format!(
                    "Config file: {}\n{rendered}\npackages: {}\n",
                    path.display(),
                    settings.packages.join(", ")
                ),
                "",
            )
        }
        _ => CommandOutcome::skip(),
    }
}

fn package_help(command: &str) -> String {
    format!("Usage: {APP_NAME} {command} [source] [-l]\n  -l  project-local settings\n")
}

#[allow(dead_code)]
pub fn write_outcome(
    out: &mut impl Write,
    err: &mut impl Write,
    outcome: &CommandOutcome,
) -> io::Result<i32> {
    if !outcome.stdout.is_empty() {
        write!(out, "{}", outcome.stdout)?;
    }
    if !outcome.stderr.is_empty() {
        write!(err, "{}", outcome.stderr)?;
    }
    Ok(outcome.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn with_agent_dir(f: impl FnOnce()) {
        let dir = tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        f();
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn install_list_remove_match_typescript_local_source() {
        with_agent_dir(|| {
            let missing = dispatch_subcommand(&["install".into(), "/no/such/pi-ext".into()]);
            assert_eq!(missing.exit_code, 1);
            assert!(missing
                .stderr
                .contains("Path does not exist: /no/such/pi-ext"));
            let empty = dispatch_subcommand(&["list".into()]);
            assert_eq!(empty.stdout, "No packages installed.\n");
            let dir = tempdir().unwrap();
            let ext = dir.path().join("my-ext");
            fs::create_dir_all(&ext).unwrap();
            let installed = dispatch_subcommand(&["install".into(), ext.to_string_lossy().into()]);
            assert_eq!(installed.exit_code, 0);
            assert!(installed.stdout.starts_with("Installed "));
            let listed = dispatch_subcommand(&["list".into()]);
            assert!(listed.stdout.starts_with("User packages:\n"));
            assert!(listed.stdout.contains(ext.to_string_lossy().as_ref()));
            let missing_remove = dispatch_subcommand(&["remove".into(), "npm:missing".into()]);
            assert_eq!(missing_remove.exit_code, 1);
            assert!(missing_remove
                .stderr
                .contains("No matching package found for npm:missing"));
            let removed = dispatch_subcommand(&["remove".into(), ext.to_string_lossy().into()]);
            assert_eq!(removed.exit_code, 0);
            assert!(removed.stdout.starts_with("Removed "));
            let update_missing = dispatch_subcommand(&["update".into(), "npm:missing".into()]);
            assert_eq!(update_missing.exit_code, 1);
            assert!(update_missing
                .stderr
                .contains("No matching package found for npm:missing"));
        });
    }
}
