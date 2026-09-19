use std::sync::Arc;
use std::time::Duration;

use davinci_agent::decision::policy::{
    add_optional_capabilities, DecisionRollout, OptionalCapability,
};
use davinci_agent::decision::provider::{DecisionError, DecisionProvider};
use davinci_agent::decision::request::{DecisionQuestion, DecisionRequest};
use davinci_coding_agent::decision_providers::typesafe::{
    TypeSafeProvider, TYPESAFE_MODEL, TYPESAFE_URL,
};
use davinci_coding_agent::decision_state::DecisionState;

#[test]
fn typesafe_is_a_decision_provider_not_a_completion_model() {
    let provider = TypeSafeProvider::new("fixture-key");
    assert_eq!(provider.name(), "typesafe");
    assert_eq!(provider.model(), TYPESAFE_MODEL);
    assert_eq!(TYPESAFE_URL, "https://api.typesafe.ai/v1/systemone");
}

#[test]
fn empty_provider_credential_fails_without_a_coding_turn_dependency() {
    let provider = TypeSafeProvider::new("");
    let request = DecisionRequest::new(
        "request",
        davinci_agent::decision::risk::DecisionRisk::Planning,
        serde_json::json!({"schemaVersion":1,"task":"redacted"}),
        TYPESAFE_MODEL,
        [(
            "test_impact_relevant".to_owned(),
            DecisionQuestion::noul("test"),
        )],
    );

    let result = provider.evaluate(&request, Duration::from_millis(800));
    assert_eq!(result, Err(DecisionError::CredentialInvalid));
}

#[test]
fn decision_state_request_has_the_eight_v1_questions_and_bounded_wire_size() {
    let request = DecisionState::from_task("fix a browser test change").request(
        "request",
        davinci_agent::decision::risk::DecisionRisk::Planning,
    );
    assert_eq!(request.questions.len(), 8);
    assert!(request.questions.contains_key("browser_relevant"));
    assert!(request.questions.contains_key("verification_scope"));
    assert!(request.questions.contains_key("regression_risk"));
    assert!(request.validate_size().is_ok());
}

#[test]
fn guarded_merge_is_additive_and_keeps_deterministic_requirements() {
    let deterministic = vec!["mandatory_verification".to_owned()];
    let optional = vec![OptionalCapability::new("browser", 0.90)];
    assert_eq!(
        add_optional_capabilities(&deterministic, &optional, DecisionRollout::GuardedAdditive),
        vec!["mandatory_verification", "browser"]
    );
    assert_eq!(
        add_optional_capabilities(&deterministic, &optional, DecisionRollout::Shadow),
        deterministic
    );
}

#[test]
fn runtime_provider_can_be_reconfigured_without_sharing_global_last_answer() {
    let first = Arc::new(TypeSafeProvider::new(""));
    let runtime = davinci_agent::decision::DecisionRuntime::new(first);
    assert!(!runtime.is_enabled());
    runtime.enable();
    let generation = runtime.generation();
    runtime.replace_provider(Arc::new(TypeSafeProvider::new("")));
    assert!(runtime.generation() > generation);
    assert!(runtime.audit().snapshot().is_empty());
}
