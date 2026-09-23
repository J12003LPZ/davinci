//! Durable session activation shared by the agent and product host.
//! Session loading parallels vendor/davinci/packages/coding-agent/src/core/agent-session.ts;
//! authoritative task recovery is a native runtime extension.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{
    RuntimeDecision, RuntimeEventEnvelope, RuntimeHandle, RuntimeSubscriber, TaskRegistry,
    TaskState,
};

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

struct WorkerSessionLease(std::fs::File);

impl WorkerSessionLease {
    fn acquire(path: &std::path::Path) -> Result<Self, String> {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("worker conversation lease is not an ordinary file".into());
            }
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let unavailable = |error| format!("worker conversation ownership unavailable: {error}");
        let file = options.open(path).map_err(unavailable)?;
        let metadata = file.metadata().map_err(unavailable)?;
        if !metadata.is_file() {
            return Err("worker conversation lease is not an ordinary file".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("worker conversation lease is a reparse point".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: file retains ownership of this live descriptor.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(unavailable(std::io::Error::last_os_error()));
            }
        }
        #[cfg(not(any(unix, windows)))]
        return Err("worker conversation ownership is unsupported on this platform".into());
        #[cfg(any(unix, windows))]
        Ok(Self(file))
    }
}

impl Drop for WorkerSessionLease {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Closing alone may leave a concurrent fork holding the description.
            // SAFETY: this guard still owns the descriptor while unlocking it.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
        #[cfg(not(unix))]
        let _ = &self.0;
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
