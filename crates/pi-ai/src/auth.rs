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
            Credential {
                kind: CredentialKind::Oauth,
                key: None,
                access: Some(access.into()),
                refresh,
                expires,
                env: HashMap::new(),
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
        let expired = cred
            .expires
            .map(|exp| exp <= now_ms.saturating_add(min_expiry_ms))
            .unwrap_or(false);
        if !expired {
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
        Ok(false)
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

pub fn default_auth_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir).join("auth.json");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
        .join("auth.json")
}

pub fn env_api_key(spec: &ProviderSpec, env: &HashMap<String, String>) -> Option<(String, String)> {
    for var in spec.env_vars {
        if let Some(value) = env.get(*var).cloned().or_else(|| std::env::var(var).ok()) {
            if !value.is_empty() {
                return Some((value, (*var).to_string()));
            }
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
                    return Some(ResolvedAuth {
                        api_key: Some(key.clone()),
                        headers: HashMap::new(),
                        source: "stored credential".into(),
                    });
                }
            }
            CredentialKind::Oauth => {
                if let Some(access) = cred.access.clone().or_else(|| cred.key.clone()) {
                    let mut headers = HashMap::new();
                    headers.insert("Authorization".into(), format!("Bearer {access}"));
                    return Some(ResolvedAuth {
                        api_key: Some(access),
                        headers,
                        source: "oauth".into(),
                    });
                }
            }
        }
    }
    if include_env {
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
    None
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
}
