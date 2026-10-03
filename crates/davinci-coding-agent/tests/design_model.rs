use davinci_agent::{
    runtime::capacity::{BudgetLimits, RootBudget},
    Agent,
};
use davinci_ai::ResolvedAuth;
use davinci_coding_agent::design::model::{DesignModel, DesignModelRequest, SubscriptionModel};
use std::sync::{atomic::AtomicBool, Arc};

#[test]
fn offline_mode_blocks_configured_design_routes_before_credentials_are_read() {
    const CHILD: &str = "DAVINCI_DESIGN_OFFLINE_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let error = SubscriptionModel::configured(&Agent::new("offline fixture"))
            .err()
            .expect("offline design must be denied");
        assert!(error.to_string().contains("offline"), "{error}");
        let (_dir, agent, model, auth) = route_fixture();
        let mut route = SubscriptionModel::from_host(&agent, model, auth).unwrap();
        let error = route
            .complete(
                &agent,
                &DesignModelRequest {
                    messages: vec![],
                    tools: vec![],
                    deadline_ms: u64::MAX,
                    abort: Arc::new(AtomicBool::new(false)),
                },
            )
            .unwrap_err();
        assert!(error.contains("offline"), "{error}");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "offline_mode_blocks_configured_design_routes_before_credentials_are_read",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("PI_OFFLINE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn route_fixture() -> (tempfile::TempDir, Agent, davinci_ai::Model, ResolvedAuth) {
    let dir = tempfile::tempdir().unwrap();
    let model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex")
        .unwrap();
    let mut agent = Agent::new("route fixture");
    agent.provider = model.provider.clone();
    agent.model_id = model.id.clone();
    agent
        .bind_root_budget(
            RootBudget::open(
                dir.path().join("root.json"),
                "route-fixture",
                BudgetLimits {
                    max_requests: 12,
                    max_output_tokens: Some(1000),
                    max_cost_microusd: None,
                    codex_subscription: None,
                    deadline_unix_ms: u64::MAX,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let auth = ResolvedAuth {
        api_key: None,
        headers: Default::default(),
        source: "oauth".into(),
    };
    (dir, agent, model, auth)
}

#[test]
fn api_key_routes_endpoint_overrides_and_model_switches_are_rejected() {
    for cause in ["api_key", "endpoint", "headers", "model", "effort"] {
        let (_dir, mut agent, mut model, mut auth) = route_fixture();
        if cause == "effort" {
            let route = SubscriptionModel::from_host(&agent, model, auth).unwrap();
            agent.thinking_level =
                if agent.request_thinking_level() == davinci_protocol::ThinkingLevel::High {
                    davinci_protocol::ThinkingLevel::Low
                } else {
                    davinci_protocol::ThinkingLevel::High
                };
            assert!(route.validate(&agent).is_err());
            continue;
        }
        match cause {
            "api_key" => auth.source = "api_key".into(),
            "endpoint" => model.base_url = Some("https://example.invalid".into()),
            "headers" => {
                model.headers.insert("x-override".into(), "fixture".into());
            }
            "model" => agent.model_id = "different-model".into(),
            _ => unreachable!(),
        }
        assert!(
            SubscriptionModel::from_host(&agent, model, auth).is_err(),
            "{cause}"
        );
    }
}
