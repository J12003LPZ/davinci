//! ChatGPT Codex backend route for the `openai-codex` provider.
//!
//! `/login openai-codex` is the only OAuth login. It uses the Codex CLI OAuth client and stores the login
//! under `openai-codex` with a route marker, and requests made with it go to the ChatGPT Codex backend,
//! the endpoint Codex CLI uses. The route is chosen by the credential alone, so no other provider, model
//! catalog entry or setting changes. A login stored before this change without the marker keeps the
//! documented `https://api.openai.com/v1` route until `/login openai-codex` replaces it.
//!
//! This route is private to the Codex CLI product, it is refused by subscription-policy admission, and it
//! has no API-key fallback. See `docs/cache/chatgpt-backend-route.md`.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::{Credential, CredentialKind, Model, ResolvedAuth};

/// Backend base URL. Requests go to `{base}/responses`, so it carries the
/// `/codex` segment Codex CLI uses (`https://chatgpt.com/backend-api/codex/responses`).
/// The pre-migration resolver appended `/codex/responses` to `.../backend-api`; the
/// current one appends only `/responses`, so the bare `.../backend-api` sent requests
/// to `.../backend-api/responses`.
pub const BACKEND_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// `Credential::env` key and value that mark a backend login.
pub const ROUTE_ENV_KEY: &str = "OPENAI_CODEX_ROUTE";
pub const ROUTE_ENV_VALUE: &str = "chatgpt-backend";
/// `ResolvedAuth::source` for a backend login, so the route can follow the credential into the stream layer.
pub const AUTH_SOURCE: &str = "OAuth (Codex CLI)";
/// OAuth login provider id. The Codex CLI login is the only OAuth login for `openai-codex`.
pub const LOGIN_PROVIDER: &str = "openai-codex";

pub fn is_backend_credential(credential: &Credential) -> bool {
    credential.kind == CredentialKind::Oauth
        && credential.env.get(ROUTE_ENV_KEY).map(String::as_str) == Some(ROUTE_ENV_VALUE)
}

/// A backend login is usable when it carries an access token with the account id the backend requires.
pub fn validate_credential(credential: &Credential) -> Result<(), String> {
    let access = credential
        .access
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or("stored ChatGPT backend login has no access token; run /login openai-codex")?;
    crate::codex::extract_account_id(access)
        .map(|_| ())
        .map_err(|_| {
            "stored ChatGPT backend login has no account id; run /login openai-codex".to_string()
        })
}

/// The model with its base URL moved to the backend when the resolved auth came from a backend login.
/// Every other model and auth is returned untouched.
pub fn route_model<'a>(model: &'a Model, auth: &ResolvedAuth) -> Cow<'a, Model> {
    if model.provider == "openai-codex" && auth.source == AUTH_SOURCE {
        let mut routed = model.clone();
        routed.base_url = Some(BACKEND_BASE_URL.into());
        Cow::Owned(routed)
    } else {
        Cow::Borrowed(model)
    }
}

pub fn credential_env() -> HashMap<String, String> {
    HashMap::from([(ROUTE_ENV_KEY.to_string(), ROUTE_ENV_VALUE.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{resolve_provider_auth, AuthStorage};
    use base64::Engine;
    use serde_json::json;

    fn token(account_id: &str) -> String {
        let payload = base64::engine::general_purpose::STANDARD.encode(
            json!({ crate::codex::JWT_CLAIM_PATH: { "chatgpt_account_id": account_id } })
                .to_string(),
        );
        format!("aaa.{payload}.bbb")
    }

    fn model(provider: &str) -> Model {
        serde_json::from_value(json!({
            "id":"gpt-6-luna", "name":"fixture", "api":"openai-codex-responses",
            "provider":provider, "baseUrl":"https://api.openai.com/v1", "reasoning":true,
            "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
            "contextWindow":1000,"maxTokens":100
        }))
        .unwrap()
    }

    fn auth(source: &str) -> ResolvedAuth {
        ResolvedAuth {
            api_key: Some("tok".into()),
            headers: HashMap::from([("Authorization".into(), "Bearer tok".into())]),
            source: source.into(),
        }
    }

    #[test]
    fn only_a_backend_login_moves_the_route() {
        let public = model("openai-codex");
        let routed = route_model(&public, &auth(AUTH_SOURCE));
        assert_eq!(routed.base_url.as_deref(), Some(BACKEND_BASE_URL));
        // The documented-route login, an API-key-shaped source and another provider keep their URL.
        for source in ["OAuth", "stored credential", "env"] {
            let kept = route_model(&public, &auth(source));
            assert_eq!(
                kept.base_url.as_deref(),
                Some("https://api.openai.com/v1"),
                "{source}"
            );
        }
        let other = model("openai");
        assert_eq!(
            route_model(&other, &auth(AUTH_SOURCE)).base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
    }

    #[test]
    fn a_backend_login_requests_the_codex_responses_endpoint() {
        // Pins the full URLs, not just the base: the base alone passed while
        // requests went to .../backend-api/responses.
        let backend = auth(AUTH_SOURCE);
        let codex = model("openai-codex");
        let routed = route_model(&codex, &backend);
        assert_eq!(
            crate::stream::request_url(&routed, &backend),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            crate::codex::resolve_codex_websocket_url(routed.base_url.as_deref()),
            "wss://chatgpt.com/backend-api/codex/responses"
        );
        // The documented route is unchanged.
        let documented = auth("OAuth");
        let kept = route_model(&codex, &documented);
        assert_eq!(
            crate::stream::request_url(&kept, &documented),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn a_backend_login_needs_the_account_id_the_backend_requires() {
        let mut storage = AuthStorage::in_memory();
        assert!(storage
            .login_codex_backend("opaque", None, Some(u64::MAX))
            .is_err());
        assert!(storage.get("openai-codex").is_none());
        storage
            .login_codex_backend(token("acct"), Some("refresh".into()), Some(u64::MAX))
            .unwrap();
        let stored = storage.get("openai-codex").unwrap();
        assert!(is_backend_credential(stored));
        assert!(validate_credential(stored).is_ok());
    }

    #[test]
    fn a_backend_login_resolves_with_its_own_source_and_the_documented_login_does_not() {
        let mut storage = AuthStorage::in_memory();
        storage
            .login_codex_backend(token("acct"), Some("refresh".into()), Some(u64::MAX))
            .unwrap();
        let resolved =
            resolve_provider_auth("openai-codex", &storage, &HashMap::new(), true).unwrap();
        assert_eq!(resolved.source, AUTH_SOURCE);
        assert!(crate::stored_credential_problem("openai-codex", &storage).is_none());
        // Status views (/login, /model) report the same source request-time resolution does.
        let check = crate::model_runtime::check_auth(
            "openai-codex",
            &crate::ModelConfig::empty(),
            &storage,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(
            (check.kind.as_str(), check.source.as_str()),
            ("oauth", AUTH_SOURCE)
        );

        // A legacy-shaped login without the marker is still refused, as before.
        let mut legacy = AuthStorage::in_memory();
        legacy
            .login_oauth(
                "openai-codex",
                token("acct"),
                Some("r".into()),
                Some(u64::MAX),
            )
            .unwrap();
        assert!(!is_backend_credential(legacy.get("openai-codex").unwrap()));
        assert!(resolve_provider_auth("openai-codex", &legacy, &HashMap::new(), true).is_none());
        let problem = crate::stored_credential_problem("openai-codex", &legacy).unwrap();
        assert!(problem.contains("legacy Codex login"), "{problem}");
    }

    #[test]
    fn the_gate_keeps_the_documented_route_and_admits_only_a_routed_backend_login() {
        use crate::stream::require_codex_subscription_route as gate;
        let public = model("openai-codex");
        // Unchanged: documented route + OAuth passes; the backend URL without a backend login is refused.
        assert!(gate(&public, &auth("OAuth")).is_ok());
        let mut backend_url = public.clone();
        backend_url.base_url = Some(BACKEND_BASE_URL.into());
        let error = gate(&backend_url, &auth("OAuth")).unwrap_err();
        assert!(error.contains("documented"), "{error}");
        // A non-OAuth source is still refused on the documented route.
        assert!(gate(&public, &auth("stored credential")).is_err());
        // A backend login passes only once routed to the backend.
        assert!(gate(&backend_url, &auth(AUTH_SOURCE)).is_ok());
        let error = gate(&public, &auth(AUTH_SOURCE)).unwrap_err();
        assert!(error.contains("backend"), "{error}");
        // Header overrides stay refused on both routes.
        let mut mismatched = auth(AUTH_SOURCE);
        mismatched
            .headers
            .insert("authorization".into(), "Bearer other".into());
        assert!(gate(&backend_url, &mismatched).is_err());
        // Other providers are not touched by the gate.
        assert!(gate(&model("openai"), &auth("anything")).is_ok());
    }

    #[test]
    fn the_gate_refuses_a_mismatched_authorization_header_in_any_case_variant() {
        use crate::stream::require_codex_subscription_route as gate;
        let mut backend_url = model("openai-codex");
        backend_url.base_url = Some(BACKEND_BASE_URL.into());
        // Each HashMap gets its own random seed, so iteration order varies per
        // map. A check that reads only the first matching header passes about
        // half the time; 64 fresh maps make that miss effectively impossible.
        for _ in 0..64 {
            let mut both = auth(AUTH_SOURCE);
            both.headers
                .insert("authorization".into(), "Bearer other".into());
            assert!(gate(&backend_url, &both).is_err());
        }
        // Matching values under both spellings are still accepted.
        let mut agreeing = auth(AUTH_SOURCE);
        agreeing
            .headers
            .insert("authorization".into(), "Bearer tok".into());
        assert!(gate(&backend_url, &agreeing).is_ok());
    }

    #[test]
    fn the_login_provider_has_its_own_oauth_requests() {
        let pkce = crate::oauth_providers::generate_pkce(&[7u8; 32]);
        let request =
            crate::oauth_providers::authorize_request(LOGIN_PROVIDER, &pkce, "state1").unwrap();
        assert!(request
            .url
            .starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(request
            .url
            .contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(request.url.contains("code_challenge_method=S256"));
        assert!(request.url.contains("state=state1"));
        let exchange = crate::oauth_providers::token_exchange_request(
            LOGIN_PROVIDER,
            "code1",
            Some(&pkce),
            None,
        )
        .unwrap();
        assert!(exchange.body.contains("grant_type=authorization_code"));
        assert!(exchange.body.contains("code=code1"));
        let refresh = crate::oauth_providers::token_refresh_request(LOGIN_PROVIDER, "rt").unwrap();
        assert!(refresh.body.contains("grant_type=refresh_token"));
    }
}
