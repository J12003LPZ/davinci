//! Process isolation for real DaVinci JSON-mode evaluation turns.

use davinci_agent::{prompt::PromptModelPolicy, PromptProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DavinciProcessConfig {
    pub binary: PathBuf,
    /// Optional arguments for a trusted launcher before DaVinci's arguments.
    /// This remains shell-free so wrapper execution cannot reinterpret values.
    pub launcher_args: Vec<String>,
    pub provider: String,
    pub model: String,
    pub prompt_profile: PromptProfile,
    pub prompt_model_policy_override: Option<PromptModelPolicy>,
    pub permission_mode: String,
    pub timeout: Duration,
    pub clean_agent_dir: PathBuf,
    /// Source auth store used to provision only the selected provider into the
    /// ephemeral agent directory. The source file is never passed to Davinci.
    pub auth_source: Option<PathBuf>,
    pub allowed_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DavinciProcessRun {
    pub exit_code: i32,
    pub stdout_lines: Vec<String>,
    pub stderr: String,
    pub wall_ms: u64,
    pub timed_out: bool,
}

impl DavinciProcessRun {
    pub fn bounded_diagnostic(&self) -> String {
        const LIMIT: usize = 2_048;
        let combined = format!("{}\n{}", self.stdout_lines.join("\n"), self.stderr);
        let redacted = regex::Regex::new(r#"(?i)(authorization|api[_-]?key)\s*[:=]\s*[^\s,"}]+"#)
            .expect("static credential pattern")
            .replace_all(&combined, "$1=[REDACTED]");
        redacted
            .chars()
            .rev()
            .take(LIMIT)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .trim()
            .to_string()
    }
}

fn drain_pipe<R: Read + Send + 'static>(mut pipe: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

fn kill_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn join_pipe(handle: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

/// Execute one real product turn with a clean agent directory and a strict
/// environment allow-list. The inherited current directory is intentionally
/// preserved for the fixture executor; callers should invoke this function
/// while scoped to the copied scenario workspace.
pub fn run_davinci_process(
    config: &DavinciProcessConfig,
    request: &str,
) -> Result<DavinciProcessRun, String> {
    fs::create_dir_all(&config.clean_agent_dir)
        .map_err(|error| format!("failed to create clean agent directory: {error}"))?;
    if let Some(source_path) = &config.auth_source {
        let source = davinci_ai::AuthStorage::open(source_path)
            .map_err(|error| format!("failed to read evaluator auth source: {error}"))?;
        if let Some(credential) = source.get(&config.provider).cloned() {
            let target_path = config.clean_agent_dir.join("auth.json");
            let mut target = davinci_ai::AuthStorage::open(&target_path)
                .map_err(|error| format!("failed to prepare evaluator auth store: {error}"))?;
            target
                .set(&config.provider, credential)
                .map_err(|error| format!("failed to provision evaluator credential: {error}"))?;
        }
    }

    let mut command = Command::new(&config.binary);
    command
        .args(&config.launcher_args)
        .arg("--mode")
        .arg("json")
        .arg("--prompt-profile")
        .arg(config.prompt_profile.id())
        .arg("--permission-mode")
        .arg(&config.permission_mode)
        .arg("--provider")
        .arg(&config.provider)
        .arg("--model")
        .arg(&config.model)
        .arg("-p")
        .arg(request)
        .env_clear()
        .env("DAVINCI_CODING_AGENT_DIR", &config.clean_agent_dir)
        .env("PI_CODING_AGENT_DIR", &config.clean_agent_dir)
        .env(
            "DAVINCI_CODING_AGENT_SESSION_DIR",
            config.clean_agent_dir.join("sessions"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(policy) = config.prompt_model_policy_override {
        command
            .env("DAVINCI_BEHAVIOR_EVAL", "1")
            .env("DAVINCI_EVAL_PROMPT_MODEL_POLICY", policy.id());
    }
    for (name, value) in &config.allowed_env {
        command.env(name, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let start = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {}: {error}", config.binary.display()))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            return Err("DaVinci stdout pipe was not available".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            return Err("DaVinci stderr pipe was not available".to_string());
        }
    };
    let stdout_reader = drain_pipe(stdout);
    let stderr_reader = drain_pipe(stderr);

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() >= config.timeout => {
                timed_out = true;
                kill_process_tree(&mut child);
                break child.wait().ok();
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                let _ = join_pipe(stdout_reader);
                let _ = join_pipe(stderr_reader);
                return Err(format!("failed waiting for DaVinci: {error}"));
            }
        }
    };

    let stdout = join_pipe(stdout_reader);
    let stderr = join_pipe(stderr_reader);
    let stdout_text = String::from_utf8_lossy(&stdout);
    let stdout_lines = stdout_text.lines().map(str::to_string).collect();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    Ok(DavinciProcessRun {
        exit_code: if timed_out {
            -1
        } else {
            status.and_then(|status| status.code()).unwrap_or(-1)
        },
        stdout_lines,
        stderr,
        wall_ms: start.elapsed().as_millis() as u64,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::{AuthStorage, Credential, CredentialKind};

    fn api_key_credential(value: &str) -> Credential {
        Credential {
            kind: CredentialKind::ApiKey,
            key: Some(value.into()),
            access: None,
            refresh: None,
            expires: None,
            env: BTreeMap::new().into_iter().collect(),
            available_model_ids: Vec::new(),
        }
    }

    #[test]
    fn captures_json_lines_and_allowlisted_environment() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_davinci.py");
        fs::write(
            &script,
            "import os\nimport sys\nprint('{\"type\":\"message\",\"role\":\"assistant\"}')\nprint(os.environ.get('DAVINCI_EVAL_ALLOWED', 'missing'), file=sys.stderr)\n",
        )
        .unwrap();
        let mut allowed = BTreeMap::new();
        allowed.insert("DAVINCI_EVAL_ALLOWED".into(), "visible".into());
        let config = DavinciProcessConfig {
            binary: "python".into(),
            launcher_args: vec![script.to_string_lossy().into_owned()],
            provider: "fixture".into(),
            model: "fixture-model".into(),
            prompt_profile: PromptProfile::Stable,
            prompt_model_policy_override: None,
            permission_mode: "read-only".into(),
            timeout: Duration::from_secs(5),
            clean_agent_dir: dir.path().join("agent"),
            auth_source: None,
            allowed_env: allowed,
        };
        let run = run_davinci_process(&config, "hello").unwrap();
        assert!(!run.timed_out);
        assert_eq!(run.exit_code, 0);
        assert_eq!(run.stdout_lines.len(), 1);
        assert!(run.stderr.contains("visible"));
        assert!(dir.path().join("agent").is_dir());
    }

    #[test]
    fn model_policy_override_is_forwarded_only_as_guarded_eval_environment() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_davinci.py");
        fs::write(
            &script,
            r#"import os
import sys
print('{"type":"agent_end"}')
print(os.environ.get('DAVINCI_BEHAVIOR_EVAL', 'missing'), file=sys.stderr)
print(os.environ.get('DAVINCI_EVAL_PROMPT_MODEL_POLICY', 'missing'), file=sys.stderr)
"#,
        )
        .unwrap();
        let config = DavinciProcessConfig {
            binary: "python".into(),
            launcher_args: vec![script.to_string_lossy().into_owned()],
            provider: "openai-codex".into(),
            model: "gpt-6-astra".into(),
            prompt_profile: PromptProfile::Stable,
            prompt_model_policy_override: Some(PromptModelPolicy::Default),
            permission_mode: "read-only".into(),
            timeout: Duration::from_secs(5),
            clean_agent_dir: dir.path().join("agent"),
            auth_source: None,
            allowed_env: BTreeMap::new(),
        };
        let run = run_davinci_process(&config, "hello").unwrap();
        assert_eq!(run.exit_code, 0);
        assert!(run.stderr.contains("1"));
        assert!(run.stderr.contains("default"));
    }

    #[test]
    fn provisions_only_the_selected_provider_credential() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source-auth.json");
        let mut source = AuthStorage::open(&source_path).unwrap();
        source
            .set("selected", api_key_credential("selected-secret"))
            .unwrap();
        source
            .set("unselected", api_key_credential("other-secret"))
            .unwrap();

        let script = dir.path().join("fake_davinci.py");
        fs::write(&script, "print('{\"type\":\"agent_end\"}')\n").unwrap();
        let clean_agent_dir = dir.path().join("agent");
        let config = DavinciProcessConfig {
            binary: "python".into(),
            launcher_args: vec![script.to_string_lossy().into_owned()],
            provider: "selected".into(),
            model: "fixture-model".into(),
            prompt_profile: PromptProfile::Stable,
            prompt_model_policy_override: None,
            permission_mode: "read-only".into(),
            timeout: Duration::from_secs(5),
            clean_agent_dir: clean_agent_dir.clone(),
            auth_source: Some(source_path),
            allowed_env: BTreeMap::new(),
        };

        let run = run_davinci_process(&config, "hello").unwrap();
        assert_eq!(run.exit_code, 0);
        let target = AuthStorage::open(&clean_agent_dir.join("auth.json")).unwrap();
        assert!(target.get("selected").is_some());
        assert!(target.get("unselected").is_none());
    }

    #[test]
    fn bounded_diagnostic_redacts_credentials_and_limits_output() {
        let run = DavinciProcessRun {
            exit_code: 1,
            stdout_lines: vec!["provider error".into()],
            stderr: format!(
                "{} Authorization: bearer-secret api_key=key-secret",
                "x".repeat(3_000),
            ),
            wall_ms: 1,
            timed_out: false,
        };
        let diagnostic = run.bounded_diagnostic();
        assert!(diagnostic.contains("Authorization=[REDACTED]"));
        assert!(diagnostic.contains("api_key=[REDACTED]"));
        assert!(!diagnostic.contains("bearer-secret"));
        assert!(!diagnostic.contains("key-secret"));
        assert!(diagnostic.chars().count() <= 2_048);
    }

    #[test]
    fn evaluator_child_receives_closed_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("stdin_eof_davinci.py");
        fs::write(
            &script,
            "import sys\nsys.stdin.read()\nprint('{\"type\":\"agent_end\"}')\n",
        )
        .unwrap();
        let config = DavinciProcessConfig {
            binary: "python".into(),
            launcher_args: vec![script.to_string_lossy().into_owned()],
            provider: "fixture".into(),
            model: "fixture-model".into(),
            prompt_profile: PromptProfile::Stable,
            prompt_model_policy_override: None,
            permission_mode: "read-only".into(),
            timeout: Duration::from_millis(250),
            clean_agent_dir: dir.path().join("agent"),
            auth_source: None,
            allowed_env: BTreeMap::new(),
        };

        let run = run_davinci_process(&config, "hello").unwrap();
        assert!(!run.timed_out, "evaluator child must see stdin EOF");
        assert_eq!(run.exit_code, 0);
    }
    #[test]
    fn timeout_returns_a_classified_result() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow_davinci.py");
        fs::write(&script, "import time\ntime.sleep(5)\n").unwrap();
        let config = DavinciProcessConfig {
            binary: "python".into(),
            launcher_args: vec![script.to_string_lossy().into_owned()],
            provider: "fixture".into(),
            model: "fixture-model".into(),
            prompt_profile: PromptProfile::Stable,
            prompt_model_policy_override: None,
            permission_mode: "read-only".into(),
            timeout: Duration::from_millis(100),
            clean_agent_dir: dir.path().join("agent"),
            auth_source: None,
            allowed_env: BTreeMap::new(),
        };
        let run = run_davinci_process(&config, &script.to_string_lossy()).unwrap();
        assert!(run.timed_out);
        assert_eq!(run.exit_code, -1);
    }
}
