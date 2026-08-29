//! ModelRuntime availability snapshot matching TS `model-runtime.ts`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::auth::{resolve_provider_auth, AuthStorage, ResolvedAuth};
use crate::catalog::Model;
use crate::model_config::{ModelConfig, NO_MODELS_AVAILABLE};

/// Auth check recorded for a provider, matching TS `AuthCheck`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCheck {
    pub kind: String,
    pub source: String,
}

/// Configured / stored / available model snapshot used by `/model`, `--list-models`, and RPC.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelRuntimeSnapshot {
    pub all: Vec<Model>,
    pub available: Vec<Model>,
    pub configured_providers: Vec<String>,
    pub stored_providers: Vec<String>,
    pub auth: BTreeMap<String, AuthCheck>,
    pub composition_errors: BTreeMap<String, String>,
    pub availability_error: Option<String>,
    pub config_error: Option<String>,
}

impl ModelRuntimeSnapshot {
    pub fn get_error(&self) -> Option<String> {
        let mut errors = Vec::new();
        if let Some(error) = &self.config_error {
            errors.push(error.clone());
        }
        for (provider_id, error) in &self.composition_errors {
            errors.push(format!("Provider \"{provider_id}\": {error}"));
        }
        if let Some(error) = &self.availability_error {
            errors.push(format!("Availability refresh: {error}"));
        }
        if errors.is_empty() {
            None
        } else {
            Some(errors.join("\n\n"))
        }
    }

    pub fn get_available_snapshot(&self) -> &[Model] {
        &self.available
    }
}

pub fn format_no_models_available_message(docs_dir: &Path) -> String {
    format!(
        "No models available. Use /login to log into a provider via OAuth or API key. See:\n  {}\n  {}",
        docs_dir.join("providers.md").display(),
        docs_dir.join("models.md").display()
    )
}

pub fn snapshot_availability(
    all: Vec<Model>,
    config: &ModelConfig,
    storage: &AuthStorage,
    env: &std::collections::HashMap<String, String>,
    composition_errors: BTreeMap<String, String>,
    availability_error: Option<String>,
) -> ModelRuntimeSnapshot {
    let stored_providers = {
        let mut keys = storage.providers();
        keys.sort();
        keys
    };
    let mut provider_ids: BTreeSet<String> =
        all.iter().map(|model| model.provider.clone()).collect();
    for id in config.provider_ids() {
        provider_ids.insert(id.to_string());
    }
    for id in &stored_providers {
        provider_ids.insert(id.clone());
    }
    let mut auth = BTreeMap::new();
    let mut configured = BTreeSet::new();
    for provider in &provider_ids {
        if let Some(check) = provider_auth_check(provider, config, storage, env) {
            configured.insert(provider.clone());
            auth.insert(provider.clone(), check);
        }
    }
    let available: Vec<Model> = all
        .iter()
        .filter(|model| configured.contains(&model.provider))
        .cloned()
        .collect();
    let config_error = config.error().map(str::to_string);
    ModelRuntimeSnapshot {
        all,
        available,
        configured_providers: configured.into_iter().collect(),
        stored_providers,
        auth,
        composition_errors,
        availability_error,
        config_error,
    }
}

fn provider_auth_check(
    provider: &str,
    config: &ModelConfig,
    storage: &AuthStorage,
    env: &std::collections::HashMap<String, String>,
) -> Option<AuthCheck> {
    if let Some(resolved) = resolve_provider_auth(provider, storage, env, true) {
        return Some(auth_check_from_resolved(resolved));
    }
    if let Some(provider_config) = config.get_provider(provider) {
        if let Some(key) = &provider_config.api_key {
            if !key.is_empty() {
                return Some(AuthCheck {
                    kind: "api-key".into(),
                    source: "models.json".into(),
                });
            }
        }
    }
    None
}

fn auth_check_from_resolved(resolved: ResolvedAuth) -> AuthCheck {
    let kind = if resolved.source.contains("oauth") {
        "oauth"
    } else {
        "api-key"
    };
    AuthCheck {
        kind: kind.into(),
        source: resolved.source,
    }
}

pub fn empty_catalog_error(snapshot: &ModelRuntimeSnapshot) -> Option<String> {
    if snapshot.all.is_empty() {
        Some(
            snapshot
                .get_error()
                .unwrap_or_else(|| NO_MODELS_AVAILABLE.into()),
        )
    } else {
        snapshot.get_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;
    use serde_json::json;
    use tempfile::tempdir;

    fn demo_model(provider: &str, id: &str) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            api: "openai-completions".into(),
            provider: provider.into(),
            base_url: Some("http://127.0.0.1:9".into()),
            reasoning: false,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 128_000,
            max_tokens: 16_384,
            compat: json!(null),
            headers: Default::default(),
        }
    }

    #[test]
    fn filters_available_to_configured_and_stored_providers() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let mut storage = AuthStorage::open(&auth_path).unwrap();
        storage.login_api_key("anthropic", "sk-test").unwrap();
        let config = ModelConfig::empty();
        let all = vec![
            demo_model("anthropic", "sonnet"),
            demo_model("openai", "gpt"),
        ];
        let snap = snapshot_availability(
            all,
            &config,
            &storage,
            &Default::default(),
            BTreeMap::new(),
            None,
        );
        assert_eq!(snap.stored_providers, vec!["anthropic".to_string()]);
        assert_eq!(snap.configured_providers, vec!["anthropic".to_string()]);
        assert_eq!(snap.available.len(), 1);
        assert_eq!(snap.available[0].id, "sonnet");
        assert_eq!(snap.all.len(), 2);
        assert!(snap.auth.contains_key("anthropic"));
        assert!(snap
            .get_available_snapshot()
            .iter()
            .all(|m| m.provider == "anthropic"));
    }

    #[test]
    fn models_json_api_key_configures_custom_provider() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(
            &path,
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","apiKey":"sk-local","models":[{"id":"demo"}]}}}"#,
        )
        .unwrap();
        let config = ModelConfig::load(&path);
        let models = config.apply(&[]).unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let snap = snapshot_availability(
            models,
            &config,
            &storage,
            &Default::default(),
            BTreeMap::new(),
            None,
        );
        assert!(snap.configured_providers.contains(&"local".to_string()));
        assert_eq!(snap.available.len(), 1);
        assert_eq!(
            snap.auth.get("local").map(|c| c.source.as_str()),
            Some("models.json")
        );
    }

    #[test]
    fn get_error_joins_config_compose_and_availability() {
        let mut composition = BTreeMap::new();
        composition.insert("acme".into(), "no api".into());
        let snap = ModelRuntimeSnapshot {
            config_error: Some(
                "Invalid models.json schema:\n  - providers: Expected object".into(),
            ),
            composition_errors: composition,
            availability_error: Some("timeout".into()),
            ..ModelRuntimeSnapshot::default()
        };
        let error = snap.get_error().unwrap();
        assert!(error.contains("Invalid models.json schema:"));
        assert!(error.contains("Provider \"acme\": no api"));
        assert!(error.contains("Availability refresh: timeout"));
    }

    #[test]
    fn login_help_copy_is_locked() {
        let message = format_no_models_available_message(Path::new("/docs"));
        assert!(message.starts_with("No models available. Use /login to log into a provider"));
        assert!(message.contains("/docs/providers.md"));
        assert!(message.contains("/docs/models.md"));
    }
}
