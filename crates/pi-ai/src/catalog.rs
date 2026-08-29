use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

const RAW_CATALOGS: &str = include_str!("catalogs.json");

fn catalogs() -> &'static serde_json::Map<String, Value> {
    use std::sync::OnceLock;
    static CATALOGS: OnceLock<serde_json::Map<String, Value>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        serde_json::from_str::<Value>(RAW_CATALOGS)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    })
}

pub fn flatten_catalog(provider: &str, groups: &Value) -> Vec<Model> {
    let mut models = Vec::new();
    if let Some(map) = groups.as_object() {
        for models_by_id in map.values() {
            if let Some(items) = models_by_id.as_object() {
                for (id, raw) in items {
                    if let Ok(mut model) = serde_json::from_value::<Model>(raw.clone()) {
                        if model.id.is_empty() {
                            model.id = id.clone();
                        }
                        if model.provider.is_empty() {
                            model.provider = provider.to_string();
                        }
                        models.push(model);
                    } else if let Some(obj) = raw.as_object() {
                        models.push(Model {
                            id: obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(id)
                                .to_string(),
                            name: obj
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(id)
                                .to_string(),
                            api: obj
                                .get("api")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            provider: obj
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or(provider)
                                .to_string(),
                            base_url: obj
                                .get("baseUrl")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            reasoning: obj
                                .get("reasoning")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            input: obj
                                .get("input")
                                .and_then(|v| v.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v.as_str().map(str::to_string))
                                        .collect()
                                })
                                .unwrap_or_else(|| vec!["text".into()]),
                            cost: serde_json::from_value(
                                obj.get("cost").cloned().unwrap_or(Value::Null),
                            )
                            .unwrap_or(ModelCost {
                                input: 0.0,
                                output: 0.0,
                                cache_read: 0.0,
                                cache_write: 0.0,
                            }),
                            context_window: obj
                                .get("contextWindow")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(128_000),
                            max_tokens: obj
                                .get("maxTokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(16_384),
                            extra: obj.clone(),
                        });
                    }
                }
            }
        }
    }
    models
}

pub fn builtin_providers() -> Vec<String> {
    catalogs().keys().cloned().collect()
}

pub fn list_models(provider: Option<&str>) -> Vec<Model> {
    let mut models = Vec::new();
    for (name, groups) in catalogs() {
        if provider.map(|p| p == name).unwrap_or(true) {
            models.extend(flatten_catalog(name, groups));
        }
    }
    models
}

pub fn get_builtin_model(provider: &str, model_id: &str) -> Option<Model> {
    list_models(Some(provider))
        .into_iter()
        .find(|m| m.id == model_id)
}

pub fn resolve_model(pattern: &str) -> Option<Model> {
    if let Some((provider, id)) = pattern.split_once('/') {
        if let Some(model) = get_builtin_model(provider, id) {
            return Some(model);
        }
    }
    let needle = pattern.to_ascii_lowercase();
    list_models(None).into_iter().find(|m| {
        m.id.to_ascii_lowercase() == needle
            || m.id.to_ascii_lowercase().contains(&needle)
            || m.name.to_ascii_lowercase().contains(&needle)
    })
}
