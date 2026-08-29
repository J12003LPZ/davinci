//! Built-in llama.cpp provider matching `vendor/pi/packages/coding-agent/src/extensions/llama`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const LLAMA_PROVIDER_ID: &str = "llama.cpp";
pub const DEFAULT_LLAMA_SERVER_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LlamaModelStatus {
    pub value: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlamaModelInfo {
    pub id: String,
    #[serde(default)]
    pub status: LlamaModelStatus,
    #[serde(default)]
    pub architecture: Option<Value>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlamaPiModel {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    pub base_url: String,
    pub input: Vec<String>,
    pub context_window: u64,
    pub max_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlamaProgress {
    pub message: String,
    pub ratio: Option<f64>,
    pub detail: Option<String>,
}

pub fn normalize_llama_server_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let parsed = url::Url::parse(trimmed).map_err(|err| err.to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Server URL must use http or https".into());
    }
    let mut parsed = parsed;
    parsed.set_fragment(None);
    parsed.set_query(None);
    let mut path = parsed.path().trim_end_matches('/').to_string();
    if let Some(stripped) = path.strip_suffix("/v1") {
        path = stripped.to_string();
    }
    if path.is_empty() {
        path = "/".into();
    }
    parsed.set_path(&path);
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

pub fn llama_inference_url(server_url: &str) -> Result<String, String> {
    Ok(format!("{}/v1", normalize_llama_server_url(server_url)?))
}

pub fn format_bytes(bytes: f64) -> String {
    if bytes < 1024.0 {
        return format!("{} B", bytes as u64);
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes / 1024.0;
    let mut unit = units[0];
    for next in units.iter().skip(1) {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    if value >= 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.2} {unit}")
    }
}

pub fn parse_hugging_face_model(value: &str) -> (String, Option<String>) {
    let slash = value.find('/');
    let search_from = slash.map(|index| index + 1).unwrap_or(0);
    if let Some(rel) = value[search_from..].find(':') {
        let colon = search_from + rel;
        (
            value[..colon].to_string(),
            Some(value[colon + 1..].to_string()),
        )
    } else {
        (value.to_string(), None)
    }
}

pub fn find_hugging_face_token() -> Option<String> {
    if let Ok(token) = std::env::var("HF_TOKEN") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("HF_TOKEN_PATH") {
        paths.push(std::path::PathBuf::from(path));
    }
    if let Ok(home) = std::env::var("HF_HOME") {
        paths.push(std::path::PathBuf::from(home).join("token"));
    }
    if let Ok(cache) = std::env::var("XDG_CACHE_HOME") {
        paths.push(
            std::path::PathBuf::from(cache)
                .join("huggingface")
                .join("token"),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            std::path::PathBuf::from(home)
                .join(".cache")
                .join("huggingface")
                .join("token"),
        );
    }
    for path in paths {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn is_model_info(value: &Value) -> bool {
    value.get("id").and_then(Value::as_str).is_some()
        && value
            .get("status")
            .and_then(|status| status.get("value"))
            .and_then(Value::as_str)
            .is_some()
}

pub fn parse_models_catalog(payload: &Value) -> Result<Vec<LlamaModelInfo>, String> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "llama.cpp returned an invalid model catalog".to_string())?;
    if !data.iter().all(is_model_info) {
        return Err("Server is not running in llama.cpp router mode".into());
    }
    data.iter()
        .map(|value| {
            serde_json::from_value::<LlamaModelInfo>(value.clone())
                .map_err(|err| format!("llama.cpp returned an invalid model catalog: {err}"))
        })
        .collect()
}

pub fn model_is_selectable(model: &LlamaModelInfo, router_autoload: bool) -> bool {
    match model.status.value.as_str() {
        "loaded" | "sleeping" => true,
        "unloaded" => {
            router_autoload && !model.status.failed && model.source.as_deref() == Some("preset")
        }
        _ => false,
    }
}

pub fn model_is_loaded(model: &LlamaModelInfo) -> bool {
    matches!(model.status.value.as_str(), "loaded" | "sleeping")
}

pub fn to_pi_model(model: &LlamaModelInfo, server_url: &str) -> Result<LlamaPiModel, String> {
    let context_window = model
        .meta
        .as_ref()
        .and_then(|meta| meta.get("n_ctx").or_else(|| meta.get("n_ctx_train")))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(128_000);
    let image = model
        .architecture
        .as_ref()
        .and_then(|value| value.get("input_modalities"))
        .and_then(Value::as_array)
        .is_some_and(|mods| mods.iter().any(|item| item.as_str() == Some("image")));
    Ok(LlamaPiModel {
        id: model.id.clone(),
        name: model.id.clone(),
        api: "openai-completions".into(),
        provider: LLAMA_PROVIDER_ID.into(),
        base_url: llama_inference_url(server_url)?,
        input: if image {
            vec!["text".into(), "image".into()]
        } else {
            vec!["text".into()]
        },
        context_window,
        max_tokens: context_window,
    })
}

pub fn selectable_models(
    catalog: &[LlamaModelInfo],
    server_url: &str,
    router_autoload: bool,
) -> Result<Vec<LlamaPiModel>, String> {
    catalog
        .iter()
        .filter(|model| model_is_selectable(model, router_autoload))
        .map(|model| to_pi_model(model, server_url))
        .collect()
}

pub fn parse_load_progress(data: &Value) -> Option<LlamaProgress> {
    let progress = data.get("progress")?;
    let stage = progress
        .get("current")
        .and_then(Value::as_str)
        .or_else(|| progress.get("stage").and_then(Value::as_str));
    let stages = progress
        .get("stages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let stage_ratio = progress
        .get("value")
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 1.0));
    let mut ratio = stage_ratio;
    if let Some(stage) = stage {
        if let Some(index) = stages.iter().position(|item| item == stage) {
            ratio = Some((index as f64 + stage_ratio.unwrap_or(0.0)) / stages.len() as f64);
        }
    }
    Some(LlamaProgress {
        message: stage
            .map(|value| format!("Loading {}", value.replace('_', " ")))
            .unwrap_or_else(|| "Loading model".into()),
        ratio,
        detail: None,
    })
}

pub fn parse_download_progress(data: &Value) -> Option<LlamaProgress> {
    let files = data.get("progress").unwrap_or(data);
    let map = files.as_object()?;
    let mut done = 0.0;
    let mut total = 0.0;
    for value in map.values() {
        let Some(entry_done) = value.get("done").and_then(Value::as_f64) else {
            continue;
        };
        let Some(entry_total) = value.get("total").and_then(Value::as_f64) else {
            continue;
        };
        done += entry_done;
        total += entry_total;
    }
    if total <= 0.0 {
        return None;
    }
    Some(LlamaProgress {
        message: "Downloading model".into(),
        ratio: Some(done / total),
        detail: Some(format!("{} / {}", format_bytes(done), format_bytes(total))),
    })
}

pub fn catalog_option_label(model: &LlamaModelInfo) -> String {
    format!("{} [{}]", model.id, model.status.value)
}

fn llama_error_message(payload: Option<&Value>, fallback: &str) -> String {
    payload
        .and_then(|value| value.get("error"))
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .filter(|message| !message.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn llama_request(
    server_url: &str,
    path: &str,
    method: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    if let Ok(raw) = std::env::var("PI_LLAMA_ACTION_REPLY") {
        if raw.is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&raw).or(Ok(Value::String(raw)));
    }
    if std::env::var("PI_LLAMA_DRY_RUN").is_ok() {
        return Ok(serde_json::json!({ "ok": true, "path": path, "method": method }));
    }
    let url = format!("{}{path}", normalize_llama_server_url(server_url)?);
    let mut request = match method {
        "POST" => ureq::post(&url),
        _ => ureq::get(&url),
    };
    if let Ok(key) = std::env::var("LLAMA_API_KEY") {
        if !key.is_empty() {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
    }
    let response = if let Some(body) = body {
        request
            .send_json(body)
            .map_err(|err| format!("Could not connect to the server. {err}"))?
    } else {
        request
            .call()
            .map_err(|err| format!("Could not connect to the server. {err}"))?
    };
    let status = response.status();
    let payload = response.into_json::<Value>().ok();
    if !(200..300).contains(&status) {
        return Err(llama_error_message(
            payload.as_ref(),
            &format!("llama.cpp returned HTTP {status}"),
        ));
    }
    Ok(payload.unwrap_or(Value::Null))
}

pub fn load_model(server_url: &str, model: &str) -> Result<(), String> {
    llama_request(
        server_url,
        "/models/load",
        "POST",
        Some(serde_json::json!({ "model": model })),
    )
    .map(|_| ())
}

pub fn unload_model(server_url: &str, model: &str) -> Result<(), String> {
    llama_request(
        server_url,
        "/models/unload",
        "POST",
        Some(serde_json::json!({ "model": model })),
    )
    .map(|_| ())
}

pub fn download_model(server_url: &str, model: &str) -> Result<(), String> {
    llama_request(
        server_url,
        "/models",
        "POST",
        Some(serde_json::json!({ "model": model })),
    )
    .map(|_| ())
}

pub fn list_models(server_url: &str) -> Result<Vec<LlamaModelInfo>, String> {
    if let Ok(raw) = std::env::var("PI_LLAMA_MODELS_REPLY") {
        let payload: Value =
            serde_json::from_str(&raw).map_err(|err| format!("PI_LLAMA_MODELS_REPLY: {err}"))?;
        return parse_models_catalog(&payload);
    }
    if std::env::var("PI_LLAMA_DRY_RUN").is_ok() {
        return Ok(Vec::new());
    }
    let url = format!("{}/models", normalize_llama_server_url(server_url)?);
    let response = ureq::get(&url)
        .call()
        .map_err(|err| format!("Could not connect to the server. {err}"))?;
    let payload: Value = response
        .into_json()
        .map_err(|err| format!("llama.cpp returned an invalid model catalog: {err}"))?;
    parse_models_catalog(&payload)
}

pub fn router_autoload(server_url: &str, catalog: &[LlamaModelInfo]) -> bool {
    if !catalog
        .iter()
        .any(|model| model.status.value == "unloaded" && model.source.as_deref() == Some("preset"))
    {
        return false;
    }
    if let Ok(raw) = std::env::var("PI_LLAMA_PROPS_REPLY") {
        return serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.get("models_autoload").and_then(Value::as_bool))
            .unwrap_or(false);
    }
    if std::env::var("PI_LLAMA_DRY_RUN").is_ok() {
        return false;
    }
    let Ok(url) = normalize_llama_server_url(server_url) else {
        return false;
    };
    ureq::get(&format!("{url}/props"))
        .call()
        .ok()
        .and_then(|response| response.into_json::<Value>().ok())
        .and_then(|value| value.get("models_autoload").and_then(Value::as_bool))
        .unwrap_or(false)
}

pub fn resolve_server_url(env: &HashMap<String, String>, fallback: Option<&str>) -> String {
    if let Some(url) = env.get("LLAMA_BASE_URL") {
        return url.clone();
    }
    if let Some(url) = fallback {
        return url.to_string();
    }
    std::env::var("LLAMA_BASE_URL").unwrap_or_else(|_| DEFAULT_LLAMA_SERVER_URL.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_management_and_inference_urls() {
        assert_eq!(
            normalize_llama_server_url("http://127.0.0.1:8080/v1/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_llama_server_url("https://example.com/prefix/v1").unwrap(),
            "https://example.com/prefix"
        );
        assert!(normalize_llama_server_url("file:///tmp/llama")
            .unwrap_err()
            .contains("http or https"));
        assert_eq!(
            llama_inference_url("http://localhost:8080").unwrap(),
            "http://localhost:8080/v1"
        );
    }

    #[test]
    fn exposes_loaded_and_sleeping_models_with_router_metadata() {
        let catalog = vec![
            LlamaModelInfo {
                id: "loaded".into(),
                status: LlamaModelStatus {
                    value: "loaded".into(),
                    args: vec!["llama-server".into(), "--n-gpu-layers".into(), "999".into()],
                    failed: false,
                },
                architecture: Some(serde_json::json!({"input_modalities":["text","image"]})),
                source: None,
                meta: Some(serde_json::json!({"n_ctx":65536,"n_ctx_train":131072})),
            },
            LlamaModelInfo {
                id: "sleeping".into(),
                status: LlamaModelStatus {
                    value: "sleeping".into(),
                    args: Vec::new(),
                    failed: false,
                },
                architecture: None,
                source: None,
                meta: None,
            },
            LlamaModelInfo {
                id: "unloaded".into(),
                status: LlamaModelStatus {
                    value: "unloaded".into(),
                    args: Vec::new(),
                    failed: false,
                },
                architecture: None,
                source: None,
                meta: None,
            },
            LlamaModelInfo {
                id: "loading".into(),
                status: LlamaModelStatus {
                    value: "loading".into(),
                    args: Vec::new(),
                    failed: false,
                },
                architecture: None,
                source: None,
                meta: None,
            },
        ];
        let models = selectable_models(&catalog, "http://localhost:8080", false).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "loaded");
        assert_eq!(models[0].base_url, "http://localhost:8080/v1");
        assert_eq!(models[0].context_window, 65536);
        assert_eq!(models[0].max_tokens, 65536);
        assert_eq!(models[0].input, ["text", "image"]);
        assert_eq!(models[1].id, "sleeping");
    }

    #[test]
    fn parses_progress_and_huggingface_ids() {
        let load = parse_load_progress(&serde_json::json!({
            "progress": {
                "current": "loading_weights",
                "stages": ["prepare", "loading_weights", "done"],
                "value": 0.5
            }
        }))
        .unwrap();
        assert_eq!(load.message, "Loading loading weights");
        assert!((load.ratio.unwrap() - ((1.0 + 0.5) / 3.0)).abs() < 1e-9);

        let download = parse_download_progress(&serde_json::json!({
            "file.bin": {"done": 512, "total": 1024}
        }))
        .unwrap();
        assert_eq!(download.message, "Downloading model");
        assert_eq!(download.ratio, Some(0.5));

        assert_eq!(
            parse_hugging_face_model("org/model:Q4_K_M"),
            ("org/model".into(), Some("Q4_K_M".into()))
        );
        assert_eq!(
            parse_hugging_face_model("org/model"),
            ("org/model".into(), None)
        );
        std::env::set_var("HF_TOKEN", "hf-fixture");
        assert_eq!(find_hugging_face_token().as_deref(), Some("hf-fixture"));
        std::env::remove_var("HF_TOKEN");
    }

    #[test]
    fn lists_models_from_fixture_reply() {
        std::env::set_var(
            "PI_LLAMA_MODELS_REPLY",
            r#"{"data":[{"id":"local","status":{"value":"loaded"},"meta":{"n_ctx":32768}}]}"#,
        );
        let models = list_models("http://127.0.0.1:8080").unwrap();
        std::env::remove_var("PI_LLAMA_MODELS_REPLY");
        assert_eq!(models[0].id, "local");
        assert_eq!(catalog_option_label(&models[0]), "local [loaded]");
    }

    #[test]
    fn load_unload_download_use_action_fixture() {
        std::env::set_var("PI_LLAMA_ACTION_REPLY", r#"{"ok":true}"#);
        load_model("http://127.0.0.1:8080", "local").unwrap();
        unload_model("http://127.0.0.1:8080", "local").unwrap();
        download_model("http://127.0.0.1:8080", "org/model:Q4_K_M").unwrap();
        std::env::remove_var("PI_LLAMA_ACTION_REPLY");
        std::env::set_var("PI_LLAMA_DRY_RUN", "1");
        load_model("http://127.0.0.1:8080", "local").unwrap();
        std::env::remove_var("PI_LLAMA_DRY_RUN");
    }
}
