//! Codex model discovery for the `openai-codex` provider. No TypeScript counterpart.
//!
//! Codex CLI does not ship a fixed model list. It asks the ChatGPT backend
//! (`GET {base}/codex/models?client_version=…`) and caches the reply in
//! `$CODEX_HOME/models_cache.json`. The built-in catalog and the pi.dev
//! overlay lag behind that list, so a model Codex offers could be missing here.
//!
//! This module keeps the two in step:
//! - [`refresh_codex_models`] fetches the live list with the stored ChatGPT
//!   login and saves it to `<agent dir>/codex-models.json`.
//! - [`overlay_codex_models`] adds every listed Codex model that the catalog
//!   lacks, from the newer of that file and Codex CLI's own cache. It reads
//!   files only, so print mode and RPC get the models with no network call.
//!
//! Existing catalog records always win: they carry reviewed prices and limits.
//! A discovered model copies transport fields (`api`, `baseUrl`, `compat`,
//! `maxTokens`) from the closest existing Codex record, takes its context
//! window, input types and reasoning levels from Codex, and has zero cost
//! because the backend publishes no prices.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Model, ModelCost};

pub const CODEX_PROVIDER: &str = "openai-codex";
pub const CODEX_MODELS_FILE: &str = "codex-models.json";
/// Same cadence as the pi.dev catalog refresh.
pub const CODEX_MODELS_REFRESH_INTERVAL_MS: u64 = 4 * 60 * 60 * 1000;
const DEFAULT_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// One model as the Codex backend describes it. Only the fields this module
/// reads are kept, so the saved file stays small.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexModelInfo {
    pub slug: String,
    /// None means older/unknown metadata; an explicit empty catalog means unsupported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tiers: Option<Vec<CodexServiceTierInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_reasoning_levels: Vec<CodexReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_in_api: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexReasoningLevel {
    pub effort: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexServiceTierInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastCapability {
    Supported,
    Unsupported,
    Unknown,
}

/// Sidecar lookup keeps capabilities even when a built-in catalog record wins.
pub fn fast_capability_for_model(agent_dir: &Path, model_id: &str) -> FastCapability {
    load_codex_models(agent_dir)
        .iter()
        .find(|model| model.slug == model_id)
        .map(CodexModelInfo::fast_capability)
        .unwrap_or(FastCapability::Unknown)
}

impl CodexModelInfo {
    pub fn fast_capability(&self) -> FastCapability {
        match &self.service_tiers {
            None => FastCapability::Unknown,
            Some(tiers)
                if tiers.iter().any(|tier| {
                    crate::CodexServiceTier::parse(&tier.id) == Some(crate::CodexServiceTier::Fast)
                }) =>
            {
                FastCapability::Supported
            }
            Some(_) => FastCapability::Unsupported,
        }
    }

    /// Codex shows a model in its picker when `visibility` is `list`. Hidden
    /// models stay usable by exact id through the resolver's custom-id path.
    fn is_listed(&self) -> bool {
        !self.slug.trim().is_empty()
            && self.visibility.as_deref().unwrap_or("list") == "list"
            && self.supported_in_api != Some(false)
    }
}

/// `<agent dir>/codex-models.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexModelsFile {
    #[serde(rename = "fetchedAt", default)]
    pub fetched_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default)]
    pub models: Vec<CodexModelInfo>,
}

pub fn codex_models_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(CODEX_MODELS_FILE)
}

/// `$CODEX_HOME/models_cache.json`, written by Codex CLI itself.
pub fn codex_cli_cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::auth::home_dir().map(|home| home.join(".codex")))?;
    Some(home.join("models_cache.json"))
}

/// Every file [`overlay_codex_models`] reads, for cache invalidation.
pub fn codex_model_source_paths(agent_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![codex_models_path(agent_dir)];
    paths.extend(codex_cli_cache_path());
    paths
}

/// Parse a models reply: the live `{ "models": [...] }` body, Codex CLI's
/// cache (same key plus metadata), or this module's own file.
pub fn parse_codex_models(value: &Value) -> Vec<CodexModelInfo> {
    let entries = value
        .get("models")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();
    entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<CodexModelInfo>(entry).ok())
        .filter(|model| !model.slug.trim().is_empty())
        .collect()
}

fn read_models(path: &Path) -> Option<(std::time::SystemTime, Vec<CodexModelInfo>)> {
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let models = parse_codex_models(&value);
    if models.is_empty() {
        return None;
    }
    let modified = fs::metadata(path).and_then(|meta| meta.modified()).ok()?;
    Some((modified, models))
}

/// The newest non-empty model list among the discovery sources.
pub fn load_codex_models(agent_dir: &Path) -> Vec<CodexModelInfo> {
    codex_model_source_paths(agent_dir)
        .iter()
        .filter_map(|path| read_models(path))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, models)| models)
        .unwrap_or_default()
}

/// Add every listed Codex model the catalog lacks. File reads only.
pub fn overlay_codex_models(models: &[Model], agent_dir: &Path) -> Vec<Model> {
    if matches!(
        std::env::var("DAVINCI_CODEX_MODEL_DISCOVERY").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    ) {
        return models.to_vec();
    }
    merge_codex_models(models, &load_codex_models(agent_dir))
}

/// Pure merge used by [`overlay_codex_models`]; existing records are kept.
pub fn merge_codex_models(models: &[Model], discovered: &[CodexModelInfo]) -> Vec<Model> {
    let mut merged = models.to_vec();
    let mut known: HashSet<String> = models
        .iter()
        .filter(|model| model.provider == CODEX_PROVIDER)
        .map(|model| model.id.clone())
        .collect();
    for info in discovered.iter().filter(|info| info.is_listed()) {
        if known.contains(&info.slug) {
            continue;
        }
        let Some(template) = closest_template(models, &info.slug) else {
            // No Codex record to copy transport settings from.
            return merged;
        };
        merged.push(model_from_info(template, info));
        known.insert(info.slug.clone());
    }
    merged
}

/// The existing Codex record sharing the longest id prefix, so `gpt-6-luna`
/// borrows from `gpt-6-astra` rather than from an older family.
fn closest_template<'a>(models: &'a [Model], slug: &str) -> Option<&'a Model> {
    let shared = |id: &str| {
        id.chars()
            .zip(slug.chars())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let mut best: Option<(&Model, usize)> = None;
    for model in models
        .iter()
        .filter(|model| model.provider == CODEX_PROVIDER && model.api == "openai-codex-responses")
    {
        let score = shared(&model.id);
        if best.is_none_or(|(_, top)| score > top) {
            best = Some((model, score));
        }
    }
    best.map(|(model, _)| model)
}

fn model_from_info(template: &Model, info: &CodexModelInfo) -> Model {
    let mut model = template.clone();
    model.id = info.slug.clone();
    model.name = info
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| info.slug.clone());
    if let Some(window) = info.context_window.filter(|window| *window > 0) {
        model.context_window = window;
    }
    if !info.input_modalities.is_empty() {
        model.input = info.input_modalities.clone();
    }
    // Long-context price tiers are looked up by id in the built-in catalog,
    // so a new id gets none; zero base cost keeps it from borrowing prices.
    model.cost = ModelCost {
        input: 0.0,
        output: 0.0,
        cache_read: 0.0,
        cache_write: 0.0,
    };
    if !info.supported_reasoning_levels.is_empty() {
        model.reasoning = true;
        model.thinking_level_map = thinking_level_map(&info.supported_reasoning_levels);
    }
    model
}

/// Map Davinci thinking levels onto the efforts Codex accepts for a model.
/// `off` is never sent (Codex has no "none" effort); `minimal` falls back to
/// the lowest supported effort; unsupported levels are hidden.
pub fn thinking_level_map(levels: &[CodexReasoningLevel]) -> BTreeMap<String, Option<String>> {
    let supported: Vec<&str> = levels.iter().map(|level| level.effort.as_str()).collect();
    let mut map = BTreeMap::new();
    map.insert("off".to_string(), None);
    let lowest = ["minimal", "low", "medium", "high"]
        .into_iter()
        .find(|effort| supported.contains(effort));
    map.insert("minimal".to_string(), lowest.map(str::to_string));
    for level in ["low", "medium", "high", "xhigh", "max"] {
        let value = supported.contains(&level).then(|| level.to_string());
        map.insert(level.to_string(), value);
    }
    map
}

/// Result of [`refresh_codex_models`].
#[derive(Debug, Clone, PartialEq)]
pub enum CodexModelsRefresh {
    /// Not attempted: no ChatGPT login, network off, or the file is fresh.
    Skipped,
    /// The backend answered; the number of models it listed.
    Updated(usize),
    /// The backend confirmed the saved list (HTTP 304).
    Unchanged,
}

/// Fetch the live Codex model list with the stored ChatGPT login and save it.
///
/// Fixtures: `DAVINCI_CODEX_MODELS_REPLY` (a JSON body or a path to one) is
/// used instead of the network; `DAVINCI_CODEX_MODELS_URL` points at a local
/// server. Tests never reach chatgpt.com.
pub fn refresh_codex_models(
    agent_dir: &Path,
    access_token: Option<&str>,
    allow_network: bool,
    force: bool,
) -> Result<CodexModelsRefresh, String> {
    let path = codex_models_path(agent_dir);
    let saved: Option<CodexModelsFile> = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    if let Ok(reply) = std::env::var("DAVINCI_CODEX_MODELS_REPLY") {
        let raw = if Path::new(&reply).is_file() {
            fs::read_to_string(&reply).map_err(|err| err.to_string())?
        } else {
            reply
        };
        let value: Value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
        return save(&path, parse_codex_models(&value), None);
    }
    let Some(token) = access_token.filter(|token| !token.is_empty()) else {
        return Ok(CodexModelsRefresh::Skipped);
    };
    if !allow_network {
        return Ok(CodexModelsRefresh::Skipped);
    }
    let now = crate::models_store::now_ms();
    if !force {
        if let Some(saved) = &saved {
            if now.saturating_sub(saved.fetched_at) < CODEX_MODELS_REFRESH_INTERVAL_MS {
                return Ok(CodexModelsRefresh::Skipped);
            }
        }
    }
    let base = std::env::var("DAVINCI_CODEX_MODELS_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_MODELS_URL.to_string());
    if cfg!(test) && !base.starts_with("http://127.0.0.1") && !base.starts_with("http://localhost")
    {
        return Ok(CodexModelsRefresh::Skipped);
    }
    let url = base;
    let mut request = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {token}"));
    if let Some(etag) = saved.as_ref().and_then(|saved| saved.etag.as_deref()) {
        request = request.set("if-none-match", etag);
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(status, _)) => {
            return Err(format!("Codex model list request failed: HTTP {status}"))
        }
        Err(err) => return Err(format!("Codex model list request failed: {err}")),
    };
    if response.status() == 304 {
        if let Some(mut saved) = saved {
            saved.fetched_at = now;
            write_file(&path, &saved)?;
        }
        return Ok(CodexModelsRefresh::Unchanged);
    }
    let etag = response.header("etag").map(str::to_string);
    let value: Value = response
        .into_json()
        .map_err(|err| format!("Invalid Codex model list: {err}"))?;
    save(&path, parse_codex_models(&value), etag)
}

fn save(
    path: &Path,
    models: Vec<CodexModelInfo>,
    etag: Option<String>,
) -> Result<CodexModelsRefresh, String> {
    if models.is_empty() {
        // Never replace a good list with an empty or unparseable reply.
        return Err("Codex model list was empty".into());
    }
    let count = models.len();
    let file = CodexModelsFile {
        fetched_at: crate::models_store::now_ms(),
        etag,
        models,
    };
    write_file(path, &file)?;
    Ok(CodexModelsRefresh::Updated(count))
}

fn write_file(path: &Path, file: &CodexModelsFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let body = serde_json::to_string_pretty(file).map_err(|err| err.to_string())?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, body).map_err(|err| err.to_string())?;
    fs::rename(&temp, path).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn codex_record(id: &str) -> Model {
        crate::catalog::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == CODEX_PROVIDER && model.id == id)
            .unwrap_or_else(|| panic!("built-in {id}"))
    }

    fn live_reply() -> Value {
        json!({"models": [
            {"slug": "gpt-6-astra", "display_name": "GPT-6-Astra", "context_window": 272000,
             "input_modalities": ["text", "image"], "visibility": "list", "supported_in_api": true,
             "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"},
                {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}, {"effort": "ultra"}]},
            {"slug": "gpt-6-luna", "display_name": "GPT-6-Luna", "context_window": 272000,
             "input_modalities": ["text", "image"], "visibility": "list", "supported_in_api": true,
             "priority": 3, "shell_type": "unified_exec",
             "supported_reasoning_levels": [{"effort": "low", "description": "Fast"},
                {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}]},
            {"slug": "gpt-reserve", "visibility": "hide", "supported_in_api": true,
             "supported_reasoning_levels": [{"effort": "low"}]},
            {"slug": "gpt-5.5-only-app", "visibility": "list", "supported_in_api": false},
            {"display_name": "no slug"}
        ]})
    }

    #[test]
    fn parses_live_reply_and_codex_cli_cache_shapes() {
        let live = parse_codex_models(&live_reply());
        assert_eq!(
            live.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(),
            [
                "gpt-6-astra",
                "gpt-6-luna",
                "gpt-reserve",
                "gpt-5.5-only-app"
            ]
        );
        let mut cache = live_reply();
        cache["fetched_at"] = json!("2026-09-28T19:33:17Z");
        cache["client_version"] = json!("0.157.1");
        assert_eq!(parse_codex_models(&cache), live);
    }

    #[test]
    fn adds_missing_listed_models_and_keeps_existing_records() {
        let astra = codex_record("gpt-6-astra");
        let catalog = vec![astra.clone()];
        let merged = merge_codex_models(&catalog, &parse_codex_models(&live_reply()));
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        // Hidden and app-only models are not added; astra is not duplicated.
        assert_eq!(ids, ["gpt-6-astra", "gpt-6-luna"]);
        assert_eq!(merged[0], astra, "existing record must not change");

        let luna = &merged[1];
        assert_eq!(luna.provider, CODEX_PROVIDER);
        assert_eq!(luna.name, "GPT-6-Luna");
        assert_eq!(luna.api, astra.api);
        assert_eq!(luna.base_url, astra.base_url);
        assert_eq!(luna.max_tokens, astra.max_tokens);
        assert_eq!(luna.context_window, 272_000);
        assert_eq!(luna.input, ["text", "image"]);
        assert_eq!(luna.cost.input, 0.0, "backend publishes no prices");
        assert_eq!(luna.cost.output, 0.0);
        assert!(luna.reasoning);
        assert_eq!(luna.thinking_level_map.get("off"), Some(&None));
        assert_eq!(
            luna.thinking_level_map.get("minimal"),
            Some(&Some("low".into()))
        );
        assert_eq!(
            luna.thinking_level_map.get("max"),
            Some(&Some("max".into()))
        );
    }

    #[test]
    fn thinking_map_hides_levels_codex_does_not_accept() {
        let levels = ["low", "medium", "high", "xhigh"]
            .map(|effort| CodexReasoningLevel {
                effort: effort.into(),
            })
            .to_vec();
        let map = thinking_level_map(&levels);
        assert_eq!(map.get("xhigh"), Some(&Some("xhigh".into())));
        assert_eq!(map.get("max"), Some(&None));
        let supported = crate::thinking::get_supported_thinking_levels(&Model {
            reasoning: true,
            thinking_level_map: map,
            ..codex_record("gpt-6-astra")
        });
        assert!(!supported.contains(&davinci_protocol::ThinkingLevel::Off));
        assert!(!supported.contains(&davinci_protocol::ThinkingLevel::Max));
        assert!(supported.contains(&davinci_protocol::ThinkingLevel::Xhigh));
    }

    #[test]
    fn template_prefers_the_same_model_family() {
        let catalog = vec![codex_record("gpt-5.5"), codex_record("gpt-6-astra")];
        assert_eq!(
            closest_template(&catalog, "gpt-6-luna").map(|m| m.id.as_str()),
            Some("gpt-6-astra")
        );
        assert_eq!(
            closest_template(&catalog, "gpt-5.9").map(|m| m.id.as_str()),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn no_codex_template_means_nothing_is_added() {
        let merged = merge_codex_models(&[], &parse_codex_models(&live_reply()));
        assert!(merged.is_empty());
    }

    #[test]
    fn overlay_reads_the_newest_source() {
        let dir = tempfile::tempdir().unwrap();
        let codex_home = dir.path().join("codex");
        fs::create_dir_all(&codex_home).unwrap();
        let _env = env_lock();
        std::env::set_var("CODEX_HOME", &codex_home);
        std::env::remove_var("DAVINCI_CODEX_MODEL_DISCOVERY");
        let catalog = vec![codex_record("gpt-6-astra")];

        // Nothing on disk: catalog unchanged.
        assert_eq!(overlay_codex_models(&catalog, dir.path()), catalog);

        // Codex CLI cache alone is enough, with no network.
        fs::write(
            codex_home.join("models_cache.json"),
            serde_json::to_string(&live_reply()).unwrap(),
        )
        .unwrap();
        let ids = |models: Vec<Model>| models.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(
            ids(overlay_codex_models(&catalog, dir.path())),
            ["gpt-6-astra", "gpt-6-luna"]
        );

        // A newer Davinci file wins over the older Codex cache.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = json!({"fetchedAt": 1, "models": [
            {"slug": "gpt-6-sol", "display_name": "GPT-6-Sol", "visibility": "list"}]});
        fs::write(codex_models_path(dir.path()), newer.to_string()).unwrap();
        assert_eq!(
            ids(overlay_codex_models(&catalog, dir.path())),
            ["gpt-6-astra", "gpt-6-sol"]
        );

        std::env::set_var("DAVINCI_CODEX_MODEL_DISCOVERY", "off");
        assert_eq!(overlay_codex_models(&catalog, dir.path()), catalog);
        std::env::remove_var("DAVINCI_CODEX_MODEL_DISCOVERY");
        std::env::remove_var("CODEX_HOME");
    }

    #[test]
    fn refresh_saves_fixture_reply_and_rejects_empty_lists() {
        let dir = tempfile::tempdir().unwrap();
        let _env = env_lock();
        std::env::set_var(
            "DAVINCI_CODEX_MODELS_REPLY",
            serde_json::to_string(&live_reply()).unwrap(),
        );
        let result = refresh_codex_models(dir.path(), None, false, false);
        assert_eq!(result, Ok(CodexModelsRefresh::Updated(4)));
        let saved: CodexModelsFile =
            serde_json::from_str(&fs::read_to_string(codex_models_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(saved.models.len(), 4);
        assert!(saved.fetched_at > 0);

        std::env::set_var("DAVINCI_CODEX_MODELS_REPLY", r#"{"models": []}"#);
        assert!(refresh_codex_models(dir.path(), None, false, false).is_err());
        let kept: CodexModelsFile =
            serde_json::from_str(&fs::read_to_string(codex_models_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(kept.models.len(), 4, "a bad reply keeps the saved list");
        std::env::remove_var("DAVINCI_CODEX_MODELS_REPLY");
    }

    #[test]
    fn refresh_needs_a_login_and_network() {
        let dir = tempfile::tempdir().unwrap();
        let _env = env_lock();
        std::env::remove_var("DAVINCI_CODEX_MODELS_REPLY");
        assert_eq!(
            refresh_codex_models(dir.path(), None, true, true),
            Ok(CodexModelsRefresh::Skipped)
        );
        assert_eq!(
            refresh_codex_models(dir.path(), Some("token"), false, true),
            Ok(CodexModelsRefresh::Skipped)
        );
    }

    #[test]
    fn refresh_sends_codex_headers_and_uses_etag() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = serde_json::to_string(&live_reply()).unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in [
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\netag: \"v1\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                ),
                "HTTP/1.1 304 Not Modified\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string(),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                stream.write_all(reply.as_bytes()).unwrap();
                requests.push(head.to_ascii_lowercase());
            }
            requests
        });
        let dir = tempfile::tempdir().unwrap();
        let _env = env_lock();
        std::env::remove_var("DAVINCI_CODEX_MODELS_REPLY");
        std::env::set_var(
            "DAVINCI_CODEX_MODELS_URL",
            format!("http://{addr}/v1/models"),
        );
        let first = refresh_codex_models(dir.path(), Some("tok-1"), true, true);
        let second = refresh_codex_models(dir.path(), Some("tok-1"), true, true);
        std::env::remove_var("DAVINCI_CODEX_MODELS_URL");
        assert_eq!(first, Ok(CodexModelsRefresh::Updated(4)));
        assert_eq!(second, Ok(CodexModelsRefresh::Unchanged));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("get /v1/models "));
        assert!(requests[0].contains("authorization: bearer tok-1"));
        assert!(!requests[0].contains("originator: "));
        assert!(!requests[0].contains("if-none-match"));
        assert!(requests[1].contains("if-none-match: \"v1\""));
        // A fresh file skips the network unless forced.
        assert_eq!(
            refresh_codex_models(dir.path(), Some("tok-1"), true, false),
            Ok(CodexModelsRefresh::Skipped)
        );
    }
}
