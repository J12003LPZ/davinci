//! ModelRuntime availability snapshot matching TS `model-runtime.ts`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::auth::{
    bedrock_ambient_source, cloudflare_auth, resolve_provider_auth, vertex_ambient_auth,
    AuthStorage, Credential, CredentialKind, ResolvedAuth,
};
use crate::catalog::Model;
use crate::model_config::{
    config_value_env_var_names, is_command_config_value, ModelConfig, NO_MODELS_AVAILABLE,
};
use crate::oauth_providers::oauth_providers;
use crate::providers::provider_spec;

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

fn provider_login_help(docs_dir: &Path) -> String {
    format!(
        "Use /login to log into a provider via OAuth or API key. See:\n  {}\n  {}",
        docs_dir
            .join("providers.md")
            .display()
            .to_string()
            .replace('\\', "/"),
        docs_dir
            .join("models.md")
            .display()
            .to_string()
            .replace('\\', "/")
    )
}

pub fn format_no_models_available_message(docs_dir: &Path) -> String {
    format!("No models available. {}", provider_login_help(docs_dir))
}

pub fn format_no_model_selected_message(docs_dir: &Path) -> String {
    format!(
        "No model selected.\n\n{}\n\nThen use /model to select a model.",
        provider_login_help(docs_dir)
    )
}

pub fn format_no_api_key_found_message(provider: &str, docs_dir: &Path) -> String {
    let provider_display = if provider == "unknown" {
        "the selected model"
    } else {
        provider
    };
    format!(
        "No API key found for {provider_display}.\n\n{}",
        provider_login_help(docs_dir)
    )
}

pub fn format_oauth_auth_failed_message(provider: &str) -> String {
    format!(
        "Authentication failed for \"{provider}\". Credentials may have expired or network is unavailable. Run '/login {provider}' to re-authenticate."
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
    provider_ids.insert("llama.cpp".into());
    let mut auth = BTreeMap::new();
    let mut configured = BTreeSet::new();
    for provider in &provider_ids {
        if let Some(check) = check_auth(provider, config, storage, env) {
            configured.insert(provider.clone());
            auth.insert(provider.clone(), check);
        }
    }
    let available = get_available(&all, None, config, storage, env);
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

/// TS `Models.checkAuth`: OAuth stored credentials are not refreshed.
pub fn check_auth(
    provider: &str,
    config: &ModelConfig,
    storage: &AuthStorage,
    env: &HashMap<String, String>,
) -> Option<AuthCheck> {
    if let Some(cred) = storage.get(provider) {
        if cred.kind == CredentialKind::Oauth {
            return if provider_supports_oauth(provider, config) {
                Some(AuthCheck {
                    kind: "oauth".into(),
                    source: "OAuth".into(),
                })
            } else {
                None
            };
        }
        if let Some(check) = provider_api_key_check(provider, Some(cred), config, env) {
            return Some(check);
        }
    }
    if let Some(check) = configured_api_key_check(config.get_provider(provider), env) {
        return Some(check);
    }
    if let Some(check) = provider_api_key_check(provider, None, config, env) {
        return Some(check);
    }
    resolve_provider_auth(provider, storage, env, true).map(auth_check_from_resolved)
}

/// TS `Models.getAvailable`: checkAuth then `filterModels` (GitHub Copilot).
pub fn get_available(
    all: &[Model],
    provider_id: Option<&str>,
    config: &ModelConfig,
    storage: &AuthStorage,
    env: &HashMap<String, String>,
) -> Vec<Model> {
    let mut providers: BTreeSet<String> = BTreeSet::new();
    if let Some(id) = provider_id {
        providers.insert(id.to_string());
    } else {
        for model in all {
            providers.insert(model.provider.clone());
        }
        for id in config.provider_ids() {
            providers.insert(id.to_string());
        }
        for id in storage.providers() {
            providers.insert(id);
        }
    }
    let authed: BTreeSet<String> = providers
        .into_iter()
        .filter(|provider| check_auth(provider, config, storage, env).is_some())
        .collect();
    all.iter()
        .filter(|model| {
            if let Some(id) = provider_id {
                if model.provider != id {
                    return false;
                }
            }
            authed.contains(&model.provider)
                && model_allowed_for_credential(model, storage.get(&model.provider))
        })
        .cloned()
        .collect()
}

fn provider_supports_oauth(provider: &str, config: &ModelConfig) -> bool {
    if oauth_providers().contains(&provider) {
        return true;
    }
    if provider_spec(provider).is_some_and(|spec| spec.oauth) {
        return true;
    }
    config
        .get_provider(provider)
        .and_then(|entry| entry.oauth.as_deref())
        == Some("radius")
}

fn configured_api_key_check(
    provider_config: Option<&crate::model_config::ModelsJsonProvider>,
    env: &HashMap<String, String>,
) -> Option<AuthCheck> {
    let key = provider_config?.api_key.as_deref()?;
    if key.is_empty() {
        return None;
    }
    if is_command_config_value(key) {
        return Some(configured_api_key_auth());
    }
    let names = config_value_env_var_names(key);
    if !names.is_empty() {
        if names.iter().all(|name| env_is_set(name, env)) {
            return Some(configured_api_key_auth());
        }
        return None;
    }
    Some(configured_api_key_auth())
}

fn configured_api_key_auth() -> AuthCheck {
    AuthCheck {
        kind: "api_key".into(),
        source: "configured API key".into(),
    }
}

fn provider_api_key_check(
    provider: &str,
    credential: Option<&Credential>,
    _config: &ModelConfig,
    env: &HashMap<String, String>,
) -> Option<AuthCheck> {
    if provider == "llama.cpp" {
        if let Some(cred) = credential {
            if env_nonempty(cred.env.get("LLAMA_BASE_URL"))
                || cred.key.as_ref().is_some_and(|k| !k.is_empty())
            {
                return Some(AuthCheck {
                    kind: "api_key".into(),
                    source: "stored credential".into(),
                });
            }
        }
        if env_is_set("LLAMA_BASE_URL", env) {
            return Some(AuthCheck {
                kind: "api_key".into(),
                source: "LLAMA_BASE_URL".into(),
            });
        }
        return None;
    }
    if provider == "google-vertex" {
        return vertex_ambient_auth(credential, env).map(auth_check_from_resolved);
    }
    if matches!(provider, "cloudflare-workers-ai" | "cloudflare-ai-gateway") {
        return cloudflare_auth(provider, credential, env).map(auth_check_from_resolved);
    }
    if provider == "amazon-bedrock" {
        if let Some(cred) = credential {
            if cred.key.as_ref().is_some_and(|k| !k.is_empty())
                || env_nonempty(cred.env.get("AWS_PROFILE"))
            {
                return Some(AuthCheck {
                    kind: "api_key".into(),
                    source: "stored credential".into(),
                });
            }
        }
        if let Some(source) = bedrock_ambient_source(env) {
            return Some(AuthCheck {
                kind: "api_key".into(),
                source,
            });
        }
        return None;
    }
    if let Some(cred) = credential {
        if cred.key.as_ref().is_some_and(|k| !k.is_empty()) {
            return Some(AuthCheck {
                kind: "api_key".into(),
                source: "stored credential".into(),
            });
        }
    }
    None
}

fn model_allowed_for_credential(model: &Model, credential: Option<&Credential>) -> bool {
    if model.provider != "github-copilot" {
        return true;
    }
    let Some(cred) = credential else {
        return true;
    };
    if cred.kind != CredentialKind::Oauth {
        return true;
    }
    if cred.available_model_ids.is_empty() {
        return true;
    }
    cred.available_model_ids.iter().any(|id| id == &model.id)
}

fn env_is_set(name: &str, env: &HashMap<String, String>) -> bool {
    env.get(name).is_some_and(|value| !value.is_empty())
}

fn env_nonempty(value: Option<&String>) -> bool {
    value.is_some_and(|text| !text.is_empty())
}

fn auth_check_from_resolved(resolved: ResolvedAuth) -> AuthCheck {
    let kind = if resolved.source == "OAuth" || resolved.source.contains("oauth") {
        "oauth"
    } else {
        "api_key"
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
            thinking_level_map: Default::default(),
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
            Some("configured API key")
        );
        assert_eq!(
            snap.auth.get("local").map(|c| c.kind.as_str()),
            Some("api_key")
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
        let no_model = format_no_model_selected_message(Path::new("/docs"));
        assert!(no_model.starts_with("No model selected."));
        assert!(no_model.contains("Then use /model to select a model."));
        let no_key = format_no_api_key_found_message("fake-provider", Path::new("/docs"));
        assert!(no_key.starts_with("No API key found for fake-provider."));
        assert!(no_key.contains("Use /login to log into a provider via OAuth or API key. See:"));
    }

    #[test]
    fn check_auth_does_not_refresh_expired_oauth() {
        let dir = tempdir().unwrap();
        let mut storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        storage
            .login_oauth("anthropic", "expired", Some("refresh".into()), Some(0))
            .unwrap();
        let check = check_auth(
            "anthropic",
            &ModelConfig::empty(),
            &storage,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(check.kind, "oauth");
        assert_eq!(check.source, "OAuth");
        assert_eq!(
            storage.get("anthropic").and_then(|c| c.access.as_deref()),
            Some("expired")
        );
    }

    #[test]
    fn command_api_key_is_configured_without_executing() {
        let dir = tempdir().unwrap();
        let counter = dir.path().join("counter");
        std::fs::write(&counter, "0").unwrap();
        let path = dir.path().join("models.json");
        let command = format!("!sh -c 'echo 1 > {}'", counter.display());
        std::fs::write(
            &path,
            format!(
                r#"{{"providers":{{"local":{{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","apiKey":{command},"models":[{{"id":"demo"}}]}}}}}}"#,
                command = serde_json::to_string(&command).unwrap()
            ),
        )
        .unwrap();
        let config = ModelConfig::load(&path);
        assert!(config.error().is_none(), "{:?}", config.error());
        let models = config.apply(&[]).unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let check = check_auth("local", &config, &storage, &Default::default()).unwrap();
        assert_eq!(check.kind, "api_key");
        assert_eq!(check.source, "configured API key");
        let available = get_available(&models, None, &config, &storage, &Default::default());
        assert_eq!(available.len(), 1);
        assert_eq!(std::fs::read_to_string(&counter).unwrap(), "0");
    }

    #[test]
    fn env_template_api_key_requires_named_vars() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(
            &path,
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","apiKey":"$PI_TEST_LOCAL_KEY","models":[{"id":"demo"}]}}}"#,
        )
        .unwrap();
        let config = ModelConfig::load(&path);
        let models = config.apply(&[]).unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        assert!(check_auth("local", &config, &storage, &Default::default()).is_none());
        let mut env = HashMap::new();
        env.insert("PI_TEST_LOCAL_KEY".into(), "sk-from-env".into());
        let check = check_auth("local", &config, &storage, &env).unwrap();
        assert_eq!(check.source, "configured API key");
        assert_eq!(
            get_available(&models, Some("local"), &config, &storage, &env).len(),
            1
        );
    }

    #[test]
    fn github_copilot_oauth_filters_available_model_ids() {
        let dir = tempdir().unwrap();
        let mut storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        storage
            .set(
                "github-copilot",
                crate::auth::Credential {
                    kind: crate::auth::CredentialKind::Oauth,
                    key: None,
                    access: Some("token".into()),
                    refresh: None,
                    expires: None,
                    env: HashMap::new(),
                    available_model_ids: vec!["gpt-4.1".into()],
                },
            )
            .unwrap();
        let all = vec![
            demo_model("github-copilot", "gpt-4.1"),
            demo_model("github-copilot", "claude-sonnet"),
        ];
        let available = get_available(
            &all,
            None,
            &ModelConfig::empty(),
            &storage,
            &Default::default(),
        );
        assert_eq!(
            available.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["gpt-4.1"]
        );
    }

    #[test]
    fn bedrock_ambient_sources_and_access_key_pair() {
        let dir = tempdir().unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let config = ModelConfig::empty();
        let mut only_id = HashMap::new();
        only_id.insert("AWS_ACCESS_KEY_ID".into(), "AKIATEST".into());
        assert!(check_auth("amazon-bedrock", &config, &storage, &only_id).is_none());
        let mut keys = HashMap::new();
        keys.insert("AWS_ACCESS_KEY_ID".into(), "AKIATEST".into());
        keys.insert("AWS_SECRET_ACCESS_KEY".into(), "secret".into());
        let check = check_auth("amazon-bedrock", &config, &storage, &keys).unwrap();
        assert_eq!(check.kind, "api_key");
        assert_eq!(check.source, "AWS access keys");
        let mut ecs = HashMap::new();
        ecs.insert(
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI".into(),
            "/creds".into(),
        );
        assert_eq!(
            check_auth("amazon-bedrock", &config, &storage, &ecs)
                .unwrap()
                .source,
            "ECS task role"
        );
        let mut web = HashMap::new();
        web.insert("AWS_WEB_IDENTITY_TOKEN_FILE".into(), "/token".into());
        assert_eq!(
            check_auth("amazon-bedrock", &config, &storage, &web)
                .unwrap()
                .source,
            "web identity token"
        );
    }

    #[test]
    fn vertex_requires_adc_project_and_location() {
        let dir = tempdir().unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let config = ModelConfig::empty();
        let mut project_only = HashMap::new();
        project_only.insert("GOOGLE_CLOUD_PROJECT".into(), "demo".into());
        assert!(check_auth("google-vertex", &config, &storage, &project_only).is_none());
        let creds = dir.path().join("adc.json");
        std::fs::write(&creds, "{}").unwrap();
        let mut env = HashMap::new();
        env.insert(
            "GOOGLE_APPLICATION_CREDENTIALS".into(),
            creds.display().to_string(),
        );
        env.insert("GOOGLE_CLOUD_PROJECT".into(), "demo".into());
        env.insert("GOOGLE_CLOUD_LOCATION".into(), "us-central1".into());
        let check = check_auth("google-vertex", &config, &storage, &env).unwrap();
        assert_eq!(check.kind, "api_key");
        assert_eq!(check.source, "gcloud application default credentials");
        env.insert("GOOGLE_CLOUD_API_KEY".into(), "vertex-key".into());
        assert_eq!(
            check_auth("google-vertex", &config, &storage, &env)
                .unwrap()
                .source,
            "GOOGLE_CLOUD_API_KEY"
        );
    }

    #[test]
    fn cloudflare_requires_account_and_optional_gateway() {
        let dir = tempdir().unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let config = ModelConfig::empty();
        let mut account_only = HashMap::new();
        account_only.insert("CLOUDFLARE_ACCOUNT_ID".into(), "acct".into());
        assert!(check_auth("cloudflare-workers-ai", &config, &storage, &account_only).is_none());
        let mut workers = HashMap::new();
        workers.insert("CLOUDFLARE_API_KEY".into(), "cf-key".into());
        workers.insert("CLOUDFLARE_ACCOUNT_ID".into(), "acct".into());
        assert_eq!(
            check_auth("cloudflare-workers-ai", &config, &storage, &workers)
                .unwrap()
                .source,
            "CLOUDFLARE_API_KEY"
        );
        assert!(check_auth("cloudflare-ai-gateway", &config, &storage, &workers).is_none());
        workers.insert("CLOUDFLARE_GATEWAY_ID".into(), "gw".into());
        assert_eq!(
            check_auth("cloudflare-ai-gateway", &config, &storage, &workers)
                .unwrap()
                .kind,
            "api_key"
        );
    }

    #[test]
    fn llama_base_url_configures_without_stored_key() {
        let dir = tempdir().unwrap();
        let storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        let mut env = HashMap::new();
        env.insert("LLAMA_BASE_URL".into(), "http://127.0.0.1:8080".into());
        let check = check_auth("llama.cpp", &ModelConfig::empty(), &storage, &env).unwrap();
        assert_eq!(check.kind, "api_key");
        assert_eq!(check.source, "LLAMA_BASE_URL");
    }

    #[test]
    fn parse_copilot_models_fixture_uses_picker_ids() {
        let ids = crate::parse_copilot_available_model_ids(
            r#"{"data":[{"id":"gpt-4.1","model_picker_enabled":true,"policy":{"state":"enabled"}},{"id":"hidden","model_picker_enabled":false,"policy":{"state":"enabled"}}]}"#,
        );
        assert_eq!(ids, vec!["gpt-4.1".to_string()]);
    }
}
