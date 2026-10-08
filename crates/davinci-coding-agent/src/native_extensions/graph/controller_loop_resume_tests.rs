//! A graph loop stopped anywhere resumes exactly where it was: the same
//! dispatches in the same order, the same revision notes in front of the
//! writer, the same spend, the same files. The baseline is one uninterrupted
//! run; every interrupted run is compared against it.
use super::super::types::*;
use super::*;

/// Where an interrupted run is stopped.
#[derive(Debug, Clone, Copy)]
enum Cut {
    /// After this dispatch (1-based) succeeds and is checkpointed.
    After(usize),
    /// While this dispatch (1-based) is still running, before it writes.
    During(usize),
}

/// One worker dispatch as the runner saw it.
#[derive(Debug, Clone, PartialEq)]
struct Dispatch {
    task_id: String,
    /// The whole briefing, with the temporary workspace path masked so
    /// separate runs compare byte for byte.
    briefing: String,
}

impl Dispatch {
    /// The briefing carried the failed verification output.
    fn saw_verify_note(&self) -> bool {
        self.briefing.contains(VERIFY_NOTE)
    }

    /// The briefing carried the review's blocking issue.
    fn saw_review_note(&self) -> bool {
        self.briefing.contains(REVIEW_NOTE)
    }
}

const VERIFY_NOTE: &str = "loop-verify-failure-marker";
const REVIEW_NOTE: &str = "loop-review-issue-marker";

/// Runs one segment of the loop. Everything the fixture decides comes from
/// the task id or the files on disk, never from closure state, so a resumed
/// segment makes the same decisions the uninterrupted run made.
fn segment(
    cwd: &Path,
    calls: &Arc<Mutex<Vec<Dispatch>>>,
    cut: Option<Cut>,
    stop_after: Option<String>,
    resume: Option<GraphRun>,
) -> GraphRun {
    let abort = Arc::new(AtomicBool::new(false));
    let recorded = calls.clone();
    let operator = abort.clone();
    let runner: Arc<WorkerRunner> = Arc::new(move |spec, worker_abort, _| {
        let number = {
            let mut calls = recorded.lock().unwrap();
            calls.push(Dispatch {
                task_id: spec.task_id.clone(),
                briefing: spec
                    .briefing
                    .replace(&spec.cwd.display().to_string(), "<cwd>"),
            });
            calls.len()
        };
        if matches!(cut, Some(Cut::During(at)) if at == number) {
            // The operator stops the graph; like a real worker, this one
            // exits only once the controller's stop reaches it.
            operator.store(true, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !worker_abort.load(Ordering::SeqCst) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "stop never reached the worker"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            return WorkerResult {
                ok: false,
                // The real worker's abort text, plus output that the failure
                // classifier would read as an environment problem.
                failure_reason: Some("worker aborted".into()),
                final_text: "checking the environment first".into(),
                ..Default::default()
            };
        }
        let artifact = match spec.expect {
            ArtifactKind::Classification => Artifact::Classification(Classification {
                task_class: TaskClass::Feature,
                complexity: Complexity::Complex,
                rationale: "loop resume fixture".into(),
                research_tasks: vec![],
                milestones: Some(vec!["first".into(), "second".into()]),
            }),
            ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                steps: vec![PlanStep {
                    description: "append the writer id".into(),
                    files: vec!["owned.txt".into()],
                }],
                tests_to_add: vec![],
                tests_to_run: vec![],
                completion_criteria: vec!["done".into()],
                invariants: vec![],
                out_of_scope: vec![],
            })),
            ArtifactKind::PatchReport => {
                let path = spec.cwd.join("owned.txt");
                let before = std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, format!("{before}{}\n", spec.task_id)).unwrap();
                Artifact::PatchReport(Box::new(PatchReport {
                    summary: format!("{} appended", spec.task_id),
                    changed_files: vec!["owned.txt".into()],
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                }))
            }
            // The first review of the run asks for changes; every later one
            // approves. The first review is always `review-1`.
            ArtifactKind::Review => {
                let changes = spec.task_id == "review-1";
                Artifact::Review(Box::new(ReviewDecision {
                    verdict: if changes {
                        Verdict::ChangesRequired
                    } else {
                        Verdict::Approve
                    },
                    issues: if changes {
                        vec![ReviewIssue {
                            severity: Severity::Major,
                            file: Some("owned.txt".into()),
                            description: REVIEW_NOTE.into(),
                        }]
                    } else {
                        vec![]
                    },
                    notes: "fixture review".into(),
                    reviewed_chunk_ids: vec![],
                }))
            }
            ArtifactKind::Evidence => unreachable!(),
        };
        let _ = super::super::store::write_artifact(&spec.artifact_path, &artifact);
        WorkerResult {
            ok: true,
            artifact: Some(artifact),
            ..Default::default()
        }
    });
    let stop = abort.clone();
    run_graph(
        RunOptions {
            goal: "two milestones through a revision loop".into(),
            cwd: cwd.into(),
            forced: None,
            dry_run: false,
            abort,
            resume_artifacts: HashMap::new(),
            resume_run: resume.map(Box::new),
        },
        ControllerDeps {
            runner,
            // Verification fails exactly while the first writer's line is the
            // newest one, so the first verification starts a revision.
            verify_exec: Arc::new(|_, cwd, _, _| {
                let text = std::fs::read_to_string(cwd.join("owned.txt")).unwrap();
                if text.lines().last() == Some("implement-1") {
                    (1, VERIFY_NOTE.into(), 1)
                } else {
                    (0, "fixture verification passed".into(), 1)
                }
            }),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    name: "fixture".into(),
                    command: "fixture".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: false,
            on_update: Arc::new(move |run, _| {
                if stop_after
                    .as_deref()
                    .and_then(|id| run.task(id))
                    .is_some_and(|task| task.status == TaskStatus::Succeeded)
                {
                    stop.store(true, Ordering::SeqCst);
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
    )
}

/// Drives the loop to completion, stopping once at `cut` and resuming from
/// the durable checkpoint each time. Returns every dispatch, the final run,
/// and how many times the loop was reopened.
fn drive(cut: Option<Cut>, baseline: &[Dispatch]) -> (Vec<Dispatch>, GraphRun, String, usize) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("owned.txt"), "original\n").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stop_after = match cut {
        Some(Cut::After(at)) => Some(baseline[at - 1].task_id.clone()),
        _ => None,
    };
    let mut run = segment(dir.path(), &calls, cut, stop_after, None);
    let mut resumes = 0;
    while run.phase != Phase::Done {
        assert_eq!(
            run.phase,
            Phase::Cancelled,
            "{cut:?}: the loop must only stop where it was cut: {:?}",
            run.blocked_reason
        );
        resumes += 1;
        assert!(resumes <= 2, "{cut:?}: resume did not converge");
        let saved = super::super::store::load_run_checked(dir.path(), &run.run_id).unwrap();
        super::super::continuation::validate_resume(&saved, dir.path())
            .unwrap_or_else(|error| panic!("{cut:?}: checkpoint is not resumable: {error}"));
        run = segment(dir.path(), &calls, None, None, Some(saved));
    }
    let owned = std::fs::read_to_string(dir.path().join("owned.txt")).unwrap();
    let calls = calls.lock().unwrap().clone();
    (calls, run, owned, resumes)
}

fn baseline() -> (Vec<Dispatch>, GraphRun, String) {
    let (calls, run, owned, resumes) = drive(None, &[]);
    assert_eq!(resumes, 0);
    (calls, run, owned)
}

#[test]
fn the_baseline_loop_revises_on_verification_and_on_review() {
    let (calls, run, owned) = baseline();
    assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
    assert_eq!(run.counters.revision_cycles, 2);
    let writers: Vec<_> = calls
        .iter()
        .filter(|call| call.task_id.starts_with("implement-"))
        .collect();
    assert_eq!(writers.len(), 4, "{calls:?}");
    // The verification revision carries the failure, the review revision
    // carries the blocking issue, and neither leaks into the next milestone.
    assert!(writers[1].saw_verify_note(), "{calls:?}");
    assert!(writers[2].saw_review_note(), "{calls:?}");
    assert!(!writers[3].saw_verify_note() && !writers[3].saw_review_note());
    assert!(calls
        .iter()
        .all(|call| !call.briefing.contains("RETRY NOTICE")));
    assert_eq!(
        owned,
        "original\nimplement-1\nimplement-2\nimplement-3\nimplement-4\n"
    );
}

#[test]
fn a_loop_stopped_after_any_dispatch_resumes_without_repeating_or_skipping_work() {
    let (expected, reference, expected_owned) = baseline();
    for at in 1..=expected.len() {
        let cut = Cut::After(at);
        let (calls, run, owned, resumes) = drive(Some(cut), &expected);
        if at < expected.len() {
            assert_eq!(resumes, 1, "{cut:?} never stopped the loop");
        }
        assert_eq!(calls, expected, "{cut:?}: dispatches differ");
        assert_eq!(owned, expected_owned, "{cut:?}: workspace differs");
        assert_eq!(
            run.counters.revision_cycles,
            reference.counters.revision_cycles
        );
        assert_eq!(run.counters.replans, reference.counters.replans);
        assert_eq!(
            run.counters.workers_spawned, reference.counters.workers_spawned,
            "{cut:?}: spend counted twice"
        );
        let cursor = run.continuation.as_ref().unwrap();
        assert_eq!(cursor.completed_milestones, 2, "{cut:?}");
        assert_eq!(
            cursor.indices,
            reference.continuation.as_ref().unwrap().indices,
            "{cut:?}: node cursor drifted"
        );
    }
}

#[test]
fn a_loop_stopped_inside_a_worker_reruns_only_that_worker() {
    let (expected, reference, expected_owned) = baseline();
    for at in 1..=expected.len() {
        let cut = Cut::During(at);
        let (calls, run, owned, resumes) = drive(Some(cut), &expected);
        assert_eq!(resumes, 1, "{cut:?} never stopped the loop");
        // The stopped attempt is the only extra dispatch, and it is retried
        // as the same node with the same briefing: a stop is not a failure,
        // so the worker is not told a previous attempt went wrong.
        let mut retried = calls.clone();
        let stopped = retried.remove(at - 1);
        assert_eq!(stopped, expected[at - 1], "{cut:?}: wrong node stopped");
        assert_eq!(retried, expected, "{cut:?}: dispatches differ");
        assert_eq!(owned, expected_owned, "{cut:?}: workspace differs");
        assert_eq!(
            run.counters.revision_cycles,
            reference.counters.revision_cycles
        );
        assert_eq!(run.counters.replans, reference.counters.replans);
        assert_eq!(
            run.continuation.as_ref().unwrap().indices,
            reference.continuation.as_ref().unwrap().indices,
            "{cut:?}: node cursor drifted"
        );
    }
}
