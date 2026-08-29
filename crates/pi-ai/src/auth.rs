use crate::error::Result;
use crate::types::ProviderEnv;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Credential {
    #[serde(rename = "api_key")]
    ApiKey {
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<ProviderEnv>,
    },
    #[serde(rename = "oauth")]
    OAuth {
        refresh: String,
        access: String,
        expires: i64,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialInfo {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "type")]
    pub credential_type: String,
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn read(&self, provider_id: &str) -> Result<Option<Credential>>;
    async fn list(&self) -> Result<Vec<CredentialInfo>>;
    async fn set(&self, provider_id: &str, cred: Credential) -> Result<()>;
    async fn delete(&self, provider_id: &str) -> Result<()>;
}

#[derive(Default, Clone)]
pub struct InMemoryCredentialStore {
    entries: Arc<RwLock<HashMap<String, Credential>>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl CredentialStore for InMemoryCredentialStore {
    async fn read(&self, provider_id: &str) -> Result<Option<Credential>> {
        let lock = self.entries.read().await;
        Ok(lock.get(provider_id).cloned())
    }

    async fn list(&self) -> Result<Vec<CredentialInfo>> {
        let lock = self.entries.read().await;
        let mut list = Vec::new();
        for (id, cred) in lock.iter() {
            let credential_type = match cred {
                Credential::ApiKey { .. } => "api_key".to_string(),
                Credential::OAuth { .. } => "oauth".to_string(),
            };
            list.push(CredentialInfo {
                provider_id: id.clone(),
                credential_type,
            });
        }
        Ok(list)
    }

    async fn set(&self, provider_id: &str, cred: Credential) -> Result<()> {
        let mut lock = self.entries.write().await;
        lock.insert(provider_id.to_string(), cred);
        Ok(())
    }

    async fn delete(&self, provider_id: &str) -> Result<()> {
        let mut lock = self.entries.write().await;
        lock.remove(provider_id);
        Ok(())
    }
}

pub fn get_env_api_key_for_provider(provider: &str) -> Option<String> {
    let env_vars: &[&str] = match provider {
        "anthropic" => &[
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
        ],
        "openai" => &["OPENAI_API_KEY"],
        "google" => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        "groq" => &["GROQ_API_KEY"],
        "cerebras" => &["CEREBRAS_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "mistral" => &["MISTRAL_API_KEY"],
        "xai" => &["XAI_API_KEY"],
        "together" => &["TOGETHER_API_KEY"],
        "fireworks" => &["FIREWORKS_API_KEY"],
        "github-copilot" => &["COPILOT_GITHUB_TOKEN"],
        _ => return None,
    };

    for &var in env_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.trim().is_empty() {
                return Some(val);
            }
        }
    }
    None
}
