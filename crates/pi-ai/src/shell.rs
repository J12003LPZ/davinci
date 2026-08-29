//! TS `getShellConfig` + `resolveConfigValue` command execution.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_millis(10_000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub shell: String,
    pub args: Vec<String>,
    /// TS WSL `bash.exe` uses `-s` and sends the command on stdin.
    pub command_transport: CommandTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTransport {
    Argv,
    Stdin,
}

#[derive(Debug, Clone)]
pub struct ResolveCommandOptions {
    pub shell_path: Option<String>,
    pub timeout: Duration,
    pub allow_command: bool,
}

impl Default for ResolveCommandOptions {
    fn default() -> Self {
        Self {
            shell_path: None,
            timeout: command_timeout_from_env(),
            allow_command: !cfg!(test),
        }
    }
}

pub fn command_timeout_from_env() -> Duration {
    std::env::var("PI_CONFIG_COMMAND_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
}

/// TS `isLegacyWslBashPath`: `X:\Windows\System32\bash.exe` or Sysnative.
pub fn is_legacy_wsl_bash_path(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    let bytes = normalized.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (normalized.ends_with("\\windows\\system32\\bash.exe")
            || normalized.ends_with("\\windows\\sysnative\\bash.exe"))
}

pub fn resolve_shell_config(custom_shell_path: Option<&str>) -> Result<ShellConfig, String> {
    if let Some(path) = custom_shell_path {
        let expanded = expand_shell_home(path);
        if Path::new(&expanded).exists() {
            return Ok(bash_shell_config(&expanded));
        }
        return Err(format!("Custom shell path not found: {path}"));
    }
    if cfg!(windows) {
        return windows_shell_config();
    }
    if Path::new("/bin/bash").exists() {
        return Ok(bash_shell_config("/bin/bash"));
    }
    if let Some(bash) = find_executable("bash") {
        return Ok(bash_shell_config(&bash));
    }
    Ok(ShellConfig {
        shell: "sh".into(),
        args: vec!["-c".into()],
        command_transport: CommandTransport::Argv,
    })
}

fn bash_shell_config(shell: &str) -> ShellConfig {
    if is_legacy_wsl_bash_path(shell) {
        ShellConfig {
            shell: shell.into(),
            args: vec!["-s".into()],
            command_transport: CommandTransport::Stdin,
        }
    } else {
        ShellConfig {
            shell: shell.into(),
            args: vec!["-c".into()],
            command_transport: CommandTransport::Argv,
        }
    }
}

fn windows_shell_config() -> Result<ShellConfig, String> {
    let mut searched = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(key) {
            let path = PathBuf::from(root).join("Git").join("bin").join("bash.exe");
            searched.push(path.display().to_string());
            if path.exists() {
                return Ok(bash_shell_config(&path.display().to_string()));
            }
        }
    }
    if let Some(bash) = find_executable("bash.exe") {
        return Ok(bash_shell_config(&bash));
    }
    Err(format!(
        "No bash shell found. Options:\n  1. Install Git for Windows: https://git-scm.com/download/win\n  2. Add your bash to PATH (Cygwin, MSYS2, etc.)\n  3. Set shellPath in settings.json\n\nSearched Git Bash in:\n{}",
        searched
            .iter()
            .map(|path| format!("  {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn expand_shell_home(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| path.into());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

fn find_executable(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
        if cfg!(windows) {
            let with_exe = dir.join(format!("{name}.exe"));
            if with_exe.is_file() {
                return Some(with_exe.display().to_string());
            }
        }
    }
    None
}

/// TS `executeCommandUncached`: Unix `/bin/sh -c`; Windows bash then `cmd.exe /d /s /c`.
pub fn execute_config_command(command: &str, options: &ResolveCommandOptions) -> Option<String> {
    if !options.allow_command {
        return None;
    }
    if cfg!(windows) {
        if let Ok(config) = resolve_shell_config(options.shell_path.as_deref()) {
            if let ShellRun::Executed(value) = run_shell_command(&config, command, options.timeout)
            {
                return value;
            }
        }
        return run_windows_cmd(command, options.timeout);
    }
    run_unix_default_shell(command, options.timeout)
}

enum ShellRun {
    Executed(Option<String>),
    NotExecuted,
}

fn run_unix_default_shell(command: &str, timeout: Duration) -> Option<String> {
    match run_shell_command(
        &ShellConfig {
            shell: "sh".into(),
            args: vec!["-c".into()],
            command_transport: CommandTransport::Argv,
        },
        command,
        timeout,
    ) {
        ShellRun::Executed(value) => value,
        ShellRun::NotExecuted => None,
    }
}

fn run_windows_cmd(command: &str, timeout: Duration) -> Option<String> {
    let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    match run_shell_command(
        &ShellConfig {
            shell: comspec,
            args: vec!["/d".into(), "/s".into(), "/c".into()],
            command_transport: CommandTransport::Argv,
        },
        command,
        timeout,
    ) {
        ShellRun::Executed(value) => value,
        ShellRun::NotExecuted => None,
    }
}

fn run_shell_command(config: &ShellConfig, command: &str, timeout: Duration) -> ShellRun {
    let mut child = Command::new(&config.shell);
    child
        .args(&config.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match config.command_transport {
        CommandTransport::Argv => {
            child.arg(command).stdin(Stdio::null());
        }
        CommandTransport::Stdin => {
            child.stdin(Stdio::piped());
        }
    }
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ShellRun::NotExecuted,
        Err(_) => return ShellRun::Executed(None),
    };
    if config.command_transport == CommandTransport::Stdin {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(command.as_bytes());
        }
    }
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = std::io::Read::read_to_end(&mut pipe, &mut stdout);
                }
                let _ = child.wait();
                if !status.success() {
                    return ShellRun::Executed(None);
                }
                let text = String::from_utf8_lossy(&stdout).trim().to_string();
                return ShellRun::Executed(if text.is_empty() { None } else { Some(text) });
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return ShellRun::Executed(None);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return ShellRun::Executed(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_bash_uses_stdin_transport() {
        assert!(is_legacy_wsl_bash_path(r"C:\Windows\System32\bash.exe"));
        assert!(is_legacy_wsl_bash_path("c:/windows/sysnative/bash.exe"));
        assert!(!is_legacy_wsl_bash_path(
            r"C:\Program Files\Git\bin\bash.exe"
        ));
        let config = bash_shell_config(r"C:\Windows\System32\bash.exe");
        assert_eq!(config.args, vec!["-s".to_string()]);
        assert_eq!(config.command_transport, CommandTransport::Stdin);
    }

    #[test]
    fn unix_default_prefers_bash_then_sh() {
        if cfg!(windows) {
            return;
        }
        let config = resolve_shell_config(None).unwrap();
        if Path::new("/bin/bash").exists() {
            assert_eq!(config.shell, "/bin/bash");
            assert_eq!(config.args, vec!["-c".to_string()]);
        } else {
            assert!(config.shell == "sh" || config.shell.ends_with("bash"));
        }
    }

    #[test]
    fn custom_missing_shell_errors() {
        let error = resolve_shell_config(Some("/definitely/missing/pi-shell")).unwrap_err();
        assert!(error.contains("Custom shell path not found"));
    }

    #[test]
    fn command_timeout_kills_long_process() {
        if cfg!(windows) {
            return;
        }
        let started = Instant::now();
        let value = execute_config_command(
            "sleep 5",
            &ResolveCommandOptions {
                shell_path: None,
                timeout: Duration::from_millis(200),
                allow_command: true,
            },
        );
        assert!(value.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn command_stdout_is_trimmed() {
        if cfg!(windows) {
            return;
        }
        let value = execute_config_command(
            "printf '  spaced-key  \\n'",
            &ResolveCommandOptions {
                shell_path: None,
                timeout: Duration::from_secs(2),
                allow_command: true,
            },
        );
        assert_eq!(value.as_deref(), Some("spaced-key"));
    }
}
