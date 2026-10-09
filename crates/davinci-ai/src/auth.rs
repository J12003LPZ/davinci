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

pub const ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE: &str =
    "Anthropic OAuth credentials are not supported by davinci; run `/login anthropic <api-key>`";

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
        for value in self.env.values_mut() {
            value.zeroize();
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
        Self::open(&try_default_auth_path()?)
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
        self.write_change(provider, Some(credential))
    }

    pub fn remove(&mut self, provider: &str) -> Result<(), AuthStorageError> {
        self.write_change(provider, None)
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
        self.set(
            provider,
            Self::oauth_credential(provider, access.into(), refresh, expires),
        )
    }

    pub fn login_openai_siwc(
        &mut self,
        session: crate::openai_siwc::OpenAiSiwcSession,
    ) -> Result<(), AuthStorageError> {
        let mut credential = Self::oauth_credential(
            "openai-codex",
            session.tokens.access.clone(),
            session.tokens.refresh.clone(),
            session.tokens.expires,
        );
        credential.env = crate::openai_siwc::credential_metadata(&session);
        self.set("openai-codex", credential)
    }

    fn oauth_credential(
        provider: &str,
        access: String,
        refresh: Option<String>,
        expires: Option<u64>,
    ) -> Credential {
        let available_model_ids = if provider == "github-copilot" {
            fetch_github_copilot_available_model_ids(&access)
        } else {
            Vec::new()
        };
        Credential {
            kind: CredentialKind::Oauth,
            key: None,
            access: Some(access),
            refresh,
            expires,
            env: HashMap::new(),
            available_model_ids,
        }
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
        let Some(snapshot) = self.get(provider).cloned() else {
            return Ok(false);
        };
        if snapshot.kind != CredentialKind::Oauth {
            return Ok(false);
        }
        if provider == "anthropic" {
            return Err(AuthStorageError::Invalid(
                ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE.into(),
            ));
        }
        if !credential_expires_by(&snapshot, now_ms.saturating_add(min_expiry_ms)) {
            return Ok(false);
        }

        let _lock = self.lock()?;
        self.data = self.read_disk()?;
        let Some(cred) = self.get(provider).cloned() else {
            return Ok(false);
        };
        if cred.kind != CredentialKind::Oauth {
            return Ok(false);
        }
        if !credential_expires_by(&cred, now_ms.saturating_add(min_expiry_ms)) {
            return Ok(true);
        }

        let refresh = cred.refresh.clone().unwrap_or_default();
        if provider == "openai-codex" {
            crate::openai_siwc::validate_credential(&cred).map_err(AuthStorageError::Invalid)?;
            if refresh.is_empty() {
                return Ok(false);
            }
            let registration =
                crate::openai_siwc::registration_from_credential(&cred).ok_or_else(|| {
                    AuthStorageError::Invalid(
                        "stored ChatGPT registration metadata is missing".into(),
                    )
                })?;
            let tokens =
                crate::openai_siwc::refresh_access_token(&registration.client_id, &refresh)
                    .map_err(AuthStorageError::Read)?;
            let mut credential =
                Self::oauth_credential(provider, tokens.access, tokens.refresh, tokens.expires);
            credential.env = cred.env.clone();
            self.store_locked(provider, Some(credential))?;
            return Ok(true);
        }
        let fixture = crate::fixtures::enabled()
            && (refresh.starts_with("pi-fixture-")
                || matches!(
                    std::env::var("PI_OAUTH_FIXTURE").as_deref(),
                    Ok("1") | Ok("true")
                ));
        if fixture {
            let credential = Self::oauth_credential(
                provider,
                format!("{refresh}-access"),
                Some(refresh),
                Some(now_ms.saturating_add(3_600_000)),
            );
            self.store_locked(provider, Some(credential))?;
            return Ok(true);
        }
        let refresh_url = crate::fixtures::enabled()
            .then(|| std::env::var("PI_OAUTH_REFRESH_URL").ok())
            .flatten();
        if let Some(url) = refresh_url {
            let body = serde_json::json!({
                "provider": provider,
                "refresh": refresh,
            });
            let response = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
                .post(&url)
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
                .and_then(|value| value.as_str())
                .ok_or_else(|| AuthStorageError::Invalid("refresh response missing access".into()))?
                .to_string();
            let next_refresh = value
                .get("refresh")
                .or_else(|| value.get("refresh_token"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or(Some(refresh));
            let expires = value
                .get("expires")
                .or_else(|| value.get("expires_at"))
                .and_then(|value| value.as_u64())
                .or(Some(now_ms.saturating_add(3_600_000)));
            let credential = Self::oauth_credential(provider, access, next_refresh, expires);
            self.store_locked(provider, Some(credential))?;
            return Ok(true);
        }
        if refresh.is_empty() {
            return Ok(false);
        }
        let tokens = crate::oauth_providers::refresh_oauth_token(provider, &refresh)
            .map_err(AuthStorageError::Read)?;
        let expires = tokens
            .expires
            .or_else(|| crate::codex::jwt_expiry_ms(&tokens.access));
        let credential = Self::oauth_credential(provider, tokens.access, tokens.refresh, expires);
        self.store_locked(provider, Some(credential))?;
        Ok(true)
    }

    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    fn lock(&self) -> Result<davinci_sys::lock::ExclusiveFileLock, AuthStorageError> {
        const AUTH_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
        davinci_sys::lock::ExclusiveFileLock::acquire(&self.lock_path(), AUTH_LOCK_WAIT)
            .map_err(|err| AuthStorageError::Write(format!("auth.json is busy: {err}")))
    }

    fn read_disk(&self) -> Result<HashMap<String, Credential>, AuthStorageError> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => {
                serde_json::from_str(&raw).map_err(|err| AuthStorageError::Invalid(err.to_string()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(err) => Err(AuthStorageError::Read(err.to_string())),
        }
    }

    fn write_change(
        &mut self,
        provider: &str,
        credential: Option<Credential>,
    ) -> Result<(), AuthStorageError> {
        let _lock = self.lock()?;
        self.store_locked(provider, credential)
    }

    fn store_locked(
        &mut self,
        provider: &str,
        credential: Option<Credential>,
    ) -> Result<(), AuthStorageError> {
        let mut data = self.read_disk()?;
        match credential {
            Some(credential) => {
                data.insert(provider.to_string(), credential);
            }
            None => {
                data.remove(provider);
            }
        }
        let raw = serde_json::to_string_pretty(&data)
            .map_err(|err| AuthStorageError::Write(err.to_string()))?;
        davinci_sys::fs::atomic_write_private(&self.path, raw.as_bytes())
            .map_err(|err| AuthStorageError::Write(err.to_string()))?;
        self.data = data;
        Ok(())
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

/// Display form of the credential path. Code that reads or writes
/// credentials uses [`try_default_auth_path`].
pub fn default_auth_path() -> PathBuf {
    try_default_auth_path().unwrap_or_else(|_| PathBuf::from(".davinci/agent/auth.json"))
}

/// The credential file. Without a home directory this fails instead of
/// falling back to the working directory, which is usually a repository
/// where `auth.json` could be committed.
pub fn try_default_auth_path() -> Result<PathBuf, AuthStorageError> {
    let override_dir = std::env::var("DAVINCI_CODING_AGENT_DIR")
        .or_else(|_| std::env::var("PI_CODING_AGENT_DIR"))
        .ok();
    resolve_auth_path(home_dir().as_deref(), override_dir.as_deref())
}

fn resolve_auth_path(
    home: Option<&Path>,
    override_dir: Option<&str>,
) -> Result<PathBuf, AuthStorageError> {
    let needs_home = override_dir.is_none_or(|dir| dir == "~" || dir.starts_with("~/"));
    match home {
        Some(home) => Ok(auth_path_for(home, override_dir)),
        None if !needs_home => Ok(auth_path_for(Path::new(""), override_dir)),
        None => Err(AuthStorageError::Read(
            "no home directory is set (HOME or USERPROFILE); set DAVINCI_CODING_AGENT_DIR to choose where credentials are stored"
                .into(),
        )),
    }
}

fn auth_path_for(home: &Path, override_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = override_dir {
        let dir = if dir == "~" {
            home.to_path_buf()
        } else if let Some(rest) = dir.strip_prefix("~/") {
            home.join(rest)
        } else {
            PathBuf::from(dir)
        };
        return dir.join("auth.json");
    }
    let current = home.join(".davinci/agent/auth.json");
    let legacy = home.join(".pi/agent/auth.json");
    // A newly created settings/catalog directory must not hide existing logins.
    if current.exists() || !legacy.exists() {
        current
    } else {
        legacy
    }
}

pub fn env_api_key(spec: &ProviderSpec, env: &HashMap<String, String>) -> Option<(String, String)> {
    for var in spec.env_vars {
        if let Some(value) = env.get(*var).cloned().filter(|value| !value.is_empty()) {
            return Some((value, (*var).to_string()));
        }
    }
    None
}

pub(crate) fn oauth_credential_usable(provider: &str, credential: &Credential) -> bool {
    if provider == "anthropic" {
        return false;
    }
    if provider == "openai-codex" {
        return crate::openai_siwc::validate_credential(credential).is_ok();
    }
    true
}

/// Why a stored credential is refused, so callers can say more than "no
/// credential". `None` when nothing is stored or the credential is usable.
/// A legacy Codex login is the common case: it predates Sign in with ChatGPT
/// and stops resolving after an upgrade, which otherwise looks like the
/// account vanished.
pub fn stored_credential_problem(provider: &str, storage: &AuthStorage) -> Option<String> {
    let credential = storage.get(provider)?;
    match (provider, &credential.kind) {
        ("openai-codex", CredentialKind::Oauth) => {
            crate::openai_siwc::validate_credential(credential).err()
        }
        ("openai-codex", CredentialKind::ApiKey) => {
            Some("an API key cannot use the ChatGPT plan; sign in again with ChatGPT".into())
        }
        _ => None,
    }
}

pub fn resolve_provider_auth(
    provider: &str,
    storage: &AuthStorage,
    env: &HashMap<String, String>,
    include_env: bool,
) -> Option<ResolvedAuth> {
    if provider != "openai-codex" {
        if let Some(key) = storage.runtime_overrides.get(provider) {
            return Some(ResolvedAuth {
                api_key: Some(key.clone()),
                headers: HashMap::new(),
                source: "runtime override".into(),
            });
        }
    }
    if let Some(cred) = storage.get(provider) {
        match cred.kind {
            CredentialKind::ApiKey => {
                if provider == "openai-codex" {
                    return None;
                }
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
                if !oauth_credential_usable(provider, cred) {
                    return None;
                }
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
            if let Some(token) = lookup_env("AWS_BEARER_TOKEN_BEDROCK", env) {
                let mut headers = HashMap::new();
                headers.insert("Authorization".into(), format!("Bearer {token}"));
                return Some(ResolvedAuth {
                    api_key: None,
                    headers,
                    source: "AWS_BEARER_TOKEN_BEDROCK".into(),
                });
            }
            // Static keys are SigV4-signed per request (`cloud_auth`).
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
    // Only ADC kinds `cloud_auth` can exchange for an access token count.
    if !crate::cloud_auth::supported_adc(Path::new(&expand_home(&adc_path))) {
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

const URL_VAR_HEADER_PREFIX: &str = "x-davinci-url-var-";

/// Internal carrier for values that belong in the request URL, not on the
/// wire; request builders must skip these entries when sending headers.
pub(crate) fn is_url_var_header(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with(URL_VAR_HEADER_PREFIX)
}

/// `(VARIABLE_NAME, value)` pairs carried by `auth` for URL templates.
pub(crate) fn url_vars(auth: &ResolvedAuth) -> Vec<(String, &str)> {
    auth.headers
        .iter()
        .filter(|(name, _)| is_url_var_header(name))
        .map(|(name, value)| {
            (
                name[URL_VAR_HEADER_PREFIX.len()..].to_ascii_uppercase(),
                value.as_str(),
            )
        })
        .collect()
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
    let mut headers = HashMap::new();
    let account_id = credential
        .and_then(|cred| cred.env.get("CLOUDFLARE_ACCOUNT_ID").cloned())
        .or_else(|| lookup_env("CLOUDFLARE_ACCOUNT_ID", env))
        .filter(|value| !value.is_empty())?;
    headers.insert(
        format!("{URL_VAR_HEADER_PREFIX}CLOUDFLARE_ACCOUNT_ID"),
        account_id,
    );
    if require_gateway {
        let gateway_id = credential
            .and_then(|cred| cred.env.get("CLOUDFLARE_GATEWAY_ID").cloned())
            .or_else(|| lookup_env("CLOUDFLARE_GATEWAY_ID", env))
            .filter(|value| !value.is_empty())?;
        headers.insert(
            format!("{URL_VAR_HEADER_PREFIX}CLOUDFLARE_GATEWAY_ID"),
            gateway_id,
        );
    }
    let source = if credential.is_some() {
        "stored credential".into()
    } else {
        "CLOUDFLARE_API_KEY".into()
    };
    if require_gateway {
        // The gateway token has its own header; `Authorization` is the
        // upstream provider's slot and may be forwarded upstream.
        headers.insert("cf-aig-authorization".into(), format!("Bearer {api_key}"));
        return Some(ResolvedAuth {
            api_key: None,
            headers,
            source,
        });
    }
    Some(ResolvedAuth {
        api_key: Some(api_key),
        headers,
        source,
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

/// Ambient Bedrock credentials that can authenticate a request (no network):
/// the bearer token, or static keys (environment, or an `AWS_PROFILE` with
/// keys in the shared credentials file) that `cloud_auth` SigV4-signs.
/// Role-based sources (ECS task role, web identity) would need an STS
/// exchange, so they do not make the provider usable.
pub fn bedrock_ambient_source(env: &HashMap<String, String>) -> Option<String> {
    if lookup_env("AWS_BEARER_TOKEN_BEDROCK", env).is_some() {
        return Some("AWS_BEARER_TOKEN_BEDROCK".into());
    }
    if lookup_env("AWS_ACCESS_KEY_ID", env).is_some()
        && lookup_env("AWS_SECRET_ACCESS_KEY", env).is_some()
    {
        return Some("AWS access keys".into());
    }
    if lookup_env("AWS_PROFILE", env).is_some()
        && crate::cloud_auth::aws_credentials(env)
            .ok()
            .flatten()
            .is_some()
    {
        return Some("AWS_PROFILE".into());
    }
    None
}

pub(crate) const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Editor identity headers GitHub expects on Copilot token and API calls.
pub(crate) fn copilot_headers() -> [(&'static str, &'static str); 4] {
    [
        ("user-agent", COPILOT_USER_AGENT),
        ("editor-version", COPILOT_EDITOR_VERSION),
        ("editor-plugin-version", COPILOT_PLUGIN_VERSION),
        ("copilot-integration-id", COPILOT_INTEGRATION_ID),
    ]
}
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
    let response = match crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .get(&url)
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {access}"))
        .set("user-agent", COPILOT_USER_AGENT)
        .set("editor-version", COPILOT_EDITOR_VERSION)
        .set("editor-plugin-version", COPILOT_PLUGIN_VERSION)
        .set("copilot-integration-id", COPILOT_INTEGRATION_ID)
        .set("x-github-api-version", COPILOT_API_VERSION)
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
    fn two_handles_do_not_lose_each_others_logins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut first = AuthStorage::open(&path).unwrap();
        let mut second = AuthStorage::open(&path).unwrap();
        first.login_api_key("openai", "sk-a").unwrap();
        second.login_api_key("anthropic", "sk-b").unwrap();
        let fresh = AuthStorage::open(&path).unwrap();
        assert!(fresh.get("openai").is_some());
        assert!(fresh.get("anthropic").is_some());
    }

    #[test]
    fn refresh_adopts_a_token_another_process_already_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let now = 1_000_000;
        let mut stale = AuthStorage::open(&path).unwrap();
        stale
            .login_oauth(
                "openai-codex",
                "old-access",
                Some("pi-fixture-r1".into()),
                Some(now),
            )
            .unwrap();
        let mut other = AuthStorage::open(&path).unwrap();
        other
            .login_oauth(
                "openai-codex",
                "new-access",
                Some("pi-fixture-r2".into()),
                Some(now + 3_600_000),
            )
            .unwrap();
        assert!(stale
            .maybe_refresh("openai-codex", now, 60_000, false)
            .unwrap());
        assert_eq!(
            stale.get("openai-codex").unwrap().access.as_deref(),
            Some("new-access")
        );
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage.login_api_key("openai", "sk").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn fixture_hooks_are_on_in_test_builds() {
        assert!(crate::fixtures::enabled());
    }

    #[test]
    fn auth_path_keeps_legacy_logins_when_current_catalog_directory_exists() {
        let home = tempdir().unwrap();
        let current = home.path().join(".davinci/agent/auth.json");
        let legacy = home.path().join(".pi/agent/auth.json");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        assert_eq!(auth_path_for(home.path(), None), current);
        fs::write(&legacy, "{}").unwrap();
        assert_eq!(auth_path_for(home.path(), None), legacy);
        assert_eq!(
            auth_path_for(home.path(), Some("~/isolated")),
            home.path().join("isolated/auth.json")
        );
        fs::write(&current, "{}").unwrap();
        assert_eq!(auth_path_for(home.path(), None), current);
    }

    #[test]
    fn missing_home_never_puts_credentials_in_the_working_directory() {
        let message = resolve_auth_path(None, None).unwrap_err().to_string();
        assert!(
            message.contains("set DAVINCI_CODING_AGENT_DIR to choose"),
            "{message}"
        );
        assert!(!message.contains("  "), "{message}");
        assert!(resolve_auth_path(None, Some("~")).is_err());
        assert!(resolve_auth_path(None, Some("~/agent")).is_err());
        assert_eq!(
            resolve_auth_path(None, Some("/srv/davinci")).unwrap(),
            PathBuf::from("/srv/davinci/auth.json")
        );
        let home = tempdir().unwrap();
        assert_eq!(
            resolve_auth_path(Some(home.path()), None).unwrap(),
            home.path().join(".davinci/agent/auth.json")
        );
    }

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
    fn stored_credential_problem_names_a_legacy_codex_login() {
        let mut storage = AuthStorage::in_memory();
        assert_eq!(stored_credential_problem("openai-codex", &storage), None);
        storage
            .login_oauth(
                "openai-codex",
                "legacy-access",
                Some("legacy-refresh".into()),
                Some(u64::MAX),
            )
            .unwrap();
        let problem = stored_credential_problem("openai-codex", &storage).unwrap();
        assert!(problem.contains("legacy Codex login"), "{problem}");
        assert!(problem.contains("sign in again with ChatGPT"), "{problem}");
        // Other providers keep their own guidance.
        storage
            .login_oauth("github-copilot", "token", None, Some(u64::MAX))
            .unwrap();
        assert_eq!(stored_credential_problem("github-copilot", &storage), None);
    }

    #[test]
    fn resolve_provider_auth_rejects_unusable_openai_codex_oauth() {
        let mut storage = AuthStorage::in_memory();
        storage
            .login_oauth(
                "openai-codex",
                "pi-fixture-access",
                Some("pi-fixture-refresh".into()),
                Some(u64::MAX),
            )
            .unwrap();

        assert!(
            resolve_provider_auth("openai-codex", &storage, &Default::default(), true,).is_none()
        );
    }

    #[test]
    fn anthropic_oauth_environment_token_is_not_an_api_key() {
        let storage = AuthStorage::in_memory();
        let mut env = HashMap::new();
        env.insert(
            "ANTHROPIC_OAUTH_TOKEN".to_string(),
            "sk-ant-oat01-x".to_string(),
        );
        assert!(resolve_provider_auth("anthropic", &storage, &env, true).is_none());
        env.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-api-y".to_string());
        let resolved = resolve_provider_auth("anthropic", &storage, &env, true).unwrap();
        assert_eq!(resolved.api_key.as_deref(), Some("sk-ant-api-y"));
    }

    #[test]
    fn cloudflare_urls_are_materialized_and_ids_never_sent_as_headers() {
        let mut env = HashMap::new();
        env.insert("CLOUDFLARE_API_KEY".to_string(), "cf-key".to_string());
        env.insert("CLOUDFLARE_ACCOUNT_ID".to_string(), "acct1".to_string());
        env.insert("CLOUDFLARE_GATEWAY_ID".to_string(), "gw1".to_string());
        let workers = cloudflare_auth("cloudflare-workers-ai", None, &env).unwrap();
        let gateway = cloudflare_auth("cloudflare-ai-gateway", None, &env).unwrap();
        let mut model = crate::load_builtin_models()
            .into_iter()
            .next()
            .expect("a model");
        model.api = "openai-completions".into();
        model.base_url = Some(
            "https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1".into(),
        );
        assert_eq!(
            crate::stream::request_url(&model, &workers),
            "https://api.cloudflare.com/client/v4/accounts/acct1/ai/v1/chat/completions"
        );
        model.api = "anthropic-messages".into();
        model.base_url = Some(
            "https://gateway.ai.cloudflare.com/v1/{CLOUDFLARE_ACCOUNT_ID}/{CLOUDFLARE_GATEWAY_ID}/anthropic"
                .into(),
        );
        assert_eq!(
            crate::stream::request_url(&model, &gateway),
            "https://gateway.ai.cloudflare.com/v1/acct1/gw1/anthropic/v1/messages"
        );
        // Only the gateway token travels as a real header; IDs stay URL-only.
        assert!(gateway
            .headers
            .keys()
            .all(|name| is_url_var_header(name) || name == "cf-aig-authorization"));
        assert!(workers.headers.keys().all(|name| is_url_var_header(name)));
    }

    #[test]
    fn stored_anthropic_oauth_credential_is_refused_with_clear_guidance() {
        let mut storage = AuthStorage::in_memory();
        storage
            .login_oauth("anthropic", "sk-ant-oat01-x", None, Some(u64::MAX))
            .unwrap();
        assert!(resolve_provider_auth("anthropic", &storage, &Default::default(), true,).is_none());
        let error = storage
            .maybe_refresh("anthropic", 10_000, u64::MAX, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("/login anthropic <api-key>"), "{error}");
    }

    #[test]
    fn fixture_oauth_refresh_extends_expiry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&path).unwrap();
        storage
            .login_oauth("xai", "expired", Some("pi-fixture-refresh".into()), Some(1))
            .unwrap();
        assert!(storage.maybe_refresh("xai", 10_000, 0, false).unwrap());
        let cred = storage.get("xai").unwrap();
        assert_eq!(cred.access.as_deref(), Some("pi-fixture-refresh-access"));
        assert!(cred.expires.unwrap() > 10_000);
        assert!(!storage.maybe_refresh("xai", 10_000, 0, true).unwrap());
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
                "xai",
                jwt_expiring_at(1_000),
                Some("pi-fixture-refresh".into()),
                None,
            )
            .unwrap();
        assert!(storage.maybe_refresh("xai", 2_000_000, 0, false).unwrap());
        let cred = storage.get("xai").unwrap();
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
                "xai",
                "access",
                Some("pi-fixture-refresh".into()),
                Some(1_000_000),
            )
            .unwrap();
        assert!(!storage
            .maybe_refresh("xai", 500_000, 60_000, false)
            .unwrap());
        // Within the five-minute window the resolver uses, it renews early.
        assert!(storage
            .maybe_refresh("xai", 700_000, 300_000, false)
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

    #[test]
    fn bedrock_bearer_token_reaches_request_headers_and_role_sources_do_not_count() {
        let storage = AuthStorage::in_memory();
        let mut env = HashMap::new();
        env.insert("AWS_BEARER_TOKEN_BEDROCK".into(), "bedrock-token".into());
        let resolved = resolve_provider_auth("amazon-bedrock", &storage, &env, true).unwrap();
        assert_eq!(
            resolved.headers.get("Authorization").map(String::as_str),
            Some("Bearer bedrock-token")
        );

        // No signing material: an ECS role or web identity is not usable.
        for (name, value) in [
            ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/creds"),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "/tmp/token"),
        ] {
            let env = HashMap::from([(name.to_string(), value.to_string())]);
            assert!(resolve_provider_auth("amazon-bedrock", &storage, &env, true).is_none());
        }
        let keys = HashMap::from([
            ("AWS_ACCESS_KEY_ID".to_string(), "AKID".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "secret".to_string()),
        ]);
        assert!(resolve_provider_auth("amazon-bedrock", &storage, &keys, true).is_some());
    }

    #[test]
    fn cloudflare_gateway_token_uses_the_gateway_header() {
        let env = HashMap::from([
            ("CLOUDFLARE_API_KEY".to_string(), "audit-token".to_string()),
            ("CLOUDFLARE_ACCOUNT_ID".to_string(), "acct".to_string()),
            ("CLOUDFLARE_GATEWAY_ID".to_string(), "gw".to_string()),
        ]);
        let gateway = cloudflare_auth("cloudflare-ai-gateway", None, &env).unwrap();
        assert_eq!(gateway.api_key, None);
        assert_eq!(
            gateway
                .headers
                .get("cf-aig-authorization")
                .map(String::as_str),
            Some("Bearer audit-token")
        );
        assert!(!gateway
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization")));
        // Workers AI itself takes the token as the ordinary bearer key.
        let workers = cloudflare_auth("cloudflare-workers-ai", None, &env).unwrap();
        assert_eq!(workers.api_key.as_deref(), Some("audit-token"));
    }

    #[test]
    fn vertex_adc_counts_only_when_it_can_become_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let adc = dir.path().join("adc.json");
        let env = |path: &Path| {
            HashMap::from([
                (
                    "GOOGLE_APPLICATION_CREDENTIALS".to_string(),
                    path.display().to_string(),
                ),
                ("GOOGLE_CLOUD_PROJECT".to_string(), "p".to_string()),
                (
                    "GOOGLE_CLOUD_LOCATION".to_string(),
                    "us-central1".to_string(),
                ),
            ])
        };
        std::fs::write(&adc, r#"{"type":"external_account"}"#).unwrap();
        assert!(vertex_ambient_auth(None, &env(&adc)).is_none());
        std::fs::write(
            &adc,
            r#"{"type":"authorized_user","client_id":"c","client_secret":"s","refresh_token":"r"}"#,
        )
        .unwrap();
        assert!(vertex_ambient_auth(None, &env(&adc)).is_some());
    }
}
