//! Real native browser dispatch through both normal-session executor attachments.
use super::{attach_shared_tool_executor, attach_tool_executor, build_agent, Args, ExtensionHost};
use crate::native_extensions::browser::{BrowserConfig, BrowserController, TOOL_NAMES};
use davinci_agent::{
    jobs::supervisor::SupervisorCommand, process_manager::ProcessManager, PermissionMode,
    PermissionRule,
};
use serde_json::json;
use std::{
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires explicitly configured trusted Node and Playwright installation"]
fn normal_browser_native_dispatch_actions_revocation_and_cleanup() {
    for shared in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut agent = build_agent(
            &Args {
                project_trust_override: Some(true),
                permission_mode: Some(PermissionMode::AlwaysApprove),
                ..Default::default()
            },
            state.path(),
            root.path(),
        )
        .unwrap();
        let supervisor = SupervisorCommand {
            executable: std::env::current_exe().unwrap(),
            argv: vec![
                "--exact".into(),
                "process_manager_integration_tests::helper_entry".into(),
                "--nocapture".into(),
            ],
        };
        agent.tool_context.foreground_supervisor = Some(supervisor.clone());
        agent.tool_context.processes = Some(
            ProcessManager::new(
                root.path(),
                agent.tool_context.jobs.clone(),
                agent.permissions.clone(),
                supervisor,
            )
            .unwrap(),
        );
        let host = ExtensionHost::load_with_cwd(state.path(), &[], root.path());
        host.native.lock().unwrap().browser = BrowserController::new(
            root.path(),
            BrowserConfig {
                enabled: true,
                node: PathBuf::from(std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH").unwrap()),
                package: PathBuf::from(
                    std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH").unwrap(),
                ),
                version: "1.62.0".into(),
            },
        );
        agent.tools = ["tool_search", "process_start", "process_stop", "edit"]
            .into_iter()
            .chain(TOOL_NAMES.iter().copied())
            .map(String::from)
            .collect();
        host.register_with(&agent.runtime_for_session().unwrap().capability_registry);
        if shared {
            attach_shared_tool_executor(&mut agent, Arc::new(Mutex::new(host.clone())));
        } else {
            attach_tool_executor(&mut agent, &host);
        }
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let html = "<html><body><label>Name<input aria-label='Name'></label><select aria-label='Choice'><option value='a'>A</option><option value='b'>B</option></select><button onclick=\"this.textContent='Done'\">Start</button></body></html>";
        std::fs::write(
            root.path().join("index.html"),
            html.replace("'Done'", "'Broken'"),
        )
        .unwrap();
        let script = format!(
            "const http=require('node:http');const fs=require('node:fs');const server=http.createServer((req,res)=>{{res.setHeader('content-type','text/html');res.end(fs.readFileSync('index.html'));}});server.listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>server.close(),60000);"
        );
        let (started, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "process_start",
            json!({"executable":"node","argv":["-e",script],"ports":[port]}),
        );
        assert!(!error, "{output}");
        let process_id = started["process"]["id"].as_u64().unwrap();
        let manager = agent.tool_context.processes.as_ref().unwrap().clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = manager
                .execute(
                    root.path(),
                    "process_output",
                    &json!({"id":process_id}),
                    None,
                    None,
                )
                .unwrap();
            if output.content.contains("READY") {
                break;
            }
            assert!(Instant::now() < deadline, "server did not become ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        let (opened, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port,"viewport":{"width":800,"height":600}}),
        );
        assert!(!error, "{output}");
        assert_eq!(opened["verification"], "observations_only");
        let broken_id = opened["browser_id"].as_str().unwrap();
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_click",
            json!({"browser_id":broken_id,"selector":{"kind":"role","role":"button","name":"Start"}}),
        );
        assert!(!error, "{output}");
        let (broken, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_snapshot",
            json!({"browser_id":broken_id}),
        );
        assert!(!error, "{output}");
        assert!(broken["result"]["html"]
            .as_str()
            .unwrap()
            .contains(">Broken</button>"));
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_close",
            json!({"browser_id":broken_id}),
        );
        assert!(!error, "{output}");
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "edit",
            json!({"path":"index.html","oldText":"'Broken'","newText":"'Done'"}),
        );
        assert!(!error, "{output}");
        let (opened, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port}),
        );
        assert!(!error, "{output}");
        let id = opened["browser_id"].as_str().unwrap();
        let controller = host.native.lock().unwrap().browser.clone();
        let mut foreign = agent.tool_context.clone();
        foreign.processes = Some(
            ProcessManager::new(
                root.path(),
                foreign.jobs.clone(),
                agent.permissions.clone(),
                foreign.foreground_supervisor.as_ref().unwrap().clone(),
            )
            .unwrap(),
        );
        assert!(controller
            .execute(
                root.path(),
                "browser_snapshot",
                &json!({"browser_id":id}),
                &foreign
            )
            .is_err());
        let mut cancelled = agent.tool_context.clone();
        cancelled.abort = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        assert!(controller
            .execute(
                root.path(),
                "browser_snapshot",
                &json!({"browser_id":id}),
                &cancelled
            )
            .is_err());
        // These rejected requests must not destroy another caller's owned context.
        for (name, args) in [
            (
                "browser_type",
                json!({"browser_id":id,"selector":{"kind":"label","value":"Name"},"text":"Ada"}),
            ),
            (
                "browser_select",
                json!({"browser_id":id,"selector":{"kind":"label","value":"Choice"},"value":"b"}),
            ),
            (
                "browser_click",
                json!({"browser_id":id,"selector":{"kind":"role","role":"button","name":"Start"}}),
            ),
        ] {
            let (details, output, error) =
                super::test_impact_integration_tests::call(&mut agent, name, args);
            assert!(!error, "{name}: {output}");
            assert_eq!(details["verification"], "observations_only");
        }
        for name in [
            "browser_snapshot",
            "browser_accessibility",
            "browser_console",
            "browser_network",
            "browser_screenshot",
        ] {
            let (details, output, error) = super::test_impact_integration_tests::call(
                &mut agent,
                name,
                json!({"browser_id":id}),
            );
            assert!(!error, "{name}: {output}");
            assert_eq!(details["verification"], "observations_only");
            if matches!(name, "browser_snapshot" | "browser_accessibility") {
                assert!(output.to_string().contains("Done"), "{output}");
            }
            if name == "browser_screenshot" {
                assert!(details["result"]["artifact"].is_string(), "{details}");
                assert!(details["result"].get("bytes").is_none());
            }
        }
        let (interrupted, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port}),
        );
        assert!(!error, "{output}");
        let interrupted_id = interrupted["browser_id"].as_str().unwrap();
        let live_abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut live_context = agent.tool_context.clone();
        live_context.abort = Some(live_abort.clone());
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                live_abort.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let result = controller.execute(root.path(), "browser_click",
                &json!({"browser_id":interrupted_id,"selector":{"kind":"role","role":"button","name":"Missing"}}),
                &live_context);
            assert!(result.is_err(), "cancelled action must not report success");
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(controller
            .execute(
                root.path(),
                "browser_snapshot",
                &json!({"browser_id":interrupted_id}),
                &agent.tool_context
            )
            .is_err());
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_snapshot",
            json!({"browser_id":id}),
        );
        assert!(
            !error,
            "another context must survive cancellation: {output}"
        );
        controller.shutdown_backend_for_test();
        let (_, _, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_snapshot",
            json!({"browser_id":id}),
        );
        assert!(error, "dead backend page state must not be reused");
        let (reopened, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port}),
        );
        assert!(!error, "fresh request must recover the backend: {output}");
        let recovered_id = reopened["browser_id"].as_str().unwrap();
        assert_ne!(recovered_id, id);
        let id = recovered_id;
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_click",
            json!({"browser_id":id,"selector":{"kind":"role","role":"button","name":"Start"}}),
        );
        assert!(
            !error,
            "recovered browser must perform new actions: {output}"
        );
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(PermissionRule::bare("browser_snapshot"));
        let (_, _, denied) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_snapshot",
            json!({"browser_id":id}),
        );
        assert!(denied, "live revocation must reject an existing browser");
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_close",
            json!({"browser_id":id}),
        );
        assert!(!error, "{output}");
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "process_stop",
            json!({"id":process_id}),
        );
        assert!(!error, "{output}");
        drop(controller);
        drop(host);
        drop(agent);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let processes = manager
                .execute(root.path(), "process_list", &json!({}), None, None)
                .unwrap()
                .details
                .unwrap();
            if processes["processes"][0]["state"] == "exited" {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit: {processes}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
