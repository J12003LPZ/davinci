//! OAuth URL + token storage matching TypeScript `packages/ai/src/auth/oauth`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::auth::{AuthError, AuthStorage, Credential};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthApp {
    pub provider: String,
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

pub fn oauth_app(provider: &str) -> Option<OAuthApp> {
    Some(match provider {
        "anthropic" => OAuthApp {
            provider: provider.into(),
            authorize_url: "https://claude.ai/oauth/authorize".into(),
            token_url: "https://console.anthropic.com/oauth/token".into(),
            client_id: "pi".into(),
            scopes: vec!["org:create_api_key".into(), "user:profile".into()],
        },
        "openai" | "openai-codex" => OAuthApp {
            provider: provider.into(),
            authorize_url: "https://auth.openai.com/oauth/authorize".into(),
            token_url: "https://auth.openai.com/oauth/token".into(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
            scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "offline_access".into(),
            ],
        },
        "google" | "google-vertex" => OAuthApp {
            provider: provider.into(),
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: "pi".into(),
            scopes: vec!["https://www.googleapis.com/auth/cloud-platform".into()],
        },
        "github-copilot" => OAuthApp {
            provider: provider.into(),
            authorize_url: "https://github.com/login/device/code".into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            client_id: "Iv1.b507a08c87ecfe98".into(),
            scopes: vec!["read:user".into()],
        },
        _ => return None,
    })
}

pub fn authorize_url(provider: &str, redirect_uri: &str, state: &str) -> Option<String> {
    let app = oauth_app(provider)?;
    let scope = app.scopes.join(" ");
    Some(format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge_method=S256",
        app.authorize_url,
        urlencoding_lite(&app.client_id),
        urlencoding_lite(redirect_uri),
        urlencoding_lite(&scope),
        urlencoding_lite(state)
    ))
}

fn urlencoding_lite(value: &str) -> String {
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

pub fn parse_token_response(raw: &str) -> Result<Credential, AuthError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| AuthError::Message(format!("Invalid OAuth token response: {e}")))?;
    let access = value
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Message("OAuth token response missing access_token".into()))?;
    let refresh = value
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let expires = value.get("expires_in").and_then(|v| v.as_i64());
    Ok(Credential::Oauth {
        access: access.to_string(),
        refresh,
        expires,
        extra: value
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| !matches!(k.as_str(), "access_token" | "refresh_token" | "expires_in"))
            .collect(),
    })
}

pub fn oauth_needs_refresh(credential: &Credential, now_secs: i64, min_expiry_secs: i64) -> bool {
    match credential {
        Credential::Oauth {
            expires, refresh, ..
        } => {
            if refresh.is_none() {
                return false;
            }
            match expires {
                Some(exp) => exp - now_secs <= min_expiry_secs,
                None => false,
            }
        }
        _ => false,
    }
}

/// Refresh using a fixture body (tests) or a live token URL when network is allowed.
pub fn refresh_oauth_token(
    provider: &str,
    refresh_token: &str,
    fixture_body: Option<&str>,
) -> Result<Credential, AuthError> {
    if let Some(raw) = fixture_body {
        return parse_token_response(raw);
    }
    if std::env::var("PI_DISABLE_NETWORK").is_ok() || std::env::var("PI_OFFLINE").is_ok() {
        return Err(AuthError::Message(
            "OAuth refresh disabled (offline)".into(),
        ));
    }
    let app =
        oauth_app(provider).ok_or_else(|| AuthError::Message("unknown oauth provider".into()))?;
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        urlencoding_lite(refresh_token),
        urlencoding_lite(&app.client_id)
    );
    let response = ureq::post(&app.token_url)
        .set("content-type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .map_err(|e| AuthError::Message(e.to_string()))?;
    let raw = response
        .into_string()
        .map_err(|e| AuthError::Message(e.to_string()))?;
    parse_token_response(&raw)
}

pub fn store_oauth(
    storage: &mut impl AuthStorage,
    provider: &str,
    credential: Credential,
) -> Result<(), AuthError> {
    storage.write(provider, credential)
}

pub fn credential_from_env_or_store(storage: &impl AuthStorage, provider: &str) -> Option<String> {
    if let Some(env) = crate::get_env_api_key(provider) {
        return Some(env);
    }
    match storage.read(provider) {
        Some(Credential::ApiKey { ref key, .. }) => Some(key.clone()),
        Some(Credential::Oauth { ref access, .. }) => Some(access.clone()),
        None => None,
    }
}

pub fn providers_with_oauth() -> HashMap<&'static str, OAuthApp> {
    [
        "anthropic",
        "openai",
        "openai-codex",
        "google",
        "github-copilot",
    ]
    .into_iter()
    .filter_map(|id| oauth_app(id).map(|app| (id, app)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::InMemoryCredentialStore;

    #[test]
    fn authorize_url_and_token_fixture() {
        let url = authorize_url("anthropic", "http://127.0.0.1:8765/cb", "st").unwrap();
        assert!(url.contains("https://claude.ai/oauth/authorize"));
        assert!(url.contains("state=st"));
        let cred = parse_token_response(
            r#"{"access_token":"tok","refresh_token":"ref","expires_in":3600}"#,
        )
        .unwrap();
        match cred {
            Credential::Oauth { ref access, .. } => assert_eq!(access, "tok"),
            other => panic!("{other:?}"),
        }
        let mut store = InMemoryCredentialStore::new();
        store_oauth(&mut store, "anthropic", cred).unwrap();
        assert!(credential_from_env_or_store(&store, "anthropic").is_some());
        let refreshed = refresh_oauth_token(
            "anthropic",
            "ref",
            Some(r#"{"access_token":"new","refresh_token":"ref2","expires_in":60}"#),
        )
        .unwrap();
        match refreshed {
            Credential::Oauth { ref access, .. } => assert_eq!(access, "new"),
            other => panic!("{other:?}"),
        }
        let stale = Credential::Oauth {
            access: "old".into(),
            refresh: Some("r".into()),
            expires: Some(10),
            extra: Default::default(),
        };
        assert!(oauth_needs_refresh(&stale, 10, 5));
        assert!(!oauth_needs_refresh(&stale, 0, 5));
    }
}
