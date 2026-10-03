use davinci_agent::{
    runtime::{worktree::WorktreeManager, AgentId, RunId},
    Agent, PermissionMode,
};
use davinci_coding_agent::design::handoff::validate_candidate;
use davinci_coding_agent::design::{
    admission::*, events::*, handoff::*, records::*, store::*, types::*,
};
use davinci_coding_agent::{
    design::handoff_verification::*, native_extensions::browser::BrowserConfig,
};
use davinci_session::{custom_entry, JsonlSession};
use std::{collections::BTreeMap, path::Path, process::Command};

fn target_check_fixture() -> (
    tempfile::TempDir,
    Agent,
    DesignStore,
    PreparedHandoff,
    VerifyHandoff,
) {
    let (dir, mut agent, store, mut request) = fixture(true);
    let root = agent.cwd.clone();
    std::fs::write(root.join("verify.test.cjs"), "const {test}=require('node:test'); const assert=require('node:assert/strict'); test('target fixture',()=>assert.equal(1+1,2));\n").unwrap();
    git(&root, &["add", "verify.test.cjs"]);
    git(
        &root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "target check fixture",
        ],
    );
    request.verification = vec![
        "node --check verify.test.cjs".into(),
        "node --test --test-reporter=tap verify.test.cjs".into(),
    ];
    agent.tool_context.foreground_supervisor =
        Some(davinci_agent::jobs::supervisor::SupervisorCommand {
            executable: env!("CARGO_BIN_EXE_davinci").into(),
            argv: vec!["--internal-process-supervisor".into()],
        });
    let prepared = prepare_handoff(&store, &mut agent, &root, request).unwrap();
    let check = VerifyHandoff {
        proposal_id: prepared.proposal.request.operation_id,
        proposal_hash: prepared.proposal_hash.clone(),
        build_command: prepared.proposal.request.verification[0].clone(),
        test_command: prepared.proposal.request.verification[1].clone(),
        timeout_ms: 15_000,
        browser: None,
        operation_id: OperationId::new(),
    };
    (dir, agent, store, prepared, check)
}

#[test]
fn target_checks_collect_actual_discovery_without_certifying_mock_or_rsc_coverage() {
    let (_dir, mut agent, store, prepared, check) = target_check_fixture();
    let root = agent.cwd.clone();
    assert!(verify_handoff(
        &store,
        &mut agent,
        &root,
        check.clone(),
        BrowserConfig::default(),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("apply this exact proposal"));
    approve_handoff(
        &store,
        &mut agent,
        &root,
        ApproveHandoff {
            proposal_id: check.proposal_id,
            proposal_hash: prepared.proposal_hash,
            operation_id: OperationId::new(),
        },
    )
    .unwrap();
    let mut unreviewed = check.clone();
    unreviewed.build_command = "npm install".into();
    assert!(verify_handoff(
        &store,
        &mut agent,
        &root,
        unreviewed,
        BrowserConfig::default(),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("verbatim"));
    let report = verify_handoff(
        &store,
        &mut agent,
        &root,
        check.clone(),
        BrowserConfig::default(),
        None,
    )
    .unwrap();
    assert_eq!(report["state"], "complete", "{report}");
    assert_eq!(report["commands"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(
        report["commands"][1]["receipt"]["assertion_counts"]["passed"], 1,
        "{report}"
    );
    assert_eq!(report["checks_passed"], false);
    assert_eq!(report["implementation"], "pending_verification");
    assert!(report["incomplete_coverage"][0]
        .as_str()
        .unwrap()
        .contains("RSC/client"));
    assert!(report["incomplete_coverage"][0]
        .as_str()
        .unwrap()
        .contains("mock removal"));
    assert_eq!(
        verify_handoff(
            &store,
            &mut agent,
            &root,
            check.clone(),
            BrowserConfig::default(),
            None
        )
        .unwrap(),
        report
    );
    let test_source = std::fs::read(root.join("verify.test.cjs")).unwrap();
    std::fs::write(root.join("verify.test.cjs"), "changed test").unwrap();
    assert!(verify_handoff(
        &store,
        &mut agent,
        &root,
        check.clone(),
        BrowserConfig::default(),
        None
    )
    .is_err());
    std::fs::write(root.join("verify.test.cjs"), test_source).unwrap();
    std::fs::write(root.join("app.tsx"), "concurrent edit").unwrap();
    assert!(verify_handoff(
        &store,
        &mut agent,
        &root,
        check,
        BrowserConfig::default(),
        None
    )
    .is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("app.tsx")).unwrap(),
        "concurrent edit"
    );
}

#[test]
fn host_commands_obey_hook_veto_and_cancellation() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let (_dir, mut agent, _store, _prepared, _) = target_check_fixture();
    let root = agent.cwd.clone();
    let command = "node --check verify.test.cjs";
    agent.pre_tool = Some(davinci_agent::PreToolHook(Arc::new(|_, _| {
        Some("fixture veto".into())
    })));
    assert!(agent
        .execute_host_verification_command(&root, command, 15_000, Arc::new(AtomicBool::new(false)))
        .is_err());
    agent.pre_tool = None;
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(agent
        .execute_host_verification_command(&root, command, 15_000, cancelled.clone())
        .is_err());
    cancelled.store(false, Ordering::Release);
    let ready = root.join("cancel-ready");
    let began = std::time::Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            cancelled.store(true, Ordering::Release);
        });
        let result = agent.execute_host_verification_command(
            &root,
            "node -e 'require(\"node:fs\").writeFileSync(\"cancel-ready\",\"ready\");setTimeout(()=>{},30000)'",
            15_000,
            cancelled.clone(),
        );
        assert!(ready.exists(), "child must start before cancellation");
        assert!(result.unwrap_err().contains("verification cancelled"));
    });
    assert!(began.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
#[ignore = "requires an explicitly installed trusted browser package and Node executable"]
fn native_target_flow_retains_screenshot_after_browser_owner_closes() {
    use davinci_agent::process_manager::ProcessManager;
    use serde_json::json;
    use std::time::{Duration, Instant};
    let (_dir, mut agent, store, prepared, mut check) = target_check_fixture();
    let root = agent.cwd.clone();
    let config = BrowserConfig {
        enabled: true,
        node: std::env::var_os("DAVINCI_DESIGN_NODE")
            .expect("explicit Node")
            .into(),
        package: Path::new(&std::env::var_os("DAVINCI_DESIGN_RUNTIME").expect("explicit runtime"))
            .join("node_modules/playwright-core"),
        version: "1.62.1".into(),
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let manager = ProcessManager::new(
        &root,
        agent.tool_context.jobs.clone(),
        agent.permissions.clone(),
        agent.tool_context.foreground_supervisor.clone().unwrap(),
    )
    .unwrap();
    agent.tool_context.processes = Some(manager.clone());
    let script = format!(
        r#"const http=require('node:http'),fs=require('node:fs');http.createServer((q,r)=>{{r.setHeader('Content-Type','text/html');r.end('<!doctype html><title>Target fixture</title>'+fs.readFileSync('app.tsx','utf8').match(/<h1>.*<\/h1>/)[0]+'<button onclick="this.textContent=\'Saved locally\'">Continue</button>')}}).listen({port},'127.0.0.1')"#
    );
    let started = manager
        .execute(
            &root,
            "process_start",
            &json!({"executable":config.node,
        "argv":["-e",script],"ports":[port]}),
            None,
            None,
        )
        .unwrap();
    let id = started.details.unwrap()["process"]["id"].as_u64().unwrap() as u32;
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_err() {
        assert!(Instant::now() < deadline, "managed target never listened");
        std::thread::sleep(Duration::from_millis(20));
    }
    approve_handoff(
        &store,
        &mut agent,
        &root,
        ApproveHandoff {
            proposal_id: check.proposal_id,
            proposal_hash: prepared.proposal_hash,
            operation_id: OperationId::new(),
        },
    )
    .unwrap();
    check.browser = Some(TargetBrowserCheck {
        process_id: id,
        port: port.into(),
        path: "/".into(),
        actions: vec![TargetBrowserAction {
            tool: "browser_click".into(),
            args: json!({"selector":{"kind":"role","role":"button","name":"Continue"}}),
        }],
        dom_contains: "Saved locally".into(),
        accessibility_contains: "Saved locally".into(),
    });
    let report = verify_handoff(&store, &mut agent, &root, check, config, None).unwrap();
    manager.shutdown();
    assert_eq!(report["checks_passed"], true, "{report}");
    assert_eq!(report["implementation"], "pending_verification");
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let screenshot: ArtifactRef =
        serde_json::from_value(report["browser"]["screenshot"].clone()).unwrap();
    let bytes = davinci_agent::runtime::evidence_store::VerificationEvidenceStore::new(
        store.blob_directory(&ctx),
    )
    .get_artifact(&(&screenshot).into())
    .unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    if let Some(directory) = std::env::var_os("DAVINCI_DESIGN_EVIDENCE") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("target-handoff.png"), bytes).unwrap();
        std::fs::write(
            directory.join("target-handoff.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok() {
        assert!(
            Instant::now() < deadline,
            "target process tree survived shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn interrupted_target_checks_require_an_explicit_new_operation() {
    let (_dir, mut agent, store, prepared, check) = target_check_fixture();
    let root = agent.cwd.clone();
    approve_handoff(
        &store,
        &mut agent,
        &root,
        ApproveHandoff {
            proposal_id: check.proposal_id,
            proposal_hash: prepared.proposal_hash,
            operation_id: OperationId::new(),
        },
    )
    .unwrap();
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let reference = davinci_agent::runtime::evidence_store::VerificationEvidenceStore::new(
        store.blob_directory(&ctx),
    )
    .store_artifact("application/json", br#"{"state":"running"}"#)
    .unwrap();
    // Reproduce the durable checkpoint left by a process that never finalized.
    let event = DesignEvent {
        schema_version: SchemaVersion,
        owner_session: ctx.session_id().into(),
        workspace: ctx.workspace_id().into(),
        operation_id: check.operation_id,
        payload_digest: digest(&check).unwrap(),
        change: DesignChange::HandoffChecked {
            artifact_id: prepared.proposal.request.artifact_id,
            run_id: check.operation_id,
            report: reference.into(),
        },
    };
    agent
        .session
        .as_mut()
        .unwrap()
        .append_entry(custom_entry(
            &check.operation_id.to_string(),
            CUSTOM_TYPE,
            serde_json::to_value(event).unwrap(),
        ))
        .unwrap();
    let before = agent.session.as_ref().unwrap().entries.len();
    let error = verify_handoff(
        &store,
        &mut agent,
        &root,
        check,
        BrowserConfig::default(),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("outcome is unknown"), "{error}");
    assert_eq!(agent.session.as_ref().unwrap().entries.len(), before);
}

#[test]
fn handoff_requires_concrete_validation_and_rejects_unbounded_or_misleading_patches() {
    let patch = "*** Begin Patch\n*** Add File: src/design.tsx\n+export const Design = () => <h1>Design</h1>;\n*** End Patch";
    assert!(validate_candidate(
        patch,
        &["Build the target and exercise its primary action".into()]
    )
    .is_ok());
    assert!(validate_candidate(patch, &[]).is_err());
    assert!(validate_candidate(&"x".repeat(256 * 1024 + 1), &["Build".into()]).is_err());
    assert!(validate_candidate(
        "*** Begin Patch\n*** Add File: ../escape\n+x\n*** End Patch",
        &["Build".into()]
    )
    .is_err());
    assert!(validate_candidate(
        "*** Begin Patch\n*** Add File: package.json\n+{}\n*** End Patch",
        &["Build".into()]
    )
    .is_err());
}

fn git(root: &Path, args: &[&str]) {
    let exe = if cfg!(windows) {
        "C:/Program Files/Git/cmd/git.exe"
    } else {
        "git"
    };
    let result = Command::new(exe)
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn fixture(accept: bool) -> (tempfile::TempDir, Agent, DesignStore, HandoffRequest) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Design test"]);
    git(&repo, &["config", "user.email", "design@example.invalid"]);
    std::fs::write(
        repo.join("app.tsx"),
        "export const App = () => <h1>Before</h1>;\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("package.json"),
        "{\"dependencies\":{\"react\":\"19.0.0\"}}\n",
    )
    .unwrap();
    git(&repo, &["add", "app.tsx", "package.json"]);
    git(
        &repo,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
    );
    let lease = WorktreeManager::new(&repo, dir.path().join("worktrees"))
        .create_lease(RunId::new(), AgentId::new(), None)
        .unwrap();
    let root = lease.path.canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.cwd = root.clone();
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(
            &dir.path().join("sessions"),
            &root.to_string_lossy(),
            None,
        )
        .unwrap(),
    );
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let store = DesignStore::new(dir.path().join("blobs"));
    let session = agent.session.as_mut().unwrap();
    let artifact = store
        .create(
            &ctx,
            session,
            CreateDesign {
                title: "Accepted test".into(),
                brief: "Use a readable heading".into(),
                kind: DesignKind::Product,
                variants: 1,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let sources = store
        .store_sources(
            &ctx,
            session,
            &BTreeMap::from([("index.html".into(), "<h1>After</h1>".into())]),
            vec!["index.html".into()],
        )
        .unwrap();
    let revision = store
        .commit_revision(
            &ctx,
            session,
            RevisionWrite {
                artifact_id: artifact.id,
                expected_revision: RevisionId(0),
                operation_id: OperationId::new(),
                sources,
                variants: vec![Variant {
                    id: VariantId::new(),
                    title: "A".into(),
                    artboards: vec![Artboard {
                        id: ArtboardId::new(),
                        title: "Home".into(),
                        entry_point: "index.html".into(),
                    }],
                }],
                bindings: vec![],
                assets: vec![],
                profile_refs: vec![],
                system_snapshot: None,
            },
        )
        .unwrap();
    if accept {
        // Synthetic acceptance only tests the handoff gate, not visual quality.
        let op = OperationId::new();
        let event = DesignEvent {
            schema_version: SchemaVersion,
            owner_session: ctx.session_id().into(),
            workspace: ctx.workspace_id().into(),
            operation_id: op,
            payload_digest: "a".repeat(64),
            change: DesignChange::Accepted {
                revision: RevisionReference {
                    artifact_id: artifact.id,
                    revision: revision.revision,
                    source_hash: revision.source_hash,
                },
                evidence_hash: "b".repeat(64),
                acknowledged_incomplete: vec!["synthetic test acceptance".into()],
            },
        };
        session
            .append_entry(custom_entry(
                &op.to_string(),
                CUSTOM_TYPE,
                serde_json::to_value(event).unwrap(),
            ))
            .unwrap();
    }
    (dir,agent,store,HandoffRequest {artifact_id:artifact.id,revision:RevisionId(1),patch:"*** Begin Patch\n*** Update File: app.tsx\n@@\n-export const App = () => <h1>Before</h1>;\n+export const App = () => <h1>After</h1>;\n*** End Patch".into(),verification:vec!["Run target build and tests, then inspect the actual primary browser flow".into()],operation_id:OperationId::new()})
}
#[test]
fn accepted_patch_uses_native_plan_and_transaction_without_claiming_verification() {
    let (_dir, mut agent, store, request) = fixture(true);
    let root = agent.cwd.clone();
    std::fs::write(root.join("unrelated.txt"), "preserved").unwrap();
    let prepared = prepare_handoff(&store, &mut agent, &root, request).unwrap();
    assert!(std::fs::read_to_string(root.join("app.tsx"))
        .unwrap()
        .contains("Before"));
    let approved = ApproveHandoff {
        proposal_id: prepared.proposal.request.operation_id,
        proposal_hash: prepared.proposal_hash,
        operation_id: OperationId::new(),
    };
    let result = approve_handoff(&store, &mut agent, &root, approved.clone()).unwrap();
    assert_eq!(result["implementation"], "pending_verification");
    assert_eq!(result["transaction"]["state"], "applied");
    assert!(std::fs::read_to_string(root.join("app.tsx"))
        .unwrap()
        .contains("After"));
    assert_eq!(
        std::fs::read_to_string(root.join("unrelated.txt")).unwrap(),
        "preserved"
    );
    approve_handoff(&store, &mut agent, &root, approved).unwrap();
}
#[test]
fn unaccepted_dirty_stale_or_plan_mode_proposals_cannot_modify_application() {
    let (_dir, mut agent, store, request) = fixture(false);
    let root = agent.cwd.clone();
    assert!(prepare_handoff(&store, &mut agent, &root, request)
        .unwrap_err()
        .to_string()
        .contains("accepted"));
    for cause in ["dirty", "dependency", "plan", "revoked", "revision", "hash"] {
        let (_dir, mut agent, store, request) = fixture(true);
        let root = agent.cwd.clone();
        if cause == "dirty" {
            std::fs::write(root.join("app.tsx"), "user edit").unwrap();
            assert!(prepare_handoff(&store, &mut agent, &root, request).is_err());
            assert_eq!(
                std::fs::read_to_string(root.join("app.tsx")).unwrap(),
                "user edit"
            );
            continue;
        }
        let prepared = prepare_handoff(&store, &mut agent, &root, request).unwrap();
        let mut approved = ApproveHandoff {
            proposal_id: prepared.proposal.request.operation_id,
            proposal_hash: prepared.proposal_hash,
            operation_id: OperationId::new(),
        };
        match cause {
            "dependency" => std::fs::write(root.join("package.json"), "{}").unwrap(),
            "plan" => agent.set_plan_mode(true),
            "revoked" => agent.set_permission_mode(PermissionMode::ReadOnly),
            "revision" => {
                let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
                let session = agent.session.as_mut().unwrap();
                store
                    .restore(
                        &ctx,
                        session,
                        prepared.proposal.request.artifact_id,
                        RevisionId(1),
                        RevisionId(1),
                        OperationId::new(),
                    )
                    .unwrap();
            }
            "hash" => approved.proposal_hash = "c".repeat(64),
            _ => unreachable!(),
        }
        assert!(
            approve_handoff(&store, &mut agent, &root, approved).is_err(),
            "{cause}"
        );
        assert!(std::fs::read_to_string(root.join("app.tsx"))
            .unwrap()
            .contains("Before"));
    }
}

use davinci_agent::{
    runtime::capacity::{BudgetLimits, RootBudget},
    CompleteOutput,
};
use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
use davinci_coding_agent::design::{error::*, handoff_draft::*, model::*};

struct DraftModel {
    patch: String,
    calls: usize,
    mutate: Option<std::path::PathBuf>,
}
impl DesignModel for DraftModel {
    fn validate(&self, _: &Agent) -> DesignResult<()> {
        Ok(())
    }
    fn supports_images(&self) -> bool {
        false
    }
    fn complete(
        &mut self,
        owner: &Agent,
        _: &DesignModelRequest,
    ) -> Result<CompleteOutput, String> {
        self.calls += 1;
        let attempt = format!("draft-{}", self.calls);
        owner
            .root_budget()
            .unwrap()
            .reserve("fixture", &attempt, 100, None)?;
        owner
            .root_budget()
            .unwrap()
            .reconcile(&attempt, Some(10), None)?;
        if let Some(path) = &self.mutate {
            std::fs::write(path, "concurrent edit").unwrap();
        }
        Ok(CompleteOutput { message:AssistantMessage {id:attempt,role:"assistant".into(),content:vec![ContentBlock::Text {text:serde_json::json!({"patch":self.patch,"verification":["Build target; exercise the primary browser action"]}).to_string()}],model:owner.model_id.clone(),usage:None,stop_reason:Some(StopReason::Stop),error_message:None,extra:Default::default()},stream_events:None,native_responses_resume:None,streamed_live:false })
    }
}
#[test]
fn draft_is_accounted_resumable_and_supports_new_ui_files_without_applying() {
    for new_file in [false, true] {
        let (dir, mut agent, store, request) = fixture(true);
        let root = agent.cwd.clone();
        agent
            .bind_root_budget(
                RootBudget::open(
                    dir.path().join("ledger.json"),
                    "draft-root",
                    BudgetLimits {
                        max_requests: 10,
                        max_output_tokens: Some(1000),
                        max_cost_microusd: None,
                        codex_subscription: None,
                        deadline_unix_ms: u64::MAX,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let target = if new_file { "new/page.html" } else { "app.tsx" };
        let mut model = DraftModel {
            patch: if new_file {
                "*** Begin Patch\n*** Add File: new/page.html\n+<h1>After</h1>\n*** End Patch"
                    .into()
            } else {
                request.patch
            },
            calls: 0,
            mutate: None,
        };
        let draft = DraftHandoff {
            artifact_id: request.artifact_id,
            revision: request.revision,
            target_files: vec![target.into()],
            operation_id: OperationId::new(),
        };
        let first =
            draft_handoff(&store, &mut agent, &root, draft.clone(), &mut model, None).unwrap();
        let second = draft_handoff(&store, &mut agent, &root, draft, &mut model, None).unwrap();
        assert_eq!(first.proposal_hash, second.proposal_hash);
        assert_eq!(model.calls, 1);
        assert_eq!(agent.root_budget().unwrap().snapshot().unwrap().requests, 1);
        assert!(std::fs::read_to_string(root.join("app.tsx"))
            .unwrap()
            .contains("Before"));
        assert!(!root.join("new/page.html").exists());
    }
}
#[test]
fn drafting_cannot_ignore_changes_to_selected_html_outside_static_sync() {
    let (dir, mut agent, store, request) = fixture(true);
    let root = agent.cwd.clone();
    std::fs::write(root.join("page.html"), "<h1>Before</h1>\n").unwrap();
    git(&root, &["add", "page.html"]);
    git(
        &root,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "html"],
    );
    agent
        .bind_root_budget(
            RootBudget::open(
                dir.path().join("ledger.json"),
                "draft-root",
                BudgetLimits {
                    max_requests: 10,
                    max_output_tokens: Some(1000),
                    max_cost_microusd: None,
                    codex_subscription: None,
                    deadline_unix_ms: u64::MAX,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let mut model=DraftModel {patch:"*** Begin Patch\n*** Update File: page.html\n@@\n-<h1>Before</h1>\n+<h1>After</h1>\n*** End Patch".into(),calls:0,mutate:Some(root.join("page.html"))};
    let error = draft_handoff(
        &store,
        &mut agent,
        &root,
        DraftHandoff {
            artifact_id: request.artifact_id,
            revision: request.revision,
            target_files: vec!["page.html".into()],
            operation_id: OperationId::new(),
        },
        &mut model,
        None,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("selected target changed"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("page.html")).unwrap(),
        "concurrent edit"
    );
}
