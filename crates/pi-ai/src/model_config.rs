//! `models.json` ModelConfig matching TS `model-config.ts` + `provider-composer.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::catalog::{Model, ModelCost};

pub const NO_MODELS_AVAILABLE: &str =
    "No models available. Check your installation or add models to models.json.";

#[derive(Debug, Clone, Default)]
pub struct ModelsJsonProvider {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api: Option<String>,
    pub oauth: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub compat: Value,
    pub auth_header: Option<bool>,
    pub models: Vec<Value>,
    pub model_overrides: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelConfig {
    providers: BTreeMap<String, ModelsJsonProvider>,
    error: Option<String>,
    path: Option<PathBuf>,
}

impl ModelConfig {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn provider_ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    pub fn get_provider(&self, id: &str) -> Option<&ModelsJsonProvider> {
        self.providers.get(id)
    }

    pub fn load(path: &Path) -> Self {
        let path = normalize_path(path);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::empty(),
            Err(err) => {
                return Self {
                    error: Some(format!(
                        "Failed to load models.json: {err}\n\nFile: {}",
                        path.display()
                    )),
                    path: Some(path),
                    ..Self::default()
                };
            }
        };
        let parsed = match serde_json::from_str::<Value>(&strip_json_comments(&strip_bom(&raw))) {
            Ok(value) => value,
            Err(err) => {
                return Self {
                    error: Some(format!(
                        "Failed to parse models.json: {err}\n\nFile: {}",
                        path.display()
                    )),
                    path: Some(path),
                    ..Self::default()
                };
            }
        };
        match parse_models_json(&parsed) {
            Ok(providers) => Self {
                providers,
                error: None,
                path: Some(path),
            },
            Err(errors) => Self {
                error: Some(format!(
                    "Invalid models.json schema:\n{errors}\n\nFile: {}",
                    path.display()
                )),
                path: Some(path),
                ..Self::default()
            },
        }
    }

    pub fn apply(&self, baseline: &[Model]) -> Result<Vec<Model>, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let mut models = baseline.to_vec();
        for (provider_id, config) in &self.providers {
            models = apply_models_json(provider_id, &models, config)?;
            models = apply_model_overrides(provider_id, models, config);
        }
        Ok(models)
    }
}

pub fn models_json_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("models.json")
}

pub fn load_models_json(path: &Path) -> Result<Vec<Model>, String> {
    let config = ModelConfig::load(path);
    if let Some(error) = config.error() {
        return Err(error.to_string());
    }
    config.apply(&[])
}

pub fn apply_models_config(baseline: &[Model], config: &ModelConfig) -> Result<Vec<Model>, String> {
    config.apply(baseline)
}

pub fn merge_headers(
    base: &BTreeMap<String, String>,
    overlay: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = base.clone();
    for (name, value) in overlay {
        let lower = name.to_ascii_lowercase();
        merged.retain(|existing, _| existing.to_ascii_lowercase() != lower);
        merged.insert(name.clone(), value.clone());
    }
    merged
}

pub fn apply_config_auth(
    auth: &mut crate::ResolvedAuth,
    config: &ModelConfig,
    provider: &str,
    model: Option<&Model>,
) {
    let Some(provider_config) = config.get_provider(provider) else {
        if let Some(model) = model {
            for (key, value) in &model.headers {
                auth.headers.insert(key.clone(), value.clone());
            }
        }
        return;
    };
    if auth.api_key.is_none() {
        if let Some(key) = &provider_config.api_key {
            auth.api_key = Some(key.clone());
            auth.source = "models.json".into();
        }
    }
    for (key, value) in &provider_config.headers {
        let lower = key.to_ascii_lowercase();
        auth.headers
            .retain(|existing, _| existing.to_ascii_lowercase() != lower);
        auth.headers.insert(key.clone(), value.clone());
    }
    if let Some(model) = model {
        for (key, value) in &model.headers {
            let lower = key.to_ascii_lowercase();
            auth.headers
                .retain(|existing, _| existing.to_ascii_lowercase() != lower);
            auth.headers.insert(key.clone(), value.clone());
        }
    }
    if provider_config.auth_header == Some(true) {
        if let Some(key) = &auth.api_key {
            auth.headers
                .insert("Authorization".into(), format!("Bearer {key}"));
        }
    }
}

fn parse_models_json(value: &Value) -> Result<BTreeMap<String, ModelsJsonProvider>, String> {
    let Some(providers) = value.get("providers").and_then(Value::as_object) else {
        return Err("  - providers: Expected object".into());
    };
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for (id, provider) in providers {
        match parse_provider(id, provider) {
            Ok(parsed) => {
                out.insert(id.clone(), parsed);
            }
            Err(err) => errors.push(err),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors.join("\n"))
    }
}

fn parse_provider(id: &str, value: &Value) -> Result<ModelsJsonProvider, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("  - providers.{id}: Expected object"));
    };
    let models = match object.get("models") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let id_value = item.get("id").and_then(Value::as_str).unwrap_or("");
                if id_value.is_empty() {
                    return Err(format!(
                        "  - providers.{id}.models.{index}.id: Expected string"
                    ));
                }
            }
            items.clone()
        }
        Some(_) => {
            return Err(format!("  - providers.{id}.models: Expected array"));
        }
    };
    let model_overrides = match object.get("modelOverrides") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        Some(_) => {
            return Err(format!(
                "  - providers.{id}.modelOverrides: Expected object"
            ));
        }
    };
    let headers = string_map(object.get("headers"), &format!("providers.{id}.headers"))?;
    Ok(ModelsJsonProvider {
        name: optional_string(object.get("name")),
        base_url: optional_string(object.get("baseUrl")),
        api_key: optional_string(object.get("apiKey")),
        api: optional_string(object.get("api")),
        oauth: optional_string(object.get("oauth")),
        headers,
        compat: object.get("compat").cloned().unwrap_or(Value::Null),
        auth_header: object.get("authHeader").and_then(Value::as_bool),
        models,
        model_overrides,
    })
}

fn apply_models_json(
    provider_id: &str,
    base_models: &[Model],
    config: &ModelsJsonProvider,
) -> Result<Vec<Model>, String> {
    if config.oauth.is_some() && config.base_url.is_none() {
        return Err(format!(
            "Provider {provider_id}: \"baseUrl\" is required when \"oauth\" is set."
        ));
    }
    let has_overrides = !config.model_overrides.is_empty();
    if config.models.is_empty()
        && config.base_url.is_none()
        && config.headers.is_empty()
        && config.compat.is_null()
        && !has_overrides
        && config.api_key.is_none()
        && config.oauth.is_none()
        && config.auth_header.is_none()
    {
        return Err(format!(
            "Provider {provider_id}: must specify \"baseUrl\", \"headers\", \"compat\", \"modelOverrides\", or \"models\"."
        ));
    }

    let mut models: Vec<Model> = base_models
        .iter()
        .filter(|model| model.provider == provider_id)
        .cloned()
        .map(|mut model| {
            if config.oauth.as_deref() != Some("radius") {
                if let Some(url) = &config.base_url {
                    model.base_url = Some(url.clone());
                }
            }
            model.compat = merge_compat(&model.compat, &config.compat);
            model.headers = merge_headers(&model.headers, &config.headers);
            model
        })
        .collect();
    let mut others: Vec<Model> = base_models
        .iter()
        .filter(|model| model.provider != provider_id)
        .cloned()
        .collect();

    for definition in &config.models {
        let id = definition
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let existing_index = models.iter().position(|model| model.id == id);
        let defaults = existing_index
            .and_then(|index| models.get(index).cloned())
            .or_else(|| models.first().cloned());
        let model = model_from_json(provider_id, definition, config, defaults.as_ref())?;
        if let Some(index) = existing_index {
            models[index] = model;
        } else {
            models.push(model);
        }
    }
    others.extend(models);
    Ok(others)
}

fn apply_model_overrides(
    provider_id: &str,
    mut models: Vec<Model>,
    config: &ModelsJsonProvider,
) -> Vec<Model> {
    for model in &mut models {
        if model.provider != provider_id {
            continue;
        }
        if let Some(override_value) = config.model_overrides.get(&model.id) {
            apply_override(model, override_value);
        }
    }
    models
}

fn model_from_json(
    provider_id: &str,
    definition: &Value,
    provider_config: &ModelsJsonProvider,
    defaults: Option<&Model>,
) -> Result<Model, String> {
    let id = definition
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let api = definition
        .get("api")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| provider_config.api.clone())
        .or_else(|| defaults.map(|model| model.api.clone()));
    let Some(api) = api else {
        return Err(format!(
            "Provider {provider_id}, model {id}: no \"api\" specified. Set at provider or model level."
        ));
    };
    let base_url = definition
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| provider_config.base_url.clone())
        .or_else(|| defaults.and_then(|model| model.base_url.clone()));
    let Some(base_url) = base_url else {
        return Err(format!(
            "Provider {provider_id}: \"baseUrl\" is required when defining custom models."
        ));
    };
    if let Some(window) = definition.get("contextWindow").and_then(Value::as_u64) {
        if window == 0 {
            return Err(format!(
                "Provider {provider_id}, model {id}: invalid contextWindow"
            ));
        }
    }
    if let Some(tokens) = definition.get("maxTokens").and_then(Value::as_u64) {
        if tokens == 0 {
            return Err(format!(
                "Provider {provider_id}, model {id}: invalid maxTokens"
            ));
        }
    }
    let cost = definition
        .get("cost")
        .and_then(|value| serde_json::from_value::<ModelCost>(value.clone()).ok())
        .unwrap_or(ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        });
    let input = definition
        .get("input")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| vec!["text".into()]);
    let headers = string_map(
        definition.get("headers"),
        &format!("providers.{provider_id}.models"),
    )
    .unwrap_or_default();
    Ok(Model {
        id: id.to_string(),
        name: definition
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string(),
        api,
        provider: provider_id.to_string(),
        base_url: Some(base_url),
        reasoning: definition
            .get("reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        input,
        cost,
        context_window: definition
            .get("contextWindow")
            .and_then(Value::as_u64)
            .unwrap_or(128_000),
        max_tokens: definition
            .get("maxTokens")
            .and_then(Value::as_u64)
            .unwrap_or(16_384),
        compat: merge_compat(
            &provider_config.compat,
            definition.get("compat").unwrap_or(&Value::Null),
        ),
        headers: merge_headers(&provider_config.headers, &headers),
    })
}

fn apply_override(model: &mut Model, override_value: &Value) {
    if let Some(name) = override_value.get("name").and_then(Value::as_str) {
        model.name = name.to_string();
    }
    if let Some(reasoning) = override_value.get("reasoning").and_then(Value::as_bool) {
        model.reasoning = reasoning;
    }
    if let Some(input) = override_value.get("input").and_then(Value::as_array) {
        model.input = input
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    if let Some(cost) = override_value.get("cost") {
        if let Some(input) = cost.get("input").and_then(Value::as_f64) {
            model.cost.input = input;
        }
        if let Some(output) = cost.get("output").and_then(Value::as_f64) {
            model.cost.output = output;
        }
        if let Some(cache_read) = cost.get("cacheRead").and_then(Value::as_f64) {
            model.cost.cache_read = cache_read;
        }
        if let Some(cache_write) = cost.get("cacheWrite").and_then(Value::as_f64) {
            model.cost.cache_write = cache_write;
        }
    }
    if let Some(window) = override_value.get("contextWindow").and_then(Value::as_u64) {
        model.context_window = window;
    }
    if let Some(tokens) = override_value.get("maxTokens").and_then(Value::as_u64) {
        model.max_tokens = tokens;
    }
    if let Some(compat) = override_value.get("compat") {
        model.compat = merge_compat(&model.compat, compat);
    }
    if let Ok(headers) = string_map(override_value.get("headers"), "headers") {
        model.headers = merge_headers(&model.headers, &headers);
    }
}

fn merge_compat(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            let mut merged = base.clone();
            for (key, value) in overlay {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        (_, Value::Null) => base.clone(),
        (Value::Null, overlay) => overlay.clone(),
        (_, overlay) => overlay.clone(),
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_map(value: Option<&Value>, path: &str) -> Result<BTreeMap<String, String>, String> {
    match value {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (key, item) in map {
                if let Some(text) = item.as_str() {
                    out.insert(key.clone(), text.to_string());
                } else {
                    return Err(format!("  - {path}.{key}: Expected string"));
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("  - {path}: Expected object")),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn strip_bom(raw: &str) -> String {
    raw.strip_prefix('\u{feff}').unwrap_or(raw).to_string()
}

fn strip_json_comments(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = index.saturating_add(2);
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_is_empty_config() {
        let dir = tempdir().unwrap();
        let config = ModelConfig::load(&dir.path().join("models.json"));
        assert!(config.error().is_none());
        assert!(config.apply(&[]).unwrap().is_empty());
    }

    #[test]
    fn parse_error_locks_ts_string() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(&path, "{").unwrap();
        let config = ModelConfig::load(&path);
        let error = config.error().unwrap();
        assert!(error.starts_with("Failed to parse models.json:"), "{error}");
        assert!(error.contains(&format!("File: {}", path.display())) || error.contains("File:"));
    }

    #[test]
    fn schema_error_locks_ts_string() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(&path, r#"{"providers":[]}"#).unwrap();
        let config = ModelConfig::load(&path);
        let error = config.error().unwrap();
        assert!(error.starts_with("Invalid models.json schema:"), "{error}");
        assert!(error.contains("providers"));
    }

    #[test]
    fn upserts_custom_model_and_applies_overrides() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(
            &path,
            r#"
            {
              // custom provider
              "providers": {
                "local": {
                  "baseUrl": "http://127.0.0.1:9",
                  "api": "openai-completions",
                  "apiKey": "sk-test",
                  "headers": { "X-Test": "1" },
                  "authHeader": true,
                  "models": [{ "id": "demo", "name": "Demo" }],
                  "modelOverrides": { "demo": { "name": "Demo Override", "maxTokens": 32 } }
                }
              }
            }
            "#,
        )
        .unwrap();
        let config = ModelConfig::load(&path);
        assert!(config.error().is_none(), "{:?}", config.error());
        let models = config.apply(&[]).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "demo");
        assert_eq!(models[0].name, "Demo Override");
        assert_eq!(models[0].max_tokens, 32);
        assert_eq!(models[0].base_url.as_deref(), Some("http://127.0.0.1:9"));
        assert_eq!(
            models[0].headers.get("X-Test").map(String::as_str),
            Some("1")
        );
        let mut auth = crate::ResolvedAuth {
            api_key: None,
            headers: Default::default(),
            source: "none".into(),
        };
        apply_config_auth(&mut auth, &config, "local", models.first());
        assert_eq!(auth.api_key.as_deref(), Some("sk-test"));
        assert_eq!(
            auth.headers.get("Authorization").map(String::as_str),
            Some("Bearer sk-test")
        );
    }

    #[test]
    fn no_models_copy_is_locked() {
        assert_eq!(
            NO_MODELS_AVAILABLE,
            "No models available. Check your installation or add models to models.json."
        );
    }
}
