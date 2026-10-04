//! Offline admission through the real provider entry point: no request is sent.
use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
use davinci_ai::provider_observation::{self, ObservationScope};
use davinci_ai::{live_complete_with, Model, ResolvedAuth, StreamOptions};
use std::sync::Arc;

#[test]
fn auxiliary_threads_cannot_replace_the_subscription_policy_or_root_allowance() {
    let directory = tempfile::tempdir().unwrap();
    let limits: BudgetLimits = serde_json::from_value(serde_json::json!({
        "max_requests":1,"deadline_unix_ms":u64::MAX,
        "codex_subscription":{"model":"gpt-6-luna","effort":"high"}
    }))
    .unwrap();
    let budget = RootBudget::open(directory.path().join("root.json"), "root", limits).unwrap();
    provider_observation::install_process_budget(
        "root".into(),
        "parent".into(),
        None,
        None,
        Arc::new(budget.clone()),
    )
    .unwrap();
    let model: Model = serde_json::from_value(serde_json::json!({
        "id":"gpt-6-luna","name":"fixture","api":"openai-codex-responses",
        "provider":"openai-codex","baseUrl":"https://api.openai.com/v1",
        "reasoning":true,"input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":1000,"maxTokens":100
    }))
    .unwrap();
    for purpose in ["coding", "retry", "compaction", "reviewer", "background"] {
        let model = model.clone();
        let budget = budget.clone();
        std::thread::spawn(move || {
            let scope = ObservationScope::capture_for("invented-root", "child", Some("parent"))
                .with_purpose(purpose);
            let auth = ResolvedAuth {
                api_key: Some("fixture-api-key".into()),
                source: "configured API key".into(),
                headers: Default::default(),
            };
            let options = StreamOptions {
                thinking_level: Some(davinci_protocol::ThinkingLevel::High),
                transport: Some("sse".into()),
                max_retries: Some(0),
                ..Default::default()
            };
            let error = live_complete_with(&model, &[], &auth, None, &[], &options).unwrap_err();
            assert!(error.contains("API-key fallback is disabled"), "{error}");
            assert_eq!(budget.snapshot().unwrap().requests, 0);
            assert_eq!(
                provider_observation::process_budget_binding().unwrap().0,
                budget.binding_identity()
            );
            scope.finish("failed");
        })
        .join()
        .unwrap();
    }
}
