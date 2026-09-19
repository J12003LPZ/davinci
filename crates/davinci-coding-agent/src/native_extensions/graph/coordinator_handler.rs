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
    let processes =
        deps.processes
            .as_ref()
            .zip(deps.permissions.as_ref())
            .map(|(manager, permissions)| {
                manager
                    .child_lease(
                        permissions.clone(),
                        roles::shell_profile(roles::role_bash_policy(spec.role)),
                    )
                    .with_provenance(davinci_agent::jobs::managed::Provenance {
                        session_id: deps
                            .runtime
                            .as_ref()
                            .and_then(|runtime| runtime.session_id.clone()),
                        agent_id: spec.runtime_agent_id,
                        task_id: spec.task_contract.as_ref().map(|contract| contract.task_id),
                        graph_node: Some(spec.task_id.clone()),
                    })
            });
    (language.is_some() || processes.is_some()).then(|| {
        Arc::new(ParentTools {
            language,
            processes,
            cwd: spec.cwd.clone(),
            contracted: spec.task_contract.is_some(),
            abort,
        }) as Arc<dyn CoordinatorToolHandler>
    })
}

impl CoordinatorToolHandler for ParentTools {
    fn handles(&self, tool: &str) -> bool {
        self.processes.is_some() && davinci_agent::tools::is_managed_process_tool(tool)
            || self
                .language
                .as_ref()
                .is_some_and(|manager| manager.handles(tool))
    }

    fn execute(&self, tool: &str, args: &Value) -> Result<ToolResult, ToolError> {
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
        let abort = Arc::new(AtomicBool::new(false));
        TaskCoordinatorTransport::bind_with_handler(
            parent,
            child,
            permissions.clone(),
            vec![
                "process_start".into(),
                "process_status".into(),
                "process_output".into(),
                "process_write".into(),
            ],
            cwd.into(),
            abort.clone(),
            Some(Arc::new(ParentTools {
                language: None,
                processes: Some(manager.child_lease(
                    permissions.clone(),
                    roles::shell_profile(roles::role_bash_policy(Role::Writer)),
                )),
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
