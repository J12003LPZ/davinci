use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

use crate::providers::{provider_spec, ProviderSpec};

#[derive(Debug, Error)]
pub enum AuthStorageError {
    #[error("Unable to read auth.json: {0}")]
    Read(String),
    #[error("Unable to write auth.json: {0}")]
    Write(String),
    #[error("Invalid auth.json: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    ApiKey,
    Oauth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    #[serde(rename = "type")]
    pub kind: CredentialKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(
        default,
        rename = "availableModelIds",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub available_model_ids: Vec<String>,
}

impl Drop for Credential {
    fn drop(&mut self) {
        if let Some(key) = &mut self.key {
            key.zeroize();
        }
        if let Some(access) = &mut self.access {
            access.zeroize();
        }
        if let Some(refresh) = &mut self.refresh {
            refresh.zeroize();
        }
    }
}

/// Whether an OAuth credential is dead by `deadline` (epoch ms). The stored
/// `expires` is authoritative; credentials written before it was recorded fall
/// back to the access token's own `exp` claim, and one that says nothing at all
/// is treated as still good rather than refreshed on every request.
pub fn credential_expires_by(cred: &Credential, deadline: u64) -> bool {
    cred.expires
        .or_else(|| cred.access.as_deref().and_then(crate::codex::jwt_expiry_ms))
        .map(|expires| expires <= deadline)
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct ResolvedAuth {
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
    pub source: String,
}

pub struct AuthStorage {
    path: PathBuf,
    data: HashMap<String, Credential>,
    runtime_overrides: HashMap<String, String>,
}

impl AuthStorage {
    pub fn create() -> Result<Self, AuthStorageError> {
        Self::open(&default_auth_path())
    }

    pub fn in_memory() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("pi-auth-memory-{}.json", std::process::id())),
            data: HashMap::new(),
            runtime_overrides: HashMap::new(),
        }
    }

    pub fn open(path: &Path) -> Result<Self, AuthStorageError> {
        let data = if path.exists() {
            let raw =
                fs::read_to_string(path).map_err(|err| AuthStorageError::Read(err.to_string()))?;
            serde_json::from_str(&raw).map_err(|err| AuthStorageError::Invalid(err.to_string()))?
        } else {
            HashMap::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
            runtime_overrides: HashMap::new(),
        })
    }

    pub fn set_runtime_override(&mut self, provider: &str, key: impl Into<String>) {
        self.runtime_overrides
            .insert(provider.to_string(), key.into());
    }

    pub fn set(&mut self, provider: &str, credential: Credential) -> Result<(), AuthStorageError> {
        self.data.insert(provider.to_string(), credential);
        self.persist()
    }

    pub fn remove(&mut self, provider: &str) -> Result<(), AuthStorageError> {
        self.data.remove(provider);
        self.persist()
    }

    pub fn get(&self, provider: &str) -> Option<&Credential> {
        self.data.get(provider)
    }

    pub fn providers(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    pub fn login_api_key(
        &mut self,
        provider: &str,
        key: impl Into<String>,
    ) -> Result<(), AuthStorageError> {
        self.set(
            provider,
            Credential {
                kind: CredentialKind::ApiKey,
                key: Some(key.into()),
                access: None,
                refresh: None,
                expires: None,
                env: HashMap::new(),
                available_model_ids: Vec::new(),
            },
        )
    }

    pub fn login_oauth(
        &mut self,
        provider: &str,
        access: impl Into<String>,
        refresh: Option<String>,
        expires: Option<u64>,
    ) -> Result<(), AuthStorageError> {
        let access = access.into();
        let available_model_ids = if provider == "github-copilot" {
            fetch_github_copilot_available_model_ids(&access)
        } else {
            Vec::new()
        };
        self.set(
            provider,
            Credential {
                kind: CredentialKind::Oauth,
                key: None,
                access: Some(access),
                refresh,
                expires,
                env: HashMap::new(),
                available_model_ids,
            },
        )
    }

    pub fn maybe_refresh(
        &mut self,
        provider: &str,
        now_ms: u64,
        min_expiry_ms: u64,
        no_refresh: bool,
    ) -> Result<bool, AuthStorageError> {
        if no_refresh {
            return Ok(false);
        }
        let Some(cred) = self.get(provider).cloned() else {
            return Ok(false);
        };
        if cred.kind != CredentialKind::Oauth {
            return Ok(false);
        }
        if !credential_expires_by(&cred, now_ms.saturating_add(min_expiry_ms)) {
            return Ok(false);
        }
        let refresh = cred.refresh.clone().unwrap_or_default();
        let fixture = refresh.starts_with("pi-fixture-")
            || matches!(
                std::env::var("PI_OAUTH_FIXTURE").as_deref(),
                Ok("1") | Ok("true")
            );
        if fixture {
            return self
                .login_oauth(
                    provider,
                    format!("{refresh}-access"),
                    Some(refresh),
                    Some(now_ms.saturating_add(3_600_000)),
                )
                .map(|_| true);
        }
        if let Ok(url) = std::env::var("PI_OAUTH_REFRESH_URL") {
            let body = serde_json::json!({
                "provider": provider,
                "refresh": refresh,
            });
            let response = ureq::post(&url)
                .set("content-type", "application/json")
                .send_string(&body.to_string())
                .map_err(|err| AuthStorageError::Read(err.to_string()))?;
            let text = response
                .into_string()
                .map_err(|err| AuthStorageError::Read(err.to_string()))?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|err| AuthStorageError::Invalid(err.to_string()))?;
            let access = value
                .get("access")
                .or_else(|| value.get("access_token"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AuthStorageError::Invalid("refresh response missing access".into())
                })?;
            let next_refresh = value
                .get("refresh")
                .or_else(|| value.get("refresh_token"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or(Some(refresh));
            let expires = value
                .get("expires")
                .or_else(|| value.get("expires_at"))
                .and_then(|v| v.as_u64())
                .or(Some(now_ms.saturating_add(3_600_000)));
            return self
                .login_oauth(provider, access, next_refresh, expires)
                .map(|_| true);
        }
        // The provider's own refresh grant. Without this a stored OAuth login
        // was good until its token died and then needed a fresh `/login`,
        // which is what "no credential" looked like from the outside.
        if refresh.is_empty() {
            return Ok(false);
        }
        let tokens = crate::oauth_providers::refresh_oauth_token(provider, &refresh)
            .map_err(AuthStorageError::Read)?;
        let expires = tokens
            .expires
            .or_else(|| crate::codex::jwt_expiry_ms(&tokens.access));
        self.login_oauth(provider, tokens.access, tokens.refresh, expires)
            .map(|_| true)
    }

    fn persist(&self) -> Result<(), AuthStorageError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| AuthStorageError::Write(err.to_string()))?;
        }
        let raw = serde_json::to_string_pretty(&self.data)
            .map_err(|err| AuthStorageError::Write(err.to_string()))?;
        fs::write(&self.path, raw).map_err(|err| AuthStorageError::Write(err.to_string()))
    }
}

/// TS `os.homedir()`: `USERPROFILE` on Windows, `HOME` on POSIX (kept as a
/// Windows fallback for MSYS/Git Bash shells).
pub(crate) fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Some(std::path::PathBuf::from(profile));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

pub fn default_auth_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir).join("auth.json");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
        .join("auth.json")
}

pub fn env_api_key(spec: &ProviderSpec, env: &HashMap<String, String>) -> Option<(String, String)> {
    for var in spec.env_vars {
        if let Some(value) = env.get(*var).cloned().filter(|value| !value.is_empty()) {
            return Some((value, (*var).to_string()));
        }
    }
    None
}

pub fn resolve_provider_auth(
    provider: &str,
    storage: &AuthStorage,
    env: &HashMap<String, String>,
    include_env: bool,
) -> Option<ResolvedAuth> {
    if let Some(key) = storage.runtime_overrides.get(provider) {
        return Some(ResolvedAuth {
            api_key: Some(key.clone()),
            headers: HashMap::new(),
            source: "runtime override".into(),
        });
    }
    if let Some(cred) = storage.get(provider) {
        match cred.kind {
            CredentialKind::ApiKey => {
                if let Some(key) = &cred.key {
                    if !key.is_empty() {
                        return Some(ResolvedAuth {
                            api_key: Some(key.clone()),
                            headers: HashMap::new(),
                            source: "stored credential".into(),
                        });
                    }
                }
                if provider == "amazon-bedrock" && env_nonempty(cred.env.get("AWS_PROFILE")) {
                    return Some(ResolvedAuth {
                        api_key: None,
                        headers: HashMap::new(),
                        source: "stored credential".into(),
                    });
                }
                if provider == "llama.cpp" && env_nonempty(cred.env.get("LLAMA_BASE_URL")) {
                    return Some(ResolvedAuth {
                        api_key: cred.key.clone(),
                        headers: HashMap::new(),
                        source: "stored credential".into(),
                    });
                }
                if provider == "google-vertex" {
                    if let Some(resolved) = vertex_ambient_auth(Some(cred), env) {
                        return Some(resolved);
                    }
                }
                if let Some(resolved) = cloudflare_auth(provider, Some(cred), env) {
                    return Some(resolved);
                }
            }
            CredentialKind::Oauth => {
                if let Some(access) = cred.access.clone().or_else(|| cred.key.clone()) {
                    let mut headers = HashMap::new();
                    headers.insert("Authorization".into(), format!("Bearer {access}"));
                    return Some(ResolvedAuth {
                        api_key: Some(access),
                        headers,
                        source: "OAuth".into(),
                    });
                }
            }
        }
    }
    if include_env {
        if provider == "amazon-bedrock" {
            if let Some(source) = bedrock_ambient_source(env) {
                return Some(ResolvedAuth {
                    api_key: None,
                    headers: HashMap::new(),
                    source,
                });
            }
        }
        if provider == "llama.cpp" {
            if let Some(url) = lookup_env("LLAMA_BASE_URL", env) {
                if !url.is_empty() {
                    return Some(ResolvedAuth {
                        api_key: Some(
                            lookup_env("LLAMA_API_KEY", env).unwrap_or_else(|| "local".into()),
                        ),
                        headers: HashMap::new(),
                        source: "LLAMA_BASE_URL".into(),
                    });
                }
            }
        }
        if provider == "google-vertex" {
            if let Some(resolved) = vertex_ambient_auth(None, env) {
                return Some(resolved);
            }
        }
        if let Some(resolved) = cloudflare_auth(provider, None, env) {
            return Some(resolved);
        }
        if provider != "amazon-bedrock"
            && provider != "llama.cpp"
            && provider != "google-vertex"
            && provider != "cloudflare-workers-ai"
            && provider != "cloudflare-ai-gateway"
        {
            if let Some(spec) = provider_spec(provider) {
                if let Some((key, source)) = env_api_key(spec, env) {
                    if provider == "anthropic" && source == "ANTHROPIC_AUTH_TOKEN" {
                        let mut headers = HashMap::new();
                        headers.insert("Authorization".into(), format!("Bearer {key}"));
                        return Some(ResolvedAuth {
                            api_key: None,
                            headers,
                            source,
                        });
                    }
                    return Some(ResolvedAuth {
                        api_key: Some(key),
                        headers: HashMap::new(),
                        source,
                    });
                }
            }
        }
    }
    None
}

fn env_nonempty(value: Option<&String>) -> bool {
    value.is_some_and(|text| !text.is_empty())
}

fn lookup_env(name: &str, env: &HashMap<String, String>) -> Option<String> {
    env.get(name).cloned().filter(|value| !value.is_empty())
}

/// TS `google-vertex` ADC + project + location (no network).
pub fn vertex_ambient_auth(
    credential: Option<&Credential>,
    env: &HashMap<String, String>,
) -> Option<ResolvedAuth> {
    if let Some(key) = credential
        .and_then(|cred| cred.key.clone())
        .filter(|key| !key.is_empty())
    {
        return Some(ResolvedAuth {
            api_key: Some(key),
            headers: HashMap::new(),
            source: "stored credential".into(),
        });
    }
    if let Some(key) = lookup_env("GOOGLE_CLOUD_API_KEY", env) {
        return Some(ResolvedAuth {
            api_key: Some(key),
            headers: HashMap::new(),
            source: "GOOGLE_CLOUD_API_KEY".into(),
        });
    }
    let adc_path = credential
        .and_then(|cred| cred.env.get("GOOGLE_APPLICATION_CREDENTIALS").cloned())
        .or_else(|| lookup_env("GOOGLE_APPLICATION_CREDENTIALS", env))
        .unwrap_or_else(default_vertex_adc_path);
    if !Path::new(&expand_home(&adc_path)).is_file() {
        return None;
    }
    let project = credential
        .and_then(|cred| cred.env.get("GOOGLE_CLOUD_PROJECT").cloned())
        .or_else(|| lookup_env("GOOGLE_CLOUD_PROJECT", env))
        .or_else(|| lookup_env("GCLOUD_PROJECT", env));
    let location = credential
        .and_then(|cred| cred.env.get("GOOGLE_CLOUD_LOCATION").cloned())
        .or_else(|| lookup_env("GOOGLE_CLOUD_LOCATION", env));
    if project.filter(|value| !value.is_empty()).is_some()
        && location.filter(|value| !value.is_empty()).is_some()
    {
        return Some(ResolvedAuth {
            api_key: None,
            headers: HashMap::new(),
            source: if credential.is_some() {
                "stored credential".into()
            } else {
                "gcloud application default credentials".into()
            },
        });
    }
    None
}

pub fn cloudflare_auth(
    provider: &str,
    credential: Option<&Credential>,
    env: &HashMap<String, String>,
) -> Option<ResolvedAuth> {
    let require_gateway = match provider {
        "cloudflare-workers-ai" => false,
        "cloudflare-ai-gateway" => true,
        _ => return None,
    };
    let api_key = credential
        .and_then(|cred| cred.key.clone())
        .filter(|key| !key.is_empty())
        .or_else(|| lookup_env("CLOUDFLARE_API_KEY", env))?;
    credential
        .and_then(|cred| cred.env.get("CLOUDFLARE_ACCOUNT_ID").cloned())
        .or_else(|| lookup_env("CLOUDFLARE_ACCOUNT_ID", env))
        .filter(|value| !value.is_empty())?;
    if require_gateway {
        credential
            .and_then(|cred| cred.env.get("CLOUDFLARE_GATEWAY_ID").cloned())
            .or_else(|| lookup_env("CLOUDFLARE_GATEWAY_ID", env))
            .filter(|value| !value.is_empty())?;
    }
    Some(ResolvedAuth {
        api_key: Some(api_key),
        headers: HashMap::new(),
        source: if credential.is_some() {
            "stored credential".into()
        } else {
            "CLOUDFLARE_API_KEY".into()
        },
    })
}

fn default_vertex_adc_path() -> String {
    "~/.config/gcloud/application_default_credentials.json".into()
}

fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    path.to_string()
}

/// TS `amazon-bedrock` ambient resolve sources (no network).
pub fn bedrock_ambient_source(env: &HashMap<String, String>) -> Option<String> {
    if lookup_env("AWS_BEARER_TOKEN_BEDROCK", env).is_some() {
        return Some("AWS_BEARER_TOKEN_BEDROCK".into());
    }
    if lookup_env("AWS_PROFILE", env).is_some() {
        return Some("AWS_PROFILE".into());
    }
    if lookup_env("AWS_ACCESS_KEY_ID", env).is_some()
        && lookup_env("AWS_SECRET_ACCESS_KEY", env).is_some()
    {
        return Some("AWS access keys".into());
    }
    if lookup_env("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", env).is_some()
        || lookup_env("AWS_CONTAINER_CREDENTIALS_FULL_URI", env).is_some()
    {
        return Some("ECS task role".into());
    }
    if lookup_env("AWS_WEB_IDENTITY_TOKEN_FILE", env).is_some() {
        return Some("web identity token".into());
    }
    None
}

const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
const COPILOT_API_VERSION: &str = "2026-06-01";
const COPILOT_DEFAULT_BASE: &str = "https://api.individual.githubcopilot.com";

/// GitHub Copilot `availableModelIds` after OAuth login/refresh.
/// Fixture `PI_COPILOT_MODELS_REPLY` / `PI_COPILOT_MODELS_URL` first.
/// Tests never hit GitHub; production GETs `{base}/models`.
pub fn copilot_available_model_ids(provider: &str) -> Vec<String> {
    if provider != "github-copilot" {
        return Vec::new();
    }
    fetch_github_copilot_available_model_ids("")
}

pub fn copilot_base_url_from_token(token: &str) -> String {
    if let Some(host) = token
        .split(';')
        .find_map(|part| part.strip_prefix("proxy-ep="))
    {
        let api_host = host.replacen("proxy.", "api.", 1);
        return format!("https://{api_host}");
    }
    if let Ok(base) = std::env::var("PI_COPILOT_BASE_URL") {
        if !base.is_empty() {
            return base;
        }
    }
    COPILOT_DEFAULT_BASE.into()
}

pub fn fetch_github_copilot_available_model_ids(access: &str) -> Vec<String> {
    if let Ok(reply) = std::env::var("PI_COPILOT_MODELS_REPLY") {
        let raw = if Path::new(&reply).is_file() {
            fs::read_to_string(&reply).unwrap_or_default()
        } else {
            reply
        };
        return parse_copilot_available_model_ids(&raw);
    }
    let url = std::env::var("PI_COPILOT_MODELS_URL").unwrap_or_else(|_| {
        format!(
            "{}/models",
            copilot_base_url_from_token(access).trim_end_matches('/')
        )
    });
    if cfg!(test) && !url.starts_with("http://127.0.0.1") && !url.starts_with("http://localhost") {
        return Vec::new();
    }
    if url.is_empty() {
        return Vec::new();
    }
    let response = match ureq::get(&url)
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {access}"))
        .set("user-agent", COPILOT_USER_AGENT)
        .set("editor-version", COPILOT_EDITOR_VERSION)
        .set("editor-plugin-version", COPILOT_PLUGIN_VERSION)
        .set("copilot-integration-id", COPILOT_INTEGRATION_ID)
        .set("x-github-api-version", COPILOT_API_VERSION)
        .timeout(std::time::Duration::from_secs(10))
        .call()
    {
        Ok(response) => response,
        Err(_) => return Vec::new(),
    };
    let text = response.into_string().unwrap_or_default();
    parse_copilot_available_model_ids(&text)
}

pub fn parse_copilot_available_model_ids(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    if let Some(ids) = value.as_array().and_then(|items| {
        items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
    }) {
        return ids;
    }
    let Some(data) = value.get("data").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    let mut picker = Vec::new();
    let mut enabled = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if item
            .pointer("/capabilities/supports/tool_calls")
            .and_then(|v| v.as_bool())
            == Some(false)
        {
            continue;
        }
        let picker_enabled = item
            .get("model_picker_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let policy_state = item
            .get("policy")
            .and_then(|v| v.get("state"))
            .and_then(|v| v.as_str());
        if picker_enabled && policy_state != Some("disabled") {
            picker.push(id.to_string());
        }
        if policy_state == Some("enabled") {
            enabled.push(id.to_string());
        }
    }
    if picker.is_empty() {
        enabled
    } else {
        picker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stored_api_key_wins_over_env() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage
            .set(
                "openai",
                Credential {
                    kind: CredentialKind::ApiKey,
                    key: Some("sk-stored".into()),
                    access: None,
                    refresh: None,
                    expires: None,
                    env: HashMap::new(),
                    available_model_ids: Vec::new(),
                },
            )
            .unwrap();
        let mut env = HashMap::new();
        env.insert("OPENAI_API_KEY".into(), "sk-env".into());
        let resolved = resolve_provider_auth("openai", &storage, &env, true).unwrap();
        assert_eq!(resolved.api_key.as_deref(), Some("sk-stored"));
        assert_eq!(resolved.source, "stored credential");
    }

    #[test]
    fn fixture_oauth_refresh_extends_expiry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage
            .login_oauth(
                "anthropic",
                "expired",
                Some("pi-fixture-refresh".into()),
                Some(1),
            )
            .unwrap();
        assert!(storage
            .maybe_refresh("anthropic", 10_000, 0, false)
            .unwrap());
        let cred = storage.get("anthropic").unwrap();
        assert_eq!(cred.access.as_deref(), Some("pi-fixture-refresh-access"));
        assert!(cred.expires.unwrap() > 10_000);
        assert!(!storage.maybe_refresh("anthropic", 10_000, 0, true).unwrap());
    }

    /// A JWT with the given `exp` (seconds) and nothing else that matters.
    fn jwt_expiring_at(exp_seconds: u64) -> String {
        use base64::Engine;
        let encode = |raw: &str| {
            base64::engine::general_purpose::STANDARD
                .encode(raw)
                .trim_end_matches('=')
                .replace('+', "-")
                .replace('/', "_")
        };
        format!(
            "{}.{}.{}",
            encode(r#"{"alg":"none"}"#),
            encode(&format!(r#"{{"exp":{exp_seconds}}}"#)),
            "sig"
        )
    }

    #[test]
    fn oauth_expiry_falls_back_to_the_access_token_claim() {
        let live = Credential {
            kind: CredentialKind::Oauth,
            key: None,
            access: Some(jwt_expiring_at(2_000)),
            refresh: None,
            expires: None,
            env: HashMap::new(),
            available_model_ids: Vec::new(),
        };
        assert!(credential_expires_by(&live, 2_000_000));
        assert!(!credential_expires_by(&live, 1_999_000));

        // A credential that says nothing about expiry is left alone rather
        // than renewed on every single request.
        let mut opaque = live.clone();
        opaque.access = Some("not-a-jwt".into());
        assert!(!credential_expires_by(&opaque, u64::MAX));
    }

    #[test]
    fn a_credential_stored_without_an_expiry_still_refreshes() {
        // Exactly the shape `/login` used to write: access and refresh, no
        // expiry. The token's own `exp` says it is dead, so the refresh runs
        // instead of the request failing with a token nothing renewed.
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage
            .login_oauth(
                "openai-codex",
                jwt_expiring_at(1_000),
                Some("pi-fixture-refresh".into()),
                None,
            )
            .unwrap();
        assert!(storage
            .maybe_refresh("openai-codex", 2_000_000, 0, false)
            .unwrap());
        let cred = storage.get("openai-codex").unwrap();
        assert_eq!(cred.access.as_deref(), Some("pi-fixture-refresh-access"));
        assert!(cred.expires.unwrap() > 2_000_000);
        assert_eq!(cred.refresh.as_deref(), Some("pi-fixture-refresh"));
    }

    #[test]
    fn a_token_with_room_left_is_not_refreshed_until_the_margin() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage
            .login_oauth(
                "openai-codex",
                "access",
                Some("pi-fixture-refresh".into()),
                Some(1_000_000),
            )
            .unwrap();
        assert!(!storage
            .maybe_refresh("openai-codex", 500_000, 60_000, false)
            .unwrap());
        // Within the five-minute window the TS resolver uses, it renews early.
        assert!(storage
            .maybe_refresh("openai-codex", 700_000, 300_000, false)
            .unwrap());
    }

    #[test]
    fn copilot_base_url_uses_proxy_ep() {
        assert_eq!(
            copilot_base_url_from_token(
                "tid=test;exp=1;proxy-ep=proxy.individual.githubcopilot.com;"
            ),
            "https://api.individual.githubcopilot.com"
        );
    }

    #[test]
    fn copilot_parse_skips_models_without_tool_calls() {
        let ids = parse_copilot_available_model_ids(
            r#"{"data":[{"id":"gpt-4.1","model_picker_enabled":true,"policy":{"state":"enabled"}},{"id":"no-tools","model_picker_enabled":true,"policy":{"state":"enabled"},"capabilities":{"supports":{"tool_calls":false}}}]}"#,
        );
        assert_eq!(ids, vec!["gpt-4.1".to_string()]);
    }

    #[test]
    fn copilot_models_fetch_uses_localhost_override() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"{"data":[{"id":"gpt-4.1","model_picker_enabled":true,"policy":{"state":"enabled"}}]}"#;
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        std::env::set_var("PI_COPILOT_MODELS_URL", format!("http://{addr}/models"));
        let ids = fetch_github_copilot_available_model_ids(
            "tid=test;proxy-ep=proxy.individual.githubcopilot.com;",
        );
        std::env::remove_var("PI_COPILOT_MODELS_URL");
        assert_eq!(ids, vec!["gpt-4.1".to_string()]);
    }
}
