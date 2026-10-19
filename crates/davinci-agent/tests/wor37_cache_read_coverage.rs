//! The provider cache counters keep an unreported cached-read (or input) count
//! apart from a measured zero, as they already do for cache writes.

use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};

#[test]
fn an_omitted_read_count_is_not_added_as_zero() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_counts(Some(100), Some(20), Some(0));
    // The next request never said how much was cached. Its input was computed
    // as if nothing were, so neither number may enter the totals.
    cache.record_provider_usage_counts(Some(500), None, Some(0));
    let stats = cache.stats();
    assert_eq!(stats.provider.cache_read_tokens, 20);
    assert_eq!(stats.provider.input_tokens, 100);
    let provider = &stats.summary()["provider"];
    assert_eq!(provider["cacheReadUnreportedRequests"], 1);
    assert_eq!(provider["cacheReadTokens"], 20);
}

#[test]
fn an_omitted_input_count_is_not_added_as_zero() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_counts(None, Some(20), Some(0));
    let provider = &cache.stats().summary()["provider"];
    assert_eq!(provider["inputUnreportedRequests"], 1);
    assert_eq!(provider["cacheReadUnreportedRequests"], 0);
}

#[test]
fn provider_turn_preserves_unknown_cache_reads_in_summary() {
    let model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-codex-responses")
        .unwrap();
    for (details, unreported, read) in [
        (serde_json::json!({}), 1, 0),
        (serde_json::json!({"cached_tokens": 0}), 0, 0),
        (serde_json::json!({"cached_tokens": 80}), 0, 80),
    ] {
        let mut agent =
            davinci_agent::Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
        agent.provider = model.provider.clone();
        agent.model_id = model.id.clone();
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("user", "Say OK."));
        let response = serde_json::json!({"type":"response.completed","response":{
            "id":"r","status":"completed","output":[{"type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"OK"}]}],
            "usage":{"input_tokens":100,"output_tokens":2,"input_tokens_details":details}}});
        let corpus = format!("data: {response}\n\n");
        agent
            .run_loop(|current| {
                Ok::<_, String>(davinci_ai::fixture_complete(
                    &model,
                    &current.messages_for_provider(),
                    &corpus,
                ))
            })
            .unwrap();
        let summary = agent.tool_context.cache.stats().summary();
        assert_eq!(
            summary["provider"]["cacheReadUnreportedRequests"],
            unreported
        );
        assert_eq!(summary["provider"]["cacheReadTokens"], read);
    }
}
