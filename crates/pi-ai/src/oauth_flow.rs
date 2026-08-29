//! OAuth callback + PKCE matching TypeScript `packages/ai/src/auth/oauth`.

use crate::auth::{AuthError, Credential};
use crate::oauth::parse_token_response;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const ANTHROPIC_CALLBACK_PORT: u16 = 53692;
pub const ANTHROPIC_CALLBACK_PATH: &str = "/callback";
pub const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate_pkce() -> Pkce {
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let mut raw = [0u8; 32];
    raw[..16].copy_from_slice(a.as_bytes());
    raw[16..].copy_from_slice(b.as_bytes());
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let digest = Sha256::digest(verifier.as_bytes());
    Pkce {
        verifier,
        challenge: URL_SAFE_NO_PAD.encode(digest),
    }
}

pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Ok(url) = url::Url::parse(value) {
        let mut code = None;
        let mut state = None;
        for (key, val) in url.query_pairs() {
            if key == "code" {
                code = Some(val.into_owned());
            } else if key == "state" {
                state = Some(val.into_owned());
            }
        }
        return (code, state);
    }
    if let Some((code, state)) = value.split_once('#') {
        return (Some(code.to_string()), Some(state.to_string()));
    }
    if value.contains("code=") {
        let mut code = None;
        let mut state = None;
        for pair in value.split('&') {
            if let Some((key, val)) = pair.split_once('=') {
                if key == "code" {
                    code = Some(val.to_string());
                }
                if key == "state" {
                    state = Some(val.to_string());
                }
            }
        }
        return (code, state);
    }
    (Some(value.to_string()), None)
}

pub fn callback_host() -> String {
    std::env::var("PI_OAUTH_CALLBACK_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".into())
}

pub fn anthropic_redirect_uri() -> String {
    format!("http://localhost:{ANTHROPIC_CALLBACK_PORT}{ANTHROPIC_CALLBACK_PATH}")
}

pub fn anthropic_authorize_url(pkce: &Pkce) -> String {
    format!(
        "{ANTHROPIC_AUTHORIZE_URL}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding(ANTHROPIC_CLIENT_ID),
        urlencoding(&anthropic_redirect_uri()),
        urlencoding(ANTHROPIC_SCOPES),
        urlencoding(&pkce.challenge),
        urlencoding(&pkce.verifier),
    )
}

fn urlencoding(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            ' ' => out.push_str("%20"),
            _ => {
                for byte in ch.to_string().into_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_oauth_page(title: &str, heading: &str, message: &str, details: Option<&str>) -> String {
    let details = details
        .map(|text| format!("<div class=\"details\">{}</div>", escape_html(text)))
        .unwrap_or_default();
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\" />\n  <title>{}</title>\n</head>\n<body>\n  <h1>{}</h1>\n  <p>{}</p>\n  {details}\n</body>\n</html>",
        escape_html(title),
        escape_html(heading),
        escape_html(message)
    )
}

pub fn oauth_success_html(message: &str) -> String {
    render_oauth_page(
        "Authentication successful",
        "Authentication successful",
        message,
        None,
    )
}

pub fn oauth_error_html(message: &str, details: Option<&str>) -> String {
    render_oauth_page(
        "Authentication failed",
        "Authentication failed",
        message,
        details,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackCode {
    pub code: String,
    pub state: String,
}

pub fn fixture_authorization_input() -> Option<String> {
    if let Ok(url) = std::env::var("PI_OAUTH_CALLBACK_URL") {
        if !url.trim().is_empty() {
            return Some(url);
        }
    }
    std::env::var("PI_OAUTH_CODE")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub fn take_fixture_callback(expected_state: &str) -> Result<Option<CallbackCode>, String> {
    let Some(input) = fixture_authorization_input() else {
        return Ok(None);
    };
    let (code, state) = parse_authorization_input(&input);
    let code = code.ok_or_else(|| "Missing authorization code".to_string())?;
    let state = state.unwrap_or_else(|| expected_state.to_string());
    if state != expected_state {
        return Err("OAuth state mismatch".into());
    }
    Ok(Some(CallbackCode { code, state }))
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| err.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let n = stream.read(&mut chunk).map_err(|err| err.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16_384 {
            break;
        }
    }
    String::from_utf8(buf).map_err(|err| err.to_string())
}

fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| err.to_string())
}

fn handle_callback_request(
    raw: &str,
    expected_state: &str,
    callback_path: &str,
) -> (u16, String, Option<CallbackCode>) {
    let first = raw.lines().next().unwrap_or_default();
    let path = first.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://localhost{path}"))
        .ok()
        .or_else(|| url::Url::parse(path).ok());
    let Some(url) = url else {
        return (
            400,
            oauth_error_html("Missing code or state parameter.", None),
            None,
        );
    };
    if url.path() != callback_path {
        return (
            404,
            oauth_error_html("Callback route not found.", None),
            None,
        );
    }
    if let Some(error) = url
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned())
    {
        return (
            400,
            oauth_error_html(
                "Anthropic authentication did not complete.",
                Some(&format!("Error: {error}")),
            ),
            None,
        );
    }
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned());
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned());
    match (code, state) {
        (Some(code), Some(state)) if state == expected_state => (
            200,
            oauth_success_html("Anthropic authentication completed. You can close this window."),
            Some(CallbackCode { code, state }),
        ),
        (Some(_), Some(_)) => (400, oauth_error_html("State mismatch.", None), None),
        _ => (
            400,
            oauth_error_html("Missing code or state parameter.", None),
            None,
        ),
    }
}

pub fn wait_for_oauth_callback(
    expected_state: &str,
    host: &str,
    port: u16,
    callback_path: &str,
) -> Result<CallbackCode, String> {
    if let Some(code) = take_fixture_callback(expected_state)? {
        return Ok(code);
    }
    let listener = TcpListener::bind((host, port)).map_err(|err| err.to_string())?;
    let (mut stream, _) = listener.accept().map_err(|err| err.to_string())?;
    let raw = read_http_request(&mut stream)?;
    let (status, body, code) = handle_callback_request(&raw, expected_state, callback_path);
    let _ = write_http(&mut stream, status, &body);
    code.ok_or_else(|| "OAuth callback did not complete.".into())
}

pub fn exchange_authorization_code(
    token_url: &str,
    body: &serde_json::Value,
) -> Result<Credential, AuthError> {
    if let Ok(path) = std::env::var("PI_OAUTH_TOKEN_FIXTURE") {
        if !path.trim().is_empty() {
            let raw = std::fs::read_to_string(path.trim())
                .map_err(|err| AuthError::Message(err.to_string()))?;
            return parse_token_response(&raw);
        }
    }
    if std::env::var("PI_DISABLE_NETWORK").is_ok() || std::env::var("PI_OFFLINE").is_ok() {
        return Err(AuthError::Message(
            "OAuth token exchange disabled (offline)".into(),
        ));
    }
    let response = ureq::post(token_url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_json(body.clone())
        .map_err(|err| {
            AuthError::Message(format!(
                "Token exchange request failed. url={token_url}; details={err}"
            ))
        })?;
    let raw = response.into_string().map_err(|err| {
        AuthError::Message(format!(
            "Token exchange request failed. url={token_url}; details={err}"
        ))
    })?;
    parse_token_response(&raw)
}

pub fn login_anthropic_oauth() -> Result<(Credential, String), String> {
    let pkce = generate_pkce();
    let url = anthropic_authorize_url(&pkce);
    let callback = wait_for_oauth_callback(
        &pkce.verifier,
        &callback_host(),
        std::env::var("PI_OAUTH_CALLBACK_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(ANTHROPIC_CALLBACK_PORT),
        ANTHROPIC_CALLBACK_PATH,
    )?;
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": ANTHROPIC_CLIENT_ID,
        "code": callback.code,
        "state": callback.state,
        "redirect_uri": anthropic_redirect_uri(),
        "code_verifier": pkce.verifier,
    });
    let credential =
        exchange_authorization_code(ANTHROPIC_TOKEN_URL, &body).map_err(|err| err.to_string())?;
    Ok((credential, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_url_and_hash_like_typescript() {
        assert_eq!(
            parse_authorization_input("http://localhost:53692/callback?code=abc&state=st"),
            (Some("abc".into()), Some("st".into()))
        );
        assert_eq!(
            parse_authorization_input("abc#st"),
            (Some("abc".into()), Some("st".into()))
        );
        assert_eq!(
            parse_authorization_input("code=abc&state=st"),
            (Some("abc".into()), Some("st".into()))
        );
        assert_eq!(
            parse_authorization_input("plain-code"),
            (Some("plain-code".into()), None)
        );
    }

    #[test]
    fn callback_html_matches_typescript_titles() {
        let ok =
            oauth_success_html("Anthropic authentication completed. You can close this window.");
        assert!(ok.contains("<title>Authentication successful</title>"));
        assert!(ok.contains("<h1>Authentication successful</h1>"));
        let err = oauth_error_html("State mismatch.", None);
        assert!(err.contains("<title>Authentication failed</title>"));
        assert!(err.contains("State mismatch."));
    }

    #[test]
    fn handle_callback_accepts_matching_state() {
        let raw = "GET /callback?code=tok&state=ver HTTP/1.1\r\n\r\n";
        let (status, body, code) = handle_callback_request(raw, "ver", "/callback");
        assert_eq!(status, 200);
        assert!(body.contains("Authentication successful"));
        assert_eq!(
            code,
            Some(CallbackCode {
                code: "tok".into(),
                state: "ver".into()
            })
        );
        let (status, _, code) =
            handle_callback_request("GET /nope HTTP/1.1\r\n\r\n", "ver", "/callback");
        assert_eq!(status, 404);
        assert!(code.is_none());
    }

    #[test]
    fn anthropic_authorize_url_uses_typescript_client_and_port() {
        let pkce = Pkce {
            verifier: "ver".into(),
            challenge: "ch".into(),
        };
        let url = anthropic_authorize_url(&pkce);
        assert!(url.starts_with(ANTHROPIC_AUTHORIZE_URL));
        assert!(url.contains(ANTHROPIC_CLIENT_ID));
        assert!(url.contains("53692"));
        assert!(url.contains("code_challenge_method=S256"));
    }
}
