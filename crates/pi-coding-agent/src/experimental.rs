//! Experimental `pi server` / `pi client` matching `vendor/pi/packages/coding-agent/src/cli/experimental`.

use std::path::PathBuf;

use pi_client::{connect_unix, write_message, PiClient};
use pi_server::{
    bind_unix, encode_auth_preamble, serve_stream_with_auth, BoundUnixListener, PiServer,
};
use std::io::Write;
use std::sync::Mutex;

static PI_LISTEN_BINDINGS: Mutex<Vec<BoundUnixListener>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixAddress {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCommand {
    pub listen: Vec<UnixAddress>,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCommand {
    pub connect: Option<UnixAddress>,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentalAuth {
    Token { token: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExperimentalCli {
    Pi {
        listen: Vec<UnixAddress>,
        auth: Option<ExperimentalAuth>,
        options: crate::args::Args,
    },
    Server {
        listen: Vec<UnixAddress>,
        auth: Option<ExperimentalAuth>,
    },
    Client {
        connect: Option<UnixAddress>,
        auth: Option<ExperimentalAuth>,
    },
}

struct ParsedCommandInput {
    values: std::collections::BTreeMap<String, Vec<String>>,
    remaining_args: Vec<String>,
    errors: Vec<String>,
}

fn parse_command_options(argv: &[String], option_names: &[&str]) -> ParsedCommandInput {
    let mut values: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut remaining_args = Vec::new();
    let mut errors = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        let argument = &argv[index];
        if argument == "--" {
            remaining_args.extend(argv[index..].iter().cloned());
            break;
        }
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (argument.as_str(), None),
        };
        if !option_names.contains(&name) {
            remaining_args.extend(argv[index..].iter().cloned());
            break;
        }
        let value = if let Some(value) = inline {
            Some(value)
        } else if argv
            .get(index + 1)
            .is_some_and(|next| !next.starts_with('-'))
        {
            index += 1;
            argv.get(index).cloned()
        } else {
            None
        };
        if value.as_deref().is_none_or(str::is_empty) {
            errors.push(format!("{name} requires a value"));
            index += 1;
            continue;
        }
        let value = value.expect("value checked");
        let entry = values.entry(name.to_string()).or_default();
        if !entry.is_empty() {
            errors.push(format!("{name} may only be specified once"));
            index += 1;
            continue;
        }
        entry.push(value);
        index += 1;
    }
    ParsedCommandInput {
        values,
        remaining_args,
        errors,
    }
}

fn parse_auth_from_input(input: &ParsedCommandInput) -> (Option<ExperimentalAuth>, Vec<String>) {
    let token = input
        .values
        .get("--auth-token")
        .and_then(|values| values.first())
        .cloned();
    let file = input
        .values
        .get("--auth-token-file")
        .and_then(|values| values.first())
        .cloned();
    match (token, file) {
        (Some(_), Some(_)) => (
            None,
            vec!["--auth-token and --auth-token-file are mutually exclusive".into()],
        ),
        (Some(token), None) => (Some(ExperimentalAuth::Token { token }), Vec::new()),
        (None, Some(path)) => (Some(ExperimentalAuth::File { path }), Vec::new()),
        (None, None) => (None, Vec::new()),
    }
}

fn parse_listen_values(input: &ParsedCommandInput) -> (Vec<UnixAddress>, Vec<String>) {
    let mut listen = Vec::new();
    let mut errors = Vec::new();
    if let Some(values) = input.values.get("--listen") {
        for value in values {
            match parse_transport_address(value, "--listen") {
                Ok(address) => listen.push(address),
                Err(error) => errors.push(error),
            }
        }
    }
    (listen, errors)
}

fn parse_connect_value(input: &ParsedCommandInput) -> (Option<UnixAddress>, Vec<String>) {
    let Some(value) = input
        .values
        .get("--connect")
        .and_then(|values| values.first())
    else {
        return (None, Vec::new());
    };
    match parse_transport_address(value, "--connect") {
        Ok(address) => (Some(address), Vec::new()),
        Err(error) => (None, vec![error]),
    }
}

fn legacy_option_errors(remaining: &[String]) -> (crate::args::Args, Vec<String>) {
    let options = crate::args::parse_args(remaining);
    let errors = options
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "error")
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    (options, errors)
}

/// TS `experimentalCli.parse`.
pub fn parse_experimental_cli(args: &[String]) -> Result<ExperimentalCli, Vec<String>> {
    match args.first().map(String::as_str) {
        Some("server") => parse_experimental_server(&args[1..]),
        Some("client") => parse_experimental_client(&args[1..]),
        _ => parse_experimental_pi(args),
    }
}

fn parse_experimental_pi(args: &[String]) -> Result<ExperimentalCli, Vec<String>> {
    let input = parse_command_options(args, &["--listen", "--auth-token", "--auth-token-file"]);
    let (auth, auth_errors) = parse_auth_from_input(&input);
    let (listen, listen_errors) = parse_listen_values(&input);
    let (options, option_errors) = legacy_option_errors(&input.remaining_args);
    let mut errors = input.errors;
    errors.extend(auth_errors);
    errors.extend(listen_errors);
    errors.extend(option_errors);
    if options.unknown_flags.contains_key("connect") {
        errors.push("--connect is only valid for client mode".into());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ExperimentalCli::Pi {
        listen,
        auth,
        options,
    })
}

fn parse_experimental_server(args: &[String]) -> Result<ExperimentalCli, Vec<String>> {
    let input = parse_command_options(args, &["--listen", "--auth-token", "--auth-token-file"]);
    let (auth, auth_errors) = parse_auth_from_input(&input);
    let (listen, listen_errors) = parse_listen_values(&input);
    let (_, option_errors) = legacy_option_errors(&input.remaining_args);
    let mut errors = input.errors;
    errors.extend(auth_errors);
    errors.extend(listen_errors);
    errors.extend(option_errors);
    if !input.remaining_args.is_empty() {
        errors.push(
            "The experimental server command does not support existing CLI options yet".into(),
        );
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ExperimentalCli::Server { listen, auth })
}

fn parse_experimental_client(args: &[String]) -> Result<ExperimentalCli, Vec<String>> {
    let input = parse_command_options(args, &["--connect", "--auth-token", "--auth-token-file"]);
    let (auth, auth_errors) = parse_auth_from_input(&input);
    let (connect, connect_errors) = parse_connect_value(&input);
    let (_, option_errors) = legacy_option_errors(&input.remaining_args);
    let mut errors = input.errors;
    errors.extend(auth_errors);
    errors.extend(connect_errors);
    errors.extend(option_errors);
    if !input.remaining_args.is_empty() {
        errors.push(
            "The experimental client command does not support existing CLI options yet".into(),
        );
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ExperimentalCli::Client { connect, auth })
}

pub fn resolve_experimental_auth(auth: Option<ExperimentalAuth>) -> Result<Option<String>, String> {
    match auth {
        None => Ok(None),
        Some(ExperimentalAuth::Token { token }) => Ok(Some(token)),
        Some(ExperimentalAuth::File { path }) => {
            let token = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
            Ok(Some(token.trim().to_string()))
        }
    }
}

pub fn bind_listen_addresses(listen: &[UnixAddress]) -> Result<String, String> {
    if listen.is_empty() {
        return Ok(String::new());
    }
    if std::env::var("PI_SERVER_DRY_RUN").is_ok() || cfg!(test) {
        return Ok(format!(
            "Listening on {}",
            listen
                .iter()
                .map(|addr| format!("unix://{}", addr.path))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let address = &listen[0];
    let bound = bind_unix(&address.path).map_err(|err| err.to_string())?;
    PI_LISTEN_BINDINGS
        .lock()
        .map_err(|err| err.to_string())?
        .push(bound);
    Ok(format!("Listening on unix://{}", address.path))
}

fn percent_decode(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| ())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

pub fn parse_transport_address(value: &str, option: &str) -> Result<UnixAddress, String> {
    let url =
        url::Url::parse(value).map_err(|_| format!("Invalid {option} address \"{value}\""))?;
    if url.scheme() != "unix" {
        return Err(format!(
            "Unsupported {option} transport \"{}:\"",
            url.scheme()
        ));
    }
    if url.host_str().is_some() || url.port().is_some() || !url.username().is_empty() {
        return Err("Unix transport address must not include an authority".into());
    }
    if !value.starts_with("unix:///")
        || value.starts_with("unix:////")
        || value.contains('?')
        || value.contains('#')
    {
        return Err(format!("Invalid {option} address \"{value}\""));
    }
    let path =
        percent_decode(url.path()).map_err(|_| format!("Invalid {option} address \"{value}\""))?;
    if path.contains('\0') {
        return Err(format!("Invalid {option} address \"{value}\""));
    }
    if !path.starts_with('/') {
        return Err("Unix transport address requires an absolute path".into());
    }
    Ok(UnixAddress { path })
}

#[allow(dead_code)]
pub fn parse_server_command(args: &[String]) -> Result<ServerCommand, String> {
    match parse_experimental_cli(
        &std::iter::once("server".to_string())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>(),
    )
    .map_err(|errors| errors.join("\n"))?
    {
        ExperimentalCli::Server { listen, auth } => Ok(ServerCommand {
            listen,
            auth_token: resolve_experimental_auth(auth)?,
        }),
        _ => Err("expected server command".into()),
    }
}

#[allow(dead_code)]
pub fn parse_client_command(args: &[String]) -> Result<ClientCommand, String> {
    match parse_experimental_cli(
        &std::iter::once("client".to_string())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>(),
    )
    .map_err(|errors| errors.join("\n"))?
    {
        ExperimentalCli::Client { connect, auth } => Ok(ClientCommand {
            connect,
            auth_token: resolve_experimental_auth(auth)?,
        }),
        _ => Err("expected client command".into()),
    }
}

pub fn run_server(command: ServerCommand) -> Result<String, String> {
    if command.listen.is_empty() {
        return Err("server requires --listen unix:///absolute/path".into());
    }
    if std::env::var("PI_SERVER_DRY_RUN").is_ok() || cfg!(test) {
        return Ok(format!(
            "Listening on {}",
            command
                .listen
                .iter()
                .map(|addr| format!("unix://{}", addr.path))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let address = &command.listen[0];
    let listener = bind_unix(&address.path).map_err(|err| err.to_string())?;
    let sessions_dir = std::env::var("PI_SESSION_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| pi_session::default_session_dir());
    let mut server = PiServer::new(sessions_dir);
    let stream = listener.accept().map_err(|err| err.to_string())?;
    serve_stream_with_auth(&mut server, stream, command.auth_token.as_deref())
        .map_err(|err| err.to_string())?;
    Ok(format!("Served {}", address.path))
}

pub fn run_client(command: ClientCommand) -> Result<String, String> {
    let address = command
        .connect
        .ok_or_else(|| "client requires --connect unix:///absolute/path".to_string())?;
    if std::env::var("PI_CLIENT_DRY_RUN").is_ok() || cfg!(test) {
        return Ok(format!("Connecting to unix://{}", address.path));
    }
    let mut stream = connect_unix(&address.path).map_err(|err| err.to_string())?;
    if let Some(token) = &command.auth_token {
        stream
            .write_all(&encode_auth_preamble(token))
            .map_err(|err| err.to_string())?;
    }
    write_message(&mut stream, &PiClient::hello_message()).map_err(|err| err.to_string())?;
    Ok(format!("Connected to unix://{}", address.path))
}

pub fn is_experimental_command(command: Option<&str>) -> bool {
    matches!(command, Some("server") | Some("client"))
}

/// TS `areExperimentalFeaturesEnabled` — `PI_EXPERIMENTAL === "1"`.
pub fn experimental_features_enabled() -> bool {
    matches!(
        std::env::var("PI_EXPERIMENTAL").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// TS `getExperimentalToolSampling` — `{ type: "json_schema", strict: "prefer" }`.
pub fn experimental_tool_sampling() -> Option<serde_json::Value> {
    if experimental_features_enabled() {
        Some(serde_json::json!({"type":"json_schema","strict":"prefer"}))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parses_unix_listen_and_rejects_authority() {
        let address = parse_transport_address("unix:///tmp/pi.sock", "--listen").unwrap();
        assert_eq!(address.path, "/tmp/pi.sock");
        assert!(parse_transport_address("tcp://127.0.0.1:9", "--listen")
            .unwrap_err()
            .contains("Unsupported --listen transport"));
        assert_eq!(
            parse_transport_address("unix://localhost/tmp/pi.sock", "--listen").unwrap_err(),
            "Unix transport address must not include an authority"
        );
        let server = parse_server_command(&[
            "--listen".into(),
            "unix:///tmp/pi.sock".into(),
            "--auth-token".into(),
            "secret".into(),
        ])
        .unwrap();
        assert_eq!(server.listen[0].path, "/tmp/pi.sock");
        assert_eq!(server.auth_token.as_deref(), Some("secret"));
        std::env::set_var("PI_SERVER_DRY_RUN", "1");
        assert!(run_server(server).unwrap().contains("unix:///tmp/pi.sock"));
        std::env::remove_var("PI_SERVER_DRY_RUN");
        std::env::set_var("PI_CLIENT_DRY_RUN", "1");
        let client =
            parse_client_command(&["--connect".into(), "unix:///tmp/pi.sock".into()]).unwrap();
        assert!(run_client(client).unwrap().contains("unix:///tmp/pi.sock"));
        std::env::remove_var("PI_CLIENT_DRY_RUN");
    }

    #[test]
    fn experimental_tool_sampling_matches_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_EXPERIMENTAL");
        assert!(!experimental_features_enabled());
        assert_eq!(experimental_tool_sampling(), None);
        std::env::set_var("PI_EXPERIMENTAL", "1");
        assert!(experimental_features_enabled());
        assert_eq!(
            experimental_tool_sampling(),
            Some(serde_json::json!({"type":"json_schema","strict":"prefer"}))
        );
        std::env::remove_var("PI_EXPERIMENTAL");
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn experimental_cli_composes_pi_options() {
        let parsed = parse_experimental_cli(&s(&[
            "--listen",
            "unix:///tmp/pi.sock",
            "--auth-token",
            "secret",
            "--provider",
            "anthropic",
            "--model",
            "claude-sonnet",
            "--thinking",
            "high",
            "inspect",
        ]))
        .unwrap();
        match parsed {
            ExperimentalCli::Pi {
                listen,
                auth,
                options,
            } => {
                assert_eq!(listen[0].path, "/tmp/pi.sock");
                assert_eq!(
                    auth,
                    Some(ExperimentalAuth::Token {
                        token: "secret".into()
                    })
                );
                assert_eq!(options.provider.as_deref(), Some("anthropic"));
                assert_eq!(options.model.as_deref(), Some("claude-sonnet"));
                assert_eq!(options.messages, ["inspect"]);
            }
            other => panic!("expected pi: {other:?}"),
        }
    }

    #[test]
    fn experimental_cli_keeps_help_and_version_on_pi() {
        match parse_experimental_cli(&s(&["--help"])).unwrap() {
            ExperimentalCli::Pi { options, .. } => assert!(options.help),
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["--version"])).unwrap() {
            ExperimentalCli::Pi { options, .. } => assert!(options.version),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn experimental_cli_rejects_server_and_client_legacy_options() {
        const SERVER: &str =
            "The experimental server command does not support existing CLI options yet";
        const CLIENT: &str =
            "The experimental client command does not support existing CLI options yet";
        assert_eq!(
            parse_experimental_cli(&s(&["server", "--help"])).unwrap_err(),
            [SERVER]
        );
        assert_eq!(
            parse_experimental_cli(&s(&["server", "--version"])).unwrap_err(),
            [SERVER]
        );
        assert_eq!(
            parse_experimental_cli(&s(&["client", "--help"])).unwrap_err(),
            [CLIENT]
        );
        assert_eq!(
            parse_experimental_cli(&s(&["client", "--version"])).unwrap_err(),
            [CLIENT]
        );
        assert_eq!(
            parse_experimental_cli(&s(&["server", "--model", "claude-sonnet", "prompt"]))
                .unwrap_err(),
            [SERVER]
        );
        assert_eq!(
            parse_experimental_cli(&s(&["client", "--tui-mode", "fullscreen", "@prompt.md"]))
                .unwrap_err(),
            [CLIENT]
        );
        assert_eq!(
            parse_experimental_cli(&s(&[
                "client",
                "--tui-mode",
                "wrong",
                "--model",
                "claude-sonnet"
            ]))
            .unwrap_err(),
            [
                "Invalid TUI mode \"wrong\". Valid values: regular, fullscreen",
                CLIENT
            ]
        );
        match parse_experimental_cli(&s(&["server"])).unwrap() {
            ExperimentalCli::Server { listen, auth } => {
                assert!(listen.is_empty());
                assert!(auth.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn experimental_cli_command_cases_match_ts() {
        match parse_experimental_cli(&s(&[
            "--provider",
            "anthropic",
            "--model",
            "claude-sonnet",
            "--thinking",
            "high",
            "inspect",
            "the project",
        ]))
        .unwrap()
        {
            ExperimentalCli::Pi {
                options, listen, ..
            } => {
                assert!(listen.is_empty());
                assert_eq!(options.messages, ["inspect", "the project"]);
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["server", "--listen", "unix:///tmp/pi.sock"])).unwrap() {
            ExperimentalCli::Server { listen, .. } => assert_eq!(listen[0].path, "/tmp/pi.sock"),
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["--system-prompt", "--listen", "unix:///tmp/pi.sock"]))
            .unwrap()
        {
            ExperimentalCli::Pi {
                listen, options, ..
            } => {
                assert!(listen.is_empty());
                assert_eq!(options.system_prompt.as_deref(), Some("--listen"));
                assert_eq!(options.messages, ["unix:///tmp/pi.sock"]);
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&[
            "--model",
            "claude-sonnet",
            "--listen=unix:///tmp/second.sock",
        ]))
        .unwrap()
        {
            ExperimentalCli::Pi {
                listen, options, ..
            } => {
                assert!(listen.is_empty());
                match options.unknown_flags.get("listen") {
                    Some(crate::args::FlagValue::String(value)) => {
                        assert_eq!(value, "unix:///tmp/second.sock");
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["client", "--connect", "unix:///tmp/pi.sock"])).unwrap() {
            ExperimentalCli::Client { connect, .. } => {
                assert_eq!(connect.unwrap().path, "/tmp/pi.sock");
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["--auth-token", "secret"])).unwrap() {
            ExperimentalCli::Pi { auth, .. } => {
                assert_eq!(
                    auth,
                    Some(ExperimentalAuth::Token {
                        token: "secret".into()
                    })
                );
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["--auth-token-file", "/tmp/token"])).unwrap() {
            ExperimentalCli::Pi { auth, .. } => {
                assert_eq!(
                    auth,
                    Some(ExperimentalAuth::File {
                        path: "/tmp/token".into()
                    })
                );
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&[
            "--unknown",
            "@prompt.md",
            "--",
            "--listen",
            "unix:///tmp/pi.sock",
        ]))
        .unwrap()
        {
            ExperimentalCli::Pi { options, .. } => {
                assert_eq!(options.file_args, ["prompt.md"]);
                assert_eq!(options.messages, ["--listen", "unix:///tmp/pi.sock"]);
                assert_eq!(
                    options.unknown_flags.get("unknown"),
                    Some(&crate::args::FlagValue::Bool(true))
                );
            }
            other => panic!("{other:?}"),
        }
        match parse_experimental_cli(&s(&["--cwd", "/workspace", "server"])).unwrap() {
            ExperimentalCli::Pi { options, .. } => {
                assert_eq!(options.messages, ["server"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn experimental_cli_rejects_invalid_input() {
        let cases: &[(&[&str], &str)] = &[
            (
                &[
                    "--listen",
                    "unix:///tmp/pi.sock",
                    "--listen",
                    "unix:///tmp/pi-admin.sock",
                ],
                "--listen may only be specified once",
            ),
            (
                &["--auth-token", "secret", "--auth-token-file", "/tmp/token"],
                "--auth-token and --auth-token-file are mutually exclusive",
            ),
            (
                &["--auth-token", "first", "--auth-token", "second"],
                "--auth-token may only be specified once",
            ),
            (
                &[
                    "--auth-token-file",
                    "/tmp/first",
                    "--auth-token-file=/tmp/second",
                ],
                "--auth-token-file may only be specified once",
            ),
            (
                &["--listen", "/tmp/pi.sock"],
                "Invalid --listen address \"/tmp/pi.sock\"",
            ),
            (
                &["--listen", "ws://localhost:8080"],
                "Unsupported --listen transport \"ws:\"",
            ),
            (
                &["--listen", "unix://relative.sock"],
                "Unix transport address must not include an authority",
            ),
            (
                &["--listen", "unix:///tmp/pi.sock?wrong=value"],
                "Invalid --listen address \"unix:///tmp/pi.sock?wrong=value\"",
            ),
            (
                &["--listen", "unix:///tmp/pi.sock#fragment"],
                "Invalid --listen address \"unix:///tmp/pi.sock#fragment\"",
            ),
            (
                &["--listen", "unix:/tmp/pi.sock"],
                "Invalid --listen address \"unix:/tmp/pi.sock\"",
            ),
            (
                &["--listen", "unix:///tmp/%00pi.sock"],
                "Invalid --listen address \"unix:///tmp/%00pi.sock\"",
            ),
            (
                &["client", "--listen", "unix:///tmp/pi.sock"],
                "The experimental client command does not support existing CLI options yet",
            ),
            (
                &["server", "--connect", "unix:///tmp/pi.sock"],
                "The experimental server command does not support existing CLI options yet",
            ),
            (
                &["client", "--connect", "ws://localhost:8080"],
                "Unsupported --connect transport \"ws:\"",
            ),
            (&["--listen"], "--listen requires a value"),
            (&["--connect="], "--connect is only valid for client mode"),
        ];
        for (argv, error) in cases {
            let errors = parse_experimental_cli(&s(argv)).unwrap_err();
            assert!(
                errors.iter().any(|item| item.contains(error)),
                "expected {error} in {errors:?} for {argv:?}"
            );
        }
        assert_eq!(
            parse_experimental_cli(&s(&[
                "client",
                "--listen",
                "ws://localhost:8080",
                "--auth-token",
                "secret",
                "--auth-token-file",
                "/tmp/token",
            ]))
            .unwrap_err(),
            ["The experimental client command does not support existing CLI options yet"]
        );
    }
}
