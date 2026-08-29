use crate::args::{APP_NAME, VERSION};
use crate::package_manager::network_disabled;
use crate::settings::{canonicalize_path, cwd_relative_path};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const LATEST_VERSION_URL: &str = "https://pi.dev/api/latest-version";
const DEFAULT_INSTALLER_API_BASE: &str = "https://pi.dev/api/installer/releases";
const MANAGED_INSTALL_MARKER: &str = "managed-install.json";
const MANAGED_LOCK_STALE: Duration = Duration::from_millis(10_000);

#[derive(Debug, Clone)]
struct LatestPiRelease {
    version: String,
    package_name: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SelfUpdatePlan {
    pub version: String,
    pub should_run: bool,
    pub note: Option<String>,
}

fn package_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_PACKAGE_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn active_managed_install_root() -> Result<Option<PathBuf>, String> {
    let configured = match std::env::var("PI_MANAGED_INSTALL_ROOT") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let managed_root = canonicalize_path(Path::new(configured.trim()));
    let releases_dir = canonicalize_path(&managed_root.join("releases"));
    if cwd_relative_path(&canonicalize_path(&package_dir()), &releases_dir).is_none() {
        return Ok(None);
    }
    let marker_path = managed_root.join(MANAGED_INSTALL_MARKER);
    let marker = fs::read_to_string(&marker_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let valid = marker.as_ref().is_some_and(|value| {
        value.get("kind").and_then(|v| v.as_str()) == Some("pi-managed-install")
            && value.get("schemaVersion").and_then(|v| v.as_u64()) == Some(1)
            && value.get("layout").and_then(|v| v.as_str()) == Some("releases-v1")
    });
    if !valid {
        return Err(format!(
            "Managed install marker is missing or invalid: {}",
            marker_path.display()
        ));
    }
    Ok(Some(managed_root))
}

fn is_valid_managed_version(version: &str) -> bool {
    let bytes = version.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut parts = version.splitn(2, ['-', '+']);
    let core = parts.next().unwrap_or_default();
    let mut nums = core.split('.');
    nums.next()
        .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
        && nums
            .next()
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
        && nums
            .next()
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
        && nums.next().is_none()
}

fn parse_semver(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().split(['-', '+']).next().unwrap_or_default();
    let mut parts = core.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

fn is_newer_package_version(candidate: &str, current: &str) -> bool {
    match (parse_semver(candidate), parse_semver(current)) {
        (Some(left), Some(right)) => left > right,
        _ => candidate.trim() != current.trim(),
    }
}

fn fixture_release() -> Option<LatestPiRelease> {
    let path = std::env::var("PI_SELF_UPDATE_FIXTURE")
        .ok()
        .or_else(|| std::env::var("PI_LATEST_VERSION_FIXTURE").ok())?;
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let version = value.get("version")?.as_str()?.trim();
    if version.is_empty() {
        return None;
    }
    Some(LatestPiRelease {
        version: version.to_string(),
        package_name: value
            .get("packageName")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        note: value
            .get("note")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string),
    })
}

fn latest_pi_release() -> Result<LatestPiRelease, String> {
    if std::env::var("PI_OFFLINE").is_ok() {
        return Err(format!("Could not determine latest {APP_NAME} version."));
    }
    if let Some(release) = fixture_release() {
        return Ok(release);
    }
    if network_disabled() {
        return Err(format!(
            "Could not determine latest {APP_NAME} version: network disabled"
        ));
    }
    let agent = format!("{APP_NAME}-coding-agent/{VERSION}");
    let response = ureq::get(LATEST_VERSION_URL)
        .set("User-Agent", &agent)
        .set("accept", "application/json")
        .call()
        .map_err(|err| format!("Could not determine latest {APP_NAME} version: {err}"))?;
    let body = response
        .into_string()
        .map_err(|err| format!("Could not determine latest {APP_NAME} version: {err}"))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|err| format!("Could not determine latest {APP_NAME} version: {err}"))?;
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("Could not determine latest {APP_NAME} version."))?;
    Ok(LatestPiRelease {
        version: version.to_string(),
        package_name: value
            .get("packageName")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        note: value
            .get("note")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

pub fn self_update_plan(force: bool) -> Result<SelfUpdatePlan, String> {
    let latest = latest_pi_release()?;
    let package_name = latest
        .package_name
        .clone()
        .unwrap_or_else(|| "@earendil-works/pi-coding-agent".into());
    if force
        || package_name != "@earendil-works/pi-coding-agent"
        || is_newer_package_version(&latest.version, VERSION)
    {
        return Ok(SelfUpdatePlan {
            version: latest.version,
            should_run: true,
            note: latest.note,
        });
    }
    Ok(SelfUpdatePlan {
        version: latest.version,
        should_run: false,
        note: latest.note,
    })
}

fn lock_path(managed_root: &Path) -> PathBuf {
    managed_root.join("update.lock")
}

fn lock_is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map(|modified| modified.elapsed().unwrap_or_default() > MANAGED_LOCK_STALE)
        .unwrap_or(false)
}

struct ManagedUpdateLock {
    path: PathBuf,
}

impl ManagedUpdateLock {
    fn acquire(managed_root: &Path) -> Result<Self, String> {
        let path = lock_path(managed_root);
        if path.exists() {
            if lock_is_stale(&path) {
                let _ = fs::remove_dir_all(&path);
                let _ = fs::remove_file(&path);
            } else {
                return Err("Another managed Pi update is already running.".into());
            }
        }
        fs::create_dir_all(managed_root).map_err(|e| e.to_string())?;
        fs::create_dir(&path)
            .map_err(|_| "Another managed Pi update is already running.".to_string())?;
        Ok(Self { path })
    }
}

impl Drop for ManagedUpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
        let _ = fs::remove_file(&self.path);
    }
}

fn cleanup_managed_staging(managed_root: &Path) {
    let staging = managed_root.join("staging");
    let Ok(entries) = fs::read_dir(&staging) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("update-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn activate_managed_release(managed_root: &Path, version: &str) -> Result<(), String> {
    let current = managed_root.join("current-version");
    let temporary = managed_root.join(format!(
        "current-version.tmp.{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    fs::write(&temporary, format!("{version}\n")).map_err(|e| e.to_string())?;
    fs::rename(&temporary, &current).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(temporary);
    Ok(())
}

fn verify_managed_release(release_dir: &Path, expected: &str) -> Result<(), String> {
    let bin = release_dir.join("node_modules").join(".bin").join(APP_NAME);
    let output = Command::new(&bin)
        .arg("--version")
        .output()
        .map_err(|err| format!("Could not verify managed Pi {expected}: {err}"))?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason.trim();
        return Err(format!(
            "Could not verify managed Pi {expected}: {}",
            if reason.is_empty() {
                format!("exit code {}", output.status.code().unwrap_or(-1))
            } else {
                reason.to_string()
            }
        ));
    }
    let installed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if installed != expected {
        return Err(format!(
            "Managed Pi smoke test returned version {installed}; expected {expected}."
        ));
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry.map_err(|e| e.to_string())?;
        let rel = entry.path().strip_prefix(from).unwrap_or(entry.path());
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(entry.path(), dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn write_version_shim(release_dir: &Path, version: &str) -> Result<(), String> {
    let bin_dir = release_dir.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let bin = bin_dir.join(APP_NAME);
    fs::write(&bin, format!("#!/bin/sh\nprintf '%s\\n' {version}\n")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bin).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn run_managed_self_update(managed_root: &Path, version: &str) -> Result<(), String> {
    if !is_valid_managed_version(version) {
        return Err(format!("Invalid managed release version: {version}"));
    }
    let _lock = ManagedUpdateLock::acquire(managed_root)?;
    cleanup_managed_staging(managed_root);
    let releases_root = managed_root.join("releases");
    fs::create_dir_all(&releases_root).map_err(|e| e.to_string())?;
    let release_dir = releases_root.join(version);
    if release_dir.exists() {
        verify_managed_release(&release_dir, version)?;
        activate_managed_release(managed_root, version)?;
        return Ok(());
    }
    let staging_root = managed_root.join("staging");
    fs::create_dir_all(&staging_root).map_err(|e| e.to_string())?;
    let stage_dir = staging_root.join(format!("update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&stage_dir);
    fs::create_dir_all(&stage_dir).map_err(|e| e.to_string())?;
    if let Ok(fixture) = std::env::var("PI_MANAGED_RELEASE_FIXTURE") {
        copy_tree(Path::new(&fixture), &stage_dir)?;
        if !stage_dir
            .join("node_modules")
            .join(".bin")
            .join(APP_NAME)
            .exists()
        {
            write_version_shim(&stage_dir, version)?;
        }
    } else if network_disabled() {
        let _ = fs::remove_dir_all(&stage_dir);
        let base = std::env::var("PI_INSTALLER_API_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INSTALLER_API_BASE.into());
        let base = base.trim_end_matches('/');
        return Err(format!(
            "Could not download managed installer package.json from {base}/{version}/package.json: network disabled"
        ));
    } else {
        let _ = fs::remove_dir_all(&stage_dir);
        return Err(format!(
            "Could not download managed installer package.json from {DEFAULT_INSTALLER_API_BASE}/{version}/package.json"
        ));
    }
    verify_managed_release(&stage_dir, version).inspect_err(|_| {
        let _ = fs::remove_dir_all(&stage_dir);
    })?;
    fs::rename(&stage_dir, &release_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&stage_dir);
        e.to_string()
    })?;
    activate_managed_release(managed_root, version)?;
    cleanup_managed_staging(managed_root);
    Ok(())
}

pub fn format_update_note(note: &str) -> String {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\nUpdate note\n{trimmed}\n\n")
}
