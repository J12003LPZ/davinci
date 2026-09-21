use super::*;

fn attempt_fixture(cwd: &Path, previous: Option<GraphRun>, fail: bool) -> GraphRun {
    let controller = super::super::GraphController::new(cwd.into());
    let (mut deps, errors) = controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
    assert!(errors.is_empty());
    deps.config.verify_commands = vec![super::super::types::VerifyCommandSpec {
        name: "fixture".into(),
        command: "fixture".into(),
        from_plan: false,
    }];
    deps.verify_exec = Arc::new(|_, _, _, _| (0, "passed".into(), 1));
    deps.runner = Arc::new(move |spec, abort, progress| {
        // The attempt must exist durably before the worker receives authority.
        let run_dir = spec.artifact_path.parent().unwrap().parent().unwrap();
        let run = super::super::store::load_run_checked(
            &spec.cwd,
            run_dir.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        let task = run.task(&spec.task_id).unwrap();
        let record = super::super::store::read_task_attempt(
            &spec.cwd,
            &run.run_id,
            &spec.task_id,
            task.attempts,
        )
        .expect("attempt must be persisted before dispatch");
        assert_eq!(record.status, TaskStatus::Running);
        assert!(record.started_at.is_some());
        assert!(record.ended_at.is_none());
        if fail {
            let usage = WorkerUsage {
                input: 7,
                ..Default::default()
            };
            progress("fixture progress", &usage);
            let durable = super::super::store::load_run_checked(&spec.cwd, &run.run_id).unwrap();
            assert_eq!(
                durable.task(&spec.task_id).unwrap().usage.input,
                task.usage.input + 7
            );
            assert_eq!(
                durable.continuation.unwrap().attempt_history[&spec.task_id]
                    .last()
                    .unwrap()
                    .usage
                    .input,
                7
            );
            std::fs::write(
                spec.artifact_path.with_extension("effects.jsonl"),
                format!("attempt {} receipt\n", task.attempts),
            )
            .unwrap();
            return WorkerResult {
                exit_code: 17,
                failure_reason: Some("fixture process failure".into()),
                usage: WorkerUsage {
                    input: 7,
                    ..Default::default()
                },
                ..Default::default()
            };
        }
        super::super::worker::run_dry_worker(spec, abort, progress)
    });
    run_graph(
        RunOptions {
            goal: "attempt history fixture".into(),
            cwd: cwd.into(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: previous.map(Box::new),
        },
        deps,
    )
}

#[test]
fn worker_attempts_are_durable_before_dispatch_and_survive_resume() {
    let dir = tempfile::tempdir().unwrap();
    let failed = attempt_fixture(dir.path(), None, true);
    assert_eq!(failed.phase, Phase::Blocked);
    let task = failed.tasks.iter().find(|task| task.attempts > 0).unwrap();
    assert_eq!(task.attempts, 2);
    let task_id = task.id.clone();
    let first =
        super::super::store::read_task_attempt(dir.path(), &failed.run_id, &task_id, 1).unwrap();
    let second =
        super::super::store::read_task_attempt(dir.path(), &failed.run_id, &task_id, 2).unwrap();
    for record in [&first, &second] {
        assert_eq!(record.status, TaskStatus::Failed);
        assert_eq!(record.usage.input, 7);
        assert!(record.ended_at.is_some());
        assert!(record.error.as_ref().unwrap().contains("fixture"));
        let archived = super::super::store::run_dir(dir.path(), &failed.run_id).join(format!(
            "artifacts/{task_id}.attempt_{}.effects.jsonl",
            record.attempt
        ));
        assert_eq!(
            std::fs::read_to_string(archived).unwrap(),
            format!("attempt {} receipt\n", record.attempt)
        );
    }
    let loaded = super::super::store::load_run_checked(dir.path(), &failed.run_id).unwrap();
    let resumed = attempt_fixture(dir.path(), Some(loaded), false);
    assert_eq!(resumed.phase, Phase::Done, "{:?}", resumed.blocked_reason);
    assert_eq!(resumed.task(&task_id).unwrap().attempts, 3);
    assert_eq!(
        super::super::store::read_task_attempt(dir.path(), &failed.run_id, &task_id, 1),
        Some(first)
    );
    assert_eq!(
        super::super::store::read_task_attempt(dir.path(), &failed.run_id, &task_id, 2),
        Some(second)
    );
    let third =
        super::super::store::read_task_attempt(dir.path(), &failed.run_id, &task_id, 3).unwrap();
    assert_eq!(third.status, TaskStatus::Succeeded);
    let artifact =
        super::super::store::run_dir(dir.path(), &failed.run_id).join(third.artifact_file.unwrap());
    assert!(artifact.is_file(), "attempt must retain its own artifact");
    assert_ne!(
        artifact,
        artifact_path(dir.path(), &failed.run_id, &task_id)
    );
}

#[test]
fn corrupt_attempt_history_is_refused_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let failed = attempt_fixture(dir.path(), None, true);
    let mut corrupt = failed.clone();
    let history = corrupt
        .continuation
        .as_mut()
        .unwrap()
        .attempt_history
        .values_mut()
        .next()
        .unwrap();
    history.push(history[0].clone());
    let refused = attempt_fixture(dir.path(), Some(corrupt), false);
    assert_eq!(refused.phase, Phase::Blocked);
    assert!(refused.blocked_reason.unwrap().contains("attempt history"));
    let durable = super::super::store::load_run_checked(dir.path(), &failed.run_id).unwrap();
    assert_eq!(durable.continuation, failed.continuation);
}

#[test]
fn attempt_record_write_failure_blocks_dispatch_or_retry() {
    for fail_before_dispatch in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let controller = super::super::GraphController::new(dir.path().into());
        let (mut deps, _) = controller.deps(true, &Arc::new(super::super::ActiveRun::default()));
        deps.on_update = Arc::new(move |run, note| {
            if fail_before_dispatch && note == Some("run created") {
                let path = super::super::store::run_dir(Path::new(&run.cwd), &run.run_id)
                    .join("artifacts/classify.attempt_1.json");
                std::fs::create_dir(path).unwrap();
            }
        });
        deps.runner = Arc::new(move |spec, _, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            let path = spec
                .artifact_path
                .with_file_name(format!("{}.attempt_1.json", spec.task_id));
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(path).unwrap();
            WorkerResult {
                exit_code: 7,
                failure_reason: Some("retryable process failure".into()),
                ..Default::default()
            }
        });
        let run = run_graph(
            RunOptions {
                goal: "attempt record failure".into(),
                cwd: dir.path().into(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
        );
        assert_eq!(run.phase, Phase::Blocked);
        assert!(run
            .blocked_reason
            .unwrap()
            .contains("checkpoint persistence failed"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(!fail_before_dispatch)
        );
    }
}
