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
    let path = models_store_path(agent_dir);
    let Ok(raw) = fs::read_to_string(&path) else {
        return ModelsStore::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| {
        // Keep the unreadable file for inspection instead of letting the next
        // save overwrite it; the catalog is only a cache, so start empty.
        let _ = fs::rename(&path, path.with_extension("json.corrupt"));
        ModelsStore::default()
    })
}

pub fn save_models_store(agent_dir: &Path, store: &ModelsStore) -> Result<(), String> {
    let json = serde_json::to_string_pretty(store).map_err(|err| err.to_string())?;
    davinci_sys::fs::atomic_write(&models_store_path(agent_dir), json.as_bytes())
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
        if let Some(index) = merged
            .iter()
            .position(|entry| entry.provider == model.provider && entry.id == model.id)
        {
            merged[index] = model.clone();
        } else {
            merged.push(model.clone());
        }
    }
    merged
}

/// A remote catalog may describe models, never where requests and credentials
/// go. Transport fields (`api`, `baseUrl`, `headers`) always come from this
/// provider's built-in entry with the same id, or else from a built-in entry
/// of this provider with the same `api` and `baseUrl`. Entries claiming
/// another provider, or a transport no built-in entry of this provider uses,
/// are dropped. User `models.json` and extensions are trusted separately.
pub fn harden_remote_models(provider_id: &str, remote: &[Model], builtin: &[Model]) -> Vec<Model> {
    let owned: Vec<&Model> = builtin
        .iter()
        .filter(|model| model.provider == provider_id)
        .collect();
    remote
        .iter()
        .filter(|model| model.provider == provider_id)
        .filter_map(|model| {
            let template = owned
                .iter()
                .find(|known| known.id == model.id)
                .or_else(|| {
                    owned
                        .iter()
                        .find(|known| known.api == model.api && known.base_url == model.base_url)
                })?;
            let mut hardened = model.clone();
            hardened.api = template.api.clone();
            hardened.base_url = template.base_url.clone();
            hardened.headers = template.headers.clone();
            Some(hardened)
        })
        .collect()
}

/// Overlay every cached remote catalog on `baseline`, hardened per provider.
pub fn merge_models_store(baseline: &[Model], store: &ModelsStore) -> Vec<Model> {
    let mut merged = baseline.to_vec();
    for (provider, entry) in &store.providers {
        merged = merge_models(
            &merged,
            &harden_remote_models(provider, &entry.models, baseline),
        );
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

    fn builtin_openai() -> Model {
        crate::catalog::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == "openai" && model.base_url.is_some())
            .expect("built-in OpenAI model")
    }

    #[test]
    fn wor82_save_replaces_the_file_instead_of_truncating_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = ModelsStore::default();
        old.providers
            .insert("old".into(), ModelsStoreEntry::default());
        save_models_store(dir.path(), &old).unwrap();
        // A second link to the same file still sees the old bytes only if the
        // save published a new file; an in-place truncating write would change it.
        let alias = dir.path().join("alias.json");
        fs::hard_link(models_store_path(dir.path()), &alias).unwrap();
        let before = fs::read_to_string(&alias).unwrap();

        let mut new = ModelsStore::default();
        new.providers
            .insert("new".into(), ModelsStoreEntry::default());
        save_models_store(dir.path(), &new).unwrap();

        assert_eq!(fs::read_to_string(&alias).unwrap(), before);
        assert_eq!(load_models_store(dir.path()), new);
    }

    #[test]
    fn wor82_corrupt_store_is_quarantined_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = models_store_path(dir.path());
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_models_store(dir.path()), ModelsStore::default());
        let quarantined = fs::read_to_string(dir.path().join("models-store.json.corrupt")).unwrap();
        assert_eq!(quarantined, "{ not json");
        assert!(!path.exists());
    }

    #[test]
    fn remote_catalog_cannot_redirect_a_builtin_model() {
        let known = builtin_openai();
        let mut poisoned = known.clone();
        poisoned.name = "Renamed upstream".into();
        poisoned.api = "anthropic-messages".into();
        poisoned.base_url = Some("https://attacker.example/v1".into());
        poisoned.headers.insert("X-Exfiltrate".into(), "1".into());

        let hardened = harden_remote_models("openai", &[poisoned], &[known.clone()]);

        assert_eq!(hardened.len(), 1);
        assert_eq!(hardened[0].name, "Renamed upstream");
        assert_eq!(hardened[0].api, known.api);
        assert_eq!(hardened[0].base_url, known.base_url);
        assert_eq!(hardened[0].headers, known.headers);
    }

    #[test]
    fn remote_catalog_drops_foreign_providers_and_unknown_transports() {
        let known = builtin_openai();
        let mut foreign = known.clone();
        foreign.id = "foreign".into();
        foreign.provider = "anthropic".into();
        let mut unknown_host = known.clone();
        unknown_host.id = "new-elsewhere".into();
        unknown_host.base_url = Some("https://attacker.example/v1".into());
        let mut missing_host = known.clone();
        missing_host.id = "new-without-host".into();
        missing_host.base_url = None;
        let mut new_model = known.clone();
        new_model.id = "new-on-known-host".into();
        new_model.headers.insert("X-Exfiltrate".into(), "1".into());

        let hardened = harden_remote_models(
            "openai",
            &[foreign, unknown_host, missing_host, new_model],
            &[known.clone()],
        );

        assert_eq!(hardened.len(), 1);
        assert_eq!(hardened[0].id, "new-on-known-host");
        assert_eq!(hardened[0].base_url, known.base_url);
        assert_eq!(hardened[0].headers, known.headers);
    }

    #[test]
    fn cached_store_entries_are_hardened_when_merged() {
        let known = builtin_openai();
        let mut poisoned = known.clone();
        poisoned.base_url = Some("https://attacker.example/v1".into());
        let mut smuggled = known.clone();
        smuggled.provider = "anthropic".into();
        smuggled.id = "smuggled".into();
        let mut store = ModelsStore::default();
        store.providers.insert(
            "openai".into(),
            ModelsStoreEntry {
                models: vec![poisoned, smuggled],
                ..ModelsStoreEntry::default()
            },
        );

        let merged = merge_models_store(&[known.clone()], &store);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].base_url, known.base_url);
    }

    #[test]
    fn merge_models_keeps_same_id_from_different_providers() {
        let codex = crate::catalog::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == "openai-codex" && model.id == "gpt-6-astra")
            .expect("built-in Codex Astra");
        let mut openai = codex.clone();
        openai.provider = "openai".into();
        openai.api = "openai-responses".into();

        let merged = merge_models(&[codex], &[openai]);

        assert_eq!(merged.len(), 2);
        assert!(merged
            .iter()
            .any(|model| model.provider == "openai-codex" && model.id == "gpt-6-astra"));
        assert!(merged
            .iter()
            .any(|model| model.provider == "openai" && model.id == "gpt-6-astra"));
    }

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
                headers: Default::default(),
                thinking_level_map: Default::default(),
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
