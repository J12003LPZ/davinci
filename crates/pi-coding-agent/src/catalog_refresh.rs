//! Interactive / CLI model-catalog refresh matching TS `model-catalog-refresh.ts`.

use std::path::Path;
use std::time::Duration;

use pi_ai::{
    builtin_provider_ids, catalog_url, load_builtin_models, load_models_store, merge_models,
    parse_remote_catalog, save_models_store, Model, ModelsStore, ModelsStoreEntry,
    DEFAULT_CATALOG_BASE_URL, REMOTE_CATALOG_REFRESH_INTERVAL_MS,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogRefreshResult {
    pub models: Vec<Model>,
    pub status: String,
    pub timed_out: bool,
    pub errors: Vec<(String, String)>,
}

pub fn refresh_status_refreshing() -> &'static str {
    "Refreshing model catalogs…"
}

pub fn refresh_status_ok() -> &'static str {
    "Model catalogs refreshed"
}

pub fn refresh_status_timeout() -> &'static str {
    "Model refresh timed out; showing cached models."
}

pub fn refresh_error_message(errors: &[(String, String)]) -> String {
    if errors.is_empty() {
        return "Could not refresh model catalogs: unknown error".into();
    }
    if errors.len() == 1 {
        return format!("Could not refresh {}; showing cached models.", errors[0].0);
    }
    let providers = errors
        .iter()
        .map(|(provider, _)| provider.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("Could not refresh {providers}; showing cached models.")
}

pub fn cli_refresh_message(result: &CatalogRefreshResult) -> Result<String, String> {
    if result.timed_out {
        return Err("Model catalog refresh timed out.".into());
    }
    if !result.errors.is_empty() {
        let detail = result
            .errors
            .iter()
            .map(|(provider, error)| format!("{provider}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("Could not refresh model catalogs: {detail}"));
    }
    Ok(refresh_status_ok().into())
}

pub fn refresh_model_catalogs(
    agent_dir: &Path,
    allow_network: bool,
    force: bool,
) -> CatalogRefreshResult {
    let offline = std::env::var("PI_OFFLINE").is_ok();
    let allow_network = allow_network && !offline;
    if std::env::var("PI_CATALOG_REFRESH_TIMEOUT").is_ok() {
        return CatalogRefreshResult {
            models: load_cached_or_builtin(agent_dir),
            status: refresh_status_timeout().into(),
            timed_out: true,
            errors: Vec::new(),
        };
    }
    if let Ok(err) = std::env::var("PI_CATALOG_REFRESH_ERROR") {
        if !err.is_empty() {
            let providers: Vec<String> = err
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
            let errors = providers
                .into_iter()
                .map(|provider| (provider, "refresh failed".into()))
                .collect::<Vec<_>>();
            return CatalogRefreshResult {
                models: load_cached_or_builtin(agent_dir),
                status: refresh_error_message(&errors),
                timed_out: false,
                errors,
            };
        }
    }
    let mut store = load_models_store(agent_dir);
    let mut errors = Vec::new();
    let mut models = load_builtin_models();
    for provider in builtin_provider_ids() {
        match refresh_provider(provider, agent_dir, &mut store, allow_network, force) {
            Ok(extra) => models = merge_models(&models, &extra),
            Err(err) => errors.push((provider.to_string(), err)),
        }
    }
    let _ = save_models_store(agent_dir, &store);
    let catalogs = agent_dir.join("models");
    let _ = std::fs::create_dir_all(&catalogs);
    for provider in builtin_provider_ids() {
        if let Some(json) = pi_ai::builtin_catalog_json(provider) {
            let _ = std::fs::write(catalogs.join(format!("{provider}.json")), json);
        }
    }
    CatalogRefreshResult {
        models,
        status: if errors.is_empty() {
            format!("{}.", refresh_status_ok())
        } else {
            refresh_error_message(&errors)
        },
        timed_out: false,
        errors,
    }
}

fn load_cached_or_builtin(agent_dir: &Path) -> Vec<Model> {
    let store = load_models_store(agent_dir);
    let mut models = load_builtin_models();
    for entry in store.providers.values() {
        models = merge_models(&models, &entry.models);
    }
    models
}

fn refresh_provider(
    provider: &str,
    _agent_dir: &Path,
    store: &mut ModelsStore,
    allow_network: bool,
    force: bool,
) -> Result<Vec<Model>, String> {
    let stored = store.providers.get(provider).cloned();
    if let Ok(raw) = std::env::var("PI_CATALOG_REFRESH_REPLY") {
        let value: serde_json::Value = if Path::new(&raw).exists() {
            let body = std::fs::read_to_string(&raw).map_err(|err| err.to_string())?;
            serde_json::from_str(&body).map_err(|err| err.to_string())?
        } else {
            serde_json::from_str(&raw).map_err(|err| err.to_string())?
        };
        if let Some(payload) = value.get(provider).cloned().or_else(|| {
            value
                .as_object()
                .and_then(|object| object.values().next().cloned())
        }) {
            let models = parse_remote_catalog(provider, &payload)?;
            store.providers.insert(
                provider.to_string(),
                ModelsStoreEntry {
                    models: models.clone(),
                    checked_at: Some(pi_ai::now_ms()),
                    last_modified: Some(pi_ai::now_ms()),
                    etag: None,
                },
            );
            return Ok(models);
        }
        if value.get("id").is_some() {
            let models = parse_remote_catalog(provider, &value)?;
            store.providers.insert(
                provider.to_string(),
                ModelsStoreEntry {
                    models: models.clone(),
                    checked_at: Some(pi_ai::now_ms()),
                    last_modified: Some(pi_ai::now_ms()),
                    etag: None,
                },
            );
            return Ok(models);
        }
        return Ok(stored.map(|entry| entry.models).unwrap_or_default());
    }
    if !allow_network || std::env::var("PI_CATALOG_DRY_RUN").is_ok() || cfg!(test) {
        return Ok(stored.map(|entry| entry.models).unwrap_or_default());
    }
    if !force {
        if let Some(entry) = &stored {
            if let (Some(checked), Some(_)) = (entry.checked_at, entry.last_modified) {
                if pi_ai::now_ms().saturating_sub(checked) < REMOTE_CATALOG_REFRESH_INTERVAL_MS {
                    return Ok(entry.models.clone());
                }
            }
        }
    }
    let base = std::env::var("PI_CATALOG_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_CATALOG_BASE_URL.to_string());
    let url = catalog_url(&base, provider);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(4000))
        .build();
    let response = agent
        .get(&url)
        .set("accept", "application/json")
        .call()
        .map_err(|err| format!("Model catalog request failed for {provider}: {err}"))?;
    let status = response.status();
    let checked_at = pi_ai::now_ms();
    if status == 304 {
        if let Some(mut entry) = stored {
            entry.checked_at = Some(checked_at);
            let models = entry.models.clone();
            store.providers.insert(provider.to_string(), entry);
            return Ok(models);
        }
        return Ok(Vec::new());
    }
    if status == 404 || status == 501 {
        store.providers.insert(
            provider.to_string(),
            ModelsStoreEntry {
                models: stored.map(|entry| entry.models).unwrap_or_default(),
                checked_at: Some(checked_at),
                last_modified: Some(0),
                etag: None,
            },
        );
        return Ok(Vec::new());
    }
    if !(200..300).contains(&status) {
        if let Some(mut entry) = stored {
            entry.checked_at = Some(checked_at);
            store.providers.insert(provider.to_string(), entry);
        }
        return Err(format!(
            "Model catalog request failed for {provider}: {status}"
        ));
    }
    let payload: serde_json::Value = response
        .into_json()
        .map_err(|err| format!("Invalid model catalog for provider \"{provider}\": {err}"))?;
    let models = parse_remote_catalog(provider, &payload)?;
    store.providers.insert(
        provider.to_string(),
        ModelsStoreEntry {
            models: models.clone(),
            checked_at: Some(checked_at),
            last_modified: Some(checked_at),
            etag: None,
        },
    );
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn refresh_strings_and_fixtures_match_ts() {
        assert_eq!(refresh_status_refreshing(), "Refreshing model catalogs…");
        assert_eq!(refresh_status_ok(), "Model catalogs refreshed");
        assert_eq!(
            refresh_status_timeout(),
            "Model refresh timed out; showing cached models."
        );
        assert_eq!(
            refresh_error_message(&[
                ("openai".into(), "x".into()),
                ("anthropic".into(), "y".into())
            ]),
            "Could not refresh openai, anthropic; showing cached models."
        );
        let dir = tempdir().unwrap();
        std::env::set_var("PI_CATALOG_DRY_RUN", "1");
        let ok = refresh_model_catalogs(dir.path(), true, true);
        std::env::remove_var("PI_CATALOG_DRY_RUN");
        assert_eq!(
            cli_refresh_message(&ok).unwrap(),
            "Model catalogs refreshed"
        );
        std::env::set_var("PI_CATALOG_REFRESH_TIMEOUT", "1");
        let timeout = refresh_model_catalogs(dir.path(), true, true);
        std::env::remove_var("PI_CATALOG_REFRESH_TIMEOUT");
        assert_eq!(
            cli_refresh_message(&timeout).unwrap_err(),
            "Model catalog refresh timed out."
        );
        std::env::set_var("PI_CATALOG_REFRESH_ERROR", "openai,anthropic");
        let failed = refresh_model_catalogs(dir.path(), true, true);
        std::env::remove_var("PI_CATALOG_REFRESH_ERROR");
        assert!(failed.status.contains("openai, anthropic"));
    }
}
