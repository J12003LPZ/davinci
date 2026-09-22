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
