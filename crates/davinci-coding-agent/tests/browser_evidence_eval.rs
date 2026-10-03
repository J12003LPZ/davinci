//! Offline evidence classification, not real-browser acceptance.
use davinci_coding_agent::interaction_testing::{
    browser::{BrowserBridgeEvent as Event, ManagedBrowserSession},
    BackendKind,
};
use serde_json::json;

#[test]
fn browser_fixture_evidence_classification_eval() {
    let mut rows = Vec::new();
    for (case, expected_pass, events) in [
        ("valid_dom", true, Vec::new()),
        (
            "failed_dom",
            false,
            vec![Event::AssertionResult {
                selector: "#login".into(),
                passed: false,
                actual: "Still logged out".into(),
            }],
        ),
        (
            "console_error",
            false,
            vec![Event::ConsoleError {
                message: "Uncaught login bug".into(),
            }],
        ),
        (
            "http_error",
            false,
            vec![Event::NetworkFailure {
                url: "http://127.0.0.1:43123/api/login".into(),
                status: 500,
                failure_text: "server error".into(),
            }],
        ),
        ("event_overflow", false, vec![Event::ActionDone; 1000]),
        (
            "oversized_snapshot",
            false,
            vec![Event::DomSnapshot {
                html: "x".repeat(1024 * 1024),
            }],
        ),
    ] {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        session.handle_bridge_event(Event::AssertionResult {
            selector: "#login".into(),
            passed: true,
            actual: "Welcome".into(),
        });
        for event in events {
            session.handle_bridge_event(event);
        }
        let receipt = session.build_receipt(case).unwrap();
        assert_eq!(receipt.backend_kind, BackendKind::FixtureOnly);
        assert_eq!(receipt.assertions_passed, expected_pass, "{case}");
        assert_eq!(
            receipt.exit_outcome,
            Some(if expected_pass { 0 } else { 1 })
        );
        assert!(receipt.event_log.len() <= 128);
        rows.push(json!({"case":case,"expected_pass":expected_pass,"observed_pass":receipt.assertions_passed}));
    }
    let artifact = json!({
        "schema":1,"scope":"offline fixture evidence classification; no browser frontend claim",
        "cases":rows,"false_successes":0,"missed_valid_cases":0,"classification":"fixture_only",
    });
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
    if let Some(path) = std::env::var_os("DAVINCI_BROWSER_EVIDENCE_EVAL_ARTIFACT") {
        std::fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}
