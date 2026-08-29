use crate::error::Result;
use crate::types::Model;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsStoreEntry {
    pub models: Vec<Model>,
    #[serde(rename = "lastModified", skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<i64>,
    #[serde(rename = "checkedAt", skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

#[async_trait]
pub trait ModelsStore: Send + Sync {
    async fn read(&self, provider_id: &str) -> Result<Option<ModelsStoreEntry>>;
    async fn write(&self, provider_id: &str, entry: ModelsStoreEntry) -> Result<()>;
    async fn delete(&self, provider_id: &str) -> Result<()>;
}

#[derive(Default, Clone)]
pub struct InMemoryModelsStore {
    entries: Arc<RwLock<HashMap<String, ModelsStoreEntry>>>,
}

impl InMemoryModelsStore {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl ModelsStore for InMemoryModelsStore {
    async fn read(&self, provider_id: &str) -> Result<Option<ModelsStoreEntry>> {
        let lock = self.entries.read().await;
        Ok(lock.get(provider_id).cloned())
    }

    async fn write(&self, provider_id: &str, entry: ModelsStoreEntry) -> Result<()> {
        let mut lock = self.entries.write().await;
        lock.insert(provider_id.to_string(), entry);
        Ok(())
    }

    async fn delete(&self, provider_id: &str) -> Result<()> {
        let mut lock = self.entries.write().await;
        lock.remove(provider_id);
        Ok(())
    }
}
