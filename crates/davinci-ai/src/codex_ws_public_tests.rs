//! Offline validation of the public Responses WebSocket experiment.
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
    /// Close without answering.
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
                 "input_tokens_details":{"cached_tokens":cached}}}})
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

type Sent = Result<(CodexWebsocketMessage, PublicWsRequestShape), String>;

fn send(url: &str, mode: PublicWsMode, session: &str, body: &Value) -> Sent {
    send_with(url, mode, session, body, None, &mut |_| {})
}

fn send_with(
    url: &str,
    mode: PublicWsMode,
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
        mode,
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
        crate::codex::close_openai_codex_websocket_sessions(Some(name));
        crate::codex::clear_cached_continuation(Some(name), PUBLIC_WS_SLOT);
        Session(name, guard)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        crate::codex::close_openai_codex_websocket_sessions(Some(self.0));
        crate::codex::clear_cached_continuation(Some(self.0), PUBLIC_WS_SLOT);
    }
}

fn input_of(value: &Value) -> Vec<Value> {
    value["input"].as_array().cloned().unwrap_or_default()
}

#[test]
fn only_explicit_values_on_the_public_route_select_the_experiment() {
    let public = public_model();
    for transport in [
        None,
        Some("auto"),
        Some("sse"),
        Some("websocket"),
        Some("websocket-cached"),
        Some(""),
        Some("Public-WebSocket"),
    ] {
        assert_eq!(
            PublicWsMode::select(&public, transport),
            None,
            "{transport:?}"
        );
    }
    assert_eq!(
        PublicWsMode::select(&public, Some(PUBLIC_WS_TRANSPORT_PERSISTENT)),
        Some(PublicWsMode::Persistent)
    );
    assert_eq!(
        PublicWsMode::select(&public, Some(PUBLIC_WS_TRANSPORT_CONTINUATION)),
        Some(PublicWsMode::Continuation)
    );
    // Never the ChatGPT backend route, and never another provider.
    let backend = backend_model();
    assert_eq!(
        PublicWsMode::select(&backend, Some(PUBLIC_WS_TRANSPORT_CONTINUATION)),
        None
    );
    let mut other = public_model();
    other.provider = "openai".into();
    assert_eq!(
        PublicWsMode::select(&other, Some(PUBLIC_WS_TRANSPORT_PERSISTENT)),
        None
    );
}

#[test]
fn the_public_url_is_the_documented_responses_endpoint() {
    assert_eq!(PUBLIC_RESPONSES_WS_URL, "wss://api.openai.com/v1/responses");
}

#[test]
fn persistent_mode_reuses_one_socket_and_always_replays_full_input() {
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
            send(
                &server.url,
                PublicWsMode::Persistent,
                session.0,
                &body((*input).clone()),
            )
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
    assert!(shapes.iter().all(|s| !s.previous_response_id));
}

#[test]
fn continuation_sends_only_new_items_with_previous_response_id() {
    let session = Session::open("public-ws-continuation");
    let out1 = text_output("m1", "one");
    let out2 = tool_output("fc2", "call_2");
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
    let tool_result = json!({"type":"function_call_output","call_id":"call_2","output":"ok"});
    let in3 = extend(&in2, &out2, tool_result.clone());
    let mode = PublicWsMode::Continuation;
    let s1 = send(&server.url, mode, session.0, &body(in1.clone()))
        .unwrap()
        .1;
    let s2 = send(&server.url, mode, session.0, &body(in2.clone()))
        .unwrap()
        .1;
    let s3 = send(&server.url, mode, session.0, &body(in3.clone()))
        .unwrap()
        .1;
    let (seen, handshakes) = server.finish();
    assert_eq!(handshakes.len(), 1);
    assert!(!s1.previous_response_id && s2.previous_response_id && s3.previous_response_id);
    assert_eq!(seen[0].1.get("previous_response_id"), None);
    assert_eq!(input_of(&seen[0].1), in1);
    assert_eq!(seen[1].1["previous_response_id"], "resp_1");
    assert_eq!(
        input_of(&seen[1].1),
        vec![user("b")],
        "only the new user item"
    );
    assert_eq!(seen[2].1["previous_response_id"], "resp_2");
    assert_eq!(
        input_of(&seen[2].1),
        vec![tool_result],
        "tool output is the only new item"
    );
    for (_, sent) in &seen {
        assert_eq!(sent["type"], "response.create");
        assert!(sent.get("stream").is_none());
        assert_eq!(sent["store"], false);
        assert_eq!(sent["prompt_cache_key"], "ci-key");
    }
    assert!(s3.input_items < in3.len());
}

#[test]
fn continuation_never_crosses_connections() {
    // The server closes after request 1, so request 2 must open a new socket and start full,
    // without a failed attempt in between.
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
    let mode = PublicWsMode::Continuation;
    send(&server.url, mode, session.0, &body(in1)).unwrap();
    // Let the server's close land before the next request would reuse the cached socket.
    std::thread::sleep(Duration::from_millis(300));
    let (message, shape) = send(&server.url, mode, session.0, &body(in2.clone())).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(message.message.stop_reason, Some(StopReason::Stop));
    assert_eq!(handshakes.len(), 2, "the closed socket was replaced");
    assert!(!shape.socket_reused && !shape.previous_response_id);
    assert_eq!(seen[1].0, 1);
    assert!(seen[1].1.get("previous_response_id").is_none());
    assert_eq!(input_of(&seen[1].1), in2);
}

#[test]
fn a_changed_prefix_such_as_compaction_replays_in_full_on_the_same_socket() {
    let session = Session::open("public-ws-compaction");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
        Reply::Events(vec![completed("resp_2", &text_output("m2", "2"), 300, 0)]),
    ]]);
    let mode = PublicWsMode::Continuation;
    send(&server.url, mode, session.0, &body(vec![user("a")])).unwrap();
    // A compacted window no longer starts with the previous request, so it is not a delta.
    let compacted = vec![
        json!({"type":"compaction","encrypted_content":"opaque"}),
        user("b"),
    ];
    let shape = send(&server.url, mode, session.0, &body(compacted.clone()))
        .unwrap()
        .1;
    let (seen, handshakes) = server.finish();
    assert_eq!(
        handshakes.len(),
        1,
        "the socket stays; only the request shape changes"
    );
    assert!(shape.socket_reused);
    assert!(!shape.previous_response_id);
    assert!(seen[1].1.get("previous_response_id").is_none());
    assert_eq!(input_of(&seen[1].1), compacted);
}

#[test]
fn changed_top_level_fields_also_force_a_full_replay() {
    let session = Session::open("public-ws-fields");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![vec![
        Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
        Reply::Events(vec![completed("resp_2", &text_output("m2", "2"), 1100, 0)]),
    ]]);
    let mode = PublicWsMode::Continuation;
    let in1 = vec![user("a")];
    send(&server.url, mode, session.0, &body(in1.clone())).unwrap();
    let mut next = body(extend(&in1, &out1, user("b")));
    next["prompt_cache_key"] = json!("another-key");
    let shape = send(&server.url, mode, session.0, &next).unwrap().1;
    let (seen, _) = server.finish();
    assert!(!shape.previous_response_id);
    assert!(seen[1].1.get("previous_response_id").is_none());
}

#[test]
fn previous_response_not_found_is_an_explicit_failed_attempt_and_the_next_request_starts_full() {
    let session = Session::open("public-ws-not-found");
    let out1 = text_output("m1", "one");
    let not_found = json!({"type":"error","status":400,"error":{"type":"invalid_request_error",
        "code":"previous_response_not_found","message":"Previous response with id 'resp_1' not found.",
        "param":"previous_response_id"}});
    let server = Server::start(vec![
        vec![
            Reply::Events(vec![completed("resp_1", &out1, 1000, 0)]),
            Reply::Events(vec![not_found]),
        ],
        vec![Reply::Events(vec![completed(
            "resp_3",
            &text_output("m3", "3"),
            1500,
            0,
        )])],
    ]);
    let mode = PublicWsMode::Continuation;
    let in1 = vec![user("a")];
    let in2 = extend(&in1, &out1, user("b"));
    send(&server.url, mode, session.0, &body(in1)).unwrap();
    let (failed, shape) = send(&server.url, mode, session.0, &body(in2.clone())).unwrap();
    // One create, no hidden retry inside the transport, surfaced as a failed response.
    assert!(shape.previous_response_id);
    assert_eq!(failed.message.stop_reason, Some(StopReason::Error));
    assert!(failed
        .message
        .error_message
        .as_deref()
        .unwrap_or("")
        .contains("not found"));
    assert!(failed.native_responses.is_none());
    // The caller's own retry then finds no stale id and no reusable socket.
    let (ok, third) = send(&server.url, mode, session.0, &body(in2.clone())).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(ok.message.stop_reason, Some(StopReason::Stop));
    assert_eq!(handshakes.len(), 2);
    assert!(!third.previous_response_id && !third.socket_reused);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1].1["previous_response_id"], "resp_1");
    assert!(seen[2].1.get("previous_response_id").is_none());
    assert_eq!(input_of(&seen[2].1), in2);
}

#[test]
fn a_connection_closed_before_any_event_is_an_error_not_a_fallback() {
    let session = Session::open("public-ws-closed");
    let server = Server::start(vec![vec![Reply::Close]]);
    let error = send(
        &server.url,
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap_err();
    let (seen, _) = server.finish();
    assert_eq!(seen.len(), 1);
    assert!(!error.is_empty());
}

#[test]
fn a_connection_closed_mid_stream_is_an_error() {
    let session = Session::open("public-ws-midstream");
    let created =
        json!({"type":"response.created","response":{"id":"resp_x","status":"in_progress"}});
    let server = Server::start(vec![vec![Reply::EventsThenClose(vec![created])]]);
    let mut started = false;
    let result = process_public_websocket(
        &server.url,
        &body(vec![user("a")]),
        &[],
        &public_model(),
        PublicWsMode::Persistent,
        2000,
        Some(2000),
        session.0,
        &mut started,
        None,
        &mut |_| {},
    );
    server.finish();
    assert!(started, "the caller learns the stream had begun");
    assert!(result.is_err());
}

#[test]
fn a_failed_response_surfaces_as_an_error_message_and_drops_the_socket_and_state() {
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
    let mode = PublicWsMode::Continuation;
    let (first, _) = send(&server.url, mode, session.0, &body(vec![user("a")])).unwrap();
    assert_eq!(first.message.stop_reason, Some(StopReason::Error));
    assert!(first.native_responses.is_none());
    let (_, shape) = send(&server.url, mode, session.0, &body(vec![user("a")])).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(
        handshakes.len(),
        2,
        "a failed response never keeps its socket"
    );
    assert!(!shape.previous_response_id && !shape.socket_reused);
    assert!(seen[1].1.get("previous_response_id").is_none());
}

#[test]
fn usage_and_cached_tokens_are_decoded_from_the_terminal_event() {
    let session = Session::open("public-ws-usage");
    let out1 = text_output("m1", "one");
    let server = Server::start(vec![vec![Reply::Events(vec![completed(
        "resp_1", &out1, 5000, 3000,
    )])]]);
    let (message, _) = send(
        &server.url,
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap();
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
    let (message, _) = send(
        &server.url,
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap();
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
        PublicWsMode::Persistent,
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
fn cancellation_mid_response_closes_the_socket_and_the_next_request_starts_full() {
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
    let mode = PublicWsMode::Continuation;
    let abort = Arc::new(AtomicBool::new(false));
    let in1 = vec![user("a")];
    let (message, _) = send_with(
        &server.url,
        mode,
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
    let (_, shape) = send(&server.url, mode, session.0, &body(in2.clone())).unwrap();
    let (seen, handshakes) = server.finish();
    assert_eq!(handshakes.len(), 2);
    assert!(!shape.previous_response_id);
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
    send(
        &server.url,
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap();
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
    let result = send(
        "ws://127.0.0.1:9",
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    );
    std::env::remove_var("PI_CODEX_WS_REPLY");
    assert!(result.unwrap_err().contains("never replays fixtures"));
}

#[test]
fn the_default_runtime_refuses_non_loopback_hosts_in_tests() {
    let session = Session::open("public-ws-no-network");
    let error = send(
        PUBLIC_RESPONSES_WS_URL,
        PublicWsMode::Persistent,
        session.0,
        &body(vec![user("a")]),
    )
    .unwrap_err();
    assert!(error.contains("not available"), "{error}");
}

#[test]
fn the_stream_entry_point_reports_the_failure_and_never_falls_back_to_http() {
    let session = Session::open("public-ws-stream");
    let model = public_model();
    let auth = crate::ResolvedAuth {
        api_key: Some("fixture-plan-access".into()),
        headers: Default::default(),
        source: "OAuth".into(),
    };
    let options = crate::StreamOptions {
        transport: Some(PUBLIC_WS_TRANSPORT_PERSISTENT.into()),
        session_id: Some(session.0.into()),
        ..Default::default()
    };
    let messages: Vec<crate::ChatMessage> =
        serde_json::from_value(json!([{"role":"user","content":[{"type":"text","text":"hi"}]}]))
            .unwrap();
    let error = crate::stream::live_complete_streaming_with_sink_envelope(
        &model,
        &messages,
        &auth,
        Some("be brief"),
        &[],
        &options,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.contains("no SSE fallback"), "{error}");
}

#[test]
fn encrypted_reasoning_is_replayed_by_persistent_mode_and_carried_by_the_server_in_continuation() {
    let reasoning =
        json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque-blob"});
    let mut out1 = vec![reasoning.clone()];
    out1.extend(text_output("m1", "one"));
    for mode in [PublicWsMode::Persistent, PublicWsMode::Continuation] {
        let name = if mode == PublicWsMode::Persistent {
            "public-ws-reasoning-p"
        } else {
            "public-ws-reasoning-c"
        };
        let session = Session::open(name);
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
        send(&server.url, mode, session.0, &body(in1)).unwrap();
        send(&server.url, mode, session.0, &body(in2.clone())).unwrap();
        let (seen, _) = server.finish();
        let second = input_of(&seen[1].1);
        match mode {
            PublicWsMode::Persistent => {
                assert!(
                    second.contains(&reasoning),
                    "the reasoning item rides in the replayed input"
                );
                assert_eq!(second, in2);
            }
            PublicWsMode::Continuation => {
                assert!(
                    !second.contains(&reasoning),
                    "the server already holds it in memory"
                );
                assert_eq!(second, vec![user("b")]);
                assert_eq!(seen[1].1["previous_response_id"], "resp_1");
            }
        }
        assert_eq!(seen[1].1["include"], json!(["reasoning.encrypted_content"]));
    }
}
