//! A usage object that omits a token count has not reported zero. The
//! subscription route's accounting must be able to tell the two apart, the
//! way it already can for cache writes (`wor37_cache_write_unreported.rs`).

use davinci_ai::{fixture_complete, load_builtin_models, Model};

fn model_for_api(api: &str) -> Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.api == api)
        .unwrap_or_else(|| panic!("catalog has a {api} model"))
}

fn responses_usage(usage: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"r\",\"status\":\"completed\",\"usage\":{usage}}}}}\n\n"
    );
    fixture_complete(&model_for_api("openai-responses"), &[], &corpus)
        .usage
        .expect("usage")
}

fn completions_usage(usage: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"hi\"}},\"finish_reason\":\"stop\"}}],\"usage\":{usage}}}\n\ndata: [DONE]\n\n"
    );
    fixture_complete(&model_for_api("openai-completions"), &[], &corpus)
        .usage
        .expect("usage")
}

#[test]
fn missing_usage_is_unknown_not_zero() {
    // Responses (the Codex route's shape): every count present.
    let full = responses_usage(
        r#"{"input_tokens":1300,"output_tokens":9,"total_tokens":1309,"input_tokens_details":{"cached_tokens":1280}}"#,
    );
    assert!(!full.input_unreported && !full.output_unreported && !full.cache_read_unreported);
    assert_eq!(full.cache_read, 1280);

    // A measured zero stays a measured zero.
    let zero = responses_usage(
        r#"{"input_tokens":1300,"output_tokens":9,"total_tokens":1309,"input_tokens_details":{"cached_tokens":0}}"#,
    );
    assert!(!zero.cache_read_unreported);
    assert_eq!(zero.cache_read, 0);

    // Each omitted count is unknown on its own.
    let no_cache =
        responses_usage(r#"{"input_tokens":1300,"output_tokens":9,"total_tokens":1309}"#);
    assert!(no_cache.cache_read_unreported);
    assert!(!no_cache.input_unreported && !no_cache.output_unreported);
    let no_output = responses_usage(
        r#"{"input_tokens":1300,"total_tokens":1300,"input_tokens_details":{"cached_tokens":0}}"#,
    );
    assert!(no_output.output_unreported);
    assert!(!no_output.input_unreported && !no_output.cache_read_unreported);
    let no_input = responses_usage(
        r#"{"output_tokens":9,"total_tokens":9,"input_tokens_details":{"cached_tokens":0}}"#,
    );
    assert!(no_input.input_unreported);
    assert!(!no_input.output_unreported && !no_input.cache_read_unreported);

    // Chat completions.
    let completions = completions_usage(r#"{"prompt_tokens":1300,"completion_tokens":9}"#);
    assert!(completions.cache_read_unreported);
    assert!(!completions.input_unreported && !completions.output_unreported);
    let completions_cached = completions_usage(
        r#"{"prompt_tokens":1300,"completion_tokens":9,"prompt_tokens_details":{"cached_tokens":0}}"#,
    );
    assert!(!completions_cached.cache_read_unreported);
}

fn anthropic_usage(start: &str, delta: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude\",\"content\":[],\"usage\":{start}}}}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{delta}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
    );
    fixture_complete(&model_for_api("anthropic-messages"), &[], &corpus)
        .usage
        .expect("usage")
}

#[test]
fn anthropic_missing_usage_is_unknown_not_zero() {
    // Everything present: nothing is unreported, and a measured zero stays one.
    let full = anthropic_usage(
        r#"{"input_tokens":25,"cache_read_input_tokens":0,"output_tokens":1}"#,
        r#"{"output_tokens":4}"#,
    );
    assert!(!full.input_unreported && !full.output_unreported && !full.cache_read_unreported);
    assert_eq!(full.cache_read, 0);
    // No cached-read count anywhere in the stream: unknown.
    let no_cache = anthropic_usage(
        r#"{"input_tokens":25,"output_tokens":1}"#,
        r#"{"output_tokens":4}"#,
    );
    assert!(no_cache.cache_read_unreported);
    assert!(!no_cache.input_unreported && !no_cache.output_unreported);
    // No input count anywhere.
    let no_input = anthropic_usage(
        r#"{"cache_read_input_tokens":0,"output_tokens":1}"#,
        r#"{"output_tokens":4}"#,
    );
    assert!(no_input.input_unreported);
    // A later delta that reports the missing field makes it known.
    let late = anthropic_usage(
        r#"{"input_tokens":25,"output_tokens":1}"#,
        r#"{"output_tokens":4,"cache_read_input_tokens":7}"#,
    );
    assert!(!late.cache_read_unreported);
    assert_eq!(late.cache_read, 7);
    // No output count anywhere.
    let no_output = anthropic_usage(
        r#"{"input_tokens":25,"cache_read_input_tokens":0}"#,
        r#"{}"#,
    );
    assert!(no_output.output_unreported);
}

#[test]
fn unreported_counts_stay_off_the_wire_when_reported() {
    let known = responses_usage(
        r#"{"input_tokens":10,"output_tokens":2,"total_tokens":12,"input_tokens_details":{"cached_tokens":0}}"#,
    );
    let json = serde_json::to_value(&known).unwrap();
    for key in ["inputUnreported", "outputUnreported", "cacheReadUnreported"] {
        assert!(json.get(key).is_none(), "{key}");
    }
    let unknown = responses_usage(r#"{"input_tokens":10}"#);
    let json = serde_json::to_value(&unknown).unwrap();
    assert_eq!(json["outputUnreported"], true);
    assert_eq!(json["cacheReadUnreported"], true);
    let back: davinci_protocol::Usage = serde_json::from_value(json).unwrap();
    assert!(back.output_unreported && back.cache_read_unreported);
    // An older stored usage without the keys still parses, as reported.
    let old: davinci_protocol::Usage = serde_json::from_value(serde_json::json!({
        "input": 1, "output": 2, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 3,
        "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0}
    }))
    .unwrap();
    assert!(!old.input_unreported && !old.output_unreported && !old.cache_read_unreported);
}
