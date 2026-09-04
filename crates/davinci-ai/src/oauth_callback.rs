//! Loopback OAuth callback HTTP server matching
//! `vendor/pi/packages/ai/src/auth/oauth/{anthropic,openai-codex,openrouter,radius,oauth-page}.ts`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

const LOGO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 800" aria-hidden="true"><path fill="#fff" fill-rule="evenodd" d="M165.29 165.29 H517.36 V400 H400 V517.36 H282.65 V634.72 H165.29 Z M282.65 282.65 V400 H400 V282.65 Z"/><path fill="#fff" d="M517.36 400 H634.72 V634.72 H517.36 Z"/></svg>"##;

pub const TITLE_SUCCESS: &str = "Authentication successful";
pub const TITLE_FAILED: &str = "Authentication failed";
pub const CONTENT_TYPE_HTML: &str = "text/html; charset=utf-8";
pub const CONTENT_TYPE_PLAIN: &str = "text/plain; charset=utf-8";

pub const ERR_CALLBACK_ROUTE_NOT_FOUND: &str = "Callback route not found.";
pub const ERR_OAUTH_CALLBACK_ROUTE_NOT_FOUND: &str = "OAuth callback route not found.";
pub const ERR_MISSING_CODE_OR_STATE: &str = "Missing code or state parameter.";
pub const ERR_STATE_MISMATCH: &str = "State mismatch.";
pub const ERR_OAUTH_STATE_MISMATCH: &str = "OAuth state mismatch.";
pub const ERR_MISSING_AUTHORIZATION_CODE: &str = "Missing authorization code.";
pub const ERR_CALLBACK_ALREADY_USED: &str = "This OAuth callback has already been used.";
pub const ERR_INTERNAL_PLAIN: &str = "Internal error";
pub const ERR_INTERNAL_HTML: &str = "Internal error while processing OAuth callback.";
pub const ERR_ANTHROPIC_INCOMPLETE: &str = "Anthropic authentication did not complete.";
pub const ERR_OPENROUTER_DENIED: &str = "OpenRouter authorization was denied.";
pub const ERR_OPENROUTER_NO_CODE: &str = "OpenRouter returned no authorization code.";

pub const MSG_ANTHROPIC_SUCCESS: &str =
    "Anthropic authentication completed. You can close this window.";
pub const MSG_CODEX_SUCCESS: &str = "OpenAI authentication completed. You can close this window.";
pub const MSG_OPENROUTER_SUCCESS: &str = "Signed in to OpenRouter. You may now close this page.";
pub const MSG_RADIUS_SUCCESS: &str = "Signed in to Radius. You may now close this page.";

pub const ANTHROPIC_CALLBACK_PORT: u16 = 53692;
pub const ANTHROPIC_CALLBACK_PATH: &str = "/callback";
pub const CODEX_CALLBACK_PORT: u16 = 1455;
pub const CODEX_CALLBACK_PATH: &str = "/auth/callback";
pub const RADIUS_CALLBACK_PORT: u16 = 1456;
pub const RADIUS_CALLBACK_PATH: &str = "/oauth/callback";
pub const OPENROUTER_CALLBACK_PORT: u16 = 8080;
pub const OPENROUTER_CALLBACK_PATH: &str = "/callback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackProvider {
    Anthropic,
    OpenAiCodex,
    OpenRouter,
    Radius,
}

impl CallbackProvider {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "anthropic" => Some(Self::Anthropic),
            "openai-codex" => Some(Self::OpenAiCodex),
            "openrouter" => Some(Self::OpenRouter),
            "radius" => Some(Self::Radius),
            _ => None,
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Self::Anthropic => ANTHROPIC_CALLBACK_PATH,
            Self::OpenAiCodex => CODEX_CALLBACK_PATH,
            Self::OpenRouter => OPENROUTER_CALLBACK_PATH,
            Self::Radius => RADIUS_CALLBACK_PATH,
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Anthropic => ANTHROPIC_CALLBACK_PORT,
            Self::OpenAiCodex => CODEX_CALLBACK_PORT,
            Self::OpenRouter => OPENROUTER_CALLBACK_PORT,
            Self::Radius => RADIUS_CALLBACK_PORT,
        }
    }

    pub fn success_message(self) -> &'static str {
        match self {
            Self::Anthropic => MSG_ANTHROPIC_SUCCESS,
            Self::OpenAiCodex => MSG_CODEX_SUCCESS,
            Self::OpenRouter => MSG_OPENROUTER_SUCCESS,
            Self::Radius => MSG_RADIUS_SUCCESS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub code: Option<String>,
    pub state: Option<String>,
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_page(title: &str, heading: &str, message: &str, details: Option<&str>) -> String {
    let title = escape_html(title);
    let heading = escape_html(heading);
    let message = escape_html(message);
    let details_html = details
        .map(|value| format!("<div class=\"details\">{}</div>", escape_html(value)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{title}</title>
  <style>
    :root {{
      --text: #fafafa;
      --text-dim: #a1a1aa;
      --page-bg: #09090b;
      --font-sans: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, "Noto Sans", sans-serif, "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji";
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    }}
    * {{ box-sizing: border-box; }}
    html {{ color-scheme: dark; }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 24px;
      background: var(--page-bg);
      color: var(--text);
      font-family: var(--font-sans);
      text-align: center;
    }}
    main {{
      width: 100%;
      max-width: 560px;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
    }}
    .logo {{
      width: 72px;
      height: 72px;
      display: block;
      margin-bottom: 24px;
    }}
    h1 {{
      margin: 0 0 10px;
      font-size: 28px;
      line-height: 1.15;
      font-weight: 650;
      color: var(--text);
    }}
    p {{
      margin: 0;
      line-height: 1.7;
      color: var(--text-dim);
      font-size: 15px;
    }}
    .details {{
      margin-top: 16px;
      font-family: var(--font-mono);
      font-size: 13px;
      color: var(--text-dim);
      white-space: pre-wrap;
      word-break: break-word;
    }}
  </style>
</head>
<body>
  <main>
    <div class="logo">{LOGO_SVG}</div>
    <h1>{heading}</h1>
    <p>{message}</p>
    {details_html}
  </main>
</body>
</html>"#
    )
}

pub fn oauth_success_html(message: &str) -> String {
    render_page(TITLE_SUCCESS, TITLE_SUCCESS, message, None)
}

pub fn oauth_error_html(message: &str, details: Option<&str>) -> String {
    render_page(TITLE_FAILED, TITLE_FAILED, message, details)
}

fn html_response(status: u16, body: String) -> CallbackResponse {
    CallbackResponse {
        status,
        content_type: CONTENT_TYPE_HTML.into(),
        body,
        code: None,
        state: None,
    }
}

fn query_param(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .filter(|value| !value.is_empty())
}

/// Handle a GET pathname+query the way each TS `createServer` callback does.
pub fn handle_callback_request(
    provider: CallbackProvider,
    pathname: &str,
    query: &str,
    expected_state: &str,
    already_used: bool,
) -> CallbackResponse {
    match provider {
        CallbackProvider::Anthropic => handle_anthropic(pathname, query, expected_state),
        CallbackProvider::OpenAiCodex => handle_codex(pathname, query, expected_state),
        CallbackProvider::OpenRouter => handle_openrouter(pathname, query, already_used),
        CallbackProvider::Radius => handle_radius(pathname, query, expected_state),
    }
}

fn handle_anthropic(pathname: &str, query: &str, expected_state: &str) -> CallbackResponse {
    if pathname != ANTHROPIC_CALLBACK_PATH {
        return html_response(404, oauth_error_html(ERR_CALLBACK_ROUTE_NOT_FOUND, None));
    }
    if let Some(error) = query_param(query, "error") {
        return html_response(
            400,
            oauth_error_html(ERR_ANTHROPIC_INCOMPLETE, Some(&format!("Error: {error}"))),
        );
    }
    let code = query_param(query, "code");
    let state = query_param(query, "state");
    if code.is_none() || state.is_none() {
        return html_response(400, oauth_error_html(ERR_MISSING_CODE_OR_STATE, None));
    }
    if state.as_deref() != Some(expected_state) {
        return html_response(400, oauth_error_html(ERR_STATE_MISMATCH, None));
    }
    let mut response = html_response(200, oauth_success_html(MSG_ANTHROPIC_SUCCESS));
    response.code = code;
    response.state = state;
    response
}

fn handle_codex(pathname: &str, query: &str, expected_state: &str) -> CallbackResponse {
    if pathname != CODEX_CALLBACK_PATH {
        return html_response(404, oauth_error_html(ERR_CALLBACK_ROUTE_NOT_FOUND, None));
    }
    if query_param(query, "state").as_deref() != Some(expected_state) {
        return html_response(400, oauth_error_html(ERR_STATE_MISMATCH, None));
    }
    let Some(code) = query_param(query, "code") else {
        return html_response(400, oauth_error_html(ERR_MISSING_AUTHORIZATION_CODE, None));
    };
    let mut response = html_response(200, oauth_success_html(MSG_CODEX_SUCCESS));
    response.code = Some(code);
    response.state = Some(expected_state.into());
    response
}

fn handle_openrouter(pathname: &str, query: &str, already_used: bool) -> CallbackResponse {
    if pathname != OPENROUTER_CALLBACK_PATH {
        return html_response(
            404,
            oauth_error_html(ERR_OAUTH_CALLBACK_ROUTE_NOT_FOUND, None),
        );
    }
    if already_used {
        return html_response(409, oauth_error_html(ERR_CALLBACK_ALREADY_USED, None));
    }
    if let Some(error) = query_param(query, "error") {
        let description = query_param(query, "error_description").unwrap_or(error);
        return html_response(
            400,
            oauth_error_html(ERR_OPENROUTER_DENIED, Some(&description)),
        );
    }
    let Some(code) = query_param(query, "code") else {
        return html_response(400, oauth_error_html(ERR_OPENROUTER_NO_CODE, None));
    };
    let mut response = html_response(200, oauth_success_html(MSG_OPENROUTER_SUCCESS));
    response.code = Some(code);
    response
}

fn handle_radius(pathname: &str, query: &str, expected_state: &str) -> CallbackResponse {
    if pathname != RADIUS_CALLBACK_PATH {
        return html_response(404, oauth_error_html(ERR_CALLBACK_ROUTE_NOT_FOUND, None));
    }
    if query_param(query, "state").as_deref() != Some(expected_state) {
        return html_response(400, oauth_error_html(ERR_OAUTH_STATE_MISMATCH, None));
    }
    if let Some(error) = query_param(query, "error") {
        let message = query_param(query, "error_description").unwrap_or(error);
        return html_response(400, oauth_error_html(&message, None));
    }
    let Some(code) = query_param(query, "code") else {
        return html_response(400, oauth_error_html(ERR_MISSING_AUTHORIZATION_CODE, None));
    };
    let mut response = html_response(200, oauth_success_html(MSG_RADIUS_SUCCESS));
    response.code = Some(code);
    response.state = Some(expected_state.into());
    response
}

pub fn callback_host() -> String {
    std::env::var("PI_OAUTH_CALLBACK_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

/// Bind `127.0.0.1` (or `PI_OAUTH_CALLBACK_HOST`). Port `0` is for tests.
pub struct CallbackServer {
    listener: TcpListener,
    provider: CallbackProvider,
    expected_state: String,
    used: bool,
}

impl CallbackServer {
    pub fn bind(
        host: &str,
        port: u16,
        provider: CallbackProvider,
        expected_state: impl Into<String>,
    ) -> Result<Self, String> {
        let listener =
            TcpListener::bind((host, port)).map_err(|err| format!("OAuth callback bind: {err}"))?;
        listener
            .set_nonblocking(false)
            .map_err(|err| err.to_string())?;
        Ok(Self {
            listener,
            provider,
            expected_state: expected_state.into(),
            used: false,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, String> {
        self.listener.local_addr().map_err(|err| err.to_string())
    }

    pub fn redirect_uri(&self) -> Result<String, String> {
        let addr = self.local_addr()?;
        Ok(format!(
            "http://{}:{}{}",
            addr.ip(),
            addr.port(),
            self.provider.path()
        ))
    }

    pub fn accept_one(&mut self) -> Result<CallbackResponse, String> {
        let (stream, _) = self
            .listener
            .accept()
            .map_err(|err| format!("OAuth callback accept: {err}"))?;
        self.serve(stream)
    }

    fn serve(&mut self, mut stream: TcpStream) -> Result<CallbackResponse, String> {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).map_err(|err| err.to_string())?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let response = match parse_http_target(&request) {
            Some((path, query)) => handle_callback_request(
                self.provider,
                &path,
                &query,
                &self.expected_state,
                self.used,
            ),
            None => CallbackResponse {
                status: 500,
                content_type: CONTENT_TYPE_PLAIN.into(),
                body: ERR_INTERNAL_PLAIN.into(),
                code: None,
                state: None,
            },
        };
        if response.code.is_some() {
            self.used = true;
        }
        write_http_response(&mut stream, &response)?;
        Ok(response)
    }
}

fn parse_http_target(request: &str) -> Option<(String, String)> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" && method != "HEAD" {
        return Some(("/".into(), String::new()));
    }
    let target = parts.next()?;
    if let Some((path, query)) = target.split_once('?') {
        Some((path.to_string(), query.to_string()))
    } else {
        Some((target.to_string(), String::new()))
    }
}

fn write_http_response(stream: &mut TcpStream, response: &CallbackResponse) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason_phrase(response.status),
        response.content_type,
        response.body.len()
    );
    stream
        .write_all(header.as_bytes())
        .map_err(|err| err.to_string())?;
    stream
        .write_all(response.body.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::thread;

    #[test]
    fn anthropic_html_and_error_strings_match_ts() {
        let ok = handle_callback_request(
            CallbackProvider::Anthropic,
            "/callback",
            "code=pi-fixture-1&state=abc",
            "abc",
            false,
        );
        assert_eq!(ok.status, 200);
        assert_eq!(ok.code.as_deref(), Some("pi-fixture-1"));
        assert!(ok.body.contains(&format!("<title>{TITLE_SUCCESS}</title>")));
        assert!(ok.body.contains(MSG_ANTHROPIC_SUCCESS));
        assert!(oauth_error_html(ERR_CALLBACK_ROUTE_NOT_FOUND, None)
            .contains(&format!("<title>{TITLE_FAILED}</title>")));

        let missing =
            handle_callback_request(CallbackProvider::Anthropic, "/callback", "", "abc", false);
        assert_eq!(missing.status, 400);
        assert!(missing.body.contains(ERR_MISSING_CODE_OR_STATE));

        let mismatch = handle_callback_request(
            CallbackProvider::Anthropic,
            "/callback",
            "code=x&state=nope",
            "abc",
            false,
        );
        assert!(mismatch.body.contains(ERR_STATE_MISMATCH));

        let not_found = handle_callback_request(
            CallbackProvider::Anthropic,
            "/other",
            "code=x&state=abc",
            "abc",
            false,
        );
        assert_eq!(not_found.status, 404);
        assert!(not_found.body.contains(ERR_CALLBACK_ROUTE_NOT_FOUND));

        let denied = handle_callback_request(
            CallbackProvider::Anthropic,
            "/callback",
            "error=access_denied",
            "abc",
            false,
        );
        assert!(denied.body.contains(ERR_ANTHROPIC_INCOMPLETE));
        assert!(denied.body.contains("Error: access_denied"));
        assert!(denied.body.contains("&lt;") || !denied.body.contains("<script>"));
    }

    #[test]
    fn codex_openrouter_radius_strings_match_ts() {
        let missing = handle_callback_request(
            CallbackProvider::OpenAiCodex,
            "/auth/callback",
            "state=s",
            "s",
            false,
        );
        assert!(missing.body.contains(ERR_MISSING_AUTHORIZATION_CODE));
        let mismatch = handle_callback_request(
            CallbackProvider::OpenAiCodex,
            "/auth/callback",
            "code=x",
            "s",
            false,
        );
        assert!(mismatch.body.contains(ERR_STATE_MISMATCH));
        let or_path = handle_callback_request(CallbackProvider::OpenRouter, "/nope", "", "", false);
        assert!(or_path.body.contains(ERR_OAUTH_CALLBACK_ROUTE_NOT_FOUND));
        let or_used = handle_callback_request(
            CallbackProvider::OpenRouter,
            "/callback",
            "code=x",
            "",
            true,
        );
        assert_eq!(or_used.status, 409);
        assert!(or_used.body.contains(ERR_CALLBACK_ALREADY_USED));
        let radius = handle_callback_request(
            CallbackProvider::Radius,
            "/oauth/callback",
            "code=x&state=wrong",
            "right",
            false,
        );
        assert!(radius.body.contains(ERR_OAUTH_STATE_MISMATCH));
        let escaped = oauth_error_html(r#"<img src="x">"#, Some("a&b"));
        assert!(escaped.contains("&lt;img src=&quot;x&quot;&gt;"));
        assert!(escaped.contains("a&amp;b"));
    }

    #[test]
    fn localhost_server_accepts_fixture_code() {
        let mut server =
            CallbackServer::bind("127.0.0.1", 0, CallbackProvider::Anthropic, "state-1").unwrap();
        let addr = server.local_addr().unwrap();
        let handle = thread::spawn(move || server.accept_one().unwrap());
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .write_all(
                format!(
                    "GET /callback?code=pi-fixture-loop&state=state-1 HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    addr
                )
                .as_bytes(),
            )
            .unwrap();
        let response = handle.join().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.code.as_deref(), Some("pi-fixture-loop"));
        assert!(response.body.contains(TITLE_SUCCESS));
    }
}
