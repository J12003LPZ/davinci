//! RFC 8628 device-authorization login for xAI, Kimi Coding and GitHub
//! Copilot, matching `vendor/davinci/packages/ai/src/auth/oauth/{xai,
//! kimi-coding,github-copilot,device-code}.ts`: request a device code, show
//! the user code and verification URI, poll the token endpoint with the
//! device-code grant, and (Copilot) trade the GitHub token for a Copilot one.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::oauth_providers::{OauthTokens, TokenExchangeRequest};

pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_OAUTH_HOST: &str = "https://auth.kimi.com";
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// RFC 8628 3.2: without `interval` the client polls every 5 seconds.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
const MINIMUM_INTERVAL: Duration = Duration::from_secs(1);
/// RFC 8628 3.5: `slow_down` adds 5 seconds.
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// Where and how one provider runs the device flow.
#[derive(Debug, Clone)]
pub(crate) struct DeviceEndpoints {
    pub device_url: String,
    pub token_url: String,
    pub client_id: &'static str,
    pub scope: Option<&'static str>,
    pub extra: &'static [(&'static str, &'static str)],
    pub user_agent: Option<&'static str>,
    pub wait_before_first_poll: bool,
    /// Copilot: the device flow yields a GitHub token that is then traded
    /// for the short-lived Copilot token at this URL.
    pub copilot_token_url: Option<String>,
}

/// A started device login, to show the user and then poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub provider: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Option<Duration>,
    pub expires_in: Duration,
}

pub fn is_device_code_provider(provider: &str) -> bool {
    endpoints(provider).is_some()
}

fn kimi_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("KIMI_OAUTH_HOST")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| KIMI_OAUTH_HOST.into())
        .trim_end_matches('/')
        .to_string()
}

pub(crate) fn endpoints(provider: &str) -> Option<DeviceEndpoints> {
    match provider {
        "xai" => Some(DeviceEndpoints {
            device_url: "https://auth.x.ai/oauth2/device/code".into(),
            token_url: "https://auth.x.ai/oauth2/token".into(),
            client_id: XAI_CLIENT_ID,
            scope: Some(XAI_SCOPE),
            extra: &[("referrer", "pi")],
            user_agent: None,
            wait_before_first_poll: false,
            copilot_token_url: None,
        }),
        "kimi-coding" => {
            let host = kimi_host();
            Some(DeviceEndpoints {
                device_url: format!("{host}/api/oauth/device_authorization"),
                token_url: format!("{host}/api/oauth/token"),
                client_id: KIMI_CLIENT_ID,
                scope: None,
                extra: &[],
                user_agent: None,
                wait_before_first_poll: false,
                copilot_token_url: None,
            })
        }
        "github-copilot" => Some(DeviceEndpoints {
            device_url: "https://github.com/login/device/code".into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            client_id: GITHUB_CLIENT_ID,
            scope: Some("read:user"),
            extra: &[],
            user_agent: Some(crate::auth::COPILOT_USER_AGENT),
            wait_before_first_poll: true,
            copilot_token_url: Some(COPILOT_TOKEN_URL.into()),
        }),
        _ => None,
    }
}

/// The token request that redeems a device code (`grant_type=device_code`).
pub fn device_token_request(provider: &str, device_code: &str) -> Option<TokenExchangeRequest> {
    let endpoints = endpoints(provider)?;
    Some(TokenExchangeRequest {
        url: endpoints.token_url.clone(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", DEVICE_CODE_GRANT)
            .append_pair("client_id", endpoints.client_id)
            .append_pair("device_code", device_code)
            .finish(),
        redirect_uri: String::new(),
    })
}

fn post_form(
    url: &str,
    form: &[(&str, &str)],
    user_agent: Option<&str>,
) -> Result<(u16, Value), String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let mut request = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .post(url)
        .set("content-type", "application/x-www-form-urlencoded")
        .set("accept", "application/json");
    if let Some(user_agent) = user_agent {
        request = request.set("user-agent", user_agent);
    }
    // Device endpoints report pending/slow_down as 4xx JSON bodies.
    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(format!("Device login request to {url} failed: {error}")),
    };
    let status = response.status();
    let text = response
        .into_string()
        .map_err(|error| format!("Device login response from {url} unreadable: {error}"))?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn trusted_uri(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value)
        .map_err(|_| "Untrusted verification URI in device authorization response".to_string())?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("Untrusted verification URI in device authorization response".into());
    }
    Ok(parsed.to_string())
}

fn positive_seconds(value: Option<&Value>) -> Option<Duration> {
    value
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Duration::from_secs_f64)
}

pub fn start_device_authorization(provider: &str) -> Result<DeviceAuthorization, String> {
    let endpoints =
        endpoints(provider).ok_or_else(|| format!("{provider} does not use device-code login"))?;
    start_with(provider, &endpoints)
}

pub(crate) fn start_with(
    provider: &str,
    endpoints: &DeviceEndpoints,
) -> Result<DeviceAuthorization, String> {
    let mut form = vec![("client_id", endpoints.client_id)];
    if let Some(scope) = endpoints.scope {
        form.push(("scope", scope));
    }
    form.extend_from_slice(endpoints.extra);
    let (status, body) = post_form(&endpoints.device_url, &form, endpoints.user_agent)?;
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let (Some(device_code), Some(user_code), Some(verification_uri)) = (
        field("device_code"),
        field("user_code"),
        field("verification_uri"),
    ) else {
        return Err(format!(
            "{provider} device authorization failed (status {status}): {}",
            oauth_error(&body)
        ));
    };
    let verification_uri = match field("verification_uri_complete") {
        Some(complete) => trusted_uri(complete)?,
        None => trusted_uri(verification_uri)?,
    };
    Ok(DeviceAuthorization {
        provider: provider.into(),
        device_code: device_code.into(),
        user_code: user_code.into(),
        verification_uri,
        interval: positive_seconds(body.get("interval")),
        expires_in: positive_seconds(body.get("expires_in")).unwrap_or(Duration::from_secs(900)),
    })
}

/// OAuth `error`/`error_description` only; never token values.
fn oauth_error(body: &Value) -> String {
    let text = |key: &str| body.get(key).and_then(Value::as_str).unwrap_or("");
    match (text("error"), text("error_description")) {
        ("", _) => "unexpected response".into(),
        (error, "") => error.into(),
        (error, description) => format!("{error}: {description}"),
    }
}

fn tokens_from(body: &Value) -> Option<OauthTokens> {
    let access = body.get("access_token").and_then(Value::as_str)?;
    Some(OauthTokens {
        access: access.into(),
        refresh: body
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires: body
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| {
                crate::models_store::now_ms().saturating_add(seconds.saturating_mul(1000))
            }),
    })
}

/// Poll until the user approves, the code expires, or `cancelled` says stop.
pub fn poll_device_authorization(
    device: &DeviceAuthorization,
    cancelled: &dyn Fn() -> bool,
) -> Result<OauthTokens, String> {
    let endpoints = endpoints(&device.provider)
        .ok_or_else(|| format!("{} does not use device-code login", device.provider))?;
    poll_with(device, &endpoints, cancelled)
}

pub(crate) fn poll_with(
    device: &DeviceAuthorization,
    endpoints: &DeviceEndpoints,
    cancelled: &dyn Fn() -> bool,
) -> Result<OauthTokens, String> {
    let deadline = Instant::now() + device.expires_in;
    let mut interval = device
        .interval
        .unwrap_or(DEFAULT_INTERVAL)
        .max(MINIMUM_INTERVAL);
    let mut slowed = false;
    let sleep = |duration: Duration| -> Result<(), String> {
        let until = (Instant::now() + duration).min(deadline);
        while Instant::now() < until {
            if cancelled() {
                return Err("Login cancelled".into());
            }
            std::thread::sleep((until - Instant::now()).min(Duration::from_millis(100)));
        }
        Ok(())
    };
    if endpoints.wait_before_first_poll {
        sleep(interval)?;
    }
    while Instant::now() < deadline {
        if cancelled() {
            return Err("Login cancelled".into());
        }
        let (_, body) = post_form(
            &endpoints.token_url,
            &[
                ("grant_type", DEVICE_CODE_GRANT),
                ("client_id", endpoints.client_id),
                ("device_code", &device.device_code),
            ],
            endpoints.user_agent,
        )?;
        if let Some(tokens) = tokens_from(&body) {
            return match &endpoints.copilot_token_url {
                Some(url) => copilot_token_at(url, &tokens.access),
                None => Ok(tokens),
            };
        }
        match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                slowed = true;
                interval = positive_seconds(body.get("interval"))
                    .unwrap_or(interval + SLOW_DOWN_INCREMENT)
                    .max(MINIMUM_INTERVAL);
            }
            Some("access_denied") => {
                return Err(format!(
                    "{} device authorization was denied",
                    device.provider
                ))
            }
            Some("expired_token") => {
                return Err(format!(
                    "{} device code expired; start login again",
                    device.provider
                ))
            }
            _ => {
                return Err(format!(
                    "{} device token polling failed: {}",
                    device.provider,
                    oauth_error(&body)
                ))
            }
        }
        sleep(interval)?;
    }
    Err(if slowed {
        "Device flow timed out after slow_down responses; check the system clock and try again."
            .into()
    } else {
        "Device flow timed out".into()
    })
}

/// Trade a GitHub OAuth token for the short-lived Copilot token. The GitHub
/// token is kept as the refresh credential.
pub fn copilot_token(github_token: &str) -> Result<OauthTokens, String> {
    copilot_token_at(COPILOT_TOKEN_URL, github_token)
}

pub(crate) fn copilot_token_at(url: &str, github_token: &str) -> Result<OauthTokens, String> {
    let mut request = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .get(url)
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {github_token}"));
    for (name, value) in crate::auth::copilot_headers() {
        request = request.set(name, value);
    }
    let response = request
        .call()
        .map_err(|error| format!("Copilot token request failed: {error}"))?;
    let body: Value = serde_json::from_str(
        &response
            .into_string()
            .map_err(|error| format!("Copilot token response unreadable: {error}"))?,
    )
    .map_err(|_| "Invalid Copilot token response".to_string())?;
    let (Some(token), Some(expires_at)) = (
        body.get("token").and_then(Value::as_str),
        body.get("expires_at").and_then(Value::as_u64),
    ) else {
        return Err("Invalid Copilot token response fields".into());
    };
    Ok(OauthTokens {
        access: token.into(),
        refresh: Some(github_token.into()),
        // Renew five minutes early, as the TS provider does.
        expires: Some(
            expires_at
                .saturating_mul(1000)
                .saturating_sub(5 * 60 * 1000),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// Serves one canned JSON reply per request, recording each request body.
    fn server(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, reply) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(format!(
                    "{}{}",
                    request_line.trim(),
                    String::from_utf8(body).unwrap()
                ));
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .unwrap();
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn device_providers_redeem_with_the_device_code_grant() {
        for provider in ["xai", "kimi-coding", "github-copilot"] {
            assert!(is_device_code_provider(provider));
            let request = device_token_request(provider, "dev-123").unwrap();
            let pairs: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(request.body.as_bytes())
                    .into_owned()
                    .collect();
            assert_eq!(pairs["grant_type"], DEVICE_CODE_GRANT, "{provider}");
            assert_eq!(pairs["device_code"], "dev-123", "{provider}");
        }
        assert!(!is_device_code_provider("openrouter"));
    }

    #[test]
    fn device_flow_shows_code_polls_and_returns_tokens() {
        let (base, server) = server(vec![
            (
                200,
                r#"{"device_code":"dev","user_code":"ABCD-1234","verification_uri":"https://example.test/device","interval":1,"expires_in":30}"#,
            ),
            (400, r#"{"error":"authorization_pending"}"#),
            (
                200,
                r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#,
            ),
        ]);
        let endpoints = DeviceEndpoints {
            device_url: format!("{base}/device"),
            token_url: format!("{base}/token"),
            client_id: "client",
            scope: Some("scope"),
            extra: &[],
            user_agent: None,
            wait_before_first_poll: false,
            copilot_token_url: None,
        };
        let device = start_with("xai", &endpoints).unwrap();
        assert_eq!(device.user_code, "ABCD-1234");
        assert_eq!(device.verification_uri, "https://example.test/device");
        let tokens = poll_with(&device, &endpoints, &|| false).unwrap();
        assert_eq!(tokens.access, "at");
        assert_eq!(tokens.refresh.as_deref(), Some("rt"));
        let seen = server.join().unwrap();
        assert!(seen[1].contains("device_code"), "{seen:?}");
        assert!(seen[1].contains("grant_type=urn%3Aietf"), "{seen:?}");
    }

    #[test]
    fn untrusted_verification_uri_is_rejected() {
        let (base, server) = server(vec![(
            200,
            r#"{"device_code":"dev","user_code":"X","verification_uri":"file:///etc/passwd","expires_in":30}"#,
        )]);
        let endpoints = DeviceEndpoints {
            device_url: format!("{base}/device"),
            token_url: format!("{base}/token"),
            client_id: "client",
            scope: None,
            extra: &[],
            user_agent: None,
            wait_before_first_poll: false,
            copilot_token_url: None,
        };
        assert!(start_with("kimi-coding", &endpoints)
            .unwrap_err()
            .contains("Untrusted"));
        server.join().unwrap();
    }
}
