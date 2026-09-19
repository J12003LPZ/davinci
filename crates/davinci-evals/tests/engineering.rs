use davinci_agent::AgentEvent;
use davinci_evals::behavior::trace::{
    BehaviorEvent, BehaviorStats, BehaviorTrace, VerificationEvent,
};
use davinci_evals::engineering::{
    empty_metrics, observe_behavior_trace, scenario_receipt_from_trace, EngineeringCategory,
    EngineeringEvalManifest, EngineeringMetric, EngineeringRunMode, EngineeringWorkflowStep,
    MetricReceipt, ScenarioReceipt, ENGINEERING_SCHEMA_VERSION, REQUIRED_FEATURE_FLAGS,
};
use serde_json::json;
use std::collections::BTreeMap;

fn manifest() -> EngineeringEvalManifest {
    let feature_flags = REQUIRED_FEATURE_FLAGS
        .iter()
        .map(|name| ((*name).to_string(), true))
        .collect::<BTreeMap<_, _>>();
    let scenarios = EngineeringCategory::ALL
        .into_iter()
        .enumerate()
        .map(|(index, category)| ScenarioReceipt {
            category,
            mode: EngineeringRunMode::OfflineNormal,
            passed: true,
            workflow_steps: EngineeringWorkflowStep::ALL
                .into_iter()
                .enumerate()
                .filter_map(|(step_index, step)| {
                    (step_index % EngineeringCategory::ALL.len() == index).then_some(step)
                })
                .collect(),
            evidence: vec!["offline-fixture-receipt".into()],
            failure_reasons: Vec::new(),
        })
        .collect();
    let metrics = EngineeringMetric::ALL
        .into_iter()
        .map(|metric| (metric, MetricReceipt::observed(1.0, "count", "fixture")))
        .collect();
    EngineeringEvalManifest {
        schema_version: ENGINEERING_SCHEMA_VERSION,
        run_id: "run-1".into(),
        fixture_id: "fixture-1".into(),
        source_identity: "sha256:fixture".into(),
        toolchain: "rustc-1.83/node-24".into(),
        mode: EngineeringRunMode::OfflineNormal,
        feature_flags,
        categories: EngineeringCategory::ALL.into(),
        workflow_steps: EngineeringWorkflowStep::ALL.into(),
        metrics,
        scenarios,
        planted_failures: vec!["stale_write".into()],
    }
}

#[test]
fn complete_offline_manifest_requires_all_coverage_and_metrics() {
    assert!(manifest().validate().is_ok());
}

#[test]
fn missing_measurements_are_null_with_reasons_and_never_zero_success() {
    let metrics = empty_metrics();
    let receipt = metrics
        .get(&EngineeringMetric::BrowserVerificationSuccess)
        .unwrap();
    assert!(receipt.value.is_none());
    assert!(!receipt.observed);
    assert!(receipt.reason.as_deref().unwrap().contains("not collected"));

    let mut incomplete = manifest();
    incomplete.metrics = metrics;
    assert!(incomplete.validate().is_ok());

    incomplete.metrics.insert(
        EngineeringMetric::CiSuccess,
        MetricReceipt {
            value: None,
            unit: "status".into(),
            source: "fixture".into(),
            reason: None,
            observed: false,
        },
    );
    let errors = incomplete.validate().unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.contains("missing metric CiSuccess")));
}

#[test]
fn incomplete_coverage_and_unexplained_failures_are_rejected() {
    let mut invalid = manifest();
    invalid.categories.pop();
    invalid.workflow_steps.pop();
    invalid.scenarios.pop();
    invalid.scenarios[0].passed = false;
    invalid.scenarios[0].failure_reasons.clear();
    invalid.feature_flags.remove("hook_policy");
    let errors = invalid.validate().unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.contains("seventeen required steps")));
    assert!(errors
        .iter()
        .any(|error| error.contains("twelve required categories")));
    assert!(errors
        .iter()
        .any(|error| error.contains("feature flag 'hook_policy'")));
    assert!(errors
        .iter()
        .any(|error| error.contains("needs a failure reason")));
}

#[test]
fn executable_workflow_coverage_cannot_be_missing() {
    let mut invalid = manifest();
    invalid.scenarios[0].workflow_steps.clear();
    let errors = invalid.validate().unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.contains("executable workflow-step coverage")));
    assert!(errors
        .iter()
        .any(|error| error.contains("executable coverage for all seventeen workflow steps")));
}

#[test]
fn scenarios_cannot_mix_normal_graph_or_live_modes() {
    let mut invalid = manifest();
    invalid.scenarios[0].mode = EngineeringRunMode::OfflineGraph;
    let errors = invalid.validate().unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.contains("does not match manifest mode")));
}

#[test]
fn trace_observer_preserves_unavailable_metrics_and_reports_missing_steps() {
    let mut trace = BehaviorTrace::new("partial-login", None);
    trace.events.extend([
        BehaviorEvent::Read {
            path: "src/app.ts".into(),
            start: None,
            end: None,
        },
        BehaviorEvent::Edit {
            path: "src/app.ts".into(),
        },
        BehaviorEvent::Shell {
            command_class: "test".into(),
            exit_code: Some(0),
        },
        BehaviorEvent::CapabilityIdentity {
            capability: "package_api".into(),
        },
    ]);
    trace.verification.push(VerificationEvent {
        kind: "test".into(),
        command: "npm test -- login".into(),
        passed: true,
    });
    trace.stats = BehaviorStats {
        tool_calls: 4,
        output_tokens: 12,
        ..BehaviorStats::default()
    };

    let observation = observe_behavior_trace(&trace);
    assert!(observation
        .workflow_steps
        .contains(&EngineeringWorkflowStep::RepoMap));
    assert!(observation
        .workflow_steps
        .contains(&EngineeringWorkflowStep::PackageApi));
    assert!(observation
        .workflow_steps
        .contains(&EngineeringWorkflowStep::TargetedTests));
    assert_eq!(
        observation.metrics[&EngineeringMetric::ToolCalls].value,
        Some(4.0)
    );
    assert_eq!(
        observation.metrics[&EngineeringMetric::OutputTokens].value,
        Some(12.0)
    );
    assert!(observation.metrics[&EngineeringMetric::InputTokens]
        .reason
        .as_deref()
        .unwrap()
        .contains("not emitted"));
    assert!(
        observation.metrics[&EngineeringMetric::BrowserVerificationSuccess]
            .value
            .is_none()
    );

    let receipt = scenario_receipt_from_trace(
        EngineeringCategory::FrontendBugFixing,
        EngineeringRunMode::OfflineNormal,
        &trace,
        vec!["trace://partial-login".into()],
    );
    assert!(!receipt.passed);
    assert!(receipt
        .failure_reasons
        .iter()
        .any(|reason| reason.contains("workflow steps not observed")));
}

#[test]
fn trace_observer_accepts_only_explicit_complete_workflow_receipts() {
    let mut trace = BehaviorTrace::new("complete-login", None);
    for step in EngineeringWorkflowStep::ALL {
        let name = serde_json::to_value(step)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        trace.events.push(BehaviorEvent::PlanEvent {
            kind: format!("workflow:{name}"),
        });
    }
    trace.events.push(BehaviorEvent::FinalResponse);
    trace.verification.push(VerificationEvent {
        kind: "test".into(),
        command: "npm test -- login".into(),
        passed: true,
    });

    let receipt = scenario_receipt_from_trace(
        EngineeringCategory::BrowserVerification,
        EngineeringRunMode::OfflineNormal,
        &trace,
        vec!["trace://complete-login".into()],
    );
    assert!(receipt.passed);
    assert_eq!(
        receipt.workflow_steps.len(),
        EngineeringWorkflowStep::ALL.len()
    );
    assert!(receipt.failure_reasons.is_empty());
}

#[test]
fn native_tool_events_are_labeled_for_engineering_observation() {
    let trace = BehaviorTrace::from_agent_events(
        "native-tools",
        None,
        &[AgentEvent::ToolExecutionStart {
            tool_call_id: "git".into(),
            tool_name: "git_branch_diff".into(),
            args: json!({"base": "main"}),
        }],
        None,
    );
    assert!(trace.events.iter().any(|event| matches!(
        event,
        BehaviorEvent::CapabilityIdentity { capability } if capability == "git_intelligence"
    )));
    assert!(trace.events.iter().any(|event| matches!(
        event,
        BehaviorEvent::CapabilityIdentity { capability } if capability == "git_branch_diff"
    )));
}

#[test]
fn diagnostics_capability_does_not_claim_symbol_discovery() {
    let trace = BehaviorTrace::from_agent_events(
        "diagnostics-only",
        None,
        &[AgentEvent::ToolExecutionStart {
            tool_call_id: "diagnostics".into(),
            tool_name: "lsp_diagnostics".into(),
            args: json!({"path": "src/lib.rs"}),
        }],
        None,
    );

    let observation = observe_behavior_trace(&trace);
    assert!(observation
        .workflow_steps
        .contains(&EngineeringWorkflowStep::LspDiagnostics));
    assert!(!observation
        .workflow_steps
        .contains(&EngineeringWorkflowStep::LspSymbols));
}

#[test]
fn process_start_metric_counts_observed_native_starts() {
    let trace = BehaviorTrace::from_agent_events(
        "two-process-starts",
        None,
        &[
            AgentEvent::ToolExecutionStart {
                tool_call_id: "start-1".into(),
                tool_name: "process_start".into(),
                args: json!({"argv": ["npm", "run", "dev"]}),
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "start-2".into(),
                tool_name: "process_start".into(),
                args: json!({"argv": ["npm", "run", "dev"]}),
            },
        ],
        None,
    );

    let observation = observe_behavior_trace(&trace);
    assert_eq!(
        observation.metrics[&EngineeringMetric::ProcessStartups].value,
        Some(2.0)
    );
}

#[test]
fn login_button_native_tool_events_cover_all_seventeen_steps() {
    let tools = [
        ("repo_map", json!({})),
        ("lsp_document_symbols", json!({"path": "src/login.ts"})),
        ("package_info", json!({"package": "login-app"})),
        (
            "git_blame_symbol",
            json!({"symbol": "loginButtonLabel", "path": "src/login.ts"}),
        ),
        ("impact_analyze", json!({"files": ["src/login.ts"]})),
        (
            "process_start",
            json!({"executable": "node", "argv": ["-e", "console.log('READY')"]}),
        ),
        (
            "workspace_checkpoint",
            json!({"path": "src/login.ts", "label": "login"}),
        ),
        (
            "edit",
            json!({"path": "src/login.ts", "oldText": "Broken", "newText": "Login"}),
        ),
        ("lsp_diagnostics", json!({"path": "src/login.ts"})),
        ("test_plan", json!({"path": "src/login.ts"})),
        ("bash", json!({"command": "node --test src/login.test.ts"})),
        ("build_command", json!({"files": ["src/login.ts"]})),
        ("browser_open", json!({"port": 1})),
        ("browser_snapshot", json!({"browser_id": "ctx"})),
        ("browser_console", json!({"browser_id": "ctx"})),
        ("workspace_diff", json!({"checkpointId": "cp"})),
        ("verification_plan", json!({"files": ["src/login.ts"]})),
    ];
    let mut events = Vec::new();
    for (index, (name, args)) in tools.iter().enumerate() {
        events.push(AgentEvent::ToolExecutionStart {
            tool_call_id: format!("s{index}"),
            tool_name: (*name).into(),
            args: args.clone(),
        });
        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: format!("s{index}"),
            tool_name: (*name).into(),
            result: json!({"ok": true}),
            is_error: false,
            details: None,
        });
    }
    let trace = BehaviorTrace::from_agent_events("login-button", None, &events, None);
    let observation = observe_behavior_trace(&trace);
    let missing: Vec<_> = EngineeringWorkflowStep::ALL
        .into_iter()
        .filter(|step| !observation.workflow_steps.contains(step))
        .collect();
    assert!(
        missing.is_empty(),
        "missing workflow steps {missing:?} observed={:?}",
        observation.workflow_steps
    );
}
