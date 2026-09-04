//! Startup version / package / tmux warnings matching TS `interactive-mode.ts`.

use std::process::Command;
use std::time::Duration;

use crate::args::APP_NAME;
use crate::settings::Settings;

const LATEST_VERSION_URL: &str = "https://pi.dev/api/latest-version";
const CHANGELOG_URL: &str = "https://pi.dev/changelog";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestPiRelease {
    pub version: String,
    pub package_name: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupNotices {
    pub version: Option<LatestPiRelease>,
    pub package_updates: Vec<String>,
    pub tmux_warning: Option<String>,
    pub models_json_error: Option<String>,
    pub migrated_auth_providers: Vec<String>,
}

pub fn collect_startup_notices(
    current_version: &str,
    settings: &Settings,
    models_json_error: Option<String>,
    migrated_auth_providers: Vec<String>,
) -> StartupNotices {
    StartupNotices {
        version: check_for_new_pi_version(current_version),
        package_updates: check_for_package_updates(settings),
        tmux_warning: check_tmux_keyboard_setup(),
        models_json_error,
        migrated_auth_providers,
    }
}

pub fn format_notices(notices: &StartupNotices) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    if let Some(release) = &notices.version {
        lines.push(("status".into(), "Update Available".into()));
        lines.push((
            "muted".into(),
            format!(
                "New version {} is available. Run {} update",
                release.version, APP_NAME
            ),
        ));
        if let Some(note) = &release.note {
            if !note.trim().is_empty() {
                lines.push(("muted".into(), note.trim().to_string()));
            }
        }
        lines.push(("muted".into(), format!("Changelog: {CHANGELOG_URL}")));
    }
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

pub fn compare_package_versions(left: &str, right: &str) -> Option<i32> {
    let left = parse_semver(left.trim())?;
    let right = parse_semver(right.trim())?;
    Some(left.cmp(&right) as i32)
}

pub fn is_newer_package_version(candidate: &str, current: &str) -> bool {
    match compare_package_versions(candidate, current) {
        Some(cmp) => cmp > 0,
        None => candidate.trim() != current.trim(),
    }
}

pub fn check_for_new_pi_version(current_version: &str) -> Option<LatestPiRelease> {
    if std::env::var("PI_SKIP_VERSION_CHECK").is_ok() || std::env::var("PI_OFFLINE").is_ok() {
        return None;
    }
    let latest = get_latest_pi_release(current_version)?;
    if is_newer_package_version(&latest.version, current_version) {
        Some(latest)
    } else {
        None
    }
}

pub fn get_latest_pi_release(current_version: &str) -> Option<LatestPiRelease> {
    if std::env::var("PI_OFFLINE").is_ok() {
        return None;
    }
    let body = if let Ok(path) = std::env::var("PI_LATEST_VERSION_REPLY") {
        std::fs::read_to_string(path).ok()?
    } else if cfg!(test) {
        return None;
    } else {
        let agent = format!("{APP_NAME}/{current_version}");
        ureq::get(LATEST_VERSION_URL)
            .set("User-Agent", &agent)
            .set("accept", "application/json")
            .timeout(Duration::from_millis(10_000))
            .call()
            .ok()?
            .into_string()
            .ok()?
    };
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    let version = value.get("version")?.as_str()?.trim();
    if version.is_empty() {
        return None;
    }
    Some(LatestPiRelease {
        version: version.to_string(),
        package_name: value
            .get("packageName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        note: value
            .get("note")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

pub fn check_for_package_updates(settings: &Settings) -> Vec<String> {
    if matches!(
        std::env::var("PI_OFFLINE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) {
        return Vec::new();
    }
    if let Ok(raw) = std::env::var("PI_PACKAGE_UPDATES_REPLY") {
        return raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect();
    }
    let agent_dir = davinci_session::default_agent_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    crate::packages::check_for_available_updates(settings, &agent_dir, &cwd)
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

fn parse_semver(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_and_falls_back_to_string() {
        assert_eq!(compare_package_versions("0.84.5", "0.84.4"), Some(1));
        assert!(is_newer_package_version("0.85.0", "0.84.4"));
        assert!(is_newer_package_version("not-semver", "0.84.4"));
        assert!(!is_newer_package_version("0.84.4", "0.84.4"));
    }

    #[test]
    fn version_check_uses_fixture_and_skip_flags() {
        std::env::set_var("PI_SKIP_VERSION_CHECK", "1");
        assert!(check_for_new_pi_version("0.84.4").is_none());
        std::env::remove_var("PI_SKIP_VERSION_CHECK");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latest.json");
        std::fs::write(&path, r#"{"version":"0.99.0","note":"ship it"}"#).unwrap();
        std::env::set_var("PI_LATEST_VERSION_REPLY", path.to_string_lossy().as_ref());
        let release = check_for_new_pi_version("0.84.4").unwrap();
        assert_eq!(release.version, "0.99.0");
        assert_eq!(release.note.as_deref(), Some("ship it"));
        std::env::remove_var("PI_LATEST_VERSION_REPLY");
    }

    #[test]
    fn package_and_tmux_fixtures() {
        std::env::set_var("PI_PACKAGE_UPDATES_REPLY", "todo, snake");
        let updates = check_for_package_updates(&Settings::default());
        assert_eq!(updates, vec!["todo".to_string(), "snake".to_string()]);
        std::env::remove_var("PI_PACKAGE_UPDATES_REPLY");
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
            version: Some(LatestPiRelease {
                version: "1.0.0".into(),
                package_name: None,
                note: None,
            }),
            package_updates: vec!["todo".into()],
            ..StartupNotices::default()
        });
        assert!(formatted.iter().any(|(_, line)| line == "Update Available"));
        assert!(formatted
            .iter()
            .any(|(_, line)| line.contains("Package Updates Available")));
        assert!(formatted
            .iter()
            .any(|(_, line)| line.contains("pi update --extensions")));
    }

    #[test]
    fn live_npm_view_fixture_detects_update() {
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
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"2.0.0\"");
        std::env::remove_var("PI_PACKAGE_UPDATES_REPLY");
        std::env::remove_var("PI_OFFLINE");
        let settings = Settings {
            packages: vec!["npm:todo".into()],
            ..Settings::default()
        };
        let updates = crate::packages::check_for_available_updates(
            &settings,
            &dir.path().join("agent"),
            dir.path(),
        );
        assert_eq!(updates, vec!["todo".to_string()]);
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"1.0.0\"");
        let none = crate::packages::check_for_available_updates(
            &settings,
            &dir.path().join("agent"),
            dir.path(),
        );
        assert!(none.is_empty());
        std::env::remove_var("PI_NPM_VIEW_REPLY");
        std::env::remove_var("PI_CODING_AGENT_DIR");
    }
}
