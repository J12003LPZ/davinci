//! Package-manager self-update matching `vendor/pi/packages/coding-agent/src/config.ts`
//! and managed-install updates from `package-manager-cli.ts`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PACKAGE_NAME: &str = "@earendil-works/pi-coding-agent";
pub const BUN_BINARY_DOWNLOAD: &str =
    "Download from: https://github.com/earendil-works/pi-mono/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    BunBinary,
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Unknown,
}

impl InstallMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BunBinary => "bun-binary",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdateCommandStep {
    pub command: String,
    pub args: Vec<String>,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdateCommand {
    pub command: String,
    pub args: Vec<String>,
    pub display: String,
    pub steps: Option<Vec<SelfUpdateCommandStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTarget {
    pub package_name: String,
    pub install_spec: String,
}

impl PackageTarget {
    pub fn new(package_name: impl Into<String>, install_spec: Option<String>) -> Self {
        let package_name = package_name.into();
        let install_spec = install_spec.unwrap_or_else(|| package_name.clone());
        Self {
            package_name,
            install_spec,
        }
    }
}

/// TS `detectInstallMethod`: `${__dirname}\0${process.execPath}` lowercased, `\\` → `/`.
pub fn detect_install_method(
    resolved_path: &str,
    bun_binary: bool,
    bun_runtime: bool,
) -> InstallMethod {
    if bun_binary {
        return InstallMethod::BunBinary;
    }
    let path = resolved_path.to_ascii_lowercase().replace('\\', "/");
    if path.contains("/pnpm/") || path.contains("/.pnpm/") {
        return InstallMethod::Pnpm;
    }
    if path.contains("/yarn/") || path.contains("/.yarn/") {
        return InstallMethod::Yarn;
    }
    if bun_runtime || path.contains("/install/global/node_modules/") {
        return InstallMethod::Bun;
    }
    if path.contains("/npm/") || path.contains("/node_modules/") {
        return InstallMethod::Npm;
    }
    InstallMethod::Unknown
}

pub fn detect_path_from_env() -> String {
    let package_dir = std::env::var("PI_PACKAGE_DIR").unwrap_or_default();
    let exec_path = std::env::var("PI_EXEC_PATH")
        .ok()
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_default();
    format!("{package_dir}\0{exec_path}")
}

pub fn is_bun_binary() -> bool {
    matches!(
        std::env::var("PI_BUN_BINARY").as_deref(),
        Ok("1") | Ok("true")
    )
}

pub fn is_bun_runtime() -> bool {
    matches!(
        std::env::var("PI_BUN_RUNTIME").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn quote_display_arg(arg: &str) -> String {
    if arg.chars().any(char::is_whitespace) {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

fn make_step(command: &str, args: Vec<String>) -> SelfUpdateCommandStep {
    let display = std::iter::once(command.to_string())
        .chain(args.iter().cloned())
        .map(|arg| quote_display_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    SelfUpdateCommandStep {
        command: command.into(),
        args,
        display,
    }
}

fn make_command(
    install: SelfUpdateCommandStep,
    uninstall: Option<SelfUpdateCommandStep>,
) -> SelfUpdateCommand {
    match uninstall {
        None => SelfUpdateCommand {
            command: install.command,
            args: install.args,
            display: install.display,
            steps: None,
        },
        Some(uninstall) => SelfUpdateCommand {
            command: install.command.clone(),
            args: install.args.clone(),
            display: format!("{} && {}", uninstall.display, install.display),
            steps: Some(vec![uninstall, install]),
        },
    }
}

/// TS `getInferredNpmInstall` — skip Windows custom prefixes.
pub fn inferred_npm_prefix(package_dir: &str, windows: bool) -> Option<String> {
    if windows || package_dir.contains('\\') {
        return None;
    }
    let package = Path::new(package_dir);
    let parent = package.parent()?;
    let root = if parent
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('@'))
        && parent.parent()?.file_name().and_then(|n| n.to_str()) == Some("node_modules")
    {
        parent.parent()?.to_path_buf()
    } else if parent.file_name().and_then(|n| n.to_str()) == Some("node_modules") {
        parent.to_path_buf()
    } else {
        return None;
    };
    let root_parent = root.parent()?;
    if root_parent.file_name().and_then(|n| n.to_str()) == Some("lib") {
        return Some(root_parent.parent()?.display().to_string());
    }
    None
}

pub fn self_update_command_for_method(
    method: InstallMethod,
    installed_package_name: &str,
    target: &PackageTarget,
    npm_command: Option<&[String]>,
    inferred_prefix: Option<&str>,
    pnpm_global_bin_dir: Option<&str>,
) -> Option<SelfUpdateCommand> {
    match method {
        InstallMethod::BunBinary | InstallMethod::Unknown => None,
        InstallMethod::Pnpm => {
            let mut args = vec![
                "install".into(),
                "-g".into(),
                "--ignore-scripts".into(),
                "--config.minimumReleaseAge=0".into(),
            ];
            let mut uninstall_args = vec!["remove".into(), "-g".into()];
            if let Some(bin_dir) = pnpm_global_bin_dir {
                let flag = format!("--config.global-bin-dir={bin_dir}");
                args.push(flag.clone());
                uninstall_args.push(flag);
            }
            args.push(target.install_spec.clone());
            let uninstall = (target.package_name != installed_package_name).then(|| {
                uninstall_args.push(installed_package_name.to_string());
                make_step("pnpm", uninstall_args)
            });
            Some(make_command(make_step("pnpm", args), uninstall))
        }
        InstallMethod::Yarn => {
            let install = make_step(
                "yarn",
                vec![
                    "global".into(),
                    "add".into(),
                    "--ignore-scripts".into(),
                    target.install_spec.clone(),
                ],
            );
            let uninstall = (target.package_name != installed_package_name).then(|| {
                make_step(
                    "yarn",
                    vec![
                        "global".into(),
                        "remove".into(),
                        installed_package_name.to_string(),
                    ],
                )
            });
            Some(make_command(install, uninstall))
        }
        InstallMethod::Bun => {
            let install = make_step(
                "bun",
                vec![
                    "install".into(),
                    "-g".into(),
                    "--ignore-scripts".into(),
                    "--minimum-release-age=0".into(),
                    target.install_spec.clone(),
                ],
            );
            let uninstall = (target.package_name != installed_package_name).then(|| {
                make_step(
                    "bun",
                    vec![
                        "uninstall".into(),
                        "-g".into(),
                        installed_package_name.to_string(),
                    ],
                )
            });
            Some(make_command(install, uninstall))
        }
        InstallMethod::Npm => {
            let (command, npm_args) = match npm_command {
                Some(parts) if !parts.is_empty() => {
                    let command = parts[0].as_str();
                    (command.to_string(), parts[1..].to_vec())
                }
                _ => ("npm".into(), Vec::new()),
            };
            let mut prefix_args = npm_args;
            if npm_command.map(|p| !p.is_empty()).unwrap_or(false) {
                // configured npmCommand: do not infer prefix
            } else if let Some(prefix) = inferred_prefix {
                prefix_args.push("--prefix".into());
                prefix_args.push(prefix.to_string());
            }
            let mut install_args = prefix_args.clone();
            install_args.extend([
                "install".into(),
                "-g".into(),
                "--ignore-scripts".into(),
                "--min-release-age=0".into(),
                target.install_spec.clone(),
            ]);
            let uninstall = (target.package_name != installed_package_name).then(|| {
                let mut args = prefix_args;
                args.extend([
                    "uninstall".into(),
                    "-g".into(),
                    installed_package_name.to_string(),
                ]);
                make_step(&command, args)
            });
            Some(make_command(make_step(&command, install_args), uninstall))
        }
    }
}

pub fn self_update_unavailable_instruction(
    method: InstallMethod,
    installed_package_name: &str,
    target: &PackageTarget,
    command: Option<&SelfUpdateCommand>,
    managed: bool,
    writable: bool,
) -> String {
    if method == InstallMethod::BunBinary {
        return BUN_BINARY_DOWNLOAD.into();
    }
    if let Some(command) = command {
        if managed && !writable {
            return format!(
                "This installation is managed by a global {} install, but the install path is not writable. Update it yourself with: {}",
                method.as_str(),
                command.display
            );
        }
        return format!(
            "This installation is not managed by a global {} install. Update it with the package manager, wrapper, or source checkout that provides it.",
            method.as_str()
        );
    }
    let _ = installed_package_name;
    format!(
        "Update {} using the package manager, wrapper, or source checkout that provides this installation.",
        target.install_spec
    )
}

pub fn update_instruction(
    _method: InstallMethod,
    command: Option<&SelfUpdateCommand>,
    fallback: &str,
) -> String {
    match command {
        Some(command) => format!("Run: {}", command.display),
        None => fallback.to_string(),
    }
}

pub fn current_install_method() -> InstallMethod {
    detect_install_method(&detect_path_from_env(), is_bun_binary(), is_bun_runtime())
}

pub const DEFAULT_INSTALLER_API_BASE: &str = "https://pi.dev/api/installer/releases";
pub const MANAGED_INSTALL_MARKER: &str = "managed-install.json";
pub const MANAGED_NPM_CI_ARGS: &[&str] = &[
    "ci",
    "--ignore-scripts",
    "--min-release-age=0",
    "--omit=dev",
    "--include=optional",
    "--no-fund",
    "--no-audit",
    "--loglevel=error",
    "--progress=false",
];

const MANAGED_KIND: &str = "pi-managed-install";
const MANAGED_LAYOUT: &str = "releases-v1";

/// TS `MANAGED_RELEASE_VERSION_RE`.
pub fn is_managed_release_version(version: &str) -> bool {
    let (core, rest) = match version.find(['-', '+']) {
        Some(index) => (&version[..index], Some(&version[index..])),
        None => (version, None),
    };
    let mut parts = core.split('.');
    let major = parts.next();
    let minor = parts.next();
    let patch = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let digits = |value: Option<&str>| {
        value.is_some_and(|item| !item.is_empty() && item.chars().all(|ch| ch.is_ascii_digit()))
    };
    if !digits(major) || !digits(minor) || !digits(patch) {
        return false;
    }
    match rest {
        None => true,
        Some(rest) => rest
            .chars()
            .all(|ch| ch == '-' || ch == '+' || ch.is_ascii_alphanumeric() || ch == '.'),
    }
}

fn path_is_inside(child: &Path, parent: &Path) -> bool {
    let child = child.to_string_lossy().replace('\\', "/");
    let parent = parent.to_string_lossy().replace('\\', "/");
    let parent = parent.trim_end_matches('/');
    child == parent || child.starts_with(&format!("{parent}/"))
}

/// TS `getActiveManagedInstallRoot`.
pub fn get_active_managed_install_root() -> Result<Option<PathBuf>, String> {
    let configured = std::env::var("PI_MANAGED_INSTALL_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(configured) = configured else {
        return Ok(None);
    };
    let managed_root = PathBuf::from(&configured);
    let releases_dir = managed_root.join("releases");
    if let Some(package_dir) = package_dir_from_env() {
        if !path_is_inside(&package_dir, &releases_dir) {
            return Ok(None);
        }
    } else {
        return Ok(None);
    }
    let marker_path = managed_root.join(MANAGED_INSTALL_MARKER);
    let raw = fs::read_to_string(&marker_path).map_err(|_| {
        format!(
            "Managed install marker is missing or invalid: {}",
            marker_path.display()
        )
    })?;
    let marker: serde_json::Value = serde_json::from_str(&raw).map_err(|_| {
        format!(
            "Managed install marker is missing or invalid: {}",
            marker_path.display()
        )
    })?;
    let kind = marker.get("kind").and_then(|value| value.as_str());
    let layout = marker.get("layout").and_then(|value| value.as_str());
    let schema = marker.get("schemaVersion").and_then(|value| value.as_u64());
    if kind != Some(MANAGED_KIND) || schema != Some(1) || layout != Some(MANAGED_LAYOUT) {
        return Err(format!(
            "Managed install marker is missing or invalid: {}",
            marker_path.display()
        ));
    }
    Ok(Some(managed_root))
}

struct ManagedUpdateLock {
    path: PathBuf,
}

impl Drop for ManagedUpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_managed_update_lock(managed_root: &Path) -> Result<ManagedUpdateLock, String> {
    let target = managed_root.join("update");
    if !target.exists() {
        fs::write(&target, b"").map_err(|err| err.to_string())?;
    }
    let lock_path = managed_root.join("update.lock");
    if lock_path.exists() {
        return Err("Another managed Pi update is already running.".into());
    }
    fs::write(&lock_path, b"locked").map_err(|err| err.to_string())?;
    Ok(ManagedUpdateLock { path: lock_path })
}

pub fn activate_managed_release(managed_root: &Path, version: &str) -> Result<(), String> {
    let current_path = managed_root.join("current-version");
    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temporary_path = managed_root.join(format!("current-version.tmp.{pid}-{now}"));
    let result = (|| {
        fs::write(&temporary_path, format!("{version}\n")).map_err(|err| err.to_string())?;
        fs::rename(&temporary_path, &current_path).map_err(|err| err.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary_path);
    result
}

fn installer_api_base() -> String {
    std::env::var("PI_INSTALLER_API_BASE")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_INSTALLER_API_BASE.to_string())
}

fn fetch_installer_artifact(url: &str, label: &str) -> Result<String, String> {
    if let Ok(reply) = std::env::var("PI_MANAGED_INSTALLER_REPLY") {
        if !reply.is_empty() {
            if let Ok(map) = serde_json::from_str::<serde_json::Value>(&reply) {
                if let Some(value) = map.get(label).and_then(|item| item.as_str()) {
                    return Ok(value.to_string());
                }
                if let Some(value) = map.get(url).and_then(|item| item.as_str()) {
                    return Ok(value.to_string());
                }
            }
            if label == "package.json" {
                return Ok(reply);
            }
        }
    }
    if let Ok(lock) = std::env::var("PI_MANAGED_INSTALLER_LOCK_REPLY") {
        if label == "package-lock.json" && !lock.is_empty() {
            return Ok(lock);
        }
    }
    if std::env::var("PI_MANAGED_INSTALL_DRY_RUN").is_ok()
        || std::env::var("PI_MANAGED_INSTALLER_REPLY").is_ok()
    {
        return Ok(if label == "package-lock.json" {
            r#"{"lockfileVersion":3}"#.into()
        } else {
            r#"{"name":"@earendil-works/pi-coding-agent"}"#.into()
        });
    }
    Err(format!(
        "Could not download managed installer {label} from {url}: refusing live HTTP (set PI_MANAGED_INSTALLER_REPLY or PI_MANAGED_INSTALL_DRY_RUN)"
    ))
}

fn cleanup_managed_staging(managed_root: &Path) {
    let staging_root = managed_root.join("staging");
    let Ok(entries) = fs::read_dir(&staging_root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("update-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// TS `runManagedSelfUpdate` (fixtures only — never hits pi.dev from tests).
pub fn run_managed_self_update(managed_root: &Path, version: &str) -> Result<(), String> {
    if !is_managed_release_version(version) {
        return Err(format!("Invalid managed release version: {version}"));
    }
    let _lock = acquire_managed_update_lock(managed_root)?;
    cleanup_managed_staging(managed_root);
    let releases_root = managed_root.join("releases");
    fs::create_dir_all(&releases_root).map_err(|err| err.to_string())?;
    let release_dir = releases_root.join(version);
    if release_dir.exists() {
        activate_managed_release(managed_root, version)?;
        return Ok(());
    }
    let installer_api_base = installer_api_base();
    let release_url = format!("{installer_api_base}/{}", urlencoding_encode(version));
    let staging_root = managed_root.join("staging");
    fs::create_dir_all(&staging_root).map_err(|err| err.to_string())?;
    let stage_dir = staging_root.join(format!("update-{}", std::process::id()));
    fs::create_dir_all(&stage_dir).map_err(|err| err.to_string())?;
    let package_json =
        fetch_installer_artifact(&format!("{release_url}/package.json"), "package.json")?;
    let package_lock = fetch_installer_artifact(
        &format!("{release_url}/package-lock.json"),
        "package-lock.json",
    )?;
    fs::write(stage_dir.join("package.json"), package_json).map_err(|err| err.to_string())?;
    fs::write(stage_dir.join("package-lock.json"), package_lock).map_err(|err| err.to_string())?;
    if std::env::var("PI_MANAGED_INSTALL_DRY_RUN").is_err()
        && std::env::var("PI_MANAGED_INSTALLER_REPLY").is_err()
    {
        return Err(format!(
            "npm {} exited with code unknown",
            MANAGED_NPM_CI_ARGS.join(" ")
        ));
    }
    fs::rename(&stage_dir, &release_dir).map_err(|err| err.to_string())?;
    activate_managed_release(managed_root, version)?;
    Ok(())
}

fn urlencoding_encode(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

pub fn package_dir_from_env() -> Option<PathBuf> {
    std::env::var("PI_PACKAGE_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_methods_and_locks_ts_argv() {
        assert_eq!(
            detect_install_method(
                r"C:\Users\Admin\Documents\pnpm-repository\global\5\.pnpm\@earendil-works+pi-coding-agent@0.67.68\node_modules\@earendil-works\pi-coding-agent\dist\cli.js",
                false,
                false,
            ),
            InstallMethod::Pnpm
        );
        assert_eq!(
            detect_install_method("/usr/local/bin/node", false, false),
            InstallMethod::Unknown
        );
        assert_eq!(
            detect_install_method(
                "/usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js",
                false,
                false
            ),
            InstallMethod::Npm
        );
        assert_eq!(
            detect_install_method(
                "/home/u/.bun/install/global/node_modules/@earendil-works/pi-coding-agent",
                false,
                false
            ),
            InstallMethod::Bun
        );
        assert_eq!(
            detect_install_method(
                "/home/u/.yarn/global/node_modules/@earendil-works/pi-coding-agent",
                false,
                false
            ),
            InstallMethod::Yarn
        );
        assert_eq!(
            detect_install_method("/anything", true, false),
            InstallMethod::BunBinary
        );

        let target = PackageTarget::new(PACKAGE_NAME, None);
        let pnpm = self_update_command_for_method(
            InstallMethod::Pnpm,
            PACKAGE_NAME,
            &target,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            pnpm.args,
            [
                "install",
                "-g",
                "--ignore-scripts",
                "--config.minimumReleaseAge=0",
                PACKAGE_NAME
            ]
        );
        assert_eq!(
            pnpm.display,
            "pnpm install -g --ignore-scripts --config.minimumReleaseAge=0 @earendil-works/pi-coding-agent"
        );

        let npm = self_update_command_for_method(
            InstallMethod::Npm,
            PACKAGE_NAME,
            &target,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            npm.args,
            [
                "install",
                "-g",
                "--ignore-scripts",
                "--min-release-age=0",
                PACKAGE_NAME
            ]
        );

        let prefix = "/opt/pi prefix ";
        let npm_prefix = self_update_command_for_method(
            InstallMethod::Npm,
            PACKAGE_NAME,
            &target,
            Some(&["npm".into(), "--prefix".into(), prefix.into()]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            npm_prefix.display,
            r#"npm --prefix "/opt/pi prefix " install -g --ignore-scripts --min-release-age=0 @earendil-works/pi-coding-agent"#
        );

        let bun = self_update_command_for_method(
            InstallMethod::Bun,
            PACKAGE_NAME,
            &target,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            bun.display,
            "bun install -g --ignore-scripts --minimum-release-age=0 @earendil-works/pi-coding-agent"
        );

        let renamed = PackageTarget::new("@new-scope/pi", None);
        let yarn = self_update_command_for_method(
            InstallMethod::Yarn,
            "@mariozechner/pi-coding-agent",
            &renamed,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            yarn.display,
            "yarn global remove @mariozechner/pi-coding-agent && yarn global add --ignore-scripts @new-scope/pi"
        );
        assert_eq!(yarn.steps.as_ref().unwrap().len(), 2);

        assert!(self_update_command_for_method(
            InstallMethod::Unknown,
            PACKAGE_NAME,
            &target,
            None,
            None,
            None
        )
        .is_none());
        assert_eq!(
            self_update_unavailable_instruction(
                InstallMethod::BunBinary,
                PACKAGE_NAME,
                &target,
                None,
                false,
                true
            ),
            BUN_BINARY_DOWNLOAD
        );
        assert_eq!(
            update_instruction(InstallMethod::Unknown, None, &self_update_unavailable_instruction(
                InstallMethod::Unknown,
                PACKAGE_NAME,
                &target,
                None,
                false,
                true
            )),
            "Update @earendil-works/pi-coding-agent using the package manager, wrapper, or source checkout that provides this installation."
        );
        assert!(inferred_npm_prefix(
            r"C:\Users\Admin\npm prefix\node_modules\@earendil-works\pi-coding-agent",
            true
        )
        .is_none());
        assert_eq!(
            inferred_npm_prefix(
                "/usr/lib/node_modules/@earendil-works/pi-coding-agent",
                false
            )
            .as_deref(),
            Some("/usr")
        );
    }

    #[test]
    fn managed_install_marker_lock_and_activate() {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("managed");
        let release = root.join("releases").join("1.2.3").join("pkg");
        std::fs::create_dir_all(&release).unwrap();
        std::env::set_var("PI_MANAGED_INSTALL_ROOT", root.display().to_string());
        std::env::set_var("PI_PACKAGE_DIR", release.display().to_string());
        std::env::set_var("PI_MANAGED_INSTALL_DRY_RUN", "1");
        std::env::remove_var("PI_MANAGED_INSTALLER_REPLY");

        let missing = get_active_managed_install_root().unwrap_err();
        assert!(missing.contains("Managed install marker is missing or invalid"));

        std::fs::write(
            root.join(MANAGED_INSTALL_MARKER),
            r#"{"kind":"wrong","schemaVersion":1,"layout":"releases-v1"}"#,
        )
        .unwrap();
        let invalid = get_active_managed_install_root().unwrap_err();
        assert!(invalid.contains("Managed install marker is missing or invalid"));

        std::fs::write(
            root.join(MANAGED_INSTALL_MARKER),
            r#"{"kind":"pi-managed-install","schemaVersion":1,"layout":"releases-v1"}"#,
        )
        .unwrap();
        assert_eq!(
            get_active_managed_install_root().unwrap().as_deref(),
            Some(root.as_path())
        );

        assert!(!is_managed_release_version("v1.2.3"));
        assert!(is_managed_release_version("1.2.3"));
        assert!(is_managed_release_version("1.2.3-beta.1+build"));
        let bad = run_managed_self_update(&root, "not-a-version").unwrap_err();
        assert_eq!(bad, "Invalid managed release version: not-a-version");

        std::fs::write(root.join("update.lock"), b"locked").unwrap();
        let locked = run_managed_self_update(&root, "1.2.4").unwrap_err();
        assert_eq!(locked, "Another managed Pi update is already running.");
        std::fs::remove_file(root.join("update.lock")).unwrap();

        run_managed_self_update(&root, "1.2.3").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("current-version")).unwrap(),
            "1.2.3\n"
        );

        std::env::set_var(
            "PI_MANAGED_INSTALLER_REPLY",
            r#"{"name":"@earendil-works/pi-coding-agent","version":"9.9.9"}"#,
        );
        run_managed_self_update(&root, "9.9.9").unwrap();
        assert!(root
            .join("releases")
            .join("9.9.9")
            .join("package.json")
            .exists());
        assert_eq!(
            std::fs::read_to_string(root.join("current-version")).unwrap(),
            "9.9.9\n"
        );
        assert_eq!(
            MANAGED_NPM_CI_ARGS,
            [
                "ci",
                "--ignore-scripts",
                "--min-release-age=0",
                "--omit=dev",
                "--include=optional",
                "--no-fund",
                "--no-audit",
                "--loglevel=error",
                "--progress=false"
            ]
        );

        std::env::remove_var("PI_MANAGED_INSTALL_ROOT");
        std::env::remove_var("PI_PACKAGE_DIR");
        std::env::remove_var("PI_MANAGED_INSTALL_DRY_RUN");
        std::env::remove_var("PI_MANAGED_INSTALLER_REPLY");
    }
}
