use crate::args::{parse_args, APP_NAME, VERSION};
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
            let ready = env.is_some() || stored.is_some();
            let status = if ready { "ready" } else { "not_ready" };
            let json =
                parsed.unknown_flags.contains_key("json") || args.iter().any(|a| a == "--json");
            let body = if json {
                serde_json::json!({
                    "status": status,
                    "provider": provider,
                    "reason": if ready { serde_json::Value::Null } else { serde_json::json!("credential_not_available") }
                })
                .to_string()
            } else {
                status.to_string()
            };
            CommandOutcome::done(if ready { 0 } else { 1 }, format!("{body}\n"), "")
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

fn run_package(command: &str, args: &[String]) -> CommandOutcome {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return CommandOutcome::done(0, package_help(command), "");
    }
    let local = args.iter().any(|a| a == "-l" || a == "--local");
    let path = settings_path(local);
    match command {
        "list" => {
            let body = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
            CommandOutcome::done(0, format!("{body}\n"), "")
        }
        "install" => {
            let source = args.iter().find(|a| !a.starts_with('-'));
            let Some(source) = source else {
                return CommandOutcome::done(1, "", "Error: install requires a source\n");
            };
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut settings: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|_| "{}".into()))
                    .unwrap_or(serde_json::json!({}));
            let packages = settings
                .as_object_mut()
                .unwrap()
                .entry("packages")
                .or_insert(serde_json::json!([]));
            if let Some(arr) = packages.as_array_mut() {
                arr.push(serde_json::json!(source));
            }
            let _ = fs::write(&path, serde_json::to_string_pretty(&settings).unwrap());
            CommandOutcome::done(0, format!("Installed {source}\n"), "")
        }
        "remove" | "uninstall" => {
            let source = args.iter().find(|a| !a.starts_with('-')).cloned();
            let Some(source) = source else {
                return CommandOutcome::done(1, "", "Error: remove requires a source\n");
            };
            if path.exists() {
                if let Ok(mut settings) =
                    serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap())
                {
                    if let Some(arr) = settings.get_mut("packages").and_then(|v| v.as_array_mut()) {
                        arr.retain(|v| v.as_str() != Some(source.as_str()));
                    }
                    let _ = fs::write(&path, serde_json::to_string_pretty(&settings).unwrap());
                }
            }
            CommandOutcome::done(0, format!("Removed {source}\n"), "")
        }
        "update" => {
            let target = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .map(String::as_str)
                .unwrap_or("all");
            CommandOutcome::done(
                0,
                format!("Updated {target} (Rust {APP_NAME} {VERSION}; catalogs bundled)\n"),
                "",
            )
        }
        "config" => CommandOutcome::done(0, format!("Config file: {}\n", path.display()), ""),
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
