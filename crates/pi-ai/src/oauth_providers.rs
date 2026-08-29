//! OAuth authorize URLs matching `vendor/pi/packages/ai/src/auth/oauth/*`.

use sha2::{Digest, Sha256};

use crate::oauth::DevicePollStatus;

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_REDIRECT: &str = "http://localhost:53692/callback";
const ANTHROPIC_SCOPES: &str =
    "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_REDIRECT: &str = "http://localhost:1455/auth/callback";
const CODEX_SCOPE: &str = "openid profile email offline_access";

const OPENROUTER_AUTHORIZE_URL: &str = "https://openrouter.ai/auth";
const OPENROUTER_TOKEN_URL: &str = "https://openrouter.ai/api/v1/auth/keys";

const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_OAUTH_HOST: &str = "https://auth.kimi.com";

const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";

const RADIUS_CLIENT_ID: &str = "pi-gateway";
const RADIUS_REDIRECT: &str = "http://127.0.0.1:1456/oauth/callback";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub provider: String,
    pub url: String,
    pub token_url: String,
    pub instructions: String,
    pub pkce: Option<Pkce>,
    pub state: Option<String>,
}

pub fn generate_pkce(verifier_bytes: &[u8]) -> Pkce {
    let verifier = base64url(verifier_bytes);
    let hash = Sha256::digest(verifier.as_bytes());
    Pkce {
        verifier,
        challenge: base64url(&hash),
    }
}

fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 63) as usize] as char);
        }
        i += 3;
    }
    out
}

pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Ok(url) = url::Url::parse(value) {
        let code = url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned());
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned());
        return (code, state);
    }
    if value.contains('#') {
        let (code, state) = value.split_once('#').unwrap();
        return (Some(code.to_string()), Some(state.to_string()));
    }
    if value.contains("code=") {
        let params = url::form_urlencoded::parse(value.as_bytes());
        let mut code = None;
        let mut state = None;
        for (k, v) in params {
            if k == "code" {
                code = Some(v.into_owned());
            } else if k == "state" {
                state = Some(v.into_owned());
            }
        }
        return (code, state);
    }
    (Some(value.to_string()), None)
}

pub fn authorize_request(provider: &str, pkce: &Pkce, state: &str) -> Option<AuthorizeRequest> {
    match provider {
        "anthropic" => {
            let mut url = url::Url::parse(ANTHROPIC_AUTHORIZE_URL).ok()?;
            url.query_pairs_mut()
                .append_pair("code", "true")
                .append_pair("client_id", ANTHROPIC_CLIENT_ID)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", ANTHROPIC_REDIRECT)
                .append_pair("scope", ANTHROPIC_SCOPES)
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &pkce.verifier);
            Some(AuthorizeRequest {
                provider: provider.into(),
                url: url.to_string(),
                token_url: ANTHROPIC_TOKEN_URL.into(),
                instructions: "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here.".into(),
                pkce: Some(pkce.clone()),
                state: Some(pkce.verifier.clone()),
            })
        }
        "openai-codex" => {
            let mut url = url::Url::parse(CODEX_AUTHORIZE_URL).ok()?;
            url.query_pairs_mut()
                .append_pair("response_type", "code")
                .append_pair("client_id", CODEX_CLIENT_ID)
                .append_pair("redirect_uri", CODEX_REDIRECT)
                .append_pair("scope", CODEX_SCOPE)
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", state)
                .append_pair("id_token_add_organizations", "true")
                .append_pair("codex_cli_simplified_flow", "true")
                .append_pair("originator", "pi");
            Some(AuthorizeRequest {
                provider: provider.into(),
                url: url.to_string(),
                token_url: CODEX_TOKEN_URL.into(),
                instructions: "Complete login in your browser, or paste the authorization code / redirect URL here:".into(),
                pkce: Some(pkce.clone()),
                state: Some(state.to_string()),
            })
        }
        "openrouter" => {
            let mut url = url::Url::parse(OPENROUTER_AUTHORIZE_URL).ok()?;
            url.query_pairs_mut()
                .append_pair("callback_url", "http://127.0.0.1:8080/callback")
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", "S256");
            Some(AuthorizeRequest {
                provider: provider.into(),
                url: url.to_string(),
                token_url: OPENROUTER_TOKEN_URL.into(),
                instructions: "Complete OpenRouter login in your browser.".into(),
                pkce: Some(pkce.clone()),
                state: None,
            })
        }
        "xai" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: XAI_DEVICE_CODE_URL.into(),
            token_url: XAI_TOKEN_URL.into(),
            instructions: format!("xAI device code. client_id={XAI_CLIENT_ID} scope={XAI_SCOPE}"),
            pkce: None,
            state: None,
        }),
        "kimi-coding" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: format!("{KIMI_OAUTH_HOST}/oauth/device/code"),
            token_url: format!("{KIMI_OAUTH_HOST}/oauth/token"),
            instructions: format!("Kimi device code. client_id={KIMI_CLIENT_ID}"),
            pkce: None,
            state: None,
        }),
        "github-copilot" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: GITHUB_DEVICE_CODE_URL.into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            instructions: format!("GitHub device code. client_id={GITHUB_CLIENT_ID}"),
            pkce: None,
            state: None,
        }),
        "radius" => {
            let mut url = url::Url::parse("https://radius.example/oauth/authorize").ok()?;
            url.query_pairs_mut()
                .append_pair("client_id", RADIUS_CLIENT_ID)
                .append_pair("redirect_uri", RADIUS_REDIRECT)
                .append_pair("response_type", "code")
                .append_pair("scope", "gateway offline_access")
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", "S256");
            Some(AuthorizeRequest {
                provider: provider.into(),
                url: url.to_string(),
                token_url: "https://radius.example/oauth/token".into(),
                instructions: "Complete Radius gateway login.".into(),
                pkce: Some(pkce.clone()),
                state: Some(state.to_string()),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenExchangeRequest {
    pub url: String,
    pub content_type: String,
    pub body: String,
    pub redirect_uri: String,
}

/// TS token POST bodies: Anthropic JSON, Codex/Radius form-urlencoded, OpenRouter JSON.
pub fn token_exchange_request(
    provider: &str,
    code: &str,
    pkce: Option<&Pkce>,
    state: Option<&str>,
) -> Option<TokenExchangeRequest> {
    let verifier = pkce.map(|p| p.verifier.as_str()).unwrap_or("");
    match provider {
        "anthropic" => {
            let state = state.unwrap_or(verifier);
            let body = serde_json::json!({
                "grant_type": "authorization_code",
                "client_id": ANTHROPIC_CLIENT_ID,
                "code": code,
                "state": state,
                "redirect_uri": ANTHROPIC_REDIRECT,
                "code_verifier": verifier,
            });
            Some(TokenExchangeRequest {
                url: ANTHROPIC_TOKEN_URL.into(),
                content_type: "application/json".into(),
                body: body.to_string(),
                redirect_uri: ANTHROPIC_REDIRECT.into(),
            })
        }
        "openai-codex" => {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "authorization_code")
                .append_pair("client_id", CODEX_CLIENT_ID)
                .append_pair("code", code)
                .append_pair("code_verifier", verifier)
                .append_pair("redirect_uri", CODEX_REDIRECT)
                .finish();
            Some(TokenExchangeRequest {
                url: CODEX_TOKEN_URL.into(),
                content_type: "application/x-www-form-urlencoded".into(),
                body,
                redirect_uri: CODEX_REDIRECT.into(),
            })
        }
        "openrouter" => {
            let body = serde_json::json!({
                "code": code,
                "code_verifier": verifier,
                "code_challenge_method": "S256",
            });
            Some(TokenExchangeRequest {
                url: OPENROUTER_TOKEN_URL.into(),
                content_type: "application/json".into(),
                body: body.to_string(),
                redirect_uri: "http://127.0.0.1:8080/callback".into(),
            })
        }
        "radius" => {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "authorization_code")
                .append_pair("client_id", RADIUS_CLIENT_ID)
                .append_pair("redirect_uri", RADIUS_REDIRECT)
                .append_pair("code", code)
                .append_pair("code_verifier", verifier)
                .finish();
            Some(TokenExchangeRequest {
                url: "https://radius.example/oauth/token".into(),
                content_type: "application/x-www-form-urlencoded".into(),
                body,
                redirect_uri: RADIUS_REDIRECT.into(),
            })
        }
        other => authorize_request(other, &generate_pkce(&[0u8; 32]), state.unwrap_or("pi")).map(
            |auth| TokenExchangeRequest {
                url: auth.token_url,
                content_type: "application/json".into(),
                body: serde_json::json!({
                    "grant_type": "authorization_code",
                    "code": code,
                    "code_verifier": verifier,
                })
                .to_string(),
                redirect_uri: String::new(),
            },
        ),
    }
}

/// Fixture token exchange never hits the network. Live POST uses the TS token URL
/// (overridable with `PI_OAUTH_TOKEN_URL`). Tests use `pi-fixture-` / `PI_OAUTH_FIXTURE`.
pub fn exchange_authorization_code(
    provider: &str,
    code: &str,
    pkce: Option<&Pkce>,
) -> Result<(String, Option<String>), String> {
    if code.starts_with("pi-fixture-") || std::env::var("PI_OAUTH_FIXTURE").is_ok() {
        let access = format!("{provider}-{code}-access");
        let refresh = pkce.map(|p| format!("pi-fixture-{}", p.verifier));
        return Ok((access, refresh));
    }
    let request =
        token_exchange_request(provider, code, pkce, pkce.map(|p| p.verifier.as_str()))
            .ok_or_else(|| format!("OAuth token exchange is not configured for {provider}"))?;
    post_token_exchange(&request)
}

fn post_token_exchange(request: &TokenExchangeRequest) -> Result<(String, Option<String>), String> {
    let url = std::env::var("PI_OAUTH_TOKEN_URL").unwrap_or_else(|_| request.url.clone());
    let response = ureq::post(&url)
        .set("content-type", &request.content_type)
        .set("accept", "application/json")
        .send_string(&request.body)
        .map_err(|err| {
            format!(
                "Token exchange request failed. url={url}; redirect_uri={}; response_type=authorization_code; details={}",
                request.redirect_uri,
                err
            )
        })?;
    let body = response.into_string().map_err(|err| {
        format!(
            "Token exchange request failed. url={url}; redirect_uri={}; response_type=authorization_code; details={}",
            request.redirect_uri,
            err
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|err| {
        format!("Token exchange returned invalid JSON. url={url}; body={body}; details={err}")
    })?;
    let access = value
        .get("access_token")
        .or_else(|| value.get("access"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!("Token exchange returned invalid JSON. url={url}; body={body}; details=missing access_token")
        })?;
    let refresh = value
        .get("refresh_token")
        .or_else(|| value.get("refresh"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok((access.to_string(), refresh))
}

pub fn oauth_providers() -> &'static [&'static str] {
    &[
        "anthropic",
        "openai-codex",
        "openrouter",
        "xai",
        "kimi-coding",
        "github-copilot",
        "radius",
    ]
}

pub fn device_status_from_error(error: &str) -> DevicePollStatus<()> {
    match error {
        "authorization_pending" => DevicePollStatus::Pending,
        "slow_down" => DevicePollStatus::SlowDown {
            interval_seconds: None,
        },
        "expired_token" => DevicePollStatus::Expired,
        _ => DevicePollStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_and_codex_authorize_urls_match_ts() {
        let pkce = generate_pkce(&[1u8; 32]);
        let anthropic = authorize_request("anthropic", &pkce, "state").unwrap();
        assert!(anthropic.url.starts_with(ANTHROPIC_AUTHORIZE_URL));
        assert!(anthropic.url.contains("code_challenge_method=S256"));
        assert!(anthropic.url.contains(ANTHROPIC_CLIENT_ID));
        assert_eq!(anthropic.token_url, ANTHROPIC_TOKEN_URL);
        assert!(anthropic
            .instructions
            .contains("paste the final redirect URL"));
        let codex = authorize_request("openai-codex", &pkce, "abc").unwrap();
        assert!(codex.url.contains("codex_cli_simplified_flow=true"));
        assert!(codex.url.contains(CODEX_CLIENT_ID));
        let (code, state) =
            parse_authorization_input("http://localhost:53692/callback?code=xyz&state=s1");
        assert_eq!(code.as_deref(), Some("xyz"));
        assert_eq!(state.as_deref(), Some("s1"));
        let (hash_code, hash_state) = parse_authorization_input("tok#st");
        assert_eq!(hash_code.as_deref(), Some("tok"));
        assert_eq!(hash_state.as_deref(), Some("st"));
        for provider in oauth_providers() {
            assert!(
                authorize_request(provider, &pkce, "s").is_some(),
                "{provider}"
            );
        }
        let (access, _) =
            exchange_authorization_code("anthropic", "pi-fixture-code", Some(&pkce)).unwrap();
        assert!(access.contains("anthropic"));
        let anthropic_token =
            token_exchange_request("anthropic", "abc", Some(&pkce), Some("st")).unwrap();
        assert_eq!(anthropic_token.content_type, "application/json");
        assert!(anthropic_token
            .body
            .contains("\"grant_type\":\"authorization_code\""));
        assert!(anthropic_token.body.contains(ANTHROPIC_CLIENT_ID));
        assert!(anthropic_token.body.contains(ANTHROPIC_REDIRECT));
        let codex_token = token_exchange_request("openai-codex", "abc", Some(&pkce), None).unwrap();
        assert_eq!(
            codex_token.content_type,
            "application/x-www-form-urlencoded"
        );
        assert!(codex_token.body.contains("grant_type=authorization_code"));
        assert!(codex_token.body.contains(CODEX_CLIENT_ID));
        let openrouter_token =
            token_exchange_request("openrouter", "abc", Some(&pkce), None).unwrap();
        assert!(openrouter_token.body.contains("code_challenge_method"));
        let failed = format!(
            "Token exchange request failed. url={}; redirect_uri={}; response_type=authorization_code; details=fixture",
            anthropic_token.url, anthropic_token.redirect_uri
        );
        assert!(failed.contains("response_type=authorization_code"));
        let invalid = format!(
            "Token exchange returned invalid JSON. url={}; body={{}}; details=fixture",
            anthropic_token.url
        );
        assert!(invalid.contains("invalid JSON"));
        assert!(matches!(
            device_status_from_error("authorization_pending"),
            DevicePollStatus::Pending
        ));
        assert!(matches!(
            device_status_from_error("slow_down"),
            DevicePollStatus::SlowDown { .. }
        ));
    }
}
