//! Codex websocket / SSE transport matching
//! `vendor/pi/packages/ai/src/api/openai-codex-responses.ts`.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use base64::Engine;
use serde_json::Value;
use uuid::Uuid;

use crate::catalog::Model;
use crate::stream::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};

pub const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub const WEBSOCKET_CONNECTION_LIMIT_REACHED: &str = "websocket_connection_limit_reached";
pub const PREVIOUS_RESPONSE_NOT_FOUND: &str = "previous_response_not_found";
pub const WEBSOCKET_MESSAGE_TOO_BIG_CLOSE_CODE: u16 = 1009;
pub const WEBSOCKET_CLOSED_BEFORE_COMPLETED: &str =
    "WebSocket stream closed before response.completed";
pub const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS: u64 = 15_000;
pub const OPENAI_BETA_RESPONSES_WEBSOCKETS: &str = "responses_websockets=2026-02-06";
pub const OPENAI_BETA_RESPONSES_EXPERIMENTAL: &str = "responses=experimental";
pub const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
pub const SESSION_WEBSOCKET_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
pub const SESSION_WEBSOCKET_MAX_AGE_MS: u64 = 55 * 60 * 1000;
pub const CODEX_ORIGINATOR: &str = "pi";

/// TS `normalizeTimeoutMs(options?.websocketConnectTimeoutMs)` then default 15000.
/// Settings export `PI_WEBSOCKET_CONNECT_TIMEOUT_MS`.
pub fn resolve_websocket_connect_timeout_ms(explicit: Option<u64>) -> u64 {
    explicit
        .or_else(|| {
            std::env::var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS)
}

pub fn websocket_connect_timeout_error(timeout_ms: u64) -> String {
    format!("WebSocket connect timeout after {timeout_ms}ms")
}

/// Codex websocket connect using the TS timeout. Tests never hit ChatGPT:
/// `PI_CODEX_WS_REPLY` / localhost only.
pub fn connect_codex_websocket(url: &str, timeout_ms: u64) -> Result<(), String> {
    if let Ok(reply) = std::env::var("PI_CODEX_WS_REPLY") {
        if reply == "timeout" {
            return Err(websocket_connect_timeout_error(timeout_ms));
        }
        return Ok(());
    }
    if cfg!(test) && !url.contains("127.0.0.1") && !url.contains("localhost") {
        return Ok(());
    }
    let host_port = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url);
    let addr: std::net::SocketAddr = host_port
        .parse()
        .or_else(|_| format!("{host_port}:80").parse())
        .map_err(|err| format!("WebSocket address: {err}"))?;
    match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(timeout_ms))
    {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            Err(websocket_connect_timeout_error(timeout_ms))
        }
        Err(err) => Err(format!("WebSocket connect failed: {err}")),
    }
}

/// Map a Codex / OpenAI Responses event `type` to a pi-ai stream event name.
pub fn map_codex_event_type(event_type: &str) -> Option<&'static str> {
    match event_type {
        "response.created" => Some("start"),
        "response.output_text.delta" | "response.refusal.delta" => Some("text_delta"),
        "response.reasoning_summary_text.delta"
        | "response.reasoning_text.delta"
        | "response.reasoning_summary_part.done" => Some("thinking_delta"),
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            Some("toolcall_delta")
        }
        "response.done" | "response.completed" | "response.incomplete" => Some("done"),
        "error" => Some("error"),
        _ => None,
    }
}

pub fn normalize_codex_terminal_event(event_type: &str) -> &str {
    match event_type {
        "response.done" | "response.completed" | "response.incomplete" => "response.completed",
        other => other,
    }
}

pub fn is_websocket_connection_limit_reached(error: &str) -> bool {
    error.contains(WEBSOCKET_CONNECTION_LIMIT_REACHED)
}

pub fn is_previous_response_not_found(error: &str) -> bool {
    error.contains(PREVIOUS_RESPONSE_NOT_FOUND)
}

/// TS retries websocket once on connection-limit, then falls back to SSE
/// only when no websocket message stream has started.
pub fn should_fallback_to_sse(error: &str, websocket_started: bool) -> bool {
    !websocket_started && is_websocket_connection_limit_reached(error)
}

pub fn should_retry_websocket_connection_limit(error: &str, already_retried: bool) -> bool {
    !already_retried && is_websocket_connection_limit_reached(error)
}

pub fn should_retry_missing_previous_response(error: &str, already_retried: bool) -> bool {
    !already_retried && is_previous_response_not_found(error)
}

pub fn websocket_idle_timeout_error(timeout_ms: u64) -> String {
    format!("WebSocket idle timeout after {timeout_ms}ms")
}

/// TS `resolveCodexUrl`.
pub fn resolve_codex_url(base_url: Option<&str>) -> String {
    let raw = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_BASE_URL);
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

/// TS `resolveCodexWebSocketUrl`.
pub fn resolve_codex_websocket_url(base_url: Option<&str>) -> String {
    let http = resolve_codex_url(base_url);
    if let Some(rest) = http.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = http.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        http
    }
}

pub fn pi_user_agent() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("pi ({platform} {}; {arch})", os_release())
}

fn os_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn decode_jwt_part(part: &str) -> Result<Vec<u8>, String> {
    let mut padded = part.replace('-', "+").replace('_', "/");
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .map_err(|_| "Failed to extract accountId from token".into())
}

/// TS `extractAccountId` from a ChatGPT Codex JWT.
pub fn extract_account_id(token: &str) -> Result<String, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Failed to extract accountId from token".into());
    }
    let payload = decode_jwt_part(parts[1])?;
    let value: Value = serde_json::from_slice(&payload)
        .map_err(|_| "Failed to extract accountId from token".to_string())?;
    value
        .get(JWT_CLAIM_PATH)
        .and_then(|claim| claim.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Failed to extract accountId from token".into())
}

fn header_key_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn upsert_header(headers: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
    if let Some(existing) = headers
        .iter_mut()
        .find(|(name, _)| header_key_eq(name, key))
    {
        existing.0 = key.to_string();
        existing.1 = value.into();
        return;
    }
    headers.push((key.to_string(), value.into()));
}

fn delete_header(headers: &mut Vec<(String, String)>, key: &str) {
    headers.retain(|(name, _)| !header_key_eq(name, key));
}

pub fn build_base_codex_headers<'a, I>(
    model_headers: I,
    additional: &[(String, String)],
    account_id: &str,
    token: &str,
) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut headers = Vec::new();
    for (key, value) in model_headers {
        upsert_header(&mut headers, key, value.clone());
    }
    for (key, value) in additional {
        upsert_header(&mut headers, key, value.clone());
    }
    upsert_header(&mut headers, "Authorization", format!("Bearer {token}"));
    upsert_header(&mut headers, "chatgpt-account-id", account_id);
    upsert_header(&mut headers, "originator", CODEX_ORIGINATOR);
    upsert_header(&mut headers, "User-Agent", pi_user_agent());
    headers
}

pub fn build_sse_headers<'a, I>(
    model_headers: I,
    additional: &[(String, String)],
    account_id: &str,
    token: &str,
    session_id: Option<&str>,
) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut headers = build_base_codex_headers(model_headers, additional, account_id, token);
    upsert_header(
        &mut headers,
        "OpenAI-Beta",
        OPENAI_BETA_RESPONSES_EXPERIMENTAL,
    );
    upsert_header(&mut headers, "accept", "text/event-stream");
    upsert_header(&mut headers, "content-type", "application/json");
    if let Some(session_id) = session_id.filter(|id| !id.is_empty()) {
        upsert_header(&mut headers, "session-id", session_id);
        upsert_header(&mut headers, "x-client-request-id", session_id);
    }
    headers
}

pub fn build_websocket_headers<'a, I>(
    model_headers: I,
    additional: &[(String, String)],
    account_id: &str,
    token: &str,
    request_id: &str,
) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut headers = build_base_codex_headers(model_headers, additional, account_id, token);
    delete_header(&mut headers, "accept");
    delete_header(&mut headers, "content-type");
    delete_header(&mut headers, "OpenAI-Beta");
    upsert_header(
        &mut headers,
        "OpenAI-Beta",
        OPENAI_BETA_RESPONSES_WEBSOCKETS,
    );
    upsert_header(&mut headers, "x-client-request-id", request_id);
    upsert_header(&mut headers, "session-id", request_id);
    headers
}

/// Handshake headers match TS `connectWebSocket`: `OpenAI-Beta` is stripped.
pub fn websocket_handshake_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(key, _)| !header_key_eq(key, "OpenAI-Beta"))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedWebSocketContinuation {
    pub last_request_body: Value,
    pub last_response_id: String,
    pub last_response_items: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenAICodexWebSocketDebugStats {
    pub requests: u64,
    pub connections_created: u64,
    pub connections_reused: u64,
    pub cached_context_requests: u64,
    pub full_context_requests: u64,
    pub delta_requests: u64,
    pub store_true_requests: u64,
    pub last_input_items: u64,
    pub last_delta_input_items: Option<u64>,
    pub last_previous_response_id: Option<String>,
    pub websocket_failures: u64,
    pub websocket_fallback_active: bool,
    pub last_websocket_error: Option<String>,
}

#[derive(Debug)]
struct CachedWebSocketConnection {
    created_at: Instant,
    last_used: Instant,
    continuation: Option<CachedWebSocketContinuation>,
}

struct CodexWebSocketState {
    sessions: HashMap<String, HashMap<String, CachedWebSocketConnection>>,
    stats: HashMap<String, OpenAICodexWebSocketDebugStats>,
    sse_fallback: HashSet<String>,
}

fn websocket_state() -> &'static Mutex<CodexWebSocketState> {
    static STATE: OnceLock<Mutex<CodexWebSocketState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(CodexWebSocketState {
            sessions: HashMap::new(),
            stats: HashMap::new(),
            sse_fallback: HashSet::new(),
        })
    })
}

fn lock_state() -> std::sync::MutexGuard<'static, CodexWebSocketState> {
    websocket_state()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

pub fn get_openai_codex_websocket_debug_stats(
    session_id: &str,
) -> Option<OpenAICodexWebSocketDebugStats> {
    lock_state().stats.get(session_id).cloned()
}

pub fn reset_openai_codex_websocket_debug_stats(session_id: Option<&str>) {
    let mut state = lock_state();
    if let Some(session_id) = session_id {
        state.stats.remove(session_id);
        state.sse_fallback.remove(session_id);
        return;
    }
    state.stats.clear();
    state.sse_fallback.clear();
}

pub fn close_openai_codex_websocket_sessions(session_id: Option<&str>) {
    let mut state = lock_state();
    if let Some(session_id) = session_id {
        state.sessions.remove(session_id);
        return;
    }
    state.sessions.clear();
}

pub fn is_websocket_sse_fallback_active(session_id: Option<&str>) -> bool {
    session_id
        .map(|id| lock_state().sse_fallback.contains(id))
        .unwrap_or(false)
}

pub fn record_websocket_sse_fallback(session_id: Option<&str>) {
    let Some(session_id) = session_id else {
        return;
    };
    let mut state = lock_state();
    state.sse_fallback.insert(session_id.to_string());
    let stats = state.stats.entry(session_id.to_string()).or_default();
    stats.websocket_fallback_active = true;
}

pub fn record_websocket_failure(session_id: Option<&str>, error: &str) {
    let Some(session_id) = session_id else {
        return;
    };
    let mut state = lock_state();
    state.sse_fallback.insert(session_id.to_string());
    let stats = state.stats.entry(session_id.to_string()).or_default();
    stats.websocket_failures += 1;
    stats.last_websocket_error = Some(error.to_string());
    stats.websocket_fallback_active = true;
}

fn get_or_create_stats<'a>(
    state: &'a mut CodexWebSocketState,
    session_id: &str,
) -> &'a mut OpenAICodexWebSocketDebugStats {
    state.stats.entry(session_id.to_string()).or_default()
}

fn is_session_expired(entry: &CachedWebSocketConnection, now: Instant) -> bool {
    now.duration_since(entry.last_used).as_millis() as u64 >= SESSION_WEBSOCKET_CACHE_TTL_MS
        || now.duration_since(entry.created_at).as_millis() as u64 >= SESSION_WEBSOCKET_MAX_AGE_MS
}

pub fn acquire_cached_continuation(
    session_id: Option<&str>,
    account_id: &str,
    now: Instant,
) -> (bool, Option<CachedWebSocketContinuation>) {
    let Some(session_id) = session_id else {
        return (false, None);
    };
    let mut state = lock_state();
    let Some(account_entries) = state.sessions.get_mut(session_id) else {
        return (false, None);
    };
    let Some(entry) = account_entries.get_mut(account_id) else {
        return (false, None);
    };
    if is_session_expired(entry, now) {
        account_entries.remove(account_id);
        if account_entries.is_empty() {
            state.sessions.remove(session_id);
        }
        return (false, None);
    }
    entry.last_used = now;
    (true, entry.continuation.clone())
}

pub fn store_cached_continuation(
    session_id: Option<&str>,
    account_id: &str,
    continuation: CachedWebSocketContinuation,
    reused: bool,
    now: Instant,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let mut state = lock_state();
    let account_entries = state.sessions.entry(session_id.to_string()).or_default();
    if let Some(entry) = account_entries.get_mut(account_id) {
        entry.last_used = now;
        entry.continuation = Some(continuation);
        let _ = reused;
        return;
    }
    account_entries.insert(
        account_id.to_string(),
        CachedWebSocketConnection {
            created_at: now,
            last_used: now,
            continuation: Some(continuation),
        },
    );
}

pub fn clear_cached_continuation(session_id: Option<&str>, account_id: &str) {
    let Some(session_id) = session_id else {
        return;
    };
    if let Some(account_entries) = lock_state().sessions.get_mut(session_id) {
        if let Some(entry) = account_entries.get_mut(account_id) {
            entry.continuation = None;
        }
    }
}

fn input_len(body: &Value) -> u64 {
    body.get("input")
        .and_then(Value::as_array)
        .map(|items| items.len() as u64)
        .unwrap_or(0)
}

pub fn record_websocket_request_stats(
    session_id: Option<&str>,
    reused: bool,
    use_cached_context: bool,
    request_body: &Value,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let mut state = lock_state();
    let stats = get_or_create_stats(&mut state, session_id);
    stats.requests += 1;
    if reused {
        stats.connections_reused += 1;
    } else {
        stats.connections_created += 1;
    }
    if use_cached_context {
        stats.cached_context_requests += 1;
    }
    if request_body.get("store") == Some(&Value::Bool(true)) {
        stats.store_true_requests += 1;
    }
    stats.last_input_items = input_len(request_body);
    if let Some(previous) = request_body
        .get("previous_response_id")
        .and_then(Value::as_str)
    {
        stats.delta_requests += 1;
        stats.last_delta_input_items = Some(input_len(request_body));
        stats.last_previous_response_id = Some(previous.to_string());
    } else {
        stats.full_context_requests += 1;
        stats.last_delta_input_items = None;
        stats.last_previous_response_id = None;
    }
}

fn request_body_without_input(body: &Value) -> Value {
    let mut clone = body.clone();
    if let Value::Object(map) = &mut clone {
        map.remove("input");
        map.remove("previous_response_id");
    }
    clone
}

fn response_inputs_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    let empty = Value::Array(Vec::new());
    left.unwrap_or(&empty) == right.unwrap_or(&empty)
}

pub fn get_cached_websocket_input_delta(
    body: &Value,
    continuation: &CachedWebSocketContinuation,
) -> Option<Value> {
    if request_body_without_input(body)
        != request_body_without_input(&continuation.last_request_body)
    {
        return None;
    }
    let current = body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut baseline = continuation
        .last_request_body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(items) = continuation.last_response_items.as_array() {
        baseline.extend(items.iter().cloned());
    }
    if current.len() < baseline.len() {
        return None;
    }
    let prefix_len = baseline.len();
    if !response_inputs_equal(
        Some(&Value::Array(current[..prefix_len].to_vec())),
        Some(&Value::Array(baseline)),
    ) {
        return None;
    }
    Some(Value::Array(current[prefix_len..].to_vec()))
}

pub fn build_cached_websocket_request_body(
    body: &Value,
    continuation: Option<&CachedWebSocketContinuation>,
) -> (Value, bool) {
    let Some(continuation) = continuation else {
        return (body.clone(), false);
    };
    let Some(delta) = get_cached_websocket_input_delta(body, continuation) else {
        return (body.clone(), false);
    };
    if continuation.last_response_id.is_empty() {
        return (body.clone(), false);
    }
    let mut next = body.clone();
    if let Value::Object(map) = &mut next {
        map.insert(
            "previous_response_id".into(),
            Value::String(continuation.last_response_id.clone()),
        );
        map.insert("input".into(), delta);
    }
    (next, true)
}

pub fn cache_session_id<'a>(
    session_id: Option<&'a str>,
    cache_retention: Option<&str>,
) -> Option<&'a str> {
    if cache_retention == Some("none") {
        None
    } else {
        session_id.filter(|id| !id.is_empty())
    }
}

pub fn use_cached_websocket_context(transport: Option<&str>) -> bool {
    matches!(transport, None | Some("auto") | Some("websocket-cached"))
}

#[derive(Debug)]
pub enum CodexWebsocketOutcome {
    Message(Box<AssistantMessage>),
    FallbackToSse,
}

/// TS websocket attempt + one connection-limit retry + one missing-continuation
/// retry, then SSE fallback when the stream never started.
#[allow(clippy::too_many_arguments)]
pub fn try_codex_websocket_transport(
    model: &Model,
    body: &Value,
    token: &str,
    options_transport: Option<&str>,
    session_id: Option<&str>,
    cache_retention: Option<&str>,
    websocket_connect_timeout_ms: Option<u64>,
    idle_timeout_ms: Option<u64>,
) -> Result<CodexWebsocketOutcome, String> {
    if options_transport == Some("sse") {
        return Ok(CodexWebsocketOutcome::FallbackToSse);
    }
    let cache_id = cache_session_id(session_id, cache_retention).map(str::to_string);
    if is_websocket_sse_fallback_active(cache_id.as_deref()) {
        record_websocket_sse_fallback(cache_id.as_deref());
        return Ok(CodexWebsocketOutcome::FallbackToSse);
    }
    let account_id = extract_account_id(token)?;
    let request_id = cache_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let headers = build_websocket_headers(&model.headers, &[], &account_id, token, &request_id);
    let timeout = resolve_websocket_connect_timeout_ms(websocket_connect_timeout_ms);
    let ws_url = std::env::var("PI_CODEX_WS_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| resolve_codex_websocket_url(model.base_url.as_deref()));
    let use_cached = use_cached_websocket_context(options_transport);
    let mut retried_limit = false;
    let mut retried_missing = false;
    loop {
        let mut started = false;
        match crate::codex_ws::process_codex_websocket(
            &ws_url,
            body,
            &headers,
            model,
            timeout,
            idle_timeout_ms,
            cache_id.as_deref(),
            &account_id,
            use_cached,
            &mut started,
        ) {
            Ok(message) => return Ok(CodexWebsocketOutcome::Message(Box::new(message))),
            Err(error) => {
                if should_retry_missing_previous_response(&error, retried_missing) {
                    retried_missing = true;
                    clear_cached_continuation(cache_id.as_deref(), &account_id);
                    continue;
                }
                if !started && should_retry_websocket_connection_limit(&error, retried_limit) {
                    retried_limit = true;
                    continue;
                }
                record_websocket_failure(cache_id.as_deref(), &error);
                if started {
                    return Err(error);
                }
                record_websocket_sse_fallback(cache_id.as_deref());
                return Ok(CodexWebsocketOutcome::FallbackToSse);
            }
        }
    }
}

fn event_delta(value: &Value) -> String {
    value
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Replay a Codex Responses JSON/SSE fixture into pi-ai assistant events.
/// Never opens a websocket or hits the network.
pub fn replay_codex_events(model: &Model, corpus: &str) -> Vec<AssistantMessageEvent> {
    let mut message = AssistantMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: Vec::new(),
        model: format!("{}/{}", model.provider, model.id),
        usage: None,
        stop_reason: None,
        error_message: None,
    };
    let mut events = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut started = false;
    let mut text_started = false;
    let mut thinking_started = false;

    for raw_event in corpus_events(corpus) {
        let event_type = raw_event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mapped = map_codex_event_type(event_type);
        match mapped {
            Some("start") => {
                if !started {
                    events.push(AssistantMessageEvent::Start {
                        partial: message.clone(),
                    });
                    started = true;
                }
            }
            Some("text_delta") => {
                let delta = event_delta(&raw_event);
                if !started {
                    events.push(AssistantMessageEvent::Start {
                        partial: message.clone(),
                    });
                    started = true;
                }
                if !text_started {
                    events.push(AssistantMessageEvent::TextStart {
                        content_index: 0,
                        partial: message.clone(),
                    });
                    text_started = true;
                }
                text.push_str(&delta);
                if let Some(ContentBlock::Text { text: existing }) = message.content.get_mut(0) {
                    existing.push_str(&delta);
                } else {
                    message.content.insert(
                        0,
                        ContentBlock::Text {
                            text: delta.clone(),
                        },
                    );
                }
                events.push(AssistantMessageEvent::TextDelta {
                    content_index: 0,
                    delta,
                    partial: message.clone(),
                });
            }
            Some("thinking_delta") => {
                let delta = if event_type == "response.reasoning_summary_part.done" {
                    "\n\n".into()
                } else {
                    event_delta(&raw_event)
                };
                if !thinking_started {
                    events.push(AssistantMessageEvent::ThinkingStart {
                        content_index: if text_started { 1 } else { 0 },
                        partial: message.clone(),
                    });
                    thinking_started = true;
                }
                thinking.push_str(&delta);
                events.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: if text_started { 1 } else { 0 },
                    delta,
                    partial: message.clone(),
                });
            }
            Some("toolcall_delta") => {
                let delta = raw_event
                    .get("delta")
                    .and_then(Value::as_str)
                    .or_else(|| raw_event.get("arguments").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                events.push(AssistantMessageEvent::ToolcallDelta {
                    content_index: message.content.len(),
                    delta,
                    partial: message.clone(),
                });
            }
            Some("done") => {
                if text_started {
                    events.push(AssistantMessageEvent::TextEnd {
                        content_index: 0,
                        content: text.clone(),
                        partial: message.clone(),
                    });
                }
                if thinking_started {
                    events.push(AssistantMessageEvent::ThinkingEnd {
                        content_index: if text_started { 1 } else { 0 },
                        content: thinking.clone(),
                        partial: message.clone(),
                    });
                }
                message.stop_reason = Some(StopReason::Stop);
                events.push(AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: message.clone(),
                });
            }
            Some("error") => {
                message.stop_reason = Some(StopReason::Error);
                message.error_message = raw_event
                    .pointer("/error/code")
                    .and_then(Value::as_str)
                    .or_else(|| raw_event.get("code").and_then(Value::as_str))
                    .map(str::to_string);
                events.push(AssistantMessageEvent::Error {
                    reason: StopReason::Error,
                    error: message.clone(),
                });
            }
            _ => {}
        }
    }
    if !events
        .iter()
        .any(|event| matches!(event, AssistantMessageEvent::Done { .. }))
    {
        message.stop_reason = Some(StopReason::Stop);
        events.push(AssistantMessageEvent::Done {
            reason: StopReason::Stop,
            message,
        });
    }
    events
}

fn corpus_events(corpus: &str) -> Vec<Value> {
    let trimmed = corpus.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).unwrap_or_default();
    }
    let mut events = Vec::new();
    if trimmed.contains("data:") {
        for block in trimmed.split("\n\n") {
            let mut data = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(rest.trim_start());
                }
            }
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                events.push(value);
            }
        }
        return events;
    }
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            events.push(value);
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn model() -> Model {
        Model {
            id: "gpt-5".into(),
            name: "gpt-5".into(),
            api: "openai-codex-responses".into(),
            provider: "openai-codex".into(),
            base_url: Some(DEFAULT_CODEX_BASE_URL.into()),
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 1,
            max_tokens: 1,
            compat: Value::Null,
            headers: Default::default(),
        }
    }

    #[test]
    fn maps_ts_event_names_and_sse_fallback() {
        assert_eq!(map_codex_event_type("response.created"), Some("start"));
        assert_eq!(
            map_codex_event_type("response.output_text.delta"),
            Some("text_delta")
        );
        assert_eq!(map_codex_event_type("response.completed"), Some("done"));
        assert_eq!(
            normalize_codex_terminal_event("response.done"),
            "response.completed"
        );
        assert!(should_fallback_to_sse(
            "error: websocket_connection_limit_reached",
            false
        ));
        assert!(!should_fallback_to_sse(
            "error: websocket_connection_limit_reached",
            true
        ));
        assert!(should_retry_missing_previous_response(
            PREVIOUS_RESPONSE_NOT_FOUND,
            false
        ));
        assert_eq!(WEBSOCKET_MESSAGE_TOO_BIG_CLOSE_CODE, 1009);
        assert_eq!(
            WEBSOCKET_CLOSED_BEFORE_COMPLETED,
            "WebSocket stream closed before response.completed"
        );
    }

    #[test]
    fn websocket_connect_timeout_uses_explicit_then_env_then_default() {
        let _guard = lock_env();
        let previous = std::env::var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS").ok();
        std::env::remove_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS");
        assert_eq!(
            resolve_websocket_connect_timeout_ms(None),
            DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS
        );
        assert_eq!(resolve_websocket_connect_timeout_ms(Some(2500)), 2500);
        std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", "3200");
        assert_eq!(resolve_websocket_connect_timeout_ms(None), 3200);
        assert_eq!(resolve_websocket_connect_timeout_ms(Some(900)), 900);
        match previous {
            Some(value) => std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", value),
            None => std::env::remove_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS"),
        }
    }

    #[test]
    fn websocket_connect_fixture_timeout_uses_resolved_ms() {
        let _guard = lock_env();
        let previous = std::env::var("PI_CODEX_WS_REPLY").ok();
        std::env::set_var("PI_CODEX_WS_REPLY", "timeout");
        let error = connect_codex_websocket("wss://chatgpt.com/backend-api", 1234).unwrap_err();
        assert_eq!(error, websocket_connect_timeout_error(1234));
        match previous {
            Some(value) => std::env::set_var("PI_CODEX_WS_REPLY", value),
            None => std::env::remove_var("PI_CODEX_WS_REPLY"),
        }
    }

    #[test]
    fn replays_codex_sse_fixture_without_network() {
        let corpus = r#"
data: {"type":"response.created","response":{"id":"resp_1"}}

data: {"type":"response.output_text.delta","output_index":0,"delta":"Hello"}

data: {"type":"response.output_text.delta","output_index":0,"delta":" Codex"}

data: {"type":"response.completed","response":{"status":"completed"}}
"#;
        let events = replay_codex_events(&model(), corpus);
        let names: Vec<&str> = events
            .iter()
            .map(|event| match event {
                AssistantMessageEvent::Start { .. } => "start",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::Done { .. } => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(
            names,
            [
                "start",
                "text_start",
                "text_delta",
                "text_delta",
                "text_end",
                "done"
            ]
        );
        let done = events.last().unwrap();
        match done {
            AssistantMessageEvent::Done { message, .. } => {
                assert_eq!(
                    match &message.content[0] {
                        ContentBlock::Text { text } => text.as_str(),
                        _ => "",
                    },
                    "Hello Codex"
                );
            }
            _ => panic!("expected done"),
        }
    }

    fn mock_token(account_id: &str) -> String {
        let payload = base64::engine::general_purpose::STANDARD.encode(
            serde_json::json!({ JWT_CLAIM_PATH: { "chatgpt_account_id": account_id } }).to_string(),
        );
        format!("aaa.{payload}.bbb")
    }

    #[test]
    fn resolve_codex_urls_match_ts() {
        assert_eq!(
            resolve_codex_url(None),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_url(Some("https://chatgpt.com/backend-api")),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_url(Some("https://chatgpt.com/backend-api/codex")),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_url(Some("https://chatgpt.com/backend-api/codex/responses")),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_websocket_url(Some("https://chatgpt.com/backend-api")),
            "wss://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_websocket_url(Some("http://127.0.0.1:9/backend-api")),
            "ws://127.0.0.1:9/backend-api/codex/responses"
        );
    }

    #[test]
    fn extract_account_id_and_headers_match_ts() {
        let token = mock_token("acc_test");
        assert_eq!(extract_account_id(&token).unwrap(), "acc_test");
        assert_eq!(
            extract_account_id("not-a-jwt").unwrap_err(),
            "Failed to extract accountId from token"
        );
        let ws = build_websocket_headers(&model().headers, &[], "acc_test", &token, "session-auto");
        let get = |key: &str| {
            ws.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str())
        };
        let expected_auth = format!("Bearer {token}");
        assert_eq!(get("Authorization"), Some(expected_auth.as_str()));
        assert_eq!(get("chatgpt-account-id"), Some("acc_test"));
        assert_eq!(get("originator"), Some("pi"));
        assert_eq!(get("OpenAI-Beta"), Some(OPENAI_BETA_RESPONSES_WEBSOCKETS));
        assert_eq!(get("session-id"), Some("session-auto"));
        assert_eq!(get("x-client-request-id"), Some("session-auto"));
        assert!(get("accept").is_none());
        assert!(get("content-type").is_none());
        assert!(pi_user_agent().starts_with("pi ("));
        let handshake = websocket_handshake_headers(&ws);
        assert!(handshake
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("OpenAI-Beta")));
        let sse = build_sse_headers(&model().headers, &[], "acc_test", &token, Some("sid"));
        let sse_get = |key: &str| {
            sse.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(
            sse_get("OpenAI-Beta"),
            Some(OPENAI_BETA_RESPONSES_EXPERIMENTAL)
        );
        assert_eq!(sse_get("accept"), Some("text/event-stream"));
    }

    #[test]
    fn cached_websocket_sends_input_delta() {
        let first = serde_json::json!({
            "model": "gpt-5",
            "input": [{"role":"user","content":"hi"}]
        });
        let second = serde_json::json!({
            "model": "gpt-5",
            "input": [
                {"role":"user","content":"hi"},
                {"role":"assistant","content":"hello"},
                {"role":"user","content":"again"}
            ]
        });
        let continuation = CachedWebSocketContinuation {
            last_request_body: first.clone(),
            last_response_id: "resp_1".into(),
            last_response_items: serde_json::json!([{"role":"assistant","content":"hello"}]),
        };
        let (delta_body, used) = build_cached_websocket_request_body(&second, Some(&continuation));
        assert!(used);
        assert_eq!(delta_body["previous_response_id"], "resp_1");
        assert_eq!(
            delta_body["input"],
            serde_json::json!([{"role":"user","content":"again"}])
        );
        let mismatched = serde_json::json!({
            "model": "other",
            "input": second["input"]
        });
        let (full, used) = build_cached_websocket_request_body(&mismatched, Some(&continuation));
        assert!(!used);
        assert_eq!(full["model"], "other");
        assert!(full.get("previous_response_id").is_none());
    }

    #[test]
    fn fixture_websocket_records_stats_and_falls_back() {
        let _guard = lock_env();
        reset_openai_codex_websocket_debug_stats(None);
        close_openai_codex_websocket_sessions(None);
        let previous = std::env::var("PI_CODEX_WS_REPLY").ok();
        std::env::set_var("PI_CODEX_WS_REPLY", "timeout");
        let token = mock_token("acc_test");
        let outcome = try_codex_websocket_transport(
            &model(),
            &serde_json::json!({"model":"gpt-5","input":[]}),
            &token,
            Some("auto"),
            Some("ws-connect-timeout"),
            None,
            Some(50),
            Some(50),
        )
        .unwrap();
        assert!(matches!(outcome, CodexWebsocketOutcome::FallbackToSse));
        let stats = get_openai_codex_websocket_debug_stats("ws-connect-timeout").unwrap();
        assert_eq!(stats.websocket_failures, 1);
        assert!(stats.websocket_fallback_active);
        assert_eq!(
            stats.last_websocket_error.as_deref(),
            Some(websocket_connect_timeout_error(50).as_str())
        );
        std::env::set_var(
            "PI_CODEX_WS_REPLY",
            r#"{"type":"response.created"}
{"type":"response.output_text.delta","delta":"Hi"}
{"type":"response.completed","response":{"id":"resp_fixture","status":"completed"}}"#,
        );
        reset_openai_codex_websocket_debug_stats(Some("session-auto"));
        let outcome = try_codex_websocket_transport(
            &model(),
            &serde_json::json!({"model":"gpt-5","input":[{"role":"user","content":"a"}]}),
            &token,
            Some("auto"),
            Some("session-auto"),
            None,
            Some(50),
            None,
        )
        .unwrap();
        match outcome {
            CodexWebsocketOutcome::Message(message) => {
                assert_eq!(
                    match &message.content[0] {
                        ContentBlock::Text { text } => text.as_str(),
                        _ => "",
                    },
                    "Hi"
                );
            }
            CodexWebsocketOutcome::FallbackToSse => panic!("expected websocket message"),
        }
        let stats = get_openai_codex_websocket_debug_stats("session-auto").unwrap();
        assert_eq!(stats.requests, 1);
        assert_eq!(stats.connections_created, 1);
        assert_eq!(stats.cached_context_requests, 1);
        assert_eq!(stats.full_context_requests, 1);
        match previous {
            Some(value) => std::env::set_var("PI_CODEX_WS_REPLY", value),
            None => std::env::remove_var("PI_CODEX_WS_REPLY"),
        }
        reset_openai_codex_websocket_debug_stats(None);
        close_openai_codex_websocket_sessions(None);
    }

    #[test]
    fn localhost_websocket_handshake_and_response_create() {
        let _guard = lock_env();
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        reset_openai_codex_websocket_debug_stats(None);
        close_openai_codex_websocket_sessions(None);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.contains("Upgrade: websocket"));
            assert!(request.contains("originator: pi"));
            assert!(request.contains("chatgpt-account-id: acc_live"));
            assert!(!request.to_ascii_lowercase().contains("openai-beta"));
            let key = request
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("sec-websocket-key")
                            .then_some(value.trim().to_string())
                    })
                })
                .unwrap();
            let accept = crate::codex_ws::accept_key_for_tests(&key);
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
            let mut frame = [0u8; 2];
            stream.read_exact(&mut frame).unwrap();
            let mut len = (frame[1] & 0x7f) as usize;
            if len == 126 {
                let mut ext = [0u8; 2];
                stream.read_exact(&mut ext).unwrap();
                len = u16::from_be_bytes(ext) as usize;
            }
            let mut mask = [0u8; 4];
            stream.read_exact(&mut mask).unwrap();
            let mut payload = vec![0u8; len];
            stream.read_exact(&mut payload).unwrap();
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
            let body: Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(body["type"], "response.create");
            assert_eq!(body["model"], "gpt-5");
            crate::codex_ws::write_unmasked_text(
                &mut stream,
                r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
            )
            .unwrap();
            crate::codex_ws::write_unmasked_text(
                &mut stream,
                r#"{"type":"response.output_text.delta","delta":"Pong"}"#,
            )
            .unwrap();
            crate::codex_ws::write_unmasked_text(
                &mut stream,
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed"}}"#,
            )
            .unwrap();
        });

        let previous_reply = std::env::var("PI_CODEX_WS_REPLY").ok();
        let previous_url = std::env::var("PI_CODEX_WS_URL").ok();
        std::env::remove_var("PI_CODEX_WS_REPLY");
        std::env::set_var("PI_CODEX_WS_URL", format!("ws://{addr}/codex/responses"));
        let token = mock_token("acc_live");
        let outcome = try_codex_websocket_transport(
            &model(),
            &serde_json::json!({"model":"gpt-5","input":[{"role":"user","content":"ping"}]}),
            &token,
            Some("websocket"),
            Some("loopback"),
            None,
            Some(2000),
            Some(2000),
        )
        .unwrap();
        match outcome {
            CodexWebsocketOutcome::Message(message) => {
                assert_eq!(
                    match &message.content[0] {
                        ContentBlock::Text { text } => text.as_str(),
                        _ => "",
                    },
                    "Pong"
                );
            }
            CodexWebsocketOutcome::FallbackToSse => panic!("expected loopback websocket"),
        }
        server.join().unwrap();
        match previous_reply {
            Some(value) => std::env::set_var("PI_CODEX_WS_REPLY", value),
            None => std::env::remove_var("PI_CODEX_WS_REPLY"),
        }
        match previous_url {
            Some(value) => std::env::set_var("PI_CODEX_WS_URL", value),
            None => std::env::remove_var("PI_CODEX_WS_URL"),
        }
        reset_openai_codex_websocket_debug_stats(None);
        close_openai_codex_websocket_sessions(None);
    }
}
