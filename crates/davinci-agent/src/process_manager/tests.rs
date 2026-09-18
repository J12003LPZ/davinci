use super::*;
use crate::{turn::Preparation, PermissionMode, PermissionPolicy, PermissionRule};
use std::time::{Duration, Instant};

fn agent(directory: &Path, mode: PermissionMode) -> crate::Agent {
    let mut agent = crate::Agent::new("process fixture");
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(mode)));
    agent.cwd = directory.into();
    agent.tool_context.processes = Some(
        ProcessManager::new(
            directory,
            agent.tool_context.jobs.clone(),
            agent.permissions.clone(),
            SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "process_manager::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            },
        )
        .unwrap()
        .with_counters(agent.counters.clone()),
    );
    agent
}

#[test]
fn helper_entry() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        crate::jobs::supervisor::run();
    }
}

fn call(agent: &crate::Agent, id: &str, name: &str, args: &Value) -> ToolResult {
    match agent.prepare_tool_call(&agent.cwd, id, name, args, 0) {
        Preparation::Ready { .. } => agent.run_prepared_call(&agent.cwd, id, name, args, 0),
        Preparation::Immediate(result) => result,
        Preparation::Wait { .. } => panic!("fixture call must not be coalesced"),
    }
}

fn server() -> Value {
    json!({"executable":"node", "argv":["-e", "process.stdout.write('READY\\n');process.stdin.on('data',b=>process.stdout.write(b));setTimeout(()=>process.exit(0),20000)"]})
}

#[test]
fn process_worker_workspace_isolation_preserves_parent_shutdown_and_reuse() {
    let parent_dir = tempfile::tempdir().unwrap();
    let worker_dir = tempfile::tempdir().unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let session = agent(parent_dir.path(), PermissionMode::AlwaysApprove);
    let parent = session.tool_context.processes.as_ref().unwrap();
    let child = |cwd: &Path| {
        parent
            .child_lease_for_workspace(
                cwd,
                session.permissions.clone(),
                crate::shell_policy::ShellPolicyProfile::Permissive,
            )
            .unwrap()
    };
    let first = child(worker_dir.path());
    let second = child(worker_dir.path());
    let other = child(other_dir.path());
    let args = server();
    let started = first
        .execute(worker_dir.path(), "process_start", &args, None, None)
        .unwrap();
    let details = started.details.unwrap();
    let id = details["process"]["id"].as_u64().unwrap() as u32;
    assert_eq!(
        details["process"]["workspace"],
        json!(worker_dir.path().canonicalize().unwrap())
    );
    let reused = second
        .execute(worker_dir.path(), "process_start", &args, None, None)
        .unwrap()
        .details
        .unwrap();
    assert_eq!(reused["process"]["id"], id);
    assert_eq!(reused["reused"], true);
    assert!(first
        .execute(
            parent_dir.path(),
            "process_status",
            &json!({"id":id}),
            None,
            None
        )
        .is_err());
    assert!(other
        .execute(
            other_dir.path(),
            "process_status",
            &json!({"id":id}),
            None,
            None
        )
        .is_err());
    let separate = other
        .execute(other_dir.path(), "process_start", &args, None, None)
        .unwrap()
        .details
        .unwrap();
    assert_ne!(separate["process"]["id"], id);
    // Equal command cwd does not make distinct workspace roots equivalent.
    let nested_dir = worker_dir.path().join("nested");
    std::fs::create_dir(&nested_dir).unwrap();
    let nested = child(&nested_dir);
    let mut nested_args = server();
    nested_args["cwd"] = json!(nested_dir);
    let from_outer = first
        .execute(worker_dir.path(), "process_start", &nested_args, None, None)
        .unwrap()
        .details
        .unwrap();
    let from_nested = nested
        .execute(&nested_dir, "process_start", &nested_args, None, None)
        .unwrap()
        .details
        .unwrap();
    assert_ne!(from_outer["process"]["id"], from_nested["process"]["id"]);
    drop(first);
    assert!(second
        .execute(
            worker_dir.path(),
            "process_status",
            &json!({"id":id}),
            None,
            None
        )
        .is_ok());
    parent.shutdown();
    assert!(second
        .execute(
            worker_dir.path(),
            "process_status",
            &json!({"id":id}),
            None,
            None
        )
        .is_err());
    assert!(other
        .execute(other_dir.path(), "process_list", &json!({}), None, None)
        .is_err());
    let until = Instant::now() + Duration::from_secs(5);
    while session
        .tool_context
        .jobs
        .lock()
        .unwrap()
        .get(id)
        .unwrap()
        .status()
        == crate::jobs::JobStatus::Running
    {
        assert!(
            Instant::now() < until,
            "worktree server survived parent shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[test]
fn browser_socket_proof_rejects_foreign_listener_and_port_takeover() {
    for host in ["127.0.0.1", "::1"] {
        browser_socket_proof_for(host);
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn browser_socket_proof_for(host: &str) {
    let address: std::net::IpAddr = host.parse().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let session = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = session.tool_context.processes.as_ref().unwrap();
    let foreign = std::net::TcpListener::bind((address, 0)).unwrap();
    let port = foreign.local_addr().unwrap().port();
    let mut start = server();
    start["ports"] = json!([port]);
    let started = manager
        .execute(directory.path(), "process_start", &start, None, None)
        .unwrap();
    let id = started.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    let args = json!({"process_id":id,"port":port,"host":host});
    let lease = manager
        .with_browser_dev_server(
            BrowserRequest {
                cwd: directory.path(),
                name: "browser_open",
                args: &args,
                abort: None,
                permit: None,
                process_id: id,
                port,
                lease: None,
            },
            |lease| Ok(lease.clone()),
        )
        .unwrap();
    // Metadata alone admits a binding but must never admit browser network I/O.
    assert!(lease.verify_listening_socket().is_err());
    assert!(manager
        .with_verified_browser_dev_server(
            BrowserRequest {
                cwd: directory.path(),
                name: "browser_open",
                args: &args,
                abort: None,
                permit: None,
                process_id: id,
                port,
                lease: Some(&lease),
            },
            |_| -> Result<(), String> { panic!("foreign listener must not enter browser I/O") }
        )
        .is_err());

    for descendant in [false, true] {
        let reserved = std::net::TcpListener::bind((address, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let script = format!("const s=require('net').createServer();s.listen({port},'{host}',()=>console.log('LISTENING'));process.stdin.on('data',()=>s.close(()=>console.log('CLOSED')));setTimeout(()=>process.exit(),20000)");
        let script = if descendant {
            format!("const c=require('child_process').spawn(process.execPath,['-e',{}],{{stdio:['pipe','pipe','pipe']}});c.stdout.pipe(process.stdout);c.stderr.pipe(process.stderr);process.stdin.pipe(c.stdin);setTimeout(()=>process.exit(),20000)", serde_json::to_string(&script).unwrap())
        } else {
            script
        };
        let started = manager
            .execute(
                directory.path(),
                "process_start",
                &json!({
                    "executable":"node", "argv":["-e",script], "ports":[port]
                }),
                None,
                None,
            )
            .unwrap();
        let id = started.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
        wait_for(|| {
            manager
                .owner
                .output(id, None, 8192)
                .unwrap()
                .text
                .contains("LISTENING")
        });
        let args = json!({"process_id":id,"port":port,"host":host});
        let lease = manager
            .with_browser_dev_server(
                BrowserRequest {
                    cwd: directory.path(),
                    name: "browser_open",
                    args: &args,
                    abort: None,
                    permit: None,
                    process_id: id,
                    port,
                    lease: None,
                },
                |lease| Ok(lease.clone()),
            )
            .unwrap();
        assert!(
            lease.verify_listening_socket().is_ok(),
            "descendant={descendant}"
        );
        let request = BrowserRequest {
            cwd: directory.path(),
            name: "browser_snapshot",
            args: &args,
            abort: None,
            permit: None,
            process_id: id,
            port,
            lease: Some(&lease),
        };
        assert!(manager
            .with_verified_browser_dev_server(request, |_| Ok(()))
            .is_ok());
        let mut takeover = None;
        let result = manager.with_verified_browser_dev_server(request, |_| {
            manager
                .execute(
                    directory.path(),
                    "process_write",
                    &json!({"id":id,"text":"CLOSE\n"}),
                    None,
                    None,
                )
                .unwrap();
            wait_for(|| {
                manager
                    .owner
                    .output(id, None, 8192)
                    .unwrap()
                    .text
                    .contains("CLOSED")
            });
            takeover = Some(std::net::TcpListener::bind((address, port)).unwrap());
            Ok("untrusted browser result")
        });
        assert!(result.is_err());
        assert!(lease.verify_listening_socket().is_err());
        assert_eq!(manager.owner.active_snapshot(id).unwrap().state, "running");
        drop(takeover);
        manager
            .execute(
                directory.path(),
                "process_stop",
                &json!({"id":id}),
                None,
                None,
            )
            .unwrap();
    }
}

#[test]
fn browser_dev_server_requires_active_owned_declared_port_and_current_authority() {
    let directory = tempfile::tempdir().unwrap();
    let session = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = session.tool_context.processes.as_ref().unwrap();
    let mut start = server();
    start["ports"] = json!([3000]);
    let result = manager
        .execute(directory.path(), "process_start", &start, None, None)
        .unwrap();
    let id = result.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    let args = json!({"process_id":id,"port":3000});
    let request = BrowserRequest {
        cwd: directory.path(),
        name: "browser_open",
        args: &args,
        abort: None,
        permit: None,
        process_id: id,
        port: 3000,
        lease: None,
    };
    let lease = manager
        .with_browser_dev_server(request, |lease| Ok(lease.clone()))
        .unwrap();
    assert_eq!(lease.origin(), "http://127.0.0.1:3000");
    let ipv6_args = json!({"process_id":id,"port":3000,"host":"::1"});
    let ipv6_request = BrowserRequest {
        args: &ipv6_args,
        ..request
    };
    let ipv6_lease = manager
        .with_browser_dev_server(ipv6_request, |lease| Ok(lease.clone()))
        .unwrap();
    assert_eq!(ipv6_lease.origin(), "http://[::1]:3000");
    assert_ne!(ipv6_lease, lease);
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                lease: Some(&lease),
                ..ipv6_request
            },
            |_| Ok(())
        )
        .is_err());
    let invalid_args = json!({"process_id":id,"port":3000,"host":"external.example"});
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                args: &invalid_args,
                ..request
            },
            |_| Ok(())
        )
        .is_err());
    assert!(lease.is_live());
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                port: 3001,
                ..request
            },
            |_| Ok(())
        )
        .is_err());
    let foreign = agent(directory.path(), PermissionMode::AlwaysApprove);
    assert!(foreign
        .tool_context
        .processes
        .as_ref()
        .unwrap()
        .with_browser_dev_server(request, |_| Ok(()))
        .is_err());
    let child = manager.child_lease(
        session.permissions.clone(),
        crate::shell_policy::ShellPolicyProfile::Permissive,
    );
    let reused = child
        .execute(directory.path(), "process_start", &start, None, None)
        .unwrap();
    let reused = reused.details.unwrap();
    assert_eq!(reused["process"]["id"], id);
    assert_eq!(reused["reused"], true);
    let child_lease = child
        .with_browser_dev_server(request, |lease| Ok(lease.clone()))
        .unwrap();
    manager.owner.release(id).unwrap();
    assert!(!lease.is_live());
    assert!(child_lease.is_live());
    assert_eq!(child.owner.active_snapshot(id).unwrap().state, "running");
    // Historical status is still useful; it is not authority to attach a browser.
    assert!(manager.owner.snapshot(id).is_ok());
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                name: "browser_snapshot",
                lease: Some(&lease),
                ..request
            },
            |_| Ok(())
        )
        .is_err());
    drop(child);
    assert!(
        !child_lease.is_live(),
        "observation must not retain its owner"
    );
}

#[test]
fn browser_dev_server_refuses_revocation_and_cancellation_before_callback() {
    let directory = tempfile::tempdir().unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = agent.tool_context.processes.as_ref().unwrap();
    let mut start = server();
    start["ports"] = json!([3000]);
    let result = manager
        .execute(directory.path(), "process_start", &start, None, None)
        .unwrap();
    let id = result.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    let args = json!({"process_id":id,"port":3000});
    let request = BrowserRequest {
        cwd: directory.path(),
        name: "browser_open",
        args: &args,
        abort: None,
        permit: None,
        process_id: id,
        port: 3000,
        lease: None,
    };
    let lease = manager
        .with_browser_dev_server(request, |lease| Ok(lease.clone()))
        .unwrap();
    let abort = AtomicBool::new(true);
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                name: "browser_snapshot",
                abort: Some(&abort),
                lease: Some(&lease),
                ..request
            },
            |_| -> Result<(), String> { panic!("cancelled callback") }
        )
        .is_err());
    agent.permissions.lock().unwrap().mode = PermissionMode::ReadOnly;
    assert!(manager
        .with_browser_dev_server(
            BrowserRequest {
                name: "browser_click",
                lease: Some(&lease),
                ..request
            },
            |_| -> Result<(), String> { panic!("revoked callback") }
        )
        .is_err());
}

#[test]
fn browser_dev_server_rejects_evidence_after_authority_changes_during_callback() {
    for change in ["permission", "cancel", "release"] {
        let directory = tempfile::tempdir().unwrap();
        let session = agent(directory.path(), PermissionMode::AlwaysApprove);
        let manager = session.tool_context.processes.as_ref().unwrap();
        let mut start = server();
        start["ports"] = json!([3000]);
        let result = manager
            .execute(directory.path(), "process_start", &start, None, None)
            .unwrap();
        let id = result.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
        let args = json!({"process_id":id,"port":3000});
        let abort = AtomicBool::new(false);
        let result = manager.with_browser_dev_server(
            BrowserRequest {
                cwd: directory.path(),
                name: "browser_click",
                args: &args,
                abort: Some(&abort),
                permit: None,
                process_id: id,
                port: 3000,
                lease: None,
            },
            |_| {
                match change {
                    "permission" => {
                        session.permissions.lock().unwrap().mode = PermissionMode::ReadOnly
                    }
                    "cancel" => abort.store(true, Ordering::SeqCst),
                    "release" => manager.owner.release(id).unwrap(),
                    _ => unreachable!(),
                }
                Ok("stale browser evidence")
            },
        );
        assert!(result.is_err(), "accepted stale evidence after {change}");
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < until, "process fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn normal_agent_managed_process_approval_reuse_stdin_and_stop() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = agent(directory.path(), PermissionMode::Ask);
    agent.set_runtime(crate::RuntimeHandle::new(
        crate::RunId::new(),
        crate::AgentId::new(),
        crate::runtime::RuntimeBus::new(),
    ));
    let approvals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = approvals.clone();
    agent.approval_responder = Some(crate::approval::ApprovalResponder(Arc::new(
        move |_, challenge| {
            counted.fetch_add(1, Ordering::SeqCst);
            crate::approval::ApprovalReply {
                challenge_id: challenge.id,
                choice_id: "once".into(),
                instructions: None,
            }
        },
    )));
    let args = server();
    let first = call(&agent, "first", "process_start", &args);
    assert!(!first.is_error, "{}", first.content);
    let first = first.details.unwrap();
    let id = first["process"]["id"].as_u64().unwrap();
    assert_eq!(first["reused"], false);
    let second = call(&agent, "second", "process_start", &args);
    assert!(!second.is_error, "{}", second.content);
    let second = second.details.unwrap();
    assert_eq!(second["process"]["id"], id);
    assert_eq!(second["reused"], true);
    assert_eq!(agent.counters.process_startups.load(Ordering::Relaxed), 1);
    assert_eq!(agent.counters.process_reuses.load(Ordering::Relaxed), 1);
    assert_eq!(
        first["process"]["origin"]["agent_id"],
        json!(agent.runtime.as_ref().unwrap().agent_id)
    );
    assert_eq!(approvals.load(Ordering::SeqCst), 2);
    assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
    let manager = agent.tool_context.processes.as_ref().unwrap();
    assert!(manager
        .execute(directory.path(), "process_start", &args, None, None)
        .is_err());
    assert!(
        !call(
            &agent,
            "stdin",
            "process_write",
            &json!({"id":id,"text":"roundtrip\n"})
        )
        .is_error
    );
    wait_for(|| {
        manager
            .execute(
                directory.path(),
                "process_output",
                &json!({"id":id}),
                None,
                None,
            )
            .unwrap()
            .content
            .contains("roundtrip")
    });
    assert!(!call(&agent, "stop", "process_stop", &json!({"id":id})).is_error);
    wait_for(|| {
        manager
            .execute(
                directory.path(),
                "process_status",
                &json!({"id":id}),
                None,
                None,
            )
            .unwrap()
            .details
            .unwrap()["state"]
            == "exited"
    });
}

#[test]
fn process_dispatch_rechecks_one_call_arguments_and_policy() {
    for change in ["args", "policy", "revoked"] {
        let directory = tempfile::tempdir().unwrap();
        let mut agent = agent(directory.path(), PermissionMode::Ask);
        agent.approval_responder = Some(crate::approval::ApprovalResponder(Arc::new(
            |_, challenge| crate::approval::ApprovalReply {
                challenge_id: challenge.id,
                choice_id: "once".into(),
                instructions: None,
            },
        )));
        let mut args = server();
        assert!(matches!(
            agent.prepare_tool_call(directory.path(), "call", "process_start", &args, 0),
            Preparation::Ready { .. }
        ));
        match change {
            "args" => args["argv"] = json!(["-e", "process.exit(0)"]),
            "policy" => agent.permissions.lock().unwrap().mode = PermissionMode::ReadOnly,
            _ => agent.approval_registry.revoke_all(),
        }
        assert!(
            agent
                .run_prepared_call(directory.path(), "call", "process_start", &args, 0)
                .is_error,
            "{change}"
        );
        assert!(agent
            .tool_context
            .processes
            .as_ref()
            .unwrap()
            .owner
            .ids()
            .is_empty());
    }
}

#[test]
fn process_every_operation_obeys_live_named_denial() {
    let directory = tempfile::tempdir().unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = agent.tool_context.processes.as_ref().unwrap();
    let started = manager
        .execute(directory.path(), "process_start", &server(), None, None)
        .unwrap();
    let id = started.details.unwrap()["process"]["id"].as_u64().unwrap();
    for (name, args) in [
        ("process_start", server()),
        ("process_status", json!({"id":id})),
        ("process_output", json!({"id":id})),
        ("process_write", json!({"id":id,"text":"blocked"})),
        ("process_stop", json!({"id":id})),
        ("process_list", json!({})),
    ] {
        agent.permissions.lock().unwrap().deny = vec![PermissionRule::bare(name)];
        assert!(
            manager
                .execute(directory.path(), name, &args, None, None)
                .is_err(),
            "{name}"
        );
    }
    agent.permissions.lock().unwrap().deny.clear();
    manager
        .execute(
            directory.path(),
            "process_stop",
            &json!({"id":id}),
            None,
            None,
        )
        .unwrap();
}

#[test]
fn process_validation_rejects_malformed_arguments_and_workspace_escape_without_spawn() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = agent.tool_context.processes.as_ref().unwrap();
    for args in [
        json!({"executable":"node","argv":"wrong"}),
        json!({"executable":"node","argv":["a\0b"]}),
        json!({"executable":"node","command":"echo forbidden"}),
        json!({"executable":"node","cwd":outside.path()}),
        json!({"executable":"node","env":{"PATH":"untrusted"}}),
        json!({"executable":"node","env":{"NODE_OPTIONS":"--require attacker.js"}}),
        json!({"executable":"node","env":{"=bad":"x"}}),
        json!({"executable":"node","restart":{"max_restarts":4,"backoff_ms":50}}),
        json!({"executable":"node","restart":{"max_restarts":1,"backoff_ms":0}}),
        json!({"executable":"node","ports":[80,80]}),
        json!({"executable":"node","ports":[0]}),
        json!({"executable":"node","provenance":{"session_id":"forged"}}),
    ] {
        assert!(manager
            .execute(directory.path(), "process_start", &args, None, None)
            .is_err());
    }
    assert!(manager.owner.ids().is_empty());
    let resolved = command::resolve(
        &directory.path().canonicalize().unwrap(),
        directory.path(),
        command::Start {
            executable: "node".into(),
            argv: vec!["literal ; $() spaces".into()],
            cwd: None,
            env: std::collections::BTreeMap::from([("FIXTURE_EXPLICIT".into(), "value".into())]),
            restart: Default::default(),
            ports: vec![],
        },
    )
    .unwrap();
    assert_eq!(resolved.argv, ["literal ; $() spaces"]);
    assert_eq!(resolved.environment["FIXTURE_EXPLICIT"], "value");
    assert!(resolved.environment.keys().all(|key| matches!(
        key.as_str(),
        "PATH"
            | "SystemRoot"
            | "WINDIR"
            | "TEMP"
            | "TMP"
            | "TMPDIR"
            | "LANG"
            | "LC_ALL"
            | "LC_CTYPE"
            | "FIXTURE_EXPLICIT"
    )));
    assert!(resolved.executable.is_absolute());
}

#[test]
fn process_hard_contract_rejects_unconfined_start_and_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    agent.set_active_contract(
        crate::runtime::TaskContract::new(
            "managed-contract",
            1,
            crate::TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec!["target/".into()],
        )
        .unwrap(),
    );
    for (name, args) in [
        ("process_start", server()),
        ("process_write", json!({"id":1,"text":"data"})),
    ] {
        let result = call(&agent, name, name, &args);
        assert!(result.is_error);
        assert!(
            result.content.contains("execution_contract_unenforceable"),
            "{}",
            result.content
        );
    }
    assert!(agent
        .tool_context
        .processes
        .as_ref()
        .unwrap()
        .owner
        .ids()
        .is_empty());
}

#[test]
fn process_bounded_restart_records_attempts_and_declared_ports() {
    let directory = tempfile::tempdir().unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = agent.tool_context.processes.as_ref().unwrap();
    let args = json!({
        "executable":"node", "argv":["-e","console.log('attempt');process.exit(7)"],
        "restart":{"max_restarts":2,"backoff_ms":50}, "ports":[4100]
    });
    let started = manager
        .execute(directory.path(), "process_start", &args, None, None)
        .unwrap();
    let id = started.details.unwrap()["process"]["id"].as_u64().unwrap();
    let status = || {
        manager
            .execute(
                directory.path(),
                "process_status",
                &json!({"id":id}),
                None,
                None,
            )
            .unwrap()
            .details
            .unwrap()
    };
    assert_eq!(
        manager
            .owner
            .wait(id as u32, Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .code,
        Some(7)
    );
    assert_eq!(
        status()["state"],
        "exited",
        "wait must cover the logical process including retries"
    );
    wait_for(|| status()["state"] == "exited");
    let final_state = status();
    assert_eq!(final_state["exit_code"], 7);
    assert_eq!(final_state["restart"]["attempts"], 2);
    assert_eq!(
        final_state["ports"],
        json!([{"port":4100,"source":"request","verified":false}])
    );
    let output = manager
        .execute(
            directory.path(),
            "process_output",
            &json!({"id":id}),
            None,
            None,
        )
        .unwrap()
        .details
        .unwrap();
    assert_eq!(
        output["text"].as_str().unwrap().matches("attempt").count(),
        3
    );
    assert_eq!(agent.counters.process_startups.load(Ordering::Relaxed), 3);
    assert_eq!(agent.counters.process_restarts.load(Ordering::Relaxed), 2);
}

#[test]
fn process_restart_rechecks_current_authority_and_stop_during_backoff() {
    for revoke in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
        let manager = agent.tool_context.processes.as_ref().unwrap();
        let args = json!({
            "executable":"node", "argv":["-e","console.log('attempt');process.exit(7)"],
            "restart":{"max_restarts":3,"backoff_ms":500}
        });
        let id = manager
            .execute(directory.path(), "process_start", &args, None, None)
            .unwrap()
            .details
            .unwrap()["process"]["id"]
            .as_u64()
            .unwrap();
        let status = || {
            manager
                .execute(
                    directory.path(),
                    "process_status",
                    &json!({"id":id}),
                    None,
                    None,
                )
                .unwrap()
                .details
                .unwrap()
        };
        wait_for(|| status()["state"] == "restarting");
        if revoke {
            agent
                .permissions
                .lock()
                .unwrap()
                .deny
                .push(PermissionRule::bare("process_start"));
        } else {
            manager
                .execute(
                    directory.path(),
                    "process_stop",
                    &json!({"id":id}),
                    None,
                    None,
                )
                .unwrap();
        }
        wait_for(|| status()["state"] == "exited");
        assert_eq!(status()["restart"]["attempts"], 0);
        if revoke {
            assert!(status()["restart"]["error"].is_string());
        }
    }
}

#[test]
fn process_session_reload_preserves_lease_and_switch_revokes_descendant_owners() {
    let directory = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let first =
        davinci_session::JsonlSession::create(sessions.path(), "process fixture", None).unwrap();
    let first_path = first.path.clone();
    agent.load_from_session(first).unwrap();
    let old = agent.tool_context.processes.as_ref().unwrap().clone();
    let child = old.child_lease(
        agent.permissions.clone(),
        crate::shell_policy::ShellPolicyProfile::Permissive,
    );
    let started = child
        .execute(directory.path(), "process_start", &server(), None, None)
        .unwrap();
    let id = started.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    agent
        .load_from_session(davinci_session::JsonlSession::open(&first_path).unwrap())
        .unwrap();
    assert!(child
        .execute(
            directory.path(),
            "process_status",
            &json!({"id":id}),
            None,
            None
        )
        .is_ok());
    let next =
        davinci_session::JsonlSession::create(sessions.path(), "process fixture", None).unwrap();
    agent.load_from_session(next).unwrap();
    for manager in [&old, &child] {
        assert!(manager
            .execute(directory.path(), "process_list", &json!({}), None, None)
            .is_err());
        assert!(manager
            .execute(directory.path(), "process_start", &server(), None, None)
            .is_err());
    }
    wait_for(|| {
        !agent
            .tool_context
            .jobs
            .lock()
            .unwrap()
            .get(id)
            .unwrap()
            .status()
            .is_running()
    });
    let result = agent
        .tool_context
        .processes
        .as_ref()
        .unwrap()
        .execute(directory.path(), "process_list", &json!({}), None, None)
        .unwrap();
    assert_eq!(result.details.unwrap()["processes"], json!([]));
}

#[test]
fn process_retries_never_replay_one_call_approval_or_successful_exit() {
    for (mode, code, restart) in [
        (
            PermissionMode::Ask,
            7,
            json!({"max_restarts":2,"backoff_ms":50}),
        ),
        (
            PermissionMode::AlwaysApprove,
            0,
            json!({"max_restarts":2,"backoff_ms":50}),
        ),
        (
            PermissionMode::AlwaysApprove,
            7,
            json!({"max_restarts":0,"backoff_ms":0}),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut agent = agent(directory.path(), mode);
        agent.approval_responder = Some(crate::approval::ApprovalResponder(Arc::new(
            |_, challenge| crate::approval::ApprovalReply {
                challenge_id: challenge.id,
                choice_id: "once".into(),
                instructions: None,
            },
        )));
        let result = call(
            &agent,
            "restart",
            "process_start",
            &json!({
                "executable":"node", "argv":["-e",format!("process.exit({code})")],"restart":restart
            }),
        );
        assert!(!result.is_error, "{}", result.content);
        let id = result.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
        let owner = &agent.tool_context.processes.as_ref().unwrap().owner;
        assert_eq!(
            owner
                .wait(id, Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .code,
            Some(code)
        );
        assert_eq!(owner.snapshot(id).unwrap().restart["attempts"], 0);
        assert_eq!(agent.counters.process_startups.load(Ordering::Relaxed), 1);
    }
}

#[cfg(windows)]
#[test]
fn process_windows_npm_uses_node_entrypoint_with_literal_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    let manager = agent.tool_context.processes.as_ref().unwrap();
    let result = manager
        .execute(
            directory.path(),
            "process_start",
            &json!({
                "executable":"npm", "argv":["--version"]
            }),
            None,
            None,
        )
        .unwrap();
    let id = result.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    let exit = manager
        .owner
        .wait(id, Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(
        exit.code,
        Some(0),
        "{}",
        manager.owner.output(id, None, 8192).unwrap().text
    );
    let snapshot = manager.owner.snapshot(id).unwrap();
    assert!(snapshot.executable.ends_with("node.exe"));
    assert!(snapshot.argv[0].ends_with("npm-cli.js"));
    assert_eq!(snapshot.argv[1], "--version");
}

#[cfg(unix)]
#[test]
fn process_cwd_symlink_cannot_escape_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
    let agent = agent(directory.path(), PermissionMode::AlwaysApprove);
    assert!(agent
        .tool_context
        .processes
        .as_ref()
        .unwrap()
        .execute(
            directory.path(),
            "process_start",
            &json!({"executable":"node","cwd":"escape"}),
            None,
            None,
        )
        .is_err());
}
