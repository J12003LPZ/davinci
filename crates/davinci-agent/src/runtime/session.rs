//! Durable session activation shared by the agent and product host.
//! Session loading parallels vendor/davinci/packages/coding-agent/src/core/agent-session.ts;
//! authoritative task recovery is a native runtime extension.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{
    RuntimeDecision, RuntimeEventEnvelope, RuntimeHandle, RuntimeSubscriber, TaskRegistry,
    TaskState,
};

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

/// RuntimeSubscriber that appends runtime lifecycle events into the session's `.runtime.jsonl` sidecar.
pub struct RuntimeLogSubscriber {
    writer: Arc<Mutex<davinci_session::RuntimeLogWriter>>,
}

impl RuntimeLogSubscriber {
    pub fn new(writer: davinci_session::RuntimeLogWriter) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    pub fn open(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, davinci_session::RuntimeLogError> {
        let writer = davinci_session::RuntimeLogWriter::open(path)?;
        Ok(Self::new(writer))
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
