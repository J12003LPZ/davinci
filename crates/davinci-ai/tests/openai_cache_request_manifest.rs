use davinci_ai::{
    load_builtin_models, request_body_with, ChatMessage, PreparedProviderRequest, StreamOptions,
};

fn sol() -> davinci_ai::Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai" && model.id == "gpt-5.6-sol")
        .expect("gpt-5.6-sol")
}

#[test]
fn real_responses_builder_manifest_finds_first_changed_tail_segment() {
    let model = sol();
    let options = StreamOptions {
        cache_key: Some("partition".into()),
        cache_retention: Some("short".into()),
        ..StreamOptions::default()
    };
    let first = request_body_with(
        &model,
        &[ChatMessage::text("user", "tail one")],
        Some("stable bootstrap"),
        &[],
        &options,
    );
    let second = request_body_with(
        &model,
        &[ChatMessage::text("user", "tail two")],
        Some("stable bootstrap"),
        &[],
        &options,
    );
    let first = PreparedProviderRequest::new(first);
    let second = PreparedProviderRequest::new(second);

    assert!(first.first_changed_cache_segment(&second).is_some());
    assert_ne!(
        first.manifest().ordered_prefix_fingerprint,
        second.manifest().ordered_prefix_fingerprint
    );
    assert!(first
        .manifest()
        .segments
        .iter()
        .any(|segment| segment.cache_sensitive));
}

#[test]
fn cache_partition_change_is_routing_not_prefix_content() {
    let model = sol();
    let a = request_body_with(
        &model,
        &[ChatMessage::text("user", "same")],
        Some("stable bootstrap"),
        &[],
        &StreamOptions {
            cache_key: Some("a".into()),
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    let b = request_body_with(
        &model,
        &[ChatMessage::text("user", "same")],
        Some("stable bootstrap"),
        &[],
        &StreamOptions {
            cache_key: Some("b".into()),
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    let a = PreparedProviderRequest::new(a);
    let b = PreparedProviderRequest::new(b);
    assert_eq!(a.first_changed_cache_segment(&b), None);
    assert_eq!(
        a.manifest().ordered_prefix_fingerprint,
        b.manifest().ordered_prefix_fingerprint
    );
}

#[test]
fn prepared_manifest_is_deterministic_content_free_and_noninterfering() {
    let secret = "SYNTHETIC_PRIVATE_BEARER_COOKIE_PATH_REASONING";
    let model = sol();
    let body = request_body_with(
        &model,
        &[ChatMessage::text("user", secret)],
        Some(secret),
        &[davinci_ai::ToolSpec {
            name: "fixture_read".into(),
            description: secret.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": secret}}
            }),
            constrained_sampling: None,
        }],
        &StreamOptions {
            cache_key: Some(secret.into()),
            thinking_level: Some(davinci_protocol::ThinkingLevel::High),
            ..StreamOptions::default()
        },
    );
    let original_bytes = serde_json::to_vec(&body).unwrap();
    let prepared = PreparedProviderRequest::new(body.clone());
    let repeated = PreparedProviderRequest::new(body.clone());
    assert_eq!(prepared.body(), &body);
    assert_eq!(serde_json::to_vec(prepared.body()).unwrap(), original_bytes);
    assert_eq!(prepared.manifest(), repeated.manifest());
    let metadata = serde_json::to_string(prepared.manifest()).unwrap();
    assert!(!metadata.contains(secret));
    assert!(!metadata.contains("fixture_read"));
    assert!(prepared.manifest().request_bytes_before_compression > 0);

    let absent = PreparedProviderRequest::new(serde_json::json!({}));
    assert_eq!(absent.body(), &serde_json::json!({}));
    assert!(absent.manifest().segments.is_empty());
}
