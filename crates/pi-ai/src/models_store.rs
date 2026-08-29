//! Persisted remote catalog overlay matching TS `models-store.ts`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::catalog::Model;

pub const REMOTE_CATALOG_REFRESH_INTERVAL_MS: u64 = 4 * 60 * 60 * 1000;
pub const DEFAULT_CATALOG_BASE_URL: &str = "https://pi.dev";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelsStoreEntry {
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default, rename = "checkedAt")]
    pub checked_at: Option<u64>,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<u64>,
    #[serde(default)]
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelsStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ModelsStoreEntry>,
}

pub fn models_store_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("models-store.json")
}

pub fn load_models_store(agent_dir: &Path) -> ModelsStore {
    fs::read_to_string(models_store_path(agent_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_models_store(agent_dir: &Path, store: &ModelsStore) -> Result<(), String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    fs::write(
        models_store_path(agent_dir),
        serde_json::to_string_pretty(store).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn merge_models(baseline: &[Model], dynamic: &[Model]) -> Vec<Model> {
    let mut merged = baseline.to_vec();
    for model in dynamic {
        if let Some(index) = merged.iter().position(|entry| entry.id == model.id) {
            merged[index] = model.clone();
        } else {
            merged.push(model.clone());
        }
    }
    merged
}

pub fn parse_remote_catalog(
    provider_id: &str,
    value: &serde_json::Value,
) -> Result<Vec<Model>, String> {
    let entries = if let Some(array) = value.as_array() {
        array.clone()
    } else if let Some(array) = value.get("models").and_then(serde_json::Value::as_array) {
        array.clone()
    } else if let Some(object) = value.as_object() {
        object.values().cloned().collect()
    } else {
        return Err(format!(
            "Invalid model catalog for provider \"{provider_id}\""
        ));
    };
    let mut models = Vec::new();
    for entry in entries {
        if let Ok(mut model) = serde_json::from_value::<Model>(entry) {
            if model.provider.is_empty() {
                model.provider = provider_id.to_string();
            }
            models.push(model);
        }
    }
    Ok(models)
}

pub fn catalog_url(base: &str, provider_id: &str) -> String {
    format!(
        "{}/api/models/providers/{}",
        base.trim_end_matches('/'),
        provider_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_merge_remote_catalog() {
        let parsed = parse_remote_catalog(
            "openai",
            &serde_json::json!([{
                "id": "gpt-test",
                "name": "GPT Test",
                "api": "openai-responses",
                "provider": "",
                "reasoning": false,
                "input": ["text"],
                "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0 },
                "contextWindow": 1000,
                "maxTokens": 16
            }]),
        )
        .unwrap();
        assert_eq!(parsed[0].provider, "openai");
        let merged = merge_models(
            &[Model {
                id: "keep".into(),
                name: "Keep".into(),
                api: "openai-responses".into(),
                provider: "openai".into(),
                base_url: None,
                reasoning: false,
                input: vec!["text".into()],
                cost: crate::catalog::ModelCost {
                    input: 0.0,
                    output: 0.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                },
                context_window: 1,
                max_tokens: 1,
                compat: serde_json::Value::Null,
            }],
            &parsed,
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(
            catalog_url(DEFAULT_CATALOG_BASE_URL, "openai"),
            "https://pi.dev/api/models/providers/openai"
        );
    }
}
