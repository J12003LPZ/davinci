//! Durable session activation shared by the agent and product host.
//! Session loading parallels vendor/davinci/packages/coding-agent/src/core/agent-session.ts;
//! authoritative task recovery is a native runtime extension.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{
    RuntimeDecision, RuntimeEventEnvelope, RuntimeHandle, RuntimeSubscriber, TaskRegistry,
    TaskState,
};
use crate::runtime::operations::{digest_bytes, LegacyObservation, LegacySourceKind};

const WORKER_IDENTITY: &str = "worker_runtime_identity";

/// Cold restore only: callers must reuse an existing live session runtime instead.
pub fn restore_session_runtime(
    runtime: RuntimeHandle,
    session: &davinci_session::JsonlSession,
) -> Result<RuntimeHandle, String> {
    restore_session_runtime_with_legacy_recovery(runtime, session, &HashMap::new())
}

/// Cold restore with an explicit host-authored mapping for record-less legacy
/// task creations. The mapping supplies descriptive metadata only; event
/// identity, ownership, results and evidence remain authoritative.
pub fn restore_session_runtime_with_legacy_recovery(
    mut runtime: RuntimeHandle,
    session: &davinci_session::JsonlSession,
    recovery: &HashMap<super::TaskId, super::LegacyTaskRecovery>,
) -> Result<RuntimeHandle, String> {
    if session
        .entries
        .iter()
        .any(|entry| entry.custom_type.as_deref() == Some(WORKER_IDENTITY))
    {
        return Err("worker conversation requires its parent-bound runtime".into());
    }
    runtime = runtime.with_session(&session.header.id);
    let source = std::fs::canonicalize(&session.path)
        .map_err(|error| format!("session source could not be resolved: {error}"))?;
    let path = davinci_session::runtime_log_path(&source);
    let events = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {
            davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&path)
                .map_err(|error| format!("runtime log could not be read: {error}"))?
        }
        Ok(_) => return Err("runtime log path is not a file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("runtime log could not be inspected: {error}")),
    };
    if runtime.parent_agent_id.is_some() {
        return Err("workers must submit session task commands to the parent coordinator".into());
    }
    let journal_path = source.with_extension("tasks.jsonl");
    let journal_initialized = match std::fs::metadata(&journal_path) {
        Ok(metadata) if metadata.is_file() => metadata.len() != 0,
        Ok(_) => return Err("task journal path is not a file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("task journal could not be inspected: {error}")),
    };
    let legacy_tasks = if journal_initialized {
        Vec::new()
    } else {
        TaskRegistry::legacy_snapshot_with_recovery(&events, &session.header.id, recovery)
            .map_err(|error| format!("legacy tasks could not be migrated: {error}"))?
    };
    let legacy_task_observations = if legacy_tasks.is_empty() {
        Vec::new()
    } else {
        let source_bytes = std::fs::read(&path).unwrap_or_default();
        let source_identity = std::fs::canonicalize(&path)
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned();
        let source_digest = digest_bytes(&source_bytes);
        legacy_tasks
            .iter()
            .map(|task| {
                let mut observation = LegacyObservation::new(
                    LegacySourceKind::SessionTaskJournal,
                    source_identity.clone(),
                    source_digest.clone(),
                    task.id.to_string(),
                    serde_json::to_value(task.state)
                        .map_err(|error| error.to_string())?
                        .as_str()
                        .unwrap_or("unknown"),
                )
                .map_err(|error| error.to_string())?;
                observation
                    .evidence_provenance
                    .push("legacy_task_projection.state".into());
                if let Some(result) = &task.result {
                    observation.known_result = Some(serde_json::json!({
                        "result_digest": digest_bytes(result.as_bytes()),
                    }));
                    observation
                        .evidence_provenance
                        .push("legacy_task_projection.result_digest".into());
                }
                observation.observed_at_ms = u64::try_from(task.updated_at_ms).ok();
                observation.validate().map_err(|error| error.to_string())?;
                Ok::<_, String>(observation)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    runtime
        .registry
        .rehydrate_from_events(&events)
        .map_err(|error| format!("runtime registry could not be replayed: {error}"))?;
    runtime.mailbox.rehydrate_from_events(&events);
    let persisted_identity = events
        .iter()
        .find_map(|event| event.agent_id.map(|agent_id| (event.run_id, agent_id)));
    if let Some((run_id, agent_id)) = persisted_identity {
        runtime.run_id = run_id;
        runtime.agent_id = agent_id;
    }
    let conversation_events: Vec<_> = events
        .iter()
        .filter(|event| {
            event.agent_id == Some(runtime.agent_id) && event.session_id == runtime.session_id
        })
        .cloned()
        .collect();
    let key = serde_json::to_string(&(source, &session.header.id))
        .map_err(|error| format!("session source key could not be encoded: {error}"))?;
    let (tasks, run_id) =
        TaskRegistry::open_session_durable_with_legacy(&journal_path, &key, legacy_tasks)
            .map_err(|error| format!("task journal could not be opened: {error}"))?;
    tasks
        .validate_operation_receipts()
        .map_err(|error| format!("task operation receipts could not be validated: {error}"))?;
    if let Some(operations) = runtime.operations.as_ref() {
        operations
            .dispatcher()
            .journal()
            .import_legacy_observations(&legacy_task_observations)
            .map_err(|error| {
                format!("legacy task observations could not be imported before recovery: {error}")
            })?;
    }
    // No worker from this previous process can still own execution. Commit
    // the existing crash-recovery state before admitting new commands.
    for task in tasks.list_tasks(Some(run_id)) {
        if task.state == TaskState::Running {
            tasks
                .fail_task(task.id, Some("process_terminated".into()))
                .map_err(|error| format!("orphaned task could not be reconciled: {error}"))?;
        }
    }
    runtime.run_id = run_id;
    runtime.restore_conversation(&conversation_events)?;
    runtime.task_registry = tasks.with_observers(runtime.bus.clone());
    let subscriber = RuntimeLogSubscriber::open(&path)
        .map_err(|error| format!("runtime log could not be opened for observation: {error}"))?;
    runtime.bus.subscribe_session(Arc::new(subscriber));

    Ok(runtime)
}

/// Restore only a worker's conversation. The caller supplies its validated
/// parent authority; an OS-held writer lease guards this history. The worker
/// never becomes a second task coordinator.
pub(crate) fn restore_worker_session_runtime(
    mut runtime: RuntimeHandle,
    session: &mut davinci_session::JsonlSession,
) -> Result<RuntimeHandle, String> {
    if runtime.parent_agent_id.is_none() {
        return Err("worker conversation requires parent authority".into());
    }
    let source = std::fs::canonicalize(&session.path)
        .map_err(|error| format!("worker session source could not be resolved: {error}"))?;
    let lease = WorkerSessionLease::acquire(&source.with_extension("worker.lock"))?;
    let identity = serde_json::json!({
        "version": 1, "run": runtime.run_id, "agent": runtime.agent_id,
        "parent": runtime.parent_agent_id, "session": session.header.id,
        "source": source, "cwd": session.header.cwd,
    });
    let bindings: Vec<_> = session
        .entries
        .iter()
        .filter(|entry| entry.custom_type.as_deref() == Some(WORKER_IDENTITY))
        .collect();
    match bindings.as_slice() {
        [] if session.entries.is_empty() => {}
        [binding]
            if binding.entry_type == "custom" && binding.extra.get("data") == Some(&identity) => {}
        _ => return Err("worker conversation belongs to a different runtime lineage".into()),
    }
    if source
        .with_extension("tasks.jsonl")
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        return Err("worker conversation contains an independent task journal".into());
    }
    let path = davinci_session::runtime_log_path(&source);
    let events = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {
            davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&path)
                .map_err(|error| format!("worker runtime log could not be read: {error}"))?
        }
        Ok(_) => return Err("worker runtime log path is not a file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(format!(
                "worker runtime log could not be inspected: {error}"
            ))
        }
    };
    if events.iter().any(|event| {
        event.run_id != runtime.run_id
            || event.agent_id != Some(runtime.agent_id)
            || event.parent_agent_id != runtime.parent_agent_id
            || event.session_id.as_deref() != Some(session.header.id.as_str())
    }) {
        return Err("worker conversation belongs to a different runtime lineage".into());
    }
    // Detach before binding: a cloned child may still share a projection with
    // its parent, and neither validation nor recovery may mutate that parent.
    runtime.session_id = Some(session.header.id.clone());
    runtime.ensure_conversation_identity_current();
    runtime.sequence = Arc::new(std::sync::atomic::AtomicU64::new(0));
    runtime.restore_conversation(&events)?;
    // Stage observers separately as well: a later tool-ledger validation
    // failure must release this candidate's lease without changing the caller.
    let bus = super::RuntimeBus::new();
    bus.replace_turn_subscribers_from(&runtime.bus);
    runtime.bus = bus;
    let mut subscriber = RuntimeLogSubscriber::open(&path)
        .map_err(|error| format!("worker runtime log could not be opened: {error}"))?;
    subscriber._worker_lease = Some(lease);
    if session.entries.is_empty() {
        session
            .append_entry(davinci_session::SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some(WORKER_IDENTITY.into()),
                extra: serde_json::Map::from_iter([("data".into(), identity)]),
            })
            .map_err(|error| format!("worker identity could not be persisted: {error}"))?;
    }
    runtime.bus.subscribe_session(Arc::new(subscriber));
    Ok(runtime)
}

/// RuntimeSubscriber that appends runtime lifecycle events into the session's `.runtime.jsonl` sidecar.
pub struct RuntimeLogSubscriber {
    writer: Arc<Mutex<davinci_session::RuntimeLogWriter>>,
    _worker_lease: Option<WorkerSessionLease>,
}

impl RuntimeLogSubscriber {
    pub fn new(writer: davinci_session::RuntimeLogWriter) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
            _worker_lease: None,
        }
    }

    pub fn open(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, davinci_session::RuntimeLogError> {
        let writer = davinci_session::RuntimeLogWriter::open(path)?;
        Ok(Self::new(writer))
    }
}

struct WorkerSessionLease(#[allow(dead_code)] davinci_sys::lock::ExclusiveFileLock);

impl WorkerSessionLease {
    fn acquire(path: &std::path::Path) -> Result<Self, String> {
        davinci_sys::lock::ExclusiveFileLock::try_acquire(path)
            .map(Self)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::InvalidInput
                    if error.to_string().ends_with("is a reparse point") =>
                {
                    "worker conversation lease is a reparse point".into()
                }
                std::io::ErrorKind::InvalidInput => {
                    "worker conversation lease is not an ordinary file".into()
                }
                std::io::ErrorKind::Unsupported => {
                    "worker conversation ownership is unsupported on this platform".into()
                }
                _ => format!("worker conversation ownership unavailable: {error}"),
            })
    }
}

impl RuntimeSubscriber for RuntimeLogSubscriber {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        if let Ok(mut writer) = self.writer.lock() {
            if let Err(error) = writer.append(event) {
                eprintln!("[davinci-runtime] runtime sidecar append failed: {error}");
            }
        } else {
            eprintln!("[davinci-runtime] runtime sidecar lock failed");
        }
        RuntimeDecision::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerSessionLease;
    use tempfile::tempdir;

    #[test]
    fn worker_session_lease_blocks_a_second_owner_until_dropped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("worker.session.lock");
        let first = WorkerSessionLease::acquire(&path).unwrap();

        let error = match WorkerSessionLease::acquire(&path) {
            Ok(_) => panic!("a second worker acquired the same session lease"),
            Err(error) => error,
        };
        assert!(
            error.starts_with("worker conversation ownership unavailable:"),
            "{error}"
        );

        drop(first);
        assert!(path.is_file(), "OS-held lock files remain in place");
        assert!(WorkerSessionLease::acquire(&path).is_ok());
    }
}
