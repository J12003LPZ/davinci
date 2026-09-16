//! Authoritative task commits; runtime observer logging is not persistence.
//! Native expansion contract: no equivalent task journal in vendor/davinci.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::tasks::TaskLineage;
use super::{AgentId, RunId, TaskError, TaskId, TaskRecord};

pub(crate) const MAX_OPERATION_RECEIPTS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskOperationRequest {
    pub task_id: Option<TaskId>,
    pub run_id: RunId,
    pub actor: AgentId,
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<TaskCreateRequest>,
}

/// Caller-supplied creation fields; generated identity and timestamps are responses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskCreateRequest {
    pub title: String,
    pub description: Option<String>,
    pub dependencies: Vec<TaskId>,
    pub assigned_to: Option<AgentId>,
    pub parent_plan_step: Option<super::tasks::PlanStepRef>,
    #[serde(default)]
    pub decision_prerequisites: Vec<super::tasks::DecisionPrerequisiteRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusRequest {
    pub state: super::TaskState,
    pub generation: u64,
    pub result: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskOperationReceipt {
    pub operation_id: Uuid,
    pub request: TaskOperationRequest,
    pub response: TaskRecord,
}

const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

fn persistence(error: impl std::fmt::Display) -> TaskError {
    TaskError::Persistence(error.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskChange {
    pub prior_revision: Option<u64>,
    pub record: TaskRecord,
}

/// Trusted storage boundary, invoked under the registry's serialized write lane.
/// Implementations must not call user hooks or reenter the task registry.
pub(crate) trait TaskCommitSink: Send + Sync {
    fn commit(&self, changes: Vec<TaskChange>) -> Result<(), TaskError>;
    fn commit_operation(
        &self,
        _changes: Vec<TaskChange>,
        _receipt: TaskOperationReceipt,
    ) -> Result<(), TaskError> {
        Err(persistence("storage does not support operation receipts"))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    schema_version: u32,
    sequence: u64,
    operation_id: Uuid,
    run_id: RunId,
    changes: Vec<TaskChange>,
    // Retain the v2 wire field name for existing claim frames and checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim: Option<TaskOperationReceipt>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    commit: Commit,
    checksum: String,
}

/// New session-bound journals have one header before their unchanged v1-v4 frames.
/// Legacy openers reject this format rather than silently losing its binding.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBinding {
    schema_version: u32,
    session_key: String,
    run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migrated_tasks: Option<Vec<TaskRecord>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionHeader {
    task_journal: SessionBinding,
    checksum: String,
}

fn checksum(commit: &Commit) -> Result<String, TaskError> {
    Ok(format!("{:x}", Sha256::digest(encode_bounded(commit)?)))
}

fn decode_frame(bytes: &[u8], run_id: RunId, previous_sequence: u64) -> Result<Frame, TaskError> {
    let frame: Frame = serde_json::from_slice(bytes).map_err(persistence)?;
    if receipt_schema(frame.commit.claim.as_ref()) != Some(frame.commit.schema_version)
        || frame.commit.run_id != run_id
        || previous_sequence.checked_add(1) != Some(frame.commit.sequence)
        || checksum(&frame.commit)? != frame.checksum
    {
        return Err(persistence(
            "invalid task journal schema, lineage, sequence or checksum; recovery required",
        ));
    }
    Ok(frame)
}

fn receipt_schema(receipt: Option<&TaskOperationReceipt>) -> Option<u32> {
    match receipt.map(|r| (&r.request.create, &r.request.status, r.request.task_id)) {
        None => Some(1),
        Some((None, None, Some(_))) => Some(2),
        Some((None, Some(_), Some(_))) => Some(3),
        Some((Some(_), None, None)) => Some(4),
        _ => None,
    }
}

fn encode_bounded(value: &impl Serialize) -> Result<Vec<u8>, TaskError> {
    struct Buffer(Vec<u8>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) >= MAX_RECORD_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "task journal record capacity exceeded",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(persistence)?;
    Ok(buffer.0)
}

struct JournalState {
    file: File,
    sequence: u64,
    bytes: u64,
    poisoned: bool,
    #[cfg(test)]
    fault: Option<Fault>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum Fault {
    BeforeWrite,
    DuringWrite,
    AfterSync,
}

pub(crate) struct TaskJournal {
    pub(crate) run_id: RunId,
    pub(crate) lineage: TaskLineage,
    state: Mutex<JournalState>,
    pub(crate) restored_operations: HashMap<Uuid, TaskOperationReceipt>,
}

/// Lock the journal file itself, so path aliases cannot create a second writer.
/// The OS releases the lease even when process::exit bypasses destructors.
fn open_exclusive(path: &Path) -> Result<File, TaskError> {
    let mut options = OpenOptions::new();
    // Windows append-only access cannot truncate a quarantined torn tail.
    // The exclusive lease and commit mutex serialize explicit end seeks.
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(path).map_err(persistence)?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // Same platform locking contract as runtime/capacity.rs. A concurrent
        // fork can briefly retain this open-file description until exec closes
        // its CLOEXEC descriptor, so allow that handoff without weakening the
        // single-writer lease.
        extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        const HANDOFF_RETRIES: usize = 10;
        for attempt in 0..=HANDOFF_RETRIES {
            // SAFETY: file owns a live descriptor; flock does not retain pointers.
            if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock || attempt == HANDOFF_RETRIES {
                return Err(persistence(error));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[cfg(not(any(windows, unix)))]
    return Err(persistence(
        "task journal locking unsupported on this platform",
    ));
    Ok(file)
}

fn apply_changes(
    tasks: &mut HashMap<TaskId, TaskRecord>,
    changes: &[TaskChange],
    lineage: &TaskLineage,
) -> Result<(), TaskError> {
    if changes.is_empty() {
        return Err(persistence("empty task commit"));
    }
    let mut seen = HashSet::new();
    for change in changes {
        let record = &change.record;
        record.validate()?;
        if !seen.insert(record.id) || !lineage.contains(record.run_id) {
            return Err(persistence("duplicate task or mismatched run in commit"));
        }
        let previous = tasks.get(&record.id);
        if previous.map(|task| task.revision) != change.prior_revision
            || change.prior_revision.unwrap_or(0).checked_add(1) != Some(record.revision)
            || previous.is_some_and(|task| {
                task.created_at_ms != record.created_at_ms || task.run_id != record.run_id
            })
            || (previous.is_none() && record.run_id != lineage.primary)
        {
            return Err(persistence(
                "task commit revision or creation-time mismatch",
            ));
        }
    }
    for change in changes {
        tasks.insert(change.record.id, change.record.clone());
    }
    for task in tasks.values() {
        if task
            .dependencies
            .iter()
            .any(|id| *id == task.id || !tasks.contains_key(id))
        {
            return Err(persistence("task commit has unresolved dependencies"));
        }
    }
    Ok(())
}

fn migration_projection(
    binding: &SessionBinding,
) -> Result<(TaskLineage, HashMap<TaskId, TaskRecord>), TaskError> {
    let mut lineage = TaskLineage {
        primary: binding.run_id,
        origins: HashSet::new(),
    };
    let mut tasks = HashMap::new();
    for record in binding.migrated_tasks.iter().flatten() {
        record.validate()?;
        lineage.origins.insert(record.run_id);
        if tasks.insert(record.id, record.clone()).is_some() {
            return Err(persistence("duplicate task in migration snapshot"));
        }
    }
    for task in tasks.values() {
        let initial = super::tasks::initial_task_state(task, &tasks, Some(&lineage))?;
        // Preserve terminal history and Running records for host reconciliation,
        // but never import claimable authority over unfinished dependencies.
        if task.state == super::TaskState::Ready && initial != super::TaskState::Ready {
            return Err(persistence("migration snapshot has an invalid ready task"));
        }
    }
    Ok((lineage, tasks))
}

impl TaskJournal {
    pub(crate) fn open_session(
        path: &Path,
        session_key: &str,
    ) -> Result<(Self, HashMap<TaskId, TaskRecord>), TaskError> {
        Self::open_session_with_legacy(path, session_key, Vec::new())
    }

    pub(crate) fn open_session_with_legacy(
        path: &Path,
        session_key: &str,
        records: Vec<TaskRecord>,
    ) -> Result<(Self, HashMap<TaskId, TaskRecord>), TaskError> {
        if session_key.trim().is_empty() || session_key.len() > 4096 {
            return Err(persistence("invalid task journal session key"));
        }
        let mut file = open_exclusive(path)?;
        let (header, bytes) = if file.metadata().map_err(persistence)?.len() == 0 {
            let binding = SessionBinding {
                schema_version: if records.is_empty() { 1 } else { 2 },
                session_key: session_key.to_owned(),
                run_id: RunId::new(),
                migrated_tasks: (!records.is_empty()).then_some(records),
            };
            migration_projection(&binding)?;
            let header = SessionHeader {
                checksum: format!("{:x}", Sha256::digest(encode_bounded(&binding)?)),
                task_journal: binding,
            };
            let mut bytes = encode_bounded(&header)?;
            bytes.push(b'\n');
            file.write_all(&bytes).map_err(persistence)?;
            (header, bytes.len() as u64)
        } else {
            let mut bytes = Vec::new();
            BufReader::new(&mut file)
                .take(MAX_RECORD_BYTES as u64 + 1)
                .read_until(b'\n', &mut bytes)
                .map_err(persistence)?;
            if bytes.len() > MAX_RECORD_BYTES || bytes.last() != Some(&b'\n') {
                return Err(persistence(
                    "invalid task journal header; recovery required",
                ));
            }
            let header: SessionHeader = serde_json::from_slice(&bytes).map_err(|_| {
                persistence(
                    "missing or invalid task journal binding; migration or recovery required",
                )
            })?;
            if !matches!(
                (
                    header.task_journal.schema_version,
                    header.task_journal.migrated_tasks.as_ref()
                ),
                (1, None) | (2, Some(_))
            ) || header.task_journal.session_key != session_key
                || header.checksum
                    != format!(
                        "{:x}",
                        Sha256::digest(encode_bounded(&header.task_journal)?)
                    )
            {
                return Err(persistence(
                    "invalid task journal binding; recovery required",
                ));
            }
            (header, bytes.len() as u64)
        };
        // BufReader may read ahead; replay must start at the first commit exactly.
        file.seek(SeekFrom::Start(bytes)).map_err(persistence)?;
        let (lineage, tasks) = migration_projection(&header.task_journal)?;
        Self::open_locked(path, file, lineage, tasks, bytes)
    }

    pub(crate) fn open(
        path: &Path,
        run_id: RunId,
    ) -> Result<(Self, HashMap<TaskId, TaskRecord>), TaskError> {
        Self::open_locked(
            path,
            open_exclusive(path)?,
            TaskLineage {
                primary: run_id,
                origins: HashSet::new(),
            },
            HashMap::new(),
            0,
        )
    }

    fn open_locked(
        path: &Path,
        mut file: File,
        lineage: TaskLineage,
        mut tasks: HashMap<TaskId, TaskRecord>,
        initial_bytes: u64,
    ) -> Result<(Self, HashMap<TaskId, TaskRecord>), TaskError> {
        if file.metadata().map_err(persistence)?.len() > MAX_JOURNAL_BYTES {
            return Err(persistence(
                "task journal exceeds capacity; recovery required",
            ));
        }
        let run_id = lineage.primary;
        let mut sequence = 0u64;
        let mut bytes = initial_bytes;
        let mut operations = HashSet::new();
        let mut claims = HashMap::new();
        let mut tail = None;
        {
            let mut reader = BufReader::new(&mut file);
            loop {
                let mut line = Vec::new();
                reader
                    .by_ref()
                    .take(MAX_RECORD_BYTES as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .map_err(persistence)?;
                if line.is_empty() {
                    break;
                }
                if line.len() > MAX_RECORD_BYTES {
                    return Err(persistence("task journal record exceeds capacity"));
                }
                if line.last() != Some(&b'\n') {
                    // A missing terminator does not authorize discarding a
                    // complete record from an unknown schema or another run.
                    if serde_json::from_slice::<serde_json::Value>(&line).is_ok() {
                        decode_frame(&line, run_id, sequence)?;
                    }
                    tail = Some(line);
                    break;
                }
                let frame = decode_frame(&line, run_id, sequence)?;
                if !operations.insert(frame.commit.operation_id) {
                    return Err(persistence(
                        "duplicate task journal operation; recovery required",
                    ));
                }
                if let Some(receipt) = frame
                    .commit
                    .claim
                    .as_ref()
                    .filter(|r| r.request.create.is_some())
                {
                    if super::tasks::initial_task_state(&receipt.response, &tasks, Some(&lineage))?
                        != receipt.response.state
                    {
                        return Err(persistence("invalid creation dependency state"));
                    }
                }
                apply_changes(&mut tasks, &frame.commit.changes, &lineage)?;
                if let Some(receipt) = frame.commit.claim {
                    validate_receipt(&receipt, &frame.commit.changes, &lineage)?;
                    if claims.len() >= MAX_OPERATION_RECEIPTS
                        || claims.insert(receipt.operation_id, receipt).is_some()
                    {
                        return Err(persistence("duplicate or excess claim receipt"));
                    }
                }
                sequence = frame.commit.sequence;
                bytes += line.len() as u64;
            }
        }
        if let Some(tail) = tail {
            // Preserve every torn byte before truncating. Never quarantine a
            // complete but corrupt record or an unknown schema as a mere tail.
            let tail_path = path.with_extension(format!("quarantine-{}", Uuid::new_v4()));
            let mut quarantine = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(tail_path)
                .map_err(persistence)?;
            quarantine.write_all(&tail).map_err(persistence)?;
            quarantine.sync_all().map_err(persistence)?;
            sync_parent(path)?;
            file.set_len(bytes)
                .map_err(|error| persistence(format!("truncate quarantined task tail: {error}")))?;
        }
        file.sync_all().map_err(persistence)?;
        sync_parent(path)?;
        Ok((
            Self {
                run_id,
                lineage,
                restored_operations: claims,
                state: Mutex::new(JournalState {
                    file,
                    sequence,
                    bytes,
                    poisoned: false,
                    #[cfg(test)]
                    fault: None,
                }),
            },
            tasks,
        ))
    }
}

fn sync_parent(path: &Path) -> Result<(), TaskError> {
    #[cfg(unix)]
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .and_then(|directory| directory.sync_all())
    .map_err(persistence)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

impl TaskCommitSink for TaskJournal {
    fn commit(&self, changes: Vec<TaskChange>) -> Result<(), TaskError> {
        self.commit_inner(changes, None)
    }

    fn commit_operation(
        &self,
        changes: Vec<TaskChange>,
        receipt: TaskOperationReceipt,
    ) -> Result<(), TaskError> {
        validate_receipt(&receipt, &changes, &self.lineage)?;
        self.commit_inner(changes, Some(receipt))
    }
}

fn validate_receipt(
    receipt: &TaskOperationReceipt,
    changes: &[TaskChange],
    lineage: &TaskLineage,
) -> Result<(), TaskError> {
    let run_id = lineage.primary;
    let request = &receipt.request;
    if let Some(create) = &request.create {
        let response = &receipt.response;
        if receipt_schema(Some(receipt)) != Some(4)
            || changes.len() != 1
            || changes[0].prior_revision.is_some()
            || changes[0].record != *response
            || request.run_id != run_id
            || response.run_id != run_id
            || request.expected_revision != 0
            || response.revision != 1
            || response.title != create.title
            || response.description != create.description
            || response.dependencies != create.dependencies
            || response.assigned_to != create.assigned_to
            || create
                .assigned_to
                .is_some_and(|actor| actor != request.actor)
            || response.parent_plan_step != create.parent_plan_step
            || response.decision_prerequisites != create.decision_prerequisites
            || response.contract_digest != create.contract_digest
            || response.owner_generation != u64::from(create.assigned_to.is_some())
            || response.result.is_some()
            || !response.evidence_refs.is_empty()
            || response.blocked_reasons.len() > 64
            || response.attempt != 0
            || !matches!(
                response.state,
                super::TaskState::Ready | super::TaskState::Pending | super::TaskState::Blocked
            )
        {
            return Err(persistence("invalid creation receipt"));
        }
        return Ok(());
    }
    let response_change = changes
        .iter()
        .find(|change| change.record.id == receipt.response.id);
    let expected_state = request
        .status
        .as_ref()
        .map_or(super::TaskState::Running, |status| status.state);
    if (request.status.is_none() && changes.len() != 1)
        || request.run_id != run_id
        || request.task_id != Some(receipt.response.id)
        || !lineage.contains(receipt.response.run_id)
        || receipt.response.assigned_to != Some(request.actor)
        || receipt.response.state != expected_state
        || request.expected_revision.checked_add(1) != Some(receipt.response.revision)
        || !response_change.is_some_and(|change| {
            change.prior_revision == Some(request.expected_revision)
                && change.record == receipt.response
        })
        || request.status.as_ref().is_some_and(|completion| {
            !matches!(
                completion.state,
                super::TaskState::Completed
                    | super::TaskState::Failed
                    | super::TaskState::Cancelled
            ) || completion.generation != receipt.response.owner_generation
                || completion.result != receipt.response.result
        })
    {
        return Err(persistence("invalid operation receipt"));
    }
    Ok(())
}

impl TaskJournal {
    fn commit_inner(
        &self,
        changes: Vec<TaskChange>,
        claim: Option<TaskOperationReceipt>,
    ) -> Result<(), TaskError> {
        if changes.is_empty()
            || changes.iter().any(|change| {
                !self.lineage.contains(change.record.run_id)
                    || (change.prior_revision.is_none() && change.record.run_id != self.run_id)
                    || change.prior_revision.unwrap_or(0).checked_add(1)
                        != Some(change.record.revision)
            })
        {
            return Err(persistence("invalid task journal lineage or revision"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| persistence("task journal lock poisoned"))?;
        if state.poisoned {
            return Err(persistence(
                "task journal requires reopen after uncertain write",
            ));
        }
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| persistence("task commit sequence exhausted"))?;
        let commit = Commit {
            schema_version: receipt_schema(claim.as_ref())
                .ok_or_else(|| persistence("invalid operation kind"))?,
            sequence,
            operation_id: Uuid::new_v4(),
            run_id: self.run_id,
            changes,
            claim,
        };
        let frame = Frame {
            checksum: checksum(&commit)?,
            commit,
        };
        let mut bytes = encode_bounded(&frame)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_RECORD_BYTES
            || state.bytes.saturating_add(bytes.len() as u64) > MAX_JOURNAL_BYTES
        {
            return Err(persistence("task journal capacity exceeded"));
        }
        #[cfg(test)]
        let fault = state.fault.take();
        #[cfg(test)]
        if matches!(fault, Some(Fault::BeforeWrite)) {
            return Err(persistence("fixture: before write"));
        }
        // Any write/sync failure is uncertain. No later command may append
        // until replay repairs a torn tail or reports a corrupt committed frame.
        state.poisoned = true;
        state.file.seek(SeekFrom::End(0)).map_err(persistence)?;
        #[cfg(test)]
        if matches!(fault, Some(Fault::DuringWrite)) {
            state
                .file
                .write_all(&bytes[..bytes.len() / 2])
                .map_err(persistence)?;
            return Err(persistence("fixture: partial write"));
        }
        state.file.write_all(&bytes).map_err(persistence)?;
        state.file.sync_all().map_err(persistence)?;
        #[cfg(test)]
        if matches!(fault, Some(Fault::AfterSync)) {
            return Err(persistence("fixture: after sync, before publish"));
        }
        state.sequence = sequence;
        state.bytes += bytes.len() as u64;
        state.poisoned = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn f03_migration_rejects_ready_tasks_with_unfinished_dependencies() {
        for state in [
            TaskState::Pending,
            TaskState::Running,
            TaskState::Failed,
            TaskState::Blocked,
            TaskState::Cancelled,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let mut dependency = TaskRecord::new(RunId::new(), "dependency");
            dependency.state = state;
            let mut dependent = TaskRecord::new(RunId::new(), "prematurely ready");
            dependent.dependencies.push(dependency.id);
            let binding = SessionBinding {
                schema_version: 2,
                session_key: "session".into(),
                run_id: RunId::new(),
                migrated_tasks: Some(vec![dependency, dependent]),
            };
            let header = SessionHeader {
                checksum: format!("{:x}", Sha256::digest(encode_bounded(&binding).unwrap())),
                task_journal: binding,
            };
            let mut bytes = encode_bounded(&header).unwrap();
            bytes.push(b'\n');
            std::fs::write(&path, &bytes).unwrap();
            assert!(
                TaskRegistry::open_session_durable(&path, "session").is_err(),
                "{state:?}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn f03_migration_snapshot_and_origin_corruption_fail_closed() {
        for kind in ["checksum", "schema", "duplicate", "cycle", "origin"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let original = TaskRecord::new(RunId::new(), "original");
            let (registry, run) = TaskRegistry::open_session_durable_with_legacy(
                &path,
                "session",
                vec![original.clone()],
            )
            .unwrap();
            registry
                .claim_task(original.id, run, AgentId::new(), 0)
                .unwrap();
            drop(registry);
            let bytes = std::fs::read(&path).unwrap();
            let lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
            let mut header: SessionHeader = serde_json::from_slice(lines[0]).unwrap();
            let mut frame: Frame = serde_json::from_slice(lines[1]).unwrap();
            match kind {
                "checksum" => header.checksum = "damaged".into(),
                "schema" => header.task_journal.schema_version = 1,
                "duplicate" => header
                    .task_journal
                    .migrated_tasks
                    .as_mut()
                    .unwrap()
                    .push(original.clone()),
                "cycle" => header.task_journal.migrated_tasks.as_mut().unwrap()[0]
                    .dependencies
                    .push(original.id),
                "origin" => {
                    // Even a permitted session run cannot replace original provenance.
                    frame.commit.changes[0].record.run_id = run;
                    frame.checksum = checksum(&frame.commit).unwrap();
                }
                _ => unreachable!(),
            }
            if kind != "checksum" {
                header.checksum = format!(
                    "{:x}",
                    Sha256::digest(encode_bounded(&header.task_journal).unwrap())
                );
            }
            let mut damaged = serde_json::to_vec(&header).unwrap();
            damaged.push(b'\n');
            damaged.extend(serde_json::to_vec(&frame).unwrap());
            damaged.push(b'\n');
            std::fs::write(&path, &damaged).unwrap();
            assert!(
                TaskRegistry::open_session_durable(&path, "session").is_err(),
                "{kind}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), damaged);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn f03_session_journal_selects_lineage_in_fresh_process() {
        const FIXTURE: &str = "PI_F03_SESSION_JOURNAL_FIXTURE";
        const TEST: &str =
            "runtime::task_store::tests::f03_session_journal_selects_lineage_in_fresh_process";
        if let Ok(input) = std::env::var(FIXTURE) {
            let input: serde_json::Value = serde_json::from_str(&input).unwrap();
            let path = Path::new(input["path"].as_str().unwrap());
            let (registry, run) =
                TaskRegistry::open_session_durable(path, "session-source").unwrap();
            match input["phase"].as_u64().unwrap() {
                0 => std::fs::write(path.with_extension("run"), run.to_string()).unwrap(),
                1 => {
                    assert_eq!(
                        run.to_string(),
                        std::fs::read_to_string(path.with_extension("run")).unwrap()
                    );
                    registry
                        .create_task(TaskRecord::new(run, "fresh process task"))
                        .unwrap();
                }
                2 => {
                    assert_eq!(
                        run.to_string(),
                        std::fs::read_to_string(path.with_extension("run")).unwrap()
                    );
                    let tasks = registry.list_tasks(Some(run));
                    assert_eq!(tasks.len(), 1);
                    assert_eq!(tasks[0].title, "fresh process task");
                }
                _ => panic!("unknown fixture phase"),
            }
            // Exercise release of the file lease without Rust destructors.
            std::process::exit(0);
        }
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        for phase in 0..3 {
            let mut child = ChildGuard(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST])
                    .env(
                        FIXTURE,
                        serde_json::json!({"path":path,"phase":phase}).to_string(),
                    )
                    .env("PI_OFFLINE", "1")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let status = (0..2000)
                .find_map(|_| {
                    let status = child.0.try_wait().unwrap();
                    if status.is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    status
                })
                .expect("session journal fixture did not finish");
            assert!(status.success(), "phase {phase}");
        }
    }

    #[test]
    fn f03_session_journal_replays_commits_and_quarantines_only_task_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let (registry, run) = TaskRegistry::open_session_durable(&path, "source/session").unwrap();
        let id = registry
            .create_task(TaskRecord::new(run, "persisted"))
            .unwrap();
        registry.cancel_task(id).unwrap();
        let expected = registry.get_task(&id).unwrap();
        drop(registry);
        let original = std::fs::read(&path).unwrap();
        assert!(TaskRegistry::open_durable(&path, run).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{torn")
            .unwrap();
        let (registry, restored_run) =
            TaskRegistry::open_session_durable(&path, "source/session").unwrap();
        assert_eq!(restored_run, run);
        assert_eq!(registry.get_task(&id).unwrap(), expected);
        drop(registry);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let quarantines: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p != &path)
            .collect();
        assert_eq!(quarantines.len(), 1);
        assert_eq!(std::fs::read(&quarantines[0]).unwrap(), b"{torn");
    }

    #[test]
    fn f03_session_journal_rejects_damaged_or_legacy_binding_without_writes() {
        for kind in [
            "torn",
            "unterminated",
            "schema",
            "checksum",
            "legacy",
            "oversized",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            if kind == "legacy" {
                let run = RunId::new();
                TaskRegistry::open_durable(&path, run)
                    .unwrap()
                    .create_task(TaskRecord::new(run, "legacy"))
                    .unwrap();
            } else {
                drop(TaskRegistry::open_session_durable(&path, "source/session").unwrap());
                let mut bytes = std::fs::read(&path).unwrap();
                match kind {
                    "torn" => bytes.truncate(10),
                    "unterminated" => {
                        bytes.pop();
                    }
                    "oversized" => bytes = vec![b'x'; MAX_RECORD_BYTES + 1],
                    _ => {
                        let mut header: SessionHeader = serde_json::from_slice(&bytes).unwrap();
                        if kind == "schema" {
                            header.task_journal.schema_version = 2;
                            header.checksum = format!(
                                "{:x}",
                                Sha256::digest(encode_bounded(&header.task_journal).unwrap())
                            );
                        } else {
                            header.checksum = "bad".into();
                        }
                        bytes = serde_json::to_vec(&header).unwrap();
                        bytes.push(b'\n');
                    }
                }
                std::fs::write(&path, bytes).unwrap();
            }
            let original = std::fs::read(&path).unwrap();
            assert!(
                TaskRegistry::open_session_durable(&path, "source/session").is_err(),
                "{kind}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), original, "{kind}");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "{kind}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.jsonl");
        for key in ["".to_owned(), " ".to_owned(), "x".repeat(4097)] {
            assert!(TaskRegistry::open_session_durable(&path, &key).is_err());
            assert!(!path.exists());
        }
    }

    #[test]
    fn f03_session_journal_retains_empty_lineage_and_checks_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = {
            let (journal, tasks) = TaskJournal::open_session(&path, "session-a").unwrap();
            assert!(tasks.is_empty());
            journal.run_id
        };
        let original = std::fs::read(&path).unwrap();
        let (journal, tasks) = TaskJournal::open_session(&path, "session-a").unwrap();
        assert_eq!(journal.run_id, run);
        assert!(tasks.is_empty());
        assert!(TaskJournal::open_session(&path, "session-a").is_err());
        drop(journal);
        assert!(TaskJournal::open_session(&path, "session-b").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn f03_fresh_process_recovers_committed_receipt_after_writer_kill() {
        use std::process::{Command, Stdio};
        use std::str::FromStr;
        const CHILD: &str = "PI_F03_JOURNAL_PROCESS_FIXTURE";
        const TEST: &str = "runtime::task_store::tests::f03_fresh_process_recovers_committed_receipt_after_writer_kill";
        if let Ok(fixture) = std::env::var(CHILD) {
            let input: serde_json::Value = serde_json::from_str(&fixture).unwrap();
            let path = Path::new(input["path"].as_str().unwrap());
            let run = RunId::from_str(input["run"].as_str().unwrap()).unwrap();
            let actor = AgentId::from_str(input["actor"].as_str().unwrap()).unwrap();
            let operation = Uuid::parse_str(input["operation"].as_str().unwrap()).unwrap();
            match input["mode"].as_str().unwrap() {
                "writer" => {
                    let (journal, _) = TaskJournal::open(path, run).unwrap();
                    let mut record = TaskRecord::new(run, "fresh process");
                    record.revision = 1;
                    let receipt = TaskOperationReceipt {
                        operation_id: operation,
                        request: TaskOperationRequest {
                            task_id: None,
                            run_id: run,
                            actor,
                            expected_revision: 0,
                            status: None,
                            create: Some(TaskCreateRequest {
                                title: record.title.clone(),
                                ..TaskCreateRequest::default()
                            }),
                        },
                        response: record.clone(),
                    };
                    journal.state.lock().unwrap().fault = Some(Fault::AfterSync);
                    assert!(journal
                        .commit_operation(
                            vec![TaskChange {
                                prior_revision: None,
                                record: record.clone()
                            }],
                            receipt
                        )
                        .is_err());
                    // Acknowledge sync, retain the OS lease, and let the parent kill
                    // this exact child. No destructor or graceful close runs.
                    std::fs::write(
                        path.with_extension("ready.tmp"),
                        serde_json::to_vec(&record).unwrap(),
                    )
                    .unwrap();
                    std::fs::rename(
                        path.with_extension("ready.tmp"),
                        path.with_extension("ready"),
                    )
                    .unwrap();
                    loop {
                        std::thread::park();
                    }
                }
                "contender" => assert!(TaskRegistry::open_durable(path, run).is_err()),
                "resume" => {
                    let before = std::fs::read(path).unwrap();
                    let registry = TaskRegistry::open_durable(path, run).unwrap();
                    let expected: TaskRecord = serde_json::from_slice(
                        &std::fs::read(path.with_extension("ready")).unwrap(),
                    )
                    .unwrap();
                    let restored = registry
                        .create_command(
                            TaskCreateRequest {
                                title: "fresh process".into(),
                                ..TaskCreateRequest::default()
                            },
                            run,
                            actor,
                            Some(operation),
                        )
                        .unwrap();
                    assert_eq!(restored, expected);
                    assert_eq!(registry.list_tasks(Some(run)), vec![expected]);
                    drop(registry);
                    assert_eq!(std::fs::read(path).unwrap(), before);
                }
                other => panic!("Unknown fixture mode: {other}"),
            }
            return;
        }

        // RAII cleanup applies even if a readiness or contender assertion fails.
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let executable = std::env::current_exe().unwrap();
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = Uuid::new_v4();
        let spawn = |mode: &str| {
            let fixture = serde_json::json!({"mode": mode, "path": path, "run": run, "actor": actor, "operation": operation});
            ChildGuard(
                Command::new(&executable)
                    .args(["--exact", TEST, "--nocapture"])
                    .env(CHILD, fixture.to_string())
                    .env("PI_OFFLINE", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap(),
            )
        };
        let wait = |child: &mut ChildGuard| {
            for _ in 0..2000 {
                if let Some(status) = child.0.try_wait().unwrap() {
                    return status;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("Fixture child did not finish");
        };
        let mut writer = spawn("writer");
        for _ in 0..2000 {
            if path.with_extension("ready").is_file() {
                break;
            }
            assert!(
                writer.0.try_wait().unwrap().is_none(),
                "Writer exited before sync acknowledgement"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            path.with_extension("ready").is_file(),
            "Writer did not acknowledge sync"
        );
        assert!(wait(&mut spawn("contender")).success());
        writer.0.kill().unwrap();
        assert!(!writer.0.wait().unwrap().success());
        assert!(wait(&mut spawn("resume")).success());
    }

    #[test]
    fn f03_creation_replay_rejects_impossible_dependencies_and_state() {
        for duplicate in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let run = RunId::new();
            {
                let registry = TaskRegistry::open_durable(&path, run).unwrap();
                let dependency = registry
                    .create_task(TaskRecord::new(run, "unfinished"))
                    .unwrap();
                registry
                    .create_command(
                        TaskCreateRequest {
                            title: "pending".into(),
                            dependencies: vec![dependency],
                            ..TaskCreateRequest::default()
                        },
                        run,
                        AgentId::new(),
                        Some(Uuid::new_v4()),
                    )
                    .unwrap();
            }
            let mut frames: Vec<Frame> = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let frame = frames.last_mut().unwrap();
            let receipt = frame.commit.claim.as_mut().unwrap();
            if duplicate {
                let dependency = receipt.response.dependencies[0];
                receipt.response.dependencies.push(dependency);
                receipt
                    .request
                    .create
                    .as_mut()
                    .unwrap()
                    .dependencies
                    .push(dependency);
            } else {
                receipt.response.state = super::super::TaskState::Ready;
            }
            frame.commit.changes[0].record = receipt.response.clone();
            frame.checksum = checksum(&frame.commit).unwrap();
            let bytes: Vec<u8> = frames
                .iter()
                .flat_map(|frame| {
                    let mut line = serde_json::to_vec(frame).unwrap();
                    line.push(b'\n');
                    line
                })
                .collect();
            std::fs::write(&path, &bytes).unwrap();
            assert!(TaskRegistry::open_durable(&path, run).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn f03_creation_replays_after_sync_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = Uuid::new_v4();
        let input = TaskCreateRequest {
            title: "uncertain creation".into(),
            assigned_to: Some(actor),
            ..TaskCreateRequest::default()
        };
        let mut response = TaskRecord::new(run, &input.title).with_assigned(actor);
        response.revision = 1;
        response.owner_generation = 1;
        let (journal, _) = TaskJournal::open(&path, run).unwrap();
        journal.state.lock().unwrap().fault = Some(Fault::AfterSync);
        let receipt = TaskOperationReceipt {
            operation_id: operation,
            request: TaskOperationRequest {
                task_id: None,
                run_id: run,
                actor,
                expected_revision: 0,
                status: None,
                create: Some(input.clone()),
            },
            response: response.clone(),
        };
        assert!(journal
            .commit_operation(
                vec![TaskChange {
                    prior_revision: None,
                    record: response.clone()
                }],
                receipt
            )
            .is_err());
        drop(journal);
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert_eq!(
            registry
                .create_command(input, run, actor, Some(operation))
                .unwrap(),
            response
        );
        assert_eq!(registry.list_tasks(None), vec![response]);
    }

    #[test]
    fn f03_claim_receipt_recovers_after_sync_before_publish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let actor = AgentId::new();
        let id = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            registry
                .create_task(TaskRecord::new(run, "uncertain"))
                .unwrap()
        };
        let (journal, tasks) = TaskJournal::open(&path, run).unwrap();
        let mut response = tasks[&id].clone();
        response.revision += 1;
        response.owner_generation += 1;
        response.assigned_to = Some(actor);
        response.state = TaskState::Running;
        let receipt = TaskOperationReceipt {
            operation_id: Uuid::new_v4(),
            request: TaskOperationRequest {
                status: None,
                create: None,
                task_id: Some(id),
                run_id: run,
                actor,
                expected_revision: 1,
            },
            response: response.clone(),
        };
        journal.state.lock().unwrap().fault = Some(Fault::AfterSync);
        assert!(journal
            .commit_operation(
                vec![TaskChange {
                    prior_revision: Some(1),
                    record: response.clone()
                }],
                receipt.clone()
            )
            .is_err());
        drop(journal);
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert_eq!(
            registry
                .claim_operation(id, run, actor, 1, receipt.operation_id)
                .unwrap(),
            response
        );
        drop(registry);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 2);
    }

    use super::*;
    use crate::runtime::{RunId, TaskRecord, TaskRegistry, TaskState};

    #[test]
    fn f03_journal_bounds_reject_before_write_without_poisoning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let id = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            let id = registry
                .create_task(TaskRecord::new(run, "bounded"))
                .unwrap();
            let original = registry.get_task(&id).unwrap();
            assert!(registry
                .complete_task(id, Some("x".repeat(MAX_RECORD_BYTES)))
                .is_err());
            assert_eq!(registry.get_task(&id), Some(original));
            registry
                .complete_task(id, Some("small result".into()))
                .unwrap();
            id
        };
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert_eq!(
            TaskRegistry::open_durable(&path, run)
                .unwrap()
                .get_task(&id)
                .unwrap()
                .result
                .as_deref(),
            Some("small result")
        );
    }

    #[test]
    fn f03_observer_panic_cannot_undo_durable_commit() {
        use crate::runtime::{
            RuntimeBus, RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope, RuntimeSubscriber,
        };
        struct PanicAfterCommit;
        impl RuntimeSubscriber for PanicAfterCommit {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCompleted { .. }) {
                    panic!("fixture observer failure");
                }
                RuntimeDecision::Continue
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let id = {
            let bus = RuntimeBus::new();
            bus.subscribe(std::sync::Arc::new(PanicAfterCommit));
            let registry = TaskRegistry::open_durable(&path, run)
                .unwrap()
                .with_observers(bus);
            let id = registry
                .create_task(TaskRecord::new(run, "observer"))
                .unwrap();
            registry.complete_task(id, Some("durable".into())).unwrap();
            assert_eq!(registry.get_task(&id).unwrap().state, TaskState::Completed);
            id
        };
        assert_eq!(
            TaskRegistry::open_durable(&path, run)
                .unwrap()
                .get_task(&id)
                .unwrap()
                .state,
            TaskState::Completed
        );
    }

    #[test]
    fn f03_journal_recovers_before_during_and_after_write_failures() {
        for fault in [Fault::BeforeWrite, Fault::DuringWrite, Fault::AfterSync] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let run = RunId::new();
            let (journal, _) = TaskJournal::open(&path, run).unwrap();
            let mut record = TaskRecord::new(run, "crash boundary");
            record.revision = 1;
            let change = TaskChange {
                prior_revision: None,
                record: record.clone(),
            };
            journal.state.lock().unwrap().fault = Some(fault);
            assert!(journal.commit(vec![change.clone()]).is_err());
            if !matches!(fault, Fault::BeforeWrite) {
                assert!(
                    journal.commit(vec![change]).is_err(),
                    "uncertain writer must fail closed"
                );
            }
            drop(journal);
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            if matches!(fault, Fault::AfterSync) {
                assert_eq!(registry.get_task(&record.id), Some(record));
            } else {
                assert!(registry.list_tasks(None).is_empty());
            }
        }
    }

    #[test]
    fn f03_durable_registry_replays_results_and_cascades() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let (parent, child, expected) = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            let parent = registry
                .create_task(TaskRecord::new(run, "parent"))
                .unwrap();
            let child = registry
                .create_task(TaskRecord::new(run, "child").with_dependencies(vec![parent]))
                .unwrap();
            registry
                .assign_task(parent, crate::runtime::AgentId::new())
                .unwrap();
            registry
                .complete_task(parent, Some("exact result".into()))
                .unwrap();
            let expected = registry.get_task(&parent).unwrap();
            assert_eq!(registry.get_task(&child).unwrap().state, TaskState::Ready);
            (parent, child, expected)
        };
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert_eq!(registry.get_task(&parent).unwrap(), expected);
        assert_eq!(registry.get_task(&child).unwrap().state, TaskState::Ready);
        assert_eq!(registry.get_task(&child).unwrap().revision, 2);
    }

    #[test]
    fn f03_journal_denies_second_writer_and_releases_on_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert!(TaskRegistry::open_durable(&path, run).is_err());
        drop(registry);
        assert!(TaskRegistry::open_durable(&path, run).is_ok());
    }

    #[test]
    fn f03_durable_failure_and_cancellation_replay_transitive_blocks() {
        for cancel in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let run = RunId::new();
            let expected = {
                let registry = TaskRegistry::open_durable(&path, run).unwrap();
                let root = registry.create_task(TaskRecord::new(run, "root")).unwrap();
                let child = registry
                    .create_task(TaskRecord::new(run, "child").with_dependencies(vec![root]))
                    .unwrap();
                let leaf = registry
                    .create_task(TaskRecord::new(run, "leaf").with_dependencies(vec![child]))
                    .unwrap();
                if cancel {
                    registry.cancel_task(root).unwrap();
                } else {
                    registry
                        .fail_task(root, Some("specific failure".into()))
                        .unwrap();
                }
                assert_eq!(registry.get_task(&child).unwrap().state, TaskState::Blocked);
                assert_eq!(registry.get_task(&leaf).unwrap().state, TaskState::Blocked);
                registry
                    .list_tasks(None)
                    .into_iter()
                    .map(|task| (task.id, task))
                    .collect::<HashMap<_, _>>()
            };
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            let restored: HashMap<_, _> = registry
                .list_tasks(None)
                .into_iter()
                .map(|task| (task.id, task))
                .collect();
            assert_eq!(restored, expected);
        }
    }

    #[test]
    fn f03_torn_tail_is_preserved_before_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let id = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            registry.create_task(TaskRecord::new(run, "kept")).unwrap()
        };
        let valid = std::fs::read(&path).unwrap();
        let torn = b"{\"commit\":\"torn";
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(torn)
            .unwrap();
        {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            assert_eq!(registry.get_task(&id).unwrap().title, "kept");
            registry.cancel_task(id).unwrap();
        }
        let quarantines: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|p| p != &path)
            .collect();
        assert_eq!(quarantines.len(), 1);
        assert_eq!(std::fs::read(&quarantines[0]).unwrap(), torn);
        assert!(std::fs::read(&path).unwrap().starts_with(&valid));
        assert_eq!(
            TaskRegistry::open_durable(&path, run)
                .unwrap()
                .get_task(&id)
                .unwrap()
                .state,
            TaskState::Cancelled
        );
    }

    #[test]
    fn f03_corrupt_committed_records_fail_closed_without_rewriting() {
        for corruption in ["checksum", "schema", "duplicate", "run"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let run = RunId::new();
            {
                let registry = TaskRegistry::open_durable(&path, run).unwrap();
                let id = registry.create_task(TaskRecord::new(run, "kept")).unwrap();
                registry.cancel_task(id).unwrap();
            }
            let original = std::fs::read_to_string(&path).unwrap();
            let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
            if corruption == "duplicate" {
                lines.insert(1, lines[0].clone());
            } else {
                let mut frame: Frame = serde_json::from_str(&lines[0]).unwrap();
                match corruption {
                    "checksum" => frame.checksum = "incorrect".into(),
                    "schema" => {
                        frame.commit.schema_version = 2;
                        frame.checksum = checksum(&frame.commit).unwrap();
                    }
                    "run" => {
                        frame.commit.run_id = RunId::new();
                        frame.checksum = checksum(&frame.commit).unwrap();
                    }
                    _ => unreachable!(),
                }
                lines[0] = serde_json::to_string(&frame).unwrap();
            }
            let corrupted = lines.join("\n") + "\n";
            std::fs::write(&path, &corrupted).unwrap();
            assert!(
                TaskRegistry::open_durable(&path, run).is_err(),
                "{corruption}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupted);
        }
    }

    #[test]
    fn f03_unknown_schema_without_newline_is_not_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            registry
                .create_task(TaskRecord::new(run, "future schema"))
                .unwrap();
        }
        let mut frame: Frame = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        frame.commit.schema_version = 2;
        frame.checksum = checksum(&frame.commit).unwrap();
        let bytes = serde_json::to_vec(&frame).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(TaskRegistry::open_durable(&path, run).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn f03_durable_registry_rejects_observer_overwrite_and_wrong_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let registry = TaskRegistry::open_durable(path, run).unwrap();
        assert!(registry.rehydrate_from_events(&[]).is_err());
        assert!(registry
            .create_task(TaskRecord::new(RunId::new(), "wrong run"))
            .is_err());
        assert!(registry.list_tasks(None).is_empty());
    }
}
