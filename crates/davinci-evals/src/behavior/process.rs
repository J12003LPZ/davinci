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

/// Replace credential-shaped values with a marker. Used for every string the
/// harness exports: diagnostics, trace events, verification commands, diffs.
///
/// What is matched:
/// - a credential-named key (`api_key`, `AWS_SECRET_ACCESS_KEY`, `password`,
///   `auth_token`, `Authorization`, ...) followed by `:` or `=` and a value
///   that looks like a credential. The separator and the value's quotes are
///   kept, so JSON text stays valid JSON;
/// - a credential-named long flag followed by a space and its value
///   (`--password hunter2`, `--api-key abc`); the short `-p` is ambiguous and
///   is left alone;
/// - the password of URL userinfo (`postgres://user:hunter2@db`);
/// - credential-shaped values wherever they appear (`sk-...`, `ghp_...`,
///   `xox[bp]-...`, `AKIA...`, Stripe `sk_live_...`, JWTs, `Bearer ...`);
/// - PEM private key blocks, up to their END line or, when the text was cut
///   short, to the end of the text.
///
/// Source code must survive: `let token = read_token();`, `token: usize`,
/// `max_token: 5` and `"input_token": 900` pass through unchanged, and so does
/// `let secret = other_secret;`: an unquoted identifier or path expression
/// (`self.password`, `config::SECRET`) is a reference, not a literal. A quoted
/// string is always a value. Any other unquoted value after a strong key
/// (secret, password, `*_key` names that hold keys, authorization) is a value
/// unless it is a number, a type or keyword, or an expression. The identifier exemption applies only in source assignments (see `is_source_reference`). After a bare
/// `token` key it must also carry a digit and some length, or be long.
pub fn redact_credentials(text: &str) -> String {
    use std::sync::OnceLock;
    static KEYED: OnceLock<regex::Regex> = OnceLock::new();
    static FLAG: OnceLock<regex::Regex> = OnceLock::new();
    static USERINFO: OnceLock<regex::Regex> = OnceLock::new();
    static PEM: OnceLock<regex::Regex> = OnceLock::new();
    static JWT: OnceLock<regex::Regex> = OnceLock::new();
    static BEARER: OnceLock<regex::Regex> = OnceLock::new();
    static BARE: OnceLock<regex::Regex> = OnceLock::new();
    let compile = |cell: &'static OnceLock<regex::Regex>, pattern: &str| -> &'static regex::Regex {
        cell.get_or_init(|| regex::Regex::new(pattern).expect("static credential pattern"))
    };
    let pem = compile(
        &PEM,
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)",
    );
    let userinfo = compile(
        &USERINFO,
        r"([A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]+:)[^/\s@]+@",
    );
    let keyed = compile(
        &KEYED,
        r#"(?i)(["']?)([A-Za-z0-9_-]*?(?:authorization|api[_-]?key|access[_-]?key|private[_-]?key|secret[_-]?key|signing[_-]?key|secret|passw(?:or)?d|passphrase|token))(["']?)(\s*[:=]\s*)("[^"]*"|'[^']*'|(?:(?:bearer|basic)\s+)?[^\s,"}'&;]+)"#,
    );
    let flag = compile(
        &FLAG,
        r#"(?i)(--[A-Za-z0-9_-]*?(?:authorization|api[_-]?key|access[_-]?key|private[_-]?key|secret[_-]?key|signing[_-]?key|secret|passw(?:or)?d|passphrase|token))(\s+)("[^"]*"|'[^']*'|[^\s"'-][^\s"']*)"#,
    );
    let jwt = compile(
        &JWT,
        r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{2,}",
    );
    let bearer = compile(&BEARER, r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}");
    let bare = compile(
        &BARE,
        r"\b(?:sk-[A-Za-z0-9_-]{16,}|(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{8,}|gh[pousr]_[A-Za-z0-9]{16,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35})\b",
    );
    // A redacted value keeps its quotes, so `"k": "v"` stays valid JSON.
    let marker = |value: &str| -> String {
        match value.chars().next() {
            Some(quote @ ('"' | '\'')) => format!("{quote}[REDACTED]{quote}"),
            _ => "[REDACTED]".to_string(),
        }
    };
    let text = pem.replace_all(text, "[REDACTED]");
    let text = userinfo.replace_all(&text, "${1}[REDACTED]@");
    let text = keyed.replace_all(&text, |caps: &regex::Captures| {
        let whole = caps.get(0).expect("whole match");
        if is_source_reference(
            &text[..whole.start()],
            &text[whole.end()..],
            &caps[2],
            &caps[4],
            &caps[5],
        ) {
            caps[0].to_string()
        } else if is_credential_value(
            &caps[2],
            &caps[5],
            is_config_line(&text[..whole.start()], &text[whole.end()..], &caps[4]),
        ) {
            format!(
                "{}{}{}{}{}",
                &caps[1],
                &caps[2],
                &caps[3],
                &caps[4],
                marker(&caps[5])
            )
        } else {
            caps[0].to_string()
        }
    });
    let text = flag.replace_all(&text, |caps: &regex::Captures| {
        format!("{}{}{}", &caps[1], &caps[2], marker(&caps[3]))
    });
    let text = jwt.replace_all(&text, "[REDACTED]");
    let text = bearer.replace_all(&text, "Bearer [REDACTED]");
    bare.replace_all(&text, "[REDACTED]").into_owned()
}

/// A value that is code referring to something else, not a credential. Only
/// when all of these hold:
/// - the key is not ALL_CAPS (that is an environment variable);
/// - the value has no digit and is a path or member expression (`self.password`,
///   `config::SECRET`) or names the credential word itself (`other_secret`);
/// - the line is not a `key=value;key=value` connection string;
/// - the match is a source assignment: a statement that BEGINS with
///   `let/const/var/static` (optionally `pub`) and ends with `;` (path or
///   names-the-word value), or a whole `ident: value,` / `ident: value;` field
///   line whose value starts with `self.` or contains `::` (an empty tail does
///   not count: that is YAML).
///
/// Env, shell, YAML and flag forms never qualify. In doubt, redact.
fn is_source_reference(before: &str, after: &str, key: &str, sep: &str, value: &str) -> bool {
    if key.chars().any(|c| c.is_ascii_uppercase()) && !key.chars().any(|c| c.is_ascii_lowercase()) {
        return false;
    }
    if value.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let before_line = before.rsplit('\n').next().unwrap_or("");
    let after_line = after.split('\n').next().unwrap_or("");
    let line = format!("{before_line}{key}{sep}{value}{after_line}");
    let pairs = line
        .split(';')
        .filter(|part| {
            let part = part.trim();
            part.split_once('=').is_some_and(|(name, _)| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ' ' | '-'))
            })
        })
        .count();
    if pairs >= 2 {
        return false;
    }
    let head = before_line.trim_start();
    let head = head
        .strip_prefix("pub ")
        .or_else(|| head.strip_prefix("pub(crate) "))
        .unwrap_or(head);
    let binding = ["let", "const", "var", "static"].iter().any(|keyword| {
        head.strip_prefix(keyword)
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_whitespace()))
    });
    let tail = after_line.trim();
    let field = before_line.trim().is_empty() && sep.contains(':') && matches!(tail, "," | ";");
    if !(binding || field) {
        return false;
    }
    let is_expression = value
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':'));
    if !is_expression {
        return false;
    }
    let normalized: String = value
        .chars()
        .filter(|c| *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    let is_path = value.contains('.') || value.contains("::");
    if field {
        // A field line (`    password: self.password,`) is exempt only for a
        // member of self or a `::` path; never for a value that merely names
        // the word, which is what a YAML `password: mysecretpassword` looks like.
        return value.starts_with("self.") || value.contains("::");
    }
    // A leading binding must end its statement.
    if tail != ";" {
        return false;
    }
    let names_the_word = [
        "authorization",
        "apikey",
        "accesskey",
        "privatekey",
        "secretkey",
        "signingkey",
        "secret",
        "password",
        "passwd",
        "passphrase",
        "token",
    ]
    .iter()
    .any(|word| normalized.contains(word));
    is_path || names_the_word
}

/// A whole line `key: value` with nothing after the value and no `,`/`;`
/// terminator: YAML or similar config, not a Rust or TS field.
fn is_config_line(before: &str, after: &str, sep: &str) -> bool {
    let before_line = before.rsplit('\n').next().unwrap_or("");
    let after_line = after.split('\n').next().unwrap_or("");
    before_line.trim().is_empty() && sep.contains(':') && after_line.trim().is_empty()
}

fn is_credential_value(key: &str, value: &str, config_line: bool) -> bool {
    let quoted = value.len() >= 3
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')));
    if quoted {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("bearer ") || lower.starts_with("basic ") {
        return true;
    }
    if value.chars().any(|c| "()<>{}[]".contains(c)) {
        return false;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    const NOT_VALUES: &[&str] = &[
        "string",
        "str",
        "usize",
        "isize",
        "bool",
        "char",
        "self",
        "none",
        "null",
        "nil",
        "true",
        "false",
        "undefined",
        "option",
        "vec",
        "box",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "f32",
        "f64",
    ];
    if NOT_VALUES.contains(&lower.as_str()) {
        return false;
    }
    let name: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    const STRONG: &[&str] = &[
        "authorization",
        "apikey",
        "accesskey",
        "privatekey",
        "secretkey",
        "signingkey",
        "secret",
        "password",
        "passwd",
        "passphrase",
    ];
    if STRONG.iter().any(|word| name.ends_with(word)) {
        return true;
    }
    // An ALL_CAPS key is an environment variable, never a bare code name.
    if key.chars().any(|c| c.is_ascii_uppercase()) && !key.chars().any(|c| c.is_ascii_lowercase()) {
        return true;
    }
    // YAML-style `token: value` on its own line is configuration.
    if config_line {
        return true;
    }
    // A bare `token` key: code names tokens all the time, so the value must
    // look like one.
    let has_digit = value.chars().any(|c| c.is_ascii_digit());
    (has_digit && value.len() >= 8)
        || (value.len() >= 24 && !value.contains('.'))
        || (value.len() >= 16 && value.contains(['/', '+', '=']))
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
        let redacted = redact_credentials(&combined);
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

    // Hosted Windows runners can spend more than 30 seconds starting Python
    // while other tests compete for resources. These tests check child
    // behavior, not startup speed; the timeout regression keeps its short bound.
    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(120);

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
            timeout: FIXTURE_TIMEOUT,
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
            timeout: FIXTURE_TIMEOUT,
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
            timeout: FIXTURE_TIMEOUT,
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
    fn redaction_covers_prefixed_env_keys_json_keys_and_bearer() {
        for (input, secret) in [
            ("OPENAI_API_KEY=fakevalue-0123 run", "fakevalue-0123"),
            ("export GITHUB_TOKEN=fakevalue-4567", "fakevalue-4567"),
            (r#"{"token": "abc-fake-1"}"#, "abc-fake-1"),
            (r#"{"client_secret":"abc fake 2"}"#, "abc fake 2"),
            (r#"{"password": 'pw fake 3'}"#, "pw fake 3"),
            ("Authorization: Bearer fakebearer4 ok", "fakebearer4"),
            (r#"{"Authorization": "Bearer fakebearer5"}"#, "fakebearer5"),
        ] {
            let out = redact_credentials(input);
            assert!(!out.contains(secret), "{input} -> {out}");
            assert!(out.contains("[REDACTED]"), "{input} -> {out}");
        }
    }

    #[test]
    fn redaction_covers_credential_names_and_value_shapes() {
        // (input, the secret that must be gone)
        for (input, secret) in [
            (
                "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG",
                "wJalrXUtnFEMI/K7MDENG",
            ),
            ("access_key=AKIAabc", "AKIAabc"),
            ("DB_PRIVATE_KEY=abcdef123", "abcdef123"),
            ("aws id AKIAIOSFODNN7EXAMPLE here", "AKIAIOSFODNN7EXAMPLE"),
            (
                "slack xoxb-1234567890-abcdefghij",
                "xoxb-1234567890-abcdefghij",
            ),
            (
                "slack xoxp-1234567890-abcdefghij",
                "xoxp-1234567890-abcdefghij",
            ),
            (
                "pat ghp_0123456789abcdefghij0123",
                "ghp_0123456789abcdefghij0123",
            ),
            ("key sk-fake-0123456789abcdef", "sk-fake-0123456789abcdef"),
            (
                "curl -H 'Authorization: Bearer abcdefgh12345'",
                "abcdefgh12345",
            ),
            ("sent Bearer abcdefgh12345 upstream", "abcdefgh12345"),
            (r#"api_key = "hunter2-fake""#, "hunter2-fake"),
            (r#"{"password": "pw-fake-1"}"#, "pw-fake-1"),
            ("export STRIPE_SECRET=fakevalue-9876", "fakevalue-9876"),
            (r#"{"token": "plain words allowed"}"#, "plain words allowed"),
            ("auth_token: abcdef0123456789", "abcdef0123456789"),
        ] {
            let out = redact_credentials(input);
            assert!(!out.contains(secret), "{input} -> {out}");
            assert!(out.contains("[REDACTED]"), "{input} -> {out}");
        }
    }

    #[test]
    fn redaction_covers_flags_userinfo_and_provider_shapes() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAfakefakefake\nabcdefghijklmnop\n-----END RSA PRIVATE KEY-----";
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.c2lnbmF0dXJlZmFrZQ";
        let wrapped = format!(
            "key:
{pem}
end"
        );
        let saw = format!("saw {jwt} in log");
        for (input, secret) in [
            ("curl --password hunter2 host", "hunter2"),
            ("mysql --api-key abc123xyz host", "abc123xyz"),
            ("tool --token xq9 host", "xq9"),
            ("tool --client-secret 'two words' go", "two words"),
            ("tool --api_key abcdef", "abcdef"),
            ("psql postgres://user:hunter2@db/app", "hunter2"),
            (
                "git clone https://bot:ghfake-pw@example.com/r.git",
                "ghfake-pw",
            ),
            (
                "charge sk_live_FAKEFAKEFAKE0123",
                "sk_live_FAKEFAKEFAKE0123",
            ),
            (
                "charge sk_test_FAKEFAKEFAKE0123",
                "sk_test_FAKEFAKEFAKE0123",
            ),
            (
                "restricted rk_live_FAKEFAKEFAKE0123",
                "rk_live_FAKEFAKEFAKE0123",
            ),
            (pem, "MIIEowIBAAKCAQEAfakefakefake"),
            (wrapped.as_str(), "abcdefghijklmnop"),
            (
                "-----BEGIN PRIVATE KEY-----\nMIIfaketruncatedbody",
                "MIIfaketruncatedbody",
            ),
            (saw.as_str(), jwt),
        ] {
            let out = redact_credentials(input);
            assert!(!out.contains(secret), "{input} -> {out}");
            assert!(out.contains("[REDACTED]"), "{input} -> {out}");
        }
        // The user name and host of a URL survive, so the log stays readable.
        assert_eq!(
            redact_credentials("psql postgres://user:hunter2@db/app"),
            "psql postgres://user:[REDACTED]@db/app"
        );
        // The short `-p` is ambiguous (port, path, password) and is left alone.
        assert_eq!(
            redact_credentials("mysql -p 3306 host"),
            "mysql -p 3306 host"
        );
        // A flag whose next word is another flag has no value to redact.
        assert_eq!(
            redact_credentials("tool --token --verbose"),
            "tool --token --verbose"
        );
        // Ordinary URLs and flags are untouched.
        for input in [
            "https://example.com:8080/path",
            "git@github.com:org/repo.git",
            "tool --max-tokens 5 --token-file creds.txt",
            "cargo test --password-policy strict",
        ] {
            assert_eq!(redact_credentials(input), input);
        }
    }

    #[test]
    fn redaction_keeps_the_original_separator_so_json_stays_valid() {
        let json = r#"{"password": "pw-fake-1", "token":"abc-fake-1", "api_key" : 'k-fake-2'}"#;
        let out = redact_credentials(json);
        assert!(
            !out.contains("pw-fake-1") && !out.contains("abc-fake-1") && !out.contains("k-fake-2")
        );
        assert_eq!(
            out,
            r#"{"password": "[REDACTED]", "token":"[REDACTED]", "api_key" : '[REDACTED]'}"#
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&redact_credentials(r#"{"password": "pw-fake-1", "n": 1}"#))
                .unwrap();
        assert_eq!(parsed["password"], "[REDACTED]");
        assert_eq!(
            redact_credentials("export API_KEY = fake-0123"),
            "export API_KEY = [REDACTED]"
        );
        assert_eq!(
            redact_credentials("auth_token: abcdef0123456789"),
            "auth_token: [REDACTED]"
        );
    }

    #[test]
    fn strong_keys_keep_source_references_but_never_env_or_config_values() {
        // Source code: a binding or field whose value is a path or names the
        // same credential word is a reference, not a literal.
        for input in [
            "let secret = other_secret;",
            "let password = self.password;",
            "const SECRET: &str = crate::keys::SECRET;",
            "let secret = settings::SECRET_VALUE;",
            "static PASSWORD_FILE: &str = config.password_file;",
            "    password: self.password,",
            "var apiKey = old_api_key;",
        ] {
            assert_eq!(redact_credentials(input), input);
        }
        // Env, shell and config forms are always redacted, whatever the shape
        // of the value, and so are source literals.
        for (input, secret) in [
            (r#"let secret = "hunter2-fake";"#, "hunter2-fake"),
            ("secret=hunter2", "hunter2"),
            ("PASSWORD=Tr0ub4dor", "Tr0ub4dor"),
            ("DB_PASSWORD=correct_horse_battery", "correct_horse_battery"),
            ("export API_TOKEN=abc_def", "abc_def"),
            ("password: hunter_two", "hunter_two"),
            ("password = other_word", "other_word"),
            ("export SECRET=old_secret", "old_secret"),
            (
                "let password = correct_horse_battery;",
                "correct_horse_battery",
            ),
            ("db:\n  password: hunter_two,", "hunter_two"),
            (
                "tool --password correct_horse_battery",
                "correct_horse_battery",
            ),
        ] {
            let out = redact_credentials(input);
            assert!(!out.contains(secret), "{input} -> {out}");
        }
    }

    #[test]
    fn redaction_leaves_code_and_numbers_alone() {
        for input in [
            "let token = read_token();",
            "token: usize",
            "max_token: 5",
            r#"{"input_token": 900}"#,
            "pub struct Cfg { token: String }",
            "pub token: usize,",
            "let secret = load_secret();",
            "password: String",
            "primary_key: u32",
            "cache_key = compute(a)",
            "token = self.next()",
            "secret: Option<String>",
        ] {
            assert_eq!(redact_credentials(input), input);
        }
    }

    #[test]
    fn redaction_leaves_non_secret_text_unchanged() {
        for input in [
            "tokenizer: x",
            "max_tokens: 5",
            r#"{"input_tokens": 900, "output_tokens": 3}"#,
            "secretary: Jo",
            "the password policy was reviewed",
        ] {
            assert_eq!(redact_credentials(input), input);
        }
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
        assert!(diagnostic.contains("Authorization: [REDACTED]"));
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
            // The assertion is about EOF semantics, not interpreter startup.
            // A child whose stdin stays open blocks in read() until this
            // timeout, so a generous bound still catches the regression while
            // a slow hosted runner (2 s was not enough on Windows) cannot
            // turn Python startup into a false failure. A pass returns as
            // soon as the child exits.
            timeout: FIXTURE_TIMEOUT,
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
