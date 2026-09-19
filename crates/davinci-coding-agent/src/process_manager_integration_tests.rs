//! Real normal-session dispatch and settings, without a provider or Graph.
use super::{
    attach_shared_tool_executor, attach_tool_executor, build_agent, Agent, Args, ExtensionHost,
};
use davinci_agent::{
    jobs::supervisor::SupervisorCommand, process_manager::ProcessManager, PermissionMode,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
fn helper_entry() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        davinci_agent::jobs::supervisor::run();
    }
}

#[test]
fn normal_agent_process_settings_discovery_reuse_output_and_shutdown() {
    for shared in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".davinci")).unwrap();
        let settings_path = root.path().join(".davinci/settings.json");
        std::fs::write(&settings_path, r#"{"processManager":{"enabled":false}}"#).unwrap();
        let parsed = Args {
            project_trust_override: Some(true),
            permission_mode: Some(PermissionMode::AlwaysApprove),
            ..Default::default()
        };
        let disabled = build_agent(&parsed, state.path(), root.path()).unwrap();
        assert!(disabled.tool_context.processes.is_none());
        assert!(!disabled
            .tools
            .iter()
            .any(|name| name.starts_with("process_")));
        assert!(!disabled
            .provider_tool_specs()
            .iter()
            .any(|spec| spec.name.starts_with("process_")));
        std::fs::write(&settings_path, r#"{"processManager":{"enabled":true}}"#).unwrap();
        let mut agent: Agent = build_agent(&parsed, state.path(), root.path()).unwrap();
        let manager = agent.tool_context.processes.as_ref().unwrap();
        assert_eq!(
            manager
                .execute(root.path(), "process_list", &json!({}), None, None)
                .unwrap()
                .details
                .unwrap()["processes"],
            json!([])
        );
        // Test executable needs the harness entry; the packaged executable has a separate integration gate.
        agent.tool_context.processes = Some(
            ProcessManager::new(
                root.path(),
                agent.tool_context.jobs.clone(),
                agent.permissions.clone(),
                SupervisorCommand {
                    executable: std::env::current_exe().unwrap(),
                    argv: vec![
                        "--exact".into(),
                        "process_manager_integration_tests::helper_entry".into(),
                        "--nocapture".into(),
                    ],
                },
            )
            .unwrap()
            .with_counters(agent.counters.clone()),
        );
        let host = ExtensionHost::load_with_cwd(state.path(), &[], root.path());
        agent.tools = [
            "tool_search",
            "process_start",
            "process_output",
            "process_status",
            "process_stop",
            "retrieve_output",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        if shared {
            attach_shared_tool_executor(&mut agent, Arc::new(Mutex::new(host.clone())));
        } else {
            attach_tool_executor(&mut agent, &host);
        }
        let post_host = host.clone();
        agent.post_tool = Some(davinci_agent::PostToolHook(Arc::new(
            move |_, _, name, args, result| post_host.native_after_tool(name, args, result),
        )));
        assert!(!agent
            .provider_tool_specs()
            .iter()
            .any(|spec| spec.name == "process_start"));
        let (discovered, _, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "tool_search",
            json!({"query":"process_start"}),
        );
        assert!(!error);
        assert_eq!(discovered["activated"], json!(["process_start"]));
        assert_eq!(agent.tool_context.jobs.lock().unwrap().running(), 0);
        let args = json!({"executable":"node", "argv":["-e","for(let i=0;i<1000;i++)console.log('line '+i+' fixture output');setTimeout(()=>process.exit(0),20000)"]});
        let (first, output, error) =
            super::test_impact_integration_tests::call(&mut agent, "process_start", args.clone());
        assert!(!error, "{output}");
        let id = first["process"]["id"].as_u64().unwrap();
        let (second, output, error) =
            super::test_impact_integration_tests::call(&mut agent, "process_start", args);
        assert!(!error, "{output}");
        assert_eq!(second["process"]["id"], id);
        assert_eq!(second["reused"], true);
        let manager = agent.tool_context.processes.as_ref().unwrap().clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let page = manager
                .execute(
                    root.path(),
                    "process_output",
                    &json!({"id":id,"limit":65536}),
                    None,
                    None,
                )
                .unwrap();
            if page.content.contains("line 999 fixture output") {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let (details, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "process_output",
            json!({"id":id,"limit":65536}),
        );
        assert!(!error, "{output}");
        let output_id = details["tokenGovernor"]["outputId"]
            .as_str()
            .expect("large process output uses the governor");
        assert!(output.to_string().contains("retrieve_output"));
        let (_, recovered, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "retrieve_output",
            json!({"id":output_id,"startLine":995,"endLine":1002}),
        );
        assert!(!error, "{recovered}");
        assert!(recovered.to_string().contains("line 999 fixture output"));
        manager.shutdown();
        assert!(manager
            .execute(root.path(), "process_list", &json!({}), None, None)
            .is_err());
        let deadline = Instant::now() + Duration::from_secs(5);
        while agent.tool_context.jobs.lock().unwrap().running() != 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
