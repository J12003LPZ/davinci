use davinci_agent::codemode::projection::{project_script_result, validate_json};
use davinci_agent::ToolResult;
use serde_json::json;

fn result(text: &str) -> ToolResult {
    ToolResult {
        content: text.into(),
        is_error: false,
        details: Some(json!({"secret":"never projected"})),
    }
}

#[test]
fn projection_is_typed_and_excludes_unrestricted_details() {
    let value = project_script_result(
        result("authorized"),
        Some(json!({"items":[1,2]})),
        "op".into(),
        1048576,
    )
    .unwrap();
    assert!(value.complete);
    assert_eq!(value.structured_content.unwrap()["items"], json!([1, 2]));
    assert!(!serde_json::to_string(
        &project_script_result(result("authorized"), None, "op".into(), 1048576).unwrap()
    )
    .unwrap()
    .contains("secret"));
}

#[test]
fn exact_serialized_child_boundary_is_inclusive() {
    let projected = project_script_result(result("界"), None, "op".into(), 1048576).unwrap();
    let bytes = serde_json::to_vec(&projected).unwrap().len();
    assert!(project_script_result(result("界"), None, "op".into(), bytes).is_ok());
    assert_eq!(
        project_script_result(result("界"), None, "op".into(), bytes - 1)
            .unwrap_err()
            .code,
        "INCOMPLETE_DATA"
    );
}

#[test]
fn unsafe_numbers_never_become_computable_json() {
    for number in [9007199254740992u64, u64::MAX] {
        assert_eq!(
            project_script_result(
                result("number"),
                Some(json!({"number":number})),
                "op".into(),
                1048576
            )
            .unwrap_err()
            .code,
            "INCOMPLETE_DATA"
        );
    }
    assert!(validate_json(&json!({"number":9007199254740991u64}), 64).is_ok());
}

#[test]
fn tool_errors_and_oversized_structured_data_fail_explicitly() {
    let mut failed = result("denied");
    failed.is_error = true;
    assert_eq!(
        project_script_result(failed, Some(json!({"secret":true})), "op".into(), 1048576)
            .unwrap_err()
            .code,
        "TOOL_FAILED"
    );
    let error = project_script_result(
        result("large"),
        Some(json!({"text":"x".repeat(1048576)})),
        "op".into(),
        1048576,
    )
    .unwrap_err();
    assert_eq!(error.code, "INCOMPLETE_DATA");
    assert!(error.message.contains("no authorized artifact"));
}

#[test]
fn oversized_child_data_returns_authorized_artifact_instead_of_invalid_json() {
    use davinci_agent::codemode::projection::{
        project_script_result_with_artifact, serialized_size,
    };
    let store_dir = tempfile::tempdir().unwrap();
    let store = davinci_agent::EvidenceStore::new(store_dir.path());
    let structured = json!({"rows":(0..2000).map(|n| json!({"n":n})).collect::<Vec<_>>()});
    let value = project_script_result_with_artifact(
        result(&"t".repeat(20000)),
        Some(structured.clone()),
        "op".into(),
        4096,
        Some(&store),
        "child",
    )
    .unwrap();
    assert!(!value.complete);
    assert!(value.structured_content.is_none());
    assert!(serialized_size(&value, 4096).is_ok());
    assert!(value.text.len() < 4096 && value.text.chars().all(|c| c == 't'));
    let artifact = value.artifact.unwrap();
    assert!(std::path::Path::new(&artifact.id).starts_with(store_dir.path()));
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&artifact.id).unwrap()).unwrap();
    assert_eq!(saved["structuredContent"], structured);
    assert_eq!(saved["text"].as_str().unwrap().len(), 20000);
    assert_eq!(
        artifact.bytes,
        std::fs::metadata(&artifact.id).unwrap().len()
    );

    // An envelope that cannot fit even without data stays an explicit failure.
    assert_eq!(
        project_script_result_with_artifact(
            result("large"),
            None,
            "op".into(),
            8,
            Some(&store),
            "x"
        )
        .unwrap_err()
        .code,
        "INCOMPLETE_DATA"
    );
}
