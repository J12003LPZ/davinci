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
}
