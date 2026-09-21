use super::*;

#[test]
fn repeated_stage_interruptions_preserve_completed_milestones_and_mutations() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("owned.txt"), "original\n").unwrap();
    let controller = super::super::GraphController::new(dir.path().into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut previous = None;
    for stop_at in [
        Some("classify"),
        Some("research-1"),
        Some("plan-1"),
        Some("implement-1"),
        Some("review-1"),
        Some("plan-2"),
        Some("implement-2"),
        Some("review-2"),
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
        deps.verify_exec = Arc::new(|_, _, _, _| (0, "passed".into(), 1));
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if spec.role == Role::Writer {
                let path = spec.cwd.join("owned.txt");
                let before = std::fs::read_to_string(&path).unwrap();
                std::fs::write(path, format!("{before}{}\n", spec.task_id)).unwrap();
            }
            let mut result = super::super::worker::run_dry_worker(spec, abort, progress);
            if let Some(Artifact::Classification(classification)) = &mut result.artifact {
                classification.milestones = Some(vec!["first".into(), "second".into()]);
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
                stop.store(true, Ordering::Relaxed);
            }
        });
        let run = run_graph(
            RunOptions {
                goal: "two milestones".into(),
                cwd: dir.path().into(),
                forced: Some(Complexity::Complex),
                dry_run: false,
                abort,
                resume_artifacts: HashMap::new(),
                resume_run: previous.map(Box::new),
            },
            deps,
        );
        if stop_at.is_some() {
            assert_eq!(
                run.phase,
                Phase::Cancelled,
                "{stop_at:?}: {:?}",
                run.blocked_reason
            );
        } else {
            assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
            assert_eq!(run.continuation.as_ref().unwrap().completed_milestones, 2);
        }
        if stop_at == Some("plan-2") {
            assert_eq!(run.continuation.as_ref().unwrap().completed_milestones, 1);
        }
        previous = Some(super::super::store::load_run_checked(dir.path(), &run.run_id).unwrap());
    }
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "classify",
            "research-1",
            "plan-1",
            "implement-1",
            "review-1",
            "plan-2",
            "implement-2",
            "review-2"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("owned.txt")).unwrap(),
        "original\nimplement-1\nimplement-2\n"
    );
    let run = previous.unwrap();
    assert!(run
        .task("implement-1")
        .unwrap()
        .mutation
        .as_ref()
        .unwrap()
        .diff()
        .contains("+implement-1"));
    assert!(run
        .task("implement-2")
        .unwrap()
        .mutation
        .as_ref()
        .unwrap()
        .diff()
        .contains("+implement-2"));
}

#[test]
fn corrupt_continuation_cannot_skip_unfinished_milestones() {
    let dir = tempfile::tempdir().unwrap();
    let initial = verification_fixture(dir.path(), None, true, Arc::new(Mutex::new(Vec::new())));
    let mut corrupt = initial.clone();
    corrupt.continuation.as_mut().unwrap().completed_milestones = 100;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let refused = verification_fixture(dir.path(), Some(corrupt), false, calls.clone());
    assert_eq!(refused.phase, Phase::Blocked);
    assert!(refused.blocked_reason.unwrap().contains("milestone"));
    assert!(calls.lock().unwrap().is_empty());
    let persisted = super::super::store::load_run_checked(dir.path(), &initial.run_id).unwrap();
    assert_eq!(
        persisted.continuation, initial.continuation,
        "a rejected snapshot must not overwrite durable state"
    );
}

#[test]
fn failed_research_resumes_only_the_failed_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let controller = super::super::GraphController::new(dir.path().into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut previous = None;
    for fail in [true, false] {
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if fail && spec.task_id == "research-2" {
                return WorkerResult {
                    failure_reason: Some("fixture unavailable".into()),
                    ..Default::default()
                };
            }
            let mut result = super::super::worker::run_dry_worker(spec, abort, progress);
            if let Some(Artifact::Classification(classification)) = &mut result.artifact {
                classification
                    .research_tasks
                    .push(super::super::types::ResearchRequest {
                        kind: super::super::types::ResearchKind::CodeSearch,
                        focus: "second sibling".into(),
                    });
            }
            result
        });
        calls.lock().unwrap().clear();
        let run = run_graph(
            RunOptions {
                goal: "research recovery".into(),
                cwd: dir.path().into(),
                forced: Some(Complexity::Standard),
                dry_run: true,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: previous.map(Box::new),
            },
            deps,
        );
        if fail {
            assert_eq!(run.phase, Phase::Blocked);
            assert_eq!(
                run.task("research-1").unwrap().status,
                TaskStatus::Succeeded
            );
        } else {
            assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
            assert_eq!(
                *calls.lock().unwrap(),
                ["research-2", "plan-1", "implement-1", "review-1"]
            );
            assert!(run.task("research-2").unwrap().attempts > 1);
        }
        previous = Some(run);
    }
}

#[test]
fn changed_source_invalidates_completed_review_but_never_replays_writer() {
    fn attempt(
        cwd: &Path,
        calls: Arc<Mutex<Vec<String>>>,
        resume: Option<GraphRun>,
        stop_after_milestone: bool,
        security: bool,
    ) -> GraphRun {
        let resuming = resume.is_some();
        let controller = super::super::GraphController::new(cwd.into());
        let (mut deps, errors) =
            controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        assert!(errors.is_empty());
        if security {
            deps.config.security_verification = SecurityPolicyMode::Always;
        }
        deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
            name: "fixture".into(),
            command: "fixture".into(),
            from_plan: false,
        }];
        deps.verify_exec = Arc::new(|_, _, _, _| (0, "passed".into(), 1));
        let recorded = calls.clone();
        deps.runner = Arc::new(move |spec, abort, progress| {
            recorded.lock().unwrap().push(spec.task_id.clone());
            if security && spec.role == Role::Writer {
                std::fs::write(spec.cwd.join("ui.rs"), "pub fn render() {}\n").unwrap();
            }
            super::super::worker::run_dry_worker(spec, abort, progress)
        });
        let abort = Arc::new(AtomicBool::new(false));
        let stop = abort.clone();
        deps.on_update = Arc::new(move |run, note| {
            if !resuming
                && if stop_after_milestone {
                    note == Some("milestone complete")
                } else {
                    run.task("review-1")
                        .is_some_and(|task| task.status == TaskStatus::Succeeded)
                }
            {
                stop.store(true, Ordering::Relaxed);
            }
        });
        let options = RunOptions {
            goal: "review fixture".into(),
            cwd: cwd.into(),
            forced: Some(Complexity::Standard),
            dry_run: false,
            abort,
            resume_artifacts: HashMap::new(),
            resume_run: resume.map(Box::new),
        };
        run_graph(options, deps)
    }
    for (change_source, stop_after_milestone, security) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (true, false, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let initial = attempt(
            dir.path(),
            calls.clone(),
            None,
            stop_after_milestone,
            security,
        );
        assert_eq!(initial.phase, Phase::Cancelled);
        assert_eq!(
            initial.task("review-1").unwrap().status,
            TaskStatus::Succeeded
        );
        if change_source {
            std::fs::write(dir.path().join("new-source.txt"), "changed input").unwrap();
        }
        calls.lock().unwrap().clear();
        let resumed = attempt(
            dir.path(),
            calls.clone(),
            Some(initial),
            stop_after_milestone,
            security,
        );
        assert_eq!(resumed.phase, Phase::Done, "{:?}", resumed.blocked_reason);
        let ids: std::collections::HashSet<_> = resumed.tasks.iter().map(|task| &task.id).collect();
        assert_eq!(
            ids.len(),
            resumed.tasks.len(),
            "resumed checks must keep distinct task identities"
        );
        let calls = calls.lock().unwrap();
        if security {
            assert_eq!(*calls, ["review-2-chunk-1", "review-2-chunk-2", "review-2"]);
        } else if change_source {
            assert_eq!(*calls, ["review-2"]);
        } else {
            assert!(
                calls.is_empty(),
                "unchanged completed review must survive: {calls:?}"
            );
        }
    }
}

fn verification_fixture(
    cwd: &Path,
    resume: Option<GraphRun>,
    missing_command: bool,
    dispatched: Arc<Mutex<Vec<String>>>,
) -> GraphRun {
    let controller = super::super::GraphController::new(cwd.into());
    let (mut deps, errors) = controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
    assert!(errors.is_empty());
    deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
        name: "fixture check".into(),
        command: "fixture-check".into(),
        from_plan: false,
    }];
    deps.verify_exec = Arc::new(move |_, _, _, _| {
        if missing_command {
            (127, "fixture-check: command not found".into(), 1)
        } else {
            (0, "fixture passed".into(), 1)
        }
    });
    deps.runner = Arc::new(move |spec, abort, progress| {
        dispatched.lock().unwrap().push(spec.task_id.clone());
        if spec.role == Role::Writer {
            std::fs::write(spec.cwd.join("owned.txt"), "after graph\n").unwrap();
        }
        super::super::worker::run_dry_worker(spec, abort, progress)
    });
    run_graph(
        RunOptions {
            goal: "repair fixture".into(),
            cwd: cwd.into(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: resume.map(Box::new),
        },
        deps,
    )
}

#[test]
fn repaired_verification_resumes_without_dispatching_completed_workers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("owned.txt"), "before graph\n").unwrap();
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let initial = verification_fixture(dir.path(), None, true, first_calls.clone());
    assert_eq!(initial.phase, Phase::Blocked);
    assert_eq!(
        initial.counters.revision_cycles, 0,
        "environment failure is not a revision"
    );
    assert_eq!(*first_calls.lock().unwrap(), ["classify", "implement-1"]);
    let durable = super::super::store::load_run_checked(dir.path(), &initial.run_id).unwrap();
    let resumed_calls = Arc::new(Mutex::new(Vec::new()));
    let resumed = verification_fixture(
        dir.path(),
        Some(durable.clone()),
        false,
        resumed_calls.clone(),
    );
    assert_eq!(resumed.phase, Phase::Done, "{:?}", resumed.blocked_reason);
    assert!(resumed_calls.lock().unwrap().is_empty());
    assert_eq!(resumed.run_id, initial.run_id);
    assert_eq!(
        resumed.counters.workers_spawned,
        initial.counters.workers_spawned
    );
    assert_eq!(
        resumed.counters.revision_cycles,
        initial.counters.revision_cycles
    );
    assert_eq!(
        resumed.tasks, durable.tasks,
        "completed attempt metadata must survive reopen"
    );
    assert!(resumed
        .task("implement-1")
        .unwrap()
        .mutation
        .as_ref()
        .unwrap()
        .diff()
        .contains("before graph"));
}
