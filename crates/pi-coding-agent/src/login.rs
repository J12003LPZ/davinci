//! Interactive /login and /logout matching TypeScript login dialogs and OAuth.

use pi_ai::auth::{AuthStorage, Credential, FileAuthStorage};
use pi_ai::oauth::oauth_app;
use pi_ai::oauth_flow::{
    anthropic_authorize_url, anthropic_redirect_uri, fixture_authorization_input, generate_pkce,
};
use pi_ai::providers::{all_providers, provider_display_name};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn open_browser(target: &str) {
    if std::env::var("PI_OAUTH_NO_BROWSER").is_ok()
        || std::env::var("PI_DISABLE_NETWORK").ok().as_deref() == Some("1")
        || fixture_authorization_input().is_some()
    {
        return;
    }
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![target])
    } else if cfg!(windows) {
        ("rundll32", vec!["url.dll,FileProtocolHandler", target])
    } else {
        ("xdg-open", vec![target])
    };
    let _ = Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub const OAUTH_LOGIN_LABEL: &str = "Sign in with an account";
pub const API_KEY_LOGIN_LABEL: &str = "Sign in with an API key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAuthType {
    Oauth,
    ApiKey,
}

impl LoginAuthType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Oauth => "subscription",
            Self::ApiKey => "API key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginProvider {
    pub id: String,
    pub name: String,
    pub auth_type: LoginAuthType,
}

pub fn auth_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("auth.json")
}

pub fn login_providers(auth_type: Option<LoginAuthType>) -> Vec<LoginProvider> {
    let mut options = Vec::new();
    for provider in all_providers() {
        let name = if provider.name.is_empty() {
            provider_display_name(&provider.id).to_string()
        } else {
            provider.name.clone()
        };
        if auth_type != Some(LoginAuthType::ApiKey) && oauth_app(&provider.id).is_some() {
            options.push(LoginProvider {
                id: provider.id.clone(),
                name: name.clone(),
                auth_type: LoginAuthType::Oauth,
            });
        }
        if auth_type != Some(LoginAuthType::Oauth) {
            options.push(LoginProvider {
                id: provider.id,
                name,
                auth_type: LoginAuthType::ApiKey,
            });
        }
    }
    options.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    options
}

pub fn find_login_providers(provider_ref: &str) -> Vec<LoginProvider> {
    let needle = provider_ref.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    login_providers(None)
        .into_iter()
        .filter(|provider| {
            provider.id.to_ascii_lowercase() == needle
                || provider.name.to_ascii_lowercase() == needle
        })
        .collect()
}

pub fn render_auth_type_selector(provider_name: Option<&str>) -> String {
    let title = if let Some(name) = provider_name {
        format!("Select authentication method for {name}:")
    } else {
        "Select authentication method:".into()
    };
    format!("1. {OAUTH_LOGIN_LABEL}\n2. {API_KEY_LOGIN_LABEL}\n{title}\n")
}

pub fn render_login_provider_list(auth_type: LoginAuthType) -> String {
    let providers = login_providers(Some(auth_type));
    if providers.is_empty() {
        return match auth_type {
            LoginAuthType::Oauth => "No subscription providers available.\n".into(),
            LoginAuthType::ApiKey => "No API key providers available.\n".into(),
        };
    }
    let mut lines = vec!["Login to a provider".into()];
    for provider in providers {
        lines.push(format!(
            "{} [{}]",
            provider.name,
            provider.auth_type.label()
        ));
    }
    lines.join("\n") + "\n"
}

pub fn render_login_dialog(
    provider_name: &str,
    url: Option<&str>,
    instructions: Option<&str>,
) -> String {
    let mut lines = vec!["─".repeat(40), format!("Login to {provider_name}")];
    if let Some(url) = url {
        lines.push(String::new());
        lines.push(url.to_string());
        let hint = if cfg!(target_os = "macos") {
            "Cmd+click to open"
        } else {
            "Ctrl+click to open"
        };
        lines.push(hint.into());
    }
    if let Some(instructions) = instructions {
        lines.push(String::new());
        lines.push(instructions.to_string());
    }
    lines.push("─".repeat(40));
    lines.join("\n") + "\n"
}

fn store_credential(
    agent_dir: &Path,
    provider: &str,
    credential: Credential,
) -> Result<PathBuf, String> {
    let path = auth_path(agent_dir);
    let mut store = FileAuthStorage::open(&path).map_err(|err| err.to_string())?;
    store
        .write(provider, credential)
        .map_err(|err| err.to_string())?;
    Ok(store.path().to_path_buf())
}

fn api_key_from_env_or_value(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("PI_LOGIN_API_KEY")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

pub fn login_api_key(
    agent_dir: &Path,
    provider: &str,
    key: Option<&str>,
) -> Result<String, String> {
    let key = api_key_from_env_or_value(key).ok_or_else(|| "API key is required".to_string())?;
    let path = store_credential(agent_dir, provider, Credential::ApiKey { key, env: None })?;
    let name = provider_display_name(provider);
    Ok(format!(
        "Saved API key for {name}. Credentials saved to {}",
        path.display()
    ))
}

pub fn login_oauth(agent_dir: &Path, provider: &str) -> Result<String, String> {
    let name = provider_display_name(provider);
    let pkce = generate_pkce();
    let (url, token_url, client_id, extra) = if provider == "anthropic" {
        (
            anthropic_authorize_url(&pkce),
            pi_ai::oauth_flow::ANTHROPIC_TOKEN_URL.to_string(),
            pi_ai::oauth_flow::ANTHROPIC_CLIENT_ID.to_string(),
            Some(pkce.verifier.clone()),
        )
    } else {
        let app = oauth_app(provider).ok_or_else(|| "No login methods available.".to_string())?;
        let redirect = anthropic_redirect_uri();
        let authorize = pi_ai::authorize_url(provider, &redirect, &pkce.verifier)
            .unwrap_or_else(|| anthropic_authorize_url(&pkce));
        (authorize, app.token_url, app.client_id, None)
    };
    open_browser(&url);
    let callback = pi_ai::oauth_flow::wait_for_oauth_callback(
        &pkce.verifier,
        &pi_ai::oauth_flow::callback_host(),
        std::env::var("PI_OAUTH_CALLBACK_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(pi_ai::oauth_flow::ANTHROPIC_CALLBACK_PORT),
        pi_ai::oauth_flow::ANTHROPIC_CALLBACK_PATH,
    )?;
    let mut body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": client_id,
        "code": callback.code,
        "redirect_uri": anthropic_redirect_uri(),
        "code_verifier": pkce.verifier,
    });
    if extra.is_some() {
        body["state"] = serde_json::Value::String(callback.state);
    }
    let credential = pi_ai::oauth_flow::exchange_authorization_code(&token_url, &body)
        .map_err(|err| err.to_string())?;
    let path = store_credential(agent_dir, provider, credential)?;
    Ok(format!(
        "Logged in to {name}. Credentials saved to {}\nListening for OAuth callback on {}",
        path.display(),
        url
    ))
}

pub fn logout_provider(agent_dir: &Path, provider: &str) -> Result<String, String> {
    let path = auth_path(agent_dir);
    let mut store = FileAuthStorage::open(&path).map_err(|err| err.to_string())?;
    if store.read(provider).is_none() {
        return Ok(
            "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged."
                .into(),
        );
    }
    let oauth = matches!(store.read(provider), Some(Credential::Oauth { .. }));
    store.delete(provider).map_err(|err| err.to_string())?;
    let name = provider_display_name(provider);
    if oauth {
        Ok(format!("Logged out of {name}"))
    } else {
        Ok(format!(
            "Removed stored API key for {name}. Environment variables and models.json config are unchanged."
        ))
    }
}

pub fn logout_empty_message() -> &'static str {
    "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged."
}

pub fn handle_login_command(agent_dir: &Path, args: &str) -> Result<String, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(render_auth_type_selector(None));
    }
    if trimmed == OAUTH_LOGIN_LABEL || trimmed == "1" || trimmed.eq_ignore_ascii_case("oauth") {
        return Ok(render_login_provider_list(LoginAuthType::Oauth));
    }
    if trimmed == API_KEY_LOGIN_LABEL || trimmed == "2" || trimmed.eq_ignore_ascii_case("api_key") {
        return Ok(render_login_provider_list(LoginAuthType::ApiKey));
    }
    let (provider_ref, rest) = trimmed
        .split_once(' ')
        .map(|(left, right)| (left, right.trim()))
        .unwrap_or((trimmed, ""));
    let matches = find_login_providers(provider_ref);
    if matches.is_empty() {
        return Err("usage: /login <provider>".into());
    }
    let ids: std::collections::BTreeSet<_> = matches.iter().map(|p| p.id.clone()).collect();
    if ids.len() > 1 {
        return Ok(render_auth_type_selector(None));
    }
    if matches.len() > 1 && rest.is_empty() {
        return Ok(render_auth_type_selector(Some(&matches[0].name)));
    }
    let auth_type = if rest.eq_ignore_ascii_case("oauth") || rest == OAUTH_LOGIN_LABEL {
        LoginAuthType::Oauth
    } else if rest.eq_ignore_ascii_case("api_key") || rest == API_KEY_LOGIN_LABEL {
        LoginAuthType::ApiKey
    } else if matches.len() == 1 {
        matches[0].auth_type
    } else {
        LoginAuthType::ApiKey
    };
    let provider = matches[0].id.clone();
    let name = matches[0].name.clone();
    match auth_type {
        LoginAuthType::ApiKey => {
            let key = if rest.is_empty()
                || rest.eq_ignore_ascii_case("api_key")
                || rest == API_KEY_LOGIN_LABEL
                || rest.eq_ignore_ascii_case("oauth")
                || rest == OAUTH_LOGIN_LABEL
            {
                None
            } else {
                Some(rest)
            };
            login_api_key(agent_dir, &provider, key)
        }
        LoginAuthType::Oauth => match login_oauth(agent_dir, &provider) {
            Ok(message) => Ok(format!(
                "{}{message}",
                render_login_dialog(
                    &name,
                    None,
                    Some("Complete login in your browser. If the browser is on another machine, paste the final redirect URL here."),
                )
            )),
            Err(err) if err == login_cancelled_message() => Ok(login_cancelled_message().into()),
            Err(err) => Err(format!("Failed to login to {name}: {err}")),
        },
    }
}

pub fn handle_logout_command(agent_dir: &Path, args: &str) -> Result<String, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        let store = FileAuthStorage::open(auth_path(agent_dir)).map_err(|err| err.to_string())?;
        if store.list().is_empty() {
            return Ok(logout_empty_message().into());
        }
        let mut lines = vec!["Select a provider to log out".into()];
        for id in store.list() {
            lines.push(format!("{} [{}]", provider_display_name(&id), id));
        }
        return Ok(lines.join("\n") + "\n");
    }
    logout_provider(agent_dir, trimmed)
}

pub fn login_cancelled_message() -> &'static str {
    "Login cancelled"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings;
    use tempfile::tempdir;

    #[test]
    fn login_command_lists_methods_and_saves_api_key() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        let listed = handle_login_command(dir.path(), "").unwrap();
        assert!(listed.contains(OAUTH_LOGIN_LABEL));
        assert!(listed.contains(API_KEY_LOGIN_LABEL));
        std::env::set_var("PI_LOGIN_API_KEY", "sk-test");
        let saved = handle_login_command(dir.path(), "anthropic api_key").unwrap();
        assert!(saved.contains("Saved API key for Anthropic"));
        assert!(saved.contains("Credentials saved to"));
        let raw = std::fs::read_to_string(auth_path(dir.path())).unwrap();
        assert!(raw.contains("sk-test"));
        std::env::remove_var("PI_LOGIN_API_KEY");
        let empty = handle_logout_command(dir.path(), "").unwrap();
        assert!(empty.contains("Anthropic"));
        let removed = handle_logout_command(dir.path(), "anthropic").unwrap();
        assert!(removed.contains("Removed stored API key for Anthropic"));
        assert_eq!(
            handle_logout_command(dir.path(), "anthropic").unwrap(),
            logout_empty_message()
        );
    }

    #[test]
    fn oauth_login_uses_token_fixture() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        let token = dir.path().join("token.json");
        std::fs::write(
            &token,
            r#"{"access_token":"tok","refresh_token":"ref","expires_in":3600}"#,
        )
        .unwrap();
        std::env::set_var("PI_OAUTH_CODE", "code#fixture-state");
        std::env::set_var("PI_OAUTH_TOKEN_FIXTURE", &token);
        // login_anthropic uses generated PKCE state, so fixture state must match.
        // Use PI_OAUTH_CALLBACK_URL without state so take_fixture uses expected_state.
        std::env::set_var(
            "PI_OAUTH_CALLBACK_URL",
            "http://localhost/callback?code=abc",
        );
        std::env::remove_var("PI_OAUTH_CODE");
        let saved = login_oauth(dir.path(), "anthropic").unwrap();
        assert!(saved.contains("Logged in to Anthropic"));
        let raw = std::fs::read_to_string(auth_path(dir.path())).unwrap();
        assert!(raw.contains("tok"));
        std::env::remove_var("PI_OAUTH_CALLBACK_URL");
        std::env::remove_var("PI_OAUTH_TOKEN_FIXTURE");
    }
}
