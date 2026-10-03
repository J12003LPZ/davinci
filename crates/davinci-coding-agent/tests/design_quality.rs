use davinci_coding_agent::design::{records::CheckState, render::geometry_checks};
use serde_json::json;

#[test]
fn actual_geometry_findings_do_not_claim_unrun_interaction() {
    let good = json!({"nodes":[{"id":"button","tag":"button","width":100,"height":44,"visible":true,"interactive":true,"name":"Continue","focusable":true}],
        "title":"Fixture","lang":"en","overflow":false,"omitted":false,"events":[],"eventsOmitted":false});
    let checks = geometry_checks(&good).unwrap();
    assert_eq!(checks["render"].state, CheckState::Current);
    assert_eq!(checks["accessibility"].state, CheckState::Current);
    assert_eq!(checks["interaction"].state, CheckState::Pending);
    let mut broken = good;
    broken["nodes"][0]["name"] = json!("");
    broken["nodes"][0]["height"] = json!(10);
    broken["overflow"] = json!(true);
    let checks = geometry_checks(&broken).unwrap();
    assert_eq!(checks["render"].state, CheckState::Failed);
    assert_eq!(checks["accessibility"].state, CheckState::Failed);
    assert_eq!(checks["accessibility"].failures.len(), 2);
    assert!(geometry_checks(&json!({})).is_err());
}
