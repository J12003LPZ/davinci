//! Deterministic orchestration scenarios: no provider calls or credentials.
use davinci_agent::{
    AgentId, AgentState, PermissionMode, RunId, RuntimeBus, RuntimeHandle, SubagentParent,
    SubagentRunner,
};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn delegation_boundary_matrix_preserves_user_control_without_extra_model_turns() {
    use davinci_agent::{Agent, CustomToolExecutor, PromptProfile, ToolResult};
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    // All completions and graph results are fixtures: no workers or providers run.
    for (provider, model) in [
        ("openai-codex", "gpt-6-astra"),
        ("openai-codex", "gpt-5.6-sol"),
        ("anthropic", "claude-sonnet-4-5"),
    ] {
        for batched in [false, true] {
            for scenario in ["forbidden", "follow-up", "relay", "reversed"] {
                let root = tempfile::tempdir().unwrap();
                let mut agent = Agent::new_builtin(PromptProfile::Stable);
                agent.cwd = root.path().into();
                agent.provider = provider.into();
                agent.model_id = model.into();
                agent.auto_compaction = false;
                agent.auto_verify = false;
                agent.requirement_review_enabled = false;
                agent.tools = vec!["batch".into(), "graph_run".into()];
                agent.set_permission_mode(PermissionMode::AlwaysApprove);
                let calls = Arc::new(AtomicUsize::new(0));
                let count = calls.clone();
                agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_, _, _| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(ToolResult {
                        content: "fixture result".into(),
                        is_error: false,
                        details: None,
                    })
                }));
                agent.prompt("Inspect the parser. Do not use subagents.");
                match scenario {
                    "follow-up" => {
                        agent.prompt("Continue the inspection.");
                    }
                    "relay" => {
                        agent.prompt_with("You can use subagents again.", &[]);
                    }
                    "reversed" => {
                        agent.prompt("You can use subagents again.");
                    }
                    _ => {}
                }
                let args = json!({"goal":"inspect the parser"});
                let (tool, args) = if batched {
                    (
                        "batch",
                        json!({"operations":[{"tool":"graph_run", "args":args}]}),
                    )
                } else {
                    ("graph_run", args)
                };
                let mut turns = 0;
                agent.run_loop(|_| {
                    turns += 1;
                    let mut reply: AssistantMessage = serde_json::from_value(json!({
                        "id":"reply", "role":"assistant", "model":model,
                        "content":[{"type":"text", "text":"Inspection complete."}], "stopReason":"stop"
                    })).unwrap();
                    if turns == 1 {
                        reply.content = vec![ContentBlock::ToolCall {
                            id:"attempt".into(), name:tool.into(), arguments:args.clone(),
                        }];
                        reply.stop_reason = Some(StopReason::ToolUse);
                    }
                    Ok(reply)
                }).unwrap();
                let case = format!("{model}/{scenario}/batch={batched}");
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    usize::from(scenario == "reversed"),
                    "{case}"
                );
                assert_eq!(
                    turns, 2,
                    "{case}: no additional completion turns are necessary"
                );
                if scenario != "reversed" {
                    assert!(
                        agent.messages.iter().any(|message| {
                            message.role == "toolResult"
                                && davinci_ai::content_text(&message.content)
                                    .contains("asked not to use subagents")
                        }),
                        "{case}: denial must reach the model"
                    );
                }
            }
        }
    }
}

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
        // The worker leaves the roster before it publishes its terminal state
        // (subagent.rs), so wait for both: leaving alone can still read Running.
        let terminal = |state: AgentState| {
            matches!(
                state,
                AgentState::Completed | AgentState::Failed | AgentState::Cancelled
            )
        };
        let started = Instant::now();
        while runtime.team.is_member(&id) || !terminal(runtime.registry.get(&id).unwrap().state) {
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
