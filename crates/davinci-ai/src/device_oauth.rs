//! Provider device-code OAuth matching TypeScript `packages/ai/src/auth/oauth`.

use crate::auth::{AuthError, Credential};
use crate::device_code::{
    default_device_clock, poll_oauth_device_code_flow, DevicePollOptions, DevicePollResult,
};
use crate::oauth::parse_token_response;
use serde_json::Value;
use std::collections::VecDeque;

pub const GITHUB_COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const GITHUB_COPILOT_SCOPE: &str = "read:user";
pub const GITHUB_COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
pub const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
pub const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
pub const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const KIMI_DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
pub const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_CODEX_DEVICE_USER_CODE_URL: &str =
    "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const OPENAI_CODEX_DEVICE_TOKEN_URL: &str =
    "https://auth.openai.com/api/accounts/deviceauth/token";
pub const OPENAI_CODEX_DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
pub const OPENAI_CODEX_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
pub const OPENAI_CODEX_DEVICE_TIMEOUT_SECONDS: f64 = 15.0 * 60.0;
pub const RADIUS_OAUTH_CLIENT_ID: &str = "pi-gateway";
pub const RADIUS_OAUTH_SCOPE: &str = "gateway offline_access";
pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const OPENAI_CODEX_DEVICE_LOGIN_METHOD: &str = "device_code";
pub const RADIUS_DEVICE_LOGIN_METHOD: &str = "device-code";

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCodeInfo {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: Option<f64>,
    pub expires_in_seconds: f64,
}

pub fn is_device_oauth_provider(provider: &str) -> bool {
    matches!(provider, "github-copilot" | "xai" | "kimi-coding")
}

pub fn supports_device_oauth(provider: &str) -> bool {
    is_device_oauth_provider(provider) || matches!(provider, "openai-codex" | "openai" | "radius")
}

pub fn oauth_login_label(provider: &str) -> Option<&'static str> {
    match provider {
        "xai" => Some("Sign in with SuperGrok or X Premium"),
        "kimi-coding" => Some("Sign in with Kimi Code"),
        "openrouter" => Some("Sign in with OpenRouter"),
        _ => None,
    }
}

pub fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    url::Url::parse(&value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

pub fn validate_verification_uri(raw: &str, https_only: bool) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|_| untrusted_uri_message(https_only))?;
    let ok = if https_only {
        url.scheme() == "https"
    } else {
        url.scheme() == "https" || url.scheme() == "http"
    };
    if !ok {
        return Err(untrusted_uri_message(https_only));
    }
    Ok(url.as_str().to_string())
}

fn untrusted_uri_message(https_only: bool) -> String {
    if https_only {
        "Untrusted verification URI in xAI OAuth response".into()
    } else {
        "Untrusted verification_uri in device code response".into()
    }
}

pub fn get_base_url_from_copilot_token(token: &str) -> Option<String> {
    let key = "proxy-ep=";
    let start = token.find(key)?;
    let rest = &token[start + key.len()..];
    let host = rest.split(';').next().filter(|h| !h.is_empty())?;
    let api_host = host.replacen("proxy.", "api.", 1);
    Some(format!("https://{api_host}"))
}

pub fn github_copilot_base_url(token: Option<&str>, enterprise_domain: Option<&str>) -> String {
    if let Some(token) = token {
        if let Some(url) = get_base_url_from_copilot_token(token) {
            return url;
        }
    }
    if let Some(domain) = enterprise_domain.filter(|d| !d.is_empty()) {
        return format!("https://copilot-api.{domain}");
    }
    "https://api.individual.githubcopilot.com".into()
}

pub fn kimi_oauth_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .or_else(|_| std::env::var("KIMI_OAUTH_HOST"))
        .ok()
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| KIMI_DEFAULT_OAUTH_HOST.into())
}

fn read_json_file_or_inline(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).map_err(|err| err.to_string());
    }
    let text = std::fs::read_to_string(trimmed).map_err(|err| err.to_string())?;
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

pub fn load_device_fixture() -> Result<Option<Value>, String> {
    match std::env::var("PI_OAUTH_DEVICE_FIXTURE") {
        Ok(value) if !value.trim().is_empty() => read_json_file_or_inline(&value).map(Some),
        _ => Ok(None),
    }
}

pub fn load_device_poll_fixture() -> Result<Option<Vec<Value>>, String> {
    match std::env::var("PI_OAUTH_DEVICE_POLL_FIXTURE") {
        Ok(value) if !value.trim().is_empty() => {
            let parsed = read_json_file_or_inline(&value)?;
            match parsed {
                Value::Array(items) => Ok(Some(items)),
                other => Ok(Some(vec![other])),
            }
        }
        _ => Ok(None),
    }
}

pub fn device_info_from_json(value: &Value, https_only: bool) -> Result<DeviceCodeInfo, String> {
    let device_code = value
        .get("device_code")
        .or_else(|| value.get("deviceCode"))
        .and_then(Value::as_str)
        .or_else(|| value.get("device_auth_id").and_then(Value::as_str))
        .ok_or_else(|| "Invalid device code response fields".to_string())?;
    let user_code = value
        .get("user_code")
        .or_else(|| value.get("userCode"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Invalid device code response fields".to_string())?;
    let verification = value
        .get("verification_uri_complete")
        .or_else(|| value.get("verificationUriComplete"))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("verification_uri")
                .or_else(|| value.get("verificationUri"))
                .and_then(Value::as_str)
        })
        .unwrap_or(OPENAI_CODEX_DEVICE_VERIFICATION_URI);
    let verification_uri = validate_verification_uri(verification, https_only)?;
    let interval = value
        .get("interval")
        .or_else(|| value.get("intervalSeconds"))
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|n| n.is_finite() && *n > 0.0);
    let expires = value
        .get("expires_in")
        .or_else(|| value.get("expiresInSeconds"))
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(OPENAI_CODEX_DEVICE_TIMEOUT_SECONDS);
    Ok(DeviceCodeInfo {
        device_code: device_code.to_string(),
        user_code: user_code.to_string(),
        verification_uri,
        interval_seconds: interval,
        expires_in_seconds: expires,
    })
}

pub fn parse_device_token_poll(value: &Value) -> DevicePollResult<Value> {
    if value.get("access_token").and_then(Value::as_str).is_some()
        || value
            .get("authorization_code")
            .and_then(Value::as_str)
            .is_some()
    {
        return DevicePollResult::Complete(value.clone());
    }
    let error = value.get("error");
    let error_code = match error {
        Some(Value::String(code)) => Some(code.as_str()),
        Some(Value::Object(obj)) => obj.get("code").and_then(Value::as_str),
        _ => None,
    };
    match error_code {
        Some("authorization_pending") | Some("deviceauth_authorization_pending") => {
            DevicePollResult::Pending
        }
        Some("slow_down") => DevicePollResult::SlowDown {
            interval_seconds: value.get("interval").and_then(Value::as_f64),
        },
        Some("access_denied") | Some("authorization_denied") => DevicePollResult::Failed {
            message: if value.to_string().contains("xAI") {
                "xAI device authorization was denied".into()
            } else {
                format!(
                    "Device flow failed: {code}{suffix}",
                    code = error_code.unwrap_or("access_denied"),
                    suffix = value
                        .get("error_description")
                        .and_then(Value::as_str)
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                )
            },
        },
        Some("expired_token") => DevicePollResult::Failed {
            message: "Device flow failed: expired_token".into(),
        },
        Some(code) => {
            let suffix = value
                .get("error_description")
                .and_then(Value::as_str)
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            DevicePollResult::Failed {
                message: format!("Device flow failed: {code}{suffix}"),
            }
        }
        None => DevicePollResult::Failed {
            message: "Invalid device token response".into(),
        },
    }
}

fn credential_from_token_value(value: &Value) -> Result<Credential, AuthError> {
    if value.get("access_token").is_some() {
        return parse_token_response(&value.to_string());
    }
    if let Some(access) = value.get("token").and_then(Value::as_str) {
        let expires = value.get("expires_at").and_then(Value::as_i64);
        return Ok(Credential::Oauth {
            access: access.to_string(),
            refresh: value
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(str::to_string),
            expires,
            extra: value
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "token" | "access_token" | "refresh_token" | "expires_at"
                    )
                })
                .collect(),
        });
    }
    parse_token_response(&value.to_string())
}

fn form_encode(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                crate::oauth::urlencoding_lite(k),
                crate::oauth::urlencoding_lite(v)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<(u16, Value), String> {
    if std::env::var("PI_DISABLE_NETWORK").is_ok() || std::env::var("PI_OFFLINE").is_ok() {
        return Err("OAuth device flow disabled (offline)".into());
    }
    let response = ureq::post(url)
        .set("Accept", "application/json")
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("User-Agent", GITHUB_COPILOT_USER_AGENT)
        .send_string(&form_encode(fields))
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let raw = response.into_string().map_err(|err| err.to_string())?;
    let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
    Ok((status, body))
}

fn post_json(url: &str, body: &Value) -> Result<(u16, Value), String> {
    if std::env::var("PI_DISABLE_NETWORK").is_ok() || std::env::var("PI_OFFLINE").is_ok() {
        return Err("OAuth device flow disabled (offline)".into());
    }
    let response = ureq::post(url)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .send_json(body.clone())
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let raw = response.into_string().map_err(|err| err.to_string())?;
    let parsed = serde_json::from_str(&raw).unwrap_or(Value::Null);
    Ok((status, parsed))
}

fn github_urls(domain: &str) -> (String, String) {
    (
        format!("https://{domain}/login/device/code"),
        format!("https://{domain}/login/oauth/access_token"),
    )
}

fn start_device_live(provider: &str, domain: &str) -> Result<DeviceCodeInfo, String> {
    let https_only = provider == "xai";
    let (status, body) = match provider {
        "github-copilot" => {
            let (device_url, _) = github_urls(domain);
            post_form(
                &device_url,
                &[
                    ("client_id", GITHUB_COPILOT_CLIENT_ID),
                    ("scope", GITHUB_COPILOT_SCOPE),
                ],
            )?
        }
        "xai" => post_form(
            XAI_DEVICE_CODE_URL,
            &[
                ("client_id", XAI_CLIENT_ID),
                ("scope", XAI_SCOPE),
                ("referrer", "pi"),
            ],
        )?,
        "kimi-coding" => post_form(
            &format!("{}/api/oauth/device_authorization", kimi_oauth_host()),
            &[("client_id", KIMI_CLIENT_ID)],
        )?,
        "openai-codex" | "openai" => post_json(
            OPENAI_CODEX_DEVICE_USER_CODE_URL,
            &serde_json::json!({ "client_id": OPENAI_CODEX_CLIENT_ID }),
        )?,
        "radius" => {
            let gateway = crate::normalize_radius_gateway_url(domain);
            post_form(
                &format!("{gateway}/v1/oauth/device"),
                &[
                    ("client_id", RADIUS_OAUTH_CLIENT_ID),
                    ("scope", RADIUS_OAUTH_SCOPE),
                ],
            )?
        }
        other => return Err(format!("No device-code login for {other}")),
    };
    if status == 404 && matches!(provider, "openai-codex" | "openai") {
        return Err(
            "OpenAI Codex device code login is not enabled for this server. Use browser login or verify the server URL."
                .into(),
        );
    }
    if status >= 400 {
        return Err(format!(
            "device authorization failed with status {status}: {body}"
        ));
    }
    let mut info = device_info_from_json(&body, https_only)?;
    if matches!(provider, "openai-codex" | "openai") {
        info.verification_uri = OPENAI_CODEX_DEVICE_VERIFICATION_URI.into();
        if info.expires_in_seconds <= 0.0 {
            info.expires_in_seconds = OPENAI_CODEX_DEVICE_TIMEOUT_SECONDS;
        }
    }
    Ok(info)
}

fn poll_live_once(provider: &str, domain: &str, device: &DeviceCodeInfo) -> Result<Value, String> {
    let (status, body) = match provider {
        "github-copilot" => {
            let (_, token_url) = github_urls(domain);
            post_form(
                &token_url,
                &[
                    ("client_id", GITHUB_COPILOT_CLIENT_ID),
                    ("device_code", &device.device_code),
                    ("grant_type", DEVICE_CODE_GRANT),
                ],
            )?
        }
        "xai" => post_form(
            XAI_TOKEN_URL,
            &[
                ("grant_type", DEVICE_CODE_GRANT),
                ("client_id", XAI_CLIENT_ID),
                ("device_code", &device.device_code),
            ],
        )?,
        "kimi-coding" => post_form(
            &format!("{}/api/oauth/token", kimi_oauth_host()),
            &[
                ("client_id", KIMI_CLIENT_ID),
                ("device_code", &device.device_code),
                ("grant_type", DEVICE_CODE_GRANT),
            ],
        )?,
        "openai-codex" | "openai" => post_json(
            OPENAI_CODEX_DEVICE_TOKEN_URL,
            &serde_json::json!({
                "device_auth_id": device.device_code,
                "user_code": device.user_code,
            }),
        )?,
        "radius" => {
            let gateway = crate::normalize_radius_gateway_url(domain);
            post_form(
                &format!("{gateway}/v1/oauth/token"),
                &[
                    ("grant_type", DEVICE_CODE_GRANT),
                    ("client_id", RADIUS_OAUTH_CLIENT_ID),
                    ("device_code", &device.device_code),
                ],
            )?
        }
        other => return Err(format!("No device-code login for {other}")),
    };
    if matches!(provider, "openai-codex" | "openai") && (status == 403 || status == 404) {
        return Ok(serde_json::json!({"error":"authorization_pending"}));
    }
    if !body.is_null() {
        return Ok(body);
    }
    Err(format!("Invalid device token response (HTTP {status})"))
}

pub fn start_device_authorization(
    provider: &str,
    enterprise_or_gateway: Option<&str>,
) -> Result<DeviceCodeInfo, String> {
    if let Some(fixture) = load_device_fixture()? {
        let https_only = provider == "xai";
        return device_info_from_json(&fixture, https_only);
    }
    let domain = match provider {
        "github-copilot" => enterprise_or_gateway
            .and_then(normalize_domain)
            .unwrap_or_else(|| "github.com".into()),
        "radius" => enterprise_or_gateway
            .map(crate::normalize_radius_gateway_url)
            .unwrap_or_else(|| crate::DEFAULT_RADIUS_GATEWAY.into()),
        _ => enterprise_or_gateway.unwrap_or("").to_string(),
    };
    start_device_live(provider, &domain)
}

pub fn poll_device_authorization(
    provider: &str,
    device: &DeviceCodeInfo,
    enterprise_or_gateway: Option<&str>,
) -> Result<Credential, String> {
    if let Ok(path) = std::env::var("PI_OAUTH_TOKEN_FIXTURE") {
        if !path.trim().is_empty() {
            let raw = if path.trim().starts_with('{') {
                path
            } else {
                std::fs::read_to_string(path.trim()).map_err(|err| err.to_string())?
            };
            return parse_token_response(&raw).map_err(|err| err.to_string());
        }
    }
    let mut queued: VecDeque<Value> = load_device_poll_fixture()?.unwrap_or_default().into();
    let domain = match provider {
        "github-copilot" => enterprise_or_gateway
            .and_then(normalize_domain)
            .unwrap_or_else(|| "github.com".into()),
        "radius" => enterprise_or_gateway
            .map(crate::normalize_radius_gateway_url)
            .unwrap_or_else(|| crate::DEFAULT_RADIUS_GATEWAY.into()),
        _ => enterprise_or_gateway.unwrap_or("").to_string(),
    };
    let wait_before = is_device_oauth_provider(provider);
    let clock = default_device_clock();
    let value = poll_oauth_device_code_flow(
        DevicePollOptions {
            interval_seconds: device.interval_seconds,
            expires_in_seconds: Some(device.expires_in_seconds),
            wait_before_first_poll: wait_before,
        },
        clock.as_ref(),
        || {
            if let Some(next) = queued.pop_front() {
                return Ok(parse_device_token_poll(&next));
            }
            if !queued.is_empty() || load_device_poll_fixture()?.is_some() {
                return Ok(DevicePollResult::Failed {
                    message: "Unexpected extra poll".into(),
                });
            }
            let body = poll_live_once(provider, &domain, device)?;
            Ok(parse_device_token_poll(&body))
        },
    )?;
    if matches!(provider, "openai-codex" | "openai") {
        if let (Some(code), Some(verifier)) = (
            value.get("authorization_code").and_then(Value::as_str),
            value.get("code_verifier").and_then(Value::as_str),
        ) {
            let exchanged = crate::oauth_flow::exchange_authorization_code(
                "https://auth.openai.com/oauth/token",
                &serde_json::json!({
                    "grant_type": "authorization_code",
                    "client_id": OPENAI_CODEX_CLIENT_ID,
                    "code": code,
                    "code_verifier": verifier,
                    "redirect_uri": OPENAI_CODEX_DEVICE_REDIRECT_URI,
                }),
            )
            .map_err(|err| err.to_string())?;
            return Ok(exchanged);
        }
    }
    credential_from_token_value(&value).map_err(|err| err.to_string())
}

pub fn login_device_oauth(
    provider: &str,
    enterprise_or_gateway: Option<&str>,
) -> Result<(Credential, DeviceCodeInfo), String> {
    let device = start_device_authorization(provider, enterprise_or_gateway)?;
    let credential = poll_device_authorization(provider, &device, enterprise_or_gateway)?;
    Ok((credential, device))
}

pub fn prefers_device_login(provider: &str, method: Option<&str>) -> bool {
    if let Some(method) = method {
        let needle = method.trim().to_ascii_lowercase();
        if needle == OPENAI_CODEX_DEVICE_LOGIN_METHOD
            || needle == RADIUS_DEVICE_LOGIN_METHOD
            || needle == "device"
            || needle == "headless"
        {
            return true;
        }
        if needle == "browser" || needle.contains("browser login") {
            return false;
        }
    } else if let Ok(env_method) = std::env::var("PI_OAUTH_LOGIN_METHOD") {
        return prefers_device_login(provider, Some(&env_method));
    }
    if is_device_oauth_provider(provider) {
        return true;
    }
    load_device_fixture().ok().flatten().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_fixture_and_github_proxy_ep() {
        let info = device_info_from_json(
            &serde_json::json!({
                "device_code": "dev",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 5
            }),
            false,
        )
        .unwrap();
        assert_eq!(info.user_code, "ABCD-1234");
        assert_eq!(info.verification_uri, "https://github.com/login/device");
        assert_eq!(
            get_base_url_from_copilot_token(
                "tid=1;exp=2;proxy-ep=proxy.individual.githubcopilot.com;x=1"
            )
            .as_deref(),
            Some("https://api.individual.githubcopilot.com")
        );
        assert_eq!(
            github_copilot_base_url(None, None),
            "https://api.individual.githubcopilot.com"
        );
        assert_eq!(
            validate_verification_uri("https://auth.x.ai/device", true).unwrap(),
            "https://auth.x.ai/device"
        );
        assert!(validate_verification_uri("http://evil.example/x", true).is_err());
        assert_eq!(
            normalize_domain("company.ghe.com").as_deref(),
            Some("company.ghe.com")
        );
        assert!(oauth_login_label("xai").unwrap().contains("SuperGrok"));
    }

    #[test]
    fn poll_parser_matches_typescript_status_codes() {
        assert!(matches!(
            parse_device_token_poll(&serde_json::json!({"error":"authorization_pending"})),
            DevicePollResult::Pending
        ));
        assert!(matches!(
            parse_device_token_poll(&serde_json::json!({"error":"slow_down","interval":8})),
            DevicePollResult::SlowDown {
                interval_seconds: Some(8.0)
            }
        ));
        match parse_device_token_poll(
            &serde_json::json!({"error":"access_denied","error_description":"nope"}),
        ) {
            DevicePollResult::Failed { message } => {
                assert!(message.contains("Device flow failed: access_denied: nope"));
            }
            other => panic!("{other:?}"),
        }
        match parse_device_token_poll(&serde_json::json!({"access_token":"tok"})) {
            DevicePollResult::Complete(value) => {
                assert_eq!(value["access_token"], "tok");
            }
            other => panic!("{other:?}"),
        }
    }
}
