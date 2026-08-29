use crate::cost::calculate_cost;
use crate::estimate::estimate_context_tokens;
use crate::faux::*;
use crate::models::{get_builtin_model, get_builtin_providers};
use crate::retry::is_retryable_assistant_error;
use crate::types::*;
use futures::StreamExt;

#[test]
fn test_models_catalog_loaded() {
    let providers = get_builtin_providers();
    assert!(!providers.is_empty());
    assert!(providers.contains(&"anthropic"));
    assert!(providers.contains(&"openai"));
    assert!(providers.contains(&"google"));

    let claude = get_builtin_model("anthropic", "claude-haiku-4-5");
    assert!(claude.is_some());
    let m = claude.unwrap();
    assert_eq!(m.provider, "anthropic");
    assert_eq!(m.api, "anthropic-messages");
    assert!(m.reasoning);
}

#[test]
fn test_cost_calculation() {
    let cost_rates = ModelCost {
        input: 3.0,
        output: 15.0,
        cache_read: 0.3,
        cache_write: 3.75,
        tiers: vec![],
    };
    let usage = Usage {
        input: 1_000_000,
        output: 100_000,
        cache_read: 500_000,
        cache_write: 200_000,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: 1_800_000,
        cost: UsageCost::default(),
    };
    let calc = calculate_cost(&cost_rates, &usage);
    assert!((calc.input - 3.0).abs() < 1e-6);
    assert!((calc.output - 1.5).abs() < 1e-6);
    assert!((calc.cache_read - 0.15).abs() < 1e-6);
    assert!((calc.cache_write - 0.75).abs() < 1e-6);
    assert!((calc.total - 5.4).abs() < 1e-6);
}

#[test]
fn test_retry_classification() {
    let retryable_err = faux_assistant_message(
        vec![],
        StopReason::Error,
        Some("rate limit exceeded (429)".to_string()),
    );
    assert!(is_retryable_assistant_error(&retryable_err));

    let non_retryable_err = faux_assistant_message(
        vec![],
        StopReason::Error,
        Some("insufficient_quota".to_string()),
    );
    assert!(!is_retryable_assistant_error(&non_retryable_err));

    let ok_msg = faux_assistant_message(vec![faux_text("hello")], StopReason::Stop, None);
    assert!(!is_retryable_assistant_error(&ok_msg));
}

#[test]
fn test_token_estimate() {
    let ctx = Context {
        system_prompt: Some("You are a helpful assistant".to_string()),
        messages: vec![Message::User(UserMessage {
            role: "user".to_string(),
            content: UserContent::Text("Hello world".to_string()),
            timestamp: 1000,
        })],
        tools: None,
    };
    let est = estimate_context_tokens(&ctx);
    assert!(est.tokens > 0);
}

#[tokio::test]
async fn test_faux_stream() {
    let model = Model {
        id: "faux-1".to_string(),
        name: "Faux".to_string(),
        api: "faux".to_string(),
        provider: "faux".to_string(),
        base_url: "http://localhost:0".to_string(),
        reasoning: false,
        thinking_level_map: None,
        input: vec!["text".to_string()],
        cost: ModelCost::default(),
        context_window: 10000,
        max_tokens: 1000,
        sampling_params: None,
        headers: None,
        compat: None,
    };
    let ctx = Context::default();
    let options = SimpleStreamOptions::default();

    let mut stream = crate::providers::stream_simple(&model, &ctx, &options)
        .await
        .unwrap();
    let mut got_start = false;
    let mut got_done = false;

    while let Some(event) = stream.next().await {
        match event {
            AssistantMessageEvent::Start { .. } => got_start = true,
            AssistantMessageEvent::Done { .. } => got_done = true,
            _ => {}
        }
    }

    assert!(got_start);
    assert!(got_done);
}
