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
fn rpc_artifact_helper_entry() {
    if std::env::var_os("DAVINCI_TEST_RPC_ARTIFACT_HELPER").is_none() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let parsed = Args {
        offline: true,
        project_trust_override: Some(true),
        permission_mode: Some(PermissionMode::AlwaysApprove),
        ..Default::default()
    };
    let mut agent = build_agent(&parsed, state.path(), root.path()).unwrap();
    let supervisor = SupervisorCommand {
        executable: std::env::current_exe().unwrap(),
        argv: vec![
            "--exact".into(),
            "process_manager_integration_tests::helper_entry".into(),
            "--nocapture".into(),
        ],
    };
    agent.tool_context.foreground_supervisor = Some(supervisor.clone());
    let manager = ProcessManager::new(
        root.path(),
        agent.tool_context.jobs.clone(),
        agent.permissions.clone(),
        supervisor,
    )
    .unwrap();
    agent.tool_context.processes = Some(manager.clone());
    let fixture_manager = if std::env::var_os("DAVINCI_TEST_RPC_CHILD_ARTIFACT").is_some() {
        manager.child_lease(
            agent.permissions.clone(),
            davinci_agent::shell_policy::ShellPolicyProfile::Permissive,
        )
    } else {
        manager.clone()
    };
    let fixture_context = davinci_agent::ToolContext {
        processes: Some(fixture_manager.clone()),
        ..agent.tool_context.clone()
    };
    let controller = BrowserController::new(
        root.path(),
        BrowserConfig {
            enabled: true,
            node: std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH")
                .unwrap()
                .into(),
            package: std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH")
                .unwrap()
                .into(),
            version: "1.62.0".into(),
        },
    );
    let host = ExtensionHost::load_with_cwd(state.path(), &[], root.path());
    host.native.lock().unwrap().browser = controller.clone();
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = format!("require('node:http').createServer((q,r)=>r.end('<html><button>Login</button></html>')).listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>process.exit(0),60000);");
    let started = fixture_manager
        .execute(
            root.path(),
            "process_start",
            &json!({"executable":"node","argv":["-e",script],"ports":[port]}),
            None,
            None,
        )
        .unwrap();
    let process_id = started.details.unwrap()["process"]["id"].as_u64().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture_manager
        .execute(
            root.path(),
            "process_output",
            &json!({"id":process_id}),
            None,
            None,
        )
        .unwrap()
        .content
        .contains("READY")
    {
        assert!(
            Instant::now() < deadline,
            "fixture server did not become ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let opened = controller
        .execute(
            root.path(),
            "browser_open",
            &json!({"process_id":process_id,"port":port}),
            &fixture_context,
        )
        .unwrap()
        .details
        .unwrap();
    let browser_id = opened["browser_id"].clone();
    let screenshot = controller
        .execute(
            root.path(),
            "browser_screenshot",
            &json!({"browser_id":browser_id}),
            &fixture_context,
        )
        .unwrap()
        .details
        .unwrap();
    controller
        .execute(
            root.path(),
            "browser_close",
            &json!({"browser_id":browser_id}),
            &fixture_context,
        )
        .unwrap();
    fixture_manager
        .execute(
            root.path(),
            "process_stop",
            &json!({"id":process_id}),
            None,
            None,
        )
        .unwrap();
    drop(fixture_context);
    drop(fixture_manager);
    println!(
        "\n{}",
        json!({"type":"artifact_fixture_ready","browser_id":browser_id,"artifact":screenshot["result"]})
    );
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    super::run_rpc_with_host(&parsed, &mut agent, Arc::new(Mutex::new(host))).unwrap();
    manager.shutdown();
}

#[test]
#[ignore = "requires explicitly configured trusted Node and Playwright installation"]
fn rpc_retained_browser_artifact_wire_exchange() {
    for child_owner in [false, true] {
        rpc_retained_browser_artifact_wire_exchange_for(child_owner);
    }
}

fn rpc_retained_browser_artifact_wire_exchange_for(child_owner: bool) {
    use base64::Engine;
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let state = tempfile::tempdir().unwrap();
    let stderr = std::fs::File::create(state.path().join("stderr.txt")).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env_clear();
    if child_owner {
        command.env("DAVINCI_TEST_RPC_CHILD_ARTIFACT", "1");
    }
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "DAVINCI_TRUSTED_NODE_TEST_PATH",
        "DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = ChildGuard(
        command
            .args([
                "--exact",
                "browser_integration_tests::rpc_artifact_helper_entry",
                "--nocapture",
            ])
            .env("DAVINCI_TEST_RPC_ARTIFACT_HELPER", "1")
            .env("HOME", state.path())
            .env("USERPROFILE", state.path())
            .env("PI_CODING_AGENT_DIR", state.path())
            .env("DAVINCI_CODING_AGENT_DIR", state.path())
            .env("PI_OFFLINE", "1")
            .env("PI_DISABLE_NETWORK", "1")
            .env("PI_HOOKS_DRY_RUN", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if tx.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let ready = loop {
        let value = rx
            .recv_timeout(Duration::from_secs(40))
            .expect("RPC fixture startup timed out");
        if value["type"] == "artifact_fixture_ready" {
            break value;
        }
    };
    let mut stdin = child.0.stdin.take().unwrap();
    let request = json!({"browser_id":ready["browser_id"],"artifact":ready["artifact"]["artifact"],"offset":0,"limit":64});
    writeln!(
        stdin,
        "{}",
        json!({"id":"view","type":"get_browser_artifact","value":request.to_string()})
    )
    .unwrap();
    stdin.flush().unwrap();
    let response = loop {
        let value = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("artifact RPC response timed out");
        if value["id"] == "view" {
            break value;
        }
    };
    assert_eq!(response["success"], true, "{response}");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(response["data"]["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes.len(), 64);
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(response["data"]["sha256"], ready["artifact"]["sha256"]);
    assert_eq!(response["data"]["verification"], "observations_only");
    writeln!(
        stdin,
        "{}",
        json!({"id":"invalid","type":"get_browser_artifact","value":"{}"})
    )
    .unwrap();
    stdin.flush().unwrap();
    let response = loop {
        let value = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("invalid artifact RPC response timed out");
        if value["id"] == "invalid" {
            break value;
        }
    };
    assert_eq!(response["success"], false);
    writeln!(stdin, "{}", json!({"id":"messages","type":"get_messages"})).unwrap();
    stdin.flush().unwrap();
    let response = loop {
        let value = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("message RPC response timed out");
        if value["id"] == "messages" {
            break value;
        }
    };
    assert_eq!(response["success"], true);
    assert!(
        response["data"]["messages"].as_array().unwrap().is_empty(),
        "frontend binary retrieval must not populate the model conversation"
    );
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "RPC child did not shut down after EOF"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires explicitly configured trusted Node and Playwright installation"]
fn normal_browser_native_dispatch_actions_revocation_and_cleanup() {
    for (shared, ipv6) in [(false, false), (true, false), (false, true), (true, true)] {
        let address: std::net::IpAddr = if ipv6 {
            std::net::Ipv6Addr::LOCALHOST.into()
        } else {
            std::net::Ipv4Addr::LOCALHOST.into()
        };
        let loopback_host = address.to_string();
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
        let port = TcpListener::bind((address, 0))
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
            "const http=require('node:http');const fs=require('node:fs');const server=http.createServer((req,res)=>{{res.setHeader('content-type','text/html');res.end(fs.readFileSync('index.html'));}});server.listen({port},'{loopback_host}',()=>console.log('READY'));setTimeout(()=>server.close(),60000);"
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
            json!({"process_id":process_id,"port":port,"host":loopback_host,"viewport":{"width":800,"height":600}}),
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
            json!({"process_id":process_id,"port":port,"host":loopback_host}),
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
        let mut retained_request = None;
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
                let request = json!({"browser_id":id,"artifact":details["result"]["artifact"],"offset":0,"limit":64});
                retained_request = Some(request.clone());
                let retrieved = controller
                    .retrieve_artifact(root.path(), &request, &agent.tool_context)
                    .unwrap();
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(retrieved["base64"].as_str().unwrap())
                    .unwrap();
                assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
                assert_eq!(retrieved["sha256"], details["result"]["sha256"]);
                assert_eq!(bytes.len(), 64);
                let mut tail = request.clone();
                tail["offset"] = details["result"]["size"].clone();
                let eof = controller
                    .retrieve_artifact(root.path(), &tail, &agent.tool_context)
                    .unwrap();
                assert_eq!(eof["eof"], true);
                assert_eq!(eof["base64"], "");
                tail["offset"] = json!(details["result"]["size"].as_u64().unwrap() + 1);
                assert!(controller
                    .retrieve_artifact(root.path(), &tail, &agent.tool_context)
                    .is_err());
                let (other, output, error) = super::test_impact_integration_tests::call(
                    &mut agent,
                    "browser_open",
                    json!({"process_id":process_id,"port":port,"host":loopback_host}),
                );
                assert!(!error, "{output}");
                let mut foreign = request.clone();
                foreign["browser_id"] = other["browser_id"].clone();
                assert!(controller
                    .retrieve_artifact(root.path(), &foreign, &agent.tool_context)
                    .is_err());
                let (_, output, error) = super::test_impact_integration_tests::call(
                    &mut agent,
                    "browser_close",
                    json!({"browser_id":other["browser_id"]}),
                );
                assert!(!error, "{output}");
                let mut invalid = request.clone();
                invalid["limit"] = json!(65537);
                assert!(controller
                    .retrieve_artifact(root.path(), &invalid, &agent.tool_context)
                    .is_err());
                invalid = request.clone();
                invalid["artifact"] = json!("../outside.png");
                assert!(controller
                    .retrieve_artifact(root.path(), &invalid, &agent.tool_context)
                    .is_err());
                agent
                    .permissions
                    .lock()
                    .unwrap()
                    .deny
                    .push(PermissionRule::bare("browser_screenshot"));
                assert!(controller
                    .retrieve_artifact(root.path(), &request, &agent.tool_context)
                    .is_err());
                agent.permissions.lock().unwrap().deny.pop();
            }
        }
        let (interrupted, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port,"host":loopback_host}),
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
            json!({"process_id":process_id,"port":port,"host":loopback_host}),
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
        let (expired, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port,"host":loopback_host}),
        );
        assert!(!error, "{output}");
        assert!(
            controller
                .retrieve_artifact(
                    root.path(),
                    retained_request.as_ref().unwrap(),
                    &agent.tool_context
                )
                .is_ok(),
            "closing the browser context must not discard retained artifact authority"
        );
        let expired_id = expired["browser_id"].as_str().unwrap();
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "process_stop",
            json!({"id":process_id}),
        );
        assert!(!error, "{output}");
        assert!(
            controller
                .retrieve_artifact(
                    root.path(),
                    retained_request.as_ref().unwrap(),
                    &agent.tool_context
                )
                .is_ok(),
            "stopping the dev server must not discard retained evidence"
        );
        let (_, _, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_console",
            json!({"browser_id":expired_id}),
        );
        assert!(error, "released managed-server lifetime must not be usable");
        assert_eq!(
            controller.context_count(),
            0,
            "stale context must leave the quota"
        );
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
