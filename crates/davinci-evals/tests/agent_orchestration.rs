//! Deterministic orchestration scenarios: no provider calls or credentials.
use davinci_agent::{
    AgentId, AgentState, PermissionMode, RunId, RuntimeBus, RuntimeHandle, SubagentParent,
    SubagentRunner,
};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn background_outcome_matrix_reports_and_terminates_after_lead_cancellation() {
    for scenario in ["success", "failure", "panic", "teammate-startup-failure"] {
        let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        runtime.cancellation_token.cancel();
        let runner = SubagentRunner::new(move |request| {
            assert!(!request.cancellation_token.as_ref().unwrap().is_cancelled());
            match scenario {
                "success" => Ok("review complete".into()),
                "failure" | "teammate-startup-failure" => Err("fixture failure".into()),
                _ => panic!("fixture panic"),
            }
        });
        let result = davinci_agent::run_subagent_tool(
            &json!({"prompt":"review", "mode": if scenario == "teammate-startup-failure" { "teammate" } else { "background" }, "name":"reviewer"}),
            &["read".into()],
            Some(&runner),
            &SubagentParent {
                runtime: Some(runtime.clone()),
                cancellation_token: Some(runtime.cancellation_token.clone()),
                permission_mode: Some(PermissionMode::Ask),
                allow_async: true,
                teams_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
        let id: AgentId = result.details.unwrap()["agentId"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let started = Instant::now();
        while runtime.team.is_member(&id) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{scenario} leaked its worker"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            runtime.registry.get(&id).unwrap().state,
            if scenario == "success" {
                AgentState::Completed
            } else {
                AgentState::Failed
            }
        );
        let reports = runtime.take_labeled_messages(10);
        assert_eq!(reports.len(), 1, "{scenario}");
        assert!(reports[0].contains("from=\"reviewer\""));
        assert!(reports[0].contains(if scenario == "success" {
            "status: completed"
        } else {
            "status: failed"
        }));
    }
}
