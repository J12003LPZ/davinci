//! Offline validation of the public Responses WebSocket transport.
//! A scripted loopback server stands in for the provider; no network, no credentials.

use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Reply {
    /// Send these events, then keep the connection open for the next request.
    Events(Vec<Value>),
    /// Send these events, then close the connection.
    EventsThenClose(Vec<Value>),
    /// Read the request, then close without answering.
    Close,
}

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<(usize, Value)>>>,
    handshakes: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// `script[connection][request]`: one reply per request the client is expected to send.
    fn start(script: Vec<Vec<Reply>>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handshakes = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_t, hs_t, stop_t) = (seen.clone(), handshakes.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            let mut workers = Vec::new();
            for (index, replies) in script.into_iter().enumerate() {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    if stop_t.load(Ordering::Relaxed) || Instant::now() > deadline {
                        for worker in workers {
                            let _: std::thread::JoinHandle<()> = worker;
                        }
                        return;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(err) => panic!("{err}"),
                    }
                };
                let (seen, handshakes) = (seen_t.clone(), hs_t.clone());
                workers.push(std::thread::spawn(move || {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        if stream.read_exact(&mut byte).is_err() {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8(head).unwrap();
                    let key = head
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(n, _)| n.eq_ignore_ascii_case("sec-websocket-key"))
                                .map(|(_, v)| v.trim().to_string())
                        })
                        .unwrap();
                    handshakes.lock().unwrap().push(head);
                    write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key)).unwrap();
                    for reply in replies {
                        let Ok(text) = read_masked_text(&mut stream) else {
                            return;
                        };
                        // A client close frame arrives here as a non-JSON payload.
                        let Ok(parsed) = serde_json::from_str(&text) else {
                            return;
                        };
                        seen.lock().unwrap().push((index, parsed));
                        match reply {
                            Reply::Events(events) => {
                                for event in events {
                                    let _ = write_unmasked_text(&mut stream, &event.to_string());
                                }
                            }
                            Reply::EventsThenClose(events) => {
                                for event in events {
                                    let _ = write_unmasked_text(&mut stream, &event.to_string());
                                }
                                return;
                            }
                            Reply::Close => return,
                        }
                    }
                    // Hold the socket open until the client is done with it.
                    let mut sink = [0u8; 1];
                    let _ = stream.read(&mut sink);
                }));
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Server {
            url,
            seen,
            handshakes,
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> (Vec<(usize, Value)>, Vec<String>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let seen = self.seen.lock().unwrap().clone();
        let handshakes = self.handshakes.lock().unwrap().clone();
        (seen, handshakes)
    }
}

fn public_model() -> Model {
    serde_json::from_value(json!({
        "id":"gpt-6-luna", "name":"fixture", "api":"openai-codex-responses",
        "provider":"openai-codex", "baseUrl":"https://api.openai.com/v1", "reasoning":true,
        "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":1000,"maxTokens":100
    }))
    .unwrap()
}

fn backend_model() -> Model {
    let mut model = public_model();
    model.base_url = Some("https://chatgpt.com/backend-api/codex".into());
    model
}

fn user(text: &str) -> Value {
    json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]})
}

fn text_output(id: &str, text: &str) -> Vec<Value> {
    vec![
        json!({"type":"message","id":id,"role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":text}]}),
    ]
}

fn tool_output(id: &str, call: &str) -> Vec<Value> {
    vec![
        json!({"type":"function_call","id":id,"call_id":call,"name":"read",
        "arguments":"{\"path\":\"a\"}","status":"completed"}),
    ]
}

fn completed(id: &str, output: &[Value], input_tokens: u64, cached: u64) -> Value {
    json!({"type":"response.completed","response":{
        "id":id,"status":"completed","output":output,
        "usage":{"input_tokens":input_tokens,"output_tokens":7,"total_tokens":input_tokens + 7,
                 "input_tokens_details":{"cached_tokens":cached,"cache_write_tokens":0}}}})
}

fn created_only() -> Value {
    json!({"type":"response.created","response":{"id":"resp_x","status":"in_progress"}})
}

fn body(input: Vec<Value>) -> Value {
    json!({"model":"gpt-6-luna","stream":true,"store":false,"prompt_cache_key":"ci-key",
           "instructions":"be brief","include":["reasoning.encrypted_content"],"input":input})
}

fn extend(input: &[Value], output: &[Value], new: Value) -> Vec<Value> {
    let mut next = input.to_vec();
    next.extend(output.iter().cloned());
    next.push(new);
    next
}

type Sent = Result<(CodexWebsocketMessage, PublicWsRequestShape), PublicWsFailure>;

fn send(url: &str, session: &str, body: &Value) -> Sent {
    send_with(url, session, body, None, &mut |_| {})
}

fn send_with(
    url: &str,
    session: &str,
    body: &Value,
    abort: AbortFlag<'_>,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Sent {
    process_public_websocket(
        url,
        body,
        &[("Authorization".into(), "Bearer fixture-plan-access".into())],
        &public_model(),
        2000,
        Some(2000),
        session,
        &mut false,
        abort,
        on_event,
    )
}

struct Session(
    &'static str,
    #[allow(dead_code)] std::sync::MutexGuard<'static, ()>,
);

impl Session {
    fn open(name: &'static str) -> Self {
        let guard = crate::codex::websocket_state_test_lock();
        std::env::remove_var("PI_CODEX_WS_REPLY");
        set_public_ws_url_for_tests(None);
        crate::codex::close_openai_codex_websocket_sessions(Some(name));
        Session(name, guard)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        set_public_ws_url_for_tests(None);
        crate::codex::close_openai_codex_websocket_sessions(Some(self.0));
    }
}

fn input_of(value: &Value) -> Vec<Value> {
    value["input"].as_array().cloned().unwrap_or_default()
}

fn auth() -> crate::ResolvedAuth {
    crate::ResolvedAuth {
        api_key: Some("fixture-plan-access".into()),
        headers: Default::default(),
        source: "OAuth".into(),
    }
}

fn entry_options(session: &str) -> crate::StreamOptions {
    crate::StreamOptions {
        transport: Some(PUBLIC_WS_TRANSPORT.into()),
        session_id: Some(session.into()),
        ..Default::default()
    }
}

fn entry_messages() -> Vec<crate::ChatMessage> {
    serde_json::from_value(json!([{"role":"user","content":[{"type":"text","text":"hi"}]}]))
        .unwrap()
}

/// Request-header names of a captured handshake, lowercased, in order.
fn header_names(head: &str) -> Vec<String> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim().to_ascii_lowercase())
        .collect()
}

#[test]
fn the_public_route_defaults_to_the_websocket_and_http_values_opt_out() {
    let public = public_model();
    for transport in [None, Some("auto"), Some(PUBLIC_WS_TRANSPORT)] {
        assert!(
            public_websocket_selected(&public, transport),
            "{transport:?}"
        );
    }
    for transport in [
        Some("sse"),
        Some("http"),
        Some("websocket"),
        Some("websocket-cached"),
        Some(""),
        Some("Public-WebSocket"),
        // The continuation arm of the confirmation study is not shipped.
        Some("public-websocket-continuation"),
    ] {
        assert!(
            !public_websocket_selected(&public, transport),
            "{transport:?}"
        );
    }
    // Never the ChatGPT backend route, and never another provider, whatever the value.
    for transport in [None, Some("auto"), Some(PUBLIC_WS_TRANSPORT)] {
        assert!(!public_websocket_selected(&backend_model(), transport));
        let mut other = public_model();
        other.provider = "openai".into();
        assert!(!public_websocket_selected(&other, transport));
    }
}

#[test]
fn the_public_url_is_the_documented_responses_endpoint() {
    let _session = Session::open("public-ws-url");
    assert_eq!(PUBLIC_RESPONSES_WS_URL, "wss://api.openai.com/v1/responses");
    assert_eq!(public_ws_url(), PUBLIC_RESPONSES_WS_URL);
    set_public_ws_url_for_tests(Some("ws://127.0.0.1:1".into()));
    assert_eq!(public_ws_url(), "ws://127.0.0.1:1");
}

#[test]
fn one_socket_serves_the_conversation_and_every_request_replays_full_input() {
    let session = Session::open("public-ws-persistent");
    let out1 = text_output("m1", "one");
    let out2 = text_output("m2", "two");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
        Reply::Events(vec![completed("resp_2", &out2, 1200, 1000)]),
        Reply::Events(vec![completed(
            "resp_3",
            &text_output("m3", "3"),
            1400,
            1200,
        )]),
    ]]);
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    let in3 = extend(&in2, &out2, user("c"));
    let shapes: Vec<_> = [&in1, &in2, &in3]
        .iter()
        .map(|input| {
            send(&server.url, session.0, &body((*input).clone()))
                .unwrap()
                .1
        })
        .collect();
    let (seen, handshakes) = server.finish();
    assert_eq!(
        handshakes.len(),
        1,
        "one connection for the whole conversation"
    );
    assert_eq!(
        shapes.iter().map(|s| s.socket_reused).collect::<Vec<_>>(),
        [false, true, true]
    );
    assert_eq!(seen.len(), 3);
    for ((connection, sent), expected) in seen.iter().zip([&in1, &in2, &in3]) {
        assert_eq!(*connection, 0);
        assert_eq!(sent["type"], "response.create");
        assert!(sent.get("previous_response_id").is_none());
        assert!(sent.get("stream").is_none(), "transport-only field leaked");
        assert_eq!(sent["store"], false);
        assert_eq!(sent["prompt_cache_key"], "ci-key");
        assert_eq!(&input_of(sent), expected, "input is replayed in full");
    }
    assert_eq!(
        shapes.iter().map(|s| s.input_items).collect::<Vec<_>>(),
        [1, 3, 5]
    );
}

#[test]
fn a_previous_response_id_from_the_caller_is_never_sent() {
    let session = Session::open("public-ws-no-prev");
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1",
        &text_output("m1", "one"),
        10,
        0,
    )])]]);
    let mut request = body(vec![user("a")]);
    request["previous_response_id"] = json!("resp_stale");
    request["background"] = json!(true);
    send(&server.url, session.0, &request).unwrap();
    let (seen, _) = server.finish();
    assert!(seen[0].1.get("previous_response_id").is_none());
    assert!(seen[0].1.get("background").is_none());
}

#[test]
fn a_socket_closed_while_idle_is_replaced_before_sending_without_a_failed_attempt() {
    let session = Session::open("public-ws-new-socket");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![
        vec![Reply::EventsThenClose(vec![completed(
            "resp_1", &out1, 1000, 0,
        )])],
        vec![Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1200,
            0,
        )])],
    ]);
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    let scope = crate::provider_observation::ObservationScope::capture();
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    send(&server.url, session.0, &body(in1)).unwrap();
    // Let the server's close land before the next request would reuse the cached socket.
    std::thread::sleep(Duration::from_millis(300));
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    let (message, shape) = send(&server.url, session.0, &body(in2.clone())).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(message.message.stop_reason, Some(StopReason::Stop));
    assert_eq!(handshakes.len(), 2, "the closed socket was replaced");
    assert!(!shape.socket_reused);
    assert_eq!(input_of(&seen[1].1), in2);
    let events = scope.finish("completed");
    let ends: Vec<_> = events.iter().filter(|e| e.kind == "attempt_end").collect();
    assert_eq!(ends.len(), 2, "no failed attempt for the dead socket");
    assert!(ends.iter().all(|e| e.status == "completed"));
}

#[test]
fn a_reused_socket_that_dies_after_the_request_is_retried_once_on_a_new_socket() {
    // The socket looks healthy when the next request is sent, so the probe passes. The server then
    // reads the request and drops the connection. Nothing reached the caller, so one new socket
    // carries the request.
    let session = Session::open("public-ws-dead-reused");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![
        vec![
            Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
            Reply::Close,
        ],
        vec![Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1200,
            0,
        )])],
    ]);
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    let scope = crate::provider_observation::ObservationScope::capture();
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    send(&server.url, session.0, &body(in1)).unwrap();
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    let mut delivered = Vec::new();
    let (message, shape) = send_with(
        &server.url,
        session.0,
        &body(in2.clone()),
        None,
        &mut |event| delivered.push(event.clone()),
    )
    .unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(message.message.stop_reason, Some(StopReason::Stop));
    assert_eq!(handshakes.len(), 2);
    assert!(!shape.socket_reused, "the retry runs on a new socket");
    assert_eq!(
        seen.len(),
        3,
        "request 2 reached the dead socket, then the new one"
    );
    assert_eq!((seen[1].0, seen[2].0), (0, 1));
    assert_eq!(input_of(&seen[2].1), in2);
    assert!(
        delivered
            .iter()
            .all(|e| !matches!(e, AssistantMessageEvent::Error { .. })),
        "the caller never saw the dead socket"
    );
    let events = scope.finish("completed");
    let statuses: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "attempt_end")
        .map(|e| e.status.as_str())
        .collect();
    assert_eq!(statuses, ["completed", "failed", "completed"]);
}

#[test]
fn the_retry_is_bounded_to_one_new_socket() {
    let session = Session::open("public-ws-retry-bound");
    let server = Server::start(vec![
        vec![
            Reply::Events(vec![completed("resp_1", &text_output("m1", "one"), 10, 0)]),
            Reply::Close,
        ],
        vec![Reply::Close],
        vec![Reply::Events(vec![completed(
            "resp_3",
            &text_output("m3", "3"),
            10,
            0,
        )])],
    ]);
    send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    let failure = send(&server.url, session.0, &body(vec![user("b")])).unwrap_err();
    let (_, handshakes) = server.finish();
    assert_eq!(handshakes.len(), 2, "no third connection");
    assert!(failure.http_fallback_safe);
}

#[test]
fn a_fresh_socket_closed_before_any_event_is_safe_to_replay_over_http() {
    let session = Session::open("public-ws-closed");
    let server = Server::start(vec![vec![Reply::Close]]);
    let failure = send(&server.url, session.0, &body(vec![user("a")])).unwrap_err();
    let (seen, _) = server.finish();
    assert_eq!(seen.len(), 1);
    assert!(failure.http_fallback_safe, "{failure:?}");
    assert!(!failure.message.is_empty());
}

#[test]
fn a_connection_closed_mid_stream_is_never_safe_to_replay() {
    let session = Session::open("public-ws-midstream");
    let server = Server::start(vec![vec![Reply::EventsThenClose(vec![created_only()])]]);
    let mut started = false;
    let result = process_public_websocket(
        &server.url,
        &body(vec![user("a")]),
        &[],
        &public_model(),
        2000,
        Some(2000),
        session.0,
        &mut started,
        None,
        &mut |_| {},
    );
    server.finish();
    assert!(started, "the caller learns the stream had begun");
    let failure = result.unwrap_err();
    assert!(!failure.http_fallback_safe, "{failure:?}");
}

#[test]
fn a_mid_stream_close_on_a_reused_socket_is_not_retried() {
    let session = Session::open("public-ws-midstream-reused");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &text_output("m1", "one"), 10, 0)]),
        Reply::EventsThenClose(vec![created_only()]),
    ]]);
    send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    let failure = send(&server.url, session.0, &body(vec![user("b")])).unwrap_err();
    let (seen, handshakes) = server.finish();
    assert!(!failure.http_fallback_safe);
    assert_eq!(
        handshakes.len(),
        1,
        "output began, so the request is not replayed"
    );
    assert_eq!(seen.len(), 2);
}

#[test]
fn a_refused_connection_is_safe_to_replay_over_http() {
    let session = Session::open("public-ws-refused");
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let failure = send(
        &format!("ws://127.0.0.1:{port}"),
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap_err();
    assert!(failure.http_fallback_safe, "{failure:?}");
}

#[test]
fn a_refused_upgrade_is_safe_to_replay_over_http() {
    let session = Session::open("public-ws-upgrade-refused");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if stream.read_exact(&mut byte).is_err() {
                return;
            }
            head.push(byte[0]);
        }
        let _ = write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    });
    let failure = send(&url, session.0, &body(vec![user("a")])).unwrap_err();
    handle.join().unwrap();
    assert!(failure.http_fallback_safe, "{failure:?}");
}

#[test]
fn a_failed_response_surfaces_as_an_error_message_and_drops_the_socket() {
    let session = Session::open("public-ws-failed");
    let failed = json!({"type":"response.failed","response":{"id":"resp_f","status":"failed",
        "error":{"code":"server_error","message":"boom"},"output":[]}});
    let server = Server::start(vec![
        vec![Reply::Events(vec![failed])],
        vec![Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            900,
            0,
        )])],
    ]);
    let (first, _) = send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    assert_eq!(first.message.stop_reason, Some(StopReason::Error));
    assert!(first.native_responses.is_none());
    let (_, shape) = send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    let (_, handshakes) = server.finish();
    assert_eq!(
        handshakes.len(),
        2,
        "a failed response never keeps its socket"
    );
    assert!(!shape.socket_reused);
}

#[test]
fn usage_and_cached_tokens_are_decoded_from_the_terminal_event() {
    let session = Session::open("public-ws-usage");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1", &out1, 5000, 3000,
    )])]]);
    let (message, _) = send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    server.finish();
    let usage = message.message.usage.clone().expect("usage");
    assert_eq!(usage.cache_read, 3000);
    assert_eq!(usage.input, 2000, "uncached input is total minus cached");
    assert_eq!(usage.output, 7);
    assert!(message.native_responses.is_some());
}

#[test]
fn tool_calls_and_text_decode_the_same_as_on_http() {
    let session = Session::open("public-ws-tools");
    let output = vec![
        text_output("m1", "checking").remove(0),
        tool_output("fc1", "call_1").remove(0),
    ];
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1", &output, 800, 0,
    )])]]);
    let (message, _) = send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    server.finish();
    let calls = message
        .message
        .content
        .iter()
        .filter(|block| matches!(block, crate::ContentBlock::ToolCall { .. }))
        .count();
    assert_eq!(calls, 1);
    assert_eq!(message.message.stop_reason, Some(StopReason::ToolUse));
}

#[test]
fn cancellation_before_connecting_never_opens_a_socket() {
    let session = Session::open("public-ws-abort-early");
    let abort = Arc::new(AtomicBool::new(true));
    let server = Server::start(vec![vec![Reply::Close]]);
    let (message, _) = send_with(
        &server.url,
        session.0,
        &body(vec![user("a")]),
        Some(&abort),
        &mut |_| {},
    )
    .unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(message.message.stop_reason, Some(StopReason::Aborted));
    assert!(seen.is_empty() && handshakes.is_empty());
}

#[test]
fn cancellation_mid_response_closes_the_socket_and_the_next_request_starts_on_a_new_one() {
    let session = Session::open("public-ws-abort-mid");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![
        vec![Reply::Events(vec![completed("resp_1", &out1, 1000, 0)])],
        vec![Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1100,
            0,
        )])],
    ]);
    let abort = Arc::new(AtomicBool::new(false));
    let in1 = vec![user("a")];
    let (message, _) = send_with(
        &server.url,
        session.0,
        &body(in1.clone()),
        Some(&abort),
        &mut |event| {
            if matches!(event, AssistantMessageEvent::TextEnd { .. }) {
                abort.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap();
    assert_eq!(message.message.stop_reason, Some(StopReason::Aborted));
    assert!(message.native_responses.is_none());
    let in2 = extend(&in1, &out1, user("b"));
    let (_, shape) = send(&server.url, session.0, &body(in2.clone())).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(handshakes.len(), 2);
    assert!(!shape.socket_reused);
    assert_eq!(input_of(&seen[1].1), in2);
}

#[test]
fn the_handshake_carries_the_bearer_token_and_no_backend_headers() {
    let session = Session::open("public-ws-headers");
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1",
        &text_output("m1", "one"),
        10,
        0,
    )])]]);
    send(&server.url, session.0, &body(vec![user("a")])).unwrap();
    let (_, handshakes) = server.finish();
    let head = handshakes[0].to_ascii_lowercase();
    assert!(head.starts_with("get /"));
    assert!(head.contains("authorization: bearer fixture-plan-access"));
    for forbidden in ["chatgpt-account-id", "openai-beta", "x-codex", "originator"] {
        assert!(
            !head.contains(forbidden),
            "{forbidden} must not be sent to the public route"
        );
    }
}

#[test]
fn fixture_replay_can_never_stand_in_for_the_network() {
    let session = Session::open("public-ws-no-fixture");
    std::env::set_var("PI_CODEX_WS_REPLY", "{}");
    let result = send("ws://127.0.0.1:9", session.0, &body(vec![user("a")]));
    std::env::remove_var("PI_CODEX_WS_REPLY");
    let failure = result.unwrap_err();
    assert!(failure.message.contains("never replays fixtures"));
    assert!(
        !failure.http_fallback_safe,
        "a configuration error is not hidden"
    );
}

#[test]
fn the_default_runtime_refuses_non_loopback_hosts_in_tests() {
    let session = Session::open("public-ws-no-network");
    let failure = send(PUBLIC_RESPONSES_WS_URL, session.0, &body(vec![user("a")])).unwrap_err();
    assert!(failure.message.contains("not available"), "{failure:?}");
    assert!(failure.http_fallback_safe);
}

#[test]
fn encrypted_reasoning_is_replayed_in_the_input() {
    let reasoning =
        json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque-blob"});
    let mut out1 = vec![reasoning.clone()];
    out1.extend(text_output("m1", "one"));
    let session = Session::open("public-ws-reasoning");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
        Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1100,
            1000,
        )]),
    ]]);
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    send(&server.url, session.0, &body(in1)).unwrap();
    send(&server.url, session.0, &body(in2.clone())).unwrap();
    let (seen, _) = server.finish();
    let second = input_of(&seen[1].1);
    assert!(
        second.contains(&reasoning),
        "the reasoning item rides in the replayed input"
    );
    assert_eq!(second, in2);
    assert_eq!(seen[1].1["include"], json!(["reasoning.encrypted_content"]));
}

#[test]
fn attempt_rows_record_the_transport_and_usage_of_every_request() {
    let session = Session::open("public-ws-observation");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
        Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1300,
            900,
        )]),
    ]]);
    let scope = crate::provider_observation::ObservationScope::capture();
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    send(&server.url, session.0, &body(in1)).unwrap();
    crate::provider_observation::begin_request("coding", "gpt-6-luna", Some("high"), "schema");
    send(&server.url, session.0, &body(in2)).unwrap();
    server.finish();
    let events = scope.finish("completed");
    let starts: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "attempt_start")
        .collect();
    assert_eq!(
        starts
            .iter()
            .map(|e| e.transport.as_deref())
            .collect::<Vec<_>>(),
        [Some(PUBLIC_WS_ATTEMPT), Some(PUBLIC_WS_ATTEMPT)]
    );
    let ends: Vec<_> = events.iter().filter(|e| e.kind == "attempt_end").collect();
    assert!(ends
        .iter()
        .all(|e| e.status == "completed" && e.usage_complete == Some(true)));
    let raw = ends[1].raw_usage.as_ref().expect("raw usage");
    assert_eq!((raw.input_total, raw.cache_read), (Some(1300), Some(900)));
}

#[test]
fn a_busy_cached_socket_forces_a_temporary_one_that_is_not_kept() {
    let session = Session::open("public-ws-busy");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![
        vec![
            Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
            Reply::Events(vec![completed("resp_3", &text_output("m3", "3"), 1300, 0)]),
        ],
        vec![Reply::Events(vec![completed(
            "resp_2",
            &text_output("m2", "2"),
            1100,
            0,
        )])],
    ]);
    let in1 = vec![user("a")];
    send(&server.url, session.0, &body(in1.clone())).unwrap();
    // Take the cached socket out so the next request finds it busy.
    let held = acquire_live_socket(&server.url, &[], 2000, Some(session.0), PUBLIC_WS_SLOT)
        .expect("the cached socket");
    assert!(held.reused);
    let in2 = extend(&in1, &out1, user("b"));
    let (_, shape) = send(&server.url, session.0, &body(in2.clone())).unwrap();
    assert!(
        !shape.socket_reused,
        "the busy socket forces a temporary one"
    );
    release_live_socket(held.key, held.stream, true);
    let (_, shape) = send(&server.url, session.0, &body(in2)).unwrap();
    let (_, handshakes) = server.finish();
    assert!(
        shape.socket_reused,
        "the original socket serves the next request"
    );
    assert_eq!(handshakes.len(), 2);
}

#[test]
fn the_stream_entry_point_sends_the_assembled_headers_in_the_handshake() {
    let session = Session::open("public-ws-entry-headers");
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1",
        &text_output("m1", "hello"),
        50,
        10,
    )])]]);
    set_public_ws_url_for_tests(Some(server.url.clone()));
    let envelope = crate::stream::live_complete_streaming_with_sink_envelope(
        &public_model(),
        &entry_messages(),
        &auth(),
        Some("be brief"),
        &[],
        &entry_options(session.0),
        &mut |_| {},
    )
    .expect("the websocket request succeeds");
    let (seen, handshakes) = server.finish();
    assert_eq!(envelope.message.stop_reason, Some(StopReason::Stop));
    assert_eq!(seen.len(), 1);
    let head = &handshakes[0];
    let names = header_names(head);
    let unique: std::collections::BTreeSet<_> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "a header was sent twice: {names:?}"
    );
    // The handshake owns these four; the HTTP request headers that describe a body are removed.
    for http_only in [
        "content-type",
        "content-length",
        "accept-encoding",
        "accept",
    ] {
        assert!(
            !names.iter().any(|n| n == http_only),
            "{http_only} leaked into the handshake: {names:?}"
        );
    }
    for required in [
        "host",
        "upgrade",
        "connection",
        "sec-websocket-key",
        "sec-websocket-version",
        "authorization",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} missing: {names:?}"
        );
    }
    assert!(head
        .to_ascii_lowercase()
        .contains("authorization: bearer fixture-plan-access"));
    for forbidden in ["chatgpt-account-id", "openai-beta", "x-codex"] {
        assert!(
            !names.iter().any(|n| n.starts_with(forbidden)),
            "{forbidden}"
        );
    }
    // Every header outside the handshake core comes from the assembled HTTP request. Pin the set
    // so a new header cannot start flowing to the public route unnoticed.
    let core = [
        "host",
        "upgrade",
        "connection",
        "sec-websocket-key",
        "sec-websocket-version",
        "authorization",
    ];
    let extra: Vec<_> = names
        .iter()
        .filter(|n| !core.contains(&n.as_str()))
        .cloned()
        .collect();
    assert_eq!(
        extra, EXPECTED_EXTRA_HEADERS,
        "unexpected handshake headers"
    );
}

/// Headers the public route sends beyond the handshake core, from the shared HTTP header assembly.
const EXPECTED_EXTRA_HEADERS: [&str; 0] = [];

#[test]
fn a_mid_stream_failure_through_the_entry_point_is_returned_and_not_replayed() {
    let session = Session::open("public-ws-entry-midstream");
    let server = Server::start(vec![vec![Reply::EventsThenClose(vec![created_only()])]]);
    set_public_ws_url_for_tests(Some(server.url.clone()));
    let error = crate::stream::live_complete_streaming_with_sink_envelope(
        &public_model(),
        &entry_messages(),
        &auth(),
        Some("be brief"),
        &[],
        &entry_options(session.0),
        &mut |_| {},
    )
    .unwrap_err();
    let (_, handshakes) = server.finish();
    assert!(error.contains("not replayed over HTTP"), "{error}");
    assert_eq!(handshakes.len(), 1);
    assert!(
        !public_ws_in_backoff(session.0),
        "a mid-stream failure is not a reason to leave the transport"
    );
}

mod backoff {
    use super::*;

    #[test]
    fn a_fallback_keeps_the_conversation_on_http_for_five_minutes() {
        let _session = Session::open("public-ws-backoff");
        let now = Instant::now();
        assert!(!backoff_active_at("public-ws-backoff", now));
        note_fallback_at("public-ws-backoff", now);
        assert!(backoff_active_at("public-ws-backoff", now));
        assert!(backoff_active_at(
            "public-ws-backoff",
            now + Duration::from_millis(PUBLIC_WS_BACKOFF_MS - 1)
        ));
        assert!(!backoff_active_at(
            "public-ws-backoff",
            now + Duration::from_millis(PUBLIC_WS_BACKOFF_MS)
        ));
        assert!(
            !backoff_active_at("public-ws-backoff", now),
            "an expired entry is removed"
        );
    }

    #[test]
    fn the_backoff_belongs_to_one_conversation() {
        let _session = Session::open("public-ws-backoff-a");
        let now = Instant::now();
        note_fallback_at("public-ws-backoff-a", now);
        assert!(backoff_active_at("public-ws-backoff-a", now));
        assert!(!backoff_active_at("public-ws-backoff-b", now));
    }

    #[test]
    fn closing_the_conversation_clears_its_backoff() {
        let session = Session::open("public-ws-backoff-close");
        note_public_ws_fallback(session.0);
        assert!(public_ws_in_backoff(session.0));
        crate::codex::close_openai_codex_websocket_sessions(Some(session.0));
        assert!(!public_ws_in_backoff(session.0));
        note_public_ws_fallback(session.0);
        crate::codex::close_openai_codex_websocket_sessions(None);
        assert!(!public_ws_in_backoff(session.0));
    }

    #[test]
    fn the_table_is_bounded() {
        let _session = Session::open("public-ws-backoff-bound");
        let now = Instant::now();
        for index in 0..(PUBLIC_WS_BACKOFF_MAX_SESSIONS * 2) {
            note_fallback_at(&format!("public-ws-bound-{index}"), now);
        }
        let size = backoff_state().lock().unwrap().len();
        assert!(size <= PUBLIC_WS_BACKOFF_MAX_SESSIONS, "{size}");
        clear_public_ws_backoff(None);
    }
}

mod tls {
    //! A real rustls session over loopback, with no certificate authority: the client accepts any
    //! server certificate and the server signs with a stub, because only the connection state
    //! matters here.
    use super::*;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::{CertifiedKey, Signer, SigningKey};
    use rustls::{DigitallySignedStruct, SignatureAlgorithm, SignatureScheme};
    use std::sync::mpsc;

    #[derive(Debug)]
    struct AcceptAll;

    impl ServerCertVerifier for AcceptAll {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    }

    #[derive(Debug)]
    struct StubKey;

    #[derive(Debug)]
    struct StubSigner;

    impl SigningKey for StubKey {
        fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
            offered
                .contains(&SignatureScheme::ED25519)
                .then(|| Box::new(StubSigner) as Box<dyn Signer>)
        }

        fn algorithm(&self) -> SignatureAlgorithm {
            SignatureAlgorithm::ED25519
        }
    }

    impl Signer for StubSigner {
        fn sign(&self, _: &[u8]) -> Result<Vec<u8>, rustls::Error> {
            Ok(vec![0; 64])
        }

        fn scheme(&self) -> SignatureScheme {
            SignatureScheme::ED25519
        }
    }

    #[derive(Debug)]
    struct StubCert;

    impl ResolvesServerCert for StubCert {
        fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(Arc::new(CertifiedKey::new(
                vec![CertificateDer::from(vec![0u8; 8])],
                Arc::new(StubKey),
            )))
        }
    }

    /// A connected TLS client socket whose server has sent "hello" and optionally a close_notify,
    /// while keeping the TCP connection open. Dropping the guard releases the server.
    struct Pair {
        client: WsStream,
        _release: mpsc::Sender<()>,
        server: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Pair {
        fn drop(&mut self) {
            if let Some(handle) = self.server.take() {
                let _ = self._release.send(());
                let _ = handle.join();
            }
        }
    }

    fn pair(close_notify: bool) -> Pair {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server_config = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider.clone())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(StubCert)),
        );
        let client_config = Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAll))
                .with_no_client_auth(),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (sent_tx, sent_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(server_config).unwrap();
            while conn.is_handshaking() {
                conn.complete_io(&mut tcp).unwrap();
            }
            conn.writer().write_all(b"hello").unwrap();
            if close_notify {
                conn.send_close_notify();
            }
            while conn.wants_write() {
                conn.write_tls(&mut tcp).unwrap();
            }
            sent_tx.send(()).unwrap();
            // Keep the TCP connection open: only the TLS close_notify, if any, has been sent.
            let _ = release_rx.recv();
        });
        let tcp = TcpStream::connect(addr).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let conn =
            rustls::ClientConnection::new(client_config, "localhost".try_into().unwrap()).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        // The handshake needs both sides to run at once; the server thread is already waiting.
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).unwrap();
        }
        sent_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Everything the server sent is now in the TCP buffer; one read takes it into rustls.
        std::thread::sleep(Duration::from_millis(200));
        let mut buf = [0u8; 16];
        let read = tls.read(&mut buf).unwrap();
        assert_eq!(&buf[..read], b"hello");
        Pair {
            client: WsStream {
                inner: WsInner::Tls(Box::new(tls)),
            },
            _release: release_tx,
            server: Some(server),
        }
    }

    #[test]
    fn a_quiet_open_tls_socket_passes_the_probe() {
        let mut pair = pair(false);
        assert!(socket_is_quiet_and_open(&mut pair.client));
    }

    #[test]
    fn a_close_notify_already_consumed_by_rustls_fails_the_probe() {
        // The TCP connection is open and has no readable byte, so a TCP peek alone calls this
        // socket healthy. rustls has already seen the close_notify.
        let mut pair = pair(true);
        if let WsInner::Tls(tls) = &pair.client.inner {
            let mut probe = [0u8; 1];
            tls.sock.set_nonblocking(true).unwrap();
            let peeked = tls.sock.peek(&mut probe);
            tls.sock.set_nonblocking(false).unwrap();
            assert!(
                matches!(peeked, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
                "the premise: TCP alone shows nothing"
            );
        }
        assert!(!socket_is_quiet_and_open(&mut pair.client));
    }
}
