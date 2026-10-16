//! Offline loopback tests for `transport: "http-reuse"`. No network, no credentials: a scripted
//! HTTP/1.1 server plays the provider and counts the connections it accepts.
use super::*;
use crate::Model;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy, PartialEq)]
enum Reply {
    /// A complete chunked event stream, keep-alive.
    Stream,
    /// A complete stream, then the server closes the connection.
    StreamThenClose,
    /// `Connection: close` in the response headers.
    StreamCloseHeader,
    /// An error status with a body, keep-alive.
    Error500,
    /// Headers and one event, then silence until the client leaves.
    Hang,
}

struct Server {
    url: String,
    accepts: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
}

fn public_model() -> Model {
    serde_json::from_value(serde_json::json!({
        "id":"gpt-6-luna", "name":"fixture", "api":"openai-codex-responses",
        "provider":"openai-codex", "baseUrl":"https://api.openai.com/v1", "reasoning":true,
        "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":1000,"maxTokens":100
    }))
    .unwrap()
}

fn read_request(reader: &mut BufReader<TcpStream>) -> bool {
    let mut length = 0;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    reader.read_exact(&mut vec![0; length]).is_ok()
}

fn chunk(text: &str) -> String {
    format!("{:x}\r\n{text}\r\n", text.len())
}

fn spawn_server(script: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let accepts = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (a, r) = (accepts.clone(), requests.clone());
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { return };
            a.fetch_add(1, Ordering::SeqCst);
            let (script, r) = (script.clone(), r.clone());
            std::thread::spawn(move || {
                let mut writer = socket.try_clone().unwrap();
                let mut reader = BufReader::new(socket);
                while read_request(&mut reader) {
                    let index = r.fetch_add(1, Ordering::SeqCst);
                    let reply = script.get(index).copied().unwrap_or(Reply::Stream);
                    let event = chunk("data: {\"type\":\"response.completed\"}\n\n");
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n";
                    match reply {
                        Reply::Stream | Reply::StreamThenClose | Reply::StreamCloseHeader => {
                            let close = if reply == Reply::StreamCloseHeader {
                                "Connection: close\r\n"
                            } else {
                                ""
                            };
                            writer
                                .write_all(format!("{head}{close}\r\n{event}").as_bytes())
                                .unwrap();
                            // The chunk terminator arrives after the event, as on the wire.
                            std::thread::sleep(Duration::from_millis(40));
                            writer.write_all(b"0\r\n\r\n").unwrap();
                            if reply != Reply::Stream {
                                return;
                            }
                        }
                        Reply::Error500 => {
                            let body = "{\"error\":\"boom\"}";
                            writer
                                .write_all(
                                    format!(
                                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{body}",
                                        body.len()
                                    )
                                    .as_bytes(),
                                )
                                .unwrap();
                        }
                        Reply::Hang => {
                            writer
                                .write_all(format!("{head}\r\n{event}").as_bytes())
                                .unwrap();
                            let mut byte = [0u8; 1];
                            let _ = reader.read(&mut byte);
                            return;
                        }
                    }
                }
            });
        }
    });
    Server {
        url,
        accepts,
        requests,
    }
}

/// Reads the response the way `read_provider_stream` does: stop at the terminal event, mark
/// the stream finished, drop the reader.
fn consume(response: crate::stream_http::Response) {
    let (reader, cancellation) = response.into_reader();
    let lines = crate::stream_reader::response_lines(reader, cancellation);
    loop {
        let line = lines.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        if line.contains("response.completed") {
            break;
        }
    }
    lines.mark_finished();
    drop(lines);
}

fn call(plan: Plan, url: &str, abort: Option<Arc<AtomicBool>>) -> crate::stream_http::Response {
    crate::stream_http::send_reusing(
        plan,
        url,
        &[],
        &serde_json::json!({"a": 1}),
        None,
        false,
        abort,
    )
    .unwrap()
}

fn id(name: &str) -> String {
    format!("{name}-{:?}", std::thread::current().id())
}

#[test]
fn selection_needs_the_public_plan_route_and_the_exact_value() {
    let model = public_model();
    assert!(selected(&model, Some("http-reuse")));
    for value in [
        None,
        Some("auto"),
        Some("sse"),
        Some("websocket"),
        Some("HTTP-REUSE"),
        Some("http-reuse "),
        Some(""),
    ] {
        assert!(!selected(&model, value), "{value:?}");
    }
    let mut other = public_model();
    other.base_url = Some("https://chatgpt.com/backend-api/codex".into());
    assert!(!selected(&other, Some("http-reuse")));
    let mut other = public_model();
    other.provider = "openai".into();
    assert!(!selected(&other, Some("http-reuse")));
}

#[test]
fn a_conversation_reuses_one_connection_across_requests_that_stop_at_the_terminal_event() {
    let server = spawn_server(vec![]);
    let key = id("reuse");
    let mut labels = vec![];
    let mut session = None;
    for _ in 0..4 {
        let plan = plan(Some(&key));
        labels.push(plan.label);
        session = Some(plan.lease.as_ref().unwrap().session());
        consume(call(plan, &server.url, None));
    }
    assert_eq!(labels, [LABEL_OPEN, LABEL_REUSE, LABEL_REUSE, LABEL_REUSE]);
    assert_eq!(server.requests.load(Ordering::SeqCst), 4);
    assert_eq!(
        server.accepts.load(Ordering::SeqCst),
        1,
        "one connection for the conversation"
    );
    assert_eq!(session.unwrap().connects(), 1);
}

#[test]
fn requests_without_a_conversation_keep_one_connection_per_request() {
    let server = spawn_server(vec![]);
    for _ in 0..3 {
        let plan = plan(None);
        assert_eq!(plan.label, LABEL_OPEN);
        assert!(plan.lease.is_none());
        consume(call(plan, &server.url, None));
    }
    assert_eq!(server.accepts.load(Ordering::SeqCst), 3);
}

#[test]
fn the_default_one_shot_path_never_pools() {
    let server = spawn_server(vec![]);
    for _ in 0..3 {
        let response =
            crate::stream_http::send(&server.url, &[], &serde_json::json!({}), None, false, None)
                .unwrap();
        consume(response);
    }
    assert_eq!(server.accepts.load(Ordering::SeqCst), 3);
}

#[test]
fn an_abort_mid_response_discards_the_connection_and_the_next_request_opens_a_new_one() {
    let server = spawn_server(vec![Reply::Hang]);
    let key = id("abort");
    let abort = Arc::new(AtomicBool::new(false));
    let first = plan(Some(&key));
    let session = first.lease.as_ref().unwrap().session();
    let response = call(first, &server.url, Some(abort.clone()));
    let (reader, cancellation) = response.into_reader();
    let lines = crate::stream_reader::response_lines(reader, cancellation);
    lines.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    abort.store(true, Ordering::Relaxed);
    // Not marked finished: this is a cancellation, not a normal end.
    drop(lines);
    let second = {
        // The aborted reader releases the session once its blocked read notices the abort.
        let started = Instant::now();
        loop {
            let candidate = plan(Some(&key));
            if candidate.lease.is_some() || started.elapsed() > Duration::from_secs(3) {
                break candidate;
            }
        }
    };
    assert_eq!(
        second.label, LABEL_OPEN,
        "an aborted response is never expected to be reusable"
    );
    consume(call(second, &server.url, None));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 2);
    assert_eq!(session.connects(), 2);
}

#[test]
fn a_cancel_that_arrives_after_the_response_never_poisons_the_next_request() {
    let server = spawn_server(vec![]);
    let key = id("late-abort");
    let abort = Arc::new(AtomicBool::new(false));
    consume(call(plan(Some(&key)), &server.url, Some(abort.clone())));
    // Let the tail drain finish: an abort raised mid-drain discards the connection by design.
    std::thread::sleep(Duration::from_millis(300));
    abort.store(true, Ordering::Relaxed);
    let second = plan(Some(&key));
    assert_eq!(second.label, LABEL_REUSE);
    consume(call(second, &server.url, None));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_concurrent_request_gets_its_own_connection_and_leaves_the_session_alone() {
    let server = spawn_server(vec![Reply::Hang]);
    let key = id("concurrent");
    let first = plan(Some(&key));
    let session = first.lease.as_ref().unwrap().session();
    let abort = Arc::new(AtomicBool::new(false));
    let response = call(first, &server.url, Some(abort.clone()));
    let (reader, cancellation) = response.into_reader();
    let lines = crate::stream_reader::response_lines(reader, cancellation);
    lines.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    let second = plan(Some(&key));
    assert_eq!(second.label, LABEL_OPEN);
    assert!(second.lease.is_none(), "the busy session is not shared");
    consume(call(second, &server.url, None));
    assert_eq!(session.connects(), 1);
    abort.store(true, Ordering::Relaxed);
    drop(lines);
}

#[test]
fn a_server_that_closes_after_a_response_costs_a_new_connection_and_no_error() {
    let server = spawn_server(vec![Reply::StreamThenClose]);
    let key = id("closed");
    consume(call(plan(Some(&key)), &server.url, None));
    std::thread::sleep(Duration::from_millis(150));
    consume(call(plan(Some(&key)), &server.url, None));
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(server.accepts.load(Ordering::SeqCst), 2);
}

#[test]
fn connection_close_in_the_response_headers_is_honoured() {
    let server = spawn_server(vec![Reply::StreamCloseHeader]);
    let key = id("close-header");
    consume(call(plan(Some(&key)), &server.url, None));
    consume(call(plan(Some(&key)), &server.url, None));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 2);
}

#[test]
fn an_error_status_with_a_body_does_not_waste_the_connection() {
    let server = spawn_server(vec![Reply::Error500]);
    let key = id("error");
    let error = crate::stream_http::send_reusing(
        plan(Some(&key)),
        &server.url,
        &[],
        &serde_json::json!({}),
        None,
        false,
        None,
    )
    .err()
    .unwrap();
    assert_eq!(error.status, Some(500));
    let second = plan(Some(&key));
    assert_eq!(second.label, LABEL_REUSE);
    consume(call(second, &server.url, None));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn conversations_never_share_a_connection() {
    let server = spawn_server(vec![]);
    consume(call(plan(Some(&id("conv-a"))), &server.url, None));
    consume(call(plan(Some(&id("conv-b"))), &server.url, None));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failed_send_releases_the_session_for_the_next_request() {
    let key = id("refused");
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1/responses", listener.local_addr().unwrap())
    };
    assert!(crate::stream_http::send_reusing(
        plan(Some(&key)),
        &dead,
        &[],
        &serde_json::json!({}),
        None,
        false,
        None
    )
    .is_err());
    let server = spawn_server(vec![]);
    let second = plan(Some(&key));
    assert_eq!(second.label, LABEL_OPEN);
    assert!(
        second.lease.is_some(),
        "the lease was released by the failure"
    );
    consume(call(second, &server.url, None));
}
