//! JavaScript extension subprocess runner matching
//! `vendor/pi/packages/coding-agent/src/core/extensions/loader.ts` when Node is present.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RUNNER_JS: &str = include_str!("extension_runner.js");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsExtensionResult {
    pub ok: bool,
    #[serde(default)]
    pub handlers: Vec<String>,
    #[serde(default)]
    pub tools: Vec<JsRegisteredTool>,
    #[serde(default)]
    pub providers: Vec<JsRegisteredProvider>,
    #[serde(default, rename = "unregisteredProviders")]
    pub unregistered_providers: Vec<String>,
    #[serde(default, rename = "terminalInputHandlers")]
    pub terminal_input_handlers: u32,
    #[serde(default, rename = "currentTheme")]
    pub current_theme: Option<Value>,
    #[serde(default, rename = "toolsExpanded")]
    pub tools_expanded: bool,
    #[serde(default)]
    pub commands: Vec<JsRegisteredCommand>,
    #[serde(default, rename = "autocompleteProviders")]
    pub autocomplete_providers: Vec<JsAutocompleteProvider>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub shortcuts: Vec<String>,
    #[serde(default, rename = "messageRenderers")]
    pub message_renderers: Vec<String>,
    #[serde(default, rename = "entryRenderers")]
    pub entry_renderers: Vec<String>,
    #[serde(default, rename = "markdownTransformers")]
    pub markdown_transformers: u32,
    #[serde(default, rename = "uiCalls")]
    pub ui_calls: Vec<Value>,
    #[serde(default, rename = "sessionCalls")]
    pub session_calls: Vec<Value>,
    #[serde(default, rename = "eventEmits")]
    pub event_emits: Vec<Value>,
    #[serde(default, rename = "hasEditor")]
    pub has_editor: bool,
    #[serde(default, rename = "hasCustom")]
    pub has_custom: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updates: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default, rename = "executionMode")]
    pub execution_mode: Option<String>,
    #[serde(default, rename = "renderShell")]
    pub render_shell: Option<String>,
    #[serde(default, rename = "hasRenderCall")]
    pub has_render_call: bool,
    #[serde(default, rename = "hasRenderResult")]
    pub has_render_result: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredProvider {
    pub name: String,
    #[serde(default)]
    pub config: Value,
    #[serde(default, rename = "hasStreamSimple")]
    pub has_stream_simple: bool,
    #[serde(default, rename = "hasRefreshModels")]
    pub has_refresh_models: bool,
    #[serde(default, rename = "hasOauth")]
    pub has_oauth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsAutocompleteItem {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsAutocompleteProvider {
    #[serde(default, rename = "triggerCharacters")]
    pub trigger_characters: Vec<String>,
    #[serde(default)]
    pub items: Vec<JsAutocompleteItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsRegisteredCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "argumentItems")]
    pub argument_items: Vec<JsAutocompleteItem>,
}

pub fn find_node() -> Option<PathBuf> {
    // `PI_NODE` stays uncached so tests and operators can retarget node
    // mid-process; the PATH probe below spawns `where`/`which` (~20-50ms on
    // Windows) and is resolved once per process.
    if let Ok(explicit) = std::env::var("PI_NODE") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Some(path);
        }
    }
    static RESOLVED: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    RESOLVED.get_or_init(find_node_uncached).clone()
}

fn find_node_uncached() -> Option<PathBuf> {
    // `where` on Windows, `which` elsewhere. Git for Windows ships a `which`
    // that answers with POSIX-style paths (/c/...) the OS cannot spawn, so the
    // resolved path is also verified with `exists()` before being trusted.
    let locator = if cfg!(windows) { "where" } else { "which" };
    for name in ["node", "nodejs"] {
        if let Ok(output) = Command::new(locator).arg(name).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let path = PathBuf::from(line);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

pub fn node_available() -> bool {
    find_node().is_some()
}

fn runner_path() -> Result<PathBuf, String> {
    static PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join("pi-extension-runner");
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        let path = dir.join(format!("extension_runner-{}.js", std::process::id()));
        std::fs::write(&path, RUNNER_JS).map_err(|err| err.to_string())?;
        Ok(path)
    })
    .clone()
}

pub fn resolve_extension_module(dir: &Path) -> Option<PathBuf> {
    // TS `resolveExtensionEntries`: index.ts before index.js, then package.json main.
    for name in [
        "index.ts",
        "index.tsx",
        "index.mts",
        "index.cts",
        "extension.ts",
        "index.js",
        "index.mjs",
        "index.cjs",
        "extension.js",
    ] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let package = dir.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(package) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            if let Some(main) = value.get("main").and_then(Value::as_str) {
                let candidate = dir.join(main);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

struct PersistentJsSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    module: PathBuf,
    /// Reply from the implicit `load` that `--persistent` performs at spawn.
    /// It carries the ui/session calls the factory made while loading; the
    /// first explicit `load` consumes it instead of re-sending.
    initial_load: Option<JsExtensionResult>,
}

impl PersistentJsSession {
    fn start(module: &Path) -> Result<Self, String> {
        let node =
            find_node().ok_or_else(|| "Node.js is not available for JS extensions".to_string())?;
        let runner = runner_path()?;
        let mut child = Command::new(node)
            .arg(&runner)
            .arg(module)
            .arg("--persistent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| err.to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "persistent stdin".to_string())?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| "persistent stdout".to_string())?,
        );
        let mut session = Self {
            child,
            stdin,
            stdout,
            module: module.to_path_buf(),
            initial_load: None,
        };
        session.initial_load = session.read_line().ok();
        Ok(session)
    }

    fn send(&mut self, op: &str, payload: &Value) -> Result<JsExtensionResult, String> {
        let line = serde_json::json!({ "op": op, "payload": payload });
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .map_err(|err| err.to_string())?;
        self.stdin.flush().map_err(|err| err.to_string())?;
        self.read_line()
    }

    fn read_line(&mut self) -> Result<JsExtensionResult, String> {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .map_err(|err| err.to_string())?;
        serde_json::from_str(line.trim()).map_err(|err| format!("extension runner: {err}: {line}"))
    }
}

impl Drop for PersistentJsSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static PERSISTENT_JS: Mutex<Option<PersistentJsSession>> = Mutex::new(None);
static NODE_LOCK: Mutex<()> = Mutex::new(());

type UiWaiter = Box<dyn FnMut(&Value) -> Value>;

thread_local! {
    static UI_WAITER: RefCell<Option<UiWaiter>> = const { RefCell::new(None) };
}

pub fn install_ui_waiter(waiter: Box<dyn FnMut(&Value) -> Value>) {
    UI_WAITER.with(|slot| {
        *slot.borrow_mut() = Some(waiter);
    });
}

pub fn clear_ui_waiter() {
    UI_WAITER.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

fn ui_waiter_installed() -> bool {
    UI_WAITER.with(|slot| slot.borrow().is_some())
}

fn dispatch_ui_waiter(call: &Value) -> Value {
    UI_WAITER.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|waiter| waiter(call))
            .unwrap_or(Value::Null)
    })
}

fn poll_ui_channel(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("req") {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(call) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        let reply = dispatch_ui_waiter(&call);
        let _ = std::fs::write(path.with_extension("rep"), reply.to_string());
        let _ = std::fs::remove_file(path);
    }
}

pub fn run_persistent_js_extension(
    module: &Path,
    op: &str,
    payload: &Value,
) -> Result<JsExtensionResult, String> {
    let _guard = NODE_LOCK.lock().map_err(|err| err.to_string())?;
    let mut slot = PERSISTENT_JS.lock().map_err(|err| err.to_string())?;
    if slot
        .as_ref()
        .is_some_and(|session| session.module != module)
    {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(PersistentJsSession::start(module)?);
    }
    slot.as_mut()
        .ok_or_else(|| "persistent JS session missing".to_string())?
        .send(op, payload)
}

pub fn stop_persistent_js_extension() {
    if let Ok(mut slot) = PERSISTENT_JS.lock() {
        *slot = None;
    }
}

/// One long-lived `--persistent` runner per extension module. A fresh node
/// spawn per event made every agent turn pay seconds of process startup
/// (worse with TypeScript extensions that load a transpiler); the runner
/// already speaks line-delimited `{op, payload}` in persistent mode, and the
/// TS reference keeps extensions loaded for the whole session anyway.
static JS_POOL: Mutex<Option<std::collections::HashMap<PathBuf, PersistentJsSession>>> =
    Mutex::new(None);

fn js_pool_enabled() -> bool {
    !matches!(
        std::env::var("PI_JS_POOL").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

fn run_pooled_js_extension(
    module: &Path,
    op: &str,
    payload: &Value,
) -> Result<JsExtensionResult, String> {
    let mut pool = JS_POOL.lock().map_err(|err| err.to_string())?;
    let pool = pool.get_or_insert_with(std::collections::HashMap::new);
    if !pool.contains_key(module) {
        pool.insert(module.to_path_buf(), PersistentJsSession::start(module)?);
    }
    let session = pool.get_mut(module).expect("pooled session");
    if op == "load" {
        if let Some(initial) = session.initial_load.take() {
            return Ok(initial);
        }
    }
    match session.send(op, payload) {
        Ok(result) => Ok(result),
        Err(_) => {
            // The runner died (crash, stdin closed): respawn once and retry.
            pool.remove(module);
            let mut fresh = PersistentJsSession::start(module)?;
            let result = fresh.send(op, payload);
            pool.insert(module.to_path_buf(), fresh);
            result
        }
    }
}

/// Drop every pooled runner (used on shutdown and by tests).
pub fn shutdown_js_pool() {
    if let Ok(mut pool) = JS_POOL.lock() {
        *pool = None;
    }
    if let Ok(mut cache) = LOAD_CACHE.lock() {
        cache.clear();
    }
}

/// `load` replies memoized per (module, mtime, payload). Host objects are
/// rebuilt frequently; without this every rebuild paid one node spawn per
/// extension just to re-read a manifest that cannot have changed.
type LoadCacheKey = (PathBuf, Option<std::time::SystemTime>, String);
static LOAD_CACHE: Mutex<std::collections::BTreeMap<LoadCacheKey, JsExtensionResult>> =
    Mutex::new(std::collections::BTreeMap::new());

fn load_cache_key(module: &Path, payload: &Value) -> LoadCacheKey {
    let mtime = std::fs::metadata(module)
        .and_then(|meta| meta.modified())
        .ok();
    (module.to_path_buf(), mtime, payload.to_string())
}

pub fn run_js_extension(
    module: &Path,
    op: &str,
    payload: &Value,
) -> Result<JsExtensionResult, String> {
    let wait_ui = ui_waiter_installed();
    // The UI-wait channel and the `PI_EXTENSION_UI_REPLY` fixture are wired
    // through spawn-time environment, so those calls keep the one-shot path.
    // `load` also stays one-shot: the factory must observe the load payload
    // (themes, cwd), which a warm runner has already consumed. Repeated
    // identical loads are served from the memo instead of a fresh spawn.
    let pooling =
        !wait_ui && std::env::var_os("PI_EXTENSION_UI_REPLY").is_none() && js_pool_enabled();
    if op == "load" && pooling {
        let key = load_cache_key(module, payload);
        if let Some(cached) = LOAD_CACHE
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
        {
            return Ok(cached);
        }
    }
    if op != "load" && pooling {
        let _guard = NODE_LOCK.lock().map_err(|err| err.to_string())?;
        return run_pooled_js_extension(module, op, payload);
    }
    let _guard = NODE_LOCK.lock().map_err(|err| err.to_string())?;
    let node =
        find_node().ok_or_else(|| "Node.js is not available for JS extensions".to_string())?;
    let runner = runner_path()?;
    let channel = if wait_ui {
        let dir = std::env::temp_dir().join(format!("pi-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        Some(dir)
    } else {
        None
    };
    let mut command = Command::new(node);
    command
        .arg(&runner)
        .arg(module)
        .arg(op)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &channel {
        command.env("PI_RPC_UI_WAIT", "1");
        command.env("PI_EXTENSION_UI_CHANNEL", dir);
        command.env_remove("PI_EXTENSION_UI_REPLY");
    }
    let mut child = command.spawn().map_err(|err| err.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.to_string().as_bytes())
            .map_err(|err| err.to_string())?;
    }
    if let Some(dir) = &channel {
        while child.try_wait().map_err(|err| err.to_string())?.is_none() {
            poll_ui_channel(dir);
            std::thread::sleep(Duration::from_millis(20));
        }
        poll_ui_channel(dir);
        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_string(&mut stdout)
                .map_err(|err| err.to_string())?;
        }
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let _ = std::fs::remove_dir_all(dir);
        return serde_json::from_str(&stdout)
            .map_err(|err| format!("extension runner: {err}: {} {stderr}", stdout.trim()));
    }
    let output = child.wait_with_output().map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: JsExtensionResult = serde_json::from_str(&stdout).map_err(|err| {
        format!(
            "extension runner: {err}: {} {}",
            stdout.trim(),
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    if op == "load" && pooling && parsed.ok {
        if let Ok(mut cache) = LOAD_CACHE.lock() {
            cache.insert(load_cache_key(module, payload), parsed.clone());
        }
    }
    Ok(parsed)
}

pub fn query_js_autocomplete(module: &Path, text: &str) -> Vec<JsAutocompleteItem> {
    let result =
        run_persistent_js_extension(module, "autocomplete", &serde_json::json!({ "text": text }))
            .ok();
    result
        .and_then(|loaded| loaded.result)
        .and_then(|value| value.get("items").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub fn run_js_stream_simple(
    module: &Path,
    provider: &str,
    model: &pi_ai::Model,
    messages: &[pi_ai::ChatMessage],
    system: &str,
) -> Result<pi_ai::AssistantMessage, String> {
    let result = run_persistent_js_extension(
        module,
        "streamSimple",
        &serde_json::json!({
            "provider": provider,
            "model": {
                "id": model.id,
                "provider": model.provider,
                "name": model.name,
            },
            "context": {
                "systemPrompt": system,
                "messages": messages,
            },
        }),
    )?;
    let value = result.result.unwrap_or(Value::Null);
    if let Some(text) = value.as_str() {
        return Ok(pi_ai::AssistantMessage {
            id: pi_agent::new_message_id(),
            role: "assistant".into(),
            content: vec![pi_ai::ContentBlock::Text {
                text: text.to_string(),
            }],
            model: format!("{}/{}", model.provider, model.id),
            usage: None,
            stop_reason: Some(pi_ai::StopReason::Stop),
            error_message: None,
        });
    }
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value.get("content").and_then(|content| {
                if let Some(text) = content.as_str() {
                    Some(text.to_string())
                } else {
                    content.as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                }
            })
        })
        .unwrap_or_default();
    Ok(pi_ai::AssistantMessage {
        id: pi_agent::new_message_id(),
        role: "assistant".into(),
        content: vec![pi_ai::ContentBlock::Text { text }],
        model: format!("{}/{}", model.provider, model.id),
        usage: None,
        stop_reason: Some(pi_ai::StopReason::Stop),
        error_message: None,
    })
}

pub fn run_js_refresh_models(
    module: &Path,
    provider: &str,
    allow_network: bool,
    force: bool,
) -> Result<Vec<pi_ai::Model>, String> {
    let result = run_persistent_js_extension(
        module,
        "refreshModels",
        &serde_json::json!({
            "provider": provider,
            "context": {
                "allowNetwork": allow_network,
                "force": force,
                "signal": { "aborted": false },
            },
        }),
    )?;
    let models = result
        .result
        .and_then(|value| value.get("models").cloned())
        .unwrap_or(Value::Null);
    Ok(pi_ai::models_from_provider_config(
        provider,
        &serde_json::json!({ "models": models }),
    ))
}

fn oauth_credentials_json(access: &str, refresh: Option<&str>, expires: Option<u64>) -> Value {
    serde_json::json!({
        "access": access,
        "refresh": refresh.unwrap_or(""),
        "expires": expires.unwrap_or(0),
    })
}

pub fn parse_oauth_credentials(
    value: &Value,
) -> Result<(String, Option<String>, Option<u64>), String> {
    let access = value
        .get("access")
        .and_then(Value::as_str)
        .ok_or_else(|| "OAuth login missing access token".to_string())?
        .to_string();
    let refresh = value
        .get("refresh")
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty())
        .map(str::to_string);
    let expires = value.get("expires").and_then(Value::as_u64);
    Ok((access, refresh, expires))
}

pub fn run_js_oauth_login(
    module: &Path,
    provider: &str,
) -> Result<(String, Option<String>, Option<u64>), String> {
    let result = run_persistent_js_extension(
        module,
        "oauthLogin",
        &serde_json::json!({ "provider": provider }),
    )?;
    let value = result.result.unwrap_or(Value::Null);
    parse_oauth_credentials(&value)
}

pub fn run_js_oauth_refresh(
    module: &Path,
    provider: &str,
    access: &str,
    refresh: Option<&str>,
    expires: Option<u64>,
) -> Result<(String, Option<String>, Option<u64>), String> {
    let result = run_persistent_js_extension(
        module,
        "oauthRefresh",
        &serde_json::json!({
            "provider": provider,
            "credentials": oauth_credentials_json(access, refresh, expires),
        }),
    )?;
    parse_oauth_credentials(&result.result.unwrap_or(Value::Null))
}

pub fn run_js_oauth_get_api_key(
    module: &Path,
    provider: &str,
    access: &str,
    refresh: Option<&str>,
    expires: Option<u64>,
) -> Option<String> {
    let result = run_persistent_js_extension(
        module,
        "oauthGetApiKey",
        &serde_json::json!({
            "provider": provider,
            "credentials": oauth_credentials_json(access, refresh, expires),
        }),
    )
    .ok()?;
    result.result.and_then(|value| {
        value
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

pub fn execute_js_tool(
    module: &Path,
    name: &str,
    args: &Value,
    cwd: &Path,
) -> Result<pi_agent::ToolResult, pi_agent::ToolError> {
    let result = run_js_extension(
        module,
        "tool",
        &serde_json::json!({
            "name": name,
            "args": args,
            "cwd": cwd,
            "toolCallId": "call",
        }),
    )
    .map_err(pi_agent::ToolError::Failed)?;
    if !result.ok {
        return Err(pi_agent::ToolError::Failed(
            result
                .error
                .unwrap_or_else(|| format!("JS tool {name} failed")),
        ));
    }
    let value = result.result.unwrap_or(Value::Null);
    let mut details = value.get("details").cloned();
    if !result.updates.is_empty() {
        let mut map = details
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        map.insert("_piUpdates".into(), Value::Array(result.updates.clone()));
        details = Some(Value::Object(map));
    }
    Ok(pi_agent::ToolResult {
        content: value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_error: value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        details,
    })
}

pub fn render_js_tool_call(module: &Path, name: &str, args: &Value, width: usize) -> Vec<String> {
    render_js_tool(module, "renderToolCall", name, args, width)
}

pub fn render_js_tool_result(
    module: &Path,
    name: &str,
    result: &Value,
    width: usize,
) -> Vec<String> {
    let payload = serde_json::json!({
        "name": name,
        "result": result,
        "width": width,
        "expanded": false,
    });
    run_js_extension(module, "renderToolResult", &payload)
        .ok()
        .and_then(|loaded| loaded.result)
        .and_then(|value| {
            value.get("lines").and_then(Value::as_array).map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn render_js_tool(module: &Path, op: &str, name: &str, args: &Value, width: usize) -> Vec<String> {
    let payload = serde_json::json!({
        "name": name,
        "args": args,
        "width": width,
    });
    run_js_extension(module, op, &payload)
        .ok()
        .and_then(|loaded| loaded.result)
        .and_then(|value| {
            value.get("lines").and_then(Value::as_array).map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default()
}

pub fn execute_command_tool(command: &str, cwd: &Path) -> Result<String, String> {
    let output = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", command])
            .current_dir(cwd)
            .output()
    } else {
        Command::new("sh")
            .args(["-c", command])
            .current_dir(cwd)
            .output()
    }
    .map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// PERSISTENT_JS is one process-wide slot keyed by module path; parallel
    /// tests with different modules evict each other's live session mid-test.
    /// Every test that touches the persistent host must hold this lock.
    static PERSISTENT_HOST_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn persistent_host_guard() -> std::sync::MutexGuard<'static, ()> {
        PERSISTENT_HOST_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn loads_js_factory_when_node_is_present() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.on("tool_call", (event) => ({ block: event.toolName === "bash", reason: "blocked by fixture" }));
  pi.registerTool({ name: "ticket", description: "lookup" });
  pi.registerCommand("tickets", { description: "list tickets" });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert!(loaded.handlers.contains(&"tool_call".into()));
        let emitted = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({"type":"tool_call","toolName":"bash"}),
        )
        .unwrap();
        assert_eq!(emitted.result.as_ref().unwrap()["block"], true);
        assert_eq!(
            emitted.result.as_ref().unwrap()["reason"],
            "blocked by fixture"
        );
    }

    #[test]
    fn event_handler_order_matches_ts_for_bash_and_before_agent_start() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.on("user_bash", () => ({ content: "first" }));
  pi.on("user_bash", () => ({ content: "second" }));
  pi.on("before_agent_start", (event) => ({
    message: { customType: "one", content: event.prompt },
    systemPrompt: event.systemPrompt + " -> one",
  }));
  pi.on("before_agent_start", (event) => ({
    message: { customType: "two", content: event.systemPrompt },
    systemPrompt: event.systemPrompt + " -> two",
  }));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();

        let bash = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({
                "type": "user_bash",
                "command": "echo hi",
                "excludeFromContext": false,
                "cwd": "."
            }),
        )
        .unwrap();
        assert_eq!(bash.result.as_ref().unwrap()["content"], "first");

        let before = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({
                "type": "before_agent_start",
                "prompt": "hello",
                "images": [],
                "systemPrompt": "base"
            }),
        )
        .unwrap();
        let result = before.result.as_ref().unwrap();
        assert_eq!(result["messages"].as_array().unwrap().len(), 2);
        assert_eq!(result["messages"][1]["content"], "base -> one");
        assert_eq!(result["systemPrompt"], "base -> one -> two");
    }

    #[test]
    fn records_set_model_get_flag_and_session_ops() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerFlag("demo", { type: "boolean", default: true });
  pi.registerCommand("go", {
    description: "go",
    handler: async (_args, ctx) => {
      const flag = pi.getFlag("demo");
      await pi.setModel({ provider: "anthropic", id: "sonnet" });
      ctx.newSession();
      ctx.switchSession("/tmp/x.jsonl");
      ctx.navigateTree("leaf-1");
      return { flag };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({
                "name": "go",
                "flagValues": { "demo": true },
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(command.result.as_ref().unwrap()["flag"], true);
        let ops: Vec<&str> = command
            .session_calls
            .iter()
            .filter_map(|call| call.get("op").and_then(|value| value.as_str()))
            .collect();
        assert!(ops.contains(&"setModel"));
        assert!(ops.contains(&"newSession"));
        assert!(ops.contains(&"switchSession"));
        assert!(ops.contains(&"navigateTree"));
    }

    #[test]
    fn records_argument_completions_and_autocomplete_providers() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r##"
module.exports = (pi) => {
  pi.registerCommand("commands", {
    description: "List commands",
    getArgumentCompletions: (prefix) => {
      const sources = ["extension", "prompt", "skill"];
      return sources.filter((s) => s.startsWith(prefix)).map((s) => ({ value: s, label: s }));
    },
  });
  pi.ui.addAutocompleteProvider((current) => ({
    ...current,
    triggerCharacters: ["#"],
    getSuggestions: async () => ({ items: [{ value: "#42", label: "#42" }], prefix: "#" }),
  }));
};
"##,
        )
        .unwrap();
        std::env::set_var(
            "PI_EXTENSION_AUTOCOMPLETE_REPLY",
            r##"[{"value":"#42","label":"#42","description":"[open] Login crash"}]"##,
        );
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        std::env::remove_var("PI_EXTENSION_AUTOCOMPLETE_REPLY");
        assert_eq!(loaded.commands[0].name, "commands");
        assert!(loaded.commands[0]
            .argument_items
            .iter()
            .any(|item| item.value == "extension"));
        assert_eq!(loaded.autocomplete_providers[0].trigger_characters, ["#"]);
        assert_eq!(loaded.autocomplete_providers[0].items[0].value, "#42");
    }

    #[test]
    fn live_autocomplete_queries_get_suggestions_with_prefix() {
        let Some(_) = find_node() else {
            return;
        };
        let _persistent = persistent_host_guard();
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r##"
module.exports = (pi) => {
  pi.ui.addAutocompleteProvider(() => ({
    triggerCharacters: ["#"],
    getSuggestions: async (lines) => {
      const text = lines.join("\n");
      return { items: [{ value: "live:" + text, label: "live:" + text }] };
    },
  }));
};
"##,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        stop_persistent_js_extension();
        let items = query_js_autocomplete(&module, "#99");
        stop_persistent_js_extension();
        assert_eq!(items[0].value, "live:#99");
    }

    #[test]
    fn events_emit_delivers_to_same_extension_listeners() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("ping", {
    handler: () => {
      let seen;
      pi.events.on("demo", (data) => { seen = data; });
      pi.events.emit("demo", { ok: true });
      return { seen };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "ping", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert_eq!(command.result.as_ref().unwrap()["seen"]["ok"], true);
        assert_eq!(
            command.event_emits[0]
                .get("channel")
                .and_then(|v| v.as_str()),
            Some("demo")
        );
    }

    #[test]
    fn stream_simple_handler_returns_assistant_text() {
        let _persistent = persistent_host_guard();
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerProvider("fixture-ai", {
    models: [{ id: "echo", name: "echo" }],
    streamSimple: async (_model, context) => {
      const last = (context.messages || []).slice(-1)[0];
      return { text: "streamed:" + (last && last.content ? JSON.stringify(last.content) : "") };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.providers[0].has_stream_simple);
        stop_persistent_js_extension();
        let model = pi_ai::Model {
            id: "echo".into(),
            name: "echo".into(),
            api: "openai-completions".into(),
            provider: "fixture-ai".into(),
            base_url: None,
            reasoning: false,
            input: vec!["text".into()],
            cost: pi_ai::ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 1000,
            max_tokens: 100,
            compat: serde_json::json!(null),
            headers: Default::default(),
            thinking_level_map: Default::default(),
        };
        let message = run_js_stream_simple(
            &module,
            "fixture-ai",
            &model,
            &[pi_ai::ChatMessage::text("user", "hi")],
            "sys",
        )
        .unwrap();
        stop_persistent_js_extension();
        let text = message
            .content
            .iter()
            .find_map(|block| match block {
                pi_ai::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or_default();
        assert!(text.contains("streamed:"));
    }

    #[test]
    fn refresh_models_and_oauth_handlers_run_from_js() {
        let _persistent = persistent_host_guard();
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerProvider("corp-ai", {
    models: [{ id: "static", name: "static" }],
    refreshModels: async ({ allowNetwork }) => {
      return [{ id: allowNetwork ? "live" : "cached", name: "dyn", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 64 }];
    },
    oauth: {
      name: "Corp",
      login: async () => ({ access: "acc-1", refresh: "ref-1", expires: 99 }),
      refreshToken: async (creds) => ({ access: creds.access + "-new", refresh: creds.refresh, expires: 100 }),
      getApiKey: (creds) => "key:" + creds.access,
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.providers[0].has_refresh_models);
        assert!(loaded.providers[0].has_oauth);
        stop_persistent_js_extension();
        let models = run_js_refresh_models(&module, "corp-ai", true, true).unwrap();
        assert!(models.iter().any(|model| model.id == "live"));
        let login = run_js_oauth_login(&module, "corp-ai").unwrap();
        assert_eq!(login.0, "acc-1");
        let refreshed =
            run_js_oauth_refresh(&module, "corp-ai", "acc-1", Some("ref-1"), Some(99)).unwrap();
        assert_eq!(refreshed.0, "acc-1-new");
        let key =
            run_js_oauth_get_api_key(&module, "corp-ai", "acc-1", Some("ref-1"), Some(99)).unwrap();
        assert_eq!(key, "key:acc-1");
        stop_persistent_js_extension();
    }

    #[test]
    fn registers_and_invokes_shortcut_handler() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerShortcut("ctrl+k", {
    description: "ping",
    handler: () => ({ handled: true }),
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.shortcuts.iter().any(|key| key == "ctrl+k"));
        let invoked =
            run_js_extension(&module, "shortcut", &serde_json::json!({ "key": "ctrl+k" })).unwrap();
        assert_eq!(invoked.result.as_ref().unwrap()["handled"], true);
    }

    #[test]
    fn records_extension_ui_calls() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.ui.setWidget("banner", ["hello widget"], { placement: "aboveEditor" });
  pi.ui.setStatus("job", "running");
  pi.ui.notify("ready", "info");
  pi.registerCommand("ask", {
    description: "ask",
    handler: async (_args, ctx) => {
      const choice = await ctx.ui.select("Pick", ["a", "b"]);
      return { choice };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert!(loaded
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setWidget" && call["key"] == "banner"));
        std::env::set_var("PI_EXTENSION_UI_REPLY", "a");
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "ask", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        std::env::remove_var("PI_EXTENSION_UI_REPLY");
        assert_eq!(command.result.as_ref().unwrap()["choice"], "a");
        assert!(command.ui_calls.iter().any(|call| call["op"] == "select"));
    }

    #[test]
    fn rpc_ui_wait_uses_waiter_and_timeout_default() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("ask", {
    description: "ask",
    handler: async (_args, ctx) => {
      const choice = await ctx.ui.select("Pick", ["a", "b"]);
      return { choice: choice ?? "timeout" };
    },
  });
  pi.registerCommand("slow", {
    description: "slow",
    handler: async (_args, ctx) => {
      const choice = await ctx.ui.select("Pick", ["a", "b"], { timeout: 80 });
      return { choice: choice ?? "timeout" };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        install_ui_waiter(Box::new(|call| {
            assert_eq!(call["op"], "select");
            Value::String("a".into())
        }));
        let answered = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "ask", "ctx": { "mode": "rpc" } }),
        )
        .unwrap();
        clear_ui_waiter();
        assert_eq!(answered.result.as_ref().unwrap()["choice"], "a");

        install_ui_waiter(Box::new(|_call| {
            std::thread::sleep(Duration::from_millis(200));
            Value::String("late".into())
        }));
        let timed = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "slow", "ctx": { "mode": "rpc" } }),
        )
        .unwrap();
        clear_ui_waiter();
        assert_eq!(timed.result.as_ref().unwrap()["choice"], "timeout");
    }

    #[test]
    fn records_ctx_session_apis() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("session", {
    description: "session",
    handler: async (_args, ctx) => {
      ctx.sendMessage("hello");
      ctx.appendEntry("note", { text: "hi" });
      ctx.setLabel("e1", "keep");
      ctx.setSessionName("work");
      await ctx.exec("echo ok");
      ctx.newSession();
      ctx.fork("root");
      return { ok: true };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        std::env::set_var("PI_EXTENSION_EXEC_REPLY", "ok");
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "session", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        std::env::remove_var("PI_EXTENSION_EXEC_REPLY");
        let ops: Vec<&str> = command
            .session_calls
            .iter()
            .filter_map(|call| call.get("op").and_then(|value| value.as_str()))
            .collect();
        assert!(ops.contains(&"sendMessage"));
        assert!(ops.contains(&"appendEntry"));
        assert!(ops.contains(&"setLabel"));
        assert!(ops.contains(&"setSessionName"));
        assert!(ops.contains(&"exec"));
        assert!(ops.contains(&"newSession"));
        assert!(ops.contains(&"fork"));
    }

    #[test]
    fn records_active_tools_thinking_and_working_indicator() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("plan", {
    description: "plan",
    handler: async (_args, ctx) => {
      const current = pi.getActiveTools();
      pi.setActiveTools(["read", "bash"]);
      pi.setThinkingLevel("max");
      ctx.ui.setWorkingVisible(false);
      ctx.ui.setWorkingIndicator({ frames: ["●"], intervalMs: 80 });
      return { current, thinking: pi.getThinkingLevel() };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let command = run_js_extension(
            &module,
            "command",
            &serde_json::json!({
                "name": "plan",
                "activeTools": ["read", "bash", "edit"],
                "thinkingLevel": "high",
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        let ops: Vec<&str> = command
            .session_calls
            .iter()
            .filter_map(|call| call.get("op").and_then(|value| value.as_str()))
            .collect();
        assert!(ops.contains(&"setActiveTools"));
        assert!(ops.contains(&"setThinkingLevel"));
        assert_eq!(command.result.as_ref().unwrap()["current"][0], "read");
        assert_eq!(command.result.as_ref().unwrap()["thinking"], "high");
        assert_eq!(
            command
                .session_calls
                .iter()
                .find(|call| call["op"] == "setThinkingLevel")
                .and_then(|call| call.get("level"))
                .and_then(|value| value.as_str()),
            Some("max")
        );
        assert!(command
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setWorkingVisible" && call["visible"] == false));
        assert!(command
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setWorkingIndicator"));
    }

    #[test]
    fn loads_typescript_factory_with_virtual_packages() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.ts"),
            r#"
import ai from "@earendil-works/pi-ai";
import { version } from "@mariozechner/pi-agent-core";

export default (pi) => {
  pi.registerTool({ name: "ticket", description: String(ai.version || version || "ok") });
  pi.on("tool_call", (event) => ({ block: event.toolName === "bash" }));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        assert!(module.extension() == Some("ts".as_ref()));
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert!(loaded.tools[0].description.contains("0.84.4"));
        assert!(loaded.handlers.contains(&"tool_call".into()));
    }

    #[test]
    fn registers_and_renders_message_renderer() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerMessageRenderer("status-update", (message, options) => {
    const pad = " ".repeat(options.outputPad || 0);
    return { render: (width) => [`${pad}${message.customType}:${message.content}:${width}`] };
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.message_renderers, ["status-update"]);
        let rendered = run_js_extension(
            &module,
            "renderMessage",
            &serde_json::json!({
                "customType": "status-update",
                "message": {"role":"custom","customType":"status-update","content":"ok"},
                "options": {"expanded": false, "outputPad": 1},
                "width": 40
            }),
        )
        .unwrap();
        let lines = rendered.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert_eq!(lines[0], " status-update:ok:40");
        assert!(run_js_extension(
            &module,
            "renderMessage",
            &serde_json::json!({"customType":"missing"})
        )
        .unwrap()
        .result
        .is_none());
    }

    #[test]
    fn registers_entry_renderer_and_markdown_transformer() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerEntryRenderer("ticket", (entry) => [`entry:${entry.customType}`]);
  pi.registerMarkdownTransformer((markdown) => markdown.replace("TODO", "DONE"));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.entry_renderers, ["ticket"]);
        assert_eq!(loaded.markdown_transformers, 1);
        let entry = run_js_extension(
            &module,
            "renderEntry",
            &serde_json::json!({"customType":"ticket","entry":{"customType":"ticket"}}),
        )
        .unwrap();
        assert_eq!(entry.result.as_ref().unwrap()["lines"][0], "entry:ticket");
        let transformed = run_js_extension(
            &module,
            "transformMarkdown",
            &serde_json::json!({"markdown":"TODO now"}),
        )
        .unwrap();
        assert_eq!(transformed.result.as_ref().unwrap()["markdown"], "DONE now");
    }

    #[test]
    fn runs_manifest_command_tool() {
        let dir = tempdir().unwrap();
        let command = if cfg!(windows) {
            "echo fixture-ok"
        } else {
            "printf fixture-ok"
        };
        let out = execute_command_tool(command, dir.path()).unwrap();
        assert_eq!(out.trim_end_matches(['\r', '\n']), "fixture-ok");
    }

    #[test]
    fn hosts_js_custom_editor_from_session_start() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Editor, matchesKey } = require("@earendil-works/pi-tui");
class ModalEditor extends Editor {
  constructor() {
    super();
    this.mode = "insert";
  }
  handleInput(data) {
    if (matchesKey(data, "escape")) {
      if (this.mode === "insert") { this.mode = "normal"; return; }
      super.handleInput(data);
      return;
    }
    if (this.mode === "normal" && data === "i") { this.mode = "insert"; return; }
    super.handleInput(data);
  }
  render(width) {
    const lines = super.render(width);
    lines.push(this.mode === "normal" ? " NORMAL " : " INSERT ");
    return lines;
  }
}
module.exports = (pi) => {
  pi.on("session_start", (_event, ctx) => {
    ctx.ui.setEditorComponent(() => new ModalEditor());
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert!(loaded.handlers.contains(&"session_start".into()));
        let rendered =
            run_js_extension(&module, "editorRender", &serde_json::json!({ "width": 40 })).unwrap();
        assert_eq!(rendered.result.as_ref().unwrap()["enabled"], true);
        let lines = rendered.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert!(
            lines.iter().any(|line| line.as_str() == Some(" INSERT ")),
            "{lines:?}"
        );
        let after_escape = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({ "data": "\u{1b}", "width": 40 }),
        )
        .unwrap();
        let escape_lines = after_escape.result.as_ref().unwrap()["lines"]
            .as_array()
            .expect("lines");
        assert!(
            escape_lines
                .iter()
                .any(|line| line.as_str() == Some(" NORMAL ")),
            "{escape_lines:?}"
        );
        let typed = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({
                "data": "h",
                "snapshot": { "text": "", "extra": { "mode": "insert" } },
                "width": 40
            }),
        )
        .unwrap();
        assert_eq!(typed.result.as_ref().unwrap()["text"], "h");
        let submitted = run_js_extension(
            &module,
            "editorInput",
            &serde_json::json!({
                "data": "\r",
                "snapshot": { "text": "hello", "extra": { "mode": "insert" } },
                "width": 40
            }),
        )
        .unwrap();
        assert_eq!(submitted.result.as_ref().unwrap()["action"], "submit");
        assert_eq!(submitted.result.as_ref().unwrap()["text"], "hello");
    }

    #[test]
    fn hosts_js_custom_overlay_until_done() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("board", {
    description: "board",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((_tui, _theme, _kb, done) => {
        return {
          label: "open",
          handleInput(data) {
            if (data === "q") done("closed");
            else this.label = data;
          },
          render() { return [this.label]; },
        };
      });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let opened = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "board", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(opened.ok, "{:?}", opened.error);
        assert_eq!(opened.result.as_ref().unwrap()["pending"], true);
        assert_eq!(opened.result.as_ref().unwrap()["lines"][0], "open");
        let typed = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "board",
                "data": "x",
                "snapshot": { "text": "", "extra": { "label": "open" } },
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(typed.result.as_ref().unwrap()["pending"], true);
        assert_eq!(typed.result.as_ref().unwrap()["lines"][0], "x");
        let closed = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "board",
                "data": "q",
                "snapshot": typed.result.as_ref().unwrap()["snapshot"],
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(closed.result.as_ref().unwrap(), "closed");
    }

    #[test]
    fn executes_registered_js_tool_and_records_provider() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerProvider("my-proxy", {
    baseUrl: "https://proxy.example.com",
    api: "anthropic-messages",
    models: [{ id: "demo", name: "Demo", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 128 }],
  });
  pi.registerTool({
    name: "ticket",
    description: "lookup",
    execute(_id, args) {
      return { content: [{ type: "text", text: "ticket:" + args.id }], isError: false };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(loaded.tools[0].name, "ticket");
        assert_eq!(loaded.providers[0].name, "my-proxy");
        let executed = execute_js_tool(
            &module,
            "ticket",
            &serde_json::json!({"id":"42"}),
            dir.path(),
        )
        .unwrap();
        assert_eq!(executed.content, "ticket:42");
        assert!(!executed.is_error);
    }

    #[test]
    fn mounts_js_virtual_tui_container_select_list_and_input() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Container, SelectList, Input, TUI, Text } = require("@earendil-works/pi-tui");
module.exports = (pi) => {
  pi.registerCommand("tools", {
    description: "tools",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((tui, _theme, _kb, done) => {
        const root = new Container();
        const title = new Text("tools");
        const list = new SelectList([
          { value: "read", label: "read", description: "files" },
          { value: "bash", label: "bash" },
        ], 8);
        const input = new Input();
        input.setValue("query");
        list.onSelect = (item) => done(item.value);
        root.addChild(title);
        root.addChild(list);
        root.addChild(input);
        tui.requestRender();
        return {
          extra: { selected: list.getSelectedItem() && list.getSelectedItem().value },
          handleInput(data) { list.handleInput(data); },
          render(width) { return root.render(width); },
        };
      });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let opened = run_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "tools", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(opened.ok, "{:?}", opened.error);
        let lines = opened.result.as_ref().unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>();
        assert!(lines.iter().any(|line| line.contains("tools")));
        assert!(lines.iter().any(|line| line.contains("read")));
        assert!(lines
            .iter()
            .any(|line| line.contains("> query") || line.contains("query")));
        let selected = run_js_extension(
            &module,
            "customInput",
            &serde_json::json!({
                "name": "tools",
                "data": "\r",
                "snapshot": opened.result.as_ref().unwrap()["snapshot"],
                "ctx": { "mode": "tui" }
            }),
        )
        .unwrap();
        assert_eq!(selected.result.as_ref().unwrap(), "read");
    }

    #[test]
    fn persistent_custom_tick_keeps_live_state() {
        let Some(_) = find_node() else {
            return;
        };
        let _persistent = persistent_host_guard();
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("counter", {
    description: "counter",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((tui, _theme, _kb, done) => {
        let n = 0;
        tui.requestRender();
        return {
          tick() { n += 1; },
          handleInput(data) { if (data === "q") done(n); },
          render() { return [String(n)]; },
        };
      }, { overlay: true, overlayOptions: { anchor: "center", width: 20 } });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let opened = run_persistent_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "counter", "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(opened.ok, "{:?}", opened.error);
        assert_eq!(opened.result.as_ref().unwrap()["pending"], true);
        assert_eq!(opened.result.as_ref().unwrap()["overlay"], true);
        assert_eq!(opened.result.as_ref().unwrap()["lines"][0], "0");
        let ticked = run_persistent_js_extension(
            &module,
            "customTick",
            &serde_json::json!({ "name": "counter", "width": 40 }),
        )
        .unwrap();
        assert_eq!(ticked.result.as_ref().unwrap()["lines"][0], "1");
        let again = run_persistent_js_extension(
            &module,
            "customTick",
            &serde_json::json!({ "name": "counter", "width": 40 }),
        )
        .unwrap();
        assert_eq!(again.result.as_ref().unwrap()["lines"][0], "2");
        stop_persistent_js_extension();
    }

    #[test]
    fn overlay_visible_callback_uses_terminal_size() {
        let _persistent = persistent_host_guard();
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerCommand("vis", {
    description: "vis",
    handler: async (_args, ctx) => {
      return ctx.ui.custom((_tui, _theme, _kb, done) => {
        return { render() { return ["OV"]; } };
      }, {
        overlay: true,
        overlayOptions: {
          visible: (width, height) => width >= 40 && height >= 10,
        },
      });
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let wide = run_persistent_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "vis", "width": 80, "height": 24, "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert!(wide.ok, "{:?}", wide.error);
        assert_eq!(
            wide.result.as_ref().unwrap()["overlayOptions"]["visible"],
            true
        );
        stop_persistent_js_extension();
        let narrow = run_persistent_js_extension(
            &module,
            "command",
            &serde_json::json!({ "name": "vis", "width": 20, "height": 8, "ctx": { "mode": "tui" } }),
        )
        .unwrap();
        assert_eq!(
            narrow.result.as_ref().unwrap()["overlayOptions"]["visible"],
            false
        );
        stop_persistent_js_extension();
    }

    #[test]
    fn set_header_factory_renders_text_component() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Text } = require("@earendil-works/pi-tui");
module.exports = (pi) => {
  pi.ui.setHeader(() => new Text("hello"));
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        let header = loaded
            .ui_calls
            .iter()
            .find(|call| call["op"] == "setHeader")
            .expect("setHeader");
        assert_eq!(header["factory"], true);
        assert_eq!(header["lines"][0], "hello");
    }

    #[test]
    fn register_tool_renders_call_result_and_updates() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
const { Text } = require("@earendil-works/pi-tui");
module.exports = (pi) => {
  pi.registerTool({
    name: "demo",
    description: "demo",
    executionMode: "sequential",
    renderShell: "self",
    renderCall: (args) => new Text("CALL " + args.x),
    renderResult: (result) => new Text("RESULT " + result.content),
    execute: (_id, args, _ctx, onUpdate) => {
      onUpdate({ content: "partial" });
      return { content: "done " + args.x, isError: false };
    },
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(&module, "load", &serde_json::json!({})).unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert_eq!(
            loaded.tools[0].execution_mode.as_deref(),
            Some("sequential")
        );
        assert_eq!(loaded.tools[0].render_shell.as_deref(), Some("self"));
        assert!(loaded.tools[0].has_render_call);
        assert!(loaded.tools[0].has_render_result);
        let call = render_js_tool_call(&module, "demo", &serde_json::json!({"x": "1"}), 80);
        assert_eq!(call, vec!["CALL 1".to_string()]);
        let result =
            render_js_tool_result(&module, "demo", &serde_json::json!({"content": "ok"}), 80);
        assert_eq!(result, vec!["RESULT ok".to_string()]);
        let executed =
            execute_js_tool(&module, "demo", &serde_json::json!({"x": "9"}), dir.path()).unwrap();
        assert_eq!(executed.content, "done 9");
        let updates = executed
            .details
            .as_ref()
            .and_then(|value| value.get("_piUpdates"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(updates[0]["content"], "partial");
    }

    #[test]
    fn emit_input_chains_transform_and_handles() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.on("input", async (event) => {
    if (event.text === "skip") return { action: "handled" };
    return { action: "transform", text: "T:" + event.text };
  });
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let transformed = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({"type":"input","text":"hello"}),
        )
        .unwrap();
        assert_eq!(transformed.result.as_ref().unwrap()["action"], "transform");
        assert_eq!(transformed.result.as_ref().unwrap()["text"], "T:hello");
        let handled = run_js_extension(
            &module,
            "emit",
            &serde_json::json!({"type":"input","text":"skip"}),
        )
        .unwrap();
        assert_eq!(handled.result.as_ref().unwrap()["action"], "handled");
    }

    #[test]
    fn records_theme_tools_terminal_input_and_unregister_provider() {
        let Some(_) = find_node() else {
            return;
        };
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            r#"
module.exports = (pi) => {
  pi.registerProvider("temp-proxy", {
    baseUrl: "https://proxy.example.com",
    api: "anthropic-messages",
    models: [{ id: "demo", name: "Demo", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 128 }],
  });
  pi.unregisterProvider("temp-proxy");
  pi.unregisterProvider("anthropic");
  const themes = pi.ui.getAllThemes();
  const result = pi.ui.setTheme("light");
  pi.ui.setToolsExpanded(true);
  pi.ui.onTerminalInput((data) => data === "ctrl+q");
  return {
    theme: pi.ui.getTheme(),
    toolsExpanded: pi.ui.getToolsExpanded(),
    themeCount: themes.length,
    setTheme: result,
  };
};
"#,
        )
        .unwrap();
        let module = resolve_extension_module(dir.path()).unwrap();
        let loaded = run_js_extension(
            &module,
            "load",
            &serde_json::json!({
                "themes": [{ "name": "dark" }, { "name": "light" }],
                "theme": "dark",
                "toolsExpanded": false,
            }),
        )
        .unwrap();
        assert!(loaded.ok, "{:?}", loaded.error);
        assert!(loaded.providers.is_empty());
        assert!(loaded
            .unregistered_providers
            .iter()
            .any(|name| name == "temp-proxy"));
        assert!(loaded
            .unregistered_providers
            .iter()
            .any(|name| name == "anthropic"));
        assert!(loaded
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setTheme" && call["name"] == "light"));
        assert!(loaded
            .ui_calls
            .iter()
            .any(|call| call["op"] == "setToolsExpanded" && call["expanded"] == true));
        assert!(loaded
            .ui_calls
            .iter()
            .any(|call| call["op"] == "onTerminalInput"));
        assert_eq!(loaded.terminal_input_handlers, 1);
        assert_eq!(
            loaded
                .current_theme
                .as_ref()
                .and_then(|value| value.as_str()),
            Some("light")
        );
        assert!(loaded.tools_expanded);
        let missing = run_js_extension(
            &module,
            "load",
            &serde_json::json!({
                "themes": [{ "name": "dark" }],
                "theme": "dark",
            }),
        )
        .unwrap();
        assert!(missing.ui_calls.iter().any(|call| {
            call["op"] == "setTheme"
                && call["success"] == false
                && call["error"] == "Theme not found: light"
        }));
    }
}
