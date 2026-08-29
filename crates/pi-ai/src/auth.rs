use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    ApiKey,
    Oauth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Credential {
    #[serde(rename = "api_key")]
    ApiKey {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    #[serde(rename = "oauth")]
    Oauth {
        access: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires: Option<i64>,
        #[serde(flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

impl Drop for Credential {
    fn drop(&mut self) {
        match self {
            Self::ApiKey { key, .. } => key.zeroize(),
            Self::Oauth {
                access, refresh, ..
            } => {
                access.zeroize();
                if let Some(refresh) = refresh {
                    refresh.zeroize();
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthResult {
    pub provider: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub trait AuthStorage: Send + Sync {
    fn read(&self, provider_id: &str) -> Option<Credential>;
    fn write(&mut self, provider_id: &str, credential: Credential) -> Result<(), AuthError>;
    fn delete(&mut self, provider_id: &str) -> Result<(), AuthError>;
    fn list(&self) -> Vec<String>;
}

#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    credentials: HashMap<String, Credential>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AuthStorage for InMemoryCredentialStore {
    fn read(&self, provider_id: &str) -> Option<Credential> {
        self.credentials.get(provider_id).cloned()
    }

    fn write(&mut self, provider_id: &str, credential: Credential) -> Result<(), AuthError> {
        self.credentials.insert(provider_id.to_string(), credential);
        Ok(())
    }

    fn delete(&mut self, provider_id: &str) -> Result<(), AuthError> {
        self.credentials.remove(provider_id);
        Ok(())
    }

    fn list(&self) -> Vec<String> {
        self.credentials.keys().cloned().collect()
    }
}

/// Persistent auth.json compatible with TypeScript `AuthStorage`.
#[derive(Debug, Clone)]
pub struct FileAuthStorage {
    path: PathBuf,
    credentials: HashMap<String, Credential>,
}

impl FileAuthStorage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AuthError> {
        let path = path.as_ref().to_path_buf();
        let credentials = if path.exists() {
            let raw = fs::read_to_string(&path)
                .map_err(|e| AuthError::Message(format!("Failed to read auth.json: {e}")))?;
            parse_auth_json(&raw)?
        } else {
            HashMap::new()
        };
        Ok(Self { path, credentials })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".pi")
            .join("agent")
            .join("auth.json")
    }

    fn persist(&self) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AuthError::Message(format!("Failed to create auth directory: {e}")))?;
        }
        let json = serde_json::to_string_pretty(&self.credentials)
            .map_err(|e| AuthError::Message(e.to_string()))?;
        fs::write(&self.path, json)
            .map_err(|e| AuthError::Message(format!("Failed to write auth.json: {e}")))
    }
}

impl AuthStorage for FileAuthStorage {
    fn read(&self, provider_id: &str) -> Option<Credential> {
        self.credentials.get(provider_id).cloned()
    }

    fn write(&mut self, provider_id: &str, credential: Credential) -> Result<(), AuthError> {
        self.credentials.insert(provider_id.to_string(), credential);
        self.persist()
    }

    fn delete(&mut self, provider_id: &str) -> Result<(), AuthError> {
        self.credentials.remove(provider_id);
        self.persist()
    }

    fn list(&self) -> Vec<String> {
        self.credentials.keys().cloned().collect()
    }
}

pub fn parse_auth_json(raw: &str) -> Result<HashMap<String, Credential>, AuthError> {
    let value: ValueOrMap = serde_json::from_str(raw)
        .map_err(|e| AuthError::Message(format!("Invalid auth.json: {e}")))?;
    match value {
        ValueOrMap::Map(map) => {
            if let Some(versioned) = map.get("version") {
                if versioned.is_number() {
                    if let Some(creds) = map.get("credentials").and_then(|v| v.as_object()) {
                        return decode_map(creds);
                    }
                }
            }
            decode_map(&map)
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ValueOrMap {
    Map(serde_json::Map<String, serde_json::Value>),
}

fn decode_map(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<HashMap<String, Credential>, AuthError> {
    let mut out = HashMap::new();
    for (key, value) in map {
        if key == "version" || key == "credentials" {
            continue;
        }
        out.insert(
            key.clone(),
            serde_json::from_value(value.clone())
                .map_err(|e| AuthError::Message(format!("Invalid credential for {key}: {e}")))?,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_key_auth_json() {
        let raw = r#"{"anthropic":{"type":"api_key","key":"sk-test"}}"#;
        let map = parse_auth_json(raw).unwrap();
        match map.get("anthropic") {
            Some(Credential::ApiKey { key, .. }) => assert_eq!(key, "sk-test"),
            other => panic!("{other:?}"),
        }
    }
}
