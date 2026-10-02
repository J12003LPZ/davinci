//! Host binding covers thread-local scopes and the actual outgoing request.
use davinci_ai::provider_observation::{
    self, AttemptBudget, ObservationScope, ProviderAttemptObservation,
};
use davinci_ai::{live_complete_with, ChatMessage, Model, ModelCost, ResolvedAuth, StreamOptions};
use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct OneRequest(AtomicUsize);
impl AttemptBudget for OneRequest {
    fn reserve(
        &self,
        event: &ProviderAttemptObservation,
        output: Option<u64>,
    ) -> Result<(), String> {
        assert_eq!(event.root_id.as_deref(), Some("approved-root"));
        assert_eq!(output, Some(17));
        self.0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| "fixture root allowance exhausted".into())
    }
    fn reconcile(&self, _: &ProviderAttemptObservation) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn host_limit_reaches_the_wire_and_cannot_be_replaced_by_a_worker_scope() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    provider_observation::install_process_budget(
        "approved-root".into(),
        "lead".into(),
        None,
        17,
        Arc::new(OneRequest(AtomicUsize::new(0))),
    )
    .unwrap();
    let model = Model {
        id: "fixture".into(),
        name: "fixture".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: Some(format!("http://{address}")),
        reasoning: false,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 1000,
        max_tokens: 50,
        compat: serde_json::Value::Null,
        headers: Default::default(),
        thinking_level_map: Default::default(),
    };
    let auth = ResolvedAuth {
        api_key: Some("fixture".into()),
        headers: Default::default(),
        source: "fixture".into(),
    };
    let options = StreamOptions {
        max_tokens: Some(500),
        max_retries: Some(0),
        ..Default::default()
    };
    let error = live_complete_with(&model, &[], &auth, None, &[], &options).unwrap_err();
    assert!(error.contains("no observation scope"), "{error}");
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "provider never sent request"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let body = loop {
            let mut buffer = [0; 8192];
            let n = socket.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let header = std::str::from_utf8(&bytes[..split]).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    break serde_json::from_slice::<serde_json::Value>(
                        &bytes[split + 4..split + 4 + length],
                    )
                    .unwrap();
                }
            }
        };
        assert_eq!(body["max_completion_tokens"], 17);
        let reply = r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}}"#;
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
    });
    let scope = ObservationScope::capture();
    live_complete_with(
        &model,
        &[ChatMessage::text("user", "done")],
        &auth,
        None,
        &[],
        &options,
    )
    .unwrap();
    assert!(scope
        .finish("completed")
        .iter()
        .any(|event| event.kind == "attempt_end"));
    server.join().unwrap();
    std::thread::spawn(move || {
        let scope = ObservationScope::capture_for("invented-root", "child", None);
        let error = live_complete_with(&model, &[], &auth, None, &[], &options).unwrap_err();
        assert!(error.contains("allowance exhausted"), "{error}");
        scope.finish("failed");
    })
    .join()
    .unwrap();
}
