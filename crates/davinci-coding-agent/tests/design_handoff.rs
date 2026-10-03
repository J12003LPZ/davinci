use davinci_agent::{
    runtime::{worktree::WorktreeManager, AgentId, RunId},
    Agent, PermissionMode,
};
use davinci_coding_agent::design::handoff::validate_candidate;
use davinci_coding_agent::design::{
    admission::*, events::*, handoff::*, records::*, store::*, types::*,
};
use davinci_session::{custom_entry, JsonlSession};
use std::{collections::BTreeMap, path::Path, process::Command};

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
