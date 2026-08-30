//! Built-in llama.cpp provider matching `vendor/pi/packages/coding-agent/src/extensions/llama`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const LLAMA_PROVIDER_ID: &str = "llama.cpp";
pub const DEFAULT_LLAMA_SERVER_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LlamaModelStatus {
    pub value: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default, rename = "exit_code")]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub progress: Option<Value>,
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
    if let Some(home) = pi_session::home_dir() {
        paths.push(home.join(".cache").join("huggingface").join("token"));
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

#[derive(Debug, Clone, PartialEq)]
pub struct LlamaModelEvent {
    pub model: String,
    pub event: String,
    pub data: Option<Value>,
}

/// TS `LlamaClient.watch` frame parser: `data:` lines joined, `\\n\\n` boundaries.
pub fn parse_sse_events(body: &str) -> Vec<LlamaModelEvent> {
    let normalized = body.replace("\r\n", "\n");
    let mut events = Vec::new();
    for frame in normalized.split("\n\n") {
        let data = frame
            .lines()
            .filter(|line| line.starts_with("data:"))
            .map(|line| line[5..].trim_start())
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let Some(model) = value.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(event) = value.get("event").and_then(Value::as_str) else {
            continue;
        };
        events.push(LlamaModelEvent {
            model: model.to_string(),
            event: event.to_string(),
            data: value.get("data").cloned(),
        });
    }
    events
}

pub fn is_connection_error(error: &str) -> bool {
    let message = error.to_lowercase();
    message.contains("fetch failed")
        || message.contains("timeout")
        || message.contains("network")
        || message.contains("could not connect")
        || message.contains("connection refused")
}

pub fn connection_error_message(error: &str) -> String {
    if is_connection_error(error) {
        "Could not connect to the server.".into()
    } else {
        error.to_string()
    }
}

pub fn connection_retry_title(server_url: &str, error: &str) -> String {
    format!(
        "llama.cpp unavailable\n{server_url}\n\n{}",
        connection_error_message(error)
    )
}

pub fn progress_bar(ratio: f64) -> String {
    let clamped = ratio.clamp(0.0, 1.0);
    let filled = (clamped * 40.0).round() as usize;
    let filled = filled.min(40);
    format!(
        "{}{} {}%",
        "█".repeat(filled),
        "─".repeat(40 - filled),
        (clamped * 100.0).round() as i32
    )
}

/// Snapshot helper used by fixtures and tests. Live callers use [`watch_live`].
pub fn watch_events(server_url: &str) -> Result<Vec<LlamaModelEvent>, String> {
    let mut events = Vec::new();
    watch_live(server_url, |event| events.push(event), None)?;
    Ok(events)
}

/// TS `LlamaClient.watch`: persistent `GET /models/sse` with incremental frames.
pub fn watch_live(
    server_url: &str,
    mut on_event: impl FnMut(LlamaModelEvent),
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    if let Ok(raw) = std::env::var("PI_LLAMA_SSE_REPLY") {
        for event in parse_sse_events(&raw) {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Ok(());
            }
            on_event(event);
        }
        return Ok(());
    }
    if std::env::var("PI_LLAMA_DRY_RUN").is_ok() {
        return Ok(());
    }
    let url = format!("{}/models/sse", normalize_llama_server_url(server_url)?);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_millis(200))
        .timeout_write(Duration::from_secs(5))
        .build();
    let mut request = agent.get(&url);
    if let Ok(key) = std::env::var("LLAMA_API_KEY") {
        if !key.is_empty() {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
    }
    let response = request
        .call()
        .map_err(|err| format!("llama.cpp SSE returned HTTP {err}"))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("llama.cpp SSE returned HTTP {status}"));
    }
    let mut reader = response.into_reader();
    let mut buffer = String::new();
    let mut chunk = [0u8; 2048];
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(());
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.push_str(&String::from_utf8_lossy(&chunk[..n]).replace("\r\n", "\n"));
                while let Some(boundary) = buffer.find("\n\n") {
                    let frame = buffer[..boundary].to_string();
                    buffer = buffer[boundary + 2..].to_string();
                    for event in parse_sse_events(&format!("{frame}\n\n")) {
                        on_event(event);
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(err) => {
                if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                    return Ok(());
                }
                return Err(format!("llama.cpp SSE returned HTTP {err}"));
            }
        }
    }
    Ok(())
}

pub fn fixture_wait_mode() -> bool {
    std::env::var("PI_LLAMA_SSE_REPLY").is_ok()
        || std::env::var("PI_LLAMA_MODELS_REPLY").is_ok()
        || std::env::var("PI_LLAMA_ACTION_REPLY").is_ok()
        || std::env::var("PI_LLAMA_DRY_RUN").is_ok()
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

#[derive(Default)]
struct LoadWatchState {
    loaded: bool,
    error: Option<String>,
    progress: Option<LlamaProgress>,
}

#[derive(Default)]
struct DownloadWatchState {
    finished: bool,
    failure: Option<String>,
    saw_downloading: bool,
    progress: Option<LlamaProgress>,
}

pub fn unload_and_wait(server_url: &str, model: &str) -> Result<(), String> {
    unload_model(server_url, model)?;
    loop {
        let catalog = list_models(server_url)?;
        let entry = catalog.iter().find(|item| item.id == model);
        if entry.is_none() || entry.is_some_and(|item| item.status.value == "unloaded") {
            return Ok(());
        }
        if fixture_wait_mode() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn apply_load_event(model: &str, event: &LlamaModelEvent, state: &mut LoadWatchState) {
    if event.model != model {
        return;
    }
    if event.event != "model_status" && event.event != "status_change" {
        return;
    }
    if event
        .data
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("loaded")
    {
        state.loaded = true;
    }
    if event
        .data
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("unloaded")
    {
        state.error = Some("Model failed to load".into());
    }
    if let Some(progress) = event.data.as_ref().and_then(parse_load_progress) {
        state.progress = Some(progress);
    }
}

fn apply_download_event(model: &str, event: &LlamaModelEvent, state: &mut DownloadWatchState) {
    if event.model != model {
        return;
    }
    if event.event == "download_finished" {
        state.finished = true;
    }
    if event.event == "download_failed" {
        state.failure = Some(llama_error_message(event.data.as_ref(), "Download failed"));
    }
    if event.event == "download_progress" {
        state.saw_downloading = true;
        if let Some(progress) = event.data.as_ref().and_then(parse_download_progress) {
            state.progress = Some(progress);
        }
    }
}

pub fn load_and_wait(
    server_url: &str,
    model: &str,
    on_progress: impl FnMut(LlamaProgress),
) -> Result<LlamaModelInfo, String> {
    load_and_wait_with_cancel(server_url, model, on_progress, None)
}

pub fn load_and_wait_with_cancel(
    server_url: &str,
    model: &str,
    mut on_progress: impl FnMut(LlamaProgress),
    cancel: Option<&AtomicBool>,
) -> Result<LlamaModelInfo, String> {
    let state = Arc::new(Mutex::new(LoadWatchState::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let url = server_url.to_string();
    let watched = model.to_string();
    let state_watch = state.clone();
    let stop_watch = stop.clone();
    let handle = std::thread::spawn(move || {
        let _ = watch_live(
            &url,
            |event| {
                if let Ok(mut guard) = state_watch.lock() {
                    apply_load_event(&watched, &event, &mut guard);
                }
            },
            Some(&stop_watch),
        );
    });
    let result = (|| {
        load_model(server_url, model)?;
        if cancelled(cancel) {
            return Err("Cancelled".into());
        }
        on_progress(LlamaProgress {
            message: "Loading model".into(),
            ratio: None,
            detail: None,
        });
        loop {
            if cancelled(cancel) {
                return Err("Cancelled".into());
            }
            if let Ok(mut guard) = state.lock() {
                if let Some(progress) = guard.progress.take() {
                    on_progress(progress);
                }
            }
            let catalog = list_models(server_url)?;
            let event_loaded = state.lock().map(|guard| guard.loaded).unwrap_or(false);
            let event_error = state.lock().ok().and_then(|guard| guard.error.clone());
            if let Some(entry) = catalog.iter().find(|item| item.id == model).cloned() {
                if entry.status.value == "loaded" {
                    return Ok(entry);
                }
                if entry.status.failed {
                    return Err(match entry.status.exit_code {
                        Some(code) => format!("Model exited with code {code}"),
                        None => event_error.unwrap_or_else(|| "Model failed to load".into()),
                    });
                }
            } else if event_loaded {
                return Ok(LlamaModelInfo {
                    id: model.to_string(),
                    status: LlamaModelStatus {
                        value: "loaded".into(),
                        ..LlamaModelStatus::default()
                    },
                    architecture: None,
                    source: None,
                    meta: None,
                });
            }
            if fixture_wait_mode() {
                if let Some(error) = event_error {
                    return Err(error);
                }
                return Err("Model failed to load".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    })();
    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    result
}

pub fn download_and_wait(
    server_url: &str,
    model: &str,
    on_progress: impl FnMut(LlamaProgress),
) -> Result<Vec<LlamaModelInfo>, String> {
    download_and_wait_with_cancel(server_url, model, on_progress, None)
}

pub fn download_and_wait_with_cancel(
    server_url: &str,
    model: &str,
    mut on_progress: impl FnMut(LlamaProgress),
    cancel: Option<&AtomicBool>,
) -> Result<Vec<LlamaModelInfo>, String> {
    let state = Arc::new(Mutex::new(DownloadWatchState::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let url = server_url.to_string();
    let watched = model.to_string();
    let state_watch = state.clone();
    let stop_watch = stop.clone();
    let handle = std::thread::spawn(move || {
        let _ = watch_live(
            &url,
            |event| {
                if let Ok(mut guard) = state_watch.lock() {
                    apply_download_event(&watched, &event, &mut guard);
                }
            },
            Some(&stop_watch),
        );
    });
    let result = (|| {
        download_model(server_url, model)?;
        if cancelled(cancel) {
            return Err("Cancelled".into());
        }
        on_progress(LlamaProgress {
            message: "Downloading model".into(),
            ratio: None,
            detail: None,
        });
        let mut polls = 0;
        loop {
            if cancelled(cancel) {
                return Err("Cancelled".into());
            }
            let (failure, finished, saw_downloading, progress) = {
                let mut guard = state.lock().map_err(|err| err.to_string())?;
                (
                    guard.failure.clone(),
                    guard.finished,
                    guard.saw_downloading,
                    guard.progress.take(),
                )
            };
            if let Some(progress) = progress {
                on_progress(progress);
            }
            if let Some(error) = failure {
                return Err(error);
            }
            let models = list_models(server_url)?;
            polls += 1;
            if let Some(entry) = models.iter().find(|item| item.id == model) {
                if entry.status.value == "downloading" {
                    if let Ok(mut guard) = state.lock() {
                        guard.saw_downloading = true;
                    }
                    if let Some(progress) = entry
                        .status
                        .progress
                        .as_ref()
                        .and_then(parse_download_progress)
                    {
                        on_progress(progress);
                    }
                } else if finished || saw_downloading || polls >= 2 {
                    return list_models_opts(server_url, true);
                }
            } else if finished || polls >= 2 {
                return list_models_opts(server_url, true);
            }
            if fixture_wait_mode() {
                return Ok(models);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();
    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    result
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
    list_models_opts(server_url, false)
}

pub fn list_models_opts(server_url: &str, reload: bool) -> Result<Vec<LlamaModelInfo>, String> {
    if let Ok(err) = std::env::var("PI_LLAMA_MODELS_ERROR") {
        if !err.is_empty() {
            return Err(err);
        }
    }
    if let Ok(raw) = std::env::var("PI_LLAMA_MODELS_REPLY") {
        let payload: Value =
            serde_json::from_str(&raw).map_err(|err| format!("PI_LLAMA_MODELS_REPLY: {err}"))?;
        return parse_models_catalog(&payload);
    }
    if std::env::var("PI_LLAMA_DRY_RUN").is_ok() {
        return Ok(Vec::new());
    }
    let path = if reload {
        "/models?reload=1"
    } else {
        "/models"
    };
    let url = format!("{}{path}", normalize_llama_server_url(server_url)?);
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

const DEFAULT_HUGGING_FACE_URL: &str = "https://huggingface.co";

#[derive(Debug, Clone, PartialEq)]
pub struct HuggingFaceModel {
    pub id: String,
    pub downloads: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HuggingFaceQuantization {
    pub name: String,
    pub size: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HuggingFaceGated {
    False,
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HuggingFaceModelDetails {
    pub id: String,
    pub gated: HuggingFaceGated,
    pub quantizations: Vec<HuggingFaceQuantization>,
}

fn hugging_face_base_url() -> String {
    std::env::var("HF_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_HUGGING_FACE_URL.into())
        .trim_end_matches('/')
        .to_string()
}

fn hugging_face_error(payload: Option<&Value>, fallback: &str) -> String {
    payload
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn hugging_face_request(path: &str) -> Result<Value, String> {
    if let Ok(raw) = std::env::var("PI_HF_SEARCH_REPLY") {
        if path.starts_with("/api/models?") {
            return serde_json::from_str(&raw).map_err(|err| format!("PI_HF_SEARCH_REPLY: {err}"));
        }
    }
    if let Ok(raw) = std::env::var("PI_HF_DETAILS_REPLY") {
        if path.contains("blobs=true")
            || (path.starts_with("/api/models/") && !path.starts_with("/api/models?"))
        {
            return serde_json::from_str(&raw).map_err(|err| format!("PI_HF_DETAILS_REPLY: {err}"));
        }
    }
    if std::env::var("PI_HF_DRY_RUN").is_ok() || cfg!(test) {
        if path.starts_with("/api/models?") {
            return Ok(serde_json::json!([]));
        }
        return Ok(serde_json::json!({ "id": "org/model", "gated": false, "siblings": [] }));
    }
    let url = format!("{}{path}", hugging_face_base_url());
    let mut request = ureq::get(&url);
    if let Some(token) = find_hugging_face_token() {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = request
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|err| format!("Could not connect to Hugging Face. {err}"))?;
    let status = response.status();
    let retry_after = response.header("retry-after").map(str::to_string);
    let ratelimit = response.header("ratelimit").map(str::to_string);
    let payload = response.into_json::<Value>().ok();
    if status == 429 {
        let delay = retry_after
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .or_else(|| parse_rate_limit_delay(ratelimit.as_deref().unwrap_or("")));
        return Err(match delay {
            Some(seconds) => format!("Hugging Face rate limit reached; retry in {seconds}s"),
            None => "Hugging Face rate limit reached".into(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(hugging_face_error(
            payload.as_ref(),
            &format!("Hugging Face returned HTTP {status}"),
        ));
    }
    Ok(payload.unwrap_or(Value::Null))
}

pub fn parse_rate_limit_delay(value: &str) -> Option<u64> {
    let marker = "t=";
    let start = value.find(marker)?;
    let rest = &value[start + marker.len()..];
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub fn strip_gguf_shard_suffix(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    if bytes.len() < 15 {
        return stem;
    }
    let suffix = &bytes[bytes.len() - 15..];
    if suffix[0] == b'-'
        && suffix[6] == b'-'
        && suffix[7] == b'o'
        && suffix[8] == b'f'
        && suffix[9] == b'-'
        && suffix[1..6].iter().all(u8::is_ascii_digit)
        && suffix[10..].iter().all(u8::is_ascii_digit)
    {
        return &stem[..stem.len() - 15];
    }
    stem
}

pub fn quantization_from_filename(filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    if !lower.ends_with(".gguf") || lower.starts_with("mmproj") {
        return None;
    }
    let stem = strip_gguf_shard_suffix(&filename[..filename.len() - 5]);
    let upper = stem.to_ascii_uppercase();
    let mut best = None;
    for (index, _) in upper.char_indices() {
        if index > 0 {
            let Some(prev) = upper[..index].chars().next_back() else {
                continue;
            };
            if prev != '-' && prev != '_' && prev != '.' {
                continue;
            }
        }
        if let Some(name) = match_quantization_suffix(&upper[index..]) {
            if index + name.len() == upper.len() {
                best = Some(name);
            }
        }
    }
    best
}

fn match_quantization_suffix(value: &str) -> Option<String> {
    let (prefix, rest) = value
        .strip_prefix("UD-")
        .map(|rest| ("UD-", rest))
        .unwrap_or(("", value));
    if matches!(rest, "BF16" | "F16" | "F32") {
        return Some(format!("{prefix}{rest}"));
    }
    if let Some(tail) = rest.strip_prefix("MXFP") {
        if tail.starts_with(|ch: char| ch.is_ascii_digit())
            && tail
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Some(format!("{prefix}{rest}"));
        }
        return None;
    }
    let body = if let Some(tail) = rest.strip_prefix("IQ") {
        tail
    } else if let Some(tail) = rest.strip_prefix('Q') {
        tail
    } else {
        return None;
    };
    let mut chars = body.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if !body.contains('_') {
        return None;
    }
    if !body
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(format!("{prefix}{rest}"))
}

pub fn parse_hugging_face_search(payload: &Value) -> Result<Vec<HuggingFaceModel>, String> {
    let items = payload
        .as_array()
        .ok_or_else(|| "Hugging Face returned invalid search results".to_string())?;
    Ok(items
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_str()?.to_string();
            let downloads = value.get("downloads").and_then(Value::as_u64).unwrap_or(0);
            Some(HuggingFaceModel { id, downloads })
        })
        .collect())
}

pub fn parse_hugging_face_details(
    payload: &Value,
    fallback_id: &str,
) -> Result<HuggingFaceModelDetails, String> {
    if !payload.is_object() {
        return Err("Hugging Face returned invalid model details".into());
    }
    let gated = match payload.get("gated").and_then(Value::as_str) {
        Some("auto") => HuggingFaceGated::Auto,
        Some("manual") => HuggingFaceGated::Manual,
        _ => HuggingFaceGated::False,
    };
    let mut sizes: HashMap<String, (f64, bool)> = HashMap::new();
    if let Some(siblings) = payload.get("siblings").and_then(Value::as_array) {
        for file in siblings {
            let Some(rfilename) = file.get("rfilename").and_then(Value::as_str) else {
                continue;
            };
            let filename = rfilename.rsplit('/').next().unwrap_or(rfilename);
            let Some(name) = quantization_from_filename(filename) else {
                continue;
            };
            let entry = sizes.entry(name).or_insert((0.0, true));
            if let Some(size) = file.get("size").and_then(Value::as_f64) {
                entry.0 += size;
            } else {
                entry.1 = false;
            }
        }
    }
    let mut quantizations: Vec<HuggingFaceQuantization> = sizes
        .into_iter()
        .map(|(name, (total, complete))| HuggingFaceQuantization {
            name,
            size: complete.then_some(total),
        })
        .collect();
    quantizations.sort_by(|left, right| {
        if left.name == "Q4_K_M" {
            return std::cmp::Ordering::Less;
        }
        if right.name == "Q4_K_M" {
            return std::cmp::Ordering::Greater;
        }
        let left_size = left.size.unwrap_or(f64::MAX);
        let right_size = right.size.unwrap_or(f64::MAX);
        left_size
            .partial_cmp(&right_size)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(HuggingFaceModelDetails {
        id: payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(fallback_id)
            .to_string(),
        gated,
        quantizations,
    })
}

fn encode_uri_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn search_hugging_face(query: &str) -> Result<Vec<HuggingFaceModel>, String> {
    let path = format!(
        "/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=20",
        encode_uri_component(query)
    );
    let payload = hugging_face_request(&path)?;
    parse_hugging_face_search(&payload)
}

pub fn hugging_face_details(id: &str) -> Result<HuggingFaceModelDetails, String> {
    let encoded = id
        .split('/')
        .map(encode_uri_component)
        .collect::<Vec<_>>()
        .join("/");
    let payload = hugging_face_request(&format!("/api/models/{encoded}?blobs=true"))?;
    parse_hugging_face_details(&payload, id)
}

pub fn quantization_option_label(entry: &HuggingFaceQuantization) -> String {
    let mut details = Vec::new();
    if let Some(size) = entry.size {
        details.push(format_bytes(size));
    }
    if entry.name == "Q4_K_M" {
        details.push("recommended".into());
    }
    if details.is_empty() {
        entry.name.clone()
    } else {
        format!("{} · {}", entry.name, details.join(" · "))
    }
}

pub fn hugging_face_search_label(model: &HuggingFaceModel) -> String {
    if model.downloads == 0 {
        model.id.clone()
    } else {
        format!(
            "{} · {} downloads",
            model.id,
            compact_count(model.downloads)
        )
    }
}

pub fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        let scaled = value as f64 / 1_000_000.0;
        if value >= 10_000_000 {
            format!("{scaled:.0}M")
        } else {
            format!("{scaled:.1}M")
        }
    } else if value >= 1_000 {
        let scaled = value as f64 / 1_000.0;
        if value >= 100_000 {
            format!("{scaled:.0}k")
        } else {
            format!("{scaled:.1}k")
        }
    } else {
        value.to_string()
    }
}

pub fn loaded_models<'a>(
    catalog: &'a [LlamaModelInfo],
    except: Option<&str>,
) -> Vec<&'a LlamaModelInfo> {
    catalog
        .iter()
        .filter(|model| model_is_loaded(model) && except.is_none_or(|id| model.id != id))
        .collect()
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
                    ..LlamaModelStatus::default()
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
                    ..LlamaModelStatus::default()
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
                    ..LlamaModelStatus::default()
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
                    ..LlamaModelStatus::default()
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

    #[test]
    fn sse_watch_and_wait_loops_match_ts() {
        let events = parse_sse_events(
            "data: {\"model\":\"local\",\"event\":\"model_status\",\"data\":{\"status\":\"loaded\",\"progress\":{\"current\":\"loading_weights\",\"stages\":[\"prepare\",\"loading_weights\"],\"value\":1}}}\n\n\
             data: not-json\n\n\
             data: {\"event\":\"ignored\"}\n\n",
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model, "local");
        assert_eq!(events[0].event, "model_status");

        std::env::set_var("PI_LLAMA_ACTION_REPLY", r#"{"ok":true}"#);
        std::env::set_var(
            "PI_LLAMA_MODELS_REPLY",
            r#"{"data":[{"id":"local","status":{"value":"loaded"}}]}"#,
        );
        std::env::set_var(
            "PI_LLAMA_SSE_REPLY",
            "data: {\"model\":\"local\",\"event\":\"model_status\",\"data\":{\"status\":\"loaded\"}}\n\n",
        );
        let loaded = load_and_wait("http://127.0.0.1:8080", "local", |_| {}).unwrap();
        assert_eq!(loaded.status.value, "loaded");
        unload_and_wait("http://127.0.0.1:8080", "gone").unwrap();

        std::env::set_var(
            "PI_LLAMA_SSE_REPLY",
            "data: {\"model\":\"org/model\",\"event\":\"download_finished\"}\n\n",
        );
        let downloaded = download_and_wait("http://127.0.0.1:8080", "org/model", |_| {}).unwrap();
        assert_eq!(downloaded[0].id, "local");

        std::env::set_var(
            "PI_LLAMA_MODELS_REPLY",
            r#"{"data":[{"id":"broken","status":{"value":"loading","failed":true,"exit_code":7}}]}"#,
        );
        std::env::remove_var("PI_LLAMA_SSE_REPLY");
        std::env::set_var("PI_LLAMA_SSE_REPLY", "");
        let err = load_and_wait("http://127.0.0.1:8080", "broken", |_| {}).unwrap_err();
        assert!(err.contains("exited with code 7"), "{err}");

        std::env::remove_var("PI_LLAMA_ACTION_REPLY");
        std::env::remove_var("PI_LLAMA_MODELS_REPLY");
        std::env::remove_var("PI_LLAMA_SSE_REPLY");
    }

    #[test]
    fn connection_retry_and_progress_bar_match_ts() {
        assert!(is_connection_error("TypeError fetch failed"));
        assert!(is_connection_error("TimeoutError: timeout"));
        assert_eq!(
            connection_error_message("network down"),
            "Could not connect to the server."
        );
        assert_eq!(
            connection_error_message("Model failed to load"),
            "Model failed to load"
        );
        assert_eq!(
            connection_retry_title("http://127.0.0.1:8080", "fetch failed"),
            "llama.cpp unavailable\nhttp://127.0.0.1:8080\n\nCould not connect to the server."
        );
        assert!(progress_bar(0.5).contains("50%"));
        assert_eq!(
            progress_bar(0.5).chars().filter(|ch| *ch == '█').count(),
            20
        );
        std::env::set_var("PI_LLAMA_MODELS_ERROR", "fetch failed");
        let err = list_models("http://127.0.0.1:8080").unwrap_err();
        std::env::remove_var("PI_LLAMA_MODELS_ERROR");
        assert_eq!(
            connection_error_message(&err),
            "Could not connect to the server."
        );
    }

    #[test]
    fn live_sse_watch_streams_frames_incrementally() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "data: {\"model\":\"local\",\"event\":\"model_status\",\"data\":{\"status\":\"loaded\",\"progress\":{\"current\":\"text_model\",\"stages\":[\"text_model\"],\"value\":1}}}\n\n";
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            }
        });
        std::env::remove_var("PI_LLAMA_SSE_REPLY");
        std::env::remove_var("PI_LLAMA_DRY_RUN");
        let url = format!("http://127.0.0.1:{}", addr.port());
        let mut got = Vec::new();
        watch_live(&url, |event| got.push(event), None).unwrap();
        assert_eq!(got[0].model, "local");
        assert_eq!(got[0].event, "model_status");
        let progress = parse_load_progress(got[0].data.as_ref().unwrap()).unwrap();
        assert_eq!(progress.message, "Loading text model");
    }

    #[test]
    fn hugging_face_search_details_and_quant_match_ts() {
        assert_eq!(
            quantization_from_filename("model-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(
            quantization_from_filename("weights.BF16.gguf").as_deref(),
            Some("BF16")
        );
        assert_eq!(
            quantization_from_filename("shard-Q5_K_M-00001-of-00002.gguf").as_deref(),
            Some("Q5_K_M")
        );
        assert_eq!(quantization_from_filename("mmproj-Q4_K_M.gguf"), None);
        assert_eq!(parse_rate_limit_delay("x;t=12"), Some(12));
        assert_eq!(compact_count(1500), "1.5k");
        assert_eq!(compact_count(12_000_000), "12M");

        std::env::set_var(
            "PI_HF_SEARCH_REPLY",
            r#"[{"id":"org/model","downloads":1500},{"nope":true}]"#,
        );
        let results = search_hugging_face("model").unwrap();
        std::env::remove_var("PI_HF_SEARCH_REPLY");
        assert_eq!(results[0].id, "org/model");
        assert_eq!(results[0].downloads, 1500);
        assert_eq!(
            hugging_face_search_label(&results[0]),
            "org/model · 1.5k downloads"
        );

        std::env::set_var(
            "PI_HF_DETAILS_REPLY",
            r#"{"id":"org/model","gated":"manual","siblings":[
                {"rfilename":"model-Q4_K_M.gguf","size":1024},
                {"rfilename":"model-Q5_K_M.gguf","size":2048},
                {"rfilename":"mmproj-F16.gguf","size":10}
            ]}"#,
        );
        let details = hugging_face_details("org/model").unwrap();
        std::env::remove_var("PI_HF_DETAILS_REPLY");
        assert_eq!(details.gated, HuggingFaceGated::Manual);
        assert_eq!(details.quantizations[0].name, "Q4_K_M");
        assert!(quantization_option_label(&details.quantizations[0]).contains("recommended"));
        assert_eq!(
            parse_hugging_face_search(&serde_json::json!({})).unwrap_err(),
            "Hugging Face returned invalid search results"
        );
    }
}
