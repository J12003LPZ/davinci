use crate::args::{APP_NAME, PACKAGE_NAME, VERSION};
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
    pub package_name: String,
    pub install_spec: String,
    pub should_run: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SelfUpdateCommand {
    pub display: String,
    pub steps: Vec<(String, Vec<String>, String)>,
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
        .unwrap_or_else(|| PACKAGE_NAME.into());
    let install_spec = format!("{}@{}", package_name, latest.version);
    if force || package_name != PACKAGE_NAME || is_newer_package_version(&latest.version, VERSION) {
        return Ok(SelfUpdatePlan {
            version: latest.version,
            package_name,
            install_spec,
            should_run: true,
            note: latest.note,
        });
    }
    Ok(SelfUpdatePlan {
        version: latest.version,
        package_name,
        install_spec,
        should_run: false,
        note: latest.note,
    })
}

pub fn detect_install_method() -> &'static str {
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
    let package = package_dir()
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let resolved = format!("{package}\0{exe}");
    if resolved.contains("/pnpm/") || resolved.contains("/.pnpm/") {
        "pnpm"
    } else if resolved.contains("/yarn/") || resolved.contains("/.yarn/") {
        "yarn"
    } else if resolved.contains("/install/global/node_modules/") {
        "bun"
    } else if resolved.contains("/npm/") || resolved.contains("/node_modules/") {
        "npm"
    } else {
        "unknown"
    }
}

fn quote_arg(arg: &str) -> String {
    if arg.chars().any(char::is_whitespace) {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

fn format_command_display(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(quote_arg)
        .collect::<Vec<_>>()
        .join(" ")
}

fn make_step(command: &str, args: Vec<String>) -> (String, Vec<String>, String) {
    let display = format_command_display(command, &args);
    (command.to_string(), args, display)
}

fn split_npm_command(npm_command: Option<&[String]>) -> (String, Vec<String>) {
    match npm_command {
        Some(args) if !args.is_empty() => (args[0].clone(), args[1..].to_vec()),
        _ => ("npm".into(), Vec::new()),
    }
}

fn read_command_output(
    command: &str,
    args: &[String],
    require_success: bool,
) -> Result<Option<String>, String> {
    match Command::new(command).args(args).output() {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok((!stdout.is_empty()).then_some(stdout));
            }
            if !require_success {
                return Ok(None);
            }
            let reason = String::from_utf8_lossy(&output.stderr);
            let reason = reason.trim();
            Err(format!(
                "Failed to run {} {}: {}",
                command,
                args.join(" "),
                if reason.is_empty() {
                    format!("exit code {}", output.status.code().unwrap_or(-1))
                } else {
                    reason.to_string()
                }
            ))
        }
        Err(err) if require_success => Err(format!("Failed to run {command}: {err}")),
        Err(_) => Ok(None),
    }
}

fn inferred_npm_install(package: &Path) -> Option<(PathBuf, PathBuf)> {
    let parent = package.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    let root = if parent_name.starts_with('@')
        && parent.parent()?.file_name()?.to_str() == Some("node_modules")
    {
        parent.parent()?.to_path_buf()
    } else if parent_name == "node_modules" {
        parent.to_path_buf()
    } else {
        return None;
    };
    let prefix = root.parent().and_then(|p| {
        if p.file_name()?.to_str() == Some("lib") {
            p.parent().map(Path::to_path_buf)
        } else {
            None
        }
    })?;
    Some((root, prefix))
}

fn path_candidates(path: &Path) -> Vec<PathBuf> {
    if !path.exists() {
        return Vec::new();
    }
    let mut out = vec![canonicalize_path(path)];
    if let Ok(raw) = path.canonicalize() {
        if !out.contains(&raw) {
            out.push(raw);
        }
    }
    out
}

fn path_is_under(child: &Path, root: &Path) -> bool {
    if child == root {
        return true;
    }
    child.starts_with(root)
}

fn dir_writable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let probe = path.join(format!(".pi-write-probe-{}", std::process::id()));
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn self_update_path_writable() -> bool {
    let package = package_dir();
    let parent = package.parent().unwrap_or(&package);
    dir_writable(&package) && dir_writable(parent)
}

fn pnpm_bin_dir_args() -> Vec<String> {
    let package = package_dir().to_string_lossy().replace('\\', "/");
    let re = regex::Regex::new(r"^(.*[/]global/[^/]+)/\.pnpm/").expect("pnpm path regex");
    if let Some(caps) = re.captures(&package) {
        let home = std::env::var("PNPM_HOME").ok().unwrap_or_else(|| {
            PathBuf::from(&caps[1])
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        });
        vec![format!("--config.global-bin-dir={home}")]
    } else {
        Vec::new()
    }
}

fn global_package_roots(
    method: &str,
    npm_command: Option<&[String]>,
) -> Result<Vec<PathBuf>, String> {
    match method {
        "npm" => {
            let configured = npm_command.is_some_and(|c| !c.is_empty());
            let (command, mut args) = split_npm_command(npm_command);
            if configured && command == "bun" {
                let mut roots = vec![dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".bun")
                    .join("install")
                    .join("global")
                    .join("node_modules")];
                args.extend(["pm".into(), "bin".into(), "-g".into()]);
                if let Some(bin) = read_command_output(&command, &args, true)? {
                    roots.push(
                        PathBuf::from(bin)
                            .join("install")
                            .join("global")
                            .join("node_modules"),
                    );
                }
                return Ok(roots);
            }
            let mut root_args = args.clone();
            root_args.extend(["root".into(), "-g".into()]);
            let mut roots = Vec::new();
            if let Some(root) = read_command_output(&command, &root_args, configured)? {
                roots.push(PathBuf::from(root));
            }
            if !configured {
                if let Some((root, _)) = inferred_npm_install(&package_dir()) {
                    roots.push(root);
                }
            }
            Ok(roots)
        }
        "pnpm" => {
            let mut roots = Vec::new();
            if let Some(root) = read_command_output("pnpm", &["root".into(), "-g".into()], false)? {
                let root = PathBuf::from(root);
                if let Some(parent) = root.parent() {
                    roots.push(parent.to_path_buf());
                }
                roots.push(root);
            } else {
                let package = package_dir().to_string_lossy().replace('\\', "/");
                let re =
                    regex::Regex::new(r"^(.*[/]global/[^/]+)/\.pnpm/").expect("pnpm path regex");
                if let Some(caps) = re.captures(&package) {
                    roots.push(PathBuf::from(&caps[1]));
                }
            }
            Ok(roots)
        }
        "yarn" => {
            let mut roots = Vec::new();
            if let Some(dir) = read_command_output("yarn", &["global".into(), "dir".into()], false)?
            {
                let dir = PathBuf::from(dir);
                roots.push(dir.join("node_modules"));
                roots.push(dir);
            }
            Ok(roots)
        }
        "bun" => {
            let mut roots = vec![dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".bun")
                .join("install")
                .join("global")
                .join("node_modules")];
            if let Some(bin) =
                read_command_output("bun", &["pm".into(), "bin".into(), "-g".into()], false)?
            {
                roots.push(
                    PathBuf::from(bin)
                        .join("install")
                        .join("global")
                        .join("node_modules"),
                );
            }
            Ok(roots)
        }
        _ => Ok(Vec::new()),
    }
}

fn managed_by_global_package_manager(
    method: &str,
    npm_command: Option<&[String]>,
) -> Result<bool, String> {
    let package_dirs = path_candidates(&package_dir());
    let roots = global_package_roots(method, npm_command)?;
    Ok(roots.iter().any(|root| {
        path_candidates(root).iter().any(|normalized| {
            package_dirs
                .iter()
                .any(|pkg| path_is_under(pkg, normalized))
        })
    }))
}

fn command_for_method(
    method: &str,
    installed_package_name: &str,
    target: &SelfUpdatePlan,
    npm_command: Option<&[String]>,
) -> Option<SelfUpdateCommand> {
    let install_spec = target.install_spec.clone();
    match method {
        "bun-binary" | "unknown" => None,
        "pnpm" => {
            let bin_dir_args = if read_command_output("pnpm", &["root".into(), "-g".into()], false)
                .ok()
                .flatten()
                .is_none()
            {
                pnpm_bin_dir_args()
            } else {
                Vec::new()
            };
            let mut install = vec![
                "install".into(),
                "-g".into(),
                "--ignore-scripts".into(),
                "--config.minimumReleaseAge=0".into(),
            ];
            install.extend(bin_dir_args.clone());
            install.push(install_spec);
            let install_step = make_step("pnpm", install);
            let uninstall = if target.package_name != installed_package_name {
                let mut args = vec!["remove".into(), "-g".into()];
                args.extend(bin_dir_args);
                args.push(installed_package_name.into());
                Some(make_step("pnpm", args))
            } else {
                None
            };
            Some(finish_command(install_step, uninstall))
        }
        "yarn" => {
            let install_step = make_step(
                "yarn",
                vec![
                    "global".into(),
                    "add".into(),
                    "--ignore-scripts".into(),
                    install_spec,
                ],
            );
            let uninstall = if target.package_name != installed_package_name {
                Some(make_step(
                    "yarn",
                    vec![
                        "global".into(),
                        "remove".into(),
                        installed_package_name.into(),
                    ],
                ))
            } else {
                None
            };
            Some(finish_command(install_step, uninstall))
        }
        "bun" => {
            let install_step = make_step(
                "bun",
                vec![
                    "install".into(),
                    "-g".into(),
                    "--ignore-scripts".into(),
                    "--minimum-release-age=0".into(),
                    install_spec,
                ],
            );
            let uninstall = if target.package_name != installed_package_name {
                Some(make_step(
                    "bun",
                    vec![
                        "uninstall".into(),
                        "-g".into(),
                        installed_package_name.into(),
                    ],
                ))
            } else {
                None
            };
            Some(finish_command(install_step, uninstall))
        }
        "npm" => {
            let (command, npm_args) = split_npm_command(npm_command);
            let inferred = if npm_command.is_some_and(|c| !c.is_empty()) {
                None
            } else {
                inferred_npm_install(&package_dir())
            };
            let mut prefix_args = npm_args;
            if let Some((_, prefix)) = inferred {
                prefix_args.push("--prefix".into());
                prefix_args.push(prefix.display().to_string());
            }
            let mut install = prefix_args.clone();
            install.extend([
                "install".into(),
                "-g".into(),
                "--ignore-scripts".into(),
                "--min-release-age=0".into(),
                install_spec,
            ]);
            let install_step = make_step(&command, install);
            let uninstall = if target.package_name != installed_package_name {
                let mut args = prefix_args;
                args.extend([
                    "uninstall".into(),
                    "-g".into(),
                    installed_package_name.into(),
                ]);
                Some(make_step(&command, args))
            } else {
                None
            };
            Some(finish_command(install_step, uninstall))
        }
        _ => None,
    }
}

fn finish_command(
    install: (String, Vec<String>, String),
    uninstall: Option<(String, Vec<String>, String)>,
) -> SelfUpdateCommand {
    if let Some(uninstall) = uninstall {
        SelfUpdateCommand {
            display: format!("{} && {}", uninstall.2, install.2),
            steps: vec![uninstall, install],
        }
    } else {
        SelfUpdateCommand {
            display: install.2.clone(),
            steps: vec![install],
        }
    }
}

pub fn self_update_command(
    npm_command: Option<&[String]>,
    plan: &SelfUpdatePlan,
) -> Result<Option<SelfUpdateCommand>, String> {
    let method = detect_install_method();
    let command = match command_for_method(method, PACKAGE_NAME, plan, npm_command) {
        Some(command) => command,
        None => return Ok(None),
    };
    if !managed_by_global_package_manager(method, npm_command)? || !self_update_path_writable() {
        return Ok(None);
    }
    Ok(Some(command))
}

pub fn self_update_unavailable_instruction(
    npm_command: Option<&[String]>,
    plan: &SelfUpdatePlan,
) -> String {
    let method = detect_install_method();
    if method == "bun-binary" {
        return "Download from: https://github.com/earendil-works/pi-mono/releases/latest".into();
    }
    if let Some(command) = command_for_method(method, PACKAGE_NAME, plan, npm_command) {
        if managed_by_global_package_manager(method, npm_command).unwrap_or(false)
            && !self_update_path_writable()
        {
            return format!(
                "This installation is managed by a global {method} install, but the install path is not writable. Update it yourself with: {}",
                command.display
            );
        }
        return format!(
            "This installation is not managed by a global {method} install. Update it with the package manager, wrapper, or source checkout that provides it."
        );
    }
    format!(
        "Update {} using the package manager, wrapper, or source checkout that provides this installation.",
        plan.install_spec
    )
}

pub fn run_self_update_command(command: &SelfUpdateCommand) -> Result<(), String> {
    for (program, args, display) in &command.steps {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|err| format!("{display} exited with code unknown: {err}"))?;
        if !output.status.success() {
            if let Some(signal) = output.status.code() {
                return Err(format!("{display} exited with code {signal}"));
            }
            return Err(format!("{display} terminated by signal"));
        }
    }
    Ok(())
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
