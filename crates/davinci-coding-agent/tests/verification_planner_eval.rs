//! Deterministic acceptance evals for verification-plan boundedness and evidence identity.

use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use serde_json::json;
use std::sync::Arc;

#[test]
fn every_planned_step_is_explicitly_unexecuted_and_source_bound() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("settings.json"),
        r#"{"verificationPlanner":{"enabled":true,"maxSteps":8}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/lib.rs"), "pub fn value() -> u8 { 1 }").unwrap();
    let mut host = NativeExtensionHost::new_with_agent_dir(
        "verification-eval",
        root.path(),
        Some(state.path()),
    );
    host.verification_planner
        .set_permissions(Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))));
    let result = host
        .execute_tool(
            root.path(),
            "verification_plan",
            &json!({"files":["src/lib.rs"],"forceFull":true}),
        )
        .unwrap()
        .details
        .unwrap();
    let identity = result["sourceIdentity"].as_str().unwrap();
    let steps = result["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    assert!(steps.len() <= 8);
    for step in steps {
        assert_eq!(step["status"], "planned");
        assert_eq!(step["sourceIdentity"], identity);
        assert!(step["program"].is_string() || step["program"].is_null());
    }
    assert_eq!(result["complete"], false);
}
