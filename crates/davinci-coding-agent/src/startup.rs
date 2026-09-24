//! Package and tmux startup warnings matching TS `interactive-mode.ts`.
//! Davinci releases independently, so upstream Pi release checks are omitted.

use std::process::Command;

use crate::args::APP_NAME;
use crate::settings::Settings;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupNotices {
    pub package_updates: Vec<String>,
    pub tmux_warning: Option<String>,
    pub models_json_error: Option<String>,
    pub migrated_auth_providers: Vec<String>,
}

pub fn collect_startup_notices(
    _current_version: &str,
    settings: &Settings,
    models_json_error: Option<String>,
    migrated_auth_providers: Vec<String>,
) -> StartupNotices {
    StartupNotices {
        package_updates: check_for_package_updates(settings),
        tmux_warning: check_tmux_keyboard_setup(),
        models_json_error,
        migrated_auth_providers,
    }
}

/// Match upstream interactive-mode.ts: optional update checks must not delay
/// the first prompt. Dropping the receiver never waits for network requests.
pub fn start_background_checks(
    current_version: &'static str,
    settings: Settings,
) -> std::sync::mpsc::Receiver<StartupNotices> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let notices = collect_startup_notices(current_version, &settings, None, Vec::new());
        let _ = sender.send(notices);
    });
    receiver
}

pub fn format_notices(notices: &StartupNotices) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    if !notices.package_updates.is_empty() {
        lines.push(("status".into(), "Package Updates Available".into()));
        lines.push((
            "muted".into(),
            format!("Package updates are available. Run {APP_NAME} update --extensions"),
        ));
        lines.push(("muted".into(), "Packages:".into()));
        for package in &notices.package_updates {
            lines.push(("muted".into(), format!("- {package}")));
        }
    }
    if let Some(warning) = &notices.tmux_warning {
        lines.push(("warning".into(), format!("Warning: {warning}")));
    }
    if !notices.migrated_auth_providers.is_empty() {
        lines.push((
            "warning".into(),
            format!(
                "Warning: Migrated credentials to auth.json: {}",
                notices.migrated_auth_providers.join(", ")
            ),
        ));
    }
    if let Some(error) = &notices.models_json_error {
        lines.push(("error".into(), format!("models.json error: {error}")));
    }
    lines
}

fn parse_package_updates_reply(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn check_for_package_updates(settings: &Settings) -> Vec<String> {
    if matches!(
        std::env::var("PI_OFFLINE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) {
        return Vec::new();
    }
    if let Ok(raw) = std::env::var("PI_PACKAGE_UPDATES_REPLY") {
        return parse_package_updates_reply(&raw);
    }
    let agent_dir = davinci_session::default_agent_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let trusted = crate::settings::is_trusted(settings, &cwd, None);
    crate::packages::check_for_available_updates(settings, &agent_dir, &cwd, trusted)
}

pub fn check_tmux_keyboard_setup() -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }
    let extended_keys = tmux_show("extended-keys")?;
    if extended_keys != "on" && extended_keys != "always" {
        return Some(
            "tmux extended-keys is off. Modified Enter keys may not work. Add `set -g extended-keys on` to ~/.tmux.conf and restart tmux."
                .into(),
        );
    }
    let format = tmux_show("extended-keys-format").unwrap_or_default();
    if format == "xterm" {
        return Some(
            "tmux extended-keys-format is xterm. Pi works best with csi-u. Add `set -g extended-keys-format csi-u` to ~/.tmux.conf and restart tmux."
                .into(),
        );
    }
    None
}

fn tmux_show(option: &str) -> Option<String> {
    if option == "extended-keys" {
        if let Ok(value) = std::env::var("PI_TMUX_EXTENDED_KEYS") {
            return Some(value);
        }
    }
    if option == "extended-keys-format" {
        if let Ok(value) = std::env::var("PI_TMUX_EXTENDED_KEYS_FORMAT") {
            return Some(value);
        }
    }
    if cfg!(test) {
        return None;
    }
    let output = Command::new("tmux")
        .args(["show", "-gv", option])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static UPDATE_CHECK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn package_and_tmux_fixtures() {
        // Parsing a fixture must not depend on (or disable) the offline guard.
        let updates = parse_package_updates_reply("todo, snake");
        assert!(parse_package_updates_reply(" , ").is_empty());
        assert_eq!(updates, vec!["todo".to_string(), "snake".to_string()]);

        std::env::set_var("TMUX", "1");
        std::env::set_var("PI_TMUX_EXTENDED_KEYS", "off");
        assert!(check_tmux_keyboard_setup()
            .unwrap()
            .contains("tmux extended-keys is off"));
        std::env::set_var("PI_TMUX_EXTENDED_KEYS", "on");
        std::env::set_var("PI_TMUX_EXTENDED_KEYS_FORMAT", "xterm");
        assert!(check_tmux_keyboard_setup()
            .unwrap()
            .contains("extended-keys-format is xterm"));
        std::env::remove_var("TMUX");
        std::env::remove_var("PI_TMUX_EXTENDED_KEYS");
        std::env::remove_var("PI_TMUX_EXTENDED_KEYS_FORMAT");
        let formatted = format_notices(&StartupNotices {
            package_updates: vec!["todo".into()],
            ..StartupNotices::default()
        });
        assert!(!formatted.iter().any(|(_, line)| line == "Update Available"));
        assert!(formatted
            .iter()
            .any(|(_, line)| line.contains("Package Updates Available")));
        assert!(formatted
            .iter()
            .any(|(_, line)| line.contains("davinci update --extensions")));
    }

    #[test]
    fn live_npm_view_fixture_detects_update() {
        let _env_lock = UPDATE_CHECK_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let npm = dir
            .path()
            .join("agent")
            .join("npm")
            .join("node_modules")
            .join("todo");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(
            npm.join("package.json"),
            r#"{"name":"todo","version":"1.0.0"}"#,
        )
        .unwrap();
        let old_npm_reply = std::env::var_os("PI_NPM_VIEW_REPLY");
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"2.0.0\"");

        // All roots and replies are fixture inputs; retain the caller's offline mode.
        let settings = Settings {
            packages: vec!["npm:todo".into()],
            ..Settings::default()
        };
        let updates = crate::packages::check_for_available_updates(
            &settings,
            &dir.path().join("agent"),
            dir.path(),
            true,
        );
        assert_eq!(updates, vec!["todo".to_string()]);
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"1.0.0\"");
        let none = crate::packages::check_for_available_updates(
            &settings,
            &dir.path().join("agent"),
            dir.path(),
            true,
        );
        assert!(none.is_empty());
        match old_npm_reply {
            Some(value) => std::env::set_var("PI_NPM_VIEW_REPLY", value),
            None => std::env::remove_var("PI_NPM_VIEW_REPLY"),
        }
    }

    #[test]
    fn untrusted_project_packages_are_not_checked_for_updates() {
        let _env_lock = UPDATE_CHECK_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let project_settings = dir.path().join(".pi");
        std::fs::create_dir_all(&project_settings).unwrap();
        std::fs::write(
            project_settings.join("settings.json"),
            r#"{"packages":["npm:evil"]}"#,
        )
        .unwrap();
        let installed = dir
            .path()
            .join(".pi")
            .join("npm")
            .join("node_modules")
            .join("evil");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(
            installed.join("package.json"),
            r#"{"name":"evil","version":"1.0.0"}"#,
        )
        .unwrap();
        let old = std::env::var_os("PI_NPM_VIEW_REPLY");
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"2.0.0\"");
        let settings = Settings::default();
        let agent = dir.path().join("agent");
        let untrusted =
            crate::packages::check_for_available_updates(&settings, &agent, dir.path(), false);
        let trusted =
            crate::packages::check_for_available_updates(&settings, &agent, dir.path(), true);
        match old {
            Some(value) => std::env::set_var("PI_NPM_VIEW_REPLY", value),
            None => std::env::remove_var("PI_NPM_VIEW_REPLY"),
        }
        assert!(untrusted.is_empty());
        assert_eq!(trusted, vec!["evil".to_string()]);
    }
}
