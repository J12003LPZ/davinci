//! OAuth authorize URLs matching `vendor/pi/packages/ai/src/auth/oauth/*`.

use sha2::{Digest, Sha256};

use crate::oauth::DevicePollStatus;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorizeRequest {
    pub provider: String,
    pub url: String,
    pub token_url: String,
    pub instructions: String,
    pub pkce: Option<Pkce>,
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_siwc: Option<OpenAiSiwcAuthorizeContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenAiSiwcAuthorizeContext {
    pub client_id: String,
    pub redirect_uri: String,
    pub nonce: String,
    pub resource: String,
    pub ext_agent_host_id: String,
    pub expected_subject: Option<String>,
    pub dynamic_registration: bool,
}

pub fn generate_pkce(verifier_bytes: &[u8]) -> Pkce {
    let verifier = base64url(verifier_bytes);
    let hash = Sha256::digest(verifier.as_bytes());
    Pkce {
        verifier,
        challenge: base64url(&hash),
    }
}

/// Generate fresh OAuth transaction material. OpenAI's OSS ChatGPT-plan flow
/// additionally binds a stable host id, OIDC nonce and issued dynamic-client id.
pub fn fresh_authorize_request_checked(provider: &str) -> Result<Option<AuthorizeRequest>, String> {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut verifier_bytes = [0u8; 32];
    verifier_bytes[..16].copy_from_slice(first.as_bytes());
    verifier_bytes[16..].copy_from_slice(second.as_bytes());
    let pkce = generate_pkce(&verifier_bytes);
    let state = uuid::Uuid::new_v4().simple().to_string();

    if provider == "openai-codex" {
        let auth_path = crate::try_default_auth_path().map_err(|err| err.to_string())?;
        let agent_dir = auth_path
            .parent()
            .ok_or("OpenAI sign-in could not resolve the DaVinci agent directory")?;
        let ext_agent_host_id = crate::openai_siwc::load_or_create_host_id(agent_dir)?;
        let stored = crate::AuthStorage::create()
            .ok()
            .and_then(|storage| storage.get(provider).cloned())
            .and_then(|credential| crate::openai_siwc::registration_from_credential(&credential));
        let provisional = crate::openai_siwc::load_issued_client_id(agent_dir);
        let (client_id, expected_subject, dynamic_registration, id_token_hint, login_hint) =
            match stored {
                Some(registration) => (
                    registration.client_id,
                    Some(registration.identity.subject),
                    false,
                    Some(registration.id_token),
                    registration.identity.email,
                ),
                None => match provisional {
                    Some(client_id) => (client_id, None, false, None, None),
                    None => (
                        crate::openai_siwc::OPENAI_SIWC_DYNAMIC_CLIENT_ID.to_string(),
                        None,
                        true,
                        None,
                        None,
                    ),
                },
            };
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let url = crate::openai_siwc::build_authorization_url(
            &client_id,
            &ext_agent_host_id,
            crate::openai_siwc::OPENAI_SIWC_REDIRECT,
            &state,
            &nonce,
            &pkce,
            dynamic_registration,
        )?;
        let mut url = url::Url::parse(&url).map_err(|err| err.to_string())?;
        if !dynamic_registration {
            let mut query = url.query_pairs_mut();
            if let Some(id_token_hint) = id_token_hint.as_deref() {
                query.append_pair("id_token_hint", id_token_hint);
            }
            if let Some(login_hint) = login_hint.as_deref() {
                query.append_pair("login_hint", login_hint);
            }
        }
        return Ok(Some(AuthorizeRequest {
            provider: provider.into(),
            url: url.to_string(),
            token_url: crate::openai_siwc::OPENAI_SIWC_TOKEN_URL.into(),
            instructions: "Continue with ChatGPT in your browser. For first-time registration, paste the complete redirect URL if the loopback callback cannot be reached.".into(),
            pkce: Some(pkce),
            state: Some(state),
            openai_siwc: Some(OpenAiSiwcAuthorizeContext {
                client_id,
                redirect_uri: crate::openai_siwc::OPENAI_SIWC_REDIRECT.into(),
                nonce,
                resource: crate::openai_siwc::OPENAI_SIWC_RESOURCE.into(),
                ext_agent_host_id,
                expected_subject,
                dynamic_registration,
            }),
        }));
    }
    Ok(authorize_request(provider, &pkce, &state))
}

pub fn fresh_authorize_request(provider: &str) -> Option<AuthorizeRequest> {
    fresh_authorize_request_checked(provider).ok().flatten()
}

const PENDING_LOGIN_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingLogin {
    created_ms: u64,
    request: AuthorizeRequest,
}

fn pending_path(agent_dir: &std::path::Path, provider: &str) -> std::path::PathBuf {
    agent_dir
        .join("oauth-pending")
        .join(format!("{provider}.json"))
}

/// Keep the PKCE verifier and state of the URL we printed so a code pasted in
/// a later invocation is exchanged with the matching verifier.
pub fn save_pending_login(
    agent_dir: &std::path::Path,
    provider: &str,
    request: &AuthorizeRequest,
) -> Result<(), String> {
    let pending = PendingLogin {
        created_ms: crate::models_store::now_ms(),
        request: request.clone(),
    };
    let bytes = serde_json::to_vec(&pending).map_err(|err| err.to_string())?;
    davinci_sys::fs::atomic_write_private(&pending_path(agent_dir, provider), &bytes)
        .map_err(|err| err.to_string())
}

pub fn take_pending_login(agent_dir: &std::path::Path, provider: &str) -> Option<AuthorizeRequest> {
    let path = pending_path(agent_dir, provider);
    let raw = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let pending: PendingLogin = serde_json::from_slice(&raw).ok()?;
    let age = crate::models_store::now_ms().saturating_sub(pending.created_ms);
    (age <= PENDING_LOGIN_TTL_MS).then_some(pending.request)
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationCallbackInput {
    pub code: Option<String>,
    pub state: Option<String>,
    pub client_id: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

fn callback_from_pairs(pairs: Vec<(String, String)>) -> AuthorizationCallbackInput {
    let mut parsed = AuthorizationCallbackInput::default();
    for (key, value) in pairs {
        match key.as_str() {
            "code" => parsed.code = Some(value),
            "state" => parsed.state = Some(value),
            "client_id" => parsed.client_id = Some(value),
            "scope" => parsed.scope = Some(value),
            "error" => parsed.error = Some(value),
            _ => {}
        }
    }
    parsed
}

pub fn parse_authorization_callback(input: &str) -> AuthorizationCallbackInput {
    let value = input.trim();
    if value.is_empty() {
        return AuthorizationCallbackInput::default();
    }
    if let Ok(url) = url::Url::parse(value) {
        return callback_from_pairs(url.query_pairs().into_owned().collect());
    }
    if let Some((code, state)) = value.split_once('#') {
        return AuthorizationCallbackInput {
            code: Some(code.to_string()),
            state: Some(state.to_string()),
            ..Default::default()
        };
    }
    if value.contains("code=") || value.contains("error=") {
        return callback_from_pairs(
            url::form_urlencoded::parse(value.as_bytes())
                .into_owned()
                .collect(),
        );
    }
    AuthorizationCallbackInput {
        code: Some(value.to_string()),
        ..Default::default()
    }
}

pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let parsed = parse_authorization_callback(input);
    (parsed.code, parsed.state)
}

pub fn authorize_request(provider: &str, pkce: &Pkce, state: &str) -> Option<AuthorizeRequest> {
    match provider {
        "openai-codex" => None,
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
                openai_siwc: None,
            })
        }
        "xai" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: XAI_DEVICE_CODE_URL.into(),
            token_url: XAI_TOKEN_URL.into(),
            instructions: format!("xAI device code. client_id={XAI_CLIENT_ID} scope={XAI_SCOPE}"),
            pkce: None,
            state: None,
            openai_siwc: None,
        }),
        "kimi-coding" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: format!("{KIMI_OAUTH_HOST}/oauth/device/code"),
            token_url: format!("{KIMI_OAUTH_HOST}/oauth/token"),
            instructions: format!("Kimi device code. client_id={KIMI_CLIENT_ID}"),
            pkce: None,
            state: None,
            openai_siwc: None,
        }),
        "github-copilot" => Some(AuthorizeRequest {
            provider: provider.into(),
            url: GITHUB_DEVICE_CODE_URL.into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            instructions: format!("GitHub device code. client_id={GITHUB_CLIENT_ID}"),
            pkce: None,
            state: None,
            openai_siwc: None,
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
                openai_siwc: None,
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

/// OAuth token POST bodies for providers supported by davinci.
pub fn token_exchange_request(
    provider: &str,
    code: &str,
    pkce: Option<&Pkce>,
    state: Option<&str>,
) -> Option<TokenExchangeRequest> {
    let verifier = pkce.map(|p| p.verifier.as_str()).unwrap_or("");
    match provider {
        "openai-codex" => None,
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
        // Device-code providers redeem the device code, never an auth code.
        other if crate::device_login::is_device_code_provider(other) => {
            crate::device_login::device_token_request(other, code)
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

/// What a token endpoint hands back. `expires` is a Unix epoch millisecond
/// stamp, the same shape the TS credential stores (`Date.now() + expires_in *
/// 1000`); a provider that omits `expires_in` leaves it `None`. Dropping this
/// field is what used to make a stored login unrefreshable: with no expiry
/// recorded, nothing ever decided the token was old enough to renew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthTokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires: Option<u64>,
}

/// Refresh bodies for supported OAuth providers use `grant_type=refresh_token`
/// with the provider client id.
pub fn token_refresh_request(provider: &str, refresh: &str) -> Option<TokenExchangeRequest> {
    let form = |url: String, client_id: &str| TokenExchangeRequest {
        url,
        content_type: "application/x-www-form-urlencoded".into(),
        body: url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("client_id", client_id)
            .append_pair("refresh_token", refresh)
            .finish(),
        redirect_uri: String::new(),
    };
    match provider {
        "openai-codex" => None,
        "xai" => Some(form(XAI_TOKEN_URL.into(), XAI_CLIENT_ID)),
        "kimi-coding" => Some(form(
            format!("{KIMI_OAUTH_HOST}/api/oauth/token"),
            KIMI_CLIENT_ID,
        )),
        // OpenRouter exchanges a code for a durable API key and has no refresh
        // grant; Radius' token URL is per-gateway and not known here.
        _ => None,
    }
}

/// Trade a refresh token for a fresh access token. Fixture refresh (a
/// `pi-fixture-` token or `PI_OAUTH_FIXTURE`) never hits the network.
pub fn refresh_oauth_token(provider: &str, refresh: &str) -> Result<OauthTokens, String> {
    if provider == "openai-codex" {
        return Err(
            "openai-codex refresh requires the issued Sign in with ChatGPT client id".into(),
        );
    }
    if provider == "anthropic" {
        return Err(crate::auth::ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE.into());
    }
    if crate::fixtures::enabled()
        && (refresh.starts_with("pi-fixture-") || std::env::var("PI_OAUTH_FIXTURE").is_ok())
    {
        return Ok(OauthTokens {
            access: format!("{refresh}-access"),
            refresh: Some(refresh.to_string()),
            expires: Some(crate::models_store::now_ms().saturating_add(3_600_000)),
        });
    }
    if provider == "github-copilot" {
        // The stored refresh credential is the GitHub token; renewing means
        // trading it for a new Copilot token.
        return crate::device_login::copilot_token(refresh);
    }
    let request = token_refresh_request(provider, refresh)
        .ok_or_else(|| format!("OAuth refresh is not configured for {provider}"))?;
    let mut tokens = post_token_exchange(&request)?;
    // Providers that rotate refresh tokens return a new one; the ones that do
    // not expect the old one to be kept. Dropping it here would log the user
    // out on the next renewal.
    if tokens.refresh.is_none() {
        tokens.refresh = Some(refresh.to_string());
    }
    Ok(tokens)
}

/// Fixture token exchange never hits the network. Live POST uses the TS token URL
/// (overridable with `PI_OAUTH_TOKEN_URL`). Tests use `pi-fixture-` / `PI_OAUTH_FIXTURE`.
pub fn exchange_authorization_code(
    provider: &str,
    code: &str,
    pkce: Option<&Pkce>,
    state: Option<&str>,
) -> Result<OauthTokens, String> {
    if provider == "openai-codex" {
        return Err("openai-codex uses the verified Sign in with ChatGPT exchange path".into());
    }
    if provider == "anthropic" {
        return Err(crate::auth::ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE.into());
    }
    if crate::fixtures::enabled()
        && (code.starts_with("pi-fixture-") || std::env::var("PI_OAUTH_FIXTURE").is_ok())
    {
        return Ok(OauthTokens {
            access: format!("{provider}-{code}-access"),
            refresh: pkce.map(|p| format!("pi-fixture-{}", p.verifier)),
            expires: Some(crate::models_store::now_ms().saturating_add(3_600_000)),
        });
    }
    let exchange_state = state.or_else(|| pkce.map(|p| p.verifier.as_str()));
    let request = token_exchange_request(provider, code, pkce, exchange_state)
        .ok_or_else(|| format!("OAuth token exchange is not configured for {provider}"))?;
    post_token_exchange(&request)
}

fn post_token_exchange(request: &TokenExchangeRequest) -> Result<OauthTokens, String> {
    let url = crate::fixtures::enabled()
        .then(|| std::env::var("PI_OAUTH_TOKEN_URL").ok())
        .flatten()
        .unwrap_or_else(|| request.url.clone());
    let response = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .post(&url)
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
        format!(
            "Token exchange returned invalid JSON. url={url}; {}; details={err}",
            describe_token_body(&body)
        )
    })?;
    let access = value
        .get("access_token")
        .or_else(|| value.get("access"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!(
                "Token exchange returned invalid JSON. url={url}; {}; details=missing access_token",
                describe_token_body(&body)
            )
        })?;
    let refresh = value
        .get("refresh_token")
        .or_else(|| value.get("refresh"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // TS reads `expires_in` seconds and stamps `Date.now() + expires_in * 1000`;
    // an absolute `expires_at`/`expires` is taken as already being that stamp.
    let expires = value
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .map(|seconds| crate::models_store::now_ms().saturating_add(seconds.saturating_mul(1000)))
        .or_else(|| {
            value
                .get("expires_at")
                .or_else(|| value.get("expires"))
                .and_then(|v| v.as_u64())
        });
    Ok(OauthTokens {
        access: access.to_string(),
        refresh,
        expires,
    })
}

/// What an error message may say about a token endpoint reply: the OAuth
/// `error` / `error_description` strings (RFC 6749 5.2, never secret) and
/// the key names, never any other value.
fn describe_token_body(body: &str) -> String {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) else {
        return format!("non-JSON body, {} bytes", body.len());
    };
    let text = |key: &str| map.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    format!(
        "error={:?}; error_description={:?}; keys=[{}]",
        text("error"),
        text("error_description"),
        keys.join(",")
    )
}

pub fn oauth_providers() -> &'static [&'static str] {
    &[
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
    fn token_error_description_never_includes_values() {
        let body =
            r#"{"weird_token":"sk-SECRET","error":"invalid_grant","error_description":"expired"}"#;
        let described = describe_token_body(body);
        assert!(!described.contains("sk-SECRET"), "{described}");
        assert!(described.contains("invalid_grant"));
        assert!(described.contains("expired"));
    }

    #[test]
    fn authorization_callback_keeps_dynamic_client_metadata() {
        let parsed = parse_authorization_callback(
            "http://127.0.0.1:1455/auth/callback?code=xyz&state=s1&client_id=oaiapp_123&scope=chatgpt.tokens.use.direct+resource.invoke",
        );
        assert_eq!(parsed.code.as_deref(), Some("xyz"));
        assert_eq!(parsed.state.as_deref(), Some("s1"));
        assert_eq!(parsed.client_id.as_deref(), Some("oaiapp_123"));
        assert!(parsed
            .scope
            .as_deref()
            .is_some_and(|scope| scope.contains("chatgpt.tokens.use.direct")));
    }

    #[test]
    fn legacy_codex_exchange_and_refresh_are_disabled() {
        let pkce = generate_pkce(&[1u8; 32]);
        assert!(authorize_request("openai-codex", &pkce, "state").is_none());
        assert!(token_exchange_request("openai-codex", "code", Some(&pkce), None).is_none());
        assert!(token_refresh_request("openai-codex", "refresh").is_none());
        assert!(
            exchange_authorization_code("openai-codex", "code", Some(&pkce), Some("state"),)
                .unwrap_err()
                .contains("Sign in with ChatGPT")
        );
        assert!(refresh_oauth_token("openai-codex", "refresh")
            .unwrap_err()
            .contains("issued Sign in with ChatGPT client id"));
    }

    #[test]
    fn non_codex_oauth_contracts_remain_available() {
        let pkce = generate_pkce(&[1u8; 32]);
        for provider in [
            "openrouter",
            "xai",
            "kimi-coding",
            "github-copilot",
            "radius",
        ] {
            assert!(
                authorize_request(provider, &pkce, "state").is_some(),
                "{provider}"
            );
        }
        assert!(token_refresh_request("xai", "rt").is_some());
        assert!(token_refresh_request("kimi-coding", "rt").is_some());
        assert!(token_refresh_request("openrouter", "rt").is_none());
    }
}
