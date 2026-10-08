//! WOR-37: a provider that omits its cache-write count is "unreported", not a
//! measured zero.

use davinci_ai::{fixture_complete, load_builtin_models, Model};

fn model_for_api(api: &str) -> Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.api == api)
        .unwrap_or_else(|| panic!("catalog has a {api} model"))
}

fn responses_usage(details: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"r\",\"status\":\"completed\",\"usage\":{{\"input_tokens\":1300,\"output_tokens\":9,\"total_tokens\":1309,\"input_tokens_details\":{details}}}}}}}\n\n"
    );
    fixture_complete(&model_for_api("openai-responses"), &[], &corpus)
        .usage
        .expect("usage")
}

fn completions_usage(details: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"hi\"}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1300,\"completion_tokens\":9,\"total_tokens\":1309,\"prompt_tokens_details\":{details}}}}}\n\ndata: [DONE]\n\n"
    );
    fixture_complete(&model_for_api("openai-completions"), &[], &corpus)
        .usage
        .expect("usage")
}

fn anthropic_usage(creation: &str) -> davinci_protocol::Usage {
    let corpus = format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude\",\"content\":[],\"usage\":{{\"input_tokens\":25{creation},\"cache_read_input_tokens\":10,\"output_tokens\":1}}}}}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":4}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
    );
    fixture_complete(&model_for_api("anthropic-messages"), &[], &corpus)
        .usage
        .expect("usage")
}

#[test]
fn responses_without_a_write_count_is_unreported() {
    let usage = responses_usage(r#"{"cached_tokens":1280}"#);
    assert!(usage.cache_write_unreported);
    assert_eq!(usage.cache_read, 1280);
}

#[test]
fn responses_with_an_explicit_zero_is_a_measured_zero() {
    let usage = responses_usage(r#"{"cached_tokens":1280,"cache_write_tokens":0}"#);
    assert!(!usage.cache_write_unreported);
    assert_eq!(usage.cache_write, 0);
}

#[test]
fn responses_with_a_write_count_keeps_it() {
    let usage = responses_usage(r#"{"cached_tokens":100,"cache_write_tokens":500}"#);
    assert!(!usage.cache_write_unreported);
    assert_eq!(usage.cache_write, 500);
}

#[test]
fn completions_distinguish_omitted_from_zero() {
    assert!(completions_usage(r#"{"cached_tokens":1000}"#).cache_write_unreported);
    let zero = completions_usage(r#"{"cached_tokens":1000,"cache_write_tokens":0}"#);
    assert!(!zero.cache_write_unreported);
    assert_eq!(zero.cache_write, 0);
}

#[test]
fn anthropic_distinguishes_omitted_from_zero() {
    assert!(anthropic_usage("").cache_write_unreported);
    let zero = anthropic_usage(",\"cache_creation_input_tokens\":0");
    assert!(!zero.cache_write_unreported);
    assert_eq!(zero.cache_write, 0);
}

#[test]
fn unreported_flag_stays_off_the_wire_when_false() {
    let usage = responses_usage(r#"{"cached_tokens":1,"cache_write_tokens":0}"#);
    let json = serde_json::to_value(&usage).unwrap();
    assert!(json.get("cacheWriteUnreported").is_none());
    let unknown = responses_usage(r#"{"cached_tokens":1}"#);
    let json = serde_json::to_value(&unknown).unwrap();
    assert_eq!(json["cacheWriteUnreported"], true);
    let back: davinci_protocol::Usage = serde_json::from_value(json).unwrap();
    assert!(back.cache_write_unreported);
}
