use super::*;

fn block_run_storage(cwd: &Path, run_id: &str, backup: &Path) {
    let path = super::super::store::run_dir(cwd, run_id);
    std::fs::rename(&path, backup).unwrap();
    std::fs::write(path, "blocked storage").unwrap();
}

#[test]
fn checkpoint_failure_prevents_dispatch_and_preserves_last_durable_state() {
    for stage in 0..3 {
        let fail_initial = stage == 0;
        let dir = tempfile::tempdir().unwrap();
        if fail_initial {
            std::fs::write(dir.path().join(".davinci"), "blocked storage").unwrap();
        }
        let dispatches = Arc::new(AtomicUsize::new(0));
        let calls = dispatches.clone();
        let updates = Arc::new(Mutex::new(Vec::new()));
        let observed = updates.clone();
        let backup = dir.path().join("durable-backup");
        let preserved = backup.clone();
        let worker_backup = backup.clone();
        let run = run_graph(
            RunOptions {
                goal: "checkpoint failure fixture".into(),
                cwd: dir.path().into(),
                forced: None,
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            ControllerDeps {
                runner: Arc::new(move |spec, _, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if stage == 2 {
                        let path = spec.artifact_path.parent().unwrap().parent().unwrap();
                        std::fs::rename(path, &worker_backup).unwrap();
                        std::fs::write(path, "blocked storage").unwrap();
                    }
                    WorkerResult {
                        ok: true,
                        artifact: Some(Artifact::Classification(
                            super::super::types::Classification {
                                task_class: super::super::types::TaskClass::Feature,
                                complexity: super::super::types::Complexity::Standard,
                                rationale: "fixture".into(),
                                research_tasks: vec![],
                                milestones: None,
                            },
                        )),
                        ..Default::default()
                    }
                }),
                verify_exec: Arc::new(|_, _, _, _| panic!("verification after save failure")),
                config: Default::default(),
                session_model: None,
                session_thinking: None,
                project_trusted: false,
                on_update: Arc::new(move |run, note| {
                    observed
                        .lock()
                        .unwrap()
                        .push((run.phase, note.map(str::to_owned)));
                    if stage == 1 && note == Some("run created") {
                        block_run_storage(Path::new(&run.cwd), &run.run_id, &preserved);
                    }
                }),
                memory: None,
                learning: None,
                governor: None,
                language_intelligence: None,
                processes: None,
                browser: None,
                runtime: None,
                permissions: None,
                task_contract: None,
            },
        );
        assert_eq!(
            dispatches.load(Ordering::SeqCst),
            usize::from(stage == 2),
            "stage={stage}"
        );
        assert_eq!(run.phase, Phase::Blocked);
        assert_eq!(run.current_lifecycle(), GraphLifecycle::Stopped);
        assert!(run
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("checkpoint persistence failed"));
        let updates = updates.lock().unwrap();
        assert_eq!(updates.last().unwrap().0, Phase::Blocked);
        assert!(updates
            .last()
            .unwrap()
            .1
            .as_deref()
            .unwrap()
            .contains("not saved"));
        if !fail_initial {
            let durable: GraphRun =
                serde_json::from_slice(&std::fs::read(backup.join("state.json")).unwrap()).unwrap();
            assert_eq!(durable.phase, Phase::Classify);
            assert_eq!(durable.tasks.len(), usize::from(stage == 2));
            assert_eq!(durable.counters.workers_spawned, u32::from(stage == 2));
        }
    }
}
