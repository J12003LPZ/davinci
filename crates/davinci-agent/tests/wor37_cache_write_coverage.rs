//! WOR-37: the provider cache counters keep an unreported write count apart
//! from a measured zero.

use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};

#[test]
fn an_omitted_write_count_is_not_added_as_zero() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_with_write(100, 20, None);
    assert_eq!(cache.provider_cache_write_coverage(), (0, 1));
    assert_eq!(cache.stats().provider.cache_write_tokens, 0);
    assert_eq!(cache.stats().provider.cache_read_tokens, 20);
    let provider = &cache.stats().summary()["provider"];
    assert!(provider["cacheWriteTokens"].is_null());
    assert_eq!(provider["cacheWriteStatus"], "unreported");
    assert_eq!(provider["cacheWriteUnreportedRequests"], 1);
}

#[test]
fn a_measured_zero_counts_as_reported() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_with_write(100, 20, Some(0));
    assert_eq!(cache.provider_cache_write_coverage(), (1, 0));
    assert_eq!(cache.stats().summary()["provider"]["cacheWriteTokens"], 0);
    assert_eq!(
        cache.stats().summary()["provider"]["cacheWriteStatus"],
        "reported"
    );
}

#[test]
fn mixed_requests_sum_only_reported_writes() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage(10, 0, 7);
    cache.record_provider_usage_with_write(10, 0, None);
    cache.record_provider_usage_with_write(10, 0, Some(3));
    assert_eq!(cache.provider_cache_write_coverage(), (2, 1));
    assert_eq!(cache.stats().provider.cache_write_tokens, 10);
    assert_eq!(
        cache.stats().summary()["provider"]["cacheWriteStatus"],
        "partial"
    );
}

#[test]
fn provider_turn_preserves_unknown_cache_writes_in_summary() {
    let model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-codex-responses")
        .unwrap();
    for (write, expected, status) in [
        (None, serde_json::Value::Null, "unreported"),
        (Some(0), serde_json::json!(0), "reported"),
    ] {
        let mut agent =
            davinci_agent::Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
        agent.provider = model.provider.clone();
        agent.model_id = model.id.clone();
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("user", "Say OK."));
        let mut details = serde_json::json!({"cached_tokens":80});
        if let Some(write) = write {
            details["cache_write_tokens"] = serde_json::json!(write);
        }
        let response = serde_json::json!({"type":"response.completed","response":{
            "id":"wor37","status":"completed","output":[{"type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"OK"}]}],
            "usage":{"input_tokens":100,"output_tokens":2,"input_tokens_details":details}}});
        let corpus = format!("data: {response}\n\n");
        let mut calls = 0;
        agent
            .run_loop(|current| {
                calls += 1;
                assert_eq!(calls, 1, "unexpected extra provider request");
                Ok::<_, String>(davinci_ai::fixture_complete(
                    &model,
                    &current.messages_for_provider(),
                    &corpus,
                ))
            })
            .unwrap();
        let summary = agent.tool_context.cache.stats().summary();
        assert_eq!(summary["provider"]["cacheReadTokens"], 80);
        assert_eq!(summary["provider"]["cacheWriteTokens"], expected);
        assert_eq!(summary["provider"]["cacheWriteStatus"], status);
    }
}
