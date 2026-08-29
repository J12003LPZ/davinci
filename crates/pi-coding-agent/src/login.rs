//! Interactive /login and /logout matching TypeScript login dialogs and OAuth.

use crate::login_dialog::{LoginDialog, LoginDialogKind};
use pi_ai::auth::{AuthStorage, Credential, FileAuthStorage};
use pi_ai::oauth::oauth_app;
use pi_ai::oauth_flow::{anthropic_authorize_url, fixture_authorization_input, generate_pkce};
use pi_ai::providers::{all_providers, provider_display_name};
use pi_ai::{login_device_oauth, oauth_login_label, prefers_device_login, DeviceCodeInfo};
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
    render_auth_type_selector_for(None, provider_name)
}

pub fn render_auth_type_selector_for(
    provider_id: Option<&str>,
    provider_name: Option<&str>,
) -> String {
    let oauth_label = provider_id
        .and_then(oauth_login_label)
        .unwrap_or(OAUTH_LOGIN_LABEL);
    let title = if let Some(name) = provider_name {
        format!("Select authentication method for {name}:")
    } else {
        "Select authentication method:".into()
    };
    format!("1. {oauth_label}\n2. {API_KEY_LOGIN_LABEL}\n{title}\n")
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

pub fn render_device_login_dialog(provider_name: &str, info: &DeviceCodeInfo) -> String {
    let mut dialog = LoginDialog::new("", provider_name, LoginDialogKind::Device);
    dialog.show_device_code(info);
    dialog.show_waiting("Waiting for authentication...");
    dialog.render()
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

pub fn login_device(agent_dir: &Path, provider: &str) -> Result<String, String> {
    let name = provider_display_name(provider);
    let enterprise = std::env::var("PI_LOGIN_ENTERPRISE_DOMAIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (credential, info) = login_device_oauth(provider, enterprise.as_deref())?;
    let path = store_credential(agent_dir, provider, credential)?;
    Ok(format!(
        "{}Logged in to {name}. Credentials saved to {}\nEnter code: {}",
        render_device_login_dialog(name, &info),
        path.display(),
        info.user_code
    ))
}

pub fn login_oauth(agent_dir: &Path, provider: &str) -> Result<String, String> {
    if prefers_device_login(provider, None) {
        return login_device(agent_dir, provider);
    }
    let name = provider_display_name(provider);
    let pkce = generate_pkce();
    if provider == "openrouter" {
        let options = pi_ai::callback_options_for(provider, &pkce.verifier);
        let callback_url = format!("http://{}:0{}", options.host, options.callback_path);
        let url = pi_ai::openrouter_authorize_url(&pkce, &callback_url);
        open_browser(&url);
        let callback = pi_ai::wait_for_oauth_callback_with(&pkce.verifier, &options)?;
        let credential = pi_ai::exchange_openrouter_code(&callback.code, &pkce.verifier)
            .map_err(|err| err.to_string())?;
        let path = store_credential(agent_dir, provider, credential)?;
        return Ok(format!(
            "Logged in to {name}. Credentials saved to {}\nListening for OpenRouter OAuth callback on {callback_url}",
            path.display()
        ));
    }
    let options = pi_ai::callback_options_for(provider, &pkce.verifier);
    let (url, token_url, client_id, extra) = if provider == "anthropic" {
        (
            anthropic_authorize_url(&pkce),
            pi_ai::oauth_flow::ANTHROPIC_TOKEN_URL.to_string(),
            pi_ai::oauth_flow::ANTHROPIC_CLIENT_ID.to_string(),
            Some(pkce.verifier.clone()),
        )
    } else if matches!(provider, "openai-codex" | "openai") {
        (
            pi_ai::openai_codex_authorize_url(&pkce, &pkce.verifier),
            pi_ai::oauth_flow::OPENAI_CODEX_TOKEN_URL.to_string(),
            pi_ai::oauth_flow::OPENAI_CODEX_CLIENT_ID.to_string(),
            None,
        )
    } else if provider == "radius" {
        let app = oauth_app(provider).ok_or_else(|| "No login methods available.".to_string())?;
        (
            pi_ai::radius_browser_authorize_url(&pkce, &pkce.verifier)?,
            app.token_url,
            app.client_id,
            None,
        )
    } else {
        let app = oauth_app(provider).ok_or_else(|| "No login methods available.".to_string())?;
        let authorize = pi_ai::authorize_url(provider, &options.redirect_uri, &pkce.verifier)
            .unwrap_or_else(|| anthropic_authorize_url(&pkce));
        (authorize, app.token_url, app.client_id, None)
    };
    open_browser(&url);
    let callback = pi_ai::wait_for_oauth_callback_with(&pkce.verifier, &options)?;
    let mut body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": client_id,
        "code": callback.code,
        "redirect_uri": options.redirect_uri,
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
        options.redirect_uri
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
        LoginAuthType::Oauth => {
            if prefers_device_login(&provider, Some(rest).filter(|value| !value.is_empty())) {
                match login_device(agent_dir, &provider) {
                    Ok(message) => return Ok(message),
                    Err(err) if err == login_cancelled_message() => {
                        return Ok(login_cancelled_message().into())
                    }
                    Err(err) => return Err(format!("Failed to login to {name}: {err}")),
                }
            }
            match login_oauth(agent_dir, &provider) {
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
            }
        }
    }
}

#[derive(Debug)]
pub enum LoginStart {
    Message(String),
    Dialog(Box<LoginDialog>),
}

pub fn begin_interactive_login(agent_dir: &Path, args: &str) -> Result<LoginStart, String> {
    let trimmed = args.trim();
    if trimmed.is_empty()
        || trimmed == OAUTH_LOGIN_LABEL
        || trimmed == "1"
        || trimmed.eq_ignore_ascii_case("oauth")
        || trimmed == API_KEY_LOGIN_LABEL
        || trimmed == "2"
        || trimmed.eq_ignore_ascii_case("api_key")
    {
        return handle_login_command(agent_dir, args).map(LoginStart::Message);
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
    if ids.len() > 1 || (matches.len() > 1 && rest.is_empty()) {
        return handle_login_command(agent_dir, args).map(LoginStart::Message);
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
            if key.is_some() || std::env::var("PI_LOGIN_API_KEY").is_ok() {
                return login_api_key(agent_dir, &provider, key).map(LoginStart::Message);
            }
            let mut dialog = LoginDialog::new(&provider, &name, LoginDialogKind::ApiKey)
                .with_title(format!("Login to {name}"));
            if provider == "amazon-bedrock" {
                dialog.show_details(&[
                    "You can also use an AWS profile, IAM keys, or role-based credentials.".into(),
                    "See:".into(),
                ]);
                dialog.show_info("Authentication can also use the environment.", &[], false);
            }
            dialog.show_prompt("Enter API key", None);
            Ok(LoginStart::Dialog(Box::new(dialog)))
        }
        LoginAuthType::Oauth => {
            if prefers_device_login(&provider, Some(rest).filter(|value| !value.is_empty()))
                && (std::env::var("PI_OAUTH_DEVICE_FIXTURE").is_ok()
                    || std::env::var("PI_OAUTH_TOKEN_FIXTURE").is_ok())
            {
                return login_device(agent_dir, &provider).map(LoginStart::Message);
            }
            if prefers_device_login(&provider, Some(rest).filter(|value| !value.is_empty())) {
                let enterprise = std::env::var("PI_LOGIN_ENTERPRISE_DOMAIN")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                match pi_ai::start_device_authorization(&provider, enterprise.as_deref()) {
                    Ok(info) => {
                        let mut dialog =
                            LoginDialog::new(&provider, &name, LoginDialogKind::Device);
                        dialog.show_device_code(&info);
                        dialog.show_waiting("Waiting for authentication...");
                        return Ok(LoginStart::Dialog(Box::new(dialog)));
                    }
                    Err(_) if enterprise.is_none() && provider == "github-copilot" => {
                        let mut dialog =
                            LoginDialog::new(&provider, &name, LoginDialogKind::Device);
                        dialog.show_prompt(
                            "GitHub Enterprise URL/domain (blank for github.com)",
                            Some("company.ghe.com"),
                        );
                        return Ok(LoginStart::Dialog(Box::new(dialog)));
                    }
                    Err(err) => return Err(format!("Failed to login to {name}: {err}")),
                }
            }
            if fixture_authorization_input().is_some()
                || std::env::var("PI_OAUTH_TOKEN_FIXTURE").is_ok()
            {
                return login_oauth(agent_dir, &provider).map(LoginStart::Message);
            }
            let mut dialog = LoginDialog::new(&provider, &name, LoginDialogKind::OauthPaste);
            let pkce = generate_pkce();
            dialog.oauth_state = Some(pkce.verifier.clone());
            let options = pi_ai::callback_options_for(&provider, &pkce.verifier);
            let url = if provider == "anthropic" {
                anthropic_authorize_url(&pkce)
            } else if provider == "openrouter" {
                pi_ai::openrouter_authorize_url(
                    &pkce,
                    &format!("http://{}:0{}", options.host, options.callback_path),
                )
            } else if matches!(provider.as_str(), "openai-codex" | "openai") {
                pi_ai::openai_codex_authorize_url(&pkce, &pkce.verifier)
            } else if provider == "radius" {
                pi_ai::radius_browser_authorize_url(&pkce, &pkce.verifier)?
            } else {
                pi_ai::authorize_url(&provider, &options.redirect_uri, &pkce.verifier)
                    .unwrap_or_else(|| anthropic_authorize_url(&pkce))
            };
            open_browser(&url);
            let listen = if provider == "openrouter" {
                format!(
                    "Listening for OpenRouter OAuth callback on http://{}:0{}",
                    options.host, options.callback_path
                )
            } else {
                format!("Listening for OAuth callback on {}", options.redirect_uri)
            };
            dialog.show_progress(&listen);
            dialog.show_auth(
                &url,
                Some("Complete login in your browser. If the browser is on another machine, paste the final redirect URL here."),
            );
            dialog.show_manual_input(
                "Complete login in your browser, or paste the authorization code / redirect URL here:",
            );
            Ok(LoginStart::Dialog(Box::new(dialog)))
        }
    }
}

pub fn login_oauth_with_input(
    agent_dir: &Path,
    provider: &str,
    input: &str,
    expected_state: Option<&str>,
) -> Result<String, String> {
    let name = provider_display_name(provider);
    let pkce = generate_pkce();
    let state = expected_state.unwrap_or(&pkce.verifier);
    if provider == "openrouter" {
        let code = pi_ai::parse_openrouter_authorization_input(input)
            .ok_or_else(|| "Missing authorization code".to_string())?;
        let credential =
            pi_ai::exchange_openrouter_code(&code, state).map_err(|err| err.to_string())?;
        let path = store_credential(agent_dir, provider, credential)?;
        return Ok(format!(
            "Logged in to {name}. Credentials saved to {}",
            path.display()
        ));
    }
    let (code, submitted_state) = pi_ai::parse_authorization_input(input);
    let code = code.ok_or_else(|| "Missing authorization code".to_string())?;
    let submitted_state = submitted_state.unwrap_or_else(|| state.to_string());
    if submitted_state != state {
        return Err("OAuth state mismatch".into());
    }
    let options = pi_ai::callback_options_for(provider, state);
    let (token_url, client_id, include_state) = if provider == "anthropic" {
        (
            pi_ai::oauth_flow::ANTHROPIC_TOKEN_URL.to_string(),
            pi_ai::oauth_flow::ANTHROPIC_CLIENT_ID.to_string(),
            true,
        )
    } else {
        let app = oauth_app(provider).ok_or_else(|| "No login methods available.".to_string())?;
        (app.token_url, app.client_id, false)
    };
    let mut body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": client_id,
        "code": code,
        "redirect_uri": options.redirect_uri,
        "code_verifier": state,
    });
    if include_state {
        body["state"] = serde_json::Value::String(submitted_state);
    }
    let credential = pi_ai::oauth_flow::exchange_authorization_code(&token_url, &body)
        .map_err(|err| err.to_string())?;
    let path = store_credential(agent_dir, provider, credential)?;
    Ok(format!(
        "Logged in to {name}. Credentials saved to {}",
        path.display()
    ))
}

pub fn complete_login_dialog(
    agent_dir: &Path,
    dialog: &LoginDialog,
    submitted: &str,
) -> Result<String, String> {
    match dialog.kind {
        LoginDialogKind::ApiKey => login_api_key(agent_dir, &dialog.provider_id, Some(submitted)),
        LoginDialogKind::OauthPaste => login_oauth_with_input(
            agent_dir,
            &dialog.provider_id,
            submitted,
            dialog.oauth_state.as_deref(),
        ),
        LoginDialogKind::Device if dialog.provider_id == "github-copilot" => login_device_oauth(
            &dialog.provider_id,
            Some(submitted.trim()).filter(|value| !value.is_empty()),
        )
        .and_then(|(credential, info)| {
            let path = store_credential(agent_dir, &dialog.provider_id, credential)?;
            Ok(format!(
                "{}Logged in to {}. Credentials saved to {}",
                render_device_login_dialog(provider_display_name(&dialog.provider_id), &info),
                provider_display_name(&dialog.provider_id),
                path.display()
            ))
        }),
        LoginDialogKind::Device => login_device(agent_dir, &dialog.provider_id),
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
    use pi_tui::keys::Key;
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

    #[test]
    fn openrouter_oauth_exchanges_key_fixture() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        let token = dir.path().join("or.json");
        std::fs::write(&token, r#"{"key":"or-key"}"#).unwrap();
        std::env::set_var("PI_OAUTH_TOKEN_FIXTURE", &token);
        std::env::set_var(
            "PI_OAUTH_CALLBACK_URL",
            "http://127.0.0.1/oauth/callback?code=abc",
        );
        let saved = login_oauth(dir.path(), "openrouter").unwrap();
        assert!(saved.contains("Logged in to OpenRouter"));
        assert!(saved.contains("Listening for OpenRouter OAuth callback"));
        let raw = std::fs::read_to_string(auth_path(dir.path())).unwrap();
        assert!(raw.contains("or-key"));
        std::env::remove_var("PI_OAUTH_CALLBACK_URL");
        std::env::remove_var("PI_OAUTH_TOKEN_FIXTURE");
    }

    #[test]
    fn device_code_login_uses_fixtures() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        let device = dir.path().join("device.json");
        std::fs::write(
            &device,
            r#"{"device_code":"dev","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":1}"#,
        )
        .unwrap();
        let token = dir.path().join("token.json");
        std::fs::write(
            &token,
            r#"{"access_token":"gh-tok","refresh_token":"gh-ref","expires_in":3600}"#,
        )
        .unwrap();
        std::env::set_var("PI_OAUTH_DEVICE_FIXTURE", &device);
        std::env::set_var("PI_OAUTH_TOKEN_FIXTURE", &token);
        std::env::set_var("PI_OAUTH_DEVICE_SLEEP_MS", "0");
        let saved = handle_login_command(dir.path(), "github-copilot oauth").unwrap();
        assert!(saved.contains("Enter code: ABCD-1234"));
        assert!(saved.contains("Waiting for authentication..."));
        assert!(saved.contains("Logged in to GitHub Copilot"));
        let raw = std::fs::read_to_string(auth_path(dir.path())).unwrap();
        assert!(raw.contains("gh-tok"));
        std::env::remove_var("PI_OAUTH_DEVICE_FIXTURE");
        std::env::remove_var("PI_OAUTH_TOKEN_FIXTURE");
        std::env::remove_var("PI_OAUTH_DEVICE_SLEEP_MS");
    }

    #[test]
    fn radius_browser_login_uses_discovery_fixture() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        let token = dir.path().join("token.json");
        std::fs::write(
            &token,
            r#"{"access_token":"rad-tok","refresh_token":"rad-ref","expires_in":3600}"#,
        )
        .unwrap();
        std::env::set_var(
            "PI_RADIUS_OAUTH_DISCOVERY_FIXTURE",
            r#"{"authorizationEndpoint":"https://auth.example/authorize"}"#,
        );
        std::env::set_var("PI_OAUTH_TOKEN_FIXTURE", &token);
        std::env::set_var(
            "PI_OAUTH_CALLBACK_URL",
            "http://127.0.0.1/oauth/callback?code=abc",
        );
        std::env::set_var("PI_OAUTH_LOGIN_METHOD", "browser");
        std::env::set_var("PI_DISABLE_NETWORK", "1");
        let saved = login_oauth(dir.path(), "radius").unwrap();
        assert!(saved.contains("Logged in to Radius"));
        let raw = std::fs::read_to_string(auth_path(dir.path())).unwrap();
        assert!(raw.contains("rad-tok"));
        std::env::remove_var("PI_RADIUS_OAUTH_DISCOVERY_FIXTURE");
        std::env::remove_var("PI_OAUTH_TOKEN_FIXTURE");
        std::env::remove_var("PI_OAUTH_CALLBACK_URL");
        std::env::remove_var("PI_OAUTH_LOGIN_METHOD");
        std::env::remove_var("PI_DISABLE_NETWORK");
    }

    #[test]
    fn interactive_api_key_login_opens_focused_input_dialog() {
        let _lock = settings::test_env_lock();
        std::env::remove_var("PI_LOGIN_API_KEY");
        let dir = tempdir().unwrap();
        match begin_interactive_login(dir.path(), "anthropic api_key").unwrap() {
            LoginStart::Dialog(dialog) => {
                let mut dialog = *dialog;
                let rendered = dialog.render();
                assert!(rendered.contains("Login to Anthropic"));
                assert!(rendered.contains("Enter API key"));
                assert!(dialog.focused());
                assert_eq!(
                    dialog.handle_key(&Key::Escape),
                    crate::login_dialog::LoginDialogAction::Cancelled
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
