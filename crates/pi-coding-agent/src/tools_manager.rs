//! Managed `fd` / `rg` binaries matching TS `utils/tools-manager.ts`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::args::APP_NAME;
use pi_session::default_agent_dir;

const NETWORK_TIMEOUT_MS: u64 = 10_000;
const DOWNLOAD_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTool {
    Fd,
    Rg,
}

impl ManagedTool {
    fn name(self) -> &'static str {
        match self {
            Self::Fd => "fd",
            Self::Rg => "ripgrep",
        }
    }

    fn repo(self) -> &'static str {
        match self {
            Self::Fd => "sharkdp/fd",
            Self::Rg => "BurntSushi/ripgrep",
        }
    }

    fn binary_name(self) -> &'static str {
        match self {
            Self::Fd => "fd",
            Self::Rg => "rg",
        }
    }

    fn tag_prefix(self) -> &'static str {
        match self {
            Self::Fd => "v",
            Self::Rg => "",
        }
    }

    fn system_binary_names(self) -> &'static [&'static str] {
        match self {
            Self::Fd => &["fd", "fdfind"],
            Self::Rg => &["rg"],
        }
    }

    fn termux_package(self) -> &'static str {
        match self {
            Self::Fd => "fd",
            Self::Rg => "ripgrep",
        }
    }

    fn asset_name(self, version: &str, plat: &str, architecture: &str) -> Option<String> {
        match (self, plat) {
            (Self::Fd, "darwin") => {
                let arch = if architecture == "arm64" {
                    "aarch64"
                } else {
                    "x86_64"
                };
                Some(format!("fd-v{version}-{arch}-apple-darwin.tar.gz"))
            }
            (Self::Fd, "linux") => {
                let arch = if architecture == "arm64" {
                    "aarch64"
                } else {
                    "x86_64"
                };
                Some(format!("fd-v{version}-{arch}-unknown-linux-gnu.tar.gz"))
            }
            (Self::Fd, "win32") => {
                let arch = if architecture == "arm64" {
                    "aarch64"
                } else {
                    "x86_64"
                };
                Some(format!("fd-v{version}-{arch}-pc-windows-msvc.zip"))
            }
            (Self::Rg, "darwin") => {
                let arch = if architecture == "arm64" {
                    "aarch64"
                } else {
                    "x86_64"
                };
                Some(format!("ripgrep-{version}-{arch}-apple-darwin.tar.gz"))
            }
            (Self::Rg, "linux") if architecture == "arm64" => Some(format!(
                "ripgrep-{version}-aarch64-unknown-linux-gnu.tar.gz"
            )),
            (Self::Rg, "linux") => Some(format!(
                "ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz"
            )),
            (Self::Rg, "win32") => {
                let arch = if architecture == "arm64" {
                    "aarch64"
                } else {
                    "x86_64"
                };
                Some(format!("ripgrep-{version}-{arch}-pc-windows-msvc.zip"))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStatus {
    pub kind: ToolStatusKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatusKind {
    Info,
    Warning,
}

/// TS `getBinDir()` — `{agentDir}/bin`.
pub fn tools_bin_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join("bin")
}

pub fn is_offline_mode_enabled() -> bool {
    match std::env::var("PI_OFFLINE") {
        Ok(value) => {
            let lower = value.to_ascii_lowercase();
            value == "1" || lower == "true" || lower == "yes"
        }
        Err(_) => false,
    }
}

fn current_platform() -> (&'static str, String) {
    let plat = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "linux"
    };
    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64".into()
    } else if cfg!(target_arch = "x86_64") {
        "x64".into()
    } else {
        std::env::consts::ARCH.to_string()
    };
    (plat, architecture)
}

fn binary_file_name(tool: ManagedTool) -> String {
    if cfg!(windows) {
        format!("{}.exe", tool.binary_name())
    } else {
        tool.binary_name().to_string()
    }
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// TS `getToolPath`.
pub fn get_tool_path(tool: ManagedTool) -> Option<String> {
    get_tool_path_in(&tools_bin_dir(&default_agent_dir()), tool)
}

pub fn get_tool_path_in(bin_dir: &Path, tool: ManagedTool) -> Option<String> {
    let local = bin_dir.join(binary_file_name(tool));
    if local.is_file() {
        return Some(local.to_string_lossy().into_owned());
    }
    for name in tool.system_binary_names() {
        if command_exists(name) {
            return Some((*name).to_string());
        }
    }
    None
}

pub fn path_with_tools_bin(current: Option<&str>, bin_dir: &Path) -> String {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let bin = bin_dir.to_string_lossy();
    match current {
        Some(path) if !path.is_empty() => {
            if path.split(sep).any(|entry| entry == bin.as_ref()) {
                path.to_string()
            } else {
                format!("{bin}{sep}{path}")
            }
        }
        _ => bin.into_owned(),
    }
}

/// Prepend `{agentDir}/bin` to PATH like TS `getShellEnv`.
pub fn prepend_tools_bin_to_path() {
    let bin_dir = tools_bin_dir(&default_agent_dir());
    let path_key = std::env::vars()
        .find(|(key, _)| key.eq_ignore_ascii_case("path"))
        .map(|(key, _)| key)
        .unwrap_or_else(|| "PATH".into());
    let current = std::env::var(&path_key).ok();
    std::env::set_var(&path_key, path_with_tools_bin(current.as_deref(), &bin_dir));
}

fn fixture_reply(tool: ManagedTool) -> Option<String> {
    let key = match tool {
        ManagedTool::Fd => "PI_ENSURE_TOOL_FD_REPLY",
        ManagedTool::Rg => "PI_ENSURE_TOOL_RG_REPLY",
    };
    std::env::var(key)
        .ok()
        .or_else(|| std::env::var("PI_ENSURE_TOOL_REPLY").ok())
}

fn copy_fixture_binary(src: &Path, dest: &Path) -> Result<String, String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::copy(src, dest).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)
            .map_err(|err| err.to_string())?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dest, perms).map_err(|err| err.to_string())?;
    }
    Ok(dest.to_string_lossy().into_owned())
}

fn latest_version(repo: &str) -> Result<String, String> {
    if let Ok(raw) = std::env::var("PI_GITHUB_RELEASE_REPLY") {
        let value: serde_json::Value = if Path::new(&raw).exists() {
            let body = fs::read_to_string(&raw).map_err(|err| err.to_string())?;
            serde_json::from_str(&body).map_err(|err| err.to_string())?
        } else {
            serde_json::from_str(&raw).map_err(|err| err.to_string())?
        };
        let tag = value
            .get("tag_name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "GitHub API error: missing tag_name".to_string())?;
        return Ok(tag.trim_start_matches('v').to_string());
    }
    if cfg!(test) && std::env::var("PI_ALLOW_TOOL_DOWNLOAD").is_err() {
        return Err(
            "refusing live HTTP (set PI_GITHUB_RELEASE_REPLY or PI_ALLOW_TOOL_DOWNLOAD)".into(),
        );
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(NETWORK_TIMEOUT_MS))
        .build();
    let response = agent
        .get(&url)
        .set("User-Agent", &format!("{APP_NAME}-coding-agent"))
        .call()
        .map_err(|err| format!("GitHub API error: {err}"))?;
    if response.status() >= 400 {
        return Err(format!("GitHub API error: {}", response.status()));
    }
    let value: serde_json::Value = response
        .into_json()
        .map_err(|err| format!("GitHub API error: {err}"))?;
    let tag = value
        .get("tag_name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "GitHub API error: missing tag_name".to_string())?;
    Ok(tag.trim_start_matches('v').to_string())
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if let Ok(reply) = std::env::var("PI_TOOL_DOWNLOAD_REPLY") {
        let src = PathBuf::from(reply);
        if src.is_file() {
            fs::copy(&src, dest).map_err(|err| err.to_string())?;
            return Ok(());
        }
        return Err(format!("Failed to download: fixture missing {src:?}"));
    }
    if cfg!(test) && std::env::var("PI_ALLOW_TOOL_DOWNLOAD").is_err() {
        return Err(
            "refusing live HTTP (set PI_TOOL_DOWNLOAD_REPLY or PI_ALLOW_TOOL_DOWNLOAD)".into(),
        );
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|err| format!("Failed to download: {err}"))?;
    if response.status() >= 400 {
        return Err(format!("Failed to download: {}", response.status()));
    }
    let mut reader = response.into_reader();
    let mut file = fs::File::create(dest).map_err(|err| err.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|err| err.to_string())?;
    Ok(())
}

fn find_binary_recursively(root: &Path, binary_file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && entry.file_name() == binary_file_name {
                return Some(path);
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

fn run_extraction(command: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|err| format!("{command}: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    };
    Err(format!("{command}: {detail}"))
}

fn extract_archive(
    archive_path: &Path,
    extract_dir: &Path,
    asset_name: &str,
) -> Result<(), String> {
    if asset_name.ends_with(".tar.gz") {
        run_extraction(
            "tar",
            &[
                "xzf",
                &archive_path.to_string_lossy(),
                "-C",
                &extract_dir.to_string_lossy(),
            ],
        )
        .map_err(|err| format!("Failed to extract {asset_name}: {err}"))
    } else if asset_name.ends_with(".zip") {
        if cfg!(windows) {
            run_extraction(
                "tar",
                &[
                    "xf",
                    &archive_path.to_string_lossy(),
                    "-C",
                    &extract_dir.to_string_lossy(),
                ],
            )
            .map_err(|err| format!("Failed to extract {asset_name}: {err}"))
        } else {
            run_extraction(
                "unzip",
                &[
                    "-q",
                    &archive_path.to_string_lossy(),
                    "-d",
                    &extract_dir.to_string_lossy(),
                ],
            )
            .or_else(|unzip_err| {
                run_extraction(
                    "tar",
                    &[
                        "xf",
                        &archive_path.to_string_lossy(),
                        "-C",
                        &extract_dir.to_string_lossy(),
                    ],
                )
                .map_err(|tar_err| {
                    format!("Failed to extract {asset_name}: {unzip_err}; {tar_err}")
                })
            })
        }
    } else {
        Err(format!("Unsupported archive format: {asset_name}"))
    }
}

fn download_tool(tool: ManagedTool, bin_dir: &Path) -> Result<String, String> {
    let (plat, architecture) = current_platform();
    let mut version = latest_version(tool.repo())?;
    if tool == ManagedTool::Fd && plat == "darwin" && architecture == "x64" {
        version = "10.3.0".into();
    }
    let asset_name = tool
        .asset_name(&version, plat, &architecture)
        .ok_or_else(|| format!("Unsupported platform: {plat}/{architecture}"))?;
    fs::create_dir_all(bin_dir).map_err(|err| err.to_string())?;
    let download_url = format!(
        "https://github.com/{}/releases/download/{}{}/{}",
        tool.repo(),
        tool.tag_prefix(),
        version,
        asset_name
    );
    let archive_path = bin_dir.join(&asset_name);
    let binary_path = bin_dir.join(binary_file_name(tool));
    download_file(&download_url, &archive_path)?;
    let extract_dir = bin_dir.join(format!(
        "extract_tmp_{}_{}_{}",
        tool.binary_name(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&extract_dir).map_err(|err| err.to_string())?;
    let result = (|| {
        extract_archive(&archive_path, &extract_dir, &asset_name)?;
        let binary_file_name = binary_file_name(tool);
        let stem = asset_name
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".zip");
        let candidates = [
            extract_dir.join(stem).join(&binary_file_name),
            extract_dir.join(&binary_file_name),
        ];
        let extracted = candidates
            .iter()
            .find(|path| path.is_file())
            .cloned()
            .or_else(|| find_binary_recursively(&extract_dir, &binary_file_name))
            .ok_or_else(|| {
                format!("Binary not found in archive: expected {binary_file_name} under {extract_dir:?}")
            })?;
        fs::rename(&extracted, &binary_path).map_err(|err| err.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&binary_path)
                .map_err(|err| err.to_string())?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&binary_path, perms).map_err(|err| err.to_string())?;
        }
        Ok(binary_path.to_string_lossy().into_owned())
    })();
    let _ = fs::remove_file(&archive_path);
    let _ = fs::remove_dir_all(&extract_dir);
    result
}

/// TS `ensureTool`.
pub fn ensure_tool(tool: ManagedTool) -> (Option<String>, Vec<ToolStatus>) {
    ensure_tool_in(&tools_bin_dir(&default_agent_dir()), tool)
}

pub fn ensure_tool_in(bin_dir: &Path, tool: ManagedTool) -> (Option<String>, Vec<ToolStatus>) {
    let mut statuses = Vec::new();
    if let Some(existing) = get_tool_path_in(bin_dir, tool) {
        return (Some(existing), statuses);
    }
    if let Some(reply) = fixture_reply(tool) {
        if reply == "offline" || reply.eq_ignore_ascii_case("skip") {
            statuses.push(ToolStatus {
                kind: ToolStatusKind::Warning,
                message: format!(
                    "{} not found. Offline mode enabled, skipping download.",
                    tool.name()
                ),
            });
            return (None, statuses);
        }
        if let Some(message) = reply.strip_prefix("error:") {
            statuses.push(ToolStatus {
                kind: ToolStatusKind::Warning,
                message: format!("Failed to download {}: {message}", tool.name()),
            });
            return (None, statuses);
        }
        let src = PathBuf::from(&reply);
        if src.is_file() {
            let dest = bin_dir.join(binary_file_name(tool));
            statuses.push(ToolStatus {
                kind: ToolStatusKind::Info,
                message: format!("{} not found. Downloading...", tool.name()),
            });
            match copy_fixture_binary(&src, &dest) {
                Ok(path) => {
                    statuses.push(ToolStatus {
                        kind: ToolStatusKind::Info,
                        message: format!("{} installed to {path}", tool.name()),
                    });
                    return (Some(path), statuses);
                }
                Err(err) => {
                    statuses.push(ToolStatus {
                        kind: ToolStatusKind::Warning,
                        message: format!("Failed to download {}: {err}", tool.name()),
                    });
                    return (None, statuses);
                }
            }
        }
    }
    if is_offline_mode_enabled() {
        statuses.push(ToolStatus {
            kind: ToolStatusKind::Warning,
            message: format!(
                "{} not found. Offline mode enabled, skipping download.",
                tool.name()
            ),
        });
        return (None, statuses);
    }
    if cfg!(target_os = "android") {
        statuses.push(ToolStatus {
            kind: ToolStatusKind::Warning,
            message: format!(
                "{} not found. Install with: pkg install {}",
                tool.name(),
                tool.termux_package()
            ),
        });
        return (None, statuses);
    }
    statuses.push(ToolStatus {
        kind: ToolStatusKind::Info,
        message: format!("{} not found. Downloading...", tool.name()),
    });
    match download_tool(tool, bin_dir) {
        Ok(path) => {
            statuses.push(ToolStatus {
                kind: ToolStatusKind::Info,
                message: format!("{} installed to {path}", tool.name()),
            });
            (Some(path), statuses)
        }
        Err(err) => {
            statuses.push(ToolStatus {
                kind: ToolStatusKind::Warning,
                message: format!("Failed to download {}: {err}", tool.name()),
            });
            (None, statuses)
        }
    }
}

pub fn ensure_managed_tools() -> Vec<ToolStatus> {
    let (fd_path, mut statuses) = ensure_tool(ManagedTool::Fd);
    let (_rg_path, rg_statuses) = ensure_tool(ManagedTool::Rg);
    statuses.extend(rg_statuses);
    if let Some(path) = fd_path {
        if path.contains('/') || path.contains('\\') {
            std::env::set_var("PI_FD_PATH", path);
        }
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    #[test]
    fn asset_names_match_ts_get_asset_name() {
        assert_eq!(
            ManagedTool::Fd
                .asset_name("10.2.0", "linux", "x64")
                .as_deref(),
            Some("fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz")
        );
        assert_eq!(
            ManagedTool::Fd
                .asset_name("10.2.0", "darwin", "arm64")
                .as_deref(),
            Some("fd-v10.2.0-aarch64-apple-darwin.tar.gz")
        );
        assert_eq!(
            ManagedTool::Rg
                .asset_name("14.1.1", "linux", "x64")
                .as_deref(),
            Some("ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz")
        );
        assert_eq!(
            ManagedTool::Rg
                .asset_name("14.1.1", "linux", "arm64")
                .as_deref(),
            Some("ripgrep-14.1.1-aarch64-unknown-linux-gnu.tar.gz")
        );
        assert_eq!(ManagedTool::Fd.asset_name("1.0.0", "freebsd", "x64"), None);
    }

    #[test]
    fn path_prepend_matches_ts_get_shell_env() {
        let bin = PathBuf::from("/tmp/agent/bin");
        assert_eq!(path_with_tools_bin(None, &bin), "/tmp/agent/bin");
        assert_eq!(
            path_with_tools_bin(Some("/usr/bin"), &bin),
            "/tmp/agent/bin:/usr/bin"
        );
        assert_eq!(
            path_with_tools_bin(Some("/tmp/agent/bin:/usr/bin"), &bin),
            "/tmp/agent/bin:/usr/bin"
        );
    }

    fn isolate_path() -> Option<String> {
        let previous = std::env::var("PATH").ok();
        std::env::set_var("PATH", "");
        previous
    }

    fn restore_path(previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }

    #[test]
    fn offline_ensure_reports_ts_warning_without_download() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        let previous_path = isolate_path();
        std::env::set_var("PI_OFFLINE", "1");
        std::env::remove_var("PI_ENSURE_TOOL_REPLY");
        std::env::remove_var("PI_ENSURE_TOOL_FD_REPLY");
        let (result, statuses) = ensure_tool_in(dir.path(), ManagedTool::Fd);
        std::env::remove_var("PI_OFFLINE");
        restore_path(previous_path);
        assert!(result.is_none());
        assert_eq!(
            statuses,
            vec![ToolStatus {
                kind: ToolStatusKind::Warning,
                message: "fd not found. Offline mode enabled, skipping download.".into(),
            }]
        );
    }

    #[test]
    fn fixture_installs_binary_into_bin_dir() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        let src = dir.path().join("fake-fd");
        fs::write(&src, "#!/bin/sh\necho fd\n").unwrap();
        let previous_path = isolate_path();
        std::env::remove_var("PI_OFFLINE");
        std::env::set_var("PI_ENSURE_TOOL_FD_REPLY", src.to_string_lossy().as_ref());
        let (installed, statuses) = ensure_tool_in(dir.path(), ManagedTool::Fd);
        std::env::remove_var("PI_ENSURE_TOOL_FD_REPLY");
        restore_path(previous_path);
        let dest = dir.path().join("fd");
        assert_eq!(installed.as_deref(), Some(dest.to_string_lossy().as_ref()));
        assert!(dest.is_file());
        assert_eq!(
            statuses[0],
            ToolStatus {
                kind: ToolStatusKind::Info,
                message: "fd not found. Downloading...".into(),
            }
        );
        assert!(statuses[1].message.starts_with("fd installed to "));
    }

    #[test]
    fn prefers_existing_local_binary() {
        let dir = tempdir().unwrap();
        let local = dir.path().join("fd");
        fs::write(&local, "fd").unwrap();
        assert_eq!(
            get_tool_path_in(dir.path(), ManagedTool::Fd).as_deref(),
            Some(local.to_string_lossy().as_ref())
        );
    }
}
