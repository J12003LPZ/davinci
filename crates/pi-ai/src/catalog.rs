use serde::{Deserialize, Serialize};

include!("catalog_include.rs");

pub fn builtin_catalog_json(provider: &str) -> Option<&'static str> {
    catalog_json(provider)
}

pub const KNOWN_PROVIDERS: &[&str] = &[
    "amazon-bedrock",
    "ant-ling",
    "anthropic",
    "google",
    "google-vertex",
    "openai",
    "azure-openai-responses",
    "openai-codex",
    "radius",
    "nvidia",
    "deepseek",
    "github-copilot",
    "xai",
    "groq",
    "cerebras",
    "openrouter",
    "vercel-ai-gateway",
    "zai",
    "zai-coding-cn",
    "mistral",
    "minimax",
    "minimax-cn",
    "moonshotai",
    "moonshotai-cn",
    "huggingface",
    "fireworks",
    "together",
    "baseten",
    "opencode",
    "opencode-go",
    "kimi-coding",
    "cloudflare-workers-ai",
    "cloudflare-ai-gateway",
    "qwen-token-plan",
    "qwen-token-plan-cn",
    "qwen-token-plan-individual",
    "xiaomi",
    "xiaomi-token-plan-cn",
    "xiaomi-token-plan-ams",
    "xiaomi-token-plan-sgp",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    #[serde(rename = "baseUrl", default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    pub cost: ModelCost,
    #[serde(rename = "contextWindow")]
    pub context_window: u64,
    #[serde(rename = "maxTokens")]
    pub max_tokens: u64,
    #[serde(default)]
    pub compat: serde_json::Value,
}

pub fn flatten_catalog(provider: &str, groups: &serde_json::Value) -> Vec<Model> {
    let mut models = Vec::new();
    let Some(map) = groups.as_object() else {
        return models;
    };
    for (_api, group) in map {
        if let Some(entries) = group.as_object() {
            for (_id, value) in entries {
                if let Ok(mut model) = serde_json::from_value::<Model>(value.clone()) {
                    if model.provider.is_empty() {
                        model.provider = provider.to_string();
                    }
                    models.push(model);
                }
            }
        }
    }
    models
}

pub fn load_builtin_models() -> Vec<Model> {
    let mut models = Vec::new();
    for provider in builtin_provider_ids() {
        if let Some(json) = catalog_json(provider) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
                models.extend(flatten_catalog(provider, &value));
            }
        }
    }
    models
}

pub fn builtin_provider_ids() -> Vec<&'static str> {
    KNOWN_PROVIDERS
        .iter()
        .copied()
        .filter(|id| catalog_json(id).is_some())
        .collect()
}
