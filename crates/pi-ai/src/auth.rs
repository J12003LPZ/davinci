use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    OAuth {
        token: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        expires: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        enterprise_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialInfo {
    pub provider_id: String,
    pub credential_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthResolved {
    pub api_key: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub env: Option<HashMap<String, String>>,
    pub source: String,
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn read(&self, provider_id: &str) -> Option<Credential>;
    async fn write(&self, provider_id: &str, credential: Credential);
    async fn delete(&self, provider_id: &str);
    async fn list(&self) -> Vec<CredentialInfo>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryCredentialStore {
    credentials: Arc<RwLock<HashMap<String, Credential>>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CredentialStore for InMemoryCredentialStore {
    async fn read(&self, provider_id: &str) -> Option<Credential> {
        let read_guard = self.credentials.read().await;
        read_guard.get(provider_id).cloned()
    }

    async fn write(&self, provider_id: &str, credential: Credential) {
        let mut write_guard = self.credentials.write().await;
        write_guard.insert(provider_id.to_string(), credential);
    }

    async fn delete(&self, provider_id: &str) {
        let mut write_guard = self.credentials.write().await;
        write_guard.remove(provider_id);
    }

    async fn list(&self) -> Vec<CredentialInfo> {
        let read_guard = self.credentials.read().await;
        read_guard
            .iter()
            .map(|(k, v)| CredentialInfo {
                provider_id: k.clone(),
                credential_type: match v {
                    Credential::ApiKey { .. } => "api_key".to_string(),
                    Credential::OAuth { .. } => "oauth".to_string(),
                },
            })
            .collect()
    }
}
