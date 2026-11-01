use davinci_agent::codemode::{
    CodeModeHost, CodeModeLimits, CodeModeMode, CodeModeOutcome, CodeModeRequest, CodeModeToolValue,
};
use davinci_agent::Agent;
use std::sync::Arc;

struct FixedHost {
    controlled: bool,
}

impl CodeModeHost for FixedHost {
    fn execute(
        &self,
        _: &CodeModeRequest,
        _: &davinci_agent::codemode::CodeModeRunContext,
        _: Arc<dyn davinci_agent::codemode::CodeModeBroker>,
    ) -> CodeModeOutcome {
        unreachable!("no runtime reaches the host in this fixture")
    }

    fn admits_controlled_mode(&self) -> bool {
        self.controlled
    }
}

#[test]
fn malformed_script_headers_are_rejected_before_dispatch() {
    let request = |code: &str| CodeModeRequest {
        code: code.into(),
        timeout_ms: None,
        max_output_bytes: None,
    };
    for bad in [
        "// @input: patch <<'PATCH'\nalpha\nreturn 1;",
        "// @input: a <<'T'\nx\nT\n// @input: a <<'U'\ny\nU\nreturn 1;",
        "// @input: a <<T\nx\nT\nreturn 1;",
    "// @input: __proto__ <<'T'\nx\nT\nreturn 1;",
        "// @script: {\"startInBackground\": true}\n// @script: {\"startInBackground\": true}\nreturn 1;",
        "// @script: {\"startInBackground\": \"yes\"}\nreturn 1;",
        "// @script: not-json\nreturn 1;",
    ] {
        assert!(
            CodeModeLimits::default().for_request(&request(bad)).is_err(),
            "{bad:?}"
        );
    }
    // A well-formed header stays admissible and its trailing newline is data.
    assert!(CodeModeLimits::default()
        .for_request(&request(
            "// @input: patch <<'PATCH'\nalpha\nPATCH\n// @script: {\"startInBackground\": true}\nreturn inputs.patch;"
        ))
        .is_ok());
}

#[test]
fn unknown_mutation_never_auto_replays() {
    use davinci_agent::codemode::{resolve_script_effect, ScriptEffectResolution};
    use davinci_agent::runtime::operations::{EffectStatus, OperationState};

    // An ambiguous effect is never success and never a retry, whatever the
    // operation state claims.
    for effect in [
        EffectStatus::Possible,
        EffectStatus::Unknown,
        EffectStatus::EffectsObserved,
    ] {
        for state in [
            OperationState::Running,
            OperationState::EffectPossible,
            OperationState::Interrupted,
            OperationState::RecoveryRequired,
            OperationState::Failed,
        ] {
            let resolution = resolve_script_effect(state, effect);
            assert!(
                matches!(resolution, ScriptEffectResolution::RecoveryRequired { .. }),
                "{state:?}/{effect:?} -> {resolution:?}"
            );
            assert!(!resolution.may_retry(), "{resolution:?}");
            assert!(resolution.requires_postcondition(), "{resolution:?}");
        }
    }
    // A committed effect is a success, not a replay.
    let committed = resolve_script_effect(OperationState::Succeeded, EffectStatus::EffectsObserved);
    assert!(matches!(
        committed,
        ScriptEffectResolution::Succeeded { .. }
    ));
    assert!(!committed.may_retry());
    // Only a proven no-effect may be issued again.
    for (state, effect) in [
        (OperationState::Failed, EffectStatus::NotStarted),
        (OperationState::Cancelled, EffectStatus::NotStarted),
        (OperationState::Failed, EffectStatus::KnownNoEffect),
    ] {
        let resolution = resolve_script_effect(state, effect);
        assert!(matches!(
            resolution,
            ScriptEffectResolution::NoEffect { .. }
        ));
        assert!(resolution.may_retry());
        assert!(!resolution.requires_postcondition());
    }
}

#[test]
fn mode_is_default_off_and_labels_are_stable() {
    assert!(matches!(CodeModeMode::default(), CodeModeMode::Off));
    assert_eq!(
        serde_json::to_value(CodeModeMode::ReadOnly).unwrap(),
        "read-only"
    );
}

#[test]
fn droid_patch_shorthand_never_rewrites_native_arguments() {
    use davinci_agent::codemode::projection::adapt_script_arguments;
    assert_eq!(
        adapt_script_arguments("apply_patch", serde_json::json!({"patch":"P"})).unwrap(),
        serde_json::json!({"input":"P"})
    );
    // Native arguments pass through unchanged: the adapter never replaces a
    // complete DaVinci-shaped call.
    assert_eq!(
        adapt_script_arguments("apply_patch", serde_json::json!({"input":"P"})).unwrap(),
        serde_json::json!({"input":"P"})
    );
    assert_eq!(
        adapt_script_arguments("write", serde_json::json!({"path":"a","content":"b"})).unwrap(),
        serde_json::json!({"path":"a","content":"b"})
    );
    for bad in [serde_json::json!({}), serde_json::json!({"patch":7})] {
        assert!(adapt_script_arguments("apply_patch", bad).is_err());
    }
}

#[test]
fn controlled_mode_is_opt_in_and_host_owned() {
    let mut agent = Agent::new("codemode mode fixture");
    // Off is the only implicit state and never stages a host.
    assert!(agent
        .enable_codemode(Arc::new(FixedHost { controlled: true }), CodeModeMode::Off)
        .is_ok());
    assert!(!agent.has_staged_codemode());

    // A host that does not admit controlled may not be escalated into it.
    let error = agent
        .enable_codemode(
            Arc::new(FixedHost { controlled: false }),
            CodeModeMode::Controlled,
        )
        .unwrap_err();
    assert_eq!(error.code, "DENIED");
    assert!(error.message.contains("controlled"), "{error:?}");

    // Read-only keeps its existing admission requirement unchanged.
    let error = agent
        .enable_codemode(
            Arc::new(FixedHost { controlled: false }),
            CodeModeMode::ReadOnly,
        )
        .unwrap_err();
    assert_eq!(error.code, "UNAVAILABLE");
    assert!(error.message.contains("journal"), "{error:?}");

    // Even with an admitting host, controlled needs the trusted session
    // journal; a missing trusted host denies instead of defaulting.
    let error = agent
        .enable_codemode(
            Arc::new(FixedHost { controlled: true }),
            CodeModeMode::Controlled,
        )
        .unwrap_err();
    assert!(error.message.contains("journal"), "{error:?}");

    // A JavaScript header is never a control channel.
    assert!(
        serde_json::from_value::<CodeModeRequest>(serde_json::json!({
            "code": "// @mode: controlled\nreturn 1",
            "mode": "controlled"
        }))
        .is_err()
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
        "\n  // a note first\n// @options: timeout=1\nreturn 1",
    ] {
        let request = CodeModeRequest {
            code: code.into(),
            timeout_ms: None,
            max_output_bytes: None,
        };
        assert!(CodeModeLimits::default().for_request(&request).is_err());
    }
}

#[test]
fn directive_text_after_the_header_is_ordinary_content() {
    for code in [
        "const example = `\n// @options: this is just text\n`;\nreturn example;",
        "return 1;\n// @codemode is mentioned in a trailing comment",
        "// an ordinary leading comment\nreturn 1",
    ] {
        let request = CodeModeRequest {
            code: code.into(),
            timeout_ms: None,
            max_output_bytes: None,
        };
        assert!(
            CodeModeLimits::default().for_request(&request).is_ok(),
            "{code:?}"
        );
    }
}
