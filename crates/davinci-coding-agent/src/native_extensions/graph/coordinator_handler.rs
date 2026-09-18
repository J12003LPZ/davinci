//! Native capabilities carried by the existing authenticated worker channel.
use super::{controller::ControllerDeps, roles, types::WorkerSpec};
use crate::native_extensions::language_intelligence::LanguageIntelligence;
use davinci_agent::{
    process_manager::ProcessManager, runtime::task_transport::CoordinatorToolHandler, ToolError,
    ToolResult,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

struct ParentTools {
    language: Option<LanguageIntelligence>,
    processes: Option<ProcessManager>,
    browser: Option<crate::native_extensions::browser::BrowserWorker>,
    cwd: PathBuf,
    contracted: bool,
    abort: Arc<AtomicBool>,
}

pub(super) fn for_worker(
    deps: &ControllerDeps,
    spec: &WorkerSpec,
    abort: Arc<AtomicBool>,
) -> Option<Arc<dyn CoordinatorToolHandler>> {
    let language = deps
        .language_intelligence
        .as_ref()
        .map(|manager| manager.for_workspace(&spec.cwd));
    let processes = deps
        .processes
        .as_ref()
        .zip(deps.permissions.as_ref())
        .and_then(|(manager, permissions)| {
            manager
                .child_lease_for_workspace(
                    &spec.cwd,
                    permissions.clone(),
                    roles::shell_profile(roles::role_bash_policy(spec.role)),
                )
                .ok()
                .map(|manager| {
                    manager.with_provenance(davinci_agent::jobs::managed::Provenance {
                        session_id: deps
                            .runtime
                            .as_ref()
                            .and_then(|runtime| runtime.session_id.clone()),
                        agent_id: spec.runtime_agent_id,
                        task_id: spec.task_contract.as_ref().map(|contract| contract.task_id),
                        graph_node: Some(spec.task_id.clone()),
                    })
                })
        });
    let browser = deps
        .browser
        .as_ref()
        .zip(processes.as_ref())
        .and_then(|(host, manager)| {
            host.for_worker(&spec.cwd, manager.clone(), abort.clone())
                .ok()
        });
    (language.is_some() || processes.is_some() || browser.is_some()).then(|| {
        Arc::new(ParentTools {
            language,
            processes,
            browser,
            cwd: spec.cwd.clone(),
            contracted: spec.task_contract.is_some(),
            abort,
        }) as Arc<dyn CoordinatorToolHandler>
    })
}

impl CoordinatorToolHandler for ParentTools {
    fn handles(&self, tool: &str) -> bool {
        self.browser.is_some() && crate::native_extensions::browser::TOOL_NAMES.contains(&tool)
            || self.processes.is_some() && davinci_agent::tools::is_managed_process_tool(tool)
            || self
                .language
                .as_ref()
                .is_some_and(|manager| manager.handles(tool))
    }

    fn execute(&self, tool: &str, args: &Value) -> Result<ToolResult, ToolError> {
        if crate::native_extensions::browser::TOOL_NAMES.contains(&tool) {
            if self.contracted {
                return Err(ToolError::Failed(
                    "execution_contract_unenforceable: browser has no contracted sandbox".into(),
                ));
            }
            return self
                .browser
                .as_ref()
                .ok_or_else(|| ToolError::Failed("worker browser unavailable".into()))?
                .execute(&self.cwd, tool, args);
        }
        if davinci_agent::tools::is_managed_process_tool(tool) {
            if self.contracted && matches!(tool, "process_start" | "process_write") {
                return Err(ToolError::Failed("execution_contract_unenforceable: managed processes have no contracted sandbox".into()));
            }
            return self
                .processes
                .as_ref()
                .ok_or_else(|| ToolError::Failed("managed processes unavailable".into()))?
                .execute(&self.cwd, tool, args, Some(&self.abort), None)
                .map_err(ToolError::Failed);
        }
        CoordinatorToolHandler::execute(
            self.language
                .as_ref()
                .ok_or_else(|| ToolError::Failed("native worker capability unavailable".into()))?,
            tool,
            args,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{roles, types::Role};
    use super::*;
    use davinci_agent::{
        jobs::{supervisor::SupervisorCommand, JobBook, JobStatus},
        runtime::{
            task_transport::TaskCoordinatorTransport, AgentId, AgentKind, AgentRecord, AgentState,
            RunId, RuntimeBus, RuntimeHandle,
        },
        PermissionMode, PermissionPolicy, PermissionRule, PermissionState,
    };
    use serde_json::json;
    use std::{
        path::Path,
        sync::Mutex,
        time::{Duration, Instant},
    };

    #[test]
    fn helper_entry() {
        if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
            davinci_agent::jobs::supervisor::run();
        }
    }

    fn parent() -> RuntimeHandle {
        RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
    }

    fn transport(
        parent: &RuntimeHandle,
        manager: &ProcessManager,
        permissions: &Arc<PermissionState>,
        cwd: &Path,
        contracted: bool,
    ) -> TaskCoordinatorTransport {
        transport_with_browser(parent, manager, permissions, cwd, contracted, None)
    }

    fn transport_with_browser(
        parent: &RuntimeHandle,
        manager: &ProcessManager,
        permissions: &Arc<PermissionState>,
        cwd: &Path,
        contracted: bool,
        browser: Option<&crate::native_extensions::browser::BrowserWorkerHost>,
    ) -> TaskCoordinatorTransport {
        transport_with_browser_abort(
            parent,
            manager,
            permissions,
            cwd,
            contracted,
            browser,
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn transport_with_browser_abort(
        parent: &RuntimeHandle,
        manager: &ProcessManager,
        permissions: &Arc<PermissionState>,
        cwd: &Path,
        contracted: bool,
        browser: Option<&crate::native_extensions::browser::BrowserWorkerHost>,
        abort: Arc<AtomicBool>,
    ) -> TaskCoordinatorTransport {
        let child = AgentId::new();
        parent
            .registry
            .register_agent(AgentRecord {
                id: child,
                run_id: parent.run_id,
                parent: Some(parent.agent_id),
                kind: AgentKind::GraphWorker,
                name: "process fixture".into(),
                provider: "fixture".into(),
                model_id: "fixture".into(),
                cwd: cwd.into(),
                state: AgentState::Running,
                task_id: None,
                worktree: None,
                started_ms: 0,
                updated_ms: 0,
                failure_reason: None,
            })
            .unwrap();
        let processes = manager
            .child_lease_for_workspace(
                cwd,
                permissions.clone(),
                roles::shell_profile(roles::role_bash_policy(Role::Writer)),
            )
            .unwrap();
        let mut tools: Vec<String> = [
            "process_start",
            "process_status",
            "process_output",
            "process_write",
        ]
        .map(str::to_string)
        .into();
        if browser.is_some() {
            tools.extend(
                crate::native_extensions::browser::TOOL_NAMES
                    .iter()
                    .map(|v| (*v).to_string()),
            );
        }
        let browser = browser.map(|host| {
            host.for_worker(cwd, processes.clone(), abort.clone())
                .unwrap()
        });
        TaskCoordinatorTransport::bind_with_handler(
            parent,
            child,
            permissions.clone(),
            tools,
            cwd.into(),
            abort.clone(),
            Some(Arc::new(ParentTools {
                language: None,
                browser,
                processes: Some(processes),
                cwd: cwd.into(),
                contracted,
                abort,
            })),
        )
        .unwrap()
    }

    fn manager(
        cwd: &Path,
        jobs: Arc<Mutex<JobBook>>,
        permissions: Arc<PermissionState>,
    ) -> ProcessManager {
        ProcessManager::new(
            cwd,
            jobs,
            permissions,
            SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "native_extensions::graph::coordinator_handler::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            },
        )
        .unwrap()
    }

    #[test]
    fn graph_worker_node_resolves_relative_script_from_canonical_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("relative.cjs"),
            "console.log('RELATIVE_SCRIPT_OK')",
        )
        .unwrap();
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(dir.path(), jobs, permissions.clone());
        let parent = parent();
        let worker = transport(&parent, &manager, &permissions, dir.path(), false);
        let started = worker
            .client()
            .call(
                "process_start",
                &json!({"executable":"node","argv":["relative.cjs"]}),
            )
            .unwrap()
            .details
            .unwrap();
        let id = &started["process"]["id"];
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let output = worker
                .client()
                .call("process_output", &json!({"id":id}))
                .unwrap();
            if output.content.contains("RELATIVE_SCRIPT_OK") {
                break;
            }
            assert!(
                Instant::now() < until,
                "relative script failed: {}",
                output.content
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn graph_browser_parent_rejects_unenforceable_contract() {
        use crate::native_extensions::browser::{BrowserController, BrowserWorkerHost};
        let dir = tempfile::tempdir().unwrap();
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(
            dir.path(),
            Arc::new(Mutex::new(JobBook::default())),
            permissions.clone(),
        );
        let host = BrowserWorkerHost {
            controller: BrowserController::default(),
            supervisor: SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![],
            },
        };
        let parent = parent();
        let worker = transport_with_browser(
            &parent,
            &manager,
            &permissions,
            dir.path(),
            true,
            Some(&host),
        );
        let result = worker
            .client()
            .call("browser_open", &json!({"process_id":1,"port":1234}));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("execution_contract_unenforceable"));
    }

    #[test]
    #[ignore = "requires explicitly configured trusted Node and Playwright installation"]
    fn graph_browser_transport_shares_server_and_isolates_worker_contexts() {
        for host in ["127.0.0.1", "::1"] {
            for worktree in [false, true] {
                graph_browser_transport_for(host, worktree);
            }
        }
    }

    fn graph_browser_transport_for(loopback_host: &str, worktree: bool) {
        use crate::native_extensions::browser::{
            BrowserConfig, BrowserController, BrowserWorkerHost,
        };
        let parent_dir = tempfile::tempdir().unwrap();
        let worker_dir = tempfile::tempdir().unwrap();
        if worktree {
            let git = |args: &[&std::ffi::OsStr]| {
                let output = std::process::Command::new("git")
                    .current_dir(parent_dir.path())
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            };
            git(&["init".as_ref(), "--quiet".as_ref()]);
            git(&[
                "-c".as_ref(),
                "user.name=fixture".as_ref(),
                "-c".as_ref(),
                "user.email=fixture@example.invalid".as_ref(),
                "commit".as_ref(),
                "--quiet".as_ref(),
                "--allow-empty".as_ref(),
                "-m".as_ref(),
                "fixture".as_ref(),
            ]);
            git(&[
                "worktree".as_ref(),
                "add".as_ref(),
                "--quiet".as_ref(),
                "--detach".as_ref(),
                worker_dir.path().as_os_str(),
            ]);
        }
        let dir = if worktree { &worker_dir } else { &parent_dir };
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(parent_dir.path(), jobs.clone(), permissions.clone());
        let parent = parent();
        let host = BrowserWorkerHost {
            controller: BrowserController::new(
                parent_dir.path(),
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
            ),
            supervisor: SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "native_extensions::graph::coordinator_handler::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            },
        };
        let first = transport_with_browser(
            &parent,
            &manager,
            &permissions,
            dir.path(),
            false,
            Some(&host),
        );
        let second = transport_with_browser(
            &parent,
            &manager,
            &permissions,
            dir.path(),
            false,
            Some(&host),
        );
        let address: std::net::IpAddr = loopback_host.parse().unwrap();
        let port = std::net::TcpListener::bind((address, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let script = format!(
            r#"const http=require('node:http');const s=http.createServer((q,r)=>{{r.setHeader('content-type','text/html');if(q.url==='/set')r.setHeader('set-cookie','owner=first; Path=/');r.end(`<label>Name<input aria-label=Name></label><select aria-label=Choice><option value=a>A</option><option value=b>B</option></select><button onclick="this.textContent='Done'">Start</button><p>`+ (q.headers.cookie||'NO_COOKIE')+'</p>');}});s.listen({port},'{loopback_host}',()=>console.log('READY'));setTimeout(()=>s.close(),60000)"#
        );
        std::fs::write(dir.path().join("server.cjs"), script).unwrap();
        let args = json!({"executable":"node","argv":["server.cjs"],"ports":[port]});
        let started = first
            .client()
            .call("process_start", &args)
            .unwrap()
            .details
            .unwrap();
        let id = started["process"]["id"].as_u64().unwrap();
        assert_eq!(
            started["process"]["workspace"],
            json!(dir.path().canonicalize().unwrap())
        );
        let reused = second
            .client()
            .call("process_start", &args)
            .unwrap()
            .details
            .unwrap();
        assert_eq!(reused["process"]["id"], id);
        assert_eq!(reused["reused"], true);
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let output = first
                .client()
                .call("process_output", &json!({"id":id}))
                .unwrap();
            if output.content.contains("READY") {
                break;
            }
            assert!(
                Instant::now() < until,
                "server not ready: {}",
                output.content
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let call = |worker: &TaskCoordinatorTransport, tool: &str, args: Value| {
            worker
                .client()
                .call_with_timeout(tool, &args, None, Duration::from_secs(35))
                .unwrap()
        };
        let one = call(
            &first,
            "browser_open",
            json!({"process_id":id,"port":port,"host":loopback_host,"path":"/set"}),
        )
        .details
        .unwrap();
        let two = call(
            &second,
            "browser_open",
            json!({"process_id":id,"port":port,"host":loopback_host}),
        )
        .details
        .unwrap();
        let one_id = one["browser_id"].as_str().unwrap();
        let two_id = two["browser_id"].as_str().unwrap();
        assert_ne!(one_id, two_id);
        assert_eq!(host.controller.context_count(), 2);
        assert!(second
            .client()
            .call("browser_snapshot", &json!({"browser_id":one_id}))
            .is_err());
        let snapshot = call(&second, "browser_snapshot", json!({"browser_id":two_id}));
        assert!(snapshot.content.contains("NO_COOKIE"));
        assert!(!snapshot.content.contains("owner=first"));
        for (tool, args) in [
            (
                "browser_type",
                json!({"browser_id":two_id,"selector":{"kind":"label","value":"Name"},"text":"Ada"}),
            ),
            (
                "browser_select",
                json!({"browser_id":two_id,"selector":{"kind":"label","value":"Choice"},"value":"b"}),
            ),
            (
                "browser_click",
                json!({"browser_id":two_id,"selector":{"kind":"role","role":"button","name":"Start"}}),
            ),
        ] {
            assert!(!call(&second, tool, args).is_error);
        }
        assert!(
            call(&second, "browser_snapshot", json!({"browser_id":two_id}))
                .content
                .contains(">Done</button>")
        );
        let mut screenshot = None;
        for tool in [
            "browser_console",
            "browser_network",
            "browser_accessibility",
            "browser_screenshot",
        ] {
            let result = call(&second, tool, json!({"browser_id":two_id}));
            assert!(!result.is_error);
            if tool == "browser_screenshot" {
                screenshot = Some(result.details.unwrap()["result"].clone());
            }
        }
        let screenshot = screenshot.unwrap();
        let artifact_request =
            json!({"browser_id":two_id,"artifact":screenshot["artifact"],"offset":0,"limit":64});
        let parent_context = davinci_agent::ToolContext {
            processes: Some(manager.clone()),
            ..Default::default()
        };
        let read_artifact = || {
            host.controller
                .retrieve_artifact(parent_dir.path(), &artifact_request, &parent_context)
        };
        let bytes = read_artifact().unwrap();
        assert_eq!(bytes["sha256"], screenshot["sha256"]);
        assert!(screenshot["action_sequence"].as_u64().unwrap() > 0);
        assert_eq!(bytes["action_sequence"], screenshot["action_sequence"]);
        assert_eq!(bytes["browser_id"], artifact_request["browser_id"]);
        assert_eq!(bytes["verification"], "observations_only");
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(bytes["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(png.len(), 64);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let abort = Arc::new(AtomicBool::new(false));
        let cancelled = transport_with_browser_abort(
            &parent,
            &manager,
            &permissions,
            dir.path(),
            false,
            Some(&host),
            abort.clone(),
        );
        let reused = call(&cancelled, "process_start", args.clone());
        assert_eq!(reused.details.unwrap()["process"]["id"], id);
        let opened = call(
            &cancelled,
            "browser_open",
            json!({"process_id":id,"port":port,"host":loopback_host}),
        );
        let cancelled_id = opened.details.unwrap()["browser_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(host.controller.context_count(), 3);
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                abort.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            // Await the parent response so early client-side abort cannot mask
            // a handler that continues running the browser action.
            let result = cancelled.client().call_with_timeout("browser_click",
                &json!({"browser_id":cancelled_id,"selector":{"kind":"role","role":"button","name":"Missing"}}),
                None, Duration::from_secs(35));
            assert!(result.is_err(), "cancelled parent action must not succeed");
        });
        drop(cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(host.controller.context_count(), 2);
        assert!(
            call(&second, "browser_snapshot", json!({"browser_id":two_id}))
                .content
                .contains(">Done</button>")
        );
        permissions
            .lock()
            .unwrap()
            .deny
            .push(PermissionRule::bare("browser_snapshot"));
        assert!(second
            .client()
            .call("browser_snapshot", &json!({"browser_id":two_id}))
            .is_err());
        drop(first);
        assert_eq!(
            host.controller.context_count(),
            1,
            "worker teardown must close its context even with revoked permission"
        );
        permissions.lock().unwrap().deny.clear();
        assert!(
            call(&second, "browser_snapshot", json!({"browser_id":two_id}))
                .content
                .contains(">Done</button>")
        );
        assert!(!call(&second, "browser_close", json!({"browser_id":two_id})).is_error);
        drop(second);
        assert_eq!(host.controller.context_count(), 0);
        assert_eq!(read_artifact().unwrap()["sha256"], screenshot["sha256"]);
        permissions
            .lock()
            .unwrap()
            .deny
            .push(PermissionRule::bare("browser_screenshot"));
        assert!(
            read_artifact().is_err(),
            "parent retrieval must recheck current permission"
        );
        permissions.lock().unwrap().deny.clear();
        let until = Instant::now() + Duration::from_secs(5);
        while jobs.lock().unwrap().get(id as u32).unwrap().status() == JobStatus::Running {
            assert!(Instant::now() < until, "worker process not released");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(read_artifact().unwrap()["sha256"], screenshot["sha256"]);
        manager.shutdown();
        assert!(
            read_artifact().is_err(),
            "session shutdown must invalidate parent retrieval"
        );
    }

    #[test]
    fn graph_managed_process_transport_reuses_and_releases_last_worker() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("server.cjs"),
            "process.stdout.write('READY\\n');setTimeout(()=>process.exit(0),20000)",
        )
        .unwrap();
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(dir.path(), jobs.clone(), permissions.clone());
        let parent = parent();
        let first = transport(&parent, &manager, &permissions, dir.path(), false);
        let second = transport(&parent, &manager, &permissions, dir.path(), false);
        let args = json!({"executable":"node","argv":["server.cjs"]});
        let one = first
            .client()
            .call("process_start", &args)
            .unwrap()
            .details
            .unwrap();
        let two = second
            .client()
            .call("process_start", &args)
            .unwrap()
            .details
            .unwrap();
        let id = one["process"]["id"].as_u64().unwrap() as u32;
        assert_eq!(two["process"]["id"], id);
        assert_eq!(two["reused"], true);
        assert_eq!(two["process"]["active_leases"], 2);
        assert!(
            manager
                .execute(dir.path(), "process_status", &json!({"id":id}), None, None)
                .is_err(),
            "unleased parent must not read worker process"
        );
        drop(first);
        let status = second
            .client()
            .call("process_status", &json!({"id":id}))
            .unwrap()
            .details
            .unwrap();
        assert_eq!(status["active_leases"], 1);
        assert_eq!(status["state"], "running");
        permissions
            .lock()
            .unwrap()
            .deny
            .push(PermissionRule::bare("process_status"));
        assert!(second
            .client()
            .call("process_status", &json!({"id":id}))
            .is_err());
        permissions.lock().unwrap().deny.clear();
        assert!(second
            .client()
            .call("process_write", &json!({"id":id,"text":"arbitrary"}))
            .is_err());
        assert!(second
            .client()
            .call(
                "process_start",
                &json!({"executable":"git","argv":["push"]})
            )
            .is_err());
        drop(second);
        let until = Instant::now() + Duration::from_secs(5);
        while jobs.lock().unwrap().get(id).unwrap().status() == JobStatus::Running {
            assert!(
                Instant::now() < until,
                "last worker did not release process"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn graph_managed_process_parent_rejects_contract_and_permission_mode() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(dir.path(), jobs, permissions.clone());
        let parent = parent();
        let worker = transport(&parent, &manager, &permissions, dir.path(), true);
        let args = json!({"executable":"node","argv":["--version"]});
        assert!(worker
            .client()
            .call("process_start", &args)
            .unwrap_err()
            .to_string()
            .contains("execution_contract_unenforceable"));
        drop(worker);
        permissions.lock().unwrap().mode = PermissionMode::ReadOnly;
        let worker = transport(&parent, &manager, &permissions, dir.path(), false);
        assert!(worker.client().call("process_start", &args).is_err());
        permissions.lock().unwrap().mode = PermissionMode::Ask;
        assert!(worker.client().call("process_start", &args).is_err());
    }

    #[test]
    fn graph_managed_processes_require_parent_and_remain_deferred() {
        for tool in [
            "process_start",
            "process_status",
            "process_output",
            "process_stop",
            "process_list",
        ] {
            assert!(roles::requires_task_coordinator(tool), "{tool}");
            let authorized = roles::role_tools(Role::Writer);
            assert!(authorized.contains(&tool.to_string()), "{tool}");
            assert!(
                !roles::initial_worker_tools(Role::Writer, &authorized).contains(&tool.to_string())
            );
            assert!(!roles::role_tools(Role::Researcher).contains(&tool.to_string()));
        }
        assert!(!roles::role_tools(Role::Writer).contains(&"process_write".to_string()));
    }

    #[test]
    fn graph_managed_process_composition_preserves_parent_output_retrieval() {
        use crate::native_extensions::{OutputStore, TokenGovernor, TokenGovernorConfig};
        let dir = tempfile::tempdir().unwrap();
        let mut governor = TokenGovernor::with_store(
            "process-recovery",
            TokenGovernorConfig::default(),
            OutputStore::new(dir.path().join("outputs")),
        );
        let exact = "fixture output line\n".repeat(1000);
        let processed = governor.after_tool(
            "process_output",
            &json!({"id":1}),
            ToolResult {
                content: exact,
                is_error: false,
                details: None,
            },
        );
        let args = json!({"id":processed.details.unwrap()["tokenGovernor"]["outputId"], "startLine":10,"endLine":20});
        let language = LanguageIntelligence::new(dir.path(), Default::default());
        language.set_governor(governor.clone());
        let handler = ParentTools {
            language: Some(language),
            processes: None,
            browser: None,
            cwd: dir.path().into(),
            contracted: false,
            abort: Arc::new(AtomicBool::new(false)),
        };
        assert!(handler.handles("retrieve_output"));
        let direct = governor.retrieve(&args).unwrap();
        let composed = handler.execute("retrieve_output", &args).unwrap();
        assert_eq!(composed.content, direct.content);
        assert!(composed.content.contains("fixture output line"));
    }

    #[test]
    fn graph_managed_process_controller_wires_worker_and_releases_lease() {
        use super::super::{
            controller::{run_saved_graph, RunOptions},
            definitions::*,
            topology::*,
            types::*,
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("server.cjs"),
            "console.log('READY');setTimeout(()=>process.exit(0),20000)",
        )
        .unwrap();
        let jobs = Arc::new(Mutex::new(JobBook::default()));
        let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let manager = manager(dir.path(), jobs.clone(), permissions.clone());
        let observed = Arc::new(Mutex::new(None));
        let saved = observed.clone();
        let deps = ControllerDeps {
            runner: Arc::new(move |spec, _, _| {
                assert_eq!(spec.role, Role::Writer);
                assert!(spec.tools.contains(&"process_start".into()));
                let client = spec
                    .coordinator_client
                    .as_ref()
                    .expect("controller must bind the parent channel");
                let result = client
                    .call(
                        "process_start",
                        &json!({"executable":"node","argv":["server.cjs"]}),
                    )
                    .unwrap()
                    .details
                    .unwrap();
                assert_eq!(result["process"]["origin"]["graph_node"], spec.task_id);
                assert_eq!(
                    result["process"]["origin"]["agent_id"],
                    json!(spec.runtime_agent_id)
                );
                *saved.lock().unwrap() = result["process"]["id"].as_u64().map(|id| id as u32);
                WorkerResult {
                    ok: true,
                    artifact: Some(Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec![],
                        summary: "Started an authorized fixture server".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))),
                    ..Default::default()
                }
            }),
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: super::super::config::GraphConfig::default(),
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: Some(manager),
            browser: None,
            runtime: Some(parent()),
            permissions: Some(permissions),
            task_contract: None,
        };
        let saved_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "process-fixture".into(),
            description: "process fixture".into(),
            graph: SavedGraphTopology {
                graph_id: "process-fixture".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![NodeDefinition {
                    id: "server".into(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    required: true,
                    allows_mutation: true,
                }],
                edges: vec![],
            },
            bindings: vec![SavedStageBinding {
                node_id: "server".into(),
                stage: "implement".into(),
                input_artifacts: vec![],
            }],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: Default::default(),
            parameters: vec![],
        };
        let run = run_saved_graph(
            RunOptions {
                goal: "process fixture".into(),
                cwd: dir.path().into(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: Default::default(),
                resume_run: None,
            },
            deps,
            saved_def,
        );
        assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
        let id = observed.lock().unwrap().expect("writer must execute");
        let until = Instant::now() + Duration::from_secs(5);
        while jobs.lock().unwrap().get(id).unwrap().status().is_running() {
            assert!(
                Instant::now() < until,
                "completed Graph worker retained process lease"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
