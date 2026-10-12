//! WS3 diagnosis: what the real turn loop does when a provider stream is cut.
//!
//! Written during the diagnosis of the 324-run luna-high benchmark, where four
//! runs exited 1 on "Stream ended before the tool call was complete" with no
//! retry. The tests first pinned that behaviour; they now assert the fix: a
//! cut inside a tool call is retried within the budget, nothing from a cut
//! response executes, and a tool call stuck repeating one fragment is
//! abandoned. Assertions still marked `PINNED (diagnosis)` describe behaviour
//! that was found and deliberately left as it is.
//!
//! The path under test is the production one, end to end and offline:
//! a loopback HTTP server plays a scripted provider, the request goes through
//! `davinci_ai::live_complete_streaming_with_sink_envelope` (real SSE reader,
//! real `ResponsesDecoder`, real provider observations), and the closure that
//! wraps it mirrors the one in `davinci-coding-agent/src/main.rs` (live sink
//! through `Agent::emit_live`, `streamed_live: started`). `Agent::run_loop`
//! is the real loop, with its real retry check.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use davinci_agent::{Agent, AgentEvent, CompleteOutput, EventSink, PermissionMode};
use davinci_ai::{
    AssistantMessageEvent, Model, ModelCost, ResolvedAuth, StopReason, StreamOptions,
};
use serde_json::{json, Value};

/// The file the first, completed tool call of a response would create.
const SIDE_EFFECT: &str = "ws3_side_effect.txt";

/// How the scripted provider answers one request.
#[derive(Clone)]
enum Reply {
    /// 200 with an event-stream body, then a clean close of the socket. A
    /// body without the terminal `response.completed` frame is a cut stream.
    Sse(String),
    /// 200 with a chunked event-stream body that is closed without the
    /// terminating zero chunk: the transport itself reports the cut.
    SseBrokenChunk(String),
    /// A plain HTTP error before any stream exists.
    Status(&'static str, &'static str),
}

struct Provider {
    base_url: String,
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
}

/// Serve `replies` in order, one per request, on a loopback socket.
fn provider(replies: Vec<Reply>) -> Provider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let (server_hits, server_bodies) = (hits.clone(), bodies.clone());
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            let header_end = loop {
                if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break request.len(),
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            };
            let head = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            server_bodies
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&request[header_end..]).unwrap_or(Value::Null));
            server_hits.fetch_add(1, Ordering::SeqCst);
            let response = match reply {
                Reply::Sse(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
                ),
                Reply::SseBrokenChunk(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n{:x}\r\nda",
                    body.len(),
                    4096
                ),
                Reply::Status(status, body) => format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ),
            };
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // Dropping the stream closes the socket: that is the cut.
        }
    });
    Provider {
        base_url,
        hits,
        bodies,
    }
}

fn model(base_url: &str) -> Model {
    Model {
        id: "ws3-fixture".into(),
        name: "ws3-fixture".into(),
        // Same decoder (`ResponsesDecoder`) as `openai-codex-responses`, the
        // API of the benchmark runs, without the Codex websocket/zstd/OAuth
        // envelope around it.
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: Some(base_url.into()),
        reasoning: false,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: Value::Null,
        headers: Default::default(),
        thinking_level_map: Default::default(),
    }
}

fn frames(values: &[Value]) -> String {
    values
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect()
}

fn created() -> Value {
    json!({"type":"response.created","response":{"id":"resp_ws3"}})
}

fn completed() -> Value {
    json!({"type":"response.completed","response":{"id":"resp_ws3","status":"completed",
        "usage":{"input_tokens":120,"output_tokens":9,"total_tokens":129,
                 // `cache_write_tokens` must be present: an attempt whose raw
                 // counters lack it is recorded as usage-incomplete.
                 "input_tokens_details":{"cached_tokens":100,"cache_write_tokens":0}}}})
}

fn text_item(index: u64, text: &str, done: bool) -> Vec<Value> {
    let mut out = vec![
        json!({"type":"response.output_item.added","output_index":index,
               "item":{"type":"message","id":format!("msg_{index}")}}),
        json!({"type":"response.output_text.delta","output_index":index,"delta":text}),
    ];
    if done {
        out.push(
            json!({"type":"response.output_item.done","output_index":index,
            "item":{"type":"message","id":format!("msg_{index}"),
                    "content":[{"type":"output_text","text":text}]}}),
        );
    }
    out
}

/// A `write` call for `SIDE_EFFECT`. `done: false` stops after the argument
/// deltas, the way the cut benchmark streams did.
fn write_call(index: u64, path: &str, done: bool) -> Vec<Value> {
    let arguments = json!({"path": path, "content": "written by a tool call"}).to_string();
    let (head, tail) = arguments.split_at(arguments.len() / 2);
    let mut out = vec![
        json!({"type":"response.output_item.added","output_index":index,
               "item":{"type":"function_call","id":format!("fc_{index}"),
                       "call_id":format!("call_{index}"),"name":"write","arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","output_index":index,"delta":head}),
    ];
    if done {
        out.push(json!({"type":"response.function_call_arguments.delta","output_index":index,"delta":tail}));
        out.push(json!({"type":"response.function_call_arguments.done","output_index":index,"arguments":arguments}));
        out.push(
            json!({"type":"response.output_item.done","output_index":index,
            "item":{"type":"function_call","id":format!("fc_{index}"),
                    "call_id":format!("call_{index}"),"name":"write","arguments":arguments}}),
        );
    }
    out
}

/// A complete, well-formed final answer.
fn final_answer() -> Reply {
    let mut all = vec![created()];
    all.extend(text_item(0, "recovered", true));
    all.push(completed());
    Reply::Sse(frames(&all))
}

fn update_kind(event: &AssistantMessageEvent) -> String {
    serde_json::to_value(event).unwrap()["type"]
        .as_str()
        .unwrap_or("?")
        .to_string()
}

fn label(event: &AgentEvent) -> String {
    match event {
        AgentEvent::MessageUpdate {
            assistant_message_event,
            ..
        } => format!("update/{}", update_kind(assistant_message_event)),
        other => other.kind().to_string(),
    }
}

struct Outcome {
    /// Requests the provider received.
    requests: usize,
    /// Request bodies, in order.
    bodies: Vec<Value>,
    /// Everything the event sink saw, in order (what `--mode json` prints).
    sunk: Vec<String>,
    /// The list `run_loop` returned (what the exit code is computed from).
    recorded: Vec<AgentEvent>,
    stats: davinci_agent::RunStats,
    side_effect_exists: bool,
    side_effect_text: Option<String>,
}

impl Outcome {
    fn recorded_labels(&self) -> Vec<String> {
        self.recorded.iter().map(label).collect()
    }

    fn count(&self, list: &[String], wanted: &str) -> usize {
        list.iter().filter(|kind| kind.as_str() == wanted).count()
    }

    /// Stop reason and error text of the last assistant stream event that
    /// `run_loop` recorded. `print_text_exit` in
    /// `davinci-coding-agent/src/main.rs` scans the recorded list backwards
    /// for exactly this, and returns exit code 1 on `Error`.
    fn final_assistant(&self) -> (Option<StopReason>, Option<String>) {
        self.recorded
            .iter()
            .rev()
            .find_map(|event| match event {
                AgentEvent::MessageUpdate {
                    assistant_message_event,
                    ..
                } => {
                    let message = assistant_message_event.message();
                    Some((message.stop_reason, message.error_message.clone()))
                }
                _ => None,
            })
            .unwrap_or((None, None))
    }

    /// The `print_text_exit` rule applied to the recorded events: the first
    /// assistant stream event from the end whose stop reason is error,
    /// aborted or length makes the process exit 1. (That function is private
    /// to the `davinci` binary; `print_mode_exits_nonzero_on_assistant_error`
    /// in `main_tests.rs` pins it there.)
    fn print_mode_exit_code(&self) -> i32 {
        let failed = self.recorded.iter().rev().any(|event| match event {
            AgentEvent::MessageUpdate {
                assistant_message_event,
                ..
            } => matches!(
                assistant_message_event.message().stop_reason,
                Some(StopReason::Error | StopReason::Aborted | StopReason::Length)
            ),
            _ => false,
        });
        i32::from(failed)
    }

    fn retry_events(&self) -> Vec<Value> {
        self.recorded
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentEvent::AutoRetryStart { .. } | AgentEvent::AutoRetryEnd { .. }
                )
            })
            .map(|event| serde_json::to_value(event).unwrap())
            .collect()
    }
}

/// Run one prompt through the real loop against `replies`.
fn run(replies: Vec<Reply>, configure: impl FnOnce(&mut Agent)) -> Outcome {
    let server = provider(replies);
    let model = model(&server.base_url);
    let auth = ResolvedAuth {
        api_key: Some("ws3-test-key".into()),
        headers: Default::default(),
        source: "test".into(),
    };
    let root = tempfile::tempdir().unwrap();
    let sunk: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = sunk.clone();

    let mut agent = Agent::new("ws3 stream cut fixture");
    agent.cwd = root.path().to_path_buf();
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    // The defaults a print-mode run gets (`Agent::new`: auto_retry = true,
    // retry_attempts = 3, provider_max_retries = None) stay as they are; only
    // the back-off is zeroed so the test does not sleep.
    agent.retry_base_delay_ms = 0;
    configure(&mut agent);
    agent.event_sink = Some(EventSink(Arc::new(move |event: &AgentEvent| {
        sink_seen.lock().unwrap().push(label(event));
    })));
    agent.prompt_user_with("Do the task", &[]);

    let mut calls = 0;
    let recorded = agent
        .run_loop(|current| {
            calls += 1;
            assert!(calls <= 8, "the loop kept asking the provider");
            // Mirrors the live closure in davinci-coding-agent/src/main.rs.
            let mut started = false;
            let mut sink = |event: &AssistantMessageEvent| {
                let partial = Arc::new(davinci_ai::assistant_to_chat(event.message()));
                if !started {
                    started = true;
                    current.emit_live(AgentEvent::MessageStart {
                        message: (*partial).clone(),
                    });
                }
                current.emit_live(AgentEvent::MessageUpdate {
                    message: partial,
                    assistant_message_event: event.clone(),
                });
            };
            let provider_messages = current.messages_for_provider();
            let system = current.provider_system_prompt();
            let result = davinci_ai::live_complete_streaming_with_sink_envelope(
                &model,
                &provider_messages,
                &auth,
                Some(&system),
                &[],
                &StreamOptions {
                    max_retries: current.provider_max_retries,
                    max_retry_delay_ms: Some(current.provider_max_retry_delay_ms),
                    timeout_ms: Some(10_000),
                    ..StreamOptions::default()
                },
                &mut sink,
            );
            result.map(|envelope| CompleteOutput {
                message: envelope.message,
                stream_events: Some(envelope.stream_events),
                native_responses_resume: None,
                streamed_live: started,
            })
        })
        .unwrap();

    let side_effect = root.path().join(SIDE_EFFECT);
    let sunk = sunk.lock().unwrap().clone();
    let bodies = server.bodies.lock().unwrap().clone();
    Outcome {
        requests: server.hits.load(Ordering::SeqCst),
        bodies,
        sunk,
        stats: agent.run_stats(),
        side_effect_exists: side_effect.exists(),
        side_effect_text: std::fs::read_to_string(&side_effect).ok(),
        recorded,
    }
}

fn describe(name: &str, outcome: &Outcome) {
    let (stop, error) = outcome.final_assistant();
    eprintln!(
        "WS3 {name}\n  requests={} retries={:?}\n  final stop={stop:?} error={error:?} exit={}\n  \
         stats: attempts={:?} retries={} usage_complete={:?} usage_unknown={:?} charged={} unknown_cost={}\n  \
         side_effect_exists={}\n  sunk={:?}\n  recorded={:?}",
        outcome.requests,
        outcome.retry_events(),
        outcome.print_mode_exit_code(),
        outcome.stats.provider_attempts,
        outcome.stats.provider_retries,
        outcome.stats.usage_complete_attempts,
        outcome.stats.usage_unknown_attempts,
        outcome.stats.budget_tokens_charged,
        outcome.stats.has_unknown_cost,
        outcome.side_effect_exists,
        outcome.sunk,
        outcome.recorded_labels(),
    );
}

/// The retry budget of a default agent, which is what print mode runs with
/// unless a settings file overrides `retry.*`.
#[test]
fn ws3_default_retry_budget_is_three_retries_and_no_http_level_retry() {
    // PINNED (diagnosis): current defaults, see `Agent::new` in lib.rs.
    let agent = Agent::new("ws3 budget");
    assert!(agent.auto_retry);
    assert_eq!(agent.retry_attempts, 3, "three retries => four attempts");
    assert_eq!(agent.retry_base_delay_ms, 2_000);
    // `None` becomes `max_retries: 0` in `ProviderRetryOptions`: the HTTP
    // layer (`retry_provider_request_controlled`) never retries by itself.
    assert_eq!(agent.provider_max_retries, None);
}

/// Both error texts a cut stream can produce are retryable. Before the fix
/// the tool-call wording matched nothing in `retry.rs` (WS3-F1).
#[test]
fn ws3_cut_tool_call_error_text_is_classified_retryable() {
    let cut_tool_call = "Stream ended before the tool call was complete";
    let cut_elsewhere = "stream ended before a terminal response event";
    assert!(davinci_ai::is_retryable_error_text(cut_tool_call));
    assert!(davinci_ai::is_retryable_error_text(cut_elsewhere));
    let message = |text: &str| -> davinci_ai::AssistantMessage {
        serde_json::from_value(json!({
            "id":"m","role":"assistant","model":"fixture","content":[],
            "stopReason":"error","errorMessage":text
        }))
        .unwrap()
    };
    assert!(davinci_ai::is_retryable_assistant_error(&message(
        cut_tool_call
    )));
    assert!(davinci_ai::is_retryable_assistant_error(&message(
        cut_elsewhere
    )));
}

/// (a) Cut while text is streaming, before any tool call starts.
#[test]
fn ws3_cut_during_text_is_retried_and_the_retry_resends_the_same_request() {
    let mut cut = vec![created()];
    cut.extend(text_item(0, "half an ans", false));
    let outcome = run(vec![Reply::Sse(frames(&cut)), final_answer()], |_| {});
    describe("(a) cut during text", &outcome);

    assert_eq!(outcome.requests, 2);
    let retries = outcome.retry_events();
    assert_eq!(retries.len(), 2);
    assert_eq!(retries[0]["type"], "auto_retry_start");
    assert_eq!(retries[0]["attempt"], 1);
    assert_eq!(retries[0]["maxAttempts"], 3);
    assert_eq!(
        retries[0]["errorMessage"],
        "stream ended before a terminal response event"
    );
    assert_eq!(retries[1]["type"], "auto_retry_end");
    assert_eq!(retries[1]["success"], true);
    assert_eq!(outcome.final_assistant(), (Some(StopReason::Stop), None));
    assert_eq!(outcome.print_mode_exit_code(), 0);

    // The retry is a full restart. The failed attempt is not added to the
    // conversation, so the second request is identical.
    assert_eq!(outcome.bodies[0], outcome.bodies[1]);

    // PINNED (diagnosis, WS3-F5, not changed): the cut attempt was already
    // streamed to the sink (start, partial text, error) and is not withdrawn;
    // the recorded list returned by `run_loop` only holds the successful
    // attempt.
    assert_eq!(outcome.count(&outcome.sunk, "update/start"), 2);
    assert_eq!(outcome.count(&outcome.sunk, "update/error"), 1);
    assert_eq!(outcome.count(&outcome.recorded_labels(), "update/start"), 1);
    assert_eq!(outcome.count(&outcome.recorded_labels(), "update/error"), 0);
}

/// The replies after a cut attempt: the whole response, then a final answer
/// for the request after the tool results and for each reminder the loop adds
/// (unverified change; requirement review when two files changed).
fn retried_whole(cut: &[Value], calls: &[Value]) -> Vec<Reply> {
    let mut whole = vec![created()];
    whole.extend(calls.iter().cloned());
    whole.push(completed());
    vec![
        Reply::Sse(frames(cut)),
        Reply::Sse(frames(&whole)),
        final_answer(),
        final_answer(),
        final_answer(),
    ]
}

const CUT_TOOL_CALL: &str = "Stream ended before the tool call was complete";

fn assert_retried_once(outcome: &Outcome, follow_ups: usize, error: &str) {
    assert_eq!(
        outcome.requests,
        2 + follow_ups,
        "cut attempt, retry, follow-ups"
    );
    let retries = outcome.retry_events();
    assert_eq!(retries[0]["type"], "auto_retry_start");
    assert_eq!(retries[0]["attempt"], 1);
    assert_eq!(retries[0]["errorMessage"], error);
    assert_eq!(retries[1]["type"], "auto_retry_end");
    assert_eq!(retries[1]["success"], true);
    assert_eq!(outcome.stats.provider_retries, 1);
    // The cut attempt left nothing in the conversation.
    assert_eq!(outcome.bodies[0], outcome.bodies[1]);
    assert_eq!(outcome.final_assistant(), (Some(StopReason::Stop), None));
    assert_eq!(outcome.print_mode_exit_code(), 0);
}

/// (b) Cut while the first tool call's arguments are streaming: retried, and
/// the call runs once, from the attempt that completed.
#[test]
fn ws3_cut_during_first_tool_call_is_retried_and_the_call_runs_once() {
    let mut cut = vec![created()];
    cut.extend(write_call(0, SIDE_EFFECT, false));
    let outcome = run(
        retried_whole(&cut, &write_call(0, SIDE_EFFECT, true)),
        |_| {},
    );
    describe("(b) cut during first tool call", &outcome);

    assert_retried_once(&outcome, 2, CUT_TOOL_CALL);
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 1);
    assert_eq!(
        outcome.side_effect_text.as_deref(),
        Some("written by a tool call")
    );
    // The cut attempt's usage stays unknown; the others are complete.
    assert_eq!(outcome.stats.provider_attempts, Some(4));
    assert_eq!(outcome.stats.usage_complete_attempts, Some(3));
    assert_eq!(outcome.stats.usage_unknown_attempts, Some(1));
}

/// (b, transport variant) The same position, but the socket dies inside an
/// HTTP chunk, so the reader reports an I/O error instead of a clean EOF.
#[test]
fn ws3_transport_error_during_first_tool_call_is_retried_as_the_same_cut() {
    let mut cut = vec![created()];
    cut.extend(write_call(0, SIDE_EFFECT, false));
    let mut replies = retried_whole(&cut, &write_call(0, SIDE_EFFECT, true));
    replies[0] = Reply::SseBrokenChunk(frames(&cut));
    let outcome = run(replies, |_| {});
    describe("(b2) transport error during first tool call", &outcome);

    // The transport's text is reported, and its typed failure (interrupted
    // transport) is what makes the attempt retryable: same retry, same
    // budget, nothing executed from the cut response.
    assert_retried_once(
        &outcome,
        2,
        "Unable to read provider response: Peer disconnected",
    );
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 1);
}

/// (c) Cut during a second tool call, after the first one completed. The
/// first call was complete on the wire in the cut attempt and must not run
/// from it: each call runs once, from the retried response.
#[test]
fn ws3_cut_during_second_tool_call_is_retried_and_each_call_runs_once() {
    let mut cut = vec![created()];
    cut.extend(write_call(0, SIDE_EFFECT, true));
    cut.extend(write_call(1, "ws3_second.txt", false));
    let mut calls = write_call(0, SIDE_EFFECT, true);
    calls.extend(write_call(1, "ws3_second.txt", true));
    let outcome = run(retried_whole(&cut, &calls), |_| {});
    describe("(c) cut during second tool call", &outcome);

    assert_retried_once(&outcome, 3, CUT_TOOL_CALL);
    // Executed twice: once per call, both from the retried response.
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 2);
    assert_eq!(
        outcome.side_effect_text.as_deref(),
        Some("written by a tool call")
    );
}

/// The retry budget still ends the run: four cut attempts, then the error
/// and exit code 1, with nothing executed.
#[test]
fn ws3_cut_tool_calls_stop_at_the_retry_budget_with_an_error() {
    let mut cut = vec![created()];
    cut.extend(write_call(0, SIDE_EFFECT, false));
    let cut = Reply::Sse(frames(&cut));
    let outcome = run(
        vec![cut.clone(), cut.clone(), cut.clone(), cut, final_answer()],
        |_| {},
    );
    describe("budget: four cut attempts", &outcome);

    assert_eq!(outcome.requests, 4, "one attempt and three retries");
    assert_eq!(outcome.stats.provider_retries, 3);
    assert_eq!(
        outcome.final_assistant(),
        (
            Some(StopReason::Error),
            Some("Stream ended before the tool call was complete".into())
        )
    );
    assert_eq!(outcome.print_mode_exit_code(), 1);
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 0);
    assert!(!outcome.side_effect_exists);
}

/// A tool call that repeats one fragment without stopping (WS3-F9) is
/// abandoned at the bound while the stream is still open, and is not retried.
#[test]
fn ws3_runaway_tool_call_is_abandoned_at_the_bound_and_not_retried() {
    let mut runaway = vec![created()];
    runaway.extend(write_call(0, SIDE_EFFECT, false));
    // The recorded fragment: an escaped newline, 49,605 times in a row.
    // Twice the bound here, to show the reader stopped before the end.
    for _ in 0..16_384 {
        runaway.push(json!({"type":"response.function_call_arguments.delta",
            "output_index":0,"delta":"\\n"}));
    }
    let outcome = run(vec![Reply::Sse(frames(&runaway)), final_answer()], |_| {});

    assert_eq!(outcome.requests, 1);
    assert!(outcome.retry_events().is_empty());
    let (stop, error) = outcome.final_assistant();
    assert_eq!(stop, Some(StopReason::Error));
    assert!(error.unwrap().contains("repeated one fragment"));
    assert_eq!(outcome.print_mode_exit_code(), 1);
    // One delta for the first half of the arguments, then the fragment 8,192
    // times: the first starts the run and the 8,193rd fails the call.
    assert_eq!(outcome.count(&outcome.sunk, "update/toolcall_delta"), 8193);
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 0);
    assert!(!outcome.side_effect_exists);
}

/// (d) Cut after a tool call completed and before the terminal event, with
/// nothing open.
#[test]
fn ws3_cut_after_a_completed_tool_call_is_retried_and_the_call_runs_once() {
    let mut cut = vec![created()];
    cut.extend(write_call(0, SIDE_EFFECT, true));
    let mut whole = cut.clone();
    whole.push(completed());
    // The loop asks again after the tool result, and once more after its own
    // "you changed files but ran no verification" reminder.
    let outcome = run(
        vec![
            Reply::Sse(frames(&cut)),
            Reply::Sse(frames(&whole)),
            final_answer(),
            final_answer(),
        ],
        |_| {},
    );
    describe("(d) cut after a completed tool call", &outcome);

    // Retried before the fix too, because no tool-call slot was open.
    assert_eq!(outcome.requests, 4, "cut attempt, retry, two follow-ups");
    let retries = outcome.retry_events();
    assert_eq!(retries[0]["type"], "auto_retry_start");
    assert_eq!(
        retries[0]["errorMessage"],
        "stream ended before a terminal response event"
    );
    assert_eq!(outcome.bodies[0], outcome.bodies[1]);
    // The call was announced twice to the sink (once per attempt) and
    // executed exactly once, from the attempt that completed.
    assert_eq!(outcome.count(&outcome.sunk, "update/toolcall_end"), 2);
    assert_eq!(outcome.count(&outcome.sunk, "tool_execution_start"), 1);
    assert_eq!(
        outcome.side_effect_text.as_deref(),
        Some("written by a tool call")
    );
    assert_eq!(outcome.print_mode_exit_code(), 0);
}

/// Usage accounting when one attempt is cut mid-stream and the retry succeeds.
#[test]
fn ws3_usage_after_a_cut_attempt_and_a_successful_retry() {
    let mut cut = vec![created()];
    cut.extend(text_item(0, "half an ans", false));
    let outcome = run(vec![Reply::Sse(frames(&cut)), final_answer()], |_| {});
    describe("usage: cut attempt then success", &outcome);

    // PINNED (diagnosis): the successful attempt's usage is kept and charged;
    // the cut attempt stays "unknown" for good. One unknown attempt is all it
    // takes for scripts/bench/bench.py to null the row's token totals.
    assert_eq!(outcome.stats.provider_attempts, Some(2));
    assert_eq!(outcome.stats.provider_retries, 1);
    assert_eq!(outcome.stats.usage_complete_attempts, Some(1));
    assert_eq!(outcome.stats.usage_unknown_attempts, Some(1));
    assert_eq!(outcome.stats.budget_tokens_charged, 129);
    assert!(outcome.stats.has_unknown_cost);

    let ends: Vec<_> = outcome
        .recorded
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderObservation { observation }
                if observation.kind == "attempt_end" =>
            {
                Some(observation.as_ref().clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[0].status, "failed");
    assert_eq!(ends[0].http_status, Some(200));
    assert!(ends[0].usage.is_none());
    assert_eq!(ends[0].usage_complete, None);
    assert_eq!(ends[1].status, "completed");
    assert_eq!(ends[1].usage_complete, Some(true));
    assert_eq!(
        ends[1].usage.as_ref().map(|usage| usage.total_tokens),
        Some(129)
    );
    // Both attempts belong to one logical request.
    assert_eq!(ends[0].logical_request_id, ends[1].logical_request_id);
}

/// Usage accounting for the benchmark's group B: an HTTP 503 before any
/// stream exists, then a successful retry.
#[test]
fn ws3_usage_after_an_http_503_and_a_successful_retry() {
    let body = r#"{"error":{"message":"The ChatGPT user is temporarily unavailable. Please try again later.","type":"service_unavailable_error","param":null,"code":"subscription_sharing_user_unavailable"}}"#;
    let outcome = run(
        vec![
            Reply::Status("503 Service Unavailable", body),
            final_answer(),
        ],
        |_| {},
    );
    describe("usage: http 503 then success", &outcome);

    // PINNED (diagnosis): a refused request that produced no stream and no
    // tokens is counted exactly like a cut one: usage unknown.
    assert_eq!(outcome.requests, 2);
    let retries = outcome.retry_events();
    assert_eq!(retries[0]["type"], "auto_retry_start");
    assert_eq!(retries[0]["maxAttempts"], 3);
    assert_eq!(retries[1]["success"], true);
    assert_eq!(outcome.print_mode_exit_code(), 0);
    assert_eq!(outcome.stats.provider_attempts, Some(2));
    assert_eq!(outcome.stats.provider_retries, 1);
    assert_eq!(outcome.stats.usage_complete_attempts, Some(1));
    assert_eq!(outcome.stats.usage_unknown_attempts, Some(1));
    assert_eq!(outcome.stats.budget_tokens_charged, 129);
    assert!(outcome.stats.has_unknown_cost);
    let failed = outcome
        .recorded
        .iter()
        .find_map(|event| match event {
            AgentEvent::ProviderObservation { observation }
                if observation.kind == "attempt_end" && observation.status == "failed" =>
            {
                Some(observation.as_ref().clone())
            }
            _ => None,
        })
        .expect("failed attempt");
    assert_eq!(failed.http_status, Some(503));
    assert!(failed.usage.is_none());
    assert_eq!(failed.usage_complete, None);
}

/// With retries turned off the budget is a single attempt, for every error.
#[test]
fn ws3_auto_retry_off_means_one_attempt_even_for_a_retryable_cut() {
    let mut cut = vec![created()];
    cut.extend(text_item(0, "half an ans", false));
    let outcome = run(vec![Reply::Sse(frames(&cut)), final_answer()], |agent| {
        agent.auto_retry = false;
    });
    describe("auto_retry off", &outcome);
    // PINNED (diagnosis).
    assert_eq!(outcome.requests, 1);
    assert!(outcome.retry_events().is_empty());
    assert_eq!(outcome.print_mode_exit_code(), 1);
}
