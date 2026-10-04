use davinci_ai::{
    complete_simple, load_builtin_models, ContentBlock, ResolvedAuth, StopReason, StreamOptions,
};
use davinci_protocol::ThinkingLevel;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

#[test]
fn responses_simple_completion_preserves_compaction_options() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
                Err(error) => panic!("accept provider request: {error}"),
            }
        };
        // Windows accepted sockets can inherit the listener's nonblocking mode.
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0, "request ended before its headers");
            request.extend_from_slice(&buffer[..count]);
            if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        while request.len() < header_end + length {
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0, "request ended before its body");
            request.extend_from_slice(&buffer[..count]);
        }
        let bytes = &request[header_end..header_end + length];
        let bytes = if headers.contains("content-encoding: zstd") {
            zstd::stream::decode_all(bytes).unwrap()
        } else {
            bytes.to_vec()
        };
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let (status, content_type, response) = if body["stream"] == true {
            ("200 OK", "text/event-stream", concat!(
                "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"Preserved audit code\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4,\"total_tokens\":16}}}\n\n"
            ))
        } else {
            (
                "200 OK",
                "application/json",
                "{\"id\":\"resp_fixture\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Preserved audit code\",\"annotations\":[]}]}],\"usage\":{\"input_tokens\":12,\"output_tokens\":4}}",
            )
        };
        write!(socket, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        body
    });
    let mut model = load_builtin_models()
        .into_iter()
        .find(|model| model.api == "openai-codex-responses")
        .unwrap();
    model.id = "gpt-6-luna".into();
    model.provider = "openai".into();
    model.api = "openai-responses".into();
    model.base_url = Some(format!("http://{address}"));
    let result = complete_simple(
        &model,
        "Summarize the conversation",
        Some("Preserve the audit code"),
        &ResolvedAuth {
            api_key: Some("fixture-api-key".into()),
            headers: Default::default(),
            source: "test".into(),
        },
        &StreamOptions {
            thinking_level: Some(ThinkingLevel::Low),
            transport: Some("sse".into()),
            cache_retention: Some("none".into()),
            timeout_ms: Some(5000),
            ..Default::default()
        },
    );
    let body = server.join().unwrap();
    assert_eq!(
        body["stream"], false,
        "complete_simple uses the non-streaming public Responses shape"
    );
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["instructions"], "Preserve the audit code");
    assert!(body.get("prompt_cache_key").is_none());
    let message = result.unwrap();
    assert_eq!(message.stop_reason, Some(StopReason::Stop));
    assert!(
        matches!(&message.content[0], ContentBlock::Text { text } if text == "Preserved audit code")
    );
    assert_eq!(message.usage.unwrap().total_tokens, 16);
}
