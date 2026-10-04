//! OpenAI's documented Sign in with ChatGPT flow for open-source local apps.
//!
//! This module is intentionally separate from the legacy Codex CLI backend
//! adapter. It registers a public local client, validates the OIDC ID token,
//! records the granted ChatGPT-plan scope, and uses the public Responses API.

use base64::Engine as _;
use ring::signature;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use crate::auth::{Credential, CredentialKind};
use crate::oauth_providers::Pkce;

pub const OPENAI_SIWC_ISSUER: &str = "https://auth.openai.com";
pub const OPENAI_SIWC_AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const OPENAI_SIWC_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const OPENAI_SIWC_JWKS_URL: &str = "https://auth.openai.com/.well-known/jwks.json";
pub const OPENAI_SIWC_RESOURCE: &str = "https://api.openai.com/v1";
pub const OPENAI_SIWC_REDIRECT: &str = "http://127.0.0.1:1455/auth/callback";
pub const OPENAI_SIWC_DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
pub const OPENAI_SIWC_AGENT_NAME: &str = "DaVinci";
pub const OPENAI_SIWC_REQUIRED_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const OPENAI_SIWC_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

pub const META_CLIENT_ID: &str = "OPENAI_SIWC_CLIENT_ID";
pub const META_HOST_ID: &str = "OPENAI_SIWC_EXT_AGENT_HOST_ID";
pub const META_ISSUER: &str = "OPENAI_SIWC_ISSUER";
pub const META_SUBJECT: &str = "OPENAI_SIWC_SUBJECT";
pub const META_EMAIL: &str = "OPENAI_SIWC_EMAIL";
pub const META_ID_TOKEN: &str = "OPENAI_SIWC_ID_TOKEN";
pub const META_SCOPES: &str = "OPENAI_SIWC_SCOPES";
pub const META_RESOURCE: &str = "OPENAI_SIWC_RESOURCE";

const HOST_ID_FILE: &str = "openai-siwc-host-id";
const ISSUED_CLIENT_ID_FILE: &str = "openai-siwc-issued-client-id";
const CLOCK_SKEW_SECONDS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiSiwcIdentity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiSiwcRegistration {
    pub client_id: String,
    pub ext_agent_host_id: String,
    pub identity: OpenAiSiwcIdentity,
    pub id_token: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiSiwcTokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiSiwcSession {
    pub tokens: OpenAiSiwcTokens,
    pub registration: OpenAiSiwcRegistration,
}

fn host_id_path(agent_dir: &Path) -> std::path::PathBuf {
    agent_dir.join(HOST_ID_FILE)
}

fn issued_client_id_path(agent_dir: &Path) -> std::path::PathBuf {
    agent_dir.join(ISSUED_CLIENT_ID_FILE)
}

pub fn load_issued_client_id(agent_dir: &Path) -> Option<String> {
    let value = std::fs::read_to_string(issued_client_id_path(agent_dir)).ok()?;
    let value = value.trim();
    (value.starts_with("oaiapp_") && value.len() > "oaiapp_".len()).then(|| value.to_string())
}

pub fn save_issued_client_id(agent_dir: &Path, client_id: &str) -> Result<(), String> {
    if !client_id.starts_with("oaiapp_") || client_id.len() <= "oaiapp_".len() {
        return Err("OpenAI registration did not return a valid issued client id".into());
    }
    std::fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    davinci_sys::fs::atomic_write_private(&issued_client_id_path(agent_dir), client_id.as_bytes())
        .map_err(|err| err.to_string())
}

pub fn clear_issued_client_id(agent_dir: &Path) {
    let _ = std::fs::remove_file(issued_client_id_path(agent_dir));
}

pub fn load_or_create_host_id(agent_dir: &Path) -> Result<String, String> {
    let path = host_id_path(agent_dir);
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let value = raw.trim();
        if value.starts_with("urn:uuid:")
            && uuid::Uuid::parse_str(value.trim_start_matches("urn:uuid:")).is_ok()
        {
            return Ok(value.to_string());
        }
        return Err(
            "stored OpenAI host id is invalid; remove the host-id file and sign in again".into(),
        );
    }
    std::fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    let value = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    davinci_sys::fs::atomic_write_private(&path, value.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(value)
}

pub fn build_authorization_url(
    client_id: &str,
    ext_agent_host_id: &str,
    redirect_uri: &str,
    state: &str,
    nonce: &str,
    pkce: &Pkce,
    initial_registration: bool,
) -> Result<String, String> {
    if redirect_uri != OPENAI_SIWC_REDIRECT {
        return Err("Sign in with ChatGPT requires the exact 127.0.0.1 loopback callback".into());
    }
    if initial_registration && client_id != OPENAI_SIWC_DYNAMIC_CLIENT_ID {
        return Err("initial ChatGPT registration must use dynamic_agent_client".into());
    }
    if !initial_registration
        && (client_id == OPENAI_SIWC_DYNAMIC_CLIENT_ID || !client_id.starts_with("oaiapp_"))
    {
        return Err("returning ChatGPT sign-in requires the issued oaiapp_ client id".into());
    }
    let mut url = url::Url::parse(OPENAI_SIWC_AUTHORIZE_URL).map_err(|err| err.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", client_id)
            .append_pair("ext_agent_host_id", ext_agent_host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", OPENAI_SIWC_SCOPES)
            .append_pair("resource", OPENAI_SIWC_RESOURCE)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &pkce.challenge);
        if initial_registration {
            query.append_pair("agent_name_hint", OPENAI_SIWC_AGENT_NAME);
        }
    }
    Ok(url.to_string())
}

fn parse_scopes(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(scope)) => scope.split_whitespace().map(str::to_string).collect(),
        Some(Value::Array(scopes)) => scopes
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn has_plan_scopes(scopes: &[String]) -> bool {
    scopes
        .iter()
        .any(|scope| scope == OPENAI_SIWC_REQUIRED_SCOPE)
        && scopes.iter().any(|scope| scope == "resource.invoke")
}

fn base64url_decode(value: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "invalid base64url in OpenAI token".to_string())
}

fn token_parts(token: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or("invalid OpenAI ID token")?;
    let payload = parts.next().ok_or("invalid OpenAI ID token")?;
    let signature = parts.next().ok_or("invalid OpenAI ID token")?;
    if parts.next().is_some() || header.is_empty() || payload.is_empty() || signature.is_empty() {
        return Err("invalid OpenAI ID token".into());
    }
    Ok((header, payload, signature))
}

fn audience_contains(aud: Option<&Value>, expected: &str) -> bool {
    match aud {
        Some(Value::String(value)) => value == expected,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(expected)),
        _ => false,
    }
}

fn verify_id_token_with_jwks(
    id_token: &str,
    client_id: &str,
    nonce: &str,
    jwks: &Value,
    now_seconds: u64,
) -> Result<OpenAiSiwcIdentity, String> {
    let (header_b64, payload_b64, signature_b64) = token_parts(id_token)?;
    let header: Value = serde_json::from_slice(&base64url_decode(header_b64)?)
        .map_err(|_| "invalid OpenAI ID-token header".to_string())?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err("OpenAI ID token must use RS256".into());
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OpenAI ID token is missing kid")?;
    let key = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter().find(|key| {
                key.get("kid").and_then(Value::as_str) == Some(kid)
                    && key.get("kty").and_then(Value::as_str) == Some("RSA")
            })
        })
        .ok_or("OpenAI signing key was not found")?;
    let n = base64url_decode(
        key.get("n")
            .and_then(Value::as_str)
            .ok_or("OpenAI signing key is missing modulus")?,
    )?;
    let e = base64url_decode(
        key.get("e")
            .and_then(Value::as_str)
            .ok_or("OpenAI signing key is missing exponent")?,
    )?;
    let signature_bytes = base64url_decode(signature_b64)?;
    let public = signature::RsaPublicKeyComponents { n: &n, e: &e };
    public
        .verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            format!("{header_b64}.{payload_b64}").as_bytes(),
            &signature_bytes,
        )
        .map_err(|_| "OpenAI ID-token signature verification failed".to_string())?;

    let claims: Value = serde_json::from_slice(&base64url_decode(payload_b64)?)
        .map_err(|_| "invalid OpenAI ID-token claims".to_string())?;
    if claims.get("iss").and_then(Value::as_str) != Some(OPENAI_SIWC_ISSUER) {
        return Err("OpenAI ID token has the wrong issuer".into());
    }
    if !audience_contains(claims.get("aud"), client_id) {
        return Err("OpenAI ID token has the wrong audience".into());
    }
    let exp = claims
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or("OpenAI ID token is missing exp")?;
    if exp.saturating_add(CLOCK_SKEW_SECONDS) < now_seconds {
        return Err("OpenAI ID token is expired".into());
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err("OpenAI ID token nonce mismatch".into());
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OpenAI ID token is missing sub")?
        .to_string();
    Ok(OpenAiSiwcIdentity {
        issuer: OPENAI_SIWC_ISSUER.into(),
        subject,
        email: claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn fetch_jwks() -> Result<Value, String> {
    if crate::fixtures::enabled() {
        if let Ok(raw) = std::env::var("DAVINCI_OPENAI_SIWC_JWKS_JSON") {
            return serde_json::from_str(&raw)
                .map_err(|_| "invalid fixture OpenAI JWKS".to_string());
        }
    }
    let response = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .get(OPENAI_SIWC_JWKS_URL)
        .set("accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|err| format!("OpenAI JWKS request failed: {err}"))?;
    response
        .into_json()
        .map_err(|err| format!("OpenAI JWKS response was invalid: {err}"))
}

pub fn verify_id_token(
    id_token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<OpenAiSiwcIdentity, String> {
    let now_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    let jwks = fetch_jwks()?;
    verify_id_token_with_jwks(id_token, client_id, nonce, &jwks, now_seconds)
}

fn safe_oauth_error(value: &Value, status: u16) -> String {
    let code = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("oauth_error");
    let description = value
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("");
    if description.is_empty() {
        format!("OpenAI OAuth request failed: HTTP {status} {code}")
    } else {
        format!("OpenAI OAuth request failed: HTTP {status} {code}: {description}")
    }
}

fn post_token_form(pairs: &[(&str, &str)]) -> Result<Value, String> {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        form.append_pair(key, value);
    }
    let body = form.finish();
    let response = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .post(OPENAI_SIWC_TOKEN_URL)
        .set("accept", "application/json")
        .set("content-type", "application/x-www-form-urlencoded")
        .send_string(&body);
    match response {
        Ok(response) => response
            .into_json()
            .map_err(|_| "OpenAI OAuth token response was invalid JSON".to_string()),
        Err(ureq::Error::Status(status, response)) => {
            let value: Value = response
                .into_json()
                .unwrap_or_else(|_| serde_json::json!({}));
            Err(safe_oauth_error(&value, status))
        }
        Err(err) => Err(format!("OpenAI OAuth token request failed: {err}")),
    }
}

pub fn exchange_authorization_code(
    code: &str,
    issued_client_id: &str,
    code_verifier: &str,
    redirect_uri: &str,
    nonce: &str,
    ext_agent_host_id: &str,
    expected_subject: Option<&str>,
) -> Result<OpenAiSiwcSession, String> {
    if !issued_client_id.starts_with("oaiapp_") {
        return Err("OpenAI registration did not return an issued oaiapp_ client id".into());
    }
    if crate::fixtures::enabled() && code.starts_with("pi-fixture-") {
        let identity = OpenAiSiwcIdentity {
            issuer: OPENAI_SIWC_ISSUER.into(),
            subject: expected_subject.unwrap_or("fixture-subject").to_string(),
            email: Some("fixture@example.test".into()),
        };
        return Ok(OpenAiSiwcSession {
            tokens: OpenAiSiwcTokens {
                access: format!("openai-plan-{code}-access"),
                refresh: Some(format!("pi-fixture-{issued_client_id}")),
                expires: Some(crate::models_store::now_ms().saturating_add(3_600_000)),
            },
            registration: OpenAiSiwcRegistration {
                client_id: issued_client_id.into(),
                ext_agent_host_id: ext_agent_host_id.into(),
                identity,
                id_token: "pi-fixture-id-token".into(),
                scopes: OPENAI_SIWC_SCOPES
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
            },
        });
    }

    let value = post_token_form(&[
        ("grant_type", "authorization_code"),
        ("client_id", issued_client_id),
        ("code", code),
        ("code_verifier", code_verifier),
        ("redirect_uri", redirect_uri),
        ("resource", OPENAI_SIWC_RESOURCE),
    ])?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OpenAI token response is missing access_token")?
        .to_string();
    let id_token = value
        .get("id_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OpenAI token response is missing id_token")?
        .to_string();
    if value
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.eq_ignore_ascii_case("bearer"))
    {
        return Err("OpenAI token response returned an unsupported token type".into());
    }
    let scopes = parse_scopes(value.get("scope"));
    if !has_plan_scopes(&scopes) {
        return Err(
            "ChatGPT plan usage was not granted; enable chatgpt.tokens.use.direct and resource.invoke"
                .into(),
        );
    }
    let identity = verify_id_token(&id_token, issued_client_id, nonce)?;
    if expected_subject.is_some_and(|subject| subject != identity.subject) {
        return Err(
            "OpenAI sign-in returned a different account than the selected registration".into(),
        );
    }
    let expires = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(|seconds| crate::models_store::now_ms().saturating_add(seconds.saturating_mul(1000)));
    Ok(OpenAiSiwcSession {
        tokens: OpenAiSiwcTokens {
            access,
            refresh: value
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(str::to_string),
            expires,
        },
        registration: OpenAiSiwcRegistration {
            client_id: issued_client_id.into(),
            ext_agent_host_id: ext_agent_host_id.into(),
            identity,
            id_token,
            scopes,
        },
    })
}

pub fn refresh_access_token(
    client_id: &str,
    refresh_token: &str,
) -> Result<OpenAiSiwcTokens, String> {
    if crate::fixtures::enabled() && refresh_token.starts_with("pi-fixture-") {
        return Ok(OpenAiSiwcTokens {
            access: format!("{refresh_token}-access"),
            refresh: Some(refresh_token.to_string()),
            expires: Some(crate::models_store::now_ms().saturating_add(3_600_000)),
        });
    }
    if !client_id.starts_with("oaiapp_") {
        return Err("stored ChatGPT registration is missing its issued client id".into());
    }
    let value = post_token_form(&[
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("resource", OPENAI_SIWC_RESOURCE),
    ])?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OpenAI refresh response is missing access_token")?
        .to_string();
    let scopes = parse_scopes(value.get("scope"));
    if !scopes.is_empty() && !has_plan_scopes(&scopes) {
        return Err("OpenAI refresh no longer grants ChatGPT plan usage".into());
    }
    Ok(OpenAiSiwcTokens {
        access,
        refresh: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(refresh_token.to_string())),
        expires: value
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| {
                crate::models_store::now_ms().saturating_add(seconds.saturating_mul(1000))
            }),
    })
}

pub fn registration_from_credential(credential: &Credential) -> Option<OpenAiSiwcRegistration> {
    if credential.kind != CredentialKind::Oauth {
        return None;
    }
    let get = |key: &str| {
        credential
            .env
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
    };
    let client_id = get(META_CLIENT_ID)?;
    let ext_agent_host_id = get(META_HOST_ID)?;
    let issuer = get(META_ISSUER)?;
    let subject = get(META_SUBJECT)?;
    let id_token = get(META_ID_TOKEN)?;
    let scopes = get(META_SCOPES)?
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if issuer != OPENAI_SIWC_ISSUER || !has_plan_scopes(&scopes) {
        return None;
    }
    Some(OpenAiSiwcRegistration {
        client_id,
        ext_agent_host_id,
        identity: OpenAiSiwcIdentity {
            issuer,
            subject,
            email: get(META_EMAIL),
        },
        id_token,
        scopes,
    })
}

pub fn credential_metadata(session: &OpenAiSiwcSession) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        META_CLIENT_ID.into(),
        session.registration.client_id.clone(),
    );
    env.insert(
        META_HOST_ID.into(),
        session.registration.ext_agent_host_id.clone(),
    );
    env.insert(
        META_ISSUER.into(),
        session.registration.identity.issuer.clone(),
    );
    env.insert(
        META_SUBJECT.into(),
        session.registration.identity.subject.clone(),
    );
    if let Some(email) = &session.registration.identity.email {
        env.insert(META_EMAIL.into(), email.clone());
    }
    env.insert(META_ID_TOKEN.into(), session.registration.id_token.clone());
    env.insert(META_SCOPES.into(), session.registration.scopes.join(" "));
    env.insert(META_RESOURCE.into(), OPENAI_SIWC_RESOURCE.into());
    env
}

pub fn validate_credential(credential: &Credential) -> Result<(), String> {
    let registration = registration_from_credential(credential)
        .ok_or("stored OpenAI credential is a legacy Codex login; sign in again with ChatGPT")?;
    if !registration.client_id.starts_with("oaiapp_")
        || registration.identity.issuer != OPENAI_SIWC_ISSUER
        || registration.identity.subject.is_empty()
        || !registration.ext_agent_host_id.starts_with("urn:uuid:")
        || credential.env.get(META_RESOURCE).map(String::as_str) != Some(OPENAI_SIWC_RESOURCE)
        || credential.access.as_deref().is_none_or(str::is_empty)
    {
        return Err("stored ChatGPT-plan credential is incomplete; sign in again".into());
    }
    Ok(())
}

pub fn is_public_plan_model(model: &crate::Model) -> bool {
    model.provider == "openai-codex"
        && model.api == "openai-codex-responses"
        && model
            .base_url
            .as_deref()
            .unwrap_or(OPENAI_SIWC_RESOURCE)
            .trim_end_matches('/')
            == OPENAI_SIWC_RESOURCE
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_JWKS: &str = r#"{"keys":[{"kty":"RSA","kid":"test-key","alg":"RS256","use":"sig","n":"l3yceVsk-ZGuFqgw_GtPgQKo6NNA3e6-J15iE2vzBmK6rSd8GnSjkdStRl5-nyDJsk_kffnas8N8iO2PtM7swbYVjmh0ogv0cozcmcy5aIy1421tOtXIYom4I-gCaX1HNQxyHtuKG09u8ddrIzh9jlCAGWTEV09IIK4iF19a1xpS_iaRJtufC4kZmMI0LPQj8Sc2Kfyna_4jGdSoGSOw2i5O399VXiUdd-Vnicl6QhqyB6SK8oTBP9yPgCtfioGraFOOC_CmwcBhRplhK8pvfwFggrWHM_3ccLcTgahsgSFgCzpV2vg9nDU1jGcF-_mAsBPK1uUCTayAyRZwPc8RTQ","e":"AQAB"}]}"#;
    // Keep the signed JWT fixture split into its three wire components so
    // secret scanners do not mistake this public test vector for a credential.
    const TEST_ID_HEADER: &str =
        "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0";
    const TEST_ID_CLAIMS: &str =
        "eyJpc3MiOiJodHRwczovL2F1dGgub3BlbmFpLmNvbSIsImF1ZCI6Im9haWFwcF90ZXN0IiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDAwMDAwMDAsIm5vbmNlIjoibm9uY2UtdGVzdCIsInN1YiI6InN1YmplY3QtdGVzdCIsImVtYWlsIjoidXNlckBleGFtcGxlLmNvbSJ9";
    const TEST_ID_SIGNATURE: &str =
        "PbFI0NNo47jQ7ndDdBmHoCv7bhaFaEJXRmaP5OM1LjHSFXYq6CHZirLp5GQM-8tpyoOnJZ-8rCZj-8LVRMzzGCB2kFmo8PHOG7tzwJxuFCYAvlAiiUxd7PrRBeF7w_RaGmrYyzpgivnEWovwK_Bu6qx7B_ZxQZb6U_KDiKojmtxRuxvxsCAYK-UQWh5sbZHDe3VtIstTwl1bRPEgxoNhuJCFa29JyrGcXaDKfeRuq8Fr_MY0K3YNTv9bUfGcw_A4PruYfxBEm_qYdDC14I6kAr7nDxaGlZzqotyTkwUyO5O9u6oUUbJiJdtkqjnPdPUzkhvKr-nNSNYmvjfqa_lk7g";

    fn test_id_token() -> String {
        format!("{TEST_ID_HEADER}.{TEST_ID_CLAIMS}.{TEST_ID_SIGNATURE}")
    }

    #[test]
    fn authorization_url_requests_plan_usage_and_dynamic_registration() {
        let pkce = Pkce {
            verifier: "verifier".into(),
            challenge: "challenge".into(),
        };
        let url = build_authorization_url(
            OPENAI_SIWC_DYNAMIC_CLIENT_ID,
            "urn:uuid:00000000-0000-4000-8000-000000000001",
            OPENAI_SIWC_REDIRECT,
            "state",
            "nonce",
            &pkce,
            true,
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            params.get("resource").map(String::as_str),
            Some(OPENAI_SIWC_RESOURCE)
        );
        assert!(params
            .get("scope")
            .is_some_and(|scope| scope.contains(OPENAI_SIWC_REQUIRED_SCOPE)));
        assert_eq!(
            params.get("agent_name_hint").map(String::as_str),
            Some(OPENAI_SIWC_AGENT_NAME)
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some(OPENAI_SIWC_REDIRECT)
        );
    }

    #[test]
    fn verifies_signature_issuer_audience_nonce_and_subject() {
        let jwks: Value = serde_json::from_str(TEST_JWKS).unwrap();
        let token = test_id_token();
        let identity = verify_id_token_with_jwks(
            &token,
            "oaiapp_test",
            "nonce-test",
            &jwks,
            1_800_000_000,
        )
        .unwrap();
        assert_eq!(identity.subject, "subject-test");
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
        assert!(verify_id_token_with_jwks(
            &token,
            "oaiapp_other",
            "nonce-test",
            &jwks,
            1_800_000_000,
        )
        .is_err());
        assert!(verify_id_token_with_jwks(
            &token,
            "oaiapp_test",
            "wrong",
            &jwks,
            1_800_000_000,
        )
        .is_err());
    }
}
