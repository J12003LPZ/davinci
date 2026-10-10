//! WOR-37: `cache-status` reports unknown, not 0, for unreported cache writes.

use davinci_coding_agent::native_extensions::NativeExtensionHost;

fn status(record: impl FnOnce(&NativeExtensionHost)) -> serde_json::Value {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("wor37", root.path(), Some(state.path()));
    record(&host);
    let status = host.command("cache-status", "").unwrap().unwrap();
    for key in [
        "cacheWriteTokens",
        "cacheWriteStatus",
        "cacheWriteReportedRequests",
        "cacheWriteUnreportedRequests",
    ] {
        assert_eq!(
            status["provider"][key], status["summary"]["provider"][key],
            "{key}"
        );
    }
    status["provider"].clone()
}

#[test]
fn omitted_write_count_reports_null_and_unreported() {
    let provider = status(|host| host.cache.record_provider_usage_with_write(100, 30, None));
    assert!(provider["cacheWriteTokens"].is_null());
    assert_eq!(provider["cacheWriteStatus"], "unreported");
    assert_eq!(provider["cacheWriteUnreportedRequests"], 1);
    assert_eq!(provider["cacheReadTokens"], 30);
}

#[test]
fn measured_zero_reports_zero() {
    let provider = status(|host| {
        host.cache
            .record_provider_usage_with_write(100, 30, Some(0))
    });
    assert_eq!(provider["cacheWriteTokens"], 0);
    assert_eq!(provider["cacheWriteStatus"], "reported");
}

#[test]
fn mixed_requests_report_a_partial_lower_bound() {
    let provider = status(|host| {
        host.cache.record_provider_usage_with_write(100, 0, Some(4));
        host.cache.record_provider_usage_with_write(100, 0, None);
    });
    assert_eq!(provider["cacheWriteTokens"], 4);
    assert_eq!(provider["cacheWriteStatus"], "partial");
}

#[test]
fn no_requests_is_not_called_reported() {
    let provider = status(|_| {});
    assert_eq!(provider["cacheWriteStatus"], "none");
}

#[test]
fn http_provider_turn_distinguishes_unknown_writes_from_measured_zero() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    for (write, expected, expected_status) in [
        (None, serde_json::Value::Null, "unreported"),
        (Some(0), serde_json::json!(0), "reported"),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "provider request never arrived");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let body = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() < 1024 * 1024);
                if let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&request[..split]).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= split + 4 + length {
                        break serde_json::from_slice::<serde_json::Value>(
                            &request[split + 4..split + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            assert_eq!(body["model"], "gpt-6-luna");
            assert_eq!(body["stream"], true);
            let mut details = serde_json::json!({"cached_tokens":80});
            if let Some(write) = write {
                details["cache_write_tokens"] = serde_json::json!(write);
            }
            let event = serde_json::json!({"type":"response.completed","response":{
                "id":"wor37-http","status":"completed","output":[{"type":"message","role":"assistant",
                    "content":[{"type":"output_text","text":"OK"}]}],
                "usage":{"input_tokens":100,"output_tokens":2,"input_tokens_details":details}}});
            let response = format!("data: {event}\n\n");
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut host =
            NativeExtensionHost::new_with_agent_dir("wor37-http", root.path(), Some(state.path()));
        let mut model = davinci_ai::load_builtin_models()
            .into_iter()
            .find(|model| model.api == "openai-responses")
            .unwrap();
        model.id = "gpt-6-luna".into();
        model.provider = "fixture".into();
        model.base_url = Some(format!("http://{address}"));
        let auth = davinci_ai::ResolvedAuth {
            api_key: None,
            headers: Default::default(),
            source: "fixture".into(),
        };
        let mut agent =
            davinci_agent::Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
        agent.cwd = root.path().to_path_buf();
        agent.provider = model.provider.clone();
        agent.model_id = model.id.clone();
        agent.prompt("Say OK.");
        let runtime = agent.runtime.clone().expect("prompt installs the runtime");
        agent.set_runtime(runtime.with_cache(host.cache.clone()));
        let mut calls = 0;
        let result = agent.run_loop(|current| {
            calls += 1;
            assert_eq!(calls, 1);
            davinci_ai::live_complete_streaming_with_sink_envelope(
                &model,
                &current.messages_for_provider(),
                &auth,
                Some(&current.provider_system_prompt()),
                &[],
                &davinci_ai::StreamOptions {
                    max_retries: Some(0),
                    timeout_ms: Some(5000),
                    thinking_level: Some(davinci_protocol::ThinkingLevel::Medium),
                    ..Default::default()
                },
                &mut |_| {},
            )
            .map(|envelope| {
                assert_eq!(
                    envelope
                        .message
                        .usage
                        .as_ref()
                        .unwrap()
                        .cache_write_unreported,
                    write.is_none(),
                    "transport decoder lost cache-write availability"
                );
                envelope.message
            })
        });
        server.join().unwrap();
        result.unwrap();
        assert_eq!(
            agent.tool_context.cache.provider_cache_write_coverage(),
            if write.is_none() { (0, 1) } else { (1, 0) },
            "agent usage recording lost availability"
        );
        assert_eq!(
            host.cache.provider_cache_write_coverage(),
            agent.tool_context.cache.provider_cache_write_coverage(),
            "test host and agent do not share the same cache runtime"
        );
        let status = host.command("cache-status", "").unwrap().unwrap();
        for provider in [&status["provider"], &status["summary"]["provider"]] {
            assert_eq!(provider["cacheWriteTokens"], expected);
            assert_eq!(provider["cacheWriteStatus"], expected_status);
            assert_eq!(provider["cacheReadTokens"], 80);
        }
    }
}
