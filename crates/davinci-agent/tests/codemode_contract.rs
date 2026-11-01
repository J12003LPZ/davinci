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
fn codex_freeform_fallback_is_safe() {
    use davinci_agent::codemode::{
        script_wire_fallback, script_wire_mode, ScriptWireCapabilities, ScriptWireMode,
    };
    // Nothing is verified on the subscription route by default.
    let unverified = ScriptWireCapabilities::default();
    assert_eq!(script_wire_mode(&unverified), ScriptWireMode::FunctionJson);
    assert_eq!(
        script_wire_fallback(ScriptWireMode::CustomFreeform, &unverified),
        ScriptWireMode::FunctionJson
    );
    assert_eq!(
        script_wire_fallback(ScriptWireMode::HostedProgrammatic, &unverified),
        ScriptWireMode::FunctionJson
    );

    // Freeform needs an observed acceptance, and hosted PTC needs both.
    let freeform = ScriptWireCapabilities {
        custom_script: true,
        ..Default::default()
    };
    assert_eq!(script_wire_mode(&freeform), ScriptWireMode::CustomFreeform);
    assert_eq!(
        script_wire_fallback(ScriptWireMode::CustomFreeform, &freeform),
        ScriptWireMode::CustomFreeform
    );
    let hosted = ScriptWireCapabilities {
        custom_script: true,
        programmatic_tool_calling: true,
    };
    assert_eq!(
        script_wire_mode(&hosted),
        ScriptWireMode::HostedProgrammatic
    );
    // Hosted PTC without freeform support is still not admitted.
    let ptc_only = ScriptWireCapabilities {
        programmatic_tool_calling: true,
        ..Default::default()
    };
    assert_eq!(script_wire_mode(&ptc_only), ScriptWireMode::FunctionJson);

    // The fallback is a retry shape, never a second effect: the JSON function
    // legacy shape stays representable in every configuration.
    for caps in [unverified, freeform, hosted, ptc_only] {
        assert_eq!(
            script_wire_fallback(ScriptWireMode::FunctionJson, &caps),
            ScriptWireMode::FunctionJson
        );
    }
}

#[test]
fn script_to_direct_switch_preserves_history_and_wire_kind() {
    use davinci_agent::codemode::{
        project_script_history, CodeModeOutcome, CodeModeStatus, SCRIPT_WIRE_KIND,
    };
    let outcome = CodeModeOutcome {
        status: CodeModeStatus::Completed,
        script_completed: true,
        output_text: "{\"summary\":\"done\"}".into(),
        output_complete: true,
        output_artifact: None,
        children: vec![],
        host_notes: vec![],
        operation_ref: "op-42".into(),
        error: None,
    };
    let read_only = project_script_history(&outcome, &CodeModeMode::ReadOnly);
    let controlled = project_script_history(&outcome, &CodeModeMode::Controlled);
    // Switching modes must not rewrite an item already in history.
    assert_eq!(read_only, controlled);
    assert_eq!(
        serde_json::to_vec(&read_only).unwrap(),
        serde_json::to_vec(&controlled).unwrap()
    );
    // The wire identity is stable and survives a record/replay round trip.
    assert_eq!(read_only.wire_kind, SCRIPT_WIRE_KIND);
    assert_eq!(read_only.wire_name, "codemode");
    assert_eq!(read_only.call_id, "op-42");
    assert_eq!(read_only.operation_ref, "op-42");
    assert_eq!(read_only.version, 1);
    let bytes = serde_json::to_vec(&read_only).unwrap();
    let restored: davinci_agent::codemode::ScriptHistoryItem =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored, read_only);
    // A truncated run says so, so a later direct turn cannot read it as whole.
    let mut partial = outcome.clone();
    partial.output_complete = false;
    assert!(!project_script_history(&partial, &CodeModeMode::Controlled).complete);
}

#[test]
fn crashed_script_never_replays_mutation() {
    use davinci_agent::codemode::{
        reconcile_script_run, ScriptRunRecord, ScriptRunRegistry, ScriptRunState,
    };
    let dir = tempfile::tempdir().unwrap();
    let registry = ScriptRunRegistry::open(dir.path()).unwrap();
    for (index, start) in [(1, ScriptRunState::Created), (2, ScriptRunState::Running)] {
        let run = format!("run-crash-{index}");
        let record =
            ScriptRunRecord::new(&run, "call-crash", Some("session-a"), "workspace-a", "rev");
        registry.create(&record).unwrap();
        if start == ScriptRunState::Running {
            registry.transition(&run, start).unwrap();
        }
        let reconciled = reconcile_script_run(&registry, &run).unwrap();
        // A host that restarted mid-run cannot resume guest JavaScript.
        assert_eq!(reconciled.state, ScriptRunState::RecoveryRequired);
        assert!(reconciled.recovered);
        assert!(reconciled
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not resumed")));
        // Reconciling again is stable: no second attempt, no replay.
        let again = reconcile_script_run(&registry, &run).unwrap();
        assert_eq!(again.state, ScriptRunState::RecoveryRequired);
        assert!(!again.recovered);
    }
    // A terminal run stays exactly as recorded.
    let record = ScriptRunRecord::new(
        "run-done",
        "call-done",
        Some("session-a"),
        "workspace-a",
        "rev",
    );
    registry.create(&record).unwrap();
    registry
        .transition("run-done", ScriptRunState::Running)
        .unwrap();
    registry
        .transition("run-done", ScriptRunState::Completed)
        .unwrap();
    let reconciled = reconcile_script_run(&registry, "run-done").unwrap();
    assert_eq!(reconciled.state, ScriptRunState::Completed);
    assert!(!reconciled.recovered);
}

#[test]
fn background_script_returns_running_and_can_finish() {
    use davinci_agent::codemode::{script_requests_background, CodeModeHost};
    use std::sync::Arc;

    struct DenyBackground;
    impl CodeModeHost for DenyBackground {
        fn execute(
            &self,
            _: &davinci_agent::codemode::CodeModeRequest,
            _: &davinci_agent::codemode::CodeModeRunContext,
            _: Arc<dyn davinci_agent::codemode::CodeModeBroker>,
        ) -> davinci_agent::codemode::CodeModeOutcome {
            unreachable!()
        }
    }
    // The directive is parsed, never inferred from a comment later in the code.
    assert!(script_requests_background(
        "// @script: {\"startInBackground\": true}\nreturn 1;"
    ));
    assert!(!script_requests_background(
        "// @script: {\"startInBackground\": false}\nreturn 1;"
    ));
    assert!(!script_requests_background(
        "const s = `\n// @script: {\"startInBackground\": true}\n`;\nreturn s;"
    ));
    // Until an owner approves the C08 exception, the host refuses to detach and
    // the script fails closed instead of silently running in the foreground.
    assert!(!DenyBackground.admits_background_scripts());
}

#[test]
fn script_state_persists_and_reconciles() {
    use davinci_agent::codemode::{ScriptRunRecord, ScriptRunRegistry, ScriptRunState};
    let dir = tempfile::tempdir().unwrap();
    let registry = ScriptRunRegistry::open(dir.path()).unwrap();
    let mut record = ScriptRunRecord::new(
        "run-1",
        "call-1",
        Some("session-a"),
        "workspace-a",
        "revision-a",
    );
    record.branch_origin = Some("entry-a".into());
    registry.create(&record).unwrap();
    let on_branch: std::collections::HashSet<String> = ["entry-a".to_string()].into();
    let off_branch: std::collections::HashSet<String> = ["entry-b".to_string()].into();
    assert_eq!(
        registry.get("run-1").unwrap().state,
        ScriptRunState::Created
    );
    assert!(registry
        .transition("run-1", ScriptRunState::Running)
        .unwrap());
    assert!(registry
        .transition("run-1", ScriptRunState::Completed)
        .unwrap());

    // State is identical after a cold reload.
    let reopened = ScriptRunRegistry::open(dir.path()).unwrap();
    assert_eq!(
        reopened.get("run-1").unwrap().state,
        ScriptRunState::Completed
    );
    // Transitions are monotonic: a terminal run cannot restart.
    assert!(reopened
        .transition("run-1", ScriptRunState::Running)
        .is_err());
    // A duplicate completion is suppressed, not duplicated.
    assert!(!reopened
        .transition("run-1", ScriptRunState::Completed)
        .unwrap());

    // Handles are scoped to their session and workspace.
    assert!(reopened
        .get_for("run-1", Some("session-b"), "workspace-a", &on_branch)
        .is_err());
    assert!(reopened
        .get_for("run-1", Some("session-a"), "workspace-b", &on_branch)
        .is_err());
    assert!(reopened
        .get_for("run-1", Some("session-a"), "workspace-a", &on_branch)
        .is_ok());
    // A run whose origin is off the current branch is denied and also drops out of wakeups.
    let other_branch = reopened
        .get_for("run-1", Some("session-a"), "workspace-a", &off_branch)
        .unwrap_err();
    assert_eq!(other_branch.code, "DENIED");
    assert!(reopened
        .pending_notifications(Some("session-a"), "workspace-a", &off_branch)
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened
            .pending_notifications(Some("session-a"), "workspace-a", &on_branch)
            .unwrap()
            .len(),
        1
    );
    // A forged or foreign id never resolves.
    assert!(reopened.get("run-../../etc/passwd").is_err());
    assert!(reopened.get("run-missing").is_err());

    // A write-ahead recovery state can still resolve to a terminal outcome.
    assert!(reopened
        .transition("run-1", ScriptRunState::RecoveryRequired)
        .is_err());
}

#[test]
fn script_log_is_readable_without_reexecuting_commands() {
    use davinci_agent::codemode::{ScriptLogEvent, ScriptLogStore};
    let dir = tempfile::tempdir().unwrap();
    let store = ScriptLogStore::new(dir.path(), "session-a");
    let events = vec![
        ScriptLogEvent::status("run started"),
        ScriptLogEvent::child("exec_command", "cargo test", "last 200 lines\nall failed"),
    ];
    let log = store.persist_script_log("run-1", &events).unwrap();
    let slice = store.read_script_log(&log.id, 0, 4096).unwrap();
    assert!(slice.text.contains("last 200 lines"), "{slice:?}");
    assert!(slice.text.contains("cargo test"), "{slice:?}");

    // A log handle is session-bound: another session cannot read it.
    let other = ScriptLogStore::new(dir.path(), "session-b");
    assert!(other.read_script_log(&log.id, 0, 64).is_err());

    // Read slices are bounded and offset-addressable.
    let slice = store.read_script_log(&log.id, 0, 8).unwrap();
    assert!(slice.text.len() <= 8, "{slice:?}");
    assert!(!slice.complete, "{slice:?}");

    // Secrets are scrubbed before retention.
    let log = store
        .persist_script_log(
            "run-2",
            &[ScriptLogEvent::output(
                "token sk-abcdef1234567890 and password=hunter2",
            )],
        )
        .unwrap();
    let text = store.read_script_log(&log.id, 0, 4096).unwrap().text;
    assert!(!text.contains("sk-abcdef"), "{text}");
    assert!(!text.contains("hunter2"), "{text}");

    // Secrets are scrubbed in the retained log.
    let log = store
        .persist_script_log(
            "run-4",
            &[ScriptLogEvent::output(
                "line one\nsk-abcdef1234567890\nBearer abc",
            )],
        )
        .unwrap();
    let text = store.read_script_log(&log.id, 0, 4096).unwrap().text;
    assert!(!text.contains("sk-abcdef"), "{text}");
    assert!(!text.contains("Bearer abc"), "{text}");
    assert!(text.contains("line one"), "{text}");

    // Retention is capped regardless of what the script emitted.
    let huge = ScriptLogEvent::output("x".repeat(2 * 1024 * 1024));
    let log = store.persist_script_log("run-3", &[huge]).unwrap();
    assert!(log.bytes <= 1024 * 1024, "{log:?}");
}

#[test]
fn droid_profile_caps_are_hard() {
    let limits = CodeModeLimits::default();
    // The published safe profile. These are ceilings, not targets to raise.
    assert_eq!(limits.script_bytes, 65536);
    assert_eq!(limits.wall_ms, 60000);
    assert_eq!(limits.tool_calls, 64);
    assert_eq!(limits.parallelism, 4);
    assert_eq!(limits.output_bytes, 16384);
    assert_eq!(limits.output_events, 1024);
    assert_eq!(limits.input_bytes, 1048576);
    // A request may lower a limit but never raise one.
    let request = CodeModeRequest {
        code: "return 1".into(),
        timeout_ms: Some(300000),
        max_output_bytes: Some(65536),
    };
    let effective = limits.for_request(&request).unwrap();
    assert!(effective.wall_ms <= limits.wall_ms);
    assert!(effective.output_bytes <= limits.output_bytes);
    assert_eq!(effective.tool_calls, limits.tool_calls);
    assert_eq!(effective.output_events, limits.output_events);
    assert_eq!(effective.input_bytes, limits.input_bytes);
}

#[test]
fn no_runtime_requirement_without_codemode() {
    // Codemode is off unless a trusted host stages it; nothing here spawns Node.
    assert!(matches!(CodeModeMode::default(), CodeModeMode::Off));
}

#[test]
fn script_execute_reports_exit_and_test_failures() {
    use davinci_agent::codemode::projection::adapt_script_tool_result;
    use davinci_agent::ToolResult;

    // A failing `cargo test` is a normal tool result, not a protocol failure:
    // the guest must see the exit code and the error lines.
    let failed = ToolResult {
        content: "test result: FAILED. 1 passed; 1 failed\n  left: 1\n right: 2".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":101})),
    };
    let value =
        adapt_script_tool_result("exec_command", failed, None, "op-fixture".into(), 1 << 20)
            .unwrap();
    assert_eq!(
        value.structured_content.as_ref().unwrap()["exitCode"],
        serde_json::json!(101)
    );
    assert!(value.text.contains("test result: FAILED"));
    assert_eq!(value.operation_ref, "op-fixture");
    assert!(value.complete);

    // A spawn or policy failure stays an error, with its text preserved.
    let error = adapt_script_tool_result(
        "exec_command",
        ToolResult {
            content: "Permission denied: disallowed command".into(),
            is_error: true,
            details: None,
        },
        None,
        "op-denied".into(),
        1 << 20,
    )
    .unwrap_err();
    assert_eq!(error.code, "TOOL_FAILED");
    assert!(error.message.contains("disallowed command"));

    // A read tool keeps its text-only shape; no invented command metadata.
    let read = ToolResult {
        content: "contents".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":101})),
    };
    let value = adapt_script_tool_result("read", read, None, "op-read".into(), 1 << 20).unwrap();
    assert!(value.structured_content.is_none());
}

#[test]
fn unknown_mutation_never_auto_replays() {
    use davinci_agent::codemode::{resolve_script_effect, ScriptEffectResolution};
    use davinci_agent::runtime::operations::{EffectStatus, OperationState};

    // An ambiguous effect is never success and never a retry, whatever the
    // operation state claims.
    for effect in [EffectStatus::Possible, EffectStatus::Unknown] {
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
    // An observed effect whose command failed is still committed: the exit
    // status is the result, not ambiguity.
    let observed = resolve_script_effect(OperationState::Failed, EffectStatus::EffectsObserved);
    assert!(matches!(observed, ScriptEffectResolution::Succeeded { .. }));
    assert!(!observed.requires_postcondition());
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
fn custom_freeform_input_maps_to_code_without_widening_unknown_fields() {
    let custom = CodeModeRequest::from_tool_arguments(&serde_json::json!({
        "input": "return 1"
    }))
    .expect("custom freeform call");
    assert_eq!(custom.code, "return 1");
    assert_eq!(custom.timeout_ms, None);

    let function = CodeModeRequest::from_tool_arguments(&serde_json::json!({
        "code": "return 2", "timeoutMs": 1000
    }))
    .expect("function call");
    assert_eq!(function.code, "return 2");
    assert_eq!(function.timeout_ms, Some(1000));

    for rejected in [
        serde_json::json!({"input": "return 1", "mode": "controlled"}),
        serde_json::json!({"input": 1}),
        serde_json::json!({"code": "return 1", "input": "return 1"}),
        serde_json::json!("return 1"),
    ] {
        assert!(
            CodeModeRequest::from_tool_arguments(&rejected).is_none(),
            "{rejected}"
        );
    }
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

#[test]
fn concurrent_registry_handles_never_lose_or_duplicate_a_transition() {
    use davinci_agent::codemode::{ScriptRunRecord, ScriptRunRegistry, ScriptRunState};
    let dir = tempfile::tempdir().unwrap();
    let registry = ScriptRunRegistry::open(dir.path()).unwrap();
    for run in ["shared", "other"] {
        registry
            .create(&ScriptRunRecord::new(run, "call", Some("s"), "w", "rev"))
            .unwrap();
        registry.transition(run, ScriptRunState::Running).unwrap();
    }
    let path = dir.path().to_path_buf();
    let handles: Vec<_> = (0..16)
        .map(|index| {
            let path = path.clone();
            std::thread::spawn(move || {
                // Each thread has its own handle, like a clone of the agent.
                let registry = ScriptRunRegistry::open(&path).unwrap();
                let won = registry
                    .transition("shared", ScriptRunState::Completed)
                    .unwrap();
                registry
                    .create(&ScriptRunRecord::new(
                        &format!("own-{index}"),
                        "call",
                        Some("s"),
                        "w",
                        "rev",
                    ))
                    .unwrap();
                registry.mark_notified("other").unwrap();
                won
            })
        })
        .collect();
    let winners = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|won| *won)
        .count();
    assert_eq!(winners, 1);
    assert_eq!(
        registry.get("shared").unwrap().state,
        ScriptRunState::Completed
    );
    assert!(registry.get("other").unwrap().notified);
    let leftovers = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

/// Hold `path` the way a live host process holds its owner lock, through a
/// separate handle: `flock` is per open file description, and Windows opens
/// with share mode 0.
fn hold_exclusively(path: &std::path::Path) -> std::fs::File {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // LOCK_EX | LOCK_NB. A sweep probing the same file can briefly hold it,
        // so retry before failing.
        let locked = (0..10).any(|attempt| {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            unsafe { libc::flock(file.as_raw_fd(), 2 | 4) == 0 }
        });
        assert!(locked, "the lock is free to take");
    }
    file
}

#[test]
fn first_open_recovers_only_runs_a_dead_process_left_open() {
    use davinci_agent::codemode::{ScriptRunRecord, ScriptRunRegistry, ScriptRunState};
    let dir = tempfile::tempdir().unwrap();
    let write = |run: &str, owner: Option<String>, state: ScriptRunState| {
        let mut record = ScriptRunRecord::new(run, "call", Some("s"), "w", "rev");
        record.owner = owner;
        record.state = state;
        std::fs::write(
            dir.path().join(format!("{run}.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    };
    // Owner liveness is a held lock, not a pid: a stale lock file with nobody
    // holding it is dead, and so is an owner with no lock file at all.
    // Owner locks live in the shared agent directory, so ids are unique per run.
    let owners = owner_locks_dir();
    let tag = uuid::Uuid::new_v4().simple();
    let dead_path = owners.join(format!("1-dead-{tag}.lock"));
    let live_path = owners.join(format!("2-live-{tag}.lock"));
    std::fs::write(&dead_path, b"").unwrap();
    let dead = Some(format!("1-dead-{tag}"));
    // Another host that is alive: a different handle holds its owner lock.
    let live_lock = hold_exclusively(&live_path);
    let live = Some(format!("2-live-{tag}"));
    write("dead-running", dead.clone(), ScriptRunState::Running);
    write("legacy-running", None, ScriptRunState::Running);
    write("live-running", live, ScriptRunState::Running);
    write("dead-done", dead, ScriptRunState::Completed);

    let registry = ScriptRunRegistry::open(dir.path()).unwrap();
    let state = |run: &str| registry.get(run).unwrap().state;
    assert_eq!(state("dead-running"), ScriptRunState::RecoveryRequired);
    assert_eq!(state("legacy-running"), ScriptRunState::RecoveryRequired);
    assert_eq!(state("live-running"), ScriptRunState::Running);
    assert_eq!(state("dead-done"), ScriptRunState::Completed);
    assert!(!dead_path.exists(), "stale lock removed");

    // The live owner dies later: resolving the single record recovers it, and
    // does not wait for another first open.
    let record = registry.get("live-running").unwrap();
    registry.reconcile_if_owner_dead(&record);
    assert_eq!(state("live-running"), ScriptRunState::Running);
    drop(live_lock);
    registry.reconcile_if_owner_dead(&record);
    assert_eq!(state("live-running"), ScriptRunState::RecoveryRequired);

    // A run this process creates afterwards is never claimed by a later open.
    registry
        .create(&ScriptRunRecord::new("mine", "call", Some("s"), "w", "rev"))
        .unwrap();
    registry
        .transition("mine", ScriptRunState::Running)
        .unwrap();
    let again = ScriptRunRegistry::open(dir.path()).unwrap();
    assert_eq!(again.get("mine").unwrap().state, ScriptRunState::Running);
    let _ = std::fs::remove_file(&live_path);
}

fn owner_locks_dir() -> std::path::PathBuf {
    let owners = davinci_session::default_agent_dir().join("script-owners");
    std::fs::create_dir_all(&owners).unwrap();
    owners
}

#[test]
fn a_registry_directory_can_be_deleted_after_a_record_is_created() {
    use davinci_agent::codemode::{ScriptRunRecord, ScriptRunRegistry};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("script-runs");
    let registry = ScriptRunRegistry::open(&root).unwrap();
    registry
        .create(&ScriptRunRecord::new("run", "call", Some("s"), "w", "rev"))
        .unwrap();
    // No lock file is held inside the directory, so Windows can delete it.
    std::fs::remove_dir_all(&root).unwrap();
    assert!(!root.exists());
}

#[test]
fn a_stale_owner_lock_counts_as_dead_and_is_removed() {
    use davinci_agent::codemode::{ScriptRunRecord, ScriptRunRegistry, ScriptRunState};
    let dir = tempfile::tempdir().unwrap();
    let owner = format!("1-stale-{}", uuid::Uuid::new_v4().simple());
    let lock = owner_locks_dir().join(format!("{owner}.lock"));
    std::fs::write(&lock, b"").unwrap();
    let mut record = ScriptRunRecord::new("stale", "call", Some("s"), "w", "rev");
    record.owner = Some(owner);
    record.state = ScriptRunState::Running;
    std::fs::write(
        dir.path().join("stale.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let registry = ScriptRunRegistry::open(dir.path()).unwrap();
    assert_eq!(
        registry.get("stale").unwrap().state,
        ScriptRunState::RecoveryRequired
    );
    assert!(!lock.exists(), "stale lock removed");
}
