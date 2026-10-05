use davinci_agent::codemode::{CodeModeLimits, CodeModeMode, CodeModeRequest, CodeModeToolValue};

#[test]
fn mode_is_default_off_and_labels_are_stable() {
    assert!(matches!(CodeModeMode::default(), CodeModeMode::Off));
    assert_eq!(
        serde_json::to_value(CodeModeMode::ReadOnly).unwrap(),
        "read-only"
    );
}

#[test]
fn requests_reject_unknown_authority_fields() {
    assert!(
        serde_json::from_value::<CodeModeRequest>(serde_json::json!({
            "code": "return 1", "timeoutMs": null, "maxOutputBytes": null,
            "mode": "controlled"
        }))
        .is_err()
    );
}

#[test]
fn envelope_retains_explicit_nulls() {
    let value = CodeModeToolValue {
        text: "ok".into(),
        structured_content: None,
        complete: true,
        artifact: None,
        operation_ref: "ephemeral:fixture".into(),
    };
    let json = serde_json::to_value(value).unwrap();
    assert!(json.get("structuredContent").unwrap().is_null());
    assert!(json.get("artifact").unwrap().is_null());
}

#[test]
fn effective_limits_cannot_exceed_hard_ceilings() {
    let limits = CodeModeLimits::default();
    assert_eq!(limits.collected_output_bytes, 1048576);
    assert_eq!(limits.wall_ms, 60000);
    let request = CodeModeRequest {
        code: "return 1".into(),
        timeout_ms: Some(300001),
        max_output_bytes: None,
    };
    assert!(limits.for_request(&request).is_err());
    let request = CodeModeRequest {
        timeout_ms: Some(1000),
        max_output_bytes: Some(4096),
        ..request
    };
    assert_eq!(limits.for_request(&request).unwrap().wall_ms, 1000);
}

#[test]
fn invalid_source_is_rejected_before_host_admission() {
    for code in [
        "",
        " \r\n\t",
        "// @codemode timeout=999999\nreturn 1",
        "// @options: timeout=999999\nreturn 1",
    ] {
        let request = CodeModeRequest {
            code: code.into(),
            timeout_ms: None,
            max_output_bytes: None,
        };
        assert!(CodeModeLimits::default().for_request(&request).is_err());
    }
}
