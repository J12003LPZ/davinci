//! Real native browser dispatch through both normal-session executor attachments.
use super::{attach_shared_tool_executor, attach_tool_executor, build_agent, Args, ExtensionHost};
use crate::native_extensions::browser::{
    BrowserAssertionSpec, BrowserConfig, BrowserController, TOOL_NAMES,
};
use davinci_agent::runtime::cache::CacheStats;
use davinci_agent::{
    jobs::supervisor::SupervisorCommand, process_manager::ProcessManager, PermissionMode,
    PermissionRule, PreToolHook,
};
use serde_json::json;
use std::{
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn cache_delta(before: &CacheStats, after: &CacheStats) -> serde_json::Value {
    let totals = |stats: &CacheStats| {
        stats
            .namespaces
            .values()
            .fold((0_u64, 0_u64), |(hits, misses), namespace| {
                (
                    hits + namespace.memory_hits + namespace.persistent_hits,
                    misses + namespace.misses,
                )
            })
    };
    let (before_hits, before_misses) = totals(before);
    let (after_hits, after_misses) = totals(after);
    let hits = after_hits.saturating_sub(before_hits);
    let misses = after_misses.saturating_sub(before_misses);
    let total = hits + misses;
    json!({
        "hits": hits,
        "misses": misses,
        "hit_rate": if total == 0 { 0.0 } else { hits as f64 / total as f64 },
    })
}

#[cfg(windows)]
fn peak_memory_bytes() -> Option<u64> {
    #[repr(C)]
    struct Counters {
        size: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters {
        size: std::mem::size_of::<Counters>() as u32,
        faults: 0,
        peak_working: 0,
        working: 0,
        peak_paged: 0,
        paged: 0,
        peak_nonpaged: 0,
        nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    let success = unsafe {
        GetProcessMemoryInfo(-1, &mut counters, std::mem::size_of::<Counters>() as u32) != 0
    };
    success.then_some(counters.peak_working as u64)
}

#[cfg(target_os = "linux")]
fn peak_memory_bytes() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:")?.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|kilobytes| kilobytes * 1024)
}

#[cfg(target_os = "macos")]
fn peak_memory_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    (result == 0).then(|| unsafe { usage.assume_init().ru_maxrss as u64 })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn peak_memory_bytes() -> Option<u64> {
    None
}

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
    let verify_browser = std::env::var_os("DAVINCI_TEST_RPC_VERIFY_BROWSER").is_some();
    let transaction_id = if verify_browser {
        std::fs::write(
            root.path().join("index.html"),
            "<button onclick=\"this.textContent='Broken'\">Start</button>",
        )
        .unwrap();
        let coordinator = davinci_agent::runtime::transactions::coordinator_for_context(
            root.path(),
            &fixture_context,
        )
        .unwrap();
        let preview = coordinator
            .preview(vec![
                davinci_agent::runtime::transactions::ProposedChange::write(
                    "index.html",
                    b"<button onclick=\"this.textContent='Done'\">Start</button>".to_vec(),
                ),
            ])
            .unwrap();
        coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
        Some(preview.id)
    } else {
        None
    };
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = if verify_browser {
        format!("const http=require('node:http'),fs=require('node:fs');http.createServer((q,r)=>r.end(fs.readFileSync('index.html'))).listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>process.exit(0),60000);")
    } else {
        format!("require('node:http').createServer((q,r)=>r.end('<html><button>Login</button></html>')).listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>process.exit(0),60000);")
    };
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
    let mut open_args = json!({"process_id":process_id,"port":port});
    if let Some(transaction_id) = &transaction_id {
        open_args["transaction_id"] = json!(transaction_id);
    }
    let opened = controller
        .execute(root.path(), "browser_open", &open_args, &fixture_context)
        .unwrap()
        .details
        .unwrap();
    let browser_id = opened["browser_id"].clone();
    if verify_browser {
        controller
            .execute(
                root.path(),
                "browser_click",
                &json!({"browser_id":browser_id,"selector":{"kind":"role","role":"button","name":"Start"}}),
                &fixture_context,
            )
            .unwrap();
        println!(
            "\n{}",
            json!({"type":"verification_fixture_ready","browser_id":browser_id,"transaction_id":transaction_id})
        );
    } else {
        let retained = controller
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
        println!(
            "\n{}",
            json!({"type":"artifact_fixture_ready","browser_id":browser_id,"artifact":retained["result"]})
        );
    }
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    super::run_rpc_with_host(&parsed, &mut agent, Arc::new(Mutex::new(host))).unwrap();
    if verify_browser {
        let _ = controller.execute(
            root.path(),
            "browser_close",
            &json!({"browser_id":browser_id}),
            &fixture_context,
        );
        let _ = fixture_manager.execute(
            root.path(),
            "process_stop",
            &json!({"id":process_id}),
            None,
            None,
        );
    }
    drop(fixture_context);
    drop(fixture_manager);
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
    assert!(ready["artifact"]["action_sequence"].as_u64().unwrap() > 0);
    assert_eq!(
        response["data"]["action_sequence"],
        ready["artifact"]["action_sequence"]
    );
    assert_eq!(response["data"]["browser_id"], ready["browser_id"]);
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
fn rpc_browser_verification_receipt_wire_exchange() {
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
            .env("DAVINCI_TEST_RPC_VERIFY_BROWSER", "1")
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
            .expect("RPC browser verification fixture startup timed out");
        if value["type"] == "verification_fixture_ready" {
            break value;
        }
    };
    let mut stdin = child.0.stdin.take().unwrap();
    let request = json!({
        "browser_id":ready["browser_id"],
        "dom_contains":">Done</button>",
        "accessibility_contains":"Done"
    });
    writeln!(
        stdin,
        "{}",
        json!({"id":"verify","type":"verify_browser","value":request.to_string()})
    )
    .unwrap();
    stdin.flush().unwrap();
    let response = loop {
        let value = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("browser verification RPC response timed out");
        if value["id"] == "verify" {
            break value;
        }
    };
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(
        response["data"]["interaction"]["backend_kind"],
        "real_browser"
    );
    assert_eq!(response["data"]["interaction"]["assertions_passed"], true);
    assert_eq!(
        response["data"]["transaction_id"],
        ready["transaction_id"].as_str().unwrap()
    );
    assert_eq!(response["data"]["affected_paths"], json!(["index.html"]));
    assert!(response["data"]["action_sequences"]
        .as_array()
        .is_some_and(|values| values.len() == 5));
    assert!(response["data"]["screenshot_artifact"].is_string());
    assert!(response["data"]["incomplete_coverage"]
        .as_array()
        .unwrap()
        .is_empty());

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
        "browser verification RPC evidence must not populate the model conversation"
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
            "RPC browser verification child did not shut down after EOF"
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
        let html = "<html><body><label>Name<input aria-label='Name'></label><select aria-label='Choice'><option value='a'>A</option><option value='b'>B</option></select><button onclick=\"run()\">Start</button><script>const mode='Done';async function run(){if(mode[0]==='B'){console.error('Planted browser console failure');await fetch('/broken-api');}document.querySelector('button').textContent=mode;}</script></body></html>";
        std::fs::write(
            root.path().join("index.html"),
            html.replace("'Done'", "'Broken'"),
        )
        .unwrap();
        let script = format!(
            "const http=require('node:http');const fs=require('node:fs');const server=http.createServer((req,res)=>{{if(req.url==='/broken-api'){{res.writeHead(500);res.end('planted failure');return;}}res.setHeader('content-type','text/html');res.end(fs.readFileSync('index.html'));}});server.listen({port},'{loopback_host}',()=>console.log('READY'));setTimeout(()=>server.close(),60000);"
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
        let (broken_console, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_console",
            json!({"browser_id":broken_id}),
        );
        assert!(!error, "{output}");
        assert!(
            !broken_console["result"]["events"]
                .as_array()
                .unwrap()
                .is_empty(),
            "planted UI failure must emit a console error"
        );
        let (broken_network, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_network",
            json!({"browser_id":broken_id}),
        );
        assert!(!error, "{output}");
        assert!(
            broken_network["result"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["status"] == 500),
            "planted UI failure must include a failed network request"
        );
        let (_, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_close",
            json!({"browser_id":broken_id}),
        );
        assert!(!error, "{output}");
        let (edited, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "edit",
            json!({"path":"index.html","oldText":"'Broken'","newText":"'Done'"}),
        );
        assert!(!error, "{output}");
        let transaction_id = edited["transaction"]["id"]
            .as_str()
            .expect("edit must return its host transaction id")
            .to_owned();
        let (opened, output, error) = super::test_impact_integration_tests::call(
            &mut agent,
            "browser_open",
            json!({"process_id":process_id,"port":port,"host":loopback_host,"transaction_id":transaction_id}),
        );
        assert!(!error, "{output}");
        assert_eq!(opened["source_binding"]["transaction_id"], transaction_id);
        assert_eq!(
            opened["source_binding"]["affected_paths"],
            json!(["index.html"])
        );
        assert!(opened["source_binding"]["source_digest"].as_str().is_some());
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
                assert!(details["result"]["action_sequence"].as_u64().unwrap() > 0);
                assert_eq!(
                    retrieved["action_sequence"],
                    details["result"]["action_sequence"]
                );
                assert_eq!(retrieved["browser_id"], id);
                assert_eq!(
                    retrieved["source_binding"]["transaction_id"],
                    transaction_id
                );
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
        let receipt = controller
            .verify_host(
                root.path(),
                uuid::Uuid::parse_str(id).unwrap(),
                BrowserAssertionSpec {
                    dom_contains: ">Done</button>".into(),
                    accessibility_contains: Some("Done".into()),
                },
                &agent.tool_context,
            )
            .expect("real browser assertions must produce a source-bound receipt");
        assert!(receipt.interaction.assertions_passed);
        assert_eq!(
            receipt.interaction.backend_kind,
            crate::interaction_testing::BackendKind::RealBrowser
        );
        assert_eq!(receipt.transaction_id, transaction_id);
        assert_eq!(receipt.affected_paths, vec!["index.html"]);
        assert!(!receipt.action_sequences.is_empty());
        assert!(receipt.screenshot_artifact.is_some());
        assert!(receipt.interaction.console_errors.is_empty());
        assert!(receipt.interaction.network_failures.is_empty());
        let transaction = davinci_agent::runtime::transactions::coordinator_for_context(
            root.path(),
            &agent.tool_context,
        )
        .unwrap()
        .status(&transaction_id)
        .unwrap();
        assert_eq!(
            transaction.state,
            davinci_agent::runtime::transactions::TransactionState::Applied,
            "browser assertions are evidence but do not mark the transaction verified"
        );

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
        std::fs::write(root.path().join("index.html"), "stale source").unwrap();
        assert!(
            controller
                .retrieve_artifact(
                    root.path(),
                    retained_request.as_ref().unwrap(),
                    &agent.tool_context
                )
                .is_err(),
            "retained evidence must not survive a source change"
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

fn snapshot_checkpoint_id(details: &serde_json::Value) -> String {
    details
        .pointer("/checkpoint/id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .expect("workspace_checkpoint must return nested checkpoint.id")
        .to_string()
}

struct GraphEnvGuard {
    previous: Vec<(&'static str, Option<String>)>,
}

impl GraphEnvGuard {
    fn apply(pairs: &[(&str, String)]) -> Self {
        const KEYS: &[&str] = &[
            "PI_GRAPH_ROLE",
            "PI_GRAPH_EXPECT",
            "PI_GRAPH_ARTIFACT_PATH",
            "PI_GRAPH_AUTHORIZED_TOOLS",
            "PI_GRAPH_NODE_ID",
            "DAVINCI_TASK_COORDINATOR_ADDR",
        ];
        let previous = KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in KEYS {
            std::env::remove_var(key);
        }
        for (key, value) in pairs {
            std::env::set_var(key, value);
        }
        Self { previous }
    }
}

impl Drop for GraphEnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[test]
#[ignore = "requires explicitly configured trusted Node and Playwright installation"]
fn login_button_seventeen_step_normal_dispatch_and_graph_deny() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let feature_enabled = std::env::var("DAVINCI_P12_FEATURES")
        .map(|value| value != "off")
        .unwrap_or(true);
    std::fs::write(
        state.path().join("settings.json"),
        serde_json::json!({
            "verificationPlanner": {"enabled": feature_enabled},
            "workspaceSnapshots": {"enabled": feature_enabled, "maxFiles": 8},
            "changeImpact": {"enabled": feature_enabled},
            "testImpact": {"enabled": feature_enabled},
            "packageIntelligence": {"enabled": feature_enabled},
            "buildIntelligence": {"enabled": feature_enabled},
            "gitIntelligence": {"enabled": feature_enabled},
            "browserVerification": {"enabled": feature_enabled},
            "processManager": {"enabled": feature_enabled}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("package.json"),
        r#"{"name":"login-app","scripts":{"test":"node --test"},"type":"module"}"#,
    )
    .unwrap();
    std::fs::write(
        root.path().join("src/login.ts"),
        "export function loginButtonLabel(ok: boolean): string { return ok ? 'Broken' : 'Broken'; }\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("src/login.test.ts"),
        "import { loginButtonLabel } from './login.ts';\nconsole.log(loginButtonLabel(true));\n",
    )
    .unwrap();
    let html = "<html><body><button onclick=\"run()\">Start</button><script>const mode='Done';async function run(){if(mode[0]==='B'){console.error('Planted browser console failure');await fetch('/broken-api');}document.querySelector('button').textContent=mode;}</script></body></html>";
    std::fs::write(
        root.path().join("index.html"),
        html.replace("'Done'", "'Broken'"),
    )
    .unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["-c", "init.defaultBranch=main", "init", "--quiet"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@example.test",
        "commit",
        "-qm",
        "login fixture",
    ]);

    let startup_started = Instant::now();
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
    if feature_enabled {
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
    }
    agent.tools = [
        "tool_search",
        "process_start",
        "process_stop",
        "edit",
        "bash",
        "repo_map",
        "lsp_document_symbols",
        "lsp_diagnostics",
        "package_info",
        "git_blame_symbol",
        "git_commit_context",
        "impact_analyze",
        "test_plan",
        "build_command",
        "verification_plan",
        "workspace_checkpoint",
        "workspace_diff",
        "workspace_restore",
    ]
    .into_iter()
    .chain(TOOL_NAMES.iter().copied())
    .map(String::from)
    .collect();
    host.register_with(&agent.runtime_for_session().unwrap().capability_registry);
    attach_tool_executor(&mut agent, &host);
    let hook_host = host.clone();
    agent.pre_tool = Some(PreToolHook(Arc::new(move |name, args| {
        hook_host.native_before_tool(name, args, String::new)
    })));
    let startup_ms = startup_started.elapsed().as_secs_f64() * 1000.0;
    let cache_before = host.native.lock().unwrap().cache.stats();

    if !feature_enabled {
        let disabled_result =
            host.native
                .lock()
                .unwrap()
                .execute_tool(root.path(), "repo_map", &json!({}));
        assert!(
            disabled_result.is_err(),
            "feature-off repo_map must be unavailable: {disabled_result:?}"
        );
        let memory_bytes = peak_memory_bytes().expect("P12 feature-off memory measurement");
        let cache = cache_delta(&cache_before, &host.native.lock().unwrap().cache.stats());
        let receipt = json!({
            "schema_version": 1,
            "test": "login_button_seventeen_step_normal_dispatch_and_graph_deny",
            "mode": "feature-off",
            "feature_enabled": false,
            "fixture_id": "login-button-seventeen-step",
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "startup_ms": startup_ms,
            "memory_bytes": memory_bytes,
            "cache": cache,
            "disabled_tool": "repo_map"
        });
        eprintln!("P12_FEATURE_OFF_RECEIPT {receipt}");
        if let Some(path) = std::env::var_os("DAVINCI_P12_EVAL_ARTIFACT") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&receipt)
                    .expect("P12 feature-off receipt must serialize as JSON"),
            )
            .expect("P12 feature-off receipt must be writable");
        }
        return;
    }

    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = format!(
        "const http=require('node:http');const fs=require('node:fs');const server=http.createServer((req,res)=>{{if(req.url==='/broken-api'){{res.writeHead(500);res.end('planted failure');return;}}res.setHeader('content-type','text/html');res.end(fs.readFileSync('index.html'));}});server.listen({port},'127.0.0.1',()=>console.log('READY'));setTimeout(()=>server.close(),60000);"
    );

    let mut dispatched = Vec::new();
    let mut timings = serde_json::Map::new();
    let mut record = |name: &str, error: bool, elapsed: Duration| {
        dispatched.push((name.to_string(), error, elapsed));
        timings.insert(
            name.to_string(),
            json!({"error": error, "ms": elapsed.as_secs_f64() * 1000.0}),
        );
    };

    let started_at = Instant::now();
    let (started, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "process_start",
        json!({"executable":"node","argv":["-e",script],"ports":[port]}),
    );
    record("process_start", error, started_at.elapsed());
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

    let cold = Instant::now();
    let (_, output, error) =
        super::test_impact_integration_tests::call(&mut agent, "repo_map", json!({}));
    record("repo_map", error, cold.elapsed());
    assert!(!error, "{output}");

    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "lsp_document_symbols",
        json!({"path":"src/login.ts"}),
    );
    record("lsp_document_symbols", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "package_info",
        json!({"package":"login-app"}),
    );
    record("package_info", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "git_blame_symbol",
        json!({"symbol":"loginButtonLabel","path":"src/login.ts"}),
    );
    record("git_blame_symbol", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "git_commit_context",
        json!({"commit":"HEAD"}),
    );
    record("git_commit_context", error, Duration::from_millis(0));

    let impact_cold = Instant::now();
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "impact_analyze",
        json!({"files":["src/login.ts"]}),
    );
    record("impact_analyze", error, impact_cold.elapsed());
    assert!(!error, "{output}");
    let impact_warm = Instant::now();
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "impact_analyze",
        json!({"files":["src/login.ts"]}),
    );
    record("impact_analyze_warm", error, impact_warm.elapsed());
    assert!(!error, "{output}");

    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "test_plan",
        json!({"path":"src/login.ts"}),
    );
    record("test_plan", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "build_command",
        json!({"files":["src/login.ts"]}),
    );
    record("build_command", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "bash",
        json!({"command":"node --test src/login.test.ts"}),
    );
    record("bash", error, Duration::from_millis(0));
    let (_, _, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "lsp_diagnostics",
        json!({"path":"src/login.ts"}),
    );
    record("lsp_diagnostics", error, Duration::from_millis(0));

    let checkpoint = Instant::now();
    let (checkpointed, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "workspace_checkpoint",
        json!({"path":"src/login.ts","label":"login"}),
    );
    record("workspace_checkpoint", error, checkpoint.elapsed());
    assert!(!error, "{output}");
    let checkpoint_id = snapshot_checkpoint_id(&checkpointed);

    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "edit",
        json!({
            "path":"src/login.ts",
            "oldText":"return ok ? 'Broken' : 'Broken';",
            "newText":"return ok ? 'Login' : 'Broken';"
        }),
    );
    record("edit", error, Duration::from_millis(0));
    assert!(!error, "{output}");

    let browser_start = Instant::now();
    let (opened, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_open",
        json!({"process_id":process_id,"port":port,"host":"127.0.0.1"}),
    );
    record("browser_open", error, browser_start.elapsed());
    assert!(!error, "{output}");
    let browser_id = opened["browser_id"].as_str().unwrap().to_string();
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_click",
        json!({"browser_id":browser_id,"selector":{"kind":"role","role":"button","name":"Start"}}),
    );
    record("browser_click", error, Duration::from_millis(0));
    assert!(!error, "{output}");
    let (snapshot, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_snapshot",
        json!({"browser_id":browser_id}),
    );
    record("browser_snapshot", error, Duration::from_millis(0));
    assert!(!error, "{output}");
    assert!(
        snapshot["result"]["html"]
            .as_str()
            .unwrap_or("")
            .contains(">Broken</button>")
            || snapshot["result"]["html"]
                .as_str()
                .unwrap_or("")
                .contains(">Done</button>")
            || snapshot.to_string().contains("Broken")
            || snapshot.to_string().contains("Done"),
        "login button snapshot missing: {snapshot}"
    );
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_console",
        json!({"browser_id":browser_id}),
    );
    record("browser_console", error, Duration::from_millis(0));
    assert!(!error, "{output}");
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_network",
        json!({"browser_id":browser_id}),
    );
    record("browser_network", error, Duration::from_millis(0));
    assert!(!error, "{output}");

    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "workspace_diff",
        json!({"checkpointId":checkpoint_id}),
    );
    record("workspace_diff", error, Duration::from_millis(0));
    assert!(
        !error,
        "workspace_diff must succeed with checkpoint.id {checkpoint_id}: {output}"
    );
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "verification_plan",
        json!({"files":["src/login.ts"]}),
    );
    record("verification_plan", error, Duration::from_millis(0));
    assert!(!error, "{output}");

    drop(record);
    let required = [
        "repo_map",
        "lsp_document_symbols",
        "package_info",
        "git_blame_symbol",
        "impact_analyze",
        "process_start",
        "workspace_checkpoint",
        "edit",
        "lsp_diagnostics",
        "test_plan",
        "bash",
        "build_command",
        "browser_open",
        "browser_snapshot",
        "browser_console",
        "workspace_diff",
        "verification_plan",
    ];
    for name in required {
        assert!(
            dispatched.iter().any(|(tool, _, _)| tool == name),
            "missing live step {name} in {dispatched:?}"
        );
    }
    let normal_cache = cache_delta(&cache_before, &host.native.lock().unwrap().cache.stats());
    let memory_bytes = peak_memory_bytes().expect("P12 normal memory measurement");
    let cache_hits = normal_cache["hits"]
        .as_u64()
        .expect("P12 cache hit count must be numeric");
    assert!(
        cache_hits > 0,
        "P12 warm query must reuse the cache/index; metrics={normal_cache}"
    );
    let process_count = manager
        .execute(root.path(), "process_list", &json!({}), None, None)
        .expect("process_list must succeed for P12 metrics")
        .details
        .and_then(|details| details["processes"].as_array().map(Vec::len))
        .expect("process_list must return processes for P12 metrics");
    assert!(
        process_count > 0,
        "P12 normal process count must be observed: {process_count}"
    );
    timings.insert("startup_ms".into(), json!(startup_ms));
    timings.insert("memory_bytes".into(), json!(memory_bytes));
    timings.insert("cache".into(), normal_cache.clone());
    timings.insert("cache_hits".into(), normal_cache["hits"].clone());
    timings.insert("cache_misses".into(), normal_cache["misses"].clone());
    timings.insert("cache_hit_rate".into(), normal_cache["hit_rate"].clone());
    timings.insert("process_count".into(), json!(process_count));
    timings.insert("browser_success".into(), json!(true));
    timings.insert("platform".into(), json!(std::env::consts::OS));
    timings.insert("arch".into(), json!(std::env::consts::ARCH));
    let normal_metrics = serde_json::Value::Object(timings.clone());
    eprintln!(
        "P12_NORMAL_RECEIPT {}",
        json!({
            "mode": "normal",
            "dispatched": dispatched.iter().map(|(name, error, elapsed)| json!({
                "tool": name,
                "error": error,
                "ms": elapsed.as_secs_f64() * 1000.0
            })).collect::<Vec<_>>(),
            "checkpoint_id": checkpoint_id,
            "metrics": normal_metrics.clone()
        })
    );
    eprintln!("P12_LOGIN_METRICS {}", normal_metrics);

    let required_csv = required.join(",");
    let artifact = root.path().join("graph-artifact.json");
    let writer_guard = GraphEnvGuard::apply(&[
        ("PI_GRAPH_ROLE", "writer".into()),
        ("PI_GRAPH_EXPECT", "patch-report".into()),
        (
            "PI_GRAPH_ARTIFACT_PATH",
            artifact.to_string_lossy().into_owned(),
        ),
        ("PI_GRAPH_NODE_ID", "writer-login".into()),
        ("PI_GRAPH_AUTHORIZED_TOOLS", required_csv.clone()),
        ("DAVINCI_TASK_COORDINATOR_ADDR", "127.0.0.1:9".into()),
    ]);
    let graph_cache_before = host.native.lock().unwrap().cache.stats();
    let mut graph_dispatched = Vec::new();
    let mut graph_timings = serde_json::Map::new();
    let mut graph_record = |name: &str, error: bool, elapsed: Duration| {
        graph_dispatched.push((name.to_string(), error, elapsed));
        graph_timings.insert(
            name.to_string(),
            json!({"error": error, "ms": elapsed.as_secs_f64() * 1000.0}),
        );
    };

    let graph_started = Instant::now();
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "process_start",
        json!({"executable":"node","argv":["--version"]}),
    );
    graph_record("process_start", error, graph_started.elapsed());
    assert!(!error, "graph writer process_start: {output}");

    for (name, args) in [
        ("repo_map", json!({})),
        ("lsp_document_symbols", json!({"path":"src/login.ts"})),
        ("package_info", json!({"package":"login-app"})),
        (
            "git_blame_symbol",
            json!({"symbol":"loginButtonLabel","path":"src/login.ts"}),
        ),
        ("impact_analyze", json!({"files":["src/login.ts"]})),
        ("test_plan", json!({"path":"src/login.ts"})),
        ("build_command", json!({"files":["src/login.ts"]})),
        ("bash", json!({"command":"node --test src/login.test.ts"})),
        ("lsp_diagnostics", json!({"path":"src/login.ts"})),
        ("verification_plan", json!({"files":["src/login.ts"]})),
    ] {
        let started = Instant::now();
        let (_, output, error) = super::test_impact_integration_tests::call(&mut agent, name, args);
        graph_record(name, error, started.elapsed());
        if matches!(
            name,
            "repo_map"
                | "package_info"
                | "impact_analyze"
                | "test_plan"
                | "build_command"
                | "verification_plan"
        ) {
            assert!(!error, "graph writer {name}: {output}");
        }
    }

    let graph_checkpoint = Instant::now();
    let (graph_checkpointed, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "workspace_checkpoint",
        json!({"path":"src/login.ts","label":"graph-login"}),
    );
    graph_record("workspace_checkpoint", error, graph_checkpoint.elapsed());
    assert!(!error, "graph writer workspace_checkpoint: {output}");
    let graph_checkpoint_id = snapshot_checkpoint_id(&graph_checkpointed);

    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "edit",
        json!({
            "path":"src/login.ts",
            "oldText":"return ok ? 'Login' : 'Broken';",
            "newText":"return ok ? 'Login' : 'Denied';"
        }),
    );
    graph_record("edit", error, Duration::from_millis(0));
    assert!(!error, "graph writer edit: {output}");

    let graph_browser = Instant::now();
    let (graph_opened, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_open",
        json!({"process_id":process_id,"port":port,"host":"127.0.0.1"}),
    );
    graph_record("browser_open", error, graph_browser.elapsed());
    assert!(!error, "graph writer browser_open: {output}");
    let graph_browser_id = graph_opened["browser_id"].as_str().unwrap().to_string();
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_click",
        json!({"browser_id":graph_browser_id,"selector":{"kind":"role","role":"button","name":"Start"}}),
    );
    graph_record("browser_click", error, Duration::from_millis(0));
    assert!(!error, "graph writer browser_click: {output}");
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_snapshot",
        json!({"browser_id":graph_browser_id}),
    );
    graph_record("browser_snapshot", error, Duration::from_millis(0));
    assert!(!error, "graph writer browser_snapshot: {output}");
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_console",
        json!({"browser_id":graph_browser_id}),
    );
    graph_record("browser_console", error, Duration::from_millis(0));
    assert!(!error, "graph writer browser_console: {output}");
    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_network",
        json!({"browser_id":graph_browser_id}),
    );
    graph_record("browser_network", error, Duration::from_millis(0));
    assert!(!error, "graph writer browser_network: {output}");

    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "workspace_diff",
        json!({"checkpointId":graph_checkpoint_id}),
    );
    graph_record("workspace_diff", error, Duration::from_millis(0));
    assert!(
        !error,
        "graph writer workspace_diff must succeed with {graph_checkpoint_id}: {output}"
    );

    drop(graph_record);
    for name in required {
        assert!(
            graph_dispatched.iter().any(|(tool, _, _)| tool == name),
            "missing graph writer step {name} in {graph_dispatched:?}"
        );
    }
    let graph_cache = cache_delta(
        &graph_cache_before,
        &host.native.lock().unwrap().cache.stats(),
    );
    let graph_memory_bytes = peak_memory_bytes().expect("P12 graph memory measurement");
    let graph_cache_hits = graph_cache["hits"]
        .as_u64()
        .expect("P12 graph cache hit count must be numeric");
    assert!(
        graph_cache_hits > 0,
        "P12 graph warm query must reuse the cache/index; metrics={graph_cache}"
    );
    let graph_process_count = manager
        .execute(root.path(), "process_list", &json!({}), None, None)
        .expect("graph process_list must succeed for P12 metrics")
        .details
        .and_then(|details| details["processes"].as_array().map(Vec::len))
        .expect("graph process_list must return processes for P12 metrics");
    assert!(
        graph_process_count > 0,
        "P12 graph process count must be observed: {graph_process_count}"
    );
    graph_timings.insert("memory_bytes".into(), json!(graph_memory_bytes));
    graph_timings.insert("cache".into(), graph_cache.clone());
    graph_timings.insert("cache_hits".into(), graph_cache["hits"].clone());
    graph_timings.insert("cache_misses".into(), graph_cache["misses"].clone());
    graph_timings.insert("cache_hit_rate".into(), graph_cache["hit_rate"].clone());
    graph_timings.insert("process_count".into(), json!(graph_process_count));
    graph_timings.insert("browser_success".into(), json!(true));
    graph_timings.insert("platform".into(), json!(std::env::consts::OS));
    graph_timings.insert("arch".into(), json!(std::env::consts::ARCH));
    let graph_metrics = serde_json::Value::Object(graph_timings.clone());
    eprintln!(
        "P12_GRAPH_WRITER_RECEIPT {}",
        json!({
            "mode": "graph-writer",
            "dispatched": graph_dispatched.iter().map(|(name, error, elapsed)| json!({
                "tool": name,
                "error": error,
                "ms": elapsed.as_secs_f64() * 1000.0
            })).collect::<Vec<_>>(),
            "checkpoint_id": graph_checkpoint_id,
            "metrics": graph_metrics.clone()
        })
    );
    drop(writer_guard);

    let _classifier_guard = GraphEnvGuard::apply(&[
        ("PI_GRAPH_ROLE", "classifier".into()),
        ("PI_GRAPH_EXPECT", "patch-report".into()),
        (
            "PI_GRAPH_ARTIFACT_PATH",
            artifact.to_string_lossy().into_owned(),
        ),
        ("PI_GRAPH_NODE_ID", "classifier-login".into()),
        ("PI_GRAPH_AUTHORIZED_TOOLS", "repo_map".into()),
    ]);
    let (_, plan_output, plan_error) = super::test_impact_integration_tests::call(
        &mut agent,
        "verification_plan",
        json!({"files":["src/login.ts"]}),
    );
    let (_, restore_output, restore_error) = super::test_impact_integration_tests::call(
        &mut agent,
        "workspace_restore",
        json!({"checkpointId":graph_checkpoint_id}),
    );
    eprintln!(
        "P12_GRAPH_DENY_RECEIPT {}",
        json!({
            "mode": "graph-classifier",
            "verification_plan_error": plan_error,
            "verification_plan_output": plan_output,
            "workspace_restore_error": restore_error,
            "workspace_restore_output": restore_output
        })
    );
    assert!(
        plan_error,
        "classifier must be denied verification_plan: {plan_output}"
    );
    assert!(
        restore_error,
        "classifier must be denied workspace_restore: {restore_output}"
    );
    drop(_classifier_guard);

    if let Some(path) = std::env::var_os("DAVINCI_P12_EVAL_ARTIFACT") {
        let artifact_receipt = json!({
            "schema_version": 1,
            "test": "login_button_seventeen_step_normal_dispatch_and_graph_deny",
            "mode": "feature-on",
            "feature_enabled": true,
            "fixture_id": "login-button-seventeen-step",
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "normal": {
                "dispatched": dispatched.iter().map(|(name, error, elapsed)| json!({
                    "tool": name,
                    "error": error,
                    "ms": elapsed.as_secs_f64() * 1000.0
                })).collect::<Vec<_>>(),
                "checkpoint_id": checkpoint_id,
                "metrics": normal_metrics,
            },
            "graph_writer": {
                "dispatched": graph_dispatched.iter().map(|(name, error, elapsed)| json!({
                    "tool": name,
                    "error": error,
                    "ms": elapsed.as_secs_f64() * 1000.0
                })).collect::<Vec<_>>(),
                "checkpoint_id": graph_checkpoint_id,
                "metrics": graph_metrics,
            },
            "graph_classifier": {
                "verification_plan_error": plan_error,
                "workspace_restore_error": restore_error,
            },
        });
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&artifact_receipt)
                .expect("P12 receipt must serialize as JSON"),
        )
        .expect("P12 receipt artifact must be writable");
    }

    let (_, output, error) = super::test_impact_integration_tests::call(
        &mut agent,
        "browser_close",
        json!({"browser_id":browser_id}),
    );
    assert!(!error, "{output}");
    let _ = super::test_impact_integration_tests::call(
        &mut agent,
        "process_stop",
        json!({"id":process_id}),
    );
}
