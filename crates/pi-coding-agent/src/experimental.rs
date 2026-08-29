//! Experimental `pi server` / `pi client` matching `vendor/pi/packages/coding-agent/src/cli/experimental`.

use std::path::PathBuf;

use pi_client::{connect_unix, write_message, PiClient};
use pi_server::{bind_unix, encode_auth_preamble, serve_stream_with_auth, PiServer};
use std::io::Write;

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

fn parse_auth(args: &[String]) -> Result<Option<String>, String> {
    let mut token = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--auth-token" => {
                index += 1;
                token = args.get(index).cloned();
            }
            "--auth-token-file" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--auth-token-file requires a path".to_string())?;
                token = Some(
                    std::fs::read_to_string(path)
                        .map_err(|err| err.to_string())?
                        .trim()
                        .to_string(),
                );
            }
            _ => {}
        }
        index += 1;
    }
    Ok(token)
}

pub fn parse_server_command(args: &[String]) -> Result<ServerCommand, String> {
    let mut listen = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--listen" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--listen requires an address".to_string())?;
            listen.push(parse_transport_address(value, "--listen")?);
        }
        index += 1;
    }
    Ok(ServerCommand {
        listen,
        auth_token: parse_auth(args)?,
    })
}

pub fn parse_client_command(args: &[String]) -> Result<ClientCommand, String> {
    let mut connect = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--connect" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--connect requires an address".to_string())?;
            connect = Some(parse_transport_address(value, "--connect")?);
        }
        index += 1;
    }
    Ok(ClientCommand {
        connect,
        auth_token: parse_auth(args)?,
    })
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
    let _ = std::fs::remove_file(&address.path);
    let listener = bind_unix(&address.path).map_err(|err| err.to_string())?;
    let sessions_dir = std::env::var("PI_SESSION_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| pi_session::default_session_dir());
    let mut server = PiServer::new(sessions_dir);
    let (stream, _) = listener.accept().map_err(|err| err.to_string())?;
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
}
