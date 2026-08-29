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
    models.extend(openrouter_image_models());
    models.extend(load_radius_models());
    models
}

pub fn models_from_provider_config(name: &str, config: &serde_json::Value) -> Vec<Model> {
    let Some(entries) = config.get("models").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let api = config
        .get("api")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("openai-completions");
    let base_url = config.get("baseUrl").and_then(serde_json::Value::as_str);
    entries
        .iter()
        .filter_map(|entry| {
            let mut model = entry.clone();
            if model.get("provider").is_none() {
                model["provider"] = serde_json::json!(name);
            }
            if model.get("api").is_none() {
                model["api"] = serde_json::json!(api);
            }
            if model.get("baseUrl").is_none() {
                if let Some(url) = base_url {
                    model["baseUrl"] = serde_json::json!(url);
                }
            }
            if model.get("cost").is_none() {
                model["cost"] = serde_json::json!({
                    "input": 0.0,
                    "output": 0.0,
                    "cacheRead": 0.0,
                    "cacheWrite": 0.0
                });
            }
            if model.get("contextWindow").is_none() {
                model["contextWindow"] = serde_json::json!(128000);
            }
            if model.get("maxTokens").is_none() {
                model["maxTokens"] = serde_json::json!(4096);
            }
            if model.get("input").is_none() {
                model["input"] = serde_json::json!(["text"]);
            }
            if model.get("name").is_none() {
                if let Some(id) = model.get("id").cloned() {
                    model["name"] = id;
                }
            }
            serde_json::from_value(model).ok()
        })
        .collect()
}

pub fn openrouter_image_models() -> Vec<Model> {
    vec![Model {
        id: "google/gemini-2.5-flash-image".into(),
        name: "Gemini 2.5 Flash Image".into(),
        api: "openrouter-images".into(),
        provider: "openrouter".into(),
        base_url: Some("https://openrouter.ai/api/v1".into()),
        reasoning: false,
        input: vec!["text".into(), "image".into()],
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 32_768,
        max_tokens: 8192,
        compat: serde_json::Value::Null,
    }]
}

pub fn load_radius_models() -> Vec<Model> {
    let config = if let Ok(path) = std::env::var("PI_RADIUS_CONFIG_REPLY") {
        std::fs::read_to_string(path).ok()
    } else if let Ok(body) = std::env::var("PI_RADIUS_CONFIG_JSON") {
        Some(body)
    } else if std::env::var("PI_RADIUS_DRY_RUN").is_ok() || cfg!(test) {
        None
    } else {
        fetch_radius_config()
    };
    let Some(raw) = config else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    radius_models_from_config(&value)
}

fn fetch_radius_config() -> Option<String> {
    let gateway =
        std::env::var("PI_RADIUS_GATEWAY").unwrap_or_else(|_| "https://radius.pi.dev".into());
    let gateway = gateway.trim_end_matches('/');
    let url = format!("{gateway}/v1/config");
    let mut request = ureq::get(&url).set("accept", "application/json");
    if let Ok(key) = std::env::var("RADIUS_API_KEY") {
        request = request.set("Authorization", &format!("Bearer {key}"));
    } else if let Ok(key) = std::env::var("PI_RADIUS_TOKEN") {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    request.call().ok()?.into_string().ok()
}

pub fn radius_models_from_config(config: &serde_json::Value) -> Vec<Model> {
    let base_url = config
        .get("baseUrl")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("https://radius.pi.dev");
    let mut wrapper = config.clone();
    if wrapper.get("api").is_none() {
        wrapper["api"] = serde_json::json!("pi-messages");
    }
    if wrapper.get("baseUrl").is_none() {
        wrapper["baseUrl"] = serde_json::json!(base_url);
    }
    models_from_provider_config("radius", &wrapper)
}

pub fn builtin_provider_ids() -> Vec<&'static str> {
    KNOWN_PROVIDERS
        .iter()
        .copied()
        .filter(|id| catalog_json(id).is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_and_registered_providers_flatten_models() {
        let config = serde_json::json!({
            "baseUrl": "https://radius.pi.dev/v1",
            "models": [{
                "id": "radius-demo",
                "name": "Radius Demo",
                "reasoning": false,
                "input": ["text"],
                "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0 },
                "contextWindow": 128000,
                "maxTokens": 4096
            }]
        });
        let models = radius_models_from_config(&config);
        assert_eq!(models[0].provider, "radius");
        assert_eq!(models[0].api, "pi-messages");
        assert_eq!(models[0].id, "radius-demo");
        let registered = models_from_provider_config(
            "my-proxy",
            &serde_json::json!({
                "baseUrl": "https://proxy.example.com",
                "api": "anthropic-messages",
                "models": [{ "id": "demo", "name": "Demo" }]
            }),
        );
        assert_eq!(registered[0].provider, "my-proxy");
        assert_eq!(registered[0].api, "anthropic-messages");
        assert!(openrouter_image_models()
            .iter()
            .all(|model| model.api == "openrouter-images"));
    }
}
