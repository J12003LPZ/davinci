use pi_ai::auth::{Credential, CredentialStore, InMemoryCredentialStore};
use pi_ai::cost::calculate_cost;
use pi_ai::models::{default_anthropic_model, default_openai_model, Models};
use pi_ai::types::{Context, Usage};
use pi_ai::utils::estimate_context_tokens;
use std::sync::Arc;

#[tokio::test]
async fn test_auth_store_in_memory() {
    let store = InMemoryCredentialStore::new();
    store
        .write(
            "anthropic",
            Credential::ApiKey {
                key: "sk-ant-test".to_string(),
                env: None,
            },
        )
        .await;

    let cred = store.read("anthropic").await.expect("read credential");
    match cred {
        Credential::ApiKey { key, .. } => assert_eq!(key, "sk-ant-test"),
        _ => panic!("Expected ApiKey"),
    }
}

#[test]
fn test_cost_calculation() {
    let model = default_anthropic_model();
    let mut usage = Usage {
        input: 1_000_000,
        output: 100_000,
        cache_read: 500_000,
        cache_write: 200_000,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: 1_800_000,
        cost: Default::default(),
    };
    calculate_cost(&model, &mut usage);
    assert!((usage.cost.input - 3.0).abs() < 1e-6);
    assert!((usage.cost.output - 1.5).abs() < 1e-6);
    assert!((usage.cost.cache_read - 0.15).abs() < 1e-6);
    assert!((usage.cost.cache_write - 0.75).abs() < 1e-6);
    assert!((usage.cost.total - 5.4).abs() < 1e-6);
}

#[tokio::test]
async fn test_models_stream_mock() {
    let store = Arc::new(InMemoryCredentialStore::new());
    let models = Models::new(store);
    let model = default_openai_model();
    let context = Context::default();

    let stream = models.stream_simple(&model, &context, None);
    let result = stream.result().await.expect("assistant message result");
    assert_eq!(result.role, "assistant");
    assert_eq!(result.stop_reason, pi_ai::types::StopReason::Stop);
    assert!(!result.content.is_empty());
}

#[test]
fn test_estimate_context_tokens() {
    let context = Context {
        system_prompt: Some("You are a helper".to_string()),
        messages: vec![],
        tools: None,
    };
    let tokens = estimate_context_tokens(&context);
    assert!(tokens > 0);
}
