use crate::args::{parse_args, Args, APP_NAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCommandKind {
    Check,
    ApiKey,
    BearerToken,
}

#[derive(Debug, Clone)]
pub struct AuthCommand {
    pub kind: AuthCommandKind,
    pub args: Vec<String>,
    pub json: bool,
    pub credentials: bool,
    pub no_refresh: bool,
    pub min_expiry_ms: Option<u64>,
}

#[derive(Debug)]
pub struct AuthCommandError(pub String);

impl std::fmt::Display for AuthCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AuthCommandError {}

pub fn get_auth_command_name(kind: AuthCommandKind) -> &'static str {
    match kind {
        AuthCommandKind::Check => "auth check",
        AuthCommandKind::ApiKey => "auth print-api-key",
        AuthCommandKind::BearerToken => "auth print-bearer-token",
    }
}

pub fn get_auth_command_usage(kind: AuthCommandKind) -> String {
    match kind {
        AuthCommandKind::Check => format!("{APP_NAME} auth check --provider <provider> [--json] [--credentials] [--no-refresh]"),
        AuthCommandKind::ApiKey => format!("{APP_NAME} auth print-api-key --provider <provider> [--model <model>]"),
        AuthCommandKind::BearerToken => format!("{APP_NAME} auth print-bearer-token --provider <provider> [--model <model>] [--min-expiry <duration>]"),
    }
}

pub fn is_auth_command_help(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("auth")
        && (args.get(1).is_none()
            || args.get(1).map(String::as_str) == Some("help")
            || args.iter().any(|a| a == "--help" || a == "-h"))
}

pub fn print_auth_command_help() -> &'static str {
    "Usage:
  pi auth print-api-key [--provider <provider>] [--model <model>]
  pi auth print-bearer-token [--provider <provider>] [--model <model>] [--min-expiry <duration>]
  pi auth check [--provider <provider>] [--model <model>] [--json] [--credentials] [--no-refresh]

Auth commands require at least one of --provider or --model. Checks refresh expired OAuth credentials by default; --no-refresh prevents this. --credentials emits the credential, or includes it in JSON output."
}

pub fn parse_auth_command(args: &[String]) -> Result<Option<AuthCommand>, AuthCommandError> {
    if args.first().map(String::as_str) != Some("auth") {
        return Ok(None);
    }
    let kind = match args.get(1).map(String::as_str) {
        Some("check") => AuthCommandKind::Check,
        Some("print-api-key") => AuthCommandKind::ApiKey,
        Some("print-bearer-token") => AuthCommandKind::BearerToken,
        other => {
            return Err(AuthCommandError(format!(
                "Unknown auth command \"{}\". Use \"{APP_NAME} auth print-api-key\", \"{APP_NAME} auth print-bearer-token\", or \"{APP_NAME} auth check\".",
                other.unwrap_or("")
            )));
        }
    };
    let mut command_args = Vec::new();
    let mut json = false;
    let mut credentials = false;
    let mut no_refresh = false;
    let mut min_expiry_ms = None;
    let mut index = 2;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--min-expiry" {
            if kind != AuthCommandKind::BearerToken {
                return Err(AuthCommandError(
                    "--min-expiry is only supported by print-bearer-token".into(),
                ));
            }
            index += 1;
            let value = args.get(index).cloned();
            let parsed = value.as_deref().and_then(parse_duration);
            min_expiry_ms = Some(parsed.ok_or_else(|| {
                AuthCommandError("--min-expiry must use a duration such as 30m or 1h".into())
            })?);
            index += 1;
            continue;
        }
        if arg == "--json" || arg == "--credentials" || arg == "--no-refresh" {
            if kind != AuthCommandKind::Check {
                return Err(AuthCommandError(format!(
                    "{arg} is only supported by auth check"
                )));
            }
            match arg {
                "--json" => json = true,
                "--credentials" => credentials = true,
                "--no-refresh" => no_refresh = true,
                _ => {}
            }
            index += 1;
            continue;
        }
        command_args.push(args[index].clone());
        index += 1;
    }
    Ok(Some(AuthCommand {
        kind,
        args: command_args,
        json,
        credentials,
        no_refresh,
        min_expiry_ms,
    }))
}

fn parse_duration(value: &str) -> Option<u64> {
    let (amount, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit())?);
    let amount: u64 = amount.parse().ok()?;
    Some(match unit {
        "ms" => amount,
        "s" => amount * 1000,
        "m" => amount * 60_000,
        "h" => amount * 3_600_000,
        _ => return None,
    })
}

pub fn validate_auth_command_args(
    args: &Args,
    kind: AuthCommandKind,
) -> Result<(Option<String>, Option<String>), AuthCommandError> {
    if !args.unknown_flags.is_empty() {
        let option = args.unknown_flags.keys().next().unwrap();
        return Err(AuthCommandError(format!(
            "Unknown option --{option} for \"{}\".",
            get_auth_command_name(kind)
        )));
    }
    if args.api_key.is_some() || !args.messages.is_empty() || !args.file_args.is_empty() {
        return Err(AuthCommandError(
            "Auth commands only accept --provider and --model".into(),
        ));
    }
    let provider = args
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let model = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if provider.is_none() && model.is_none() {
        return Err(AuthCommandError(if kind == AuthCommandKind::Check {
            "Auth checks require --provider <provider> or --model <model>".into()
        } else {
            "Credential printing requires --provider <provider> or --model <model>".into()
        }));
    }
    Ok((provider, model))
}

pub fn parsed_auth_args(command: &AuthCommand) -> Args {
    parse_args(&command.args)
}
