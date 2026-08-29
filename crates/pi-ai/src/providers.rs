use crate::catalog::{list_models, Model};

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub api: String,
    pub base_url: Option<String>,
    pub env_keys: Vec<String>,
}

pub fn provider_display_name(id: &str) -> &'static str {
    match id {
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "google" => "Google",
        "openai-codex" => "OpenAI Codex",
        "github-copilot" => "GitHub Copilot",
        "openrouter" => "OpenRouter",
        other => Box::leak(other.to_string().into_boxed_str()),
    }
}

pub fn all_providers() -> Vec<ProviderInfo> {
    let mut seen = std::collections::BTreeMap::new();
    for model in list_models(None) {
        seen.entry(model.provider.clone())
            .or_insert_with(|| ProviderInfo {
                id: model.provider.clone(),
                name: provider_display_name(&model.provider).to_string(),
                api: model.api.clone(),
                base_url: model.base_url.clone(),
                env_keys: crate::env_api_key_vars(&model.provider)
                    .unwrap_or(&[])
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            });
    }
    seen.into_values().collect()
}

pub fn models_for_provider(id: &str) -> Vec<Model> {
    list_models(Some(id))
}
