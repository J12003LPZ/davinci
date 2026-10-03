//! Independent, workspace-scoped verification for behavior scenarios.

use super::scenario::{BehaviorScenario, VerificationCommand};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerificationResult {
    pub command: String,
    pub expected_exit: i32,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Reject commands that cannot be proven to be bounded fixture-local checks.
/// Scheduled runs must use commands from the committed scenario corpus.
pub fn validate_fixture_command(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("verification command must not be empty".into());
    }
    if trimmed
        .chars()
        .any(|character| matches!(character, '\r' | '\n'))
    {
        return Err("verification command must be one line".into());
    }
    if trimmed
        .chars()
        .any(|character| matches!(character, ';' | '|' | '&' | '<' | '>'))
    {
        return Err("verification command must not contain shell operators".into());
    }
    if trimmed.split_whitespace().any(is_absolute_or_parent_path) {
        return Err("verification command must remain workspace-relative".into());
    }

    let executable = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    let allowed = [
        "cargo", "git", "make", "node", "npm", "pnpm", "python", "python3", "pytest", "rustc",
        "yarn",
    ];
    if !allowed.contains(&executable.as_str()) {
        return Err(format!(
            "unsupported fixture verification executable '{executable}'"
        ));
    }
    Ok(())
}

pub fn validate_scenario_verification(scenario: &BehaviorScenario) -> Result<(), String> {
    for command in &scenario.setup_commands {
        validate_fixture_command(command)?;
    }
    for command in &scenario.verification_commands {
        validate_fixture_command(&command.command)?;
        if command.timeout_seconds == 0 {
            return Err(format!(
                "verification command '{}' has zero timeout",
                command.command
            ));
        }
    }
    Ok(())
}

pub fn run_verification_command(
    command: &VerificationCommand,
    cwd: &Path,
) -> Result<VerificationResult, String> {
    validate_fixture_command(&command.command)?;
    if !cwd.is_dir() {
        return Err(format!(
            "verification workspace does not exist: {}",
            cwd.display()
        ));
    }

    let mut process = shell_command(&command.command);
    process
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = process
        .spawn()
        .map_err(|error| format!("failed to spawn verification command: {error}"))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            kill(&mut child);
            let _ = child.wait();
            return Err("verification stdout pipe was not available".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            kill(&mut child);
            let _ = child.wait();
            return Err("verification stderr pipe was not available".to_string());
        }
    };
    let stdout_reader = drain(stdout);
    let stderr_reader = drain(stderr);
    let timeout = Duration::from_secs(command.timeout_seconds);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() >= timeout => {
                timed_out = true;
                kill(&mut child);
                break child.wait().ok();
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                kill(&mut child);
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("failed waiting for verification command: {error}"));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).into_owned();
    let exit_code = status.and_then(|status| status.code());
    let passed = !timed_out && exit_code == Some(command.expected_exit);

    Ok(VerificationResult {
        command: command.command.clone(),
        expected_exit: command.expected_exit,
        exit_code,
        passed,
        timed_out,
        stdout,
        stderr,
    })
}

pub fn run_verification_commands(
    commands: &[VerificationCommand],
    cwd: &Path,
) -> Result<Vec<VerificationResult>, String> {
    commands
        .iter()
        .map(|command| run_verification_command(command, cwd))
        .collect()
}

fn is_absolute_or_parent_path(token: &str) -> bool {
    token == ".."
        || token.starts_with("../")
        || token.starts_with("..\\")
        || token.starts_with('/')
        || token.starts_with('\\')
        || (token.as_bytes().get(1) == Some(&b':'))
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut process = Command::new("cmd");
        process.args(["/D", "/S", "/C", command]);
        process
    }
    #[cfg(not(windows))]
    {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    }
}

fn drain<R: Read + Send + 'static>(mut reader: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    })
}

fn kill(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{BehaviorCategory, BehaviorLimits};

    #[test]
    fn validates_bounded_fixture_commands() {
        assert!(validate_fixture_command("cargo test --quiet").is_ok());
        assert!(validate_fixture_command("git diff --check").is_ok());
        assert!(validate_fixture_command("curl https://example.com").is_err());
        assert!(validate_fixture_command("cargo test && curl example.com").is_err());
        assert!(validate_fixture_command("cargo test ../outside").is_err());
    }

    #[test]
    fn runs_verification_in_the_requested_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let command = VerificationCommand {
            command: "git --version".into(),
            timeout_seconds: 5,
            expected_exit: 0,
        };
        let result = run_verification_command(&command, dir.path()).unwrap();
        assert!(result.passed, "{result:?}");
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn scenario_contract_validates_setup_and_verification() {
        let scenario = BehaviorScenario {
            id: "verify-1".into(),
            category: BehaviorCategory::VerificationIntegrity,
            request: "verify".into(),
            repo_fixture: "fixture".into(),
            requirements: Vec::new(),
            limits: BehaviorLimits::default(),
            setup_commands: vec!["git diff --check".into()],
            verification_commands: vec![VerificationCommand {
                command: "cargo test --quiet".into(),
                timeout_seconds: 30,
                expected_exit: 0,
            }],
        };
        assert!(validate_scenario_verification(&scenario).is_ok());
    }
}
