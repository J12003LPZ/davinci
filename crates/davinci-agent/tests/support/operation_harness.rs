use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

pub const CHILD_CONFIG_ENV: &str = "DAVINCI_OPERATION_CRASH_CONFIG";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildConfig {
    pub journal_directory: PathBuf,
    pub endpoint_path: PathBuf,
    pub sink_path: PathBuf,
    pub marker_path: PathBuf,
    pub identity: SerializedIdentity,
    pub root: RootNamespaceId,
    pub owner: ExecutionOwner,
    pub spec: OperationSpec,
    pub fault: FaultPoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedIdentity {
    pub journal_id: JournalId,
    pub workspace: WorkspaceIdentity,
}

impl SerializedIdentity {
    pub fn into_journal_identity(self) -> JournalIdentity {
        JournalIdentity::new(self.journal_id, self.workspace).unwrap()
    }
}

pub struct ChildCrashInjector {
    pub selected: FaultPoint,
    pub marker: PathBuf,
}

impl FaultInjector for ChildCrashInjector {
    fn checkpoint(&mut self, point: FaultPoint) {
        if point != self.selected {
            return;
        }
        fs::write(&self.marker, format!("{point:?}")).unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
}

pub fn wait_for_marker(child: &mut Child, marker: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if marker.is_file() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(output) = child.stdout.as_mut() {
                output.read_to_string(&mut stdout).unwrap();
            }
            if let Some(output) = child.stderr.as_mut() {
                output.read_to_string(&mut stderr).unwrap();
            }
            let panic_detail =
                fs::read_to_string(marker.with_extension("panic")).unwrap_or_default();
            panic!(
                "child exited before fault boundary with status {status}; panic: {panic_detail}; stdout: {stdout}; stderr: {stderr}"
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "child process did not reach fault boundary: {}",
        marker.display()
    );
}

pub fn increment_endpoint(path: &Path, operation_id: OperationId) -> rusqlite::Result<()> {
    let connection = rusqlite::Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS endpoint_mutations (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            operation_id TEXT NOT NULL
        );",
    )?;
    connection.execute(
        "INSERT INTO endpoint_mutations (operation_id) VALUES (?1)",
        [operation_id.to_string()],
    )?;
    Ok(())
}

pub fn probe_endpoint(path: &Path, operation_id: OperationId) -> rusqlite::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let connection = rusqlite::Connection::open(path)?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM endpoint_mutations WHERE operation_id = ?1",
        [operation_id.to_string()],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

pub fn commit_projection_once(
    path: &Path,
    event_id: &str,
    payload: &Value,
) -> rusqlite::Result<()> {
    let connection = rusqlite::Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS projections (
            event_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );",
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO projections (event_id, payload_json) VALUES (?1, ?2)",
        rusqlite::params![event_id, serde_json::to_string(payload).unwrap()],
    )?;
    Ok(())
}

pub fn projection_count(path: &Path) -> rusqlite::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let connection = rusqlite::Connection::open(path)?;
    let count: i64 =
        connection.query_row("SELECT count(*) FROM projections", [], |row| row.get(0))?;
    Ok(count as u64)
}

pub fn standard_spec(
    identity: &SerializedIdentity,
    root: RootNamespaceId,
    key: &str,
) -> OperationSpec {
    OperationSpec::new(
        OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: root,
            session_id: format!("crash-{key}"),
            runtime_run_id: RunId::new(),
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: Some(TaskId::new()),
            graph: None,
            workspace: identity.workspace.clone(),
            caller: CallerType::HostControl,
            wire_tool_call_id: Some(format!("wire-{key}")),
        },
        ScopedIdempotencyKey::new(IdempotencyScope::HostControl, key).unwrap(),
        OperationKind::CustomExternalAction,
        EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: false,
            supports_postcondition_probe: true,
            supports_compensation: false,
            requires_live_owner: true,
        },
        json!({"operation": key, "action": "increment fake endpoint"}),
        vec![],
    )
    .unwrap()
}
