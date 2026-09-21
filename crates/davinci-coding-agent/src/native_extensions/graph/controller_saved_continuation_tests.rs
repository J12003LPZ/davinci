use super::super::definitions::*;
use super::super::topology::{EdgeCondition, EdgeDefinition, NodeDefinition};
use super::*;

fn saved_pipeline() -> SavedGraphDefinitionV1 {
    let stages = [
        (
            "survey",
            Role::Researcher,
            ArtifactKind::Evidence,
            "research",
        ),
        ("design", Role::Planner, ArtifactKind::Plan, "plan"),
        (
            "write",
            Role::Writer,
            ArtifactKind::PatchReport,
            "implement",
        ),
        (
            "check",
            Role::TestAnalyzer,
            ArtifactKind::Evidence,
            "verify",
        ),
        ("inspect", Role::Reviewer, ArtifactKind::Review, "review"),
    ];
    SavedGraphDefinitionV1 {
        schema_version: 1,
        name: "resume-fixture".into(),
        description: "Saved stage continuation fixture".into(),
        graph: SavedGraphTopology {
            graph_id: "resume-fixture".into(),
            version: 7,
            mode: GraphMode::Standard,
            nodes: stages
                .iter()
                .map(|(id, role, expect, _)| NodeDefinition {
                    id: (*id).into(),
                    role: *role,
                    expect: *expect,
                    required: true,
                    allows_mutation: *role == Role::Writer,
                })
                .collect(),
            edges: stages
                .windows(2)
                .map(|pair| EdgeDefinition {
                    from: pair[0].0.into(),
                    to: pair[1].0.into(),
                    condition: EdgeCondition::OnSuccess,
                })
                .collect(),
        },
        bindings: stages
            .iter()
            .enumerate()
            .map(|(index, (id, _, _, stage))| SavedStageBinding {
                node_id: (*id).into(),
                stage: (*stage).into(),
                input_artifacts: match index {
                    1 => vec!["survey".into()],
                    2 => vec!["design".into(), "survey".into()],
                    _ => vec![],
                },
            })
            .collect(),
        budgets: None,
        verification_policy: None,
        artifact_contract_versions: Default::default(),
        parameters: vec![],
    }
}

#[test]
fn saved_pipeline_repairs_verification_without_replaying_completed_workers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("owned.txt"), "before\n").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let controller = super::super::GraphController::new(dir.path().into());
    let mut previous = None;
    for fail in [true, false] {
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
            name: "fixture".into(),
            command: "fixture".into(),
            from_plan: false,
        }];
        deps.verify_exec =
            Arc::new(move |_, _, _, _| (if fail { 127 } else { 0 }, "fixture".into(), 1));
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if spec.role == Role::Writer {
                std::fs::write(spec.cwd.join("owned.txt"), "after\n").unwrap();
            }
            if spec.role == Role::Reviewer {
                assert!(
                    spec.briefing.contains("-before"),
                    "original baseline was lost"
                );
                assert!(spec.briefing.contains("+after"), "writer delta was lost");
            }
            super::super::worker::run_dry_worker(spec, abort, progress)
        });
        let options = RunOptions {
            goal: "saved continuation".into(),
            cwd: dir.path().into(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: previous.map(Box::new),
        };
        calls.lock().unwrap().clear();
        let run = run_saved_graph(options, deps, saved_pipeline());
        if fail {
            assert_eq!(run.phase, Phase::Blocked);
            assert_eq!(*calls.lock().unwrap(), ["survey", "design", "write"]);
        } else {
            assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
            assert_eq!(*calls.lock().unwrap(), ["inspect"]);
            assert_eq!(run.tasks.len(), 5);
            assert!(run
                .task("write")
                .unwrap()
                .mutation
                .as_ref()
                .unwrap()
                .diff()
                .contains("-before"));
        }
        previous = Some(super::super::store::load_run_checked(dir.path(), &run.run_id).unwrap());
    }
}

#[test]
fn saved_pipeline_reloads_every_stage_and_rechecks_changed_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("owned.txt"), "before\n").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let verifies = Arc::new(AtomicUsize::new(0));
    let controller = super::super::GraphController::new(dir.path().into());
    let mut previous = None;
    for stop_at in [
        Some("survey"),
        Some("design"),
        Some("write"),
        Some("check"),
        Some("inspect"),
        None,
    ] {
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
            name: "fixture".into(),
            command: "fixture".into(),
            from_plan: false,
        }];
        let checked = verifies.clone();
        deps.verify_exec = Arc::new(move |_, _, _, _| {
            checked.fetch_add(1, Ordering::SeqCst);
            (0, "passed".into(), 1)
        });
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if spec.role == Role::Planner || spec.role == Role::Writer {
                assert!(
                    spec.briefing.contains("UNIQUE_RESEARCH_FINDING"),
                    "saved binding lost its evidence artifact"
                );
            }
            if spec.role == Role::Writer {
                assert!(
                    spec.briefing.contains("UNIQUE_PLAN_STEP"),
                    "saved binding lost its plan artifact"
                );
                let path = spec.cwd.join("owned.txt");
                assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
                std::fs::write(path, "after\n").unwrap();
            }
            if spec.role == Role::Reviewer {
                assert!(spec.briefing.contains("-before"));
                assert!(spec.briefing.contains("+after"));
            }
            let mut result = super::super::worker::run_dry_worker(spec, abort, progress);
            match &mut result.artifact {
                Some(Artifact::Evidence(evidence)) => {
                    evidence.findings[0].claim = "UNIQUE_RESEARCH_FINDING".into()
                }
                Some(Artifact::Plan(plan)) => plan.steps[0].description = "UNIQUE_PLAN_STEP".into(),
                _ => {}
            }
            result
        });
        let abort = Arc::new(AtomicBool::new(false));
        let stop = abort.clone();
        deps.on_update = Arc::new(move |run, _| {
            if stop_at
                .and_then(|id| run.task(id))
                .is_some_and(|task| task.status == TaskStatus::Succeeded)
            {
                stop.store(true, Ordering::SeqCst);
            }
        });
        if stop_at.is_none() {
            std::fs::write(dir.path().join("extra.txt"), "changed after review").unwrap();
        }
        let run = run_saved_graph(
            RunOptions {
                goal: "saved continuation".into(),
                cwd: dir.path().into(),
                forced: None,
                dry_run: false,
                abort,
                resume_artifacts: HashMap::new(),
                resume_run: previous.map(Box::new),
            },
            deps,
            saved_pipeline(),
        );
        assert_eq!(
            run.phase,
            if stop_at.is_some() {
                Phase::Cancelled
            } else {
                Phase::Done
            },
            "{:?}",
            run.blocked_reason
        );
        let mut loaded = super::super::store::load_run_checked(dir.path(), &run.run_id).unwrap();
        if stop_at == Some("write") {
            // Model process death after the successful artifact checkpoint,
            // before the controller persists the writer's mutation companion.
            loaded
                .tasks
                .iter_mut()
                .find(|task| task.id == "write")
                .unwrap()
                .mutation = None;
            super::super::store::save_run(&mut loaded).unwrap();
            std::fs::remove_file(
                super::super::store::run_dir(dir.path(), &run.run_id)
                    .join("artifacts/write.mutation.json"),
            )
            .unwrap();
        }
        previous = Some(loaded);
    }
    assert_eq!(
        *calls.lock().unwrap(),
        ["survey", "design", "write", "inspect", "inspect"]
    );
    assert_eq!(verifies.load(Ordering::SeqCst), 2);
    let completed = previous.unwrap();
    assert_eq!(completed.tasks.len(), 5);
    assert!(completed
        .task("write")
        .unwrap()
        .mutation
        .as_ref()
        .expect("writer mutation must survive artifact/checkpoint interruption")
        .diff()
        .contains("-before"));
}

#[test]
fn saved_failure_branch_resumes_without_repeating_its_failed_parent() {
    let dir = tempfile::tempdir().unwrap();
    let controller = super::super::GraphController::new(dir.path().into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut definition = saved_pipeline();
    definition.graph.mode = GraphMode::Simple;
    definition.graph.nodes.truncate(1);
    definition.graph.nodes[0].required = false;
    definition.graph.nodes.push(NodeDefinition {
        id: "fallback".into(),
        role: Role::Researcher,
        expect: ArtifactKind::Evidence,
        required: true,
        allows_mutation: false,
    });
    definition.graph.edges = vec![EdgeDefinition {
        from: "survey".into(),
        to: "fallback".into(),
        condition: EdgeCondition::OnFailure,
    }];
    definition.bindings.truncate(1);
    definition.bindings.push(SavedStageBinding {
        node_id: "fallback".into(),
        stage: "research".into(),
        input_artifacts: vec![],
    });
    let mut previous = None;
    for fail_fallback in [true, false] {
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if spec.task_id == "survey" || fail_fallback {
                return WorkerResult {
                    failure_reason: Some("fixture unavailable".into()),
                    ..Default::default()
                };
            }
            super::super::worker::run_dry_worker(spec, abort, progress)
        });
        calls.lock().unwrap().clear();
        let run = run_saved_graph(
            RunOptions {
                goal: "saved fallback".into(),
                cwd: dir.path().into(),
                forced: None,
                dry_run: true,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: previous.map(Box::new),
            },
            deps,
            definition.clone(),
        );
        if fail_fallback {
            assert_eq!(run.phase, Phase::Blocked);
            assert!(calls.lock().unwrap().iter().any(|id| id == "fallback"));
        } else {
            assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
            assert_eq!(*calls.lock().unwrap(), ["fallback"]);
            assert_eq!(run.tasks.len(), 2);
        }
        previous = Some(super::super::store::load_run_checked(dir.path(), &run.run_id).unwrap());
    }
}

#[test]
fn saved_pipeline_rejects_changes_requested_and_unstable_verification_inputs() {
    for (change_during_verify, verdict) in [
        (false, Verdict::ChangesRequired),
        (true, Verdict::ChangesRequired),
        (false, Verdict::Approve),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let controller = super::super::GraphController::new(dir.path().into());
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
            name: "fixture".into(),
            command: "fixture".into(),
            from_plan: false,
        }];
        let root = dir.path().to_path_buf();
        deps.verify_exec = Arc::new(move |_, _, _, _| {
            if change_during_verify {
                std::fs::write(root.join("changed.txt"), "changed while checking").unwrap();
            }
            (0, "passed".into(), 1)
        });
        let reviews = Arc::new(AtomicUsize::new(0));
        let reviewed = reviews.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            let mut result = super::super::worker::run_dry_worker(spec, abort, progress);
            if let Some(Artifact::Review(review)) = &mut result.artifact {
                reviewed.fetch_add(1, Ordering::SeqCst);
                review.verdict = verdict;
                review.notes = "fixture requires repair".into();
                review.issues.push(super::super::types::ReviewIssue {
                    severity: super::super::types::Severity::Major,
                    file: None,
                    description: "fixture requires repair".into(),
                });
            }
            result
        });
        let run = run_saved_graph(
            RunOptions {
                goal: "reject unsafe completion".into(),
                cwd: dir.path().into(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
            saved_pipeline(),
        );
        assert_eq!(run.phase, Phase::Blocked, "{:?}", run.blocked_reason);
        if change_during_verify {
            assert_eq!(reviews.load(Ordering::SeqCst), 0);
            assert!(run
                .blocked_reason
                .as_deref()
                .unwrap()
                .contains("inputs changed"));
        } else {
            assert_eq!(reviews.load(Ordering::SeqCst), 1);
            assert_eq!(run.task("inspect").unwrap().status, TaskStatus::Failed);
            let mut interrupted = run;
            interrupted
                .tasks
                .iter_mut()
                .find(|task| task.id == "inspect")
                .unwrap()
                .status = TaskStatus::Succeeded;
            // The worker artifact was checkpointed, but the controller died
            // before interpreting its ChangesRequired verdict.
            let (mut resumed_deps, _) =
                controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
            resumed_deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
                name: "fixture".into(),
                command: "fixture".into(),
                from_plan: false,
            }];
            resumed_deps.verify_exec = Arc::new(|_, _, _, _| (0, "passed".into(), 1));
            resumed_deps.runner = Arc::new(|spec, abort, progress| {
                let mut result = super::super::worker::run_dry_worker(spec, abort, progress);
                if let Some(Artifact::Review(review)) = &mut result.artifact {
                    review.verdict = Verdict::ChangesRequired;
                    review.issues.push(super::super::types::ReviewIssue {
                        severity: super::super::types::Severity::Major,
                        file: None,
                        description: "fixture still requires repair".into(),
                    });
                }
                result
            });
            let resumed = run_graph(
                RunOptions {
                    goal: interrupted.goal.clone(),
                    cwd: dir.path().into(),
                    forced: None,
                    dry_run: false,
                    abort: Arc::new(AtomicBool::new(false)),
                    resume_artifacts: HashMap::new(),
                    resume_run: Some(Box::new(interrupted)),
                },
                resumed_deps,
            );
            assert_eq!(
                resumed.phase,
                Phase::Blocked,
                "interrupted negative review must not become approval"
            );
        }
    }
}
