//! `models.json` ModelConfig matching TS `model-config.ts` + `provider-composer.ts`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::catalog::{Model, ModelCost};

pub const NO_MODELS_AVAILABLE: &str =
    "No models available. Check your installation or add models to models.json.";

/// TS `isCommandConfigValue`: values starting with `!` are shell commands.
pub fn is_command_config_value(config: &str) -> bool {
    config.starts_with('!')
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplatePart {
    Literal(String),
    Env(String),
}

/// TS `getConfigValueEnvVarNames` for `$VAR` / `${VAR}` templates.
pub fn config_value_env_var_names(config: &str) -> Vec<String> {
    if is_command_config_value(config) {
        return Vec::new();
    }
    let mut names = Vec::new();
    for part in parse_config_value_template(config) {
        if let TemplatePart::Env(name) = part {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// TS `resolveConfigValue`: expand `$ENV` / `${ENV}` and (outside tests) `!command`.
pub fn resolve_config_value(config: &str, env: &HashMap<String, String>) -> Option<String> {
    if is_command_config_value(config) {
        return resolve_command_config_value(config);
    }
    resolve_template_value(&parse_config_value_template(config), env)
}

fn parse_config_value_template(config: &str) -> Vec<TemplatePart> {
    let mut parts = Vec::new();
    let chars: Vec<char> = config.chars().collect();
    let mut index = 0;
    let mut literal = String::new();
    let flush = |literal: &mut String, parts: &mut Vec<TemplatePart>| {
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(std::mem::take(literal)));
        }
    };
    while index < chars.len() {
        if chars[index] != '$' {
            literal.push(chars[index]);
            index += 1;
            continue;
        }
        let next = chars.get(index + 1).copied();
        if next == Some('$') || next == Some('!') {
            literal.push(next.unwrap());
            index += 2;
            continue;
        }
        if next == Some('{') {
            if let Some(end) = chars[index + 2..].iter().position(|c| *c == '}') {
                let name: String = chars[index + 2..index + 2 + end].iter().collect();
                if is_env_var_name(&name) {
                    flush(&mut literal, &mut parts);
                    parts.push(TemplatePart::Env(name));
                    index += 3 + end;
                    continue;
                }
                literal.push_str(&chars[index..index + 3 + end].iter().collect::<String>());
                index += 3 + end;
                continue;
            }
            literal.push('$');
            index += 1;
            continue;
        }
        let rest: String = chars[index + 1..].iter().collect();
        if let Some(name) = env_var_name_prefix(&rest) {
            flush(&mut literal, &mut parts);
            parts.push(TemplatePart::Env(name.clone()));
            index += 1 + name.len();
            continue;
        }
        literal.push('$');
        index += 1;
    }
    flush(&mut literal, &mut parts);
    parts
}

fn resolve_template_value(parts: &[TemplatePart], env: &HashMap<String, String>) -> Option<String> {
    let mut resolved = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(value) => resolved.push_str(value),
            TemplatePart::Env(name) => {
                let value = env.get(name).cloned().filter(|value| !value.is_empty())?;
                resolved.push_str(&value);
            }
        }
    }
    Some(resolved)
}

fn resolve_command_config_value(config: &str) -> Option<String> {
    if let Ok(reply) = std::env::var("PI_CONFIG_VALUE_REPLY") {
        return if reply.is_empty() { None } else { Some(reply) };
    }
    if cfg!(test) {
        return None;
    }
    let cache = command_result_cache();
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(config) {
            return cached.clone();
        }
    }
    let command = &config[1..];
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .output()
        .ok();
    let value = output.and_then(|result| {
        if !result.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&result.stdout).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    });
    if let Ok(mut guard) = cache.lock() {
        guard.insert(config.to_string(), value.clone());
    }
    value
}

fn command_result_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn env_var_name_prefix(rest: &str) -> Option<String> {
    let mut name = String::new();
    for (i, c) in rest.chars().enumerate() {
        if i == 0 {
            if !(c.is_ascii_alphabetic() || c == '_') {
                return None;
            }
        } else if !(c.is_ascii_alphanumeric() || c == '_') {
            break;
        }
        name.push(c);
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

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
    env: &HashMap<String, String>,
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
            if let Some(resolved) = resolve_config_value(key, env) {
                auth.api_key = Some(resolved);
                auth.source = "configured API key".into();
            }
        }
    }
    for (key, value) in &provider_config.headers {
        let lower = key.to_ascii_lowercase();
        auth.headers
            .retain(|existing, _| existing.to_ascii_lowercase() != lower);
        if let Some(resolved) = resolve_config_value(value, env) {
            auth.headers.insert(key.clone(), resolved);
        }
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
    validate_models_config(value)?;
    let providers = value
        .get("providers")
        .and_then(Value::as_object)
        .ok_or_else(|| "  - providers: Expected object".to_string())?;
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

fn validate_models_config(value: &Value) -> Result<(), String> {
    let mut errors = Vec::new();
    let Some(root) = value.as_object() else {
        return Err("  - root: Expected object".into());
    };
    match root.get("providers") {
        None => errors.push("  - providers: Expected required property".into()),
        Some(providers) => match providers.as_object() {
            None => errors.push("  - providers: Expected object".into()),
            Some(map) => {
                for (id, provider) in map {
                    validate_provider_schema(&format!("providers.{id}"), provider, &mut errors);
                }
            }
        },
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn validate_provider_schema(path: &str, value: &Value, errors: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    expect_optional_min_string(&format!("{path}.name"), object.get("name"), errors);
    expect_optional_min_string(&format!("{path}.baseUrl"), object.get("baseUrl"), errors);
    expect_optional_min_string(&format!("{path}.apiKey"), object.get("apiKey"), errors);
    expect_optional_min_string(&format!("{path}.api"), object.get("api"), errors);
    match object.get("oauth") {
        None | Some(Value::Null) => {}
        Some(Value::String(value)) if value == "radius" => {}
        Some(_) => errors.push(format!("  - {path}.oauth: Expected const value")),
    }
    match object.get("authHeader") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(_)) => {}
        Some(_) => errors.push(format!("  - {path}.authHeader: Expected boolean")),
    }
    validate_string_record(&format!("{path}.headers"), object.get("headers"), errors);
    validate_compat(&format!("{path}.compat"), object.get("compat"), errors);
    match object.get("models") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                validate_model_definition(&format!("{path}.models.{index}"), item, errors);
            }
        }
        Some(_) => errors.push(format!("  - {path}.models: Expected array")),
    }
    match object.get("modelOverrides") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (id, overlay) in map {
                validate_model_override(&format!("{path}.modelOverrides.{id}"), overlay, errors);
            }
        }
        Some(_) => errors.push(format!("  - {path}.modelOverrides: Expected object")),
    }
}

fn validate_model_definition(path: &str, value: &Value, errors: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    match object.get("id") {
        None => errors.push(format!("  - {path}.id: Expected required property")),
        Some(Value::String(id)) if id.is_empty() => {
            errors.push(format!(
                "  - {path}.id: Expected string length greater or equal to 1"
            ));
        }
        Some(Value::String(_)) => {}
        Some(_) => errors.push(format!("  - {path}.id: Expected string")),
    }
    expect_optional_min_string(&format!("{path}.name"), object.get("name"), errors);
    expect_optional_min_string(&format!("{path}.api"), object.get("api"), errors);
    expect_optional_min_string(&format!("{path}.baseUrl"), object.get("baseUrl"), errors);
    expect_optional_bool(
        &format!("{path}.reasoning"),
        object.get("reasoning"),
        errors,
    );
    expect_optional_number(
        &format!("{path}.contextWindow"),
        object.get("contextWindow"),
        errors,
    );
    expect_optional_number(
        &format!("{path}.maxTokens"),
        object.get("maxTokens"),
        errors,
    );
    validate_thinking_level_map(
        &format!("{path}.thinkingLevelMap"),
        object.get("thinkingLevelMap"),
        errors,
    );
    validate_input_array(&format!("{path}.input"), object.get("input"), errors);
    validate_model_cost(&format!("{path}.cost"), object.get("cost"), true, errors);
    validate_string_record(&format!("{path}.headers"), object.get("headers"), errors);
    validate_compat(&format!("{path}.compat"), object.get("compat"), errors);
}

fn validate_model_override(path: &str, value: &Value, errors: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    expect_optional_min_string(&format!("{path}.name"), object.get("name"), errors);
    expect_optional_bool(
        &format!("{path}.reasoning"),
        object.get("reasoning"),
        errors,
    );
    expect_optional_number(
        &format!("{path}.contextWindow"),
        object.get("contextWindow"),
        errors,
    );
    expect_optional_number(
        &format!("{path}.maxTokens"),
        object.get("maxTokens"),
        errors,
    );
    validate_thinking_level_map(
        &format!("{path}.thinkingLevelMap"),
        object.get("thinkingLevelMap"),
        errors,
    );
    validate_input_array(&format!("{path}.input"), object.get("input"), errors);
    validate_model_cost(&format!("{path}.cost"), object.get("cost"), false, errors);
    validate_string_record(&format!("{path}.headers"), object.get("headers"), errors);
    validate_compat(&format!("{path}.compat"), object.get("compat"), errors);
}

fn validate_model_cost(
    path: &str,
    value: Option<&Value>,
    required_rates: bool,
    errors: &mut Vec<String>,
) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    for key in ["input", "output", "cacheRead", "cacheWrite"] {
        match object.get(key) {
            None if required_rates => {
                errors.push(format!("  - {path}.{key}: Expected required property"));
            }
            None => {}
            Some(item) if item.is_number() => {}
            Some(_) => errors.push(format!("  - {path}.{key}: Expected number")),
        }
    }
}

fn validate_input_array(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(items) = value.as_array() else {
        errors.push(format!("  - {path}: Expected array"));
        return;
    };
    for (index, item) in items.iter().enumerate() {
        match item.as_str() {
            Some("text") | Some("image") => {}
            _ => errors.push(format!("  - {path}.{index}: Expected union value")),
        }
    }
}

fn validate_compat(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected union value"));
        return;
    };
    expect_optional_union(
        &format!("{path}.thinkingFormat"),
        object.get("thinkingFormat"),
        &[
            "openai",
            "openrouter",
            "together",
            "baseten",
            "deepseek",
            "zai",
            "qwen",
            "chat-template",
            "qwen-chat-template",
            "string-thinking",
            "ant-ling",
        ],
        errors,
    );
    expect_optional_const(
        &format!("{path}.cacheControlFormat"),
        object.get("cacheControlFormat"),
        "anthropic",
        errors,
    );
    expect_optional_union(
        &format!("{path}.maxTokensField"),
        object.get("maxTokensField"),
        &["max_completion_tokens", "max_tokens"],
        errors,
    );
    expect_optional_union(
        &format!("{path}.sessionAffinityFormat"),
        object.get("sessionAffinityFormat"),
        &["openai", "openai-nosession", "openrouter"],
        errors,
    );
    expect_optional_const(
        &format!("{path}.deferredToolsMode"),
        object.get("deferredToolsMode"),
        "kimi",
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsStore"),
        object.get("supportsStore"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsDeveloperRole"),
        object.get("supportsDeveloperRole"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsReasoningEffort"),
        object.get("supportsReasoningEffort"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsUsageInStreaming"),
        object.get("supportsUsageInStreaming"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsFinishReason"),
        object.get("supportsFinishReason"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsStrictMode"),
        object.get("supportsStrictMode"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsLongCacheRetention"),
        object.get("supportsLongCacheRetention"),
        errors,
    );
    expect_optional_bool(
        &format!("{path}.supportsEagerToolInputStreaming"),
        object.get("supportsEagerToolInputStreaming"),
        errors,
    );
    for key in [
        "requiresToolResultName",
        "requiresAssistantAfterToolResult",
        "requiresThinkingAsText",
        "requiresReasoningContentOnAssistantMessages",
        "supportsOpenAIGrammarTools",
        "sendSessionAffinityHeaders",
        "supportsAdditionalTools",
        "supportsToolSearch",
        "supportsCacheControlOnTools",
        "supportsTemperature",
        "forceAdaptiveThinking",
        "allowEmptySignature",
        "supportsStrictTools",
        "supportsToolReferences",
    ] {
        expect_optional_bool(&format!("{path}.{key}"), object.get(key), errors);
    }
    validate_openrouter_routing(
        &format!("{path}.openRouterRouting"),
        object.get("openRouterRouting"),
        errors,
    );
    validate_vercel_gateway_routing(
        &format!("{path}.vercelGatewayRouting"),
        object.get("vercelGatewayRouting"),
        errors,
    );
    validate_chat_template_record(
        &format!("{path}.chatTemplateKwargs"),
        object.get("chatTemplateKwargs"),
        errors,
    );
    validate_chat_template_record(
        &format!("{path}.chatTemplateArgs"),
        object.get("chatTemplateArgs"),
        errors,
    );
}

fn validate_thinking_level_map(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    for key in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
        match object.get(key) {
            None | Some(Value::Null) | Some(Value::String(_)) => {}
            Some(_) => errors.push(format!("  - {path}.{key}: Expected union value")),
        }
    }
}

fn validate_openrouter_routing(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    for key in [
        "allow_fallbacks",
        "require_parameters",
        "zdr",
        "enforce_distillable_text",
    ] {
        expect_optional_bool(&format!("{path}.{key}"), object.get(key), errors);
    }
    expect_optional_union(
        &format!("{path}.data_collection"),
        object.get("data_collection"),
        &["deny", "allow"],
        errors,
    );
    for key in ["order", "only", "ignore", "quantizations"] {
        validate_string_array(&format!("{path}.{key}"), object.get(key), errors);
    }
    validate_openrouter_sort(&format!("{path}.sort"), object.get("sort"), errors);
    validate_openrouter_max_price(
        &format!("{path}.max_price"),
        object.get("max_price"),
        errors,
    );
    validate_number_or_percentiles(
        &format!("{path}.preferred_min_throughput"),
        object.get("preferred_min_throughput"),
        errors,
    );
    validate_number_or_percentiles(
        &format!("{path}.preferred_max_latency"),
        object.get("preferred_max_latency"),
        errors,
    );
}

fn validate_vercel_gateway_routing(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    validate_string_array(&format!("{path}.only"), object.get("only"), errors);
    validate_string_array(&format!("{path}.order"), object.get("order"), errors);
}

fn validate_openrouter_sort(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) | Some(Value::String(_)) => {}
        Some(Value::Object(object)) => {
            match object.get("by") {
                None | Some(Value::Null) | Some(Value::String(_)) => {}
                Some(_) => errors.push(format!("  - {path}.by: Expected string")),
            }
            match object.get("partition") {
                None | Some(Value::Null) | Some(Value::String(_)) => {}
                Some(_) => errors.push(format!("  - {path}.partition: Expected union value")),
            }
        }
        Some(_) => errors.push(format!("  - {path}: Expected union value")),
    }
}

fn validate_openrouter_max_price(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    for key in ["prompt", "completion", "image", "audio", "request"] {
        match object.get(key) {
            None | Some(Value::Null) => {}
            Some(item) if item.is_number() || item.is_string() => {}
            Some(_) => errors.push(format!("  - {path}.{key}: Expected union value")),
        }
    }
}

fn validate_number_or_percentiles(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(item) if item.is_number() => {}
        Some(Value::Object(object)) => {
            for key in ["p50", "p75", "p90", "p99"] {
                expect_optional_number(&format!("{path}.{key}"), object.get(key), errors);
            }
        }
        Some(_) => errors.push(format!("  - {path}: Expected union value")),
    }
}

fn validate_string_array(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                if !item.is_string() {
                    errors.push(format!("  - {path}.{index}: Expected string"));
                }
            }
        }
        Some(_) => errors.push(format!("  - {path}: Expected array")),
    }
}

fn validate_chat_template_record(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(object) = value.as_object() else {
        errors.push(format!("  - {path}: Expected object"));
        return;
    };
    for (key, item) in object {
        match item {
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {}
            Value::Object(map) => match map.get("$var") {
                None => errors.push(format!("  - {path}.{key}.$var: Expected required property")),
                Some(Value::String(var))
                    if var == "thinking.enabled" || var == "thinking.effort" =>
                {
                    expect_optional_bool(
                        &format!("{path}.{key}.omitWhenOff"),
                        map.get("omitWhenOff"),
                        errors,
                    );
                }
                Some(_) => errors.push(format!("  - {path}.{key}.$var: Expected union value")),
            },
            _ => errors.push(format!("  - {path}.{key}: Expected union value")),
        }
    }
}

fn expect_optional_union(
    path: &str,
    value: Option<&Value>,
    allowed: &[&str],
    errors: &mut Vec<String>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) if allowed.contains(&text.as_str()) => {}
        Some(_) => errors.push(format!("  - {path}: Expected union value")),
    }
}

fn expect_optional_const(
    path: &str,
    value: Option<&Value>,
    expected: &str,
    errors: &mut Vec<String>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) if text == expected => {}
        Some(_) => errors.push(format!("  - {path}: Expected const value")),
    }
}

fn validate_string_record(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (key, item) in map {
                if !item.is_string() {
                    errors.push(format!("  - {path}.{key}: Expected string"));
                }
            }
        }
        Some(_) => errors.push(format!("  - {path}: Expected object")),
    }
}

fn expect_optional_min_string(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) if text.is_empty() => {
            errors.push(format!(
                "  - {path}: Expected string length greater or equal to 1"
            ));
        }
        Some(Value::String(_)) => {}
        Some(_) => errors.push(format!("  - {path}: Expected string")),
    }
}

fn expect_optional_bool(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Bool(_)) => {}
        Some(_) => errors.push(format!("  - {path}: Expected boolean")),
    }
}

fn expect_optional_number(path: &str, value: Option<&Value>, errors: &mut Vec<String>) {
    match value {
        None | Some(Value::Null) => {}
        Some(item) if item.is_number() => {}
        Some(_) => errors.push(format!("  - {path}: Expected number")),
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
        apply_config_auth(&mut auth, &config, "local", models.first(), &HashMap::new());
        assert_eq!(auth.api_key.as_deref(), Some("sk-test"));
        assert_eq!(auth.source, "configured API key");
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

    #[test]
    fn typebox_keyword_error_paths() {
        let dir = tempdir().unwrap();
        let write = |name: &str, raw: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, raw).unwrap();
            ModelConfig::load(&path)
        };

        let missing = write("missing.json", r#"{}"#);
        let missing_error = missing.error().unwrap();
        assert!(missing_error.contains("  - providers: Expected required property"));

        let array = write("array.json", r#"{"providers":[]}"#);
        assert!(array
            .error()
            .unwrap()
            .contains("  - providers: Expected object"));

        let required_id = write(
            "id.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","models":[{}]}}}"#,
        );
        assert!(required_id
            .error()
            .unwrap()
            .contains("  - providers.local.models.0.id: Expected required property"));

        let empty_name = write(
            "name.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","models":[{"id":"demo","name":""}]}}}"#,
        );
        assert!(empty_name.error().unwrap().contains(
            "  - providers.local.models.0.name: Expected string length greater or equal to 1"
        ));

        let oauth = write(
            "oauth.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","oauth":"github"}}}"#,
        );
        assert!(oauth
            .error()
            .unwrap()
            .contains("  - providers.local.oauth: Expected const value"));

        let cost = write(
            "cost.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","models":[{"id":"demo","cost":"free"}]}}}"#,
        );
        assert!(cost
            .error()
            .unwrap()
            .contains("  - providers.local.models.0.cost: Expected object"));

        let cost_field = write(
            "cost-field.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","models":[{"id":"demo","cost":{"input":"x","output":1,"cacheRead":0,"cacheWrite":0}}]}}}"#,
        );
        assert!(cost_field
            .error()
            .unwrap()
            .contains("  - providers.local.models.0.cost.input: Expected number"));

        let input = write(
            "input.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","api":"openai-completions","models":[{"id":"demo","input":["video"]}]}}}"#,
        );
        assert!(input
            .error()
            .unwrap()
            .contains("  - providers.local.models.0.input.0: Expected union value"));

        let collected = write(
            "multi.json",
            r#"{"providers":{"local":{"apiKey":"","oauth":"github","models":[{"id":""}]}}}"#,
        );
        let collected_error = collected.error().unwrap();
        assert!(collected_error
            .contains("  - providers.local.apiKey: Expected string length greater or equal to 1"));
        assert!(collected_error.contains("  - providers.local.oauth: Expected const value"));
        assert!(collected_error.contains(
            "  - providers.local.models.0.id: Expected string length greater or equal to 1"
        ));

        let compat = write(
            "compat.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","compat":{"thinkingFormat":"nope","cacheControlFormat":"openai"}}}}"#,
        );
        let compat_error = compat.error().unwrap();
        assert!(compat_error
            .contains("  - providers.local.compat.thinkingFormat: Expected union value"));
        assert!(compat_error
            .contains("  - providers.local.compat.cacheControlFormat: Expected const value"));

        let routing = write(
            "routing.json",
            r#"{"providers":{"local":{"baseUrl":"http://127.0.0.1:9","compat":{"openRouterRouting":{"allow_fallbacks":"yes","data_collection":"maybe","order":"anthropic","sort":1,"max_price":{"prompt":true},"preferred_min_throughput":{"p50":"x"}},"vercelGatewayRouting":{"only":1},"chatTemplateKwargs":{"thinking":{"$var":"thinking.foo"}},"requiresToolResultName":"no"}}}}"#,
        );
        let routing_error = routing.error().unwrap();
        assert!(routing_error.contains(
            "  - providers.local.compat.openRouterRouting.allow_fallbacks: Expected boolean"
        ));
        assert!(routing_error.contains(
            "  - providers.local.compat.openRouterRouting.data_collection: Expected union value"
        ));
        assert!(routing_error
            .contains("  - providers.local.compat.openRouterRouting.order: Expected array"));
        assert!(routing_error
            .contains("  - providers.local.compat.openRouterRouting.sort: Expected union value"));
        assert!(routing_error.contains(
            "  - providers.local.compat.openRouterRouting.max_price.prompt: Expected union value"
        ));
        assert!(routing_error.contains(
            "  - providers.local.compat.openRouterRouting.preferred_min_throughput.p50: Expected number"
        ));
        assert!(routing_error
            .contains("  - providers.local.compat.vercelGatewayRouting.only: Expected array"));
        assert!(routing_error.contains(
            "  - providers.local.compat.chatTemplateKwargs.thinking.$var: Expected union value"
        ));
        assert!(routing_error
            .contains("  - providers.local.compat.requiresToolResultName: Expected boolean"));
    }

    #[test]
    fn config_value_env_names_match_ts() {
        assert!(is_command_config_value("!echo key"));
        assert!(!is_command_config_value("$!echo"));
        assert_eq!(
            config_value_env_var_names("$FOO-${BAR}_$$BAZ"),
            vec!["FOO".to_string(), "BAR".to_string()]
        );
        assert_eq!(
            config_value_env_var_names("$FOO_${BAR}"),
            vec!["FOO_".to_string(), "BAR".to_string()]
        );
        assert!(config_value_env_var_names("literal-key").is_empty());
        assert!(config_value_env_var_names("!echo $FOO").is_empty());
        let mut env = HashMap::new();
        env.insert("FOO".into(), "left".into());
        env.insert("BAR".into(), "right".into());
        assert_eq!(
            resolve_config_value("$FOO-${BAR}", &env).as_deref(),
            Some("left-right")
        );
        assert_eq!(
            resolve_config_value("$!literal", &env).as_deref(),
            Some("!literal")
        );
        assert!(resolve_config_value("!echo should-not-run", &env).is_none());
    }
}
