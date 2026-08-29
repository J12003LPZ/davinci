use crate::args::{parse_args, APP_NAME, VERSION};
use crate::config_selector::{render_config, run_interactive_config, WriteScope};
use crate::package_manager::{
    format_package_list, install_and_persist, list_configured_packages, project_is_trusted,
    remove_and_persist, update_configured,
};
use pi_ai::{get_env_api_key, AuthStorage, FileAuthStorage};
use std::io::{self, IsTerminal, Write};

pub use crate::settings::{agent_dir, settings_path};

pub const PACKAGE_NAME: &str = "@earendil-works/pi-coding-agent";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateTarget {
    All,
    SelfOnly,
    Extensions,
    Models,
}

fn package_usage(command: &str) -> String {
    match command {
        "install" => format!("{APP_NAME} install <source> [-l] [--approve|--no-approve]"),
        "remove" | "uninstall" => {
            format!("{APP_NAME} remove <source> [-l] [--approve|--no-approve]")
        }
        "update" => format!(
            "{APP_NAME} update [source|self|pi] [--self|--extensions|--models|--all] [--extension <source>] [--approve|--no-approve] [--force]"
        ),
        "list" => format!("{APP_NAME} list [--approve|--no-approve]"),
        "config" => format!("{APP_NAME} config [-l] [--approve|--no-approve]"),
        other => format!("{APP_NAME} {other}"),
    }
}

fn package_help(command: &str) -> String {
    match command {
        "install" => format!(
            "Usage:
  {}

Install a package and add it to settings.

Options:
  -l, --local       Install project-locally (.pi/settings.json)
  -a, --approve     Trust project-local files for this command
  -na, --no-approve Ignore project-local files for this command

Examples:
  {APP_NAME} install npm:@foo/bar
  {APP_NAME} install git:github.com/user/repo
  {APP_NAME} install git:git@github.com:user/repo
  {APP_NAME} install https://github.com/user/repo
  {APP_NAME} install ssh://git@github.com/user/repo
  {APP_NAME} install ./local/path
",
            package_usage("install")
        ),
        "remove" | "uninstall" => format!(
            "Usage:
  {}

Remove a package and its source from settings.
Alias: {APP_NAME} uninstall <source> [-l]

Options:
  -l, --local       Remove from project settings (.pi/settings.json)
  -a, --approve     Trust project-local files for this command
  -na, --no-approve Ignore project-local files for this command
",
            package_usage("remove")
        ),
        "update" => format!(
            "Usage:
  {}

Update pi, installed packages, or model catalogs.

Options:
  --self                  Update pi only (default when no target is given)
  --extensions            Update installed packages only
  --models                Refresh model catalogs only
  --all                   Update pi and installed packages
  --extension <source>    Update one package only
  --force                 Reinstall pi even if the current version is latest
",
            package_usage("update")
        ),
        "list" => format!(
            "Usage:
  {}

List installed packages from user and project settings.
",
            package_usage("list")
        ),
        "config" => format!(
            "Usage:
  {}

Open the resource configuration TUI to enable or disable package resources.
Without -l, starts in global settings (~/.pi/agent/settings.json).
Press Tab in the TUI to switch between global and project-local modes.

Options:
  -l, --local       Edit project overrides (.pi/settings.json)
  -a, --approve     Trust project-local files for this command with -l
  -na, --no-approve Ignore project-local files for this command with -l
",
            package_usage("config")
        ),
        other => format!("Usage: {}\n", package_usage(other)),
    }
}

fn detect_install_method() -> &'static str {
    if let Ok(method) = std::env::var("PI_INSTALL_METHOD") {
        return match method.as_str() {
            "npm" => "npm",
            "pnpm" => "pnpm",
            "yarn" => "yarn",
            "bun" => "bun",
            "bun-binary" => "bun-binary",
            _ => "unknown",
        };
    }
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase())
        .unwrap_or_default();
    if exe.contains("/pnpm/") || exe.contains("/.pnpm/") {
        "pnpm"
    } else if exe.contains("/yarn/") || exe.contains("/.yarn/") {
        "yarn"
    } else if exe.contains("/install/global/node_modules/") {
        "bun"
    } else if exe.contains("/npm/") || exe.contains("/node_modules/") {
        "npm"
    } else {
        "unknown"
    }
}

fn self_update_unavailable_instruction() -> String {
    let method = detect_install_method();
    if method == "bun-binary" {
        return "Download from: https://github.com/earendil-works/pi-mono/releases/latest".into();
    }
    if method == "unknown" {
        return format!(
            "Update {PACKAGE_NAME} using the package manager, wrapper, or source checkout that provides this installation."
        );
    }
    format!(
        "This installation is not managed by a global {method} install. Update it with the package manager, wrapper, or source checkout that provides it."
    )
}

fn run_self_update(force: bool) -> CommandOutcome {
    let _ = force;
    if let Ok(fixture) = std::env::var("PI_SELF_UPDATE_FIXTURE") {
        if let Ok(raw) = std::fs::read_to_string(&fixture) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(version) = value.get("version").and_then(|v| v.as_str()) {
                    return CommandOutcome::done(
                        0,
                        format!("Updated {APP_NAME} from {VERSION} to {version}\n"),
                        "",
                    );
                }
            }
        }
    }
    let mut stderr = format!("error: {APP_NAME} cannot self-update this installation.\n");
    stderr.push_str(&self_update_unavailable_instruction());
    stderr.push('\n');
    if let Ok(exe) = std::env::current_exe() {
        stderr.push('\n');
        stderr.push_str(&format!(
            "Location of {APP_NAME} executable: {}\n",
            exe.display()
        ));
    }
    CommandOutcome::done(1, "", stderr)
}

fn refresh_model_catalogs() -> Result<(), String> {
    if crate::package_manager::network_disabled() {
        if let Ok(fixture) = std::env::var("PI_MODELS_REFRESH_FIXTURE") {
            let dest = agent_dir().join("models.json");
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(fixture, dest).map_err(|e| e.to_string())?;
            return Ok(());
        }
        return Err("Model catalog refresh timed out.".into());
    }
    Ok(())
}

fn parse_approve(args: &[String]) -> Result<Option<bool>, String> {
    let mut approve = None;
    for arg in args {
        if arg == "-a" || arg == "--approve" {
            approve = Some(true);
        } else if arg == "-na" || arg == "--no-approve" {
            approve = Some(false);
        }
    }
    Ok(approve)
}

fn run_package(command: &str, args: &[String]) -> CommandOutcome {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return CommandOutcome::done(0, package_help(command), "");
    }

    if command == "config" {
        return run_config(args);
    }

    let mut local = false;
    let mut force = false;
    let mut source: Option<String> = None;
    let mut self_flag = false;
    let mut extensions_flag = false;
    let mut models_flag = false;
    let mut all_flag = false;
    let mut extension_flag_source: Option<String> = None;
    let mut invalid_option = None;
    let mut invalid_argument = None;
    let mut missing_option_value = None;
    let mut conflicting = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-l" || arg == "--local" {
            if command == "install" || command == "remove" || command == "uninstall" {
                local = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--self" {
            if command == "update" {
                self_flag = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--extensions" {
            if command == "update" {
                extensions_flag = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--models" {
            if command == "update" {
                models_flag = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--all" {
            if command == "update" {
                all_flag = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--force" {
            if command == "update" {
                force = true;
            } else {
                invalid_option = invalid_option.or(Some(arg.clone()));
            }
        } else if arg == "--extension" {
            if command != "update" {
                invalid_option = invalid_option.or(Some(arg.clone()));
            } else {
                let value = args.get(i + 1);
                if value.is_none() || value.is_some_and(|v| v.starts_with('-')) {
                    missing_option_value = missing_option_value.or(Some(arg.clone()));
                } else if extension_flag_source.is_some() {
                    conflicting =
                        conflicting.or(Some("--extension can only be provided once".into()));
                    i += 1;
                } else {
                    extension_flag_source = value.cloned();
                    i += 1;
                }
            }
        } else if arg == "-a" || arg == "--approve" || arg == "-na" || arg == "--no-approve" {
        } else if arg.starts_with('-') {
            invalid_option = invalid_option.or(Some(arg.clone()));
        } else if source.is_none() {
            source = Some(arg.clone());
        } else {
            invalid_argument = invalid_argument.or(Some(arg.clone()));
        }
        i += 1;
    }

    if let Some(option) = invalid_option {
        return CommandOutcome::done(
            1,
            "",
            format!(
                "Unknown option {option} for \"{command}\".\nUse \"{APP_NAME} --help\" or \"{}\".\n",
                package_usage(command)
            ),
        );
    }
    if let Some(option) = missing_option_value {
        return CommandOutcome::done(
            1,
            "",
            format!(
                "Missing value for {option}.\nUsage: {}\n",
                package_usage(command)
            ),
        );
    }
    if let Some(arg) = invalid_argument {
        return CommandOutcome::done(
            1,
            "",
            format!(
                "Unexpected argument {arg}.\nUsage: {}\n",
                package_usage(command)
            ),
        );
    }

    let approve = parse_approve(args).unwrap_or(None);
    let trusted = project_is_trusted(approve);
    if local && !trusted && (command == "install" || command == "remove" || command == "uninstall")
    {
        return CommandOutcome::done(
            1,
            "",
            "Project is not trusted. Use --approve to modify local package config.\n",
        );
    }

    match command {
        "list" => CommandOutcome::done(0, format_package_list(&list_configured_packages()), ""),
        "install" => {
            let Some(source) = source else {
                return CommandOutcome::done(
                    1,
                    "",
                    format!(
                        "Missing install source.\nUsage: {}\n",
                        package_usage("install")
                    ),
                );
            };
            match install_and_persist(&source, local) {
                Ok(_) => CommandOutcome::done(0, format!("Installed {source}\n"), ""),
                Err(err) => CommandOutcome::done(1, "", format!("Error: {err}\n")),
            }
        }
        "remove" | "uninstall" => {
            let Some(source) = source else {
                return CommandOutcome::done(
                    1,
                    "",
                    format!(
                        "Missing remove source.\nUsage: {}\n",
                        package_usage("remove")
                    ),
                );
            };
            match remove_and_persist(&source, local) {
                Ok(true) => CommandOutcome::done(0, format!("Removed {source}\n"), ""),
                Ok(false) => {
                    CommandOutcome::done(1, "", format!("No matching package found for {source}\n"))
                }
                Err(err) => CommandOutcome::done(1, "", format!("Error: {err}\n")),
            }
        }
        "update" => run_update(
            source,
            self_flag,
            extensions_flag,
            models_flag,
            all_flag,
            extension_flag_source,
            force,
            conflicting,
        ),
        _ => CommandOutcome::skip(),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_update(
    source: Option<String>,
    self_flag: bool,
    extensions_flag: bool,
    models_flag: bool,
    all_flag: bool,
    extension_flag_source: Option<String>,
    force: bool,
    mut conflicting: Option<String>,
) -> CommandOutcome {
    if all_flag && (self_flag || extensions_flag || models_flag || extension_flag_source.is_some())
    {
        conflicting = conflicting.or(Some(
            "--all cannot be combined with --self, --extensions, --models, or --extension".into(),
        ));
    }
    if all_flag && source.is_some() {
        conflicting = conflicting.or(Some(
            "--all cannot be combined with a positional source".into(),
        ));
    }
    if models_flag && (self_flag || extensions_flag || all_flag || extension_flag_source.is_some())
    {
        conflicting = conflicting.or(Some(
            "--models cannot be combined with --self, --extensions, --all, or --extension".into(),
        ));
    }
    if models_flag && source.is_some() {
        conflicting = conflicting.or(Some(
            "--models cannot be combined with a positional source".into(),
        ));
    }
    if extension_flag_source.is_some() && (self_flag || extensions_flag || all_flag) {
        conflicting = conflicting.or(Some(
            "--extension cannot be combined with --self, --extensions, or --all".into(),
        ));
    }
    if extension_flag_source.is_some() && source.is_some() {
        conflicting = conflicting.or(Some(
            "--extension cannot be combined with a positional source".into(),
        ));
    }
    if source.is_some()
        && !matches!(source.as_deref(), Some("self") | Some("pi"))
        && (extensions_flag || self_flag || all_flag)
    {
        conflicting = conflicting.or(Some(
            "positional update targets cannot be combined with --self, --extensions, or --all"
                .into(),
        ));
    }
    if let Some(message) = conflicting {
        return CommandOutcome::done(
            1,
            "",
            format!("{message}\nUsage: {}\n", package_usage("update")),
        );
    }

    let (target, skipped_note) = if models_flag {
        (UpdateTarget::Models, false)
    } else if let Some(ext) = extension_flag_source.clone() {
        return update_extensions(Some(ext));
    } else if let Some(source) = source.clone() {
        if source == "self" || source == "pi" {
            if extensions_flag {
                (UpdateTarget::All, false)
            } else {
                (UpdateTarget::SelfOnly, false)
            }
        } else {
            return update_extensions(Some(source));
        }
    } else if all_flag || (self_flag && extensions_flag) {
        (UpdateTarget::All, false)
    } else if self_flag {
        (UpdateTarget::SelfOnly, false)
    } else if extensions_flag {
        (UpdateTarget::Extensions, false)
    } else {
        (UpdateTarget::SelfOnly, true)
    };

    let mut stdout = String::new();
    if skipped_note {
        stdout.push_str(&format!(
            "Extensions are skipped. Run {APP_NAME} update --extensions to update extensions.\n"
        ));
    }
    match target {
        UpdateTarget::Models => match refresh_model_catalogs() {
            Ok(()) => CommandOutcome::done(0, "Model catalogs refreshed\n", ""),
            Err(err) => CommandOutcome::done(1, "", format!("Error: {err}\n")),
        },
        UpdateTarget::Extensions => {
            let mut out = stdout;
            let result = update_extensions(None);
            out.push_str(&result.stdout);
            CommandOutcome::done(result.exit_code, out, result.stderr)
        }
        UpdateTarget::SelfOnly => {
            let result = run_self_update(force);
            CommandOutcome::done(result.exit_code, stdout + &result.stdout, result.stderr)
        }
        UpdateTarget::All => {
            let ext = update_extensions(None);
            if ext.exit_code != 0 {
                return CommandOutcome::done(ext.exit_code, stdout + &ext.stdout, ext.stderr);
            }
            stdout.push_str(&ext.stdout);
            let self_update = run_self_update(force);
            CommandOutcome::done(
                self_update.exit_code,
                stdout + &self_update.stdout,
                self_update.stderr,
            )
        }
    }
}

fn update_extensions(source: Option<String>) -> CommandOutcome {
    match update_configured(source.as_deref()) {
        Ok(_) => {
            if let Some(source) = source {
                CommandOutcome::done(0, format!("Updated {source}\n"), "")
            } else {
                CommandOutcome::done(0, "Updated packages\n", "")
            }
        }
        Err(err) => CommandOutcome::done(1, "", format!("Error: {err}\n")),
    }
}

fn run_config(args: &[String]) -> CommandOutcome {
    let mut local = false;
    let mut approve = None;
    for arg in args {
        if arg == "-l" || arg == "--local" {
            local = true;
        } else if arg == "-a" || arg == "--approve" {
            approve = Some(true);
        } else if arg == "-na" || arg == "--no-approve" {
            approve = Some(false);
        } else if arg.starts_with('-') {
            return CommandOutcome::done(
                1,
                "",
                format!(
                    "Unknown option {arg} for \"config\".\nUse \"{APP_NAME} --help\" or \"{}\".\n",
                    package_usage("config")
                ),
            );
        } else {
            return CommandOutcome::done(
                1,
                "",
                format!(
                    "Unexpected argument {arg}.\nUsage: {}\n",
                    package_usage("config")
                ),
            );
        }
    }
    let trusted = project_is_trusted(approve);
    if local && !trusted {
        return CommandOutcome::done(
            1,
            "",
            "Project is not trusted. Use --approve to modify local resource config.\n",
        );
    }
    let scope = if local {
        WriteScope::Project
    } else {
        WriteScope::Global
    };
    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let rendered = if stdin_tty && stdout_tty {
        run_interactive_config(scope, trusted)
    } else {
        render_config(scope, trusted)
    };
    CommandOutcome::done(0, rendered, "")
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
        let _lock = crate::settings::test_env_lock();
        let dir = tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        let previous_cwd = std::env::current_dir().ok();
        let previous_net = std::env::var("PI_DISABLE_NETWORK").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        std::env::set_var("PI_DISABLE_NETWORK", "1");
        let _ = std::env::set_current_dir(dir.path());
        f();
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        match previous_net {
            Some(value) => std::env::set_var("PI_DISABLE_NETWORK", value),
            None => std::env::remove_var("PI_DISABLE_NETWORK"),
        }
        if let Some(cwd) = previous_cwd {
            let _ = std::env::set_current_dir(cwd);
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
            std::fs::create_dir_all(&ext).unwrap();
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

    #[test]
    fn fixture_npm_install_lists_installed_path_and_config_resources() {
        with_agent_dir(|| {
            let dir = tempdir().unwrap();
            let fixture = dir.path().join("fixture").join("npm").join("pi-cli");
            std::fs::create_dir_all(fixture.join("extensions")).unwrap();
            std::fs::write(
                fixture.join("extensions").join("index.ts"),
                "export default function () {}",
            )
            .unwrap();
            let previous_fixture = std::env::var("PI_PACKAGE_FIXTURE").ok();
            std::env::set_var("PI_PACKAGE_FIXTURE", dir.path().join("fixture"));
            std::env::set_var("PI_DISABLE_NETWORK", "1");
            let installed = dispatch_subcommand(&["install".into(), "npm:pi-cli".into()]);
            assert_eq!(installed.exit_code, 0, "{}", installed.stderr);
            let listed = dispatch_subcommand(&["list".into()]);
            assert!(listed.stdout.contains("npm:pi-cli"));
            assert!(listed.stdout.contains("npm/node_modules/pi-cli"));
            let config = dispatch_subcommand(&["config".into()]);
            assert_eq!(config.exit_code, 0);
            assert!(config.stdout.contains("Global Resources"));
            assert!(config.stdout.contains("npm:pi-cli (user)"));
            let update = dispatch_subcommand(&["update".into()]);
            assert!(update.stdout.contains("Extensions are skipped"));
            assert!(update
                .stderr
                .contains("cannot self-update this installation"));
            let unknown = dispatch_subcommand(&["list".into(), "-l".into()]);
            assert_eq!(unknown.exit_code, 1);
            assert!(unknown.stderr.contains("Unknown option -l for \"list\""));
            match previous_fixture {
                Some(v) => std::env::set_var("PI_PACKAGE_FIXTURE", v),
                None => std::env::remove_var("PI_PACKAGE_FIXTURE"),
            }
        });
    }
}
