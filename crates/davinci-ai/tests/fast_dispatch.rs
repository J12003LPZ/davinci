//! No provider request is allowed in this regression test.
#[test]
fn review_saved_fast_is_rejected_before_any_transport_for_unsupported_model() {
    const CHILD: &str = "DAVINCI_REVIEW_FAST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let mut model = davinci_ai::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == "openai-codex")
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        model.base_url = Some(format!("http://{}", listener.local_addr().unwrap()));
        let dir = std::path::PathBuf::from(std::env::var_os("DAVINCI_CODING_AGENT_DIR").unwrap());
        std::fs::write(
            dir.join("codex-models.json"),
            serde_json::to_vec(&serde_json::json!({
                "models": [{"slug": model.id, "service_tiers": []}]
            }))
            .unwrap(),
        )
        .unwrap();
        let error = davinci_ai::live_complete_streaming_with_sink_envelope(
            &model,
            &[],
            &davinci_ai::ResolvedAuth {
                api_key: None,
                headers: Default::default(),
                source: "oauth".into(),
            },
            None,
            &[],
            &davinci_ai::StreamOptions {
                service_tier: Some(davinci_ai::CodexServiceTier::Fast),
                timeout_ms: Some(10),
                max_retries: Some(0),
                ..Default::default()
            },
            &mut |_| {},
        )
        .err()
        .expect("unsupported Fast must fail before dispatch");
        assert!(error.contains("Fast is not advertised"), "{error}");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "review_saved_fast_is_rejected_before_any_transport_for_unsupported_model",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("DAVINCI_CODING_AGENT_DIR", dir.path())
        .env("CODEX_HOME", dir.path().join("empty-codex"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
