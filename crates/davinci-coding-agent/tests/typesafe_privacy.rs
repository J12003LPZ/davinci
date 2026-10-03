use davinci_agent::decision::audit::{record_for, DecisionAuditOutcome};
use davinci_agent::decision::request::{DecisionQuestion, DecisionRequest};
use davinci_agent::decision::response::{parse_and_validate_response, DecisionAnswer};
use davinci_agent::decision::risk::DecisionRisk;
use davinci_coding_agent::decision_state::DecisionState;
use serde_json::json;

#[test]
fn state_redacts_credentials_paths_environment_values_and_source_like_file_names() {
    let state = DecisionState::from_task(
        "Fix src/login.rs using C:\\Users\\sergi\\repo\\secret.rs TYPESAFE_API_KEY=real-key and Bearer bearer-value",
    );
    let serialized = serde_json::to_string(&state).unwrap();
    for forbidden in [
        "src/login.rs",
        "C:\\Users\\sergi\\repo\\secret.rs",
        "real-key",
        "bearer-value",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "found forbidden field: {forbidden}"
        );
    }
}

#[test]
fn audit_contains_metadata_only_and_no_request_or_response_body() {
    let request = DecisionRequest::new(
        "request-id",
        DecisionRisk::Planning,
        json!({"schemaVersion":1,"task":"redacted"}),
        "jev-latest",
        [(
            "browser_relevant".to_owned(),
            DecisionQuestion::noul("browser"),
        )],
    );
    let record = record_for(
        &request,
        "typesafe",
        "jev-latest",
        5,
        DecisionAuditOutcome::Success,
    );
    let encoded = serde_json::to_string(&record).unwrap();
    assert!(encoded.contains("state_hash"));
    assert!(!encoded.contains("Authorization"));
    assert!(!encoded.contains("real-key"));
    assert!(!encoded.contains("full response"));
}

#[test]
fn response_validation_rejects_unsafe_shapes_and_accepts_bounded_noul() {
    let request = DecisionRequest::new(
        "request-id",
        DecisionRisk::Ranking,
        json!({"schemaVersion":1,"task":"redacted"}),
        "jev-latest",
        [(
            "browser_relevant".to_owned(),
            DecisionQuestion::noul("browser"),
        )],
    );
    let response = parse_and_validate_response(
        br#"{"answers":{"browser_relevant":{"type":"noul","noul":0.9}}}"#,
        &request,
    )
    .unwrap();
    assert!(matches!(
        response.answers.get("browser_relevant"),
        Some(DecisionAnswer::Noul { value }) if (*value - 0.9).abs() < 0.001
    ));
    assert!(parse_and_validate_response(
        br#"{"answers":{"browser_relevant":{"type":"unknown","value":1}}}"#,
        &request,
    )
    .is_err());
    assert!(parse_and_validate_response(
        br#"{"answers":{"browser_relevant":{"type":"noul","noul":2}}}"#,
        &request,
    )
    .is_err());
}
