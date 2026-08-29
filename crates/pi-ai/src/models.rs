use crate::types::*;
use std::collections::HashMap;
use std::sync::LazyLock;

pub static BUILTIN_ANTHROPIC_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/anthropic.json");
pub static BUILTIN_OPENAI_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/openai.json");
pub static BUILTIN_GOOGLE_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/google.json");
pub static BUILTIN_OPENROUTER_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/openrouter.json");
pub static BUILTIN_MISTRAL_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/mistral.json");
pub static BUILTIN_GROQ_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/groq.json");
pub static BUILTIN_CEREBRAS_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/cerebras.json");
pub static BUILTIN_DEEPSEEK_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/deepseek.json");
pub static BUILTIN_XAI_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/xai.json");
pub static BUILTIN_TOGETHER_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/together.json");
pub static BUILTIN_FIREWORKS_DATA: &str =
    include_str!("../../../vendor/pi/packages/ai/src/providers/data/fireworks.json");

pub fn parse_models_from_json(json_str: &str) -> Vec<Model> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    if let serde_json::Value::Object(map) = val {
        for (_api_key, group_val) in map {
            if let serde_json::Value::Object(group) = group_val {
                for (_model_id, m_val) in group {
                    if let Ok(model) = serde_json::from_value::<Model>(m_val) {
                        models.push(model);
                    }
                }
            }
        }
    }
    models
}

pub static BUILTIN_MODELS: LazyLock<HashMap<String, Vec<Model>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "anthropic".to_string(),
        parse_models_from_json(BUILTIN_ANTHROPIC_DATA),
    );
    map.insert(
        "openai".to_string(),
        parse_models_from_json(BUILTIN_OPENAI_DATA),
    );
    map.insert(
        "google".to_string(),
        parse_models_from_json(BUILTIN_GOOGLE_DATA),
    );
    map.insert(
        "openrouter".to_string(),
        parse_models_from_json(BUILTIN_OPENROUTER_DATA),
    );
    map.insert(
        "mistral".to_string(),
        parse_models_from_json(BUILTIN_MISTRAL_DATA),
    );
    map.insert(
        "groq".to_string(),
        parse_models_from_json(BUILTIN_GROQ_DATA),
    );
    map.insert(
        "cerebras".to_string(),
        parse_models_from_json(BUILTIN_CEREBRAS_DATA),
    );
    map.insert(
        "deepseek".to_string(),
        parse_models_from_json(BUILTIN_DEEPSEEK_DATA),
    );
    map.insert("xai".to_string(), parse_models_from_json(BUILTIN_XAI_DATA));
    map.insert(
        "together".to_string(),
        parse_models_from_json(BUILTIN_TOGETHER_DATA),
    );
    map.insert(
        "fireworks".to_string(),
        parse_models_from_json(BUILTIN_FIREWORKS_DATA),
    );
    map
});

pub fn get_builtin_model(provider: &str, model_id: &str) -> Option<Model> {
    if let Some(list) = BUILTIN_MODELS.get(provider) {
        return list.iter().find(|m| m.id == model_id).cloned();
    }
    None
}

pub fn get_builtin_providers() -> Vec<&'static str> {
    vec![
        "anthropic",
        "openai",
        "google",
        "openrouter",
        "mistral",
        "groq",
        "cerebras",
        "deepseek",
        "xai",
        "together",
        "fireworks",
    ]
}
