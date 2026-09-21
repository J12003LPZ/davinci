use super::super::types::*;
use super::*;

fn revision_fixture(fail_review: bool, fail_forever: bool, resume: Option<GraphRun>) -> GraphRun {
    let dir = tempfile::tempdir().unwrap();
    let reviews = Arc::new(AtomicUsize::new(0));
    let verifies = Arc::new(AtomicUsize::new(0));
    let runner: Arc<WorkerRunner> = Arc::new(move |spec, _, _| {
        let artifact = match spec.expect {
            ArtifactKind::Classification => Artifact::Classification(Classification {
                task_class: TaskClass::Feature,
                complexity: Complexity::Complex,
                rationale: "revision fixture".into(),
                research_tasks: vec![],
                milestones: Some(vec!["first".into(), "second".into(), "third".into()]),
            }),
            ArtifactKind::Plan => Artifact::Plan(Box::new(ImplementationPlan {
                steps: vec![],
                tests_to_add: vec![],
                tests_to_run: vec![],
                completion_criteria: vec!["done".into()],
                invariants: vec![],
                out_of_scope: vec![],
            })),
            ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                summary: "fixture".into(),
                changed_files: vec![],
                deviations: vec![],
                plan_invalidated: false,
                invalidation_reason: None,
            })),
            ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                verdict: if fail_review && reviews.fetch_add(1, Ordering::SeqCst) == 0 {
                    Verdict::ChangesRequired
                } else {
                    Verdict::Approve
                },
                issues: vec![],
                notes: "fixture review".into(),
                reviewed_chunk_ids: vec![],
            })),
            ArtifactKind::Evidence => unreachable!(),
        };
        WorkerResult {
            ok: true,
            artifact: Some(artifact),
            ..Default::default()
        }
    });
    run_graph(
        RunOptions {
            goal: "three milestones with revisions".into(),
            cwd: dir.path().into(),
            forced: None,
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: resume.map(Box::new),
        },
        ControllerDeps {
            runner,
            verify_exec: Arc::new(move |_, _, _, _| {
                let fail =
                    fail_forever || (!fail_review && verifies.fetch_add(1, Ordering::SeqCst) == 0);
                (if fail { 1 } else { 0 }, "fixture verification".into(), 1)
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
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
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

#[test]
fn milestone_verification_revision_does_not_use_next_milestone_dependencies() {
    let run = revision_fixture(false, false, None);
    assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
    assert_eq!(run.counters.revision_cycles, 1);
    assert_eq!(
        run.tasks.iter().filter(|t| t.role == Role::Writer).count(),
        4
    );
    assert_eq!(
        run.tasks
            .iter()
            .filter(|t| t.role == Role::Reviewer)
            .count(),
        3
    );
    let definition = run.definition.as_ref().unwrap();
    assert!(definition
        .incoming_edges("implement-2")
        .any(|edge| edge.from == "implement-1"));
    assert!(definition
        .incoming_edges("review-1")
        .any(|edge| edge.from == "implement-2"));
    assert!(run
        .task("implement-2")
        .unwrap()
        .depends_on
        .contains(&"implement-1".into()));
    validate_definition(definition).unwrap();
}

#[test]
fn milestone_review_revision_does_not_use_next_milestone_dependencies() {
    let run = revision_fixture(true, false, None);
    assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
    assert_eq!(run.counters.revision_cycles, 1);
    validate_definition(run.definition.as_ref().unwrap()).unwrap();
}

#[test]
fn resumed_milestone_revisions_reach_verification_limit_not_topology_failure() {
    let initial = revision_fixture(false, true, None);
    assert!(initial
        .blocked_reason
        .as_deref()
        .unwrap()
        .contains("verification still failing"));
    let resumed = revision_fixture(false, false, Some(initial.clone()));
    assert_eq!(resumed.phase, Phase::Done, "{:?}", resumed.blocked_reason);
    assert_eq!(resumed.run_id, initial.run_id);
    assert_eq!(
        resumed.counters.revision_cycles,
        initial.counters.revision_cycles + 1
    );
}
