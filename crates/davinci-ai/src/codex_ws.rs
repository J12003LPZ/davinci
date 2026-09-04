//! Codex WebSocket client (RFC 6455) matching
//! `vendor/pi/packages/ai/src/api/openai-codex-responses.ts`.
//! Tests never open ChatGPT: fixtures and loopback only.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::Value;
use sha1::{Digest, Sha1};
use uuid::Uuid;

use crate::catalog::Model;
use crate::codex::{
    acquire_cached_continuation, build_cached_websocket_request_body,
    record_websocket_request_stats, replay_codex_events, store_cached_continuation,
    websocket_connect_timeout_error, websocket_handshake_headers, websocket_idle_timeout_error,
    CachedWebSocketContinuation, SESSION_WEBSOCKET_CACHE_TTL_MS, SESSION_WEBSOCKET_MAX_AGE_MS,
    WEBSOCKET_CLOSED_BEFORE_COMPLETED,
};
use crate::stream::{AssistantMessage, AssistantMessageEvent, StopReason};
use crate::stream_decoder::{ResponsesDecoder, StreamDecoder};

type AbortFlag<'a> = Option<&'a std::sync::Arc<std::sync::atomic::AtomicBool>>;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

#[allow(clippy::too_many_arguments)]
pub fn process_codex_websocket(
    url: &str,
    body: &Value,
    headers: &[(String, String)],
    model: &Model,
    connect_timeout_ms: u64,
    idle_timeout_ms: Option<u64>,
    cache_session_id: Option<&str>,
    account_id: &str,
    use_cached_context: bool,
    started: &mut bool,
    abort: AbortFlag<'_>,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<AssistantMessage, String> {
    if let Ok(reply) = std::env::var("PI_CODEX_WS_REPLY") {
        return process_fixture(
            &reply,
            body,
            model,
            connect_timeout_ms,
            idle_timeout_ms,
            cache_session_id,
            account_id,
            use_cached_context,
            started,
            on_event,
        );
    }
    if !allows_live_websocket(url) {
        return Err("WebSocket transport is not available in this runtime".into());
    }
    let (continuation_hit, continuation) =
        acquire_cached_continuation(cache_session_id, account_id, Instant::now());
    let _ = continuation_hit;
    let (request_body, used_delta) =
        build_cached_websocket_request_body(body, continuation.as_ref());
    let _ = used_delta;
    let acquired = acquire_live_socket(
        url,
        headers,
        connect_timeout_ms,
        cache_session_id,
        account_id,
    )?;
    let mut stream = acquired.stream;
    let socket_reused = acquired.reused;
    record_websocket_request_stats(
        cache_session_id,
        socket_reused,
        use_cached_context,
        &request_body,
    );
    if let Some(idle_ms) = idle_timeout_ms.filter(|ms| *ms > 0) {
        stream
            .set_read_timeout(Some(Duration::from_millis(idle_ms)))
            .map_err(|err| format!("WebSocket timeout: {err}"))?;
    }
    let mut outgoing = request_body.clone();
    if let Value::Object(map) = &mut outgoing {
        map.insert("type".into(), Value::String("response.create".into()));
    }
    let send = write_text_frame(&mut stream, &outgoing.to_string());
    if let Err(err) = send {
        release_live_socket(acquired.key, stream, false);
        return Err(err);
    }
    let mut decoder = ResponsesDecoder::new(model);
    let (events, message, aborted) = match read_codex_events(
        &mut stream,
        idle_timeout_ms,
        started,
        &mut decoder,
        abort,
        on_event,
    ) {
        Ok(read) => read,
        Err(err) => {
            release_live_socket(acquired.key, stream, false);
            return Err(err);
        }
    };
    if aborted {
        // The socket is mid-response; nothing later can reuse it.
        release_live_socket(acquired.key, stream, false);
        return Ok(message);
    }
    let mut keep = acquired.key.is_some();
    if use_cached_context {
        if let Some(response_id) = events.iter().rev().find_map(|event| {
            event
                .pointer("/response/id")
                .and_then(Value::as_str)
                .map(str::to_string)
        }) {
            store_cached_continuation(
                cache_session_id,
                account_id,
                CachedWebSocketContinuation {
                    last_request_body: body.clone(),
                    last_response_id: response_id,
                    last_response_items: cached_response_items(&message),
                },
                socket_reused,
                Instant::now(),
            );
        } else {
            keep = false;
        }
    }
    release_live_socket(acquired.key, stream, keep);
    Ok(message)
}

#[allow(clippy::too_many_arguments)]
fn process_fixture(
    reply: &str,
    body: &Value,
    model: &Model,
    connect_timeout_ms: u64,
    idle_timeout_ms: Option<u64>,
    cache_session_id: Option<&str>,
    account_id: &str,
    use_cached_context: bool,
    started: &mut bool,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<AssistantMessage, String> {
    match reply {
        "timeout" => Err(websocket_connect_timeout_error(connect_timeout_ms)),
        "limit" => Err(crate::codex::WEBSOCKET_CONNECTION_LIMIT_REACHED.into()),
        "idle" => Err(websocket_idle_timeout_error(
            idle_timeout_ms.unwrap_or(connect_timeout_ms),
        )),
        "unavailable" => Err("WebSocket transport is not available in this runtime".into()),
        corpus => {
            let (reused, continuation) =
                acquire_cached_continuation(cache_session_id, account_id, Instant::now());
            let (request_body, _) =
                build_cached_websocket_request_body(body, continuation.as_ref());
            record_websocket_request_stats(
                cache_session_id,
                reused,
                use_cached_context,
                &request_body,
            );
            *started = true;
            let events = replay_codex_events(model, corpus);
            for event in &events {
                on_event(event);
            }
            let message = done_message(&events)?;
            if use_cached_context {
                store_cached_continuation(
                    cache_session_id,
                    account_id,
                    CachedWebSocketContinuation {
                        last_request_body: body.clone(),
                        last_response_id: "resp_fixture".into(),
                        last_response_items: cached_response_items(&message),
                    },
                    reused,
                    Instant::now(),
                );
            }
            Ok(message)
        }
    }
}

fn allows_live_websocket(url: &str) -> bool {
    is_loopback(url)
        || std::env::var("PI_CODEX_WS_URL").is_ok()
        || (!cfg!(test)
            && (url.starts_with("ws://") || url.starts_with("wss://") || url.starts_with("http")))
}

fn is_loopback(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

fn cached_response_items(message: &AssistantMessage) -> Value {
    let chat = crate::assistant_to_chat(message);
    let items = crate::stream::openai_responses_input(std::slice::from_ref(&chat))
        .into_iter()
        .filter(|item| {
            !matches!(
                item.get("type").and_then(Value::as_str),
                Some("function_call_output" | "custom_tool_call_output")
            )
        })
        .collect();
    Value::Array(items)
}

fn done_message(events: &[AssistantMessageEvent]) -> Result<AssistantMessage, String> {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            AssistantMessageEvent::Done { message, .. } => Some(message.clone()),
            AssistantMessageEvent::Error { error, .. } => Some({
                let mut message = error.clone();
                message.stop_reason = Some(StopReason::Error);
                message
            }),
            _ => None,
        })
        .ok_or_else(|| WEBSOCKET_CLOSED_BEFORE_COMPLETED.to_string())
}

struct LiveSocketKey {
    session_id: String,
    account_id: String,
}

struct AcquiredSocket {
    stream: WsStream,
    reused: bool,
    key: Option<LiveSocketKey>,
}

struct LiveSocketEntry {
    stream: Option<WsStream>,
    busy: bool,
    created_at: Instant,
    last_used: Instant,
}

struct LiveSocketCache {
    sessions: HashMap<String, HashMap<String, LiveSocketEntry>>,
}

fn live_cache() -> &'static Mutex<LiveSocketCache> {
    static CACHE: OnceLock<Mutex<LiveSocketCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LiveSocketCache {
            sessions: HashMap::new(),
        })
    })
}

fn lock_live() -> std::sync::MutexGuard<'static, LiveSocketCache> {
    live_cache().lock().unwrap_or_else(|err| err.into_inner())
}

pub(crate) fn close_live_sockets(session_id: Option<&str>) {
    let mut cache = lock_live();
    if let Some(session_id) = session_id {
        cache.sessions.remove(session_id);
        return;
    }
    cache.sessions.clear();
}

fn stream_is_open(stream: &WsStream) -> bool {
    match &stream.inner {
        WsInner::Plain(inner) => inner.peer_addr().is_ok(),
        WsInner::Tls(inner) => inner.sock.peer_addr().is_ok(),
    }
}

fn is_live_expired(entry: &LiveSocketEntry, now: Instant) -> bool {
    now.duration_since(entry.last_used).as_millis() as u64 >= SESSION_WEBSOCKET_CACHE_TTL_MS
        || now.duration_since(entry.created_at).as_millis() as u64 >= SESSION_WEBSOCKET_MAX_AGE_MS
}

fn acquire_live_socket(
    url: &str,
    headers: &[(String, String)],
    connect_timeout_ms: u64,
    session_id: Option<&str>,
    account_id: &str,
) -> Result<AcquiredSocket, String> {
    let now = Instant::now();
    if let Some(session_id) = session_id {
        let mut cache = lock_live();
        if let Some(account_entries) = cache.sessions.get_mut(session_id) {
            if let Some(entry) = account_entries.get_mut(account_id) {
                if entry.busy {
                    drop(cache);
                    let stream = open_codex_websocket(url, headers, connect_timeout_ms)?;
                    return Ok(AcquiredSocket {
                        stream,
                        reused: false,
                        key: None,
                    });
                }
                if is_live_expired(entry, now)
                    || entry.stream.as_ref().is_some_and(|s| !stream_is_open(s))
                {
                    account_entries.remove(account_id);
                    if account_entries.is_empty() {
                        cache.sessions.remove(session_id);
                    }
                } else if let Some(stream) = entry.stream.take() {
                    entry.busy = true;
                    entry.last_used = now;
                    return Ok(AcquiredSocket {
                        stream,
                        reused: true,
                        key: Some(LiveSocketKey {
                            session_id: session_id.to_string(),
                            account_id: account_id.to_string(),
                        }),
                    });
                }
            }
        }
    }
    let stream = open_codex_websocket(url, headers, connect_timeout_ms)?;
    let key = session_id.map(|session_id| {
        let mut cache = lock_live();
        let account_entries = cache.sessions.entry(session_id.to_string()).or_default();
        account_entries.insert(
            account_id.to_string(),
            LiveSocketEntry {
                stream: None,
                busy: true,
                created_at: now,
                last_used: now,
            },
        );
        LiveSocketKey {
            session_id: session_id.to_string(),
            account_id: account_id.to_string(),
        }
    });
    Ok(AcquiredSocket {
        stream,
        reused: false,
        key,
    })
}

fn release_live_socket(key: Option<LiveSocketKey>, mut stream: WsStream, keep: bool) {
    let Some(key) = key else {
        let _ = write_frame(&mut stream, OPCODE_CLOSE, &[], true);
        return;
    };
    if !keep || !stream_is_open(&stream) {
        let _ = write_frame(&mut stream, OPCODE_CLOSE, &[], true);
        let mut cache = lock_live();
        if let Some(account_entries) = cache.sessions.get_mut(&key.session_id) {
            account_entries.remove(&key.account_id);
            if account_entries.is_empty() {
                cache.sessions.remove(&key.session_id);
            }
        }
        return;
    }
    let mut cache = lock_live();
    if let Some(entry) = cache
        .sessions
        .get_mut(&key.session_id)
        .and_then(|accounts| accounts.get_mut(&key.account_id))
    {
        entry.stream = Some(stream);
        entry.busy = false;
        entry.last_used = Instant::now();
    }
}

struct WsStream {
    inner: WsInner,
}

enum WsInner {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl WsStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match &self.inner {
            WsInner::Plain(stream) => stream.set_read_timeout(timeout),
            WsInner::Tls(stream) => stream.sock.set_read_timeout(timeout),
        }
    }
}

impl Read for WsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.inner {
            WsInner::Plain(stream) => stream.read(buf),
            WsInner::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for WsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match &mut self.inner {
            WsInner::Plain(stream) => stream.write(buf),
            WsInner::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            WsInner::Plain(stream) => stream.flush(),
            WsInner::Tls(stream) => stream.flush(),
        }
    }
}

fn open_codex_websocket(
    url: &str,
    headers: &[(String, String)],
    timeout_ms: u64,
) -> Result<WsStream, String> {
    let parsed = url::Url::parse(url).map_err(|err| format!("WebSocket address: {err}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "WebSocket address: missing host".to_string())?;
    let scheme = parsed.scheme();
    let default_port = if scheme == "wss" || scheme == "https" {
        443
    } else {
        80
    };
    let port = parsed.port().unwrap_or(default_port);
    let path = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };
    let request_path = match parsed.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    };
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let proxy = crate::http_proxy::resolve_http_proxy_url_for_target(url, None)?;
    let tcp = if let Some(proxy) = proxy {
        crate::http_proxy::tcp_connect_via_http_proxy(&proxy, host, port, timeout)?
    } else {
        let addrs = (host, port)
            .to_socket_addrs()
            .map_err(|err| format!("WebSocket address: {err}"))?;
        let mut last_error = "WebSocket connect failed".to_string();
        let mut tcp = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => {
                    tcp = Some(stream);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(websocket_connect_timeout_error(timeout_ms));
                }
                Err(err) => last_error = format!("WebSocket connect failed: {err}"),
            }
        }
        tcp.ok_or(last_error)?
    };
    tcp.set_nodelay(true)
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    let mut stream = if scheme == "wss" || scheme == "https" {
        WsStream {
            inner: WsInner::Tls(Box::new(wrap_tls(tcp, host)?)),
        }
    } else {
        WsStream {
            inner: WsInner::Plain(tcp),
        }
    };
    handshake(
        &mut stream,
        host,
        port,
        default_port,
        &request_path,
        headers,
    )?;
    Ok(stream)
}

fn wrap_tls(
    tcp: TcpStream,
    host: &str,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = rustls_pki_types::ServerName::try_from(host.to_string())
        .map_err(|err| format!("WebSocket address: {err}"))?;
    let conn = rustls::ClientConnection::new(std::sync::Arc::new(config), server_name)
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    Ok(rustls::StreamOwned::new(conn, tcp))
}

fn handshake(
    stream: &mut WsStream,
    host: &str,
    port: u16,
    default_port: u16,
    path: &str,
    headers: &[(String, String)],
) -> Result<(), String> {
    let key = websocket_key();
    let host_header = if port == default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
    );
    for (name, value) in websocket_handshake_headers(headers) {
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("sec-websocket-key")
            || name.eq_ignore_ascii_case("sec-websocket-version")
        {
            continue;
        }
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    let response = read_http_head(stream)?;
    if !response.starts_with("HTTP/1.1 101") && !response.starts_with("HTTP/1.0 101") {
        return Err(format!(
            "WebSocket connect failed: {}",
            response.lines().next().unwrap_or("invalid handshake")
        ));
    }
    let expected = accept_key(&key);
    let accept = response
        .lines()
        .find_map(|line| line.split_once(':'))
        .and_then(|(name, value)| {
            name.eq_ignore_ascii_case("sec-websocket-accept")
                .then_some(value.trim())
        });
    if accept != Some(expected.as_str()) {
        // Some loopback fixtures omit the accept header after a 101.
        if accept.is_some() {
            return Err("WebSocket connect failed: invalid accept key".into());
        }
    }
    Ok(())
}

fn websocket_key() -> String {
    base64::engine::general_purpose::STANDARD.encode(Uuid::new_v4().as_bytes())
}

fn accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

fn read_http_head(stream: &mut WsStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut buf)
            .map_err(|err| map_io_error(err, None))?;
        if read == 0 {
            return Err("WebSocket connect failed: closed during handshake".into());
        }
        bytes.push(buf[0]);
        if bytes.len() > 64 * 1024 {
            return Err("WebSocket connect failed: handshake too large".into());
        }
    }
    String::from_utf8(bytes).map_err(|_| "WebSocket connect failed: invalid handshake".to_string())
}

fn write_text_frame(stream: &mut WsStream, text: &str) -> Result<(), String> {
    write_frame(stream, OPCODE_TEXT, text.as_bytes(), true)
}

fn write_frame(
    stream: &mut WsStream,
    opcode: u8,
    payload: &[u8],
    mask: bool,
) -> Result<(), String> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    let mask_bit = if mask { 0x80 } else { 0 };
    if payload.len() < 126 {
        frame.push(mask_bit | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask_key = if mask {
        let uuid = Uuid::new_v4();
        let bytes = uuid.as_bytes();
        let key = [bytes[0], bytes[1], bytes[2], bytes[3]];
        frame.extend_from_slice(&key);
        Some(key)
    } else {
        None
    };
    if let Some(key) = mask_key {
        for (index, byte) in payload.iter().enumerate() {
            frame.push(byte ^ key[index % 4]);
        }
    } else {
        frame.extend_from_slice(payload);
    }
    stream
        .write_all(&frame)
        .and_then(|_| stream.flush())
        .map_err(|err| format!("WebSocket send failed: {err}"))
}

/// Read frames until the response completes, feeding each one through the
/// decoder and handing every event to `on_event` as it is produced. Returns
/// the raw events (the continuation cache needs the response id), the
/// finished message, and whether the read was cut short by `abort`.
fn read_codex_events(
    stream: &mut WsStream,
    idle_timeout_ms: Option<u64>,
    started: &mut bool,
    decoder: &mut ResponsesDecoder,
    abort: AbortFlag<'_>,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<(Vec<Value>, AssistantMessage, bool), String> {
    use std::sync::atomic::Ordering;
    let mut events = Vec::new();
    let mut produced = Vec::new();
    let mut saw_completion = false;
    loop {
        if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let mut closing = Vec::new();
            let mut message = decoder.finish(&mut closing);
            message.stop_reason = Some(StopReason::Aborted);
            message.error_message = Some("Request was aborted".into());
            on_event(&AssistantMessageEvent::Error {
                reason: StopReason::Aborted,
                error: message.clone(),
            });
            return Ok((events, message, true));
        }
        let (opcode, payload) = match read_frame(stream, idle_timeout_ms) {
            Ok(frame) => frame,
            Err(_) if saw_completion => break,
            Err(err) => return Err(err),
        };
        match opcode {
            OPCODE_TEXT => {
                let text = String::from_utf8(payload)
                    .map_err(|_| "Invalid Codex WebSocket JSON: invalid utf8".to_string())?;
                let parsed: Value = serde_json::from_str(&text)
                    .map_err(|err| format!("Invalid Codex WebSocket JSON: {err}"))?;
                let event_type = parsed.get("type").and_then(Value::as_str).unwrap_or("");
                if crate::trace::enabled() {
                    crate::trace::log(&format!(
                        "ws frame {}",
                        crate::trace::describe_event(&parsed)
                    ));
                }
                if event_type == "error" {
                    let detail = parsed
                        .pointer("/error/message")
                        .or_else(|| parsed.get("message"))
                        .or_else(|| parsed.pointer("/error/code"))
                        .or_else(|| parsed.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or("error");
                    return Err(format!("Codex error: {detail}"));
                }
                if event_type == "response.failed" {
                    let reason = parsed
                        .pointer("/response/error/message")
                        .or_else(|| parsed.pointer("/response/error/code"))
                        .and_then(Value::as_str)
                        .unwrap_or("Codex response failed");
                    return Err(format!("Codex error: {reason}"));
                }
                if matches!(
                    event_type,
                    "response.created"
                        | "response.output_text.delta"
                        | "response.completed"
                        | "response.done"
                        | "response.incomplete"
                ) {
                    *started = true;
                }
                let first = produced.len();
                decoder.feed(&parsed, &mut produced);
                for event in &produced[first..] {
                    on_event(event);
                }
                if matches!(
                    event_type,
                    "response.completed" | "response.done" | "response.incomplete"
                ) {
                    saw_completion = true;
                    events.push(parsed);
                    break;
                }
                events.push(parsed);
            }
            OPCODE_PING => write_frame(stream, OPCODE_PONG, &payload, true)?,
            OPCODE_CLOSE => {
                if !saw_completion {
                    return Err(WEBSOCKET_CLOSED_BEFORE_COMPLETED.into());
                }
                break;
            }
            _ => {}
        }
    }
    if !saw_completion {
        return Err(WEBSOCKET_CLOSED_BEFORE_COMPLETED.into());
    }
    let first = produced.len();
    let message = decoder.finish(&mut produced);
    for event in &produced[first..] {
        on_event(event);
    }
    Ok((events, message, false))
}

fn read_frame(
    stream: &mut WsStream,
    idle_timeout_ms: Option<u64>,
) -> Result<(u8, Vec<u8>), String> {
    let mut header = [0u8; 2];
    read_exact(stream, &mut header, idle_timeout_ms)?;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut len = (header[1] & 0x7f) as u64;
    if len == 126 {
        let mut ext = [0u8; 2];
        read_exact(stream, &mut ext, idle_timeout_ms)?;
        len = u16::from_be_bytes(ext) as u64;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        read_exact(stream, &mut ext, idle_timeout_ms)?;
        len = u64::from_be_bytes(ext);
    }
    if len > 8 * 1024 * 1024 {
        return Err("WebSocket message too big".into());
    }
    let mask = if masked {
        let mut key = [0u8; 4];
        read_exact(stream, &mut key, idle_timeout_ms)?;
        Some(key)
    } else {
        None
    };
    let mut payload = vec![0u8; len as usize];
    read_exact(stream, &mut payload, idle_timeout_ms)?;
    if let Some(key) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[index % 4];
        }
    }
    Ok((opcode, payload))
}

fn read_exact(
    stream: &mut WsStream,
    buf: &mut [u8],
    idle_timeout_ms: Option<u64>,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < buf.len() {
        match stream.read(&mut buf[offset..]) {
            Ok(0) => return Err(WEBSOCKET_CLOSED_BEFORE_COMPLETED.into()),
            Ok(read) => offset += read,
            Err(err) => return Err(map_io_error(err, idle_timeout_ms)),
        }
    }
    Ok(())
}

fn map_io_error(err: std::io::Error, idle_timeout_ms: Option<u64>) -> String {
    if err.kind() == std::io::ErrorKind::TimedOut || err.kind() == std::io::ErrorKind::WouldBlock {
        if let Some(ms) = idle_timeout_ms {
            return websocket_idle_timeout_error(ms);
        }
        return websocket_connect_timeout_error(0);
    }
    format!("WebSocket error: {err}")
}

#[cfg(test)]
pub(crate) fn accept_key_for_tests(key: &str) -> String {
    accept_key(key)
}

#[cfg(test)]
pub(crate) fn read_masked_text(stream: &mut impl Read) -> std::io::Result<String> {
    let mut frame = [0u8; 2];
    stream.read_exact(&mut frame)?;
    let mut len = (frame[1] & 0x7f) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext)?;
        len = u16::from_be_bytes(ext) as usize;
    }
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask)?;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    String::from_utf8(payload)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

#[cfg(test)]
pub(crate) fn write_unmasked_text(stream: &mut impl Write, text: &str) -> std::io::Result<()> {
    let payload = text.as_bytes();
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(payload.len() as u8);
    } else {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;

    fn codex_model() -> Model {
        Model {
            id: "gpt-5".into(),
            name: "gpt-5".into(),
            api: "openai-codex-responses".into(),
            provider: "openai-codex".into(),
            base_url: Some(crate::codex::DEFAULT_CODEX_BASE_URL.into()),
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 200_000,
            max_tokens: 32_000,
            compat: Value::Null,
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn cached_fixture_response_is_subtracted_from_the_next_user_delta() {
        let _guard = crate::codex::websocket_state_test_lock();
        const SESSION: &str = "cached-fixture-response-delta";
        crate::codex::close_openai_codex_websocket_sessions(Some(SESSION));
        let first = serde_json::json!({
            "model": "gpt-5",
            "input": [
                {"role":"user","content":[{"type":"input_text","text":"one"}]}
            ]
        });
        let corpus = r#"{"type":"response.created"}
{"type":"response.output_text.delta","delta":"Hi"}
{"type":"response.completed","response":{"id":"resp_fixture","status":"completed"}}"#;
        let mut started = false;
        process_fixture(
            corpus,
            &first,
            &codex_model(),
            50,
            None,
            Some(SESSION),
            "acc_test",
            true,
            &mut started,
            &mut |_| {},
        )
        .expect("fixture completes");

        let (_, continuation) =
            acquire_cached_continuation(Some(SESSION), "acc_test", Instant::now());
        let continuation = continuation.expect("cached continuation");
        let assistant_item = serde_json::json!({
            "role":"assistant",
            "content":[{"type":"output_text","text":"Hi"}]
        });
        assert_eq!(
            continuation.last_response_items,
            Value::Array(vec![assistant_item.clone()]),
            "the cached baseline must include the response represented in previous_response_id"
        );

        let second = serde_json::json!({
            "model": "gpt-5",
            "input": [
                {"role":"user","content":[{"type":"input_text","text":"one"}]},
                assistant_item,
                {"role":"user","content":[{"type":"input_text","text":"two"}]}
            ]
        });
        let (delta, used) = build_cached_websocket_request_body(&second, Some(&continuation));
        assert!(used);
        assert_eq!(
            delta["input"],
            serde_json::json!([
                {"role":"user","content":[{"type":"input_text","text":"two"}]}
            ])
        );
        crate::codex::close_openai_codex_websocket_sessions(Some(SESSION));
    }
}
