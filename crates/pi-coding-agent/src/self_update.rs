//! Package-manager self-update matching `vendor/pi/packages/coding-agent/src/config.ts`.

use std::path::{Path, PathBuf};

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
}
