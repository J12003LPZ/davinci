use davinci_agent::runtime::checkpoints::compute_sha256;
use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, BlobStore, CheckpointError, RunId, TaskId};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    directory: PathBuf,
    identity: JournalIdentity,
    root: RootNamespaceId,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let directory = temp_root.join("operation-journal");
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        Self {
            _temp: temp,
            directory,
            identity,
            root: RootNamespaceId::new(),
        }
    }

    fn open(&self) -> OperationJournal {
        OperationJournal::open(&self.directory, self.identity.clone(), self.root.clone()).unwrap()
    }

    fn spec(&self, key: &str, payload: Value) -> OperationSpec {
        OperationSpec::new(
            OperationContext {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root.clone(),
                session_id: "operation-capacity-test".to_owned(),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: self.identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: None,
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, key).unwrap(),
            OperationKind::CustomExternalAction,
            EffectProfile {
                classification: EffectClass::ExternalMutation,
                supports_idempotency_key: true,
                supports_postcondition_probe: false,
                supports_compensation: false,
                requires_live_owner: true,
            },
            payload,
            vec![],
        )
        .unwrap()
    }
}

fn owner() -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap()
}

fn running_with_possible_effect(
    journal: &OperationJournal,
    attempt: OperationAttempt,
    spec: &OperationSpec,
) -> OperationAttempt {
    let attempt = journal
        .transition(
            attempt.owner(),
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                spec.intent_digest().unwrap(),
                "capacity-test-policy".to_owned(),
                Timestamp::from_unix_millis(10),
            )),
            None,
            vec![],
        )
        .unwrap();
    let attempt = journal
        .transition(
            attempt.owner(),
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )
        .unwrap();
    let claim = journal
        .claim_dispatch(attempt.owner(), attempt.attempt_id(), attempt.revision())
        .unwrap();
    let permit = journal
        .latch_effect_start(claim, Timestamp::from_unix_millis(20))
        .unwrap();
    permit.dispatch(journal, || ()).unwrap();
    journal.load_attempt(attempt.attempt_id()).unwrap()
}

fn percentile_micros(samples: &mut [Duration], percentile: usize) -> u128 {
    samples.sort_unstable();
    let index = ((samples.len().saturating_sub(1)) * percentile / 100).min(samples.len() - 1);
    samples[index].as_micros()
}

fn report_measurement(name: &str, samples: &mut [Duration], count: usize) {
    let p50 = percentile_micros(samples, 50);
    let p95 = percentile_micros(samples, 95);
    let total = samples.iter().copied().sum::<Duration>();
    let throughput = count as f64 / total.as_secs_f64().max(f64::EPSILON);
    println!(
        "scenario={name} count={count} p50_us={p50} p95_us={p95} throughput_ops_s={throughput:.2}"
    );
}

#[test]
fn oversized_result_is_rejected_before_any_state_change() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec("oversized-result", json!({"kind": "mutation"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt, &spec);
    let payload = json!({"result": "x".repeat(1_100_000)});
    let result = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());

    assert!(matches!(
        journal.transition(
            attempt.owner(),
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::CompleteSuccess {
                result,
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(30),
            },
            Some(payload),
            vec![],
        ),
        Err(JournalError::OversizedRecord {
            record: "result payload",
            ..
        })
    ));
    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot.results.is_empty());
    assert_eq!(snapshot.attempts[0].state(), OperationState::EffectPossible);
}

#[test]
fn capacity_observation_reports_occupancy_without_materializing_records() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let empty = journal.capacity().unwrap();
    assert_eq!(empty.operation_count, 0);
    assert_eq!(empty.pending_outbox_count, 0);
    assert_eq!(empty.snapshot_record_count, 0);
    assert_eq!(empty.max_pending_outbox, 4096);

    let spec = fixture.spec("capacity-observation", json!({"kind": "read"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    journal.persist_intent(&spec, &initial).unwrap();

    let occupied = journal.capacity().unwrap();
    assert_eq!(occupied.operation_count, 1);
    assert_eq!(occupied.pending_outbox_count, 0);
    assert_eq!(occupied.snapshot_record_count, 3);
    assert_eq!(occupied.max_operations, 100_000);
}

#[test]
fn oversized_outbox_is_rejected_before_result_or_event_publication() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let spec = fixture.spec("oversized-outbox", json!({"kind": "mutation"}));
    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
    let attempt = journal.persist_intent(&spec, &initial).unwrap();
    let attempt = running_with_possible_effect(&journal, attempt, &spec);
    let payload = json!({"ok": true});
    let result = ResultRef::new(PayloadDigest::of_json(&payload).unwrap());
    let outbox = OutboxDraft::new("projection", json!({"data": "x".repeat(300_000)})).unwrap();

    assert!(matches!(
        journal.transition(
            attempt.owner(),
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::CompleteSuccess {
                result,
                effect_status: EffectStatus::EffectsObserved,
                finished_at: Timestamp::from_unix_millis(30),
            },
            Some(payload),
            vec![outbox],
        ),
        Err(JournalError::OversizedRecord {
            record: "outbox payload",
            ..
        })
    ));
    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot.results.is_empty());
    assert!(snapshot.outbox.is_empty());
    assert_eq!(snapshot.attempts[0].state(), OperationState::EffectPossible);
}

#[test]
fn writer_contention_is_bounded_without_an_unbounded_queue() {
    const WRITERS: usize = 32;
    let fixture = Fixture::new();
    let journal = fixture.open();
    let specs = (0..WRITERS)
        .map(|index| fixture.spec(&format!("contention-{index}"), json!({"index": index})))
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(WRITERS));
    let journal_ref = &journal;

    let outcomes = thread::scope(|scope| {
        let handles = specs
            .iter()
            .map(|spec| {
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
                    journal_ref.persist_intent(spec, &initial)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert!(outcomes
        .iter()
        .all(|outcome| { matches!(outcome, Ok(_) | Err(JournalError::WriterBusy)) }));
    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot.operations.len() <= WRITERS);
}

#[test]
fn checkpoint_disk_full_is_reported_before_blob_publication() {
    let store = BlobStore::new();
    let task_id = TaskId::new();
    store.set_disk_full(true);
    assert!(matches!(
        store.store_blob_for_task(task_id, b"effect preimage"),
        Err(CheckpointError::DiskFull)
    ));
    assert!(store
        .get_blob(&compute_sha256(b"effect preimage"))
        .is_none());

    store.set_disk_full(false);
    let hash = store
        .store_blob_for_task(task_id, b"effect preimage")
        .unwrap();
    assert_eq!(
        store.get_blob(&hash).as_deref(),
        Some(b"effect preimage".as_slice())
    );
}

#[test]
#[ignore = "local reference measurement; run with --ignored --nocapture"]
fn benchmark_journal_overhead_reference_workload() {
    const SAMPLES: usize = 32;
    println!(
        "benchmark=os-boundary os={} arch={} profile={} fs=local-tempdir",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    let mut baseline = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let started = Instant::now();
        let bytes = serde_json::to_vec(&json!({"index": index, "kind": "read"})).unwrap();
        std::hint::black_box(bytes);
        baseline.push(started.elapsed());
    }
    report_measurement("baseline-encode", &mut baseline, SAMPLES);

    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut admitted = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let spec = fixture.spec(&format!("admit-{index}"), json!({"index": index}));
        let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
        let started = Instant::now();
        journal.persist_intent(&spec, &initial).unwrap();
        admitted.push(started.elapsed());
    }
    report_measurement("journaled-admission", &mut admitted, SAMPLES);

    let read_spec = fixture.spec("read-hot", json!({"kind": "read"}));
    let read_initial = OperationAttempt::new(read_spec.operation_id(), 1, owner()).unwrap();
    journal.persist_intent(&read_spec, &read_initial).unwrap();
    let mut reads = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        std::hint::black_box(journal.load_spec(read_spec.operation_id()).unwrap());
        reads.push(started.elapsed());
    }
    report_measurement("journaled-read", &mut reads, SAMPLES);

    let mut mutations = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let spec = fixture.spec(&format!("mutate-{index}"), json!({"kind": "mutation"}));
        let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
        let started = Instant::now();
        let admitted = journal.persist_intent(&spec, &initial).unwrap();
        let _ = journal
            .transition(
                admitted.owner(),
                admitted.attempt_id(),
                admitted.revision(),
                OperationEvent::Authorize(AuthorizationReceipt::new(
                    spec.intent_digest().unwrap(),
                    "benchmark-policy".to_owned(),
                    Timestamp::from_unix_millis(10),
                )),
                None,
                vec![],
            )
            .unwrap();
        mutations.push(started.elapsed());
    }
    report_measurement("journaled-mutation", &mut mutations, SAMPLES);

    let mut batch = Vec::with_capacity(SAMPLES);
    for group in 0..4 {
        let started = Instant::now();
        for item in 0..(SAMPLES / 4) {
            let index = group * (SAMPLES / 4) + item;
            let spec = fixture.spec(&format!("batch-{index}"), json!({"index": index}));
            let initial = OperationAttempt::new(spec.operation_id(), 1, owner()).unwrap();
            journal.persist_intent(&spec, &initial).unwrap();
        }
        batch.push(started.elapsed());
    }
    report_measurement("journaled-batch", &mut batch, 4);

    let mut worker = Vec::with_capacity(SAMPLES);
    thread::scope(|scope| {
        let handles = (0..SAMPLES)
            .map(|_| {
                scope.spawn(|| {
                    let started = Instant::now();
                    loop {
                        match journal.snapshot() {
                            Ok(snapshot) => {
                                std::hint::black_box(snapshot);
                                break;
                            }
                            Err(JournalError::WriterBusy) => thread::yield_now(),
                            Err(error) => panic!("worker snapshot failed: {error}"),
                        }
                    }
                    started.elapsed()
                })
            })
            .collect::<Vec<_>>();
        worker.extend(handles.into_iter().map(|handle| handle.join().unwrap()));
    });
    report_measurement("journaled-worker-read", &mut worker, SAMPLES);
}
